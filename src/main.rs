#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use tauri::{command, Manager, Emitter};
use base64::Engine;
use std::sync::{Arc, Mutex};

// GStreamer modules (REFACTORED - clean implementations)
mod gstreamer_camera;
mod gstreamer_composite;

use gstreamer_camera::GStreamerCamera;
use gstreamer_composite::GStreamerComposite;

use shared_memory::{Shmem, ShmemConf};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use futures_util::{StreamExt, SinkExt};
use tokio::sync::broadcast;

// Camera state
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct CameraDeviceInfo {
    id: String,
    name: String,
    description: String,
    is_available: bool,
}

// Global state for GStreamer (REFACTORED)
lazy_static::lazy_static! {
    static ref GSTREAMER_CAMERA: Arc<parking_lot::RwLock<Option<GStreamerCamera>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref CAMERA_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
    
    static ref GSTREAMER_COMPOSITE: Arc<parking_lot::RwLock<Option<GStreamerComposite>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref COMPOSITE_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref CAMERA_LAYER_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref OVERLAY_LAYER_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
}

// WebSocket ports
const CAMERA_WS_PORT: u16 = 9876;
// IPC replaces WebSocket ports - no longer needed

// Monitor info structure
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MonitorInfo {
    name: Option<String>,
    position: (i32, i32),
    size: (u32, u32),
    scale_factor: f64,
    is_primary: bool,
    screenshot: Option<String>,
}

// Global cache for monitor screenshots
static MONITOR_SCREENSHOTS: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());
static MODAL_IS_OPEN: Mutex<bool> = Mutex::new(false);

// Windows GDI screen capture fallback
fn capture_monitor_screenshot_fallback(x: i32, y: i32, width: i32, height: i32) -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
            GetDC, GetDIBits, ReleaseDC, SelectObject, SRCCOPY,
            BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS
        };
        use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        use std::mem;

        unsafe {
            let hwnd_desktop = GetDesktopWindow();
            let hdc_screen = GetDC(hwnd_desktop);
            if hdc_screen.is_invalid() {
                return None;
            }

            let hdc_mem = CreateCompatibleDC(hdc_screen);
            if hdc_mem.is_invalid() {
                let _ = ReleaseDC(hwnd_desktop, hdc_screen);
                return None;
            }

            let hbm = CreateCompatibleBitmap(hdc_screen, width, height);
            if hbm.is_invalid() {
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(hwnd_desktop, hdc_screen);
                return None;
            }

            let hbm_old = SelectObject(hdc_mem, hbm);

            if BitBlt(hdc_mem, 0, 0, width, height, hdc_screen, x, y, SRCCOPY).is_err() {
                let _ = SelectObject(hdc_mem, hbm_old);
                let _ = DeleteObject(hbm);
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(hwnd_desktop, hdc_screen);
                return None;
            }

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                },
                bmiColors: [Default::default(); 1],
            };

            let buffer_size = (width * height * 4) as usize;
            let mut buffer: Vec<u8> = vec![0; buffer_size];

            let result = GetDIBits(
                hdc_screen,
                hbm,
                0,
                height as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            );

            let _ = SelectObject(hdc_mem, hbm_old);
            let _ = DeleteObject(hbm);
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(hwnd_desktop, hdc_screen);

            if result == 0 {
                return None;
            }

            let is_all_zeros = buffer.iter().all(|&b| b == 0);
            if is_all_zeros {
                return None;
            }

            // Convert BGRA to RGBA
            for chunk in buffer.chunks_exact_mut(4) {
                let b = chunk[0];
                let g = chunk[1];
                let r = chunk[2];
                let a = chunk[3];
                chunk[0] = r;
                chunk[1] = g;
                chunk[2] = b;
                chunk[3] = a;
            }

            let img = image::RgbaImage::from_raw(width as u32, height as u32, buffer)?;
            let thumbnail_width = 400u32;
            let thumbnail_height = ((height as f32 / width as f32) * thumbnail_width as f32) as u32;
            let resized_img = image::imageops::resize(&img, thumbnail_width, thumbnail_height, image::imageops::FilterType::Triangle);

            let mut png_buffer = Vec::new();
            resized_img.write_to(&mut std::io::Cursor::new(&mut png_buffer), image::ImageFormat::Png).ok()?;

            let base64_string = base64::engine::general_purpose::STANDARD.encode(&png_buffer);
            let data_url = format!("data:image/png;base64,{}", base64_string);

            Some(data_url)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

fn read_monitors(app: &tauri::AppHandle) -> Vec<MonitorInfo> {
    let monitors = app.available_monitors().unwrap_or_default();
    println!("=== MONITOR DETECTION ===");
    println!("Found {} monitors", monitors.len());

    {
        let mut cache = MONITOR_SCREENSHOTS.lock().unwrap();
        if cache.len() != monitors.len() {
            cache.resize(monitors.len(), None);
        }
    }

    let monitor_infos: Vec<MonitorInfo> = monitors
        .into_iter()
        .enumerate()
        .map(|(i, m)| {
            let position = m.position();
            let size = m.size();
            let scale_factor = m.scale_factor();
            let name = m.name().cloned();

            println!("Monitor {}: name={:?}, size={}x{}, position=({}, {}), scale={}",
                     i, name, size.width, size.height, position.x, position.y, scale_factor);

            let screenshot = {
                if let Ok(cache) = MONITOR_SCREENSHOTS.lock() {
                    if i < cache.len() {
                        cache[i].clone()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            MonitorInfo {
                name,
                position: (position.x, position.y),
                size: (size.width, size.height),
                scale_factor,
                is_primary: i == 0,
                screenshot,
            }
        })
        .collect();

    let mut primary_index = 0;
    for (i, monitor) in monitor_infos.iter().enumerate() {
        if monitor.position.0 == 0 && monitor.position.1 == 0 {
            primary_index = i;
            println!("Found primary monitor at index {} (position 0,0)", i);
            break;
        }
    }

    let mut result = monitor_infos;
    for (i, monitor) in result.iter_mut().enumerate() {
        monitor.is_primary = i == primary_index;
    }

    println!("Primary monitor set to index {}", primary_index);
    println!("=== END MONITOR DETECTION ===");
    result
}

#[command]
async fn get_monitors(app: tauri::AppHandle) -> Vec<MonitorInfo> {
    read_monitors(&app)
}

#[command]
async fn set_modal_open(is_open: bool) {
    if let Ok(mut modal_state) = MODAL_IS_OPEN.lock() {
        *modal_state = is_open;
        println!("Monitor selection modal state set to: {}", is_open);
    }
}

#[command]
async fn start_realtime_capture(app: tauri::AppHandle) -> Result<(), String> {
    println!("Starting real-time monitor capture...");

    {
        let mut cache = MONITOR_SCREENSHOTS.lock().unwrap();
        cache.clear();
    }

    let monitors = app.available_monitors().unwrap_or_default();
    {
        let mut cache = MONITOR_SCREENSHOTS.lock().unwrap();
        cache.resize(monitors.len(), None);
    }

    let mut capture_tasks = vec![];

    for (i, monitor) in monitors.iter().enumerate() {
        let app_clone = app.clone();
        let monitor_clone = monitor.clone();

        let task = tauri::async_runtime::spawn(async move {
            let position = monitor_clone.position();
            let size = monitor_clone.size();

            let capture_width = size.width as i32;
            let capture_height = size.height as i32;

            let screenshot = capture_monitor_screenshot_fallback(
                position.x,
                position.y,
                capture_width,
                capture_height
            );

            if let Ok(mut cache) = crate::MONITOR_SCREENSHOTS.lock() {
                if i < cache.len() {
                    cache[i] = screenshot;
                    println!("Real-time capture ready for monitor {}: {}x{} (full monitor)", i, capture_width, capture_height);
                }
            }

            let _ = app_clone.emit("screenshot://ready", i);
        });

        capture_tasks.push(task);
    }

    for task in capture_tasks {
        let _ = task.await;
    }

    let monitors_clone = monitors.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let should_continue = {
                if let Ok(modal_state) = crate::MODAL_IS_OPEN.lock() {
                    *modal_state
                } else {
                    false
                }
            };

            if !should_continue {
                println!("Real-time background updates stopped - modal closed");
                break;
            }

            let mut update_tasks = vec![];

            for (i, monitor) in monitors_clone.iter().enumerate() {
                let app_clone = app.clone();
                let monitor_data = monitor.clone();

                let task = tauri::async_runtime::spawn(async move {
                    let position = monitor_data.position();
                    let size = monitor_data.size();

                    let capture_width = size.width as i32;
                    let capture_height = size.height as i32;

                    let screenshot = capture_monitor_screenshot_fallback(
                        position.x,
                        position.y,
                        capture_width,
                        capture_height
                    );

                    if let Ok(mut cache) = crate::MONITOR_SCREENSHOTS.lock() {
                        if i < cache.len() {
                            cache[i] = screenshot;
                        }
                    }

                    let _ = app_clone.emit("screenshot://updated", i);
                });

                update_tasks.push(task);
            }

            for task in update_tasks {
                let _ = task.await;
            }

            tokio::time::sleep(std::time::Duration::from_millis(67)).await;
        }
    });

    println!("Real-time monitor capture initialized");
    Ok(())
}

fn start_monitor_broadcast(_app: tauri::AppHandle) {
    // Broadcast disabled - monitors are fetched on-demand when modal opens
}

#[command]
async fn check_tv_monitor_window(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let window = app.get_window("tv-monitor");
    match window {
        Some(w) => match w.is_visible() {
            Ok(true) => Ok(serde_json::json!({ "isOpen": true })),
            _ => Ok(serde_json::json!({ "isOpen": false }))
        },
        None => Ok(serde_json::json!({ "isOpen": false }))
    }
}

#[command]
async fn close_tv_monitor_window(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_window("tv-monitor") {
        window.close().map_err(|e| format!("Failed to close window: {}", e))?;
        println!("TV monitor window closed and destroyed");
    } else {
        println!("TV monitor window not found to close");
    }
    Ok(())
}

#[command]
async fn create_monitor_window(
    app: tauri::AppHandle,
    url: String,
    monitor_index: usize
) -> Result<(), String> {
    println!("Creating monitor window for monitor index: {}", monitor_index);

    let native_monitors = app.available_monitors().unwrap_or_default();
    println!("Found {} native monitors", native_monitors.len());

    if monitor_index >= native_monitors.len() {
        return Err(format!("Invalid monitor index: {} (total monitors: {})", monitor_index, native_monitors.len()));
    }

    let native_monitor = &native_monitors[monitor_index];
    let monitor_pos = native_monitor.position();
    let monitor_size = native_monitor.size();
    let scale_factor = native_monitor.scale_factor();

    println!("Native monitor {}: position=({}, {}), size={}x{}, scale={}",
             monitor_index, monitor_pos.x, monitor_pos.y, monitor_size.width, monitor_size.height, scale_factor);

    let logical_width = monitor_size.width as f64 / scale_factor;
    let logical_height = monitor_size.height as f64 / scale_factor;
    let logical_x = monitor_pos.x as f64 / scale_factor;
    let logical_y = monitor_pos.y as f64 / scale_factor;

    println!("Logical coordinates: position=({}, {}), size={}x{}",
             logical_x, logical_y, logical_width, logical_height);

    if let Some(existing_window) = app.get_window("tv-monitor") {
        println!("Destroying existing TV monitor window before creating new one");
        let destroy_result = existing_window.destroy();
        match destroy_result {
            Ok(_) => println!("Window destroyed successfully"),
            Err(e) => println!("Failed to destroy window: {}", e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    println!("Creating borderless monitor-filling window");
    println!("Monitor window URL: {}", url);

    let window = tauri::webview::WebviewWindowBuilder::new(&app, "tv-monitor", tauri::WebviewUrl::External(url.parse().unwrap()))
        .title("TV Monitor")
        .inner_size(logical_width, logical_height)
        .position(logical_x, logical_y)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .fullscreen(false)
        .build()
        .map_err(|e| format!("Failed to create monitor window: {}", e))?;

    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            println!("TV monitor window was closed by user");
            let _ = app_handle.emit("tv-monitor-closed", ());
        }
    });

    window.show()
        .map_err(|e| format!("Failed to show monitor window: {}", e))?;
    window.set_focus()
        .map_err(|e| format!("Failed to focus monitor window: {}", e))?;

    println!("Monitor window created successfully on monitor {} at logical position ({}, {}) with size {}x{}",
             monitor_index, logical_x, logical_y, logical_width, logical_height);
    Ok(())
}

#[command]
async fn create_regular_window(app: tauri::AppHandle, url: String) -> Result<(), String> {
    println!("Creating regular window: 1080x640 at center-right position with direct URL");

    if let Some(existing_window) = app.get_window("tv-monitor") {
        println!("Destroying existing TV monitor window before creating regular window");
        let destroy_result = existing_window.destroy();
        match destroy_result {
            Ok(_) => println!("Window destroyed successfully"),
            Err(e) => println!("Failed to destroy window: {}", e),
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let monitors = app.available_monitors().unwrap_or_default();

    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let monitor = &monitors[0];
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();
    let scale_factor = monitor.scale_factor();

    println!("Primary monitor: position=({}, {}), size={}x{}, scale={}",
             monitor_pos.x, monitor_pos.y, monitor_size.width, monitor_size.height, scale_factor);

    let window_width = 1080.0;
    let window_height = 640.0;

    let logical_monitor_width = monitor_size.width as f64 / scale_factor;
    let logical_monitor_height = monitor_size.height as f64 / scale_factor;
    let logical_monitor_x = monitor_pos.x as f64 / scale_factor;
    let logical_monitor_y = monitor_pos.y as f64 / scale_factor;

    let x = logical_monitor_x + logical_monitor_width - window_width;
    let y = logical_monitor_y + (logical_monitor_height - window_height) / 2.0;

    println!("Logical monitor: {}x{} at ({}, {}), positioning window at ({}, {})",
             logical_monitor_width, logical_monitor_height, logical_monitor_x, logical_monitor_y, x, y);
    println!("Regular window URL: {}", url);

    let window = tauri::webview::WebviewWindowBuilder::new(&app, "tv-monitor", tauri::WebviewUrl::External(url.parse().unwrap()))
        .title("TV Monitor")
        .inner_size(window_width, window_height)
        .position(x, y)
        .decorations(true)
        .resizable(true)
        .minimizable(true)
        .maximizable(true)
        .closable(true)
        .build()
        .map_err(|e| format!("Failed to create regular window: {}", e))?;

    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            println!("TV monitor regular window was closed by user");
            let _ = app_handle.emit("tv-monitor-closed", ());
        }
    });

    window.show()
        .map_err(|e| format!("Failed to show regular window: {}", e))?;
    window.set_focus()
        .map_err(|e| format!("Failed to focus regular window: {}", e))?;

    println!("Regular window created successfully: {}x{} at logical position ({}, {}) with URL: {}", window_width, window_height, x, y, url);

    Ok(())
}

// ========================================
// CAMERA COMMANDS (REFACTORED - Clean implementation)
// ========================================

#[command]
async fn initialize_camera_system() -> Result<String, String> {
    println!("[Camera] Initializing camera system");
    
    if CAMERA_FRAME_SENDER.read().is_some() {
        println!("[Camera] Already initialized");
        return Ok("Camera system already initialized".to_string());
    }
    
    let camera = GStreamerCamera::new()
        .map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
    
    *GSTREAMER_CAMERA.write() = Some(camera);
    
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(2);
    
    if let Some(cam) = GSTREAMER_CAMERA.read().as_ref() {
        cam.set_frame_sender(tx.clone());
    }
    
    *CAMERA_FRAME_SENDER.write() = Some(tx);
    
    start_camera_websocket_server().await;
    
    println!("[Camera] ✅ WebSocket server started on port {}", CAMERA_WS_PORT);
    Ok(format!("Camera initialized - WebSocket on port {}", CAMERA_WS_PORT))
}

async fn start_camera_websocket_server() {
    tokio::spawn(async {
        let addr = format!("127.0.0.1:{}", CAMERA_WS_PORT);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                println!("[Camera WS] Failed to bind to {}: {}", addr, e);
                return;
            }
        };
        
        println!("[Camera WS] WebSocket server listening on {}", addr);
        
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let ws_stream = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        println!("[Camera WS] WebSocket handshake failed: {}", e);
                        return;
                    }
                };
                
                println!("[Camera WS] Client connected");
                
                let mut rx = match CAMERA_FRAME_SENDER.read().as_ref() {
                    Some(sender) => sender.subscribe(),
                    None => {
                        println!("[Camera WS] No frame sender available");
                        return;
                    }
                };
                
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();
                
                loop {
                    tokio::select! {
                        frame_result = rx.recv() => {
                            match frame_result {
                                Ok(frame_data) => {
                                    if ws_sender.send(tokio_tungstenite::tungstenite::Message::Binary(frame_data)).await.is_err() {
                                        println!("[Camera WS] Client disconnected");
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                        msg = ws_receiver.next() => {
                            match msg {
                                Some(Ok(_)) => {},
                                _ => {
                                    println!("[Camera WS] Client disconnected");
                                    break;
                                }
                            }
                        }
                    }
                }
            });
        }
    });
}

#[command]
async fn get_available_cameras() -> Result<Vec<CameraDeviceInfo>, String> {
    println!("[Camera] Enumerating cameras");
    
    let cameras_info = GStreamerCamera::list_cameras()?;
    
    let cameras: Vec<CameraDeviceInfo> = cameras_info
        .into_iter()
        .map(|cam| {
            println!("[Camera] Found: {}", cam.name);
            CameraDeviceInfo {
                id: cam.id,
                name: cam.name,
                description: cam.description,
                is_available: true,
            }
        })
        .collect();
    
    println!("[Camera] Total cameras found: {}", cameras.len());
    Ok(cameras)
}

#[command]
async fn start_camera_preview(device_id: String, _app: tauri::AppHandle) -> Result<(), String> {
    start_camera_preview_with_quality(device_id, "high".to_string(), _app).await
}

#[command]
async fn start_camera_preview_with_quality(device_id: String, quality: String, _app: tauri::AppHandle) -> Result<(), String> {
    println!("[Camera] Starting preview for device: {} with quality: {}", device_id, quality);
    
    stop_camera_preview().await?;
    
    let mut camera_lock = GSTREAMER_CAMERA.write();
    if let Some(camera) = camera_lock.as_mut() {
        camera.start_with_quality(&device_id, &quality)?;
        println!("[Camera] ✅ Camera started successfully!");
    } else {
        return Err("Camera not initialized".to_string());
    }
    drop(camera_lock);
    
    Ok(())
}

#[command]
async fn stop_camera_preview() -> Result<(), String> {
    println!("[Camera] Stopping preview");
    
    let mut camera_lock = GSTREAMER_CAMERA.write();
    if let Some(camera) = camera_lock.as_mut() {
        camera.stop()?;
    }
    drop(camera_lock);
    
    Ok(())
}

// ========================================
// COMPOSITE PIPELINE COMMANDS (REFACTORED - Clean implementation)
// ========================================

#[command]
async fn initialize_composite_system() -> Result<String, String> {
    println!("[Composite] Initializing composite system");
    
    {
        let sender_lock = COMPOSITE_FRAME_SENDER.read();
        if sender_lock.is_some() {
            println!("[Composite] Already initialized");
            return Ok("Composite system already initialized".to_string());
        }
    }
    
    let composite = GStreamerComposite::new()
        .map_err(|e| format!("Failed to initialize composite: {}", e))?;
    
    *GSTREAMER_COMPOSITE.write() = Some(composite);
    
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(2);
    let (camera_layer_tx, _camera_layer_rx) = broadcast::channel::<Vec<u8>>(2);
    let (overlay_layer_tx, _overlay_layer_rx) = broadcast::channel::<Vec<u8>>(2);

    if let Some(comp) = GSTREAMER_COMPOSITE.read().as_ref() {
        comp.set_frame_sender(tx.clone());
        comp.set_camera_frame_sender(camera_layer_tx.clone());
        comp.set_overlay_frame_sender(overlay_layer_tx.clone());
    }

    *COMPOSITE_FRAME_SENDER.write() = Some(tx);
    *CAMERA_LAYER_FRAME_SENDER.write() = Some(camera_layer_tx);
    *OVERLAY_LAYER_FRAME_SENDER.write() = Some(overlay_layer_tx);
    
    // Start IPC frame emitters instead of WebSocket servers
    start_composite_frame_emitter(app_handle.clone()).await;
    start_camera_layer_frame_emitter(app_handle.clone()).await;
    start_overlay_layer_frame_emitter(app_handle.clone()).await;
    
    println!("[Composite] ✅ Composite system initialized with IPC");
    Ok("Composite initialized with IPC".to_string())
}

// IPC Frame Emitters (replaces WebSocket servers)
async fn start_composite_frame_emitter(app_handle: tauri::AppHandle) {
    tokio::spawn(async move {
        let tx_opt = COMPOSITE_FRAME_SENDER.read().as_ref().cloned();
        
        if let Some(tx) = tx_opt {
            let mut rx = tx.subscribe();
            println!("[Composite IPC] Frame emitter started");
            
            while let Ok(frame_data) = rx.recv().await {
                // Emit as base64 to frontend
                let base64_frame = base64::engine::general_purpose::STANDARD.encode(&frame_data);
                let _ = app_handle.emit("composite-frame", base64_frame);
            }
        }
    });
}

async fn start_camera_layer_frame_emitter(app_handle: tauri::AppHandle) {
    tokio::spawn(async move {
        let tx_opt = CAMERA_LAYER_FRAME_SENDER.read().as_ref().cloned();
        
        if let Some(tx) = tx_opt {
            let mut rx = tx.subscribe();
            println!("[Camera Layer IPC] Frame emitter started");
            
            while let Ok(frame_data) = rx.recv().await {
                // Emit as base64 to frontend
                let base64_frame = base64::engine::general_purpose::STANDARD.encode(&frame_data);
                let _ = app_handle.emit("camera-layer-frame", base64_frame);
            }
        }
    });
}

async fn start_overlay_layer_frame_emitter(app_handle: tauri::AppHandle) {
    tokio::spawn(async move {
        let tx_opt = OVERLAY_LAYER_FRAME_SENDER.read().as_ref().cloned();
        
        if let Some(tx) = tx_opt {
            let mut rx = tx.subscribe();
            println!("[Overlay Layer IPC] Frame emitter started");
            
            while let Ok(frame_data) = rx.recv().await {
                // Emit as base64 to frontend
                let base64_frame = base64::engine::general_purpose::STANDARD.encode(&frame_data);
                let _ = app_handle.emit("overlay-layer-frame", base64_frame);
            }
        }
    });
}

#[command]
async fn start_composite_pipeline(camera_device_id: String, width: u32, height: u32, fps: u32, rotation: u32) -> Result<(), String> {
    println!("[Composite] Starting composite pipeline: {}x{} @ {}fps (rotation: {}°)", width, height, fps, rotation);
    
    let mut composite_lock = GSTREAMER_COMPOSITE.write();
    if let Some(composite) = composite_lock.as_mut() {
        composite.start(&camera_device_id, width, height, fps, rotation)?;
        println!("[Composite] ✅ Composite pipeline started");
    } else {
        return Err("Composite pipeline not initialized".to_string());
    }
    drop(composite_lock);
    
    Ok(())
}

#[command]
async fn stop_composite_pipeline() -> Result<(), String> {
    println!("[Composite] Stopping composite pipeline");
    
    let mut composite_lock = GSTREAMER_COMPOSITE.write();
    if let Some(composite) = composite_lock.as_mut() {
        composite.stop()?;
    }
    drop(composite_lock);
    
    Ok(())
}

#[command]
async fn update_composite_layers(camera: (bool, f64), overlay: (bool, f64)) -> Result<(), String> {
    let composite_lock = GSTREAMER_COMPOSITE.read();
    if let Some(composite) = composite_lock.as_ref() {
        composite.update_layers(camera, overlay);
    }
    drop(composite_lock);
    
    Ok(())
}

#[command]
async fn start_composite_output(format: String, width: u32, height: u32) -> Result<(), String> {
    println!("[Composite] Starting output: {} ({}x{})", format, width, height);
    
    let mut composite_lock = GSTREAMER_COMPOSITE.write();
    if let Some(composite) = composite_lock.as_mut() {
        composite.set_output_format(&format)?;
        println!("[Composite] ✅ Output started: {}", format);
    } else {
        return Err("Composite pipeline not initialized".to_string());
    }
    drop(composite_lock);
    
    Ok(())
}

#[command]
async fn stop_composite_output() -> Result<(), String> {
    println!("[Composite] Stopping output");
    
    let mut composite_lock = GSTREAMER_COMPOSITE.write();
    if let Some(composite) = composite_lock.as_mut() {
        composite.set_output_format("preview")?;
    }
    drop(composite_lock);
    
    Ok(())
}

#[command]
async fn play_composite_fx(
    _app: tauri::AppHandle,
    file_url: String,
    _file_data: Option<Vec<u8>>,
    filename: String,
    keycolor: String,
    tolerance: f64,
    similarity: f64,
    use_chroma_key: bool
) -> Result<(), String> {
    println!("[Composite] 🎬 Playing FX: {} (chroma: {})", filename, use_chroma_key);
    
    let clean_filename = filename
        .replace("%20", "_")
        .replace("/", "_")
        .replace("\\", "_");
    
    let temp_dir = std::env::temp_dir().join("battles_fx_cache");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;
    
    let local_path = temp_dir.join(&clean_filename);
    
    if !local_path.exists() {
        println!("[Composite] 📥 Downloading FX from Nuxt proxy...");
        
        let full_url = format!("https://local.battles.app:3000{}", file_url);
        
        let local_path_clone = local_path.clone();
        let full_url_clone = full_url.clone();
        let _download_result = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            
            let client = reqwest::blocking::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
            
            let response = client
                .get(&full_url_clone)
                .send()
                .map_err(|e| format!("Failed to download FX: {}", e))?;
            
            if !response.status().is_success() {
                return Err(format!("HTTP error: {}", response.status()));
            }
            
            let bytes = response.bytes()
                .map_err(|e| format!("Failed to read response: {}", e))?;
            
            println!("[Composite] 💾 Writing {} bytes to cache...", bytes.len());
            let mut file = std::fs::File::create(&local_path_clone)
                .map_err(|e| format!("Failed to create temp file: {}", e))?;
            file.write_all(&bytes)
                .map_err(|e| format!("Failed to write temp file: {}", e))?;
            
            Ok::<(), String>(())
        }).await.map_err(|e| format!("Download task failed: {}", e))??;
        
        println!("[Composite] ✅ Cached to {:?}", local_path.file_name());
    } else {
        println!("[Composite] ⚡ Using existing cache (instant)");
    }
    
    let file_path_str = local_path.to_string_lossy().to_string();
    
    // Overlay WebSocket server already started at initialization
    
    let mut composite_lock = GSTREAMER_COMPOSITE.write();
    if let Some(composite) = composite_lock.as_mut() {
        composite.play_fx_from_file(file_path_str, keycolor, tolerance, similarity, use_chroma_key)?;
        println!("[Composite] ✅ FX playback started");
    } else {
        return Err("Composite pipeline not initialized".to_string());
    }
    drop(composite_lock);
    
    Ok(())
}

#[command]
async fn stop_composite_fx() -> Result<(), String> {
    println!("[Composite] Stopping FX");

    let mut composite_lock = GSTREAMER_COMPOSITE.write();
    if let Some(composite) = composite_lock.as_mut() {
        composite.stop_fx()?;
        println!("[Composite] ✅ FX stopped");
    } else {
        return Err("Composite pipeline not initialized".to_string());
    }
    drop(composite_lock);

    // Note: Keep overlay WebSocket server running (don't reset OVERLAY_WS_STARTED)
    // It will be reused for the next FX play

    Ok(())
}

// ========================================
// VIRTUAL CAMERA (DirectShow-compatible)
// ========================================

lazy_static::lazy_static! {
    static ref VIRTUAL_CAM_RUNNING: parking_lot::RwLock<bool> = parking_lot::RwLock::new(false);
}

const VCAM_WIDTH: u32 = 1920;
const VCAM_HEIGHT: u32 = 1080;
const VCAM_FPS: u32 = 30;
const VCAM_SHMEM_NAME: &str = "BattlesVirtualCam";

#[derive(serde::Serialize, serde::Deserialize)]
struct VirtualCamInfo {
    name: String,
    width: u32,
    height: u32,
    fps: u32,
    is_running: bool,
}

fn get_or_create_shmem() -> Result<Shmem, String> {
    let frame_size = (VCAM_WIDTH * VCAM_HEIGHT * 3) as usize;
    let header_size = 256;
    let total_size = header_size + frame_size;
    
    match ShmemConf::new()
        .size(total_size)
        .os_id(VCAM_SHMEM_NAME)
        .create()
    {
        Ok(m) => Ok(m),
        Err(_) => {
            ShmemConf::new()
                .os_id(VCAM_SHMEM_NAME)
                .open()
                .map_err(|e| format!("Failed to create/open shared memory: {}", e))
        }
    }
}

#[command]
async fn initialize_virtual_camera() -> Result<VirtualCamInfo, String> {
    println!("[VCam] Initializing virtual camera system");
    
    let frame_size = (VCAM_WIDTH * VCAM_HEIGHT * 3) as usize;
    let header_size = 256;
    let total_size = header_size + frame_size;
    
    let _ = get_or_create_shmem()?;
    
    println!("[VCam] Virtual camera initialized - {} bytes shared memory", total_size);
    
    Ok(VirtualCamInfo {
        name: "Battles Virtual Camera".to_string(),
        width: VCAM_WIDTH,
        height: VCAM_HEIGHT,
        fps: VCAM_FPS,
        is_running: *VIRTUAL_CAM_RUNNING.read(),
    })
}

#[command]
async fn start_virtual_camera() -> Result<(), String> {
    println!("[VCam] Starting virtual camera");
    
    let _ = get_or_create_shmem()?;
    
    *VIRTUAL_CAM_RUNNING.write() = true;
    
    println!("[VCam] Virtual camera started - ready to receive frames");
    Ok(())
}

#[command]
async fn stop_virtual_camera() -> Result<(), String> {
    println!("[VCam] Stopping virtual camera");
    *VIRTUAL_CAM_RUNNING.write() = false;
    Ok(())
}

#[command]
async fn send_frame_to_virtual_camera(frame_data: Vec<u8>, width: u32, height: u32) -> Result<(), String> {
    if !*VIRTUAL_CAM_RUNNING.read() {
        return Err("Virtual camera not running".to_string());
    }
    
    let shmem = get_or_create_shmem()?;
    
    let expected_size = (width * height * 3) as usize;
    if frame_data.len() != expected_size {
        return Err(format!("Invalid frame size: expected {}, got {}", expected_size, frame_data.len()));
    }
    
    unsafe {
        let ptr = shmem.as_ptr() as *mut u8;
        
        let timestamp = chrono::Utc::now().timestamp_millis();
        std::ptr::copy_nonoverlapping(&timestamp as *const i64 as *const u8, ptr, 8);
        std::ptr::copy_nonoverlapping(&width as *const u32 as *const u8, ptr.add(8), 4);
        std::ptr::copy_nonoverlapping(&height as *const u32 as *const u8, ptr.add(12), 4);
        
        std::ptr::copy_nonoverlapping(frame_data.as_ptr(), ptr.add(256), frame_data.len());
    }
    
    Ok(())
}

#[command]
async fn get_virtual_camera_status() -> Result<VirtualCamInfo, String> {
    Ok(VirtualCamInfo {
        name: "Battles Virtual Camera".to_string(),
        width: VCAM_WIDTH,
        height: VCAM_HEIGHT,
        fps: VCAM_FPS,
        is_running: *VIRTUAL_CAM_RUNNING.read(),
    })
}

fn main() {
    // Configure cache plugin for FX files
    let cache_config = tauri_plugin_cache::CacheConfig {
        cache_dir: Some("battles_fx_cache".into()),
        cache_file_name: Some("fx_cache.json".into()),
        cleanup_interval: Some(300),
        default_compression: Some(true),
        compression_level: Some(6),
        compression_threshold: Some(1024 * 100),
        compression_method: None,
    };
    
    tauri::Builder::default()
        .plugin(tauri_plugin_cache::init_with_config(cache_config))
        .setup(|app| {
            let app_handle = app.handle().clone();
            start_monitor_broadcast(app_handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_monitors,
            check_tv_monitor_window,
            close_tv_monitor_window,
            create_monitor_window,
            create_regular_window,
            set_modal_open,
            start_realtime_capture,
            initialize_camera_system,
            initialize_composite_system,
            get_available_cameras,
            start_camera_preview,
            start_camera_preview_with_quality,
            stop_camera_preview,
            start_composite_pipeline,
            stop_composite_pipeline,
            update_composite_layers,
            start_composite_output,
            stop_composite_output,
            play_composite_fx,
            stop_composite_fx,
            initialize_virtual_camera,
            start_virtual_camera,
            stop_virtual_camera,
            send_frame_to_virtual_camera,
            get_virtual_camera_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
