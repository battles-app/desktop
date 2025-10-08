#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use tauri::{command, Manager, Emitter};
use base64::Engine;
use std::sync::{Arc, Mutex};

// GStreamer camera module (OBS-quality video pipeline)
mod gstreamer_camera;
use gstreamer_camera::GStreamerCamera;

// GStreamer composite module (OBS replacement)
mod gstreamer_composite;
use gstreamer_composite::GStreamerComposite;

// WGPU surface renderer (direct window rendering)
mod wgpu_surface_renderer;

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

// Global state for GStreamer camera
lazy_static::lazy_static! {
    static ref GSTREAMER_CAMERA: Arc<parking_lot::RwLock<Option<GStreamerCamera>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref CAMERA_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
    
    // Composite pipeline (OBS replacement)
    static ref GSTREAMER_COMPOSITE: Arc<parking_lot::RwLock<Option<GStreamerComposite>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref COMPOSITE_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
    
    // Latest frame for direct access (no WebSocket overhead)
    static ref LATEST_COMPOSITE_FRAME: Arc<parking_lot::RwLock<Option<Vec<u8>>>> = Arc::new(parking_lot::RwLock::new(None));
}

const CAMERA_WS_PORT: u16 = 9876;
const COMPOSITE_WS_PORT: u16 = 9877;

// Monitor info structure
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MonitorInfo {
    name: Option<String>,
    position: (i32, i32),
    size: (u32, u32),
    scale_factor: f64,
    is_primary: bool,
    screenshot: Option<String>, // Base64 encoded PNG data URL
}

// Global cache for monitor screenshots
static MONITOR_SCREENSHOTS: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());

// Global flag to track if monitor selection modal is open
static MODAL_IS_OPEN: Mutex<bool> = Mutex::new(false);



// Fallback function using Windows GDI (original implementation)
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
            // Get desktop window and DC
            let hwnd_desktop = GetDesktopWindow();
            let hdc_screen = GetDC(hwnd_desktop);

            if hdc_screen.is_invalid() {
                return None;
            }

            // Create memory DC
            let hdc_mem = CreateCompatibleDC(hdc_screen);
            if hdc_mem.is_invalid() {
                let _ = ReleaseDC(hwnd_desktop, hdc_screen);
                return None;
            }

            // Create bitmap
            let hbm = CreateCompatibleBitmap(hdc_screen, width, height);
            if hbm.is_invalid() {
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(hwnd_desktop, hdc_screen);
                return None;
            }

            // Select bitmap
            let hbm_old = SelectObject(hdc_mem, hbm);

            // Copy from screen to memory DC
            if BitBlt(hdc_mem, 0, 0, width, height, hdc_screen, x, y, SRCCOPY).is_err() {
                let _ = SelectObject(hdc_mem, hbm_old);
                let _ = DeleteObject(hbm);
                let _ = DeleteDC(hdc_mem);
                let _ = ReleaseDC(hwnd_desktop, hdc_screen);
                return None;
            }

            // Prepare bitmap info
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height, // top-down
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

            // Get bitmap data
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

            // Cleanup
            let _ = SelectObject(hdc_mem, hbm_old);
            let _ = DeleteObject(hbm);
            let _ = DeleteDC(hdc_mem);
            let _ = ReleaseDC(hwnd_desktop, hdc_screen);

            if result == 0 {
                return None;
            }

            // Check if buffer has actual screen data
            let is_all_zeros = buffer.iter().all(|&b| b == 0);
            if is_all_zeros {
                return None; // Screen capture failed
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

            // Create image from RGBA buffer
            let img = image::RgbaImage::from_raw(width as u32, height as u32, buffer)?;

            // Resize to higher quality thumbnail (400x225) for better monitor overview
            let thumbnail_width = 400u32;
            let thumbnail_height = ((height as f32 / width as f32) * thumbnail_width as f32) as u32;
            let resized_img = image::imageops::resize(&img, thumbnail_width, thumbnail_height, image::imageops::FilterType::Triangle);

            // Create PNG from thumbnail
            let mut png_buffer = Vec::new();
            resized_img.write_to(&mut std::io::Cursor::new(&mut png_buffer), image::ImageFormat::Png).ok()?;

            // Convert to base64 data URL
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

// Read monitors using Tauri's built-in API and cached screenshots
fn read_monitors(app: &tauri::AppHandle) -> Vec<MonitorInfo> {
    let monitors = app.available_monitors().unwrap_or_default();
    println!("=== MONITOR DETECTION ===");
    println!("Found {} monitors", monitors.len());

    // Update cache size if monitors changed
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

            // Get screenshot from cache (captured in background)
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
                is_primary: i == 0, // Assume first monitor is primary
                screenshot,
            }
        })
        .collect();

    // Find actual primary monitor (the one at 0,0 or with primary flag)
    let mut primary_index = 0;
    for (i, monitor) in monitor_infos.iter().enumerate() {
        if monitor.position.0 == 0 && monitor.position.1 == 0 {
            primary_index = i;
            println!("Found primary monitor at index {} (position 0,0)", i);
            break;
        }
    }

    // Mark the correct primary monitor
    let mut result = monitor_infos;
    for (i, monitor) in result.iter_mut().enumerate() {
        monitor.is_primary = i == primary_index;
    }

    println!("Primary monitor set to index {}", primary_index);
    println!("=== END MONITOR DETECTION ===");
    result
}

// Get monitor information
#[command]
async fn get_monitors(app: tauri::AppHandle) -> Vec<MonitorInfo> {
    read_monitors(&app)
}


// Set modal open state
#[command]
async fn set_modal_open(is_open: bool) {
    if let Ok(mut modal_state) = MODAL_IS_OPEN.lock() {
        *modal_state = is_open;
        println!("Monitor selection modal state set to: {}", is_open);
    }
}

// Start real-time monitor capture (called when modal opens)
#[command]
async fn start_realtime_capture(app: tauri::AppHandle) -> Result<(), String> {
    println!("Starting real-time monitor capture...");

    // Clear existing cache to start fresh
    {
        let mut cache = MONITOR_SCREENSHOTS.lock().unwrap();
        cache.clear();
    }

    // Get current monitors and resize cache
    let monitors = app.available_monitors().unwrap_or_default();
    {
        let mut cache = MONITOR_SCREENSHOTS.lock().unwrap();
        cache.resize(monitors.len(), None);
    }

    // Start parallel capture for all monitors using scap for true video streaming
    let mut capture_tasks = vec![];

    for (i, monitor) in monitors.iter().enumerate() {
        let app_clone = app.clone();
        let monitor_clone = monitor.clone(); // Clone the monitor data

        let task = tauri::async_runtime::spawn(async move {
            let position = monitor_clone.position();
            let size = monitor_clone.size();

            // Capture ENTIRE monitor content for complete preview
            // Shows full desktop, taskbar, windows, icons - everything visible on monitor
            let capture_width = size.width as i32;
            let capture_height = size.height as i32;

            let screenshot = capture_monitor_screenshot_fallback(
                position.x,
                position.y,
                capture_width,
                capture_height
            );

            // Update cache immediately when ready
            if let Ok(mut cache) = crate::MONITOR_SCREENSHOTS.lock() {
                if i < cache.len() {
                    cache[i] = screenshot;
                    println!("Real-time capture ready for monitor {}: {}x{} (full monitor)", i, capture_width, capture_height);
                }
            }

            // Emit event to notify frontend that screenshot is ready
            let _ = app_clone.emit("screenshot://ready", i);
        });

        capture_tasks.push(task);
    }

    // Wait for all captures to complete (they run in parallel)
    for task in capture_tasks {
        let _ = task.await;
    }

    // Start background updates while modal is open
    let monitors_clone = monitors.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            // Check if modal is still open
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

            // Update all monitors simultaneously
            let mut update_tasks = vec![];

            for (i, monitor) in monitors_clone.iter().enumerate() {
                let app_clone = app.clone();
                let monitor_data = monitor.clone(); // Clone monitor data for async task

                let task = tauri::async_runtime::spawn(async move {
                    let position = monitor_data.position();
                    let size = monitor_data.size();

                    // Capture ENTIRE monitor content for real-time video broadcast
                    // Shows full desktop, taskbar, windows, icons - everything visible on monitor
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

            // Wait for all updates to complete
            for task in update_tasks {
                let _ = task.await;
            }

            // Update every 67ms for real-time video-like experience (15fps)
            tokio::time::sleep(std::time::Duration::from_millis(67)).await;
        }
    });

    println!("Real-time monitor capture initialized");
    Ok(())
}

// Start live monitor broadcast - DISABLED to reduce CPU/console spam
// Monitors are only loaded when the modal is opened via get_monitors command
fn start_monitor_broadcast(_app: tauri::AppHandle) {
    // Broadcast disabled - monitors are fetched on-demand when modal opens
    // This prevents unnecessary CPU usage and console spam
}

// Check if TV monitor window is open
#[command]
async fn check_tv_monitor_window(app: tauri::AppHandle) -> Result<serde_json::Value, String> {
    let window = app.get_window("tv-monitor");

    match window {
        Some(w) => {
            // Try to check if window is visible
            match w.is_visible() {
                Ok(true) => Ok(serde_json::json!({ "isOpen": true })),
                _ => Ok(serde_json::json!({ "isOpen": false }))
            }
        },
        None => Ok(serde_json::json!({ "isOpen": false }))
    }
}

// Close TV monitor window (completely destroy it)
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

// Create a window on a specific monitor
#[command]
async fn create_monitor_window(
    app: tauri::AppHandle,
    url: String,
    monitor_index: usize
) -> Result<(), String> {
    println!("Creating monitor window for monitor index: {}", monitor_index);

    // Get Tauri's native monitor info directly for accurate positioning
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

    // Convert physical pixels to logical pixels for Tauri v2
    // Tauri's inner_size and position expect logical pixels
    let logical_width = monitor_size.width as f64 / scale_factor;
    let logical_height = monitor_size.height as f64 / scale_factor;
    let logical_x = monitor_pos.x as f64 / scale_factor;
    let logical_y = monitor_pos.y as f64 / scale_factor;

    println!("Logical coordinates: position=({}, {}), size={}x{}",
             logical_x, logical_y, logical_width, logical_height);

    // Close any existing TV monitor window first
    if let Some(existing_window) = app.get_window("tv-monitor") {
        println!("Destroying existing TV monitor window before creating new one");
        let destroy_result = existing_window.destroy();
        match destroy_result {
            Ok(_) => println!("Window destroyed successfully"),
            Err(e) => println!("Failed to destroy window: {}", e),
        }
        // Add a small delay to ensure destruction completes
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Create a borderless window that fills the entire monitor
    println!("Creating borderless monitor-filling window");
    println!("Monitor window URL: {}", url);

    // Use WebviewWindowBuilder which supports URL in Tauri v2
    // Pass the authenticated URL directly - no loading page needed
    let window = tauri::webview::WebviewWindowBuilder::new(&app, "tv-monitor", tauri::WebviewUrl::External(url.parse().unwrap()))
        .title("TV Monitor")
        .inner_size(logical_width, logical_height)
        .position(logical_x, logical_y)
        .decorations(false) // No borders for fullscreen-like appearance
        .resizable(false)
        .always_on_top(true)
        .fullscreen(false) // Use borderless window instead of true fullscreen
        .build()
        .map_err(|e| format!("Failed to create monitor window: {}", e))?;

    // Listen for window close events to notify the main window
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            println!("TV monitor window was closed by user");
            // Emit event to notify main window that TV monitor was closed
            let _ = app_handle.emit("tv-monitor-closed", ());
        }
    });

    // Show and focus the window
    window.show()
        .map_err(|e| format!("Failed to show monitor window: {}", e))?;
    window.set_focus()
        .map_err(|e| format!("Failed to focus monitor window: {}", e))?;

    println!("Monitor window created successfully on monitor {} at logical position ({}, {}) with size {}x{}",
             monitor_index, logical_x, logical_y, logical_width, logical_height);
    Ok(())
}

// Create regular window (1080x640, resizable, movable, center-right position)
#[command]
async fn create_regular_window(app: tauri::AppHandle, url: String) -> Result<(), String> {
    println!("Creating regular window: 1080x640 at center-right position with direct URL");

    // Close any existing TV monitor window first
    if let Some(existing_window) = app.get_window("tv-monitor") {
        println!("Destroying existing TV monitor window before creating regular window");
        let destroy_result = existing_window.destroy();
        match destroy_result {
            Ok(_) => println!("Window destroyed successfully"),
            Err(e) => println!("Failed to destroy window: {}", e),
        }
        // Add a small delay to ensure destruction completes
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Get the primary monitor for positioning (first monitor is typically primary)
    let monitors = app.available_monitors().unwrap_or_default();

    if monitors.is_empty() {
        return Err("No monitors found".to_string());
    }

    let monitor = &monitors[0]; // Use first monitor (typically primary)
    let monitor_pos = monitor.position();
    let monitor_size = monitor.size();
    let scale_factor = monitor.scale_factor();

    println!("Primary monitor: position=({}, {}), size={}x{}, scale={}",
             monitor_pos.x, monitor_pos.y, monitor_size.width, monitor_size.height, scale_factor);

    // Window dimensions in logical pixels
    let window_width = 1080.0;
    let window_height = 640.0;

    // Convert monitor physical pixels to logical pixels
    let logical_monitor_width = monitor_size.width as f64 / scale_factor;
    let logical_monitor_height = monitor_size.height as f64 / scale_factor;
    let logical_monitor_x = monitor_pos.x as f64 / scale_factor;
    let logical_monitor_y = monitor_pos.y as f64 / scale_factor;

    // Calculate center-right position in logical pixels
    // Right edge of window touches right edge of monitor
    let x = logical_monitor_x + logical_monitor_width - window_width;
    // Center vertically
    let y = logical_monitor_y + (logical_monitor_height - window_height) / 2.0;

    println!("Logical monitor: {}x{} at ({}, {}), positioning window at ({}, {})",
             logical_monitor_width, logical_monitor_height, logical_monitor_x, logical_monitor_y, x, y);
    println!("Regular window URL: {}", url);

    // Create regular window with title bar, resizable, movable
    // Pass authenticated URL directly - no loading page needed
    let window = tauri::webview::WebviewWindowBuilder::new(&app, "tv-monitor", tauri::WebviewUrl::External(url.parse().unwrap()))
        .title("TV Monitor")
        .inner_size(window_width, window_height)
        .position(x, y)
        .decorations(true) // Enable title bar and borders
        .resizable(true) // Allow resizing
        .minimizable(true)
        .maximizable(true)
        .closable(true)
        .build()
        .map_err(|e| format!("Failed to create regular window: {}", e))?;

    // Listen for window close events to notify the main window
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            println!("TV monitor regular window was closed by user");
            // Emit event to notify main window that TV monitor was closed
            let _ = app_handle.emit("tv-monitor-closed", ());
        }
    });

    // Show and focus the window
    window.show()
        .map_err(|e| format!("Failed to show regular window: {}", e))?;
    window.set_focus()
        .map_err(|e| format!("Failed to focus regular window: {}", e))?;

    println!("Regular window created successfully: {}x{} at logical position ({}, {}) with URL: {}", window_width, window_height, x, y, url);

    Ok(())
}

// Camera Commands

#[command]
async fn initialize_camera_system() -> Result<String, String> {
    println!("[GStreamer] Initializing camera system");
    
    // Only initialize once
    if CAMERA_FRAME_SENDER.read().is_some() {
        println!("[GStreamer] Already initialized");
        return Ok("GStreamer camera system already initialized".to_string());
    }
    
    // Initialize GStreamer camera
    let camera = GStreamerCamera::new()
        .map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
    
    *GSTREAMER_CAMERA.write() = Some(camera);
    
    // Create broadcast channel for camera frames (capacity: 2 frames, drops old frames if full)
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(2);
    
    // Set frame sender in camera
    if let Some(cam) = GSTREAMER_CAMERA.read().as_ref() {
        cam.set_frame_sender(tx.clone());
    }
    
    *CAMERA_FRAME_SENDER.write() = Some(tx);
    
    // Start WebSocket server (only once)
    start_camera_websocket_server().await;
    
    println!("[GStreamer] ✅ WebSocket server started on port {}", CAMERA_WS_PORT);
    Ok(format!("GStreamer initialized - WebSocket on port {}", CAMERA_WS_PORT))
}

// WebSocket server for real-time binary camera streaming
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
                
                // Subscribe to camera frames
                let mut rx = match CAMERA_FRAME_SENDER.read().as_ref() {
                    Some(sender) => sender.subscribe(),
                    None => {
                        println!("[Camera WS] No frame sender available");
                        return;
                    }
                };
                
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();
                
                // Stream frames to client
                loop {
                    tokio::select! {
                        frame_result = rx.recv() => {
                            match frame_result {
                                Ok(frame_data) => {
                                    // Send binary frame
                                    if ws_sender.send(tokio_tungstenite::tungstenite::Message::Binary(frame_data)).await.is_err() {
                                        println!("[Camera WS] Client disconnected");
                                        break;
                                    }
                                }
                                Err(_) => {
                                    // Channel closed or lagged
                                    break;
                                }
                            }
                        }
                        msg = ws_receiver.next() => {
                            match msg {
                                Some(Ok(_)) => {}, // Ignore client messages
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

// Initialize composite system with transparent window overlay
#[command]
async fn initialize_composite_system(app: tauri::AppHandle, width: u32, height: u32) -> Result<String, String> {
    println!("[Composite] 🚀 Initializing with TRANSPARENT WINDOW OVERLAY architecture");
    
    // Only initialize once - check if already done
    {
        let sender_lock = COMPOSITE_FRAME_SENDER.read();
        if sender_lock.is_some() {
            println!("[Composite] Already initialized");
            return Ok("Composite system already initialized".to_string());
        }
    } // Release lock before async operations
    
    // Initialize composite pipeline
    let mut composite = GStreamerComposite::new()
        .map_err(|e| format!("Failed to initialize composite: {}", e))?;
    
    // Get the native window (NOT WebView!) for WGPU surface
    let window = app.get_window("main")
        .ok_or("Failed to get main window".to_string())?;
    
    println!("[Composite] 🔧 Setting up WGPU surface on native window...");
    composite.set_window_async(Arc::new(window), width, height)
        .await
        .map_err(|e| format!("Failed to initialize surface: {}", e))?;
    
    *GSTREAMER_COMPOSITE.write() = Some(composite);
    
    // Create broadcast channel for FX commands (NOT video frames!)
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(2);
    
    // Set frame sender in composite (for FX only)
    if let Some(comp) = GSTREAMER_COMPOSITE.read().as_ref() {
        comp.set_frame_sender(tx.clone());
    }
    
    // Set sender before starting WebSocket to prevent multiple initializations
    *COMPOSITE_FRAME_SENDER.write() = Some(tx);
    
    // Start WebSocket server (ONLY for FX commands, NOT video frames!)
    start_composite_websocket_server().await;

    println!("[Composite] ✅ Composite system initialized with DIRECT SURFACE RENDERING!");
    println!("[Composite] 💡 Video renders directly to window - ZERO WebSocket overhead!");
    Ok("Composite initialized with direct surface rendering".to_string())
}

// WebSocket server for composite frames
async fn start_composite_websocket_server() {
    tokio::spawn(async {
        let addr = format!("127.0.0.1:{}", COMPOSITE_WS_PORT);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                println!("[Composite WS] ❌ Failed to bind to {}: {}", addr, e);
                println!("[Composite WS] This might be because port {} is already in use", COMPOSITE_WS_PORT);
                return;
            }
        };

        println!("[Composite WS] ✅ WebSocket server listening on {}", addr);
        
        while let Ok((stream, _)) = listener.accept().await {
            // Clone the sender before spawning
            let tx_opt = COMPOSITE_FRAME_SENDER.read().as_ref().cloned();
            
            tokio::spawn(async move {
                let ws_stream = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        println!("[Composite WS] Error during handshake: {}", e);
                        return;
                    }
                };
                
                println!("[Composite WS] ✅ Client connected");
                
                let (mut ws_sender, _ws_receiver) = ws_stream.split();
                
                // Subscribe to composite frames
                if let Some(tx) = tx_opt {
                    let mut rx = tx.subscribe();
                    let mut frame_count = 0u64;
                    let start_time = std::time::Instant::now();

                    while let Ok(frame_data) = rx.recv().await {
                        use futures_util::SinkExt;
                        use tokio_tungstenite::tungstenite::protocol::Message;

                        frame_count += 1;
                        
                        // Log every 30 frames instead of every frame to reduce spam
                        if frame_count % 30 == 0 {
                            let elapsed = start_time.elapsed().as_secs_f64();
                            let fps = frame_count as f64 / elapsed;
                            println!("[Composite WS] 📡 Sending frame {} ({} bytes, {:.1} fps)", frame_count, frame_data.len(), fps);
                        }
                        
                        if ws_sender.send(Message::Binary(frame_data)).await.is_err() {
                            println!("[Composite WS] ❌ Client disconnected after {} frames", frame_count);
                            break;
                        }
                    }
                    
                    println!("[Composite WS] ℹ️ Broadcast channel closed");
                } else {
                    println!("[Composite WS] ❌ No broadcast channel available for client");
                }
            });
        }
    });
}

#[command]
async fn get_available_cameras() -> Result<Vec<CameraDeviceInfo>, String> {
    println!("[GStreamer] Enumerating cameras");
    
    let cameras_info = GStreamerCamera::list_cameras()?;
    
    let cameras: Vec<CameraDeviceInfo> = cameras_info
        .into_iter()
        .map(|cam| {
            println!("[GStreamer] Found: {}", cam.name);
            CameraDeviceInfo {
                id: cam.id,
                name: cam.name,
                description: cam.description,
                is_available: true,
            }
        })
        .collect();
    
    println!("[GStreamer] Total cameras found: {}", cameras.len());
    Ok(cameras)
}

// System monitoring task - DISABLED because emergency_cleanup() was killing pipelines
// The monitor was calling emergency_cleanup() every 10 seconds which set pipeline to Null state
#[allow(dead_code)]
async fn start_system_monitor() {
    // DISABLED - This was causing pipeline to stop every 10 seconds
    // The emergency_cleanup() call was setting pipeline state to Null
    // If you need monitoring, remove the emergency_cleanup() call
    println!("[System] ⚠️ System monitor disabled to prevent pipeline interference");
}

#[command]
async fn start_camera_preview(device_id: String, _app: tauri::AppHandle) -> Result<(), String> {
    start_camera_preview_with_quality(device_id, "high".to_string(), _app).await
}

#[command]
async fn start_camera_preview_with_quality(device_id: String, quality: String, _app: tauri::AppHandle) -> Result<(), String> {
    println!("[GStreamer] Starting preview for device: {} with quality: {}", device_id, quality);
    
    // Stop any existing camera
    stop_camera_preview().await?;
    
    // Start GStreamer pipeline (runs in background)
    let mut camera_lock = GSTREAMER_CAMERA.write();
    if let Some(camera) = camera_lock.as_mut() {
        camera.start_with_quality(&device_id, &quality)?;
        println!("[GStreamer] ✅ Camera started successfully!");
    } else {
        return Err("GStreamer camera not initialized".to_string());
    }
    drop(camera_lock);
    
    Ok(())
}

#[command]
async fn stop_camera_preview() -> Result<(), String> {
    println!("[GStreamer] Stopping preview");
    
    let mut camera_lock = GSTREAMER_CAMERA.write();
    if let Some(camera) = camera_lock.as_mut() {
        camera.stop()?;
    }
    drop(camera_lock);
    
    Ok(())
}

// ====================
// COMPOSITE PIPELINE COMMANDS (OBS REPLACEMENT)
// ====================

#[command]
async fn start_composite_pipeline(camera_device_id: String, width: u32, height: u32, fps: u32, rotation: u32, has_camera: bool) -> Result<(), String> {
    println!("[Composite] Starting composite pipeline: {}x{} @ {}fps (rotation: {}°)", width, height, fps, rotation);
    println!("[Composite] Main.rs received - camera_device_id: '{}', has_camera: {}", camera_device_id, has_camera);

    // Validate camera device if camera is selected
    if has_camera && !camera_device_id.is_empty() {
        println!("[Composite] 🔍 Validating camera device path...");
        // The device path should be a Windows device path format
        if !camera_device_id.contains("vid_") && !camera_device_id.contains("videotestsrc") {
            println!("[Composite] ⚠️ Warning: Camera device path format looks unusual: {}", camera_device_id);
        }
    }

    // Call the synchronous start method
    let result = {
        let mut composite_lock = GSTREAMER_COMPOSITE.write();
        if let Some(composite) = composite_lock.as_mut() {
            composite.start(&camera_device_id, width, height, fps, rotation, has_camera)
        } else {
            Err("Composite pipeline not initialized. Call initialize_composite_system first.".to_string())
        }
        // Lock is automatically dropped here when the scope ends
    };
    
    match result {
        Ok(_) => {
            println!("[Composite] ✅ Composite pipeline started successfully");
            
            // Give the pipeline a moment to start producing frames
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            
            Ok(())
        }
        Err(e) => {
            println!("[Composite] ❌ Failed to start composite pipeline: {}", e);
            Err(e)
        }
    }
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
    _file_data: Option<Vec<u8>>, // No longer used - kept for API compatibility
    filename: String,
    keycolor: String,
    tolerance: f64,
    similarity: f64,
    use_chroma_key: bool
) -> Result<(), String> {
    println!("[Composite] 🎬 Playing FX: {} (chroma: {})", filename, use_chroma_key);
    
    // Clean filename for caching
    let clean_filename = filename
        .replace("%20", "_")
        .replace("/", "_")
        .replace("\\", "_");
    
    let temp_dir = std::env::temp_dir().join("battles_fx_cache");
    std::fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp directory: {}", e))?;
    
    let local_path = temp_dir.join(&clean_filename);
    
    // Check if already cached
    if !local_path.exists() {
        println!("[Composite] 📥 Downloading FX from Nuxt proxy...");
        
        // Download from Nuxt proxy (handles authentication)
        let full_url = format!("https://local.battles.app:3000{}", file_url);
        
        // Download asynchronously in background
        let local_path_clone = local_path.clone();
        let full_url_clone = full_url.clone();
        let _download_result = tokio::task::spawn_blocking(move || {
            use std::io::Write;
            
            // Use reqwest with danger_accept_invalid_certs for local dev
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
    
    // NOW lock and play (fast, no I/O while locked)
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
    
    Ok(())
}

// OLD NOKHWA CODE - KEEPING FOR REFERENCE (DELETE LATER)
/*
    std::thread::spawn(move || {
        // Force MJPEG formats only (hardware-compressed, no decode overhead)
        // Start with lower resolutions for guaranteed 30fps
        let format_attempts = vec![
            (320, 240, FrameFormat::MJPEG, 30),
            (640, 360, FrameFormat::MJPEG, 30),
            (640, 480, FrameFormat::MJPEG, 30),
            (800, 600, FrameFormat::MJPEG, 30),
            (960, 540, FrameFormat::MJPEG, 30),
            (1280, 720, FrameFormat::MJPEG, 30),
        ];
        
        let mut camera = None;
        let mut used_format = None;
        
        for (width, height, format, fps) in format_attempts.iter() {
            println!("[Camera] Trying {}x{} @ {}fps in {:?}...", width, height, fps, format);
            
            let requested = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(
                CameraFormat::new_from(*width, *height, *format, *fps)
            ));
            
            match NokhwaCamera::new(CameraIndex::Index(index), requested) {
                Ok(mut cam) => {
                    println!("[Camera] Camera created, waiting for initialization...");
                    std::thread::sleep(std::time::Duration::from_millis(500)); // Wait for camera init
                    
                    if cam.open_stream().is_ok() {
                        println!("[Camera] Stream opened, waiting for stabilization...");
                        std::thread::sleep(std::time::Duration::from_millis(1000)); // Wait for stream to stabilize
                        
                        let actual_format = cam.camera_format();
                        let actual_fps = actual_format.frame_rate();
                        
                        println!("[Camera] ✅ Got {}x{} @ {}fps in {:?}", 
                            actual_format.width(), 
                            actual_format.height(), 
                            actual_fps,
                            actual_format.format()
                        );
                        
                        // Accept if we got at least 25fps
                        if actual_fps >= 25 {
                            // Warm up the camera by capturing a few frames
                            println!("[Camera] Warming up camera...");
                            for _ in 0..5 {
                                let _ = cam.frame();
                                std::thread::sleep(std::time::Duration::from_millis(50));
                            }
                            println!("[Camera] Camera ready!");
                            
                            camera = Some(cam);
                            used_format = Some(actual_format);
                            break;
                        } else {
                            println!("[Camera] ❌ Only got {}fps, trying next format...", actual_fps);
                            let _ = cam.stop_stream();
                        }
                    }
                }
                Err(e) => {
                    println!("[Camera] ❌ Failed: {}", e);
                }
            }
        }
        
        let mut camera = match camera {
            Some(cam) => cam,
            None => {
                println!("[Camera] ❌ No suitable format found! Camera may not support 30fps.");
                *CAMERA_RUNNING.write() = false;
                return;
            }
        };
        
        let camera_format = used_format.unwrap();
        println!("[Camera] 🚀 STREAMING at {}x{} @ {}fps", 
            camera_format.width(), 
            camera_format.height(), 
            camera_format.frame_rate()
        );
        
        // Check if camera provides native MJPEG
        let is_mjpeg = matches!(camera_format.format(), FrameFormat::MJPEG);
        println!("[Camera] Using {} encoding", if is_mjpeg { "ZERO-COPY MJPEG" } else { "SOFTWARE JPEG" });
        
        // Real-time streaming loop - FULL SPEED (no artificial limiting)
        let mut frame_count = 0u64;
        let mut last_fps_log = std::time::Instant::now();
        
        loop {
            // Check if still running
            if !*CAMERA_RUNNING.read() {
                println!("[Camera] Stopping camera capture");
                let _ = camera.stop_stream();
                break;
            }
            
            // Capture frame at MAXIMUM SPEED
            if let Ok(frame) = camera.frame() {
                let jpeg_data = if is_mjpeg {
                    // ZERO-COPY: Use camera's native MJPEG buffer
                    frame.buffer().to_vec()
                } else {
                    // SOFTWARE: Decode and re-encode to JPEG
                    if let Ok(img) = frame.decode_image::<RgbFormat>() {
                        let mut jpeg_buffer = Vec::new();
                        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buffer, 70);
                        
                        if encoder.encode(img.as_raw(), img.width(), img.height(), image::ColorType::Rgb8.into()).is_ok() {
                            jpeg_buffer
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    }
                };
                
                // Broadcast binary frame to WebSocket clients
                if let Some(sender) = CAMERA_FRAME_SENDER.read().as_ref() {
                    let _ = sender.send(jpeg_data);
                }
                
                frame_count += 1;
            }
            
            // Log actual FPS every 3 seconds
            if last_fps_log.elapsed().as_secs() >= 3 {
                let actual_fps = frame_count as f64 / last_fps_log.elapsed().as_secs_f64();
                println!("[Camera] 🚀 Streaming at {:.1} fps", actual_fps);
                frame_count = 0;
                last_fps_log = std::time::Instant::now();
            }
            
            // NO SLEEP - Run at maximum hardware speed!
        }
        
        println!("[Camera] Camera thread exited");
    });
    
    // Wait a bit for camera to initialize
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    
    Ok(())
}

async fn stop_camera_preview_OLD() -> Result<(), String> {
    println!("[Camera] Stopping preview");
    
    // Signal camera thread to stop
    *CAMERA_RUNNING.write() = false;
    
    // Clear frame buffer
    *CAMERA_FRAME.write() = None;
    
    // Give camera thread time to cleanup
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    
    println!("[Camera] Preview stopped");
    Ok(())
}
*/

// ========================================
// VIRTUAL CAMERA (DirectShow-compatible)
// ========================================

// Virtual camera state (simple atomic flag)
lazy_static::lazy_static! {
    static ref VIRTUAL_CAM_RUNNING: parking_lot::RwLock<bool> = parking_lot::RwLock::new(false);
}

// Virtual camera configuration
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

// Helper to open/create shared memory (called per operation)
fn get_or_create_shmem() -> Result<Shmem, String> {
    // Calculate shared memory size (RGB24: 3 bytes per pixel + header)
    let frame_size = (VCAM_WIDTH * VCAM_HEIGHT * 3) as usize;
    let header_size = 256; // For metadata (timestamp, frame number, etc.)
    let total_size = header_size + frame_size;
    
    // Try to create or open shared memory
    match ShmemConf::new()
        .size(total_size)
        .os_id(VCAM_SHMEM_NAME)
        .create()
    {
        Ok(m) => Ok(m),
        Err(_) => {
            // Try to open existing
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
    
    // Test creating shared memory
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
    
    // Ensure shared memory exists
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
    // Check if virtual camera is running
    if !*VIRTUAL_CAM_RUNNING.read() {
        return Err("Virtual camera not running".to_string());
    }
    
    // Get shared memory (opens existing)
    let shmem = get_or_create_shmem()?;
    
    // Validate frame size
    let expected_size = (width * height * 3) as usize; // RGB24
    if frame_data.len() != expected_size {
        return Err(format!("Invalid frame size: expected {}, got {}", expected_size, frame_data.len()));
    }
    
    // Write frame to shared memory
    unsafe {
        let ptr = shmem.as_ptr() as *mut u8;
        
        // Write header (256 bytes)
        let timestamp = chrono::Utc::now().timestamp_millis();
        std::ptr::copy_nonoverlapping(&timestamp as *const i64 as *const u8, ptr, 8);
        std::ptr::copy_nonoverlapping(&width as *const u32 as *const u8, ptr.add(8), 4);
        std::ptr::copy_nonoverlapping(&height as *const u32 as *const u8, ptr.add(12), 4);
        
        // Write frame data (after 256-byte header)
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
        cleanup_interval: Some(300), // Clean expired items every 5 minutes
        default_compression: Some(true), // Compress by default (video files are large)
        compression_level: Some(6), // Balanced compression
        compression_threshold: Some(1024 * 100), // Only compress files larger than 100KB
        compression_method: None, // Use default compression method
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