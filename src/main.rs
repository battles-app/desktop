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

// Screen capture for monitor previews
mod screen_capture;
use screen_capture::ScreenCaptureMonitor;

// Stream Deck integration
mod streamdeck_manager;
use streamdeck_manager::{StreamDeckManager, FxButton, STREAMDECK_MANAGER};

mod streamdeck_diagnostics;
use streamdeck_diagnostics::{run_diagnostics, get_driver_download_info, StreamDeckDiagnostics, DriverDownloadInfo};

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
    
    // Screen capture monitors (for monitor preview in selection modal)
    static ref SCREEN_CAPTURES: Arc<parking_lot::RwLock<Vec<Option<ScreenCaptureMonitor>>>> = Arc::new(parking_lot::RwLock::new(Vec::new()));
    static ref SCREEN_CAPTURE_SENDERS: Arc<parking_lot::RwLock<Vec<Option<broadcast::Sender<Vec<u8>>>>>> = Arc::new(parking_lot::RwLock::new(Vec::new()));
}

const CAMERA_WS_PORT: u16 = 9876;
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

// Start GStreamer screen capture for monitor previews (NEW - replaces screenshots)
#[command]
async fn start_monitor_previews(app: tauri::AppHandle) -> Result<(), String> {
    println!("[Monitor Preview] Starting GStreamer screen captures...");
    
    // Stop any existing captures first
    stop_monitor_previews().await?;
    
    // Get available monitors
    let monitors = app.available_monitors().map_err(|e| format!("Failed to get monitors: {}", e))?;
    let monitor_count = monitors.len();
    
    println!("[Monitor Preview] Found {} monitors", monitor_count);
    
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
        let position = monitor.position();
        let size = monitor.size();
        
        println!("[Monitor Preview {}] Starting capture: {}x{} at ({}, {})", 
            index, size.width, size.height, position.x, position.y);
        
        // Create screen capture
        let mut capture = ScreenCaptureMonitor::new(index)
            .map_err(|e| format!("Failed to create capture {}: {}", index, e))?;
        
        // Create broadcast channel for this monitor with larger buffer
        // 60 frames = 2 seconds of buffer at 30fps (prevents lag spikes during preview)
        let (tx, _rx) = broadcast::channel::<Vec<u8>>(60);
        capture.set_frame_sender(tx.clone());
        
        // Start the capture pipeline
        capture.start(position.x, position.y, size.width, size.height)
            .map_err(|e| format!("Failed to start capture {}: {}", index, e))?;
        
        // Store in global state
        {
            let mut captures = SCREEN_CAPTURES.write();
            let mut senders = SCREEN_CAPTURE_SENDERS.write();
            captures[index] = Some(capture);
            senders[index] = Some(tx.clone());
        }
        
        // Start WebSocket server for this monitor
        let port = SCREEN_CAPTURE_BASE_PORT + index as u16;
        start_monitor_preview_websocket(index, port).await;
    }
    
    println!("[Monitor Preview] ✅ All {} monitors started", monitor_count);
    Ok(())
}

// Stop all monitor preview captures
#[command]
async fn stop_monitor_previews() -> Result<(), String> {
    println!("[Monitor Preview] Stopping all captures...");
    
    let mut captures = SCREEN_CAPTURES.write();
    let mut senders = SCREEN_CAPTURE_SENDERS.write();
    
    // Stop all captures
    for (index, capture_opt) in captures.iter_mut().enumerate() {
        if let Some(mut capture) = capture_opt.take() {
            capture.stop().map_err(|e| format!("Failed to stop capture {}: {}", index, e))?;
        }
    }
    
    captures.clear();
    senders.clear();
    
    println!("[Monitor Preview] ✅ All captures stopped");
    Ok(())
}

// WebSocket server for monitor preview streaming
async fn start_monitor_preview_websocket(monitor_index: usize, port: u16) {
    tokio::spawn(async move {
        let addr = format!("127.0.0.1:{}", port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                println!("[Monitor Preview {}] ❌ Failed to bind to {}: {}", monitor_index, addr, e);
                return;
            }
        };
        
        println!("[Monitor Preview {}] ✅ WebSocket server listening on {}", monitor_index, addr);
        
        while let Ok((stream, _)) = listener.accept().await {
            // Get sender for this monitor
            let tx = {
                let senders = SCREEN_CAPTURE_SENDERS.read();
                match &senders.get(monitor_index) {
                    Some(Some(sender)) => sender.clone(),
                    _ => {
                        println!("[Monitor Preview {}] ❌ No sender available", monitor_index);
                        continue;
                    }
                }
            };
            
            tokio::spawn(async move {
                let ws_stream = match accept_async(stream).await {
                    Ok(ws) => ws,
                    Err(e) => {
                        println!("[Monitor Preview {}] Error during handshake: {}", monitor_index, e);
                        return;
                    }
                };
                
                println!("[Monitor Preview {}] ✅ Client connected", monitor_index);
                
                use tokio_tungstenite::tungstenite::protocol::Message;
                let (mut ws_sender, mut ws_receiver) = ws_stream.split();
                
                // Subscribe to frames
                let mut rx = tx.subscribe();
                let mut frame_count = 0u64;
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
                                        frame_count += 1;
                                        last_send_time = now;
                                        
                                        // Send frame
                                        if ws_sender.send(Message::Binary(frame_data)).await.is_err() {
                                            println!("[Monitor Preview {}] ❌ Client disconnected after {} frames", monitor_index, frame_count);
                                            break;
                                        }
                                    }
                                    // Else: Drop frame silently (preview doesn't need full framerate)
                                },
                                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                    // Should rarely happen with 60-frame buffer + rate limiting
                                    if skipped > 10 {
                                        println!("[Monitor Preview {}] ⚠️ Severe lag: skipped {} frames", monitor_index, skipped);
                                    }
                                    continue;
                                },
                                Err(broadcast::error::RecvError::Closed) => {
                                    println!("[Monitor Preview {}] ℹ️ Broadcast channel closed", monitor_index);
                                    break;
                                }
                            }
                        }
                        ws_msg = ws_receiver.next() => {
                            if ws_msg.is_none() {
                                println!("[Monitor Preview {}] 🔌 Client disconnected gracefully", monitor_index);
                                break;
                            }
                        }
                    }
                }
                
                println!("[Monitor Preview {}] 🔌 Client disconnected (sent {} frames)", monitor_index, frame_count);
            });
        }
    });
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
    if let Some(window) = app.get_webview_window("tv-monitor") {
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
    monitor_index: usize,
    monitor_position: (i32, i32),  // Pass position from frontend to match preview
    monitor_size: (u32, u32)       // Pass size from frontend to match preview
) -> Result<(), String> {
    println!("Creating monitor window for monitor index: {}", monitor_index);
    println!("Frontend monitor data: position=({}, {}), size={}x{}", 
        monitor_position.0, monitor_position.1, monitor_size.0, monitor_size.1);

    // Get Tauri's native monitor info to find the matching monitor
    let native_monitors = app.available_monitors().unwrap_or_default();
    println!("Found {} native monitors", native_monitors.len());

    // Find the monitor that matches the position and size from frontend
    let native_monitor = native_monitors.iter()
        .find(|m| {
            let pos = m.position();
            let size = m.size();
            pos.x == monitor_position.0 && 
            pos.y == monitor_position.1 && 
            size.width == monitor_size.0 && 
            size.height == monitor_size.1
        })
        .ok_or_else(|| format!(
            "No monitor found matching position ({}, {}) and size {}x{}",
            monitor_position.0, monitor_position.1, monitor_size.0, monitor_size.1
        ))?;

    let monitor_pos = native_monitor.position();
    let monitor_size_actual = native_monitor.size();
    let scale_factor = native_monitor.scale_factor();

    println!("✅ Matched monitor: position=({}, {}), size={}x{}, scale={}",
             monitor_pos.x, monitor_pos.y, monitor_size_actual.width, monitor_size_actual.height, scale_factor);

    // Convert physical pixels to logical pixels for Tauri v2
    // Tauri's inner_size and position expect logical pixels
    let logical_width = monitor_size_actual.width as f64 / scale_factor;
    let logical_height = monitor_size_actual.height as f64 / scale_factor;
    let logical_x = monitor_pos.x as f64 / scale_factor;
    let logical_y = monitor_pos.y as f64 / scale_factor;

    println!("Logical coordinates: position=({}, {}), size={}x{}",
             logical_x, logical_y, logical_width, logical_height);

    // Close any existing TV monitor window first (Tauri v2 API)
    if let Some(existing_window) = app.get_webview_window("tv-monitor") {
        println!("⚠️  Found existing TV monitor window, destroying it first...");
        
        // Try to close the window first (more graceful than destroy)
        match existing_window.close() {
            Ok(_) => println!("✅ Window close() called successfully"),
            Err(e) => println!("⚠️  Window close() failed: {}, trying destroy()...", e),
        }
        
        // Wait longer to ensure window is fully closed/destroyed
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        // Check if window still exists, if so, force destroy
        if let Some(still_exists) = app.get_webview_window("tv-monitor") {
            println!("⚠️  Window still exists after close(), forcing destroy()...");
            match still_exists.destroy() {
                Ok(_) => println!("✅ Window destroy() completed"),
                Err(e) => {
                    let err_msg = format!("❌ CRITICAL: Cannot destroy window: {}. Cannot proceed.", e);
                    println!("{}", err_msg);
                    return Err(err_msg);
                }
            }
            // Extra wait after forced destroy
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        
        // Final verification - window MUST be gone before proceeding
        if app.get_webview_window("tv-monitor").is_some() {
            let err_msg = "❌ CRITICAL: Window still exists after multiple destroy attempts. Cannot proceed.".to_string();
            println!("{}", err_msg);
            return Err(err_msg);
        }
        
        println!("✅ Window fully destroyed and verified gone, proceeding with creation...");
    } else {
        println!("✅ No existing tv-monitor window found, safe to create new one");
    }

    // Create a borderless fullscreen window on the selected monitor
    println!("Creating borderless fullscreen window");
    println!("Monitor window URL: {}", url);
    println!("Window configuration: {}x{} at ({}, {}), always_on_top=true, decorations=false",
             logical_width, logical_height, logical_x, logical_y);

    // Parse URL safely
    let parsed_url = url.parse()
        .map_err(|e| format!("Failed to parse URL '{}': {}", url, e))?;
    
    println!("Successfully parsed URL");

    // Use WebviewWindowBuilder which supports URL in Tauri v2
    println!("🔨 Building WebviewWindow with label 'tv-monitor'...");
    let window = tauri::webview::WebviewWindowBuilder::new(&app, "tv-monitor", tauri::WebviewUrl::External(parsed_url))
        .title("TV Monitor - Battles.app")
        .inner_size(logical_width, logical_height)
        .position(logical_x, logical_y)
        .decorations(false) // Borderless
        .resizable(false)   // Fixed size
        .always_on_top(true) // Above all other windows
        .visible(true)      // ✅ Start VISIBLE (was false)
        .fullscreen(false)   // Use borderless window (not true fullscreen)
        .skip_taskbar(false) // Show in taskbar for easy access
        .build()
        .map_err(|e| {
            let error_msg = format!("❌ Failed to build monitor window: {}", e);
            println!("{}", error_msg);
            error_msg
        })?;
    
    println!("✅ WebviewWindow build() completed successfully");
    
    // CRITICAL: Wait for window to be registered in Tauri's window manager
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    
    // Verify the window was actually created and registered
    match app.get_webview_window("tv-monitor") {
        Some(_) => println!("✅ Window VERIFIED in app.get_webview_window()"),
        None => {
            let err = "❌ CRITICAL: Window was built but NOT found in app.get_webview_window()! This is a Tauri registration issue.";
            println!("{}", err);
            return Err(err.to_string());
        }
    }
    
    println!("✅ Window object created and verified");

    // Listen for window close events to notify the main window
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Destroyed = event {
            println!("TV monitor window was closed by user");
            // Emit event to notify main window that TV monitor was closed
            let _ = app_handle.emit("tv-monitor-closed", ());
        }
    });

    // Window was created with visible(true), now just ensure it's on top and focused
    println!("🎯 Configuring window visibility and focus...");
    
    // Verify initial visibility
    match window.is_visible() {
        Ok(true) => println!("✅ Window is already visible (as expected)"),
        Ok(false) => {
            println!("⚠️  Window reports as NOT visible even though created with visible(true)!");
            println!("    Calling show() explicitly...");
            window.show().map_err(|e| format!("Failed to show window: {}", e))?;
        }
        Err(e) => println!("⚠️  Could not check visibility: {}", e)
    }
    
    // Unminimize if somehow minimized
    if let Ok(is_minimized) = window.is_minimized() {
        if is_minimized {
            println!("⚠️  Window is minimized, unminimizing...");
            window.unminimize().map_err(|e| format!("Failed to unminimize: {}", e))?;
        }
    }
    
    // Set focus to bring window to front
    println!("🎯 Setting window focus...");
    window.set_focus()
        .map_err(|e| {
            let error_msg = format!("Failed to focus window: {}", e);
            println!("❌ {}", error_msg);
            error_msg
        })?;
    println!("✅ Window focused");
    
    // Final visibility check
    match window.is_visible() {
        Ok(true) => println!("✅ Final check: Window IS visible"),
        Ok(false) => {
            println!("❌ CRITICAL: Window still reports as NOT visible!");
            println!("   This means the window exists but Tauri thinks it's hidden.");
            println!("   Attempting one more show() call...");
            window.show().map_err(|e| format!("Failed to show window (final): {}", e))?;
        }
        Err(e) => println!("⚠️  Could not check final visibility: {}", e)
    }

    println!("✅ Monitor window created and shown successfully!");
    println!("   Monitor: {}", monitor_index);
    println!("   Position: ({}, {})", logical_x, logical_y);
    println!("   Size: {}x{}", logical_width, logical_height);
    println!("   URL: {}", url);
    println!("   Always on top: true");
    println!("   Decorations: false (borderless)");
    println!("   ℹ️  If you don't see the window, check monitor {} (it should be fullscreen there)", monitor_index);
    Ok(())
}

// Create regular window (1080x640, resizable, movable, center-right position)
#[command]
async fn create_regular_window(app: tauri::AppHandle, url: String) -> Result<(), String> {
    println!("Creating regular window: 1080x640 at center-right position with direct URL");

    // Close any existing TV monitor window first (Tauri v2 API)
    if let Some(existing_window) = app.get_webview_window("tv-monitor") {
        println!("⚠️  Found existing TV monitor window, destroying before creating regular window...");
        
        match existing_window.close() {
            Ok(_) => println!("✅ Window close() called"),
            Err(e) => println!("⚠️  Window close() failed: {}", e),
        }
        
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        
        if let Some(still_exists) = app.get_webview_window("tv-monitor") {
            println!("⚠️  Window still exists, forcing destroy()...");
            let _ = still_exists.destroy();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
        
        println!("✅ Old window cleanup complete");
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
    
    // Create broadcast channel for camera frames with larger buffer
    // 60 frames = 2 seconds of buffer at 30fps (prevents lag spikes)
    let (tx, _rx) = broadcast::channel::<Vec<u8>>(60);
    
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
async fn streamdeck_init() -> Result<(), String> {
    println!("[Stream Deck] Initializing Stream Deck system");
    
    let manager = StreamDeckManager::new()?;
    *STREAMDECK_MANAGER.lock() = Some(manager);
    
    println!("[Stream Deck] ✅ Initialized");
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
    println!("[Stream Deck] Connecting to device...");
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        let result = manager.connect()?;
        println!("[Stream Deck] ✅ {}", result);
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
    println!("[Stream Deck] Received layout update request");
    println!("[Stream Deck]   Battle Board: {} items", battle_board.len());
    println!("[Stream Deck]   User FX: {} items", user_fx.len());
    
    // Debug first few items
    if !battle_board.is_empty() {
        println!("[Stream Deck]   First battle board item: {:?}", &battle_board[0]);
    }
    if !user_fx.is_empty() {
        println!("[Stream Deck]   First user FX item: {:?}", &user_fx[0]);
    }
    
    let mut manager_lock = STREAMDECK_MANAGER.lock();
    
    if let Some(ref mut manager) = *manager_lock {
        manager.update_layout(battle_board, user_fx)?;
        println!("[Stream Deck] ✅ Layout updated successfully");
        Ok(())
    } else {
        Err("Stream Deck not initialized".to_string())
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
        let mut button_check_interval = tokio::time::interval(std::time::Duration::from_millis(5));
        let mut was_connected = false;
        
        loop {
            tokio::select! {
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
                            println!("[Stream Deck Watcher] Device connected");
                            let _ = app.emit("streamdeck://connected", ());
                        } else {
                            println!("[Stream Deck Watcher] Device disconnected");
                            let _ = app.emit("streamdeck://disconnected", ());
                            
                            // Try to reconnect
                            let mut manager_lock = STREAMDECK_MANAGER.lock();
                            if let Some(ref mut manager) = *manager_lock {
                                println!("[Stream Deck Watcher] Attempting to reconnect...");
                                if let Ok(info) = manager.connect() {
                                    println!("[Stream Deck Watcher] ✅ Reconnected: {}", info);
                                    let _ = app.emit("streamdeck://connected", ());
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
                                // Handle button press and get FX info
                                if let Some((fx_id, is_playing)) = manager.handle_button_press(button_idx) {
                                    
                                    // Emit event to frontend with FX ID and new state
                                    #[derive(Clone, serde::Serialize)]
                                    struct ButtonPressEvent {
                                        fx_id: String,
                                        is_playing: bool,
                                        button_idx: u8,
                                    }
                                    
                                    let _ = app.emit("streamdeck://button_press", ButtonPressEvent {
                                        fx_id,
                                        is_playing,
                                        button_idx,
                                    });
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
            start_monitor_broadcast(app_handle.clone());
            
            // Start Stream Deck watcher
            start_streamdeck_watcher(app_handle.clone());
            println!("[Stream Deck] Watcher started");
            
            // Stream Deck cleanup is handled by disconnect() in frontend onUnmounted
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_monitors,
            check_tv_monitor_window,
            close_tv_monitor_window,
            create_monitor_window,
            create_regular_window,
            set_modal_open,
            start_monitor_previews,
            stop_monitor_previews,
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
            get_virtual_camera_status,
            streamdeck_init,
            streamdeck_scan,
            streamdeck_connect,
            streamdeck_disconnect,
            streamdeck_get_info,
            streamdeck_update_layout,
            streamdeck_set_button_state,
            streamdeck_run_diagnostics,
            streamdeck_get_driver_info
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}