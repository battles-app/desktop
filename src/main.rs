#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use tauri::{command, Manager, Emitter};
use base64::Engine;
use std::sync::{Arc, Mutex};
use serde::{Serialize, Deserialize};

// File logger module
mod file_logger;

// GStreamer composite module (OBS replacement)
mod gstreamer_composite;
use gstreamer_composite::GStreamerComposite;

// WGPU surface renderer (direct window rendering)
mod wgpu_surface_renderer;

// Screen capture for monitor previews
mod screen_capture;
use screen_capture::ScreenCaptureMonitor;

// Stream Deck integration
mod streamdeck_manager;
use streamdeck_manager::{StreamDeckManager, FxButton, STREAMDECK_MANAGER};

mod streamdeck_diagnostics;
use streamdeck_diagnostics::{run_diagnostics, get_driver_download_info, StreamDeckDiagnostics, DriverDownloadInfo};

// DMX Lighting Control
mod dmx_manager;
mod dmx_commands;
use dmx_commands::{
    scan_dmx_devices, connect_dmx_device, disconnect_dmx_device,
    get_dmx_state, send_dmx_data, set_dmx_rgb, set_dmx_dimmer, set_dmx_pan_tilt, set_dmx_complete, dmx_blackout
};

// Media Converter for Automation
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
    // Composite pipeline (OBS replacement)
    static ref GSTREAMER_COMPOSITE: Arc<parking_lot::RwLock<Option<GStreamerComposite>>> = Arc::new(parking_lot::RwLock::new(None));
    static ref COMPOSITE_FRAME_SENDER: Arc<parking_lot::RwLock<Option<broadcast::Sender<Vec<u8>>>>> = Arc::new(parking_lot::RwLock::new(None));
    
    // Latest frame for direct access (no WebSocket overhead)
    static ref LATEST_COMPOSITE_FRAME: Arc<parking_lot::RwLock<Option<Vec<u8>>>> = Arc::new(parking_lot::RwLock::new(None));
    
    // Screen capture monitors (for monitor preview in selection modal)
    static ref SCREEN_CAPTURES: Arc<parking_lot::RwLock<Vec<Option<ScreenCaptureMonitor>>>> = Arc::new(parking_lot::RwLock::new(Vec::new()));
    static ref SCREEN_CAPTURE_SENDERS: Arc<parking_lot::RwLock<Vec<Option<broadcast::Sender<Vec<u8>>>>>> = Arc::new(parking_lot::RwLock::new(Vec::new()));
}

const COMPOSITE_WS_PORT: u16 = 9877;
const SCREEN_CAPTURE_BASE_PORT: u16 = 9880; // 9880, 9881, 9882... for each monitor

// Monitor info structure
#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct MonitorInfo {
    name: Option<String>,
    position: (i32, i32),
    size: (u32, u32),
    scale_factor: f64,
    is_primary: bool,
    screenshot: Option<String>, // Base64 encoded PNG data URL
    #[serde(skip_serializing_if = "Option::is_none")]
    tauri_index: Option<usize>, // Original Tauri enumeration index for preview mapping
}

// Global cache for monitor screenshots
static MONITOR_SCREENSHOTS: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());

// 🔧 FIX: Cache monitor order to ensure consistency between preview ports and indices
// Key: (position_x, position_y, width, height) -> Value: Tauri enumeration index
lazy_static::lazy_static! {
    static ref MONITOR_ORDER_CACHE: Mutex<std::collections::HashMap<(i32, i32, u32, u32), usize>> = Mutex::new(std::collections::HashMap::new());
}

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

    // Update cache size if monitors changed
    {
        let mut cache = MONITOR_SCREENSHOTS.lock().unwrap();
        if cache.len() != monitors.len() {
            cache.resize(monitors.len(), None);
        }
    }

    // 🔧 FIX: Collect monitors and look up their Tauri indices from cache
    let mut monitor_data: Vec<(usize, MonitorInfo)> = monitors
        .into_iter()
        .enumerate()
        .map(|(enum_index, m)| {
            let position = m.position();
            let size = m.size();
            let scale_factor = m.scale_factor();
            let name = m.name().cloned();

            // 🔧 FIX: Look up actual Tauri index from cache (matches start_monitor_previews order)
            let key = (position.x, position.y, size.width, size.height);
            let tauri_index = if let Ok(order_cache) = MONITOR_ORDER_CACHE.lock() {
                order_cache.get(&key).copied().unwrap_or(enum_index)
            } else {
                enum_index
            };

            println!("[Monitor Lookup] Position ({}, {}), Size {}x{} -> Tauri index: {}", 
                     position.x, position.y, size.width, size.height, tauri_index);

            // Get screenshot from cache using Tauri's original index
            let screenshot = {
                if let Ok(cache) = MONITOR_SCREENSHOTS.lock() {
                    if tauri_index < cache.len() {
                        cache[tauri_index].clone()
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            let monitor_info = MonitorInfo {
                name,
                position: (position.x, position.y),
                size: (size.width, size.height),
                scale_factor,
                is_primary: false, // Will be set later
                screenshot,
                tauri_index: Some(tauri_index), // Store cached index for preview mapping
            };

            (tauri_index, monitor_info)
        })
        .collect();

    // 🔧 FIX: Sort monitors by position (left-to-right, top-to-bottom)
    // This matches Windows Display Settings order
    monitor_data.sort_by(|(_, a), (_, b)| {
        // Primary sort by Y position (top monitors first)
        let y_cmp = a.position.1.cmp(&b.position.1);
        if y_cmp != std::cmp::Ordering::Equal {
            return y_cmp;
        }
        // Secondary sort by X position (left monitors first)
        a.position.0.cmp(&b.position.0)
    });

    // 🔧 FIX: Extract sorted monitor infos (now in display order)
    let mut result: Vec<MonitorInfo> = monitor_data.into_iter().map(|(_, info)| info).collect();

    // Find actual primary monitor (the one at 0,0 or closest to 0,0)
    let mut primary_index = 0;
    let mut min_distance = i32::MAX;
    
    for (i, monitor) in result.iter().enumerate() {
        // Calculate distance from origin (0, 0)
        let distance = monitor.position.0.abs() + monitor.position.1.abs();
        if distance < min_distance {
            min_distance = distance;
            primary_index = i;
        }
    }

    // Mark the correct primary monitor
    for (i, monitor) in result.iter_mut().enumerate() {
        monitor.is_primary = i == primary_index;
    }

    result
}

// Get monitor information
#[command]
async fn get_monitors(app: tauri::AppHandle) -> Vec<MonitorInfo> {
    crate::file_logger::log("[Monitors] 📺 get_monitors() called");
    let monitors = read_monitors(&app);
    crate::file_logger::log(&format!("[Monitors]   Found {} monitors", monitors.len()));
    for (i, monitor) in monitors.iter().enumerate() {
        crate::file_logger::log(&format!("[Monitors]   Monitor {}: {}x{} at ({},{}), primary={}, tauri_index={:?}", 
            i, 
            monitor.size.0, monitor.size.1,
            monitor.position.0, monitor.position.1,
            monitor.is_primary,
            monitor.tauri_index
        ));
    }
    monitors
}


// Set modal open state
#[command]
async fn set_modal_open(is_open: bool) {
    if let Ok(mut modal_state) = MODAL_IS_OPEN.lock() {
        *modal_state = is_open;
    }
}

// Start GStreamer screen capture for monitor previews (NEW - replaces screenshots)
#[command]
async fn start_monitor_previews(app: tauri::AppHandle) -> Result<(), String> {
    crate::file_logger::log("[MonitorPreviews] 🎬 start_monitor_previews() called");
    
    // Stop any existing captures first
    stop_monitor_previews().await?;
    
    // Get available monitors
    crate::file_logger::log("[MonitorPreviews]   Fetching available monitors...");
    let monitors = app.available_monitors().map_err(|e| {
        let error_msg = format!("Failed to get monitors: {}", e);
        crate::file_logger::log(&format!("[MonitorPreviews]   ❌ {}", error_msg));
        error_msg
    })?;
    let monitor_count = monitors.len();
    
    crate::file_logger::log(&format!("[MonitorPreviews]   ✅ Found {} monitors", monitor_count));
    
    // 🔧 FIX: Cache monitor order for consistent indexing
    {
        let mut order_cache = MONITOR_ORDER_CACHE.lock().unwrap();
        order_cache.clear();
        for (index, monitor) in monitors.iter().enumerate() {
            let pos = monitor.position();
            let size = monitor.size();
            let key = (pos.x, pos.y, size.width, size.height);
            order_cache.insert(key, index);
            println!("[Monitor Cache] Tauri index {}: position ({}, {}), size {}x{}", index, pos.x, pos.y, size.width, size.height);
        }
    }
    
    // Initialize storage
    {
        let mut captures = SCREEN_CAPTURES.write();
        let mut senders = SCREEN_CAPTURE_SENDERS.write();
        captures.clear();
        senders.clear();
        // Pre-allocate with None values
        for _ in 0..monitor_count {
            captures.push(None);
            senders.push(None);
        }
    }
    
    // Start capture for each monitor
    for (index, monitor) in monitors.iter().enumerate() {
        let _position = monitor.position();
        let _size = monitor.size();
        
        // Starting capture for monitor {index}
        
        // Create screen capture
        let capture = ScreenCaptureMonitor::new(index)
            .map_err(|e| format!("Failed to create capture {}: {}", index, e))?;
        
        // Create broadcast channel for this monitor with larger buffer
        // 60 frames = 2 seconds of buffer at 30fps (prevents lag spikes during preview)
        let (tx, _rx) = broadcast::channel::<Vec<u8>>(60);
        capture.set_frame_sender(tx.clone());
        
        // Store in global state BEFORE starting capture or WebSocket
        {
            let mut captures = SCREEN_CAPTURES.write();
            let mut senders = SCREEN_CAPTURE_SENDERS.write();
            captures[index] = Some(capture);
            senders[index] = Some(tx.clone());
        }
        
        // Start WebSocket server FIRST (so clients can connect)
        let port = SCREEN_CAPTURE_BASE_PORT + index as u16;
        start_monitor_preview_websocket(index, port).await;
    }
    
    // Small delay to allow WebSocket clients to connect before frames start arriving
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // NOW start all capture pipelines
    for (index, monitor) in monitors.iter().enumerate() {
        let position = monitor.position();
        let size = monitor.size();
        
        let mut captures = SCREEN_CAPTURES.write();
        if let Some(capture) = captures[index].as_mut() {
            capture.start(position.x, position.y, size.width, size.height)
                .map_err(|e| format!("Failed to start capture {}: {}", index, e))?;
        }
    }
    
    Ok(())
}

// Stop all monitor preview captures
#[command]
async fn stop_monitor_previews() -> Result<(), String> {
    crate::file_logger::log("[MonitorPreviews] 🛑 stop_monitor_previews() called");
    
    let mut captures = SCREEN_CAPTURES.write();
    let mut senders = SCREEN_CAPTURE_SENDERS.write();
    
    let capture_count = captures.iter().filter(|c| c.is_some()).count();
    crate::file_logger::log(&format!("[MonitorPreviews]   Stopping {} active captures...", capture_count));
    
    // Stop all captures
    for (index, capture_opt) in captures.iter_mut().enumerate() {
        if let Some(mut capture) = capture_opt.take() {
            crate::file_logger::log(&format!("[MonitorPreviews]   Stopping capture {}...", index));
            capture.stop().map_err(|e| {
                let error_msg = format!("Failed to stop capture {}: {}", index, e);
                crate::file_logger::log(&format!("[MonitorPreviews]   ❌ {}", error_msg));
                error_msg
            })?;
            crate::file_logger::log(&format!("[MonitorPreviews]   ✅ Capture {} stopped", index));
        }
    }
    
    captures.clear();
    senders.clear();
    
    crate::file_logger::log("[MonitorPreviews] ✅ All monitor previews stopped");
    
    Ok(())
}

// WebSocket server for monitor preview streaming
async fn start_monitor_preview_websocket(monitor_index: usize, port: u16) {
    tokio::spawn(async move {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[Monitor Preview {}] Failed to bind to {}: {}", monitor_index, addr, e);
                return;
            }
        };
        
        while let Ok((stream, _)) = listener.accept().await {
            // Get sender for this monitor
            let tx = {
                let senders = SCREEN_CAPTURE_SENDERS.read();
                match &senders.get(monitor_index) {
                    Some(Some(sender)) => sender.clone(),
                    _ => continue,
                }
            };
            
            tokio::spawn(async move {
                let ws_stream = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(_) => return,
                };
                
                use tokio_tungstenite::tungstenite::protocol::Message;
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();
                
                // Subscribe to frames
                let mut rx = tx.subscribe();
                let mut _frame_count = 0u64;
                // Lower FPS for monitor previews (they're just thumbnails)
                let target_fps = 15.0;
                let frame_interval = std::time::Duration::from_secs_f64(1.0 / target_fps);
                let mut last_send_time = std::time::Instant::now();
                
                loop {
                    tokio::select! {
                        recv_result = rx.recv() => {
                            match recv_result {
                                Ok(frame_data) => {
                                    // Frame rate limiting: Only send at 15fps for previews
                                    let now = std::time::Instant::now();
                                    let elapsed = now.duration_since(last_send_time);
                                    
                                    if elapsed >= frame_interval {
                                        _frame_count += 1;
                                        last_send_time = now;
                                        
                                        // Send frame
                                        if ws_sender.send(Message::Binary(frame_data)).await.is_err() {
                                            break;
                                        }
                                    }
                                    // Else: Drop frame silently (preview doesn't need full framerate)
                                },
                                Err(broadcast::error::RecvError::Lagged(_skipped)) => {
                                    // Should rarely happen with 60-frame buffer + rate limiting
                                    continue;
                                },
                                Err(broadcast::error::RecvError::Closed) => {
                                    break;
                                }
                            }
                        }
                        ws_msg = ws_receiver.next() => {
                            if ws_msg.is_none() {
                                break;
                            }
                        }
                    }
                }
            });
        }
    });
}

// Start real-time monitor capture (called when modal opens)
#[command]
async fn start_realtime_capture(app: tauri::AppHandle) -> Result<(), String> {

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
    let window = app.get_webview_window("tv-monitor");

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
    crate::file_logger::log("[TV Monitor] 🔴 close_tv_monitor_window() called");
    
    if let Some(window) = app.get_webview_window("tv-monitor") {
        crate::file_logger::log("[TV Monitor]   ✅ Found TV monitor window, attempting to close...");
        
        // Try close first
        match window.close() {
            Ok(_) => {
                crate::file_logger::log("[TV Monitor]   ✅ Window closed successfully");
            }
            Err(e) => {
                crate::file_logger::log(&format!("[TV Monitor]   ⚠️ Close failed: {}, trying destroy...", e));
                // If close fails, force destroy
                window.destroy().map_err(|e2| {
                    let error_msg = format!("Failed to destroy window: {}", e2);
                    crate::file_logger::log(&format!("[TV Monitor]   ❌ {}", error_msg));
                    error_msg
                })?;
                crate::file_logger::log("[TV Monitor]   ✅ Window destroyed successfully");
            }
        }
        
        // Wait a bit to ensure window is fully destroyed
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        
        // Verify window is gone
        if app.get_webview_window("tv-monitor").is_some() {
            let error_msg = "Window still exists after close/destroy attempt".to_string();
            crate::file_logger::log(&format!("[TV Monitor]   ❌ {}", error_msg));
            return Err(error_msg);
        }
        
        crate::file_logger::log("[TV Monitor]   ✅ Verified: Window successfully removed");
    } else {
        crate::file_logger::log("[TV Monitor]   ℹ️ No TV monitor window found (already closed?)");
    }
    
    Ok(())
}

// Create a window on a specific monitor
#[command]
async fn create_monitor_window(
    app: tauri::AppHandle,
    url: String,
    _monitor_index: usize,
    monitor_position: (i32, i32),  // Pass position from frontend to match preview
    monitor_size: (u32, u32)       // Pass size from frontend to match preview
) -> Result<(), String> {
    crate::file_logger::log("[TV Monitor] 📺 create_monitor_window() called");
    crate::file_logger::log(&format!("[TV Monitor]   URL: {}", url));
    crate::file_logger::log(&format!("[TV Monitor]   Target position: ({}, {})", monitor_position.0, monitor_position.1));
    crate::file_logger::log(&format!("[TV Monitor]   Target size: {}x{}", monitor_size.0, monitor_size.1));
    
    // Get Tauri's native monitor info to find the matching monitor
    crate::file_logger::log("[TV Monitor]   Fetching available monitors...");
    let native_monitors = app.available_monitors().unwrap_or_default();
    crate::file_logger::log(&format!("[TV Monitor]   Found {} native monitors", native_monitors.len()));
    
    for (i, m) in native_monitors.iter().enumerate() {
        let pos = m.position();
        let size = m.size();
        crate::file_logger::log(&format!("[TV Monitor]     Monitor {}: {}x{} at ({}, {}), scale={}", 
            i, size.width, size.height, pos.x, pos.y, m.scale_factor()));
    }

    // Find the monitor that matches the position and size from frontend
    crate::file_logger::log("[TV Monitor]   Searching for matching monitor...");
    let native_monitor = native_monitors.iter()
        .find(|m| {
            let pos = m.position();
            let size = m.size();
            pos.x == monitor_position.0 && 
            pos.y == monitor_position.1 && 
            size.width == monitor_size.0 && 
            size.height == monitor_size.1
        })
        .ok_or_else(|| {
            let error_msg = format!(
                "No monitor found matching position ({}, {}) and size {}x{}",
                monitor_position.0, monitor_position.1, monitor_size.0, monitor_size.1
            );
            crate::file_logger::log(&format!("[TV Monitor]   ❌ {}", error_msg));
            error_msg
        })?;

    let monitor_pos = native_monitor.position();
    let monitor_size_actual = native_monitor.size();
    let scale_factor = native_monitor.scale_factor();
    
    crate::file_logger::log(&format!("[TV Monitor]   ✅ Found matching monitor: {}x{} at ({}, {})", 
        monitor_size_actual.width, monitor_size_actual.height, monitor_pos.x, monitor_pos.y));

    // Use PHYSICAL pixels for BOTH size and position to ensure correct fullscreen on high-DPI monitors
    // Tauri's position() expects physical pixels, and inner_size() can also accept physical
    let physical_width = monitor_size_actual.width as f64;
    let physical_height = monitor_size_actual.height as f64;
    let physical_x = monitor_pos.x as f64;
    let physical_y = monitor_pos.y as f64;
    
    crate::file_logger::log(&format!("[TV Monitor]   Physical position: ({}, {})", physical_x, physical_y));
    crate::file_logger::log(&format!("[TV Monitor]   Physical size: {}x{}", physical_width, physical_height));
    crate::file_logger::log(&format!("[TV Monitor]   Scale factor: {}", scale_factor));

    // Close any existing TV monitor window first (Tauri v2 API)
    if let Some(existing_window) = app.get_webview_window("tv-monitor") {
        // Try to close the window first (more graceful than destroy)
        let _ = existing_window.close();
        
        // Wait longer to ensure window is fully closed/destroyed
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        // Check if window still exists, if so, force destroy
        if let Some(still_exists) = app.get_webview_window("tv-monitor") {
            still_exists.destroy().map_err(|e| format!("Cannot destroy window: {}", e))?;
            // Extra wait after forced destroy
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        
        // Final verification - window MUST be gone before proceeding
        if app.get_webview_window("tv-monitor").is_some() {
            return Err("Window still exists after multiple destroy attempts".to_string());
        }
    }

    // Parse URL safely
    let parsed_url = url.parse()
        .map_err(|e| format!("Failed to parse URL '{}': {}", url, e))?;
    
    // 🔥 EMERGENCY FIX: Build window HIDDEN first, then show after it's ready
    // This prevents the UI from freezing while the page loads
    crate::file_logger::log("[TV Monitor]   Building window (hidden initially)...");
    
    let window = tauri::webview::WebviewWindowBuilder::new(
        &app, 
        "tv-monitor", 
        tauri::WebviewUrl::External(parsed_url)
    )
        .title("TV Monitor - Battles.app")
        .inner_size(physical_width, physical_height)
        .position(physical_x, physical_y)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .visible(false)  // 🔥 Start HIDDEN to prevent freeze!
        .fullscreen(false)
        .skip_taskbar(false)
        .transparent(false)
        .shadow(false)
        .build()
        .map_err(|e| {
            let error_msg = format!("Failed to build monitor window: {}", e);
            crate::file_logger::log(&format!("[TV Monitor]   ❌ {}", error_msg));
            error_msg
        })?;
    
    crate::file_logger::log("[TV Monitor]   ✅ Window built successfully (hidden)");
    
    // Show window in a separate task to avoid blocking
    let window_clone = window.clone();
    tauri::async_runtime::spawn(async move {
        // Wait a moment for webview to initialize
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        crate::file_logger::log("[TV Monitor]   📺 Showing window now...");
        
        // Show the window
        let _ = window_clone.show();
        let _ = window_clone.set_focus();
        
        crate::file_logger::log("[TV Monitor]   ✅ Window shown");
    });
    
    // CRITICAL: Wait for window to be registered in Tauri's window manager
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // Verify the window was actually created and registered
    if app.get_webview_window("tv-monitor").is_none() {
        return Err("Window was built but not found in app.get_webview_window()".to_string());
    }

    // Listen for window close events to notify the main window
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            // Emit event to notify main window that TV monitor was closed
            let _ = app_handle.emit("tv-monitor-closed", ());
        }
    });

    // 🔥 CRITICAL: Return immediately! Window will show itself asynchronously
    // This prevents the entire app from freezing while waiting for the webview to load
    crate::file_logger::log("[TV Monitor]   ✅ Returning (window will show in background)");
    Ok(())
}

// Create regular window (1080x640, resizable, movable, center-right position)
#[command]
async fn create_regular_window(app: tauri::AppHandle, url: String) -> Result<(), String> {
    // Close any existing TV monitor window first (Tauri v2 API)
    if let Some(existing_window) = app.get_webview_window("tv-monitor") {
        let _ = existing_window.close();
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        if let Some(still_exists) = app.get_webview_window("tv-monitor") {
            let _ = still_exists.destroy();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
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
        .transparent(false)  // 🚀 NO transparency = massive GPU savings
        .shadow(true)        // Keep shadow for visual separation
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
    Err("Camera system has been removed".to_string())
}

// Camera WebSocket server removed (camera system deleted)

// Initialize composite system
#[command]
async fn initialize_composite_system() -> Result<String, String> {
    println!("[Composite] Initializing composite system");
    
    // Only initialize once - check if already done
    {
        let sender_lock = COMPOSITE_FRAME_SENDER.read();
        if sender_lock.is_some() {
            println!("[Composite] Already initialized");
            return Ok("Composite system already initialized".to_string());
        }
    } // Release lock before async operations
    
    // Initialize composite pipeline
    let composite = GStreamerComposite::new()
        .map_err(|e| format!("Failed to initialize composite: {}", e))?;
    
    *GSTREAMER_COMPOSITE.write() = Some(composite);
    
    // Create broadcast channel for composite frames with larger buffer
    // 60 frames = 2 seconds of buffer at 30fps (prevents lag spikes)
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(60);
    
    // Set frame sender in composite (for FX only)
    if let Some(comp) = GSTREAMER_COMPOSITE.read().as_ref() {
        comp.set_frame_sender(tx.clone());
    }
    
    // Set sender before starting WebSocket to prevent multiple initializations
    *COMPOSITE_FRAME_SENDER.write() = Some(tx);
    
    // Start WebSocket server for frame delivery
    start_composite_websocket_server().await;

    println!("[Composite] ✅ Composite system initialized");
    println!("[Composite] 💡 Using optimized async readback (~60ms latency)");
    Ok("Composite initialized".to_string())
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
            // Clone the sender before spawning (keeps it alive)
            let tx = match COMPOSITE_FRAME_SENDER.read().as_ref() {
                Some(sender) => sender.clone(),
                None => {
                    println!("[Composite WS] ❌ No frame sender available");
                    continue;
                }
            };
            
            tokio::spawn(async move {
                let ws_stream = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        println!("[Composite WS] Error during handshake: {}", e);
                        return;
                    }
                };
                
                println!("[Composite WS] ✅ Client connected");
                
                use futures_util::{SinkExt, StreamExt};
                use tokio_tungstenite::tungstenite::protocol::Message;
                
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();
                
                // Subscribe to composite frames (receiver stays alive for entire connection)
                let mut rx = tx.subscribe();
                let mut frame_count = 0u64;
                let target_fps = 30.0;
                let frame_interval = std::time::Duration::from_secs_f64(1.0 / target_fps);
                let mut last_send_time = std::time::Instant::now();

                loop {
                    // Non-blocking check for client messages (for graceful disconnect)
                    tokio::select! {
                        // Receive frames from broadcast channel
                        recv_result = rx.recv() => {
                            match recv_result {
                                Ok(frame_data) => {
                                    // Frame rate limiting: Only send at 30fps max
                                    let now = std::time::Instant::now();
                                    let elapsed = now.duration_since(last_send_time);
                                    
                                    if elapsed >= frame_interval {
                                        frame_count += 1;
                                        last_send_time = now;
                                        
                                        // Send frame (WebSocket handles backpressure)
                                        if ws_sender.send(Message::Binary(frame_data)).await.is_err() {
                                            println!("[Composite WS] ❌ Client disconnected after {} frames", frame_count);
                                            break;
                                        }
                                    }
                                    // Else: Drop frame silently (frontend can't handle more than 30fps)
                                },
                                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                    // This should rarely happen now with 60-frame buffer + rate limiting
                                    if skipped > 10 {
                                        println!("[Composite WS] ⚠️ Severe lag: skipped {} frames (check system resources)", skipped);
                                    }
                                    continue;
                                },
                                Err(broadcast::error::RecvError::Closed) => {
                                    println!("[Composite WS] ℹ️ Broadcast channel closed");
                                    break;
                                }
                            }
                        }
                        // Check for client disconnect
                        ws_msg = ws_receiver.next() => {
                            if ws_msg.is_none() {
                                println!("[Composite WS] 🔌 Client disconnected gracefully");
                                break;
                            }
                        }
                    }
                }
                
                println!("[Composite WS] 🔌 Client disconnected (sent {} frames)", frame_count);
            });
        }
    });
}

#[command]
async fn get_available_cameras() -> Result<Vec<CameraDeviceInfo>, String> {
    Ok(vec![]) // Camera system removed
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
async fn start_camera_preview(_device_id: String, _app: tauri::AppHandle) -> Result<(), String> {
    Err("Camera system removed".to_string())
}

#[command]
async fn start_camera_preview_with_quality(_device_id: String, _quality: String, _app: tauri::AppHandle) -> Result<(), String> {
    Err("Camera system removed".to_string())
}

#[command]
async fn stop_camera_preview() -> Result<(), String> {
    Ok(()) // No-op - camera system removed
}

// ====================
// COMPOSITE PIPELINE COMMANDS (OBS REPLACEMENT)
// ====================

#[command]
async fn start_composite_pipeline(width: u32, height: u32) -> Result<(), String> {
    println!("[Composite] Starting composite pipeline: {}x{}", width, height);

    // Call the synchronous start method
    let result = {
        let mut composite_lock = GSTREAMER_COMPOSITE.write();
        if let Some(composite) = composite_lock.as_mut() {
            composite.start(width, height)
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
async fn update_composite_layers(overlay: (bool, f64)) -> Result<(), String> {
    let composite_lock = GSTREAMER_COMPOSITE.read();
    if let Some(composite) = composite_lock.as_ref() {
        composite.update_layers(overlay);
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
        let full_url = format!("https://battles.app{}", file_url);
        
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

#[command]
async fn download_and_cache_video_loop(
    file_url: String,
    filename: String,
    file_type: String // "video" or "thumbnail"
) -> Result<String, String> {
    println!("[VideoLoop] 📥 Caching {} ({})...", filename, file_type);
    
    // Clean filename for caching
    let clean_filename = filename
        .replace("%20", "_")
        .replace("/", "_")
        .replace("\\", "_");
    
    // Create separate directories for videos and thumbnails
    let cache_subdir = if file_type == "thumbnail" {
        "battles_loop_thumbnails"
    } else {
        "battles_loop_videos"
    };
    
    let cache_dir = std::env::temp_dir().join(cache_subdir);
    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("Failed to create cache directory: {}", e))?;
    
    let local_path = cache_dir.join(&clean_filename);
    
    // Check if already cached
    if local_path.exists() {
        println!("[VideoLoop] ⚡ Using existing cache for {}", clean_filename);
        return Ok(local_path.to_string_lossy().to_string());
    }
    
    // Download from Directus (full URL)
    println!("[VideoLoop] 📥 Downloading from: {}", file_url);
    
    let local_path_clone = local_path.clone();
    let file_url_clone = file_url.clone();
    
    tokio::task::spawn_blocking(move || {
        use std::io::Write;
        
        let client = reqwest::blocking::Client::builder()
            .danger_accept_invalid_certs(true)
            .timeout(std::time::Duration::from_secs(60)) // Longer timeout for videos
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;
        
        let response = client
            .get(&file_url_clone)
            .send()
            .map_err(|e| format!("Failed to download: {}", e))?;
        
        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()));
        }
        
        let bytes = response.bytes()
            .map_err(|e| format!("Failed to read response: {}", e))?;
        
        println!("[VideoLoop] 💾 Writing {} bytes to cache...", bytes.len());
        let mut file = std::fs::File::create(&local_path_clone)
            .map_err(|e| format!("Failed to create file: {}", e))?;
        file.write_all(&bytes)
            .map_err(|e| format!("Failed to write file: {}", e))?;
        
        Ok::<(), String>(())
    }).await.map_err(|e| format!("Download task failed: {}", e))??;
    
    println!("[VideoLoop] ✅ Cached {} successfully", clean_filename);
    Ok(local_path.to_string_lossy().to_string())
}

#[command]
async fn get_cached_video_loop_path(
    filename: String,
    file_type: String
) -> Result<Option<String>, String> {
    let clean_filename = filename
        .replace("%20", "_")
        .replace("/", "_")
        .replace("\\", "_");
    
    let cache_subdir = if file_type == "thumbnail" {
        "battles_loop_thumbnails"
    } else {
        "battles_loop_videos"
    };
    
    let cache_dir = std::env::temp_dir().join(cache_subdir);
    let local_path = cache_dir.join(&clean_filename);
    
    if local_path.exists() {
        Ok(Some(local_path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

#[command]
async fn clear_video_loop_cache() -> Result<(), String> {
    println!("[VideoLoop] 🧹 Clearing video loop cache...");
    
    let video_cache = std::env::temp_dir().join("battles_loop_videos");
    let thumb_cache = std::env::temp_dir().join("battles_loop_thumbnails");
    
    if video_cache.exists() {
        std::fs::remove_dir_all(&video_cache)
            .map_err(|e| format!("Failed to clear video cache: {}", e))?;
    }
    
    if thumb_cache.exists() {
        std::fs::remove_dir_all(&thumb_cache)
            .map_err(|e| format!("Failed to clear thumbnail cache: {}", e))?;
    }
    
    println!("[VideoLoop] ✅ Cache cleared");
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
                
                // Camera broadcast removed
                
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

// ========================================
// STREAM DECK COMMANDS
// ========================================

#[derive(serde::Serialize, serde::Deserialize)]
struct StreamDeckInfo {
    connected: bool,
    device_name: String,
    button_count: usize,
    serial_number: Option<String>,
}

#[command]
async fn streamdeck_init(app: tauri::AppHandle) -> Result<(), String> {
    crate::file_logger::log("[StreamDeck] streamdeck_init() called from frontend");
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    // Check if already initialized
    if let Some(ref manager) = *manager_lock {
        let is_connected = manager.is_connected();
        let button_count = manager.get_loaded_button_count();
        crate::file_logger::log(&format!("[StreamDeck] Already initialized (connected={}, buttons={}), skipping (idempotent)", 
            is_connected, button_count));
        
        // Re-emit connected event if connected to help frontend catch up
        if is_connected {
            crate::file_logger::log("[StreamDeck] Re-emitting streamdeck://connected");
            let _ = app.emit("streamdeck://connected", ());
        }
        
        return Ok(());
    }
    
    // Not initialized yet - create new manager
    crate::file_logger::log("[StreamDeck] Creating new StreamDeck manager (first init)");
    let manager = StreamDeckManager::new()?;
    *manager_lock = Some(manager);
    crate::file_logger::log("[StreamDeck] StreamDeck manager created successfully");
    
    Ok(())
}

#[command]
async fn streamdeck_scan() -> Result<Vec<String>, String> {
    println!("[Stream Deck] Scanning for devices...");
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        let devices = manager.scan_devices()?;
        let device_names: Vec<String> = devices
            .iter()
            .map(|(kind, serial)| format!("{:?} ({})", kind, serial))
            .collect();
        
        println!("[Stream Deck] Found {} devices", device_names.len());
        Ok(device_names)
    } else {
        Err("Stream Deck not initialized".to_string())
    }
}

#[command]
async fn streamdeck_connect() -> Result<String, String> {
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        let result = manager.connect()?;
        Ok(result)
    } else {
        Err("Stream Deck not initialized".to_string())
    }
}

#[command]
async fn streamdeck_disconnect() -> Result<(), String> {
    println!("[Stream Deck] Disconnecting...");
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        manager.disconnect();
        println!("[Stream Deck] ✅ Disconnected");
    }
    
    Ok(())
}

#[command]
async fn streamdeck_get_info() -> Result<StreamDeckInfo, String> {
    let manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref manager) = *manager_lock {
        let serial = manager.get_serial_number().ok();
        
        Ok(StreamDeckInfo {
            connected: manager.is_connected(),
            device_name: manager.device_kind_name(),
            button_count: manager.button_count(),
            serial_number: serial,
        })
    } else {
        Ok(StreamDeckInfo {
            connected: false,
            device_name: "Not initialized".to_string(),
            button_count: 0,
            serial_number: None,
        })
    }
}

#[command]
async fn streamdeck_update_layout(
    battle_board: Vec<FxButton>,
    user_fx: Vec<FxButton>
) -> Result<(), String> {
    crate::file_logger::log(&format!("[StreamDeck] 🎨 Update layout called - Battle Board: {} buttons, User FX: {} buttons", 
        battle_board.len(), user_fx.len()));
    
    // Log button names for debugging
    if !battle_board.is_empty() {
        let names: Vec<String> = battle_board.iter().take(3).map(|b| b.name.clone()).collect();
        crate::file_logger::log(&format!("[StreamDeck]    Battle Board: {:?}...", names));
    }
    if !user_fx.is_empty() {
        let names: Vec<String> = user_fx.iter().take(3).map(|b| b.name.clone()).collect();
        crate::file_logger::log(&format!("[StreamDeck]    User FX: {:?}...", names));
    }
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        match manager.update_layout(battle_board.clone(), user_fx.clone()) {
            Ok(_) => {
                // Check actual animation state after update
                let anim_active = manager.is_loading_animation_active();
                let total_buttons = battle_board.len() + user_fx.len();
                
                if total_buttons == 0 {
                    crate::file_logger::log("[StreamDeck] ✅ Empty layout update - Animation still playing");
                } else if anim_active {
                    crate::file_logger::log(&format!("[StreamDeck] ⚠️ Layout updated but animation STILL ACTIVE (bug!)"));
                } else {
                    crate::file_logger::log(&format!("[StreamDeck] ✅ Layout updated - Animation STOPPED, {} buttons mapped", total_buttons));
                }
                Ok(())
            }
            Err(e) => {
                crate::file_logger::log(&format!("[StreamDeck] ❌ Layout update failed: {}", e));
                Err(e)
            }
        }
    } else {
        crate::file_logger::log("[StreamDeck] ❌ Not initialized");
        Err("Stream Deck not initialized".to_string())
    }
}

// NEW: Sync mappings from database to StreamDeck device
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ButtonMappingData {
    x: i32,
    y: i32,
    button_index: usize,
    item_type: String,
    item_id: String,
    item_name: String,
    image_url: String,
    config: serde_json::Value,
}

#[command]
async fn streamdeck_sync_mappings(
    device_name: String,
    grid_cols: usize,
    grid_rows: usize,
    button_mappings: Vec<ButtonMappingData>
) -> Result<(), String> {
    crate::file_logger::log(&format!(
        "[StreamDeck] 📡 Sync mappings called - Device: {}, Grid: {}x{}, Buttons: {}", 
        device_name, grid_cols, grid_rows, button_mappings.len()
    ));
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        // Convert button mappings to FxButton format for rendering
        let mut fx_buttons: Vec<(usize, FxButton)> = Vec::new();
        
        for mapping in button_mappings {
            let button_index = (mapping.y * grid_cols as i32 + mapping.x) as usize;
            
            fx_buttons.push((
                button_index,
                FxButton {
                    id: mapping.item_id,
                    name: mapping.item_name,
                    image_url: if mapping.image_url.is_empty() { None } else { Some(mapping.image_url) },
                    is_global: mapping.item_type == "battle_fx",
                    position: button_index,
                    item_type: mapping.item_type.clone(),
                }
            ));
        }
        
        crate::file_logger::log(&format!("[StreamDeck] 📍 Button positions: {:?}", 
            fx_buttons.iter().map(|(idx, btn)| format!("{}:{}", idx, btn.name)).collect::<Vec<_>>()));
        
        // Render buttons using the sync_mappings method
        manager.sync_mappings(grid_cols, grid_rows, fx_buttons)?;
        
        crate::file_logger::log("[StreamDeck] ✅ Mappings synced successfully");
        Ok(())
    } else {
        crate::file_logger::log("[StreamDeck] ⚠️ Manager not initialized");
        Err("Manager not initialized".to_string())
    }
}

#[command]
async fn streamdeck_set_button_state(fx_id: String, is_playing: bool) -> Result<(), String> {
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        manager.set_button_state(&fx_id, is_playing)?;
    }
    
    Ok(())
}

#[command]
async fn streamdeck_flush_updates() -> Result<(), String> {
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        manager.flush_updates()?;
    }
    
    Ok(())
}

#[command]
async fn streamdeck_run_diagnostics() -> StreamDeckDiagnostics {
    println!("[Stream Deck] Running diagnostics...");
    let diagnostics = run_diagnostics();
    
    println!("[Stream Deck] Diagnostics Results:");
    println!("  HidAPI Initialized: {}", diagnostics.hidapi_initialized);
    println!("  Devices Found: {}", diagnostics.devices_found);
    
    for (i, device) in diagnostics.device_details.iter().enumerate() {
        println!("  Device {}: {} (VID: 0x{:04x}, PID: 0x{:04x}, Serial: {})",
            i + 1, device.kind, device.vendor_id, device.product_id, device.serial);
    }
    
    println!("  Recommendations:");
    for rec in &diagnostics.recommendations {
        println!("    {}", rec);
    }
    
    diagnostics
}

#[command]
async fn streamdeck_get_driver_info() -> DriverDownloadInfo {
    get_driver_download_info()
}

// Start Stream Deck watcher thread
fn start_streamdeck_watcher(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        // Run diagnostics on startup
        println!("[Stream Deck Watcher] Running initial diagnostics...");
        let initial_diagnostics = run_diagnostics();
        
        println!("[Stream Deck Watcher] === DIAGNOSTIC RESULTS ===");
        println!("[Stream Deck Watcher] HID API Initialized: {}", initial_diagnostics.hidapi_initialized);
        println!("[Stream Deck Watcher] Devices Found: {}", initial_diagnostics.devices_found);
        
        if !initial_diagnostics.device_details.is_empty() {
            println!("[Stream Deck Watcher] Detected Devices:");
            for (i, device) in initial_diagnostics.device_details.iter().enumerate() {
                println!("[Stream Deck Watcher]   {}. {} (VID: 0x{:04x}, PID: 0x{:04x})",
                    i + 1, device.kind, device.vendor_id, device.product_id);
                println!("[Stream Deck Watcher]      Serial: {}", device.serial);
            }
        }
        
        if !initial_diagnostics.recommendations.is_empty() {
            println!("[Stream Deck Watcher] Recommendations:");
            for rec in &initial_diagnostics.recommendations {
                println!("[Stream Deck Watcher]   {}", rec);
            }
        }
        println!("[Stream Deck Watcher] === END DIAGNOSTICS ===");
        
        // Try to connect immediately if device was found
        let devices_found = initial_diagnostics.devices_found;
        
        // Emit diagnostics to frontend
        let _ = app.emit("streamdeck://diagnostics", initial_diagnostics);
        
        if devices_found > 0 {
            let mut manager_lock = STREAMDECK_MANAGER.lock();
            if let Some(ref mut manager) = *manager_lock {
                println!("[Stream Deck Watcher] Attempting initial connection...");
                match manager.connect() {
                    Ok(info) => {
                        println!("[Stream Deck Watcher] ✅ Initial connection successful: {}", info);
                        let _ = app.emit("streamdeck://connected", ());
                    }
                    Err(e) => {
                        println!("[Stream Deck Watcher] ❌ Initial connection failed: {}", e);
                    }
                }
            }
        }
        
        let mut connection_check_interval = tokio::time::interval(std::time::Duration::from_secs(2));
        let mut button_check_interval = tokio::time::interval(std::time::Duration::from_millis(1)); // 1000 FPS - INSTANT button detection
        let mut animation_check_interval = tokio::time::interval(std::time::Duration::from_millis(33)); // ~30 FPS - smooth animation without killing performance
        let mut was_connected = false;
        let mut animation_frame: usize = 0;
        let mut last_animation_state = false;
        let mut last_animation_log_frame: usize = 0;
        
        loop {
            tokio::select! {
                _ = animation_check_interval.tick() => {
                    // Check if animation should be playing
                    let should_animate = {
                        if let Some(manager_lock) = STREAMDECK_MANAGER.try_lock() {
                            manager_lock.as_ref().map(|m| m.is_loading_animation_active()).unwrap_or(false)
                        } else {
                            false
                        }
                    };
                    
                    // Log animation state changes
                    if should_animate != last_animation_state {
                        crate::file_logger::log(&format!("[Watcher] Animation state changed: {} -> {} (frame {})", 
                            last_animation_state, should_animate, animation_frame));
                        last_animation_state = should_animate;
                    }
                    
                    // Log periodically when animation is active (every 1000 frames = ~6 seconds)
                    if should_animate && animation_frame > 0 && animation_frame % 1000 == 0 && animation_frame != last_animation_log_frame {
                        crate::file_logger::log(&format!("[Watcher] Animation still active at frame {}", animation_frame));
                        last_animation_log_frame = animation_frame;
                    }
                    
                    // Play next animation frame if active
                    if should_animate {
                        if let Some(mut manager_lock) = STREAMDECK_MANAGER.try_lock() {
                            if let Some(ref mut manager) = *manager_lock {
                                let _ = manager.continue_loading_background(animation_frame);
                                animation_frame = animation_frame.wrapping_add(1);
                            }
                        }
                    } else {
                        // Reset frame counter when animation stops
                        if animation_frame > 0 {
                            animation_frame = 0;
                        }
                    }
                }
                _ = connection_check_interval.tick() => {
                    // Check if device is connected
                    let is_connected = {
                        let manager_lock = STREAMDECK_MANAGER.lock();
                        manager_lock.as_ref().map(|m| m.is_connected()).unwrap_or(false)
                    };
                    
                    // If connection state changed, notify frontend
                    if is_connected != was_connected {
                        was_connected = is_connected;
                        
                        if is_connected {
                            let _ = app.emit("streamdeck://connected", ());
                        } else {
                            println!("[Stream Deck Watcher] Device disconnected");
                            let _ = app.emit("streamdeck://disconnected", ());
                            
                            // Try to reconnect
                            let mut manager_lock = STREAMDECK_MANAGER.lock();
                            if let Some(ref mut manager) = *manager_lock {
                                crate::file_logger::log("[Stream Deck Watcher] Attempting to reconnect...");
                                if let Ok(info) = manager.connect() {
                                    crate::file_logger::log(&format!("[Stream Deck Watcher] ✅ Reconnected: {}", info));
                                    let _ = app.emit("streamdeck://connected", ());
                                } else {
                                    crate::file_logger::log("[Stream Deck Watcher] ❌ Reconnection failed");
                                }
                            }
                        }
                    }
                }
                _ = button_check_interval.tick() => {
                    // Check if device is connected (quick check without lock)
                    let is_connected = {
                        if let Some(manager_lock) = STREAMDECK_MANAGER.try_lock() {
                            manager_lock.as_ref().map(|m| m.is_connected()).unwrap_or(false)
                        } else {
                            // If we can't get the lock, assume connected and try button read anyway
                            true
                        }
                    };
                    
                    // Read button presses if connected
                    if is_connected {
                        // Use try_lock to avoid blocking if layout update is happening
                        if let Some(mut manager_lock) = STREAMDECK_MANAGER.try_lock() {
                            if let Some(ref mut manager) = *manager_lock {
                                let pressed_buttons = manager.read_button_presses();
                            
                            for button_idx in pressed_buttons {
                                // Handle button press and get event
                                if let Some(event) = manager.handle_button_press(button_idx) {
                                    use crate::streamdeck_manager::ButtonPressEvent;
                                    
                                    match event {
                                        ButtonPressEvent::FxPressed { fx_id, is_playing, item_type } => {
                                            // PERFORMANCE: Minimal logging in hot path
                                            println!("[SD] BTN{}: {} {} [{}]", button_idx, fx_id, if is_playing { "▶" } else { "⏹" }, item_type);
                                            
                                            #[derive(Clone, serde::Serialize)]
                                            struct FxButtonPressEvent {
                                                fx_id: String,
                                                is_playing: bool,
                                                button_idx: u8,
                                                item_type: String,
                                            }
                                            
                                            let event_payload = FxButtonPressEvent {
                                                fx_id: fx_id.clone(),
                                                is_playing,
                                                button_idx,
                                                item_type: item_type.clone(),
                                            };
                                            
                                            // Fire and forget - no logging for performance
                                            let _ = app.emit("streamdeck://button_press", event_payload);
                                        }
                                        ButtonPressEvent::TvMonitorToggle => {
                                            crate::file_logger::log("[StreamDeck] 📺 TV Monitor toggle pressed!");
                                            
                                            #[derive(Clone, serde::Serialize)]
                                            struct TvMonitorEvent {
                                                action: String,
                                            }
                                            
                                            let event_payload = TvMonitorEvent {
                                                action: "toggle".to_string(),
                                            };
                                            
                                            match app.emit("streamdeck://tv_monitor", event_payload) {
                                                Ok(_) => {
                                                    crate::file_logger::log("[StreamDeck] ✅ TV Monitor event emitted");
                                                }
                                                Err(e) => {
                                                    crate::file_logger::log(&format!("[StreamDeck] ❌ Failed to emit TV event: {}", e));
                                                }
                                            }
                                        }
                                        ButtonPressEvent::VideoToggle { is_playing } => {
                                            crate::file_logger::log(&format!("[StreamDeck] 🎬 Video toggle pressed! Playing: {}", is_playing));
                                            
                                            #[derive(Clone, serde::Serialize)]
                                            struct VideoToggleEvent {
                                                is_playing: bool,
                                            }
                                            
                                            let event_payload = VideoToggleEvent { is_playing };
                                            
                                            match app.emit("streamdeck://video_toggle", event_payload) {
                                                Ok(_) => {
                                                    crate::file_logger::log("[StreamDeck] ✅ Video toggle event emitted");
                                                }
                                                Err(e) => {
                                                    crate::file_logger::log(&format!("[StreamDeck] ❌ Failed to emit video toggle: {}", e));
                                                }
                                            }
                                        }
                                        ButtonPressEvent::VideoLoopBrowse { direction, loop_index } => {
                                            crate::file_logger::log(&format!("[StreamDeck] 🔄 Video loop browse! Direction: {}, Index: {}", direction, loop_index));
                                            
                                            #[derive(Clone, serde::Serialize)]
                                            struct VideoLoopBrowseEvent {
                                                direction: i32,
                                                loop_index: usize,
                                            }
                                            
                                            let event_payload = VideoLoopBrowseEvent {
                                                direction,
                                                loop_index,
                                            };
                                            
                                            match app.emit("streamdeck://video_loop_browse", event_payload) {
                                                Ok(_) => {
                                                    crate::file_logger::log("[StreamDeck] ✅ Video loop browse event emitted");
                                                }
                                                Err(e) => {
                                                    crate::file_logger::log(&format!("[StreamDeck] ❌ Failed to emit loop browse: {}", e));
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    crate::file_logger::log(&format!("[StreamDeck] ⚠️ Button {} has no mapping", button_idx));
                                }
                            }
                            }
                        }
                    }
                }
            }
        }
    });
}

fn main() {
    // Initialize file logger first
    file_logger::init_logger();
    
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
            crate::file_logger::log("[Setup] ⚙️ Starting application setup");
            
            // TEMPORARILY DISABLED: Menu creation (testing heap corruption fix)
            // Build menu with Edit shortcuts to enable Copy/Paste/Delete
            // let menu = MenuBuilder::new(app)
            //     .items(&[
            //         &Submenu::with_items(
            //             app,
            //             "Edit",
            //             true,
            //             &[
            //                 &PredefinedMenuItem::undo(app, None)?,
            //                 &PredefinedMenuItem::redo(app, None)?,
            //                 &PredefinedMenuItem::separator(app)?,
            //                 &PredefinedMenuItem::cut(app, None)?,
            //                 &PredefinedMenuItem::copy(app, None)?,
            //                 &PredefinedMenuItem::paste(app, None)?,
            //                 &PredefinedMenuItem::select_all(app, None)?,
            //             ],
            //         )?,
            //     ])
            //     .build()?;
            
            // Set menu for the app
            // app.set_menu(menu)?;
            
            // crate::file_logger::log("[Menu] ✅ Edit menu initialized with keyboard shortcuts");
            crate::file_logger::log("[Menu] ⚠️ Menu temporarily disabled for heap corruption debugging");
            

            // Configure GStreamer paths using Tauri v2 resource_dir API
            #[cfg(windows)]
            {
                use tauri::Manager;
                
                if let Ok(resource_dir) = app.path().resource_dir() {
                    // In debug builds, no DLLs are bundled (using system GStreamer)
                    // In release builds, all DLLs are bundled via tauri.conf resources field
                    #[cfg(debug_assertions)]
                    {
                        crate::file_logger::log("[GStreamer] 🔧 DEV MODE: Using system GStreamer (no bundled DLLs)");
                    }
                    
                    #[cfg(not(debug_assertions))]
                    {
                        crate::file_logger::log(&format!("[GStreamer] 📁 Resource directory: {}", resource_dir.display()));
                        
                        // Count DLLs in resource directory (bundled via tauri.conf resources field)
                        if let Ok(entries) = std::fs::read_dir(&resource_dir) {
                            let dll_count = entries.filter(|e| {
                                e.as_ref().ok().map(|e| {
                                    e.path().extension().and_then(|ext| ext.to_str()).map(|s| s == "dll").unwrap_or(false)
                                }).unwrap_or(false)
                            }).count();
                            crate::file_logger::log(&format!("[GStreamer]   Found {} bundled DLLs", dll_count));
                        }
                    }
                    
                    // GStreamer runtime subdirectory (if exists)
                    let gst_runtime_dir = resource_dir.join("gstreamer-runtime");
                    let gst_plugin_dir = gst_runtime_dir.join("gstreamer-1.0");
                    
                    crate::file_logger::log(&format!("[GStreamer]   Runtime dir exists: {}", gst_runtime_dir.exists()));
                    crate::file_logger::log(&format!("[GStreamer]   Plugin dir exists: {}", gst_plugin_dir.exists()));
                    
                    // CRITICAL: All GStreamer DLLs (core + plugins) are in the resource root
                    // Point GST_PLUGIN_PATH to resource_dir directly (not a subdirectory)
                    let mut plugin_paths = vec![resource_dir.to_string_lossy().to_string()];
                    
                    // Also add subdirectory if it exists (for organized structure)
                    if gst_plugin_dir.exists() {
                        plugin_paths.insert(0, gst_plugin_dir.to_string_lossy().to_string());
                    }
                    
                    // Try to detect system GStreamer installation as fallback
                    let system_gst_paths = vec![
                        "E:\\gstreamer\\1.0\\msvc_x86_64\\lib\\gstreamer-1.0",
                        "C:\\gstreamer\\1.0\\msvc_x86_64\\lib\\gstreamer-1.0",
                        "C:\\gstreamer\\1.0\\x86_64\\lib\\gstreamer-1.0",
                        "C:\\Program Files\\GStreamer\\1.0\\msvc_x86_64\\lib\\gstreamer-1.0",
                    ];
                    
                    // Add system GStreamer plugins as fallback
                    for sys_path in system_gst_paths {
                        if std::path::Path::new(sys_path).exists() {
                            crate::file_logger::log(&format!("[GStreamer] 🔍 Found system GStreamer at: {}", sys_path));
                            plugin_paths.push(sys_path.to_string());
                            
                            // Also add system bin directory to PATH for DLL dependencies
                            let sys_bin = sys_path.replace("lib\\gstreamer-1.0", "bin");
                            if std::path::Path::new(&sys_bin).exists() {
                                if let Ok(mut path) = std::env::var("PATH") {
                                    path = format!("{};{}", sys_bin, path);
                                    std::env::set_var("PATH", path);
                                    crate::file_logger::log(&format!("[GStreamer] ✅ Added system bin to PATH: {}", sys_bin));
                                }
                            }
                            break;
                        }
                    }
                    
                    // Set GST_PLUGIN_PATH with bundled plugins first, then system plugins
                    let plugin_path_str = plugin_paths.join(";");
                    std::env::set_var("GST_PLUGIN_PATH", &plugin_path_str);
                    crate::file_logger::log("[GStreamer] 🔌 Plugin search paths:");
                    for path in &plugin_paths {
                        crate::file_logger::log(&format!("[GStreamer]    - {}", path));
                    }
                    
                    // Set GST_PLUGIN_SYSTEM_PATH to include system plugins
                    if plugin_paths.len() > 1 {
                        std::env::set_var("GST_PLUGIN_SYSTEM_PATH", plugin_paths.last().unwrap().clone());
                        crate::file_logger::log(&format!("[GStreamer] 🌐 System plugin path: {}", plugin_paths.last().unwrap()));
                    }
                    
                    // CRITICAL: Add resource directory to PATH for DLL discovery
                    // Tauri's resources field places DLLs directly in resource_dir
                    // IMPORTANT: Prepend to PATH so bundled DLLs take priority over system DLLs
                    if let Ok(path) = std::env::var("PATH") {
                        #[cfg(debug_assertions)]
                        {
                            // DEV MODE: Keep system GStreamer, just add resource dir for other DLLs
                            let new_path = format!("{};{}", resource_dir.display(), path);
                            std::env::set_var("PATH", new_path);
                            crate::file_logger::log("[GStreamer] ✅ DEV MODE: Added resource dir to PATH (keeping system GStreamer)");
                            crate::file_logger::log(&format!("[GStreamer]    Priority 1: {}", resource_dir.display()));
                        }
                        
                        #[cfg(not(debug_assertions))]
                        {
                            // RELEASE MODE: Remove system GStreamer to use ONLY bundled DLLs
                            let path_clean = path.split(';')
                                .filter(|p| !p.to_lowercase().contains("gstreamer"))
                                .collect::<Vec<_>>()
                                .join(";");
                            
                            // Add resource dir FIRST (contains all bundled DLLs from tauri.conf resources)
                            let new_path = format!("{};{}", resource_dir.display(), path_clean);
                            
                            std::env::set_var("PATH", new_path);
                            crate::file_logger::log("[GStreamer] ✅ RELEASE MODE: Using ONLY bundled GStreamer DLLs");
                            crate::file_logger::log("[GStreamer]    ⚠️  Removed system GStreamer from PATH to prevent conflicts");
                            crate::file_logger::log(&format!("[GStreamer]    Priority 1: {}", resource_dir.display()));
                        }
                    }
                } else {
                    // Fallback to exe directory for development
                    if let Ok(exe_path) = std::env::current_exe() {
                        if let Some(exe_dir) = exe_path.parent() {
                            let plugin_path = exe_dir.join("gstreamer-1.0");
                            std::env::set_var("GST_PLUGIN_PATH", plugin_path.to_string_lossy().to_string());
                            
                            if let Ok(path) = std::env::var("PATH") {
                                let new_path = format!("{};{}", exe_dir.display(), path);
                                std::env::set_var("PATH", new_path);
                            }
                            
                            println!("[GStreamer] 🔧 DEV MODE: Using exe directory");
                            println!("[GStreamer] Plugin path: {}", plugin_path.display());
                        }
                    }
                }
            }
            
            let app_handle = app.handle().clone();
            start_monitor_broadcast(app_handle.clone());
            
            // Start Stream Deck watcher
            start_streamdeck_watcher(app_handle.clone());
            println!("[Stream Deck] Watcher started");
            
            // DISABLED: GPU optimization was blocking all input events (keyboard, mouse, drag-drop)
            // The transform: translateZ(0) creates a compositing layer that intercepts pointer events
            // TODO: Find a way to optimize GPU without breaking WebView2 input
            // if let Some(window) = app.get_webview_window("main") {
            //     let _ = window.eval(r#"
            //         if (window.requestIdleCallback) {
            //             window.requestIdleCallback(() => {
            //                 document.body.style.willChange = 'auto';
            //                 document.body.style.transform = 'translateZ(0)';
            //             });
            //         }
            //         console.log('[GPU Optimization] Applied rendering optimizations');
            //     "#);
            // }
            
            // FIX: Ensure WebView2 can receive keyboard input and drag-drop events
            if let Some(window) = app.get_webview_window("main") {
                // Give the window focus first
                let _ = window.set_focus();
                
                // Inject JavaScript to ensure the page can receive keyboard and drag events
                let _ = window.eval(r#"
                    (function() {
                        console.log('[WebView Fix] Enabling keyboard input and drag-drop...');
                        
                        // Ensure body and document are focusable
                        document.body.setAttribute('tabindex', '0');
                        
                        // Focus the body to receive keyboard events
                        if (document.readyState === 'loading') {
                            document.addEventListener('DOMContentLoaded', function() {
                                setTimeout(function() {
                                    document.body.focus();
                                    console.log('[WebView Fix] ✅ Document focused, keyboard enabled');
                                }, 100);
                            });
                        } else {
                            document.body.focus();
                            console.log('[WebView Fix] ✅ Document focused, keyboard enabled');
                        }
                        
                        // Test keyboard listener
                        window.addEventListener('keydown', function(e) {
                            console.log('[WebView Fix] Keyboard event detected:', e.key);
                        }, { once: true });
                    })();
                "#);
            }
            
            // Setup window close handler for cleanup
            let app_handle_cleanup = app.handle().clone();
            if let Some(window) = app.get_webview_window("main") {
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        crate::file_logger::log("[Cleanup] Main window closing - cleaning up...");
                        
                        // 1. Disconnect StreamDeck
                        crate::file_logger::log("[Cleanup] Disconnecting StreamDeck...");
                        if let Some(mut manager_lock) = STREAMDECK_MANAGER.try_lock() {
                            if let Some(ref mut manager) = *manager_lock {
                                let _ = manager.disconnect();
                                crate::file_logger::log("[Cleanup] ✅ StreamDeck disconnected");
                            }
                        }
                        
                        // 2. Close any open TV monitor windows
                        crate::file_logger::log("[Cleanup] Closing TV monitor windows...");
                        let windows: Vec<_> = app_handle_cleanup.webview_windows().into_iter().collect();
                        crate::file_logger::log(&format!("[Cleanup] Found {} windows total", windows.len()));
                        for (label, window) in windows {
                            crate::file_logger::log(&format!("[Cleanup] Checking window: {}", label));
                            // Match exact label "tv-monitor" (not "tv-monitor-" with hyphen)
                            if label == "tv-monitor" || label.starts_with("tv-monitor-") {
                                crate::file_logger::log(&format!("[Cleanup] ✅ Closing TV monitor window: {}", label));
                                match window.close() {
                                    Ok(_) => crate::file_logger::log(&format!("[Cleanup] ✅ Successfully closed: {}", label)),
                                    Err(e) => crate::file_logger::log(&format!("[Cleanup] ❌ Failed to close {}: {}", label, e)),
                                }
                            }
                        }
                        crate::file_logger::log("[Cleanup] ✅ All cleanup completed");
                    }
                });
            }
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_monitors,
            check_tv_monitor_window,
            close_tv_monitor_window,
            create_monitor_window,
            create_regular_window,
            set_modal_open,
            // ✅ SCREEN CAPTURE: GStreamer preview for TV monitor modal (ACTIVELY USED)
            start_monitor_previews,
            stop_monitor_previews,
            // ❌ REMOVED: start_realtime_capture - dead code (scap library, not called)
            // ❌ REMOVED: All camera device handlers - camera system deleted, stubs return errors
            // ❌ REMOVED: initialize_camera_system, get_available_cameras, start_camera_preview, 
            //            start_camera_preview_with_quality, stop_camera_preview
            initialize_composite_system,
            start_composite_pipeline,
            stop_composite_pipeline,
            update_composite_layers,
            start_composite_output,
            stop_composite_output,
            play_composite_fx,
            stop_composite_fx,
            // DMX Lighting Control
            scan_dmx_devices,
            connect_dmx_device,
            disconnect_dmx_device,
            get_dmx_state,
            send_dmx_data,
            set_dmx_rgb,
            set_dmx_dimmer,
            set_dmx_pan_tilt,
            set_dmx_complete,
            dmx_blackout,
            // Video Loop Cache
            download_and_cache_video_loop,
            get_cached_video_loop_path,
            clear_video_loop_cache,
            // ❌ REMOVED: All virtual camera handlers - not used by frontend
            // ❌ REMOVED: initialize_virtual_camera, start_virtual_camera, stop_virtual_camera,
            //            send_frame_to_virtual_camera, get_virtual_camera_status
            streamdeck_init,
            streamdeck_scan,
            streamdeck_connect,
            streamdeck_disconnect,
            streamdeck_get_info,
            streamdeck_update_layout,
            streamdeck_sync_mappings, // NEW: Sync database mappings
            streamdeck_set_button_state,
            streamdeck_flush_updates,
            streamdeck_run_diagnostics,
            streamdeck_get_driver_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}