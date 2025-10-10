// GStreamer-based camera capture (OBS-quality performance)
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline};
use gstreamer_app::AppSink;
use tokio::sync::broadcast;
use std::sync::Arc;
use parking_lot::RwLock;

pub struct GStreamerCamera {
    pipeline: Option<Pipeline>,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
}

impl GStreamerCamera {
    pub fn new() -> Result<Self, String> {
        // Initialize GStreamer
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
        
        println!("[GStreamer] Initialized successfully");
        
        Ok(Self {
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        })
    }
    
    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write() = Some(sender);
    }
    
    pub fn list_cameras() -> Result<Vec<CameraInfo>, String> {
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
        
        // Log available GStreamer plugins for debugging
        println!("[GStreamer] 🔌 Checking available plugins...");
        let registry = gst::Registry::get();
        
        // Check for important video source plugins
        let important_plugins = vec![
            ("dshowvideosrc", "DirectShow video source (Windows cameras)"),
            ("mfvideosrc", "Media Foundation video source (Windows cameras)"),
            ("ksvideosrc", "Kernel Streaming video source (legacy Windows)"),
            ("ndisrc", "NDI source (NewTek NDI protocol)"),
            ("nvh264dec", "NVIDIA hardware decoder"),
            ("d3d11", "Direct3D 11 elements"),
            ("wasapi", "Windows Audio Session API"),
        ];
        
        for (plugin_name, description) in important_plugins {
            if let Some(plugin) = registry.find_plugin(plugin_name) {
                println!("[GStreamer]   ✅ {} - {} v{}", 
                         plugin_name, 
                         description,
                         plugin.version());
            } else if let Some(feature) = registry.find_feature(plugin_name, gst::ElementFactory::static_type()) {
                println!("[GStreamer]   ✅ {} - {} (available)", plugin_name, description);
            } else {
                println!("[GStreamer]   ❌ {} - {} (NOT AVAILABLE)", plugin_name, description);
            }
        }
        
        let mut cameras = Vec::new();
        
        // On Windows, use GStreamer device monitor to enumerate real cameras
        #[cfg(target_os = "windows")]
        {
            use gstreamer::DeviceMonitor;
            
            println!("[GStreamer] Starting device enumeration...");
            
            let monitor = DeviceMonitor::new();
            
            // Add filter for video sources - use broader filter to catch all video sources
            // This includes: webcams, virtual cameras, NDI, capture cards, etc.
            let caps = gst::Caps::builder("video/x-raw").build();
            if monitor.add_filter(Some("Video/Source"), Some(&caps)).is_none() {
                println!("[GStreamer] ⚠️ Failed to add filter (returned None)");
            }
            
            // Start monitoring to get active devices
            if let Err(e) = monitor.start() {
                println!("[GStreamer] ❌ Failed to start device monitor: {:?}", e);
                println!("[GStreamer] ℹ️ Falling back to manual DirectShow enumeration...");
                
                // Fallback: Try to enumerate using DirectShow element directly
                // This is a workaround when DeviceMonitor doesn't work
                for i in 0..10 {
                    // Try to probe each device index
                    let test_pipeline_str = format!("mfvideosrc device-index={} ! fakesink", i);
                    
                    if let Ok(pipeline) = gst::parse::launch(&test_pipeline_str) {
                        if let Ok(_) = pipeline.set_state(gst::State::Ready) {
                            cameras.push(CameraInfo {
                                id: format!("Camera {}", i),
                                name: format!("Camera {} (Media Foundation)", i),
                                description: format!("Video Source (index: {})", i),
                            });
                            println!("[GStreamer]   ✅ Found camera at index {}", i);
                            let _ = pipeline.set_state(gst::State::Null);
                        } else {
                            // No more cameras
                            break;
                        }
                    } else {
                        break;
                    }
                }
                
                if !cameras.is_empty() {
                    println!("[GStreamer] 📹 Manual enumeration found {} cameras", cameras.len());
                    monitor.stop();
                    return Ok(cameras);
                }
                
                println!("[GStreamer] ❌ Manual enumeration also failed");
                return Ok(cameras);
            }
            
            let devices = monitor.devices();
            println!("[GStreamer] Found {} potential video sources", devices.len());
            
            let mut device_index = 0;
            let mut seen_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
            
            // Enumerate ALL video sources (including virtual cameras, NDI, etc.)
            for device in devices.iter() {
                let display_name = device.display_name();
                let device_class = device.device_class();
                
                // Check if device has valid caps (indicates it's actually working)
                if let Some(device_caps) = device.caps() {
                    if device_caps.is_empty() {
                        continue; // Skip devices with empty capabilities silently
                    }
                    
                    // Get device path for deduplication
                    let device_path = device.properties()
                        .and_then(|props| props.get::<String>("device.path").ok());
                    
                    let device_api = device.properties()
                        .and_then(|props| props.get::<String>("device.api").ok());
                    
                    // Deduplicate devices by path
                    // Prefer Media Foundation devices over DirectShow
                    if let Some(ref path) = device_path {
                        if seen_paths.contains(path) {
                            // Skip duplicate device
                            continue;
                        }
                        seen_paths.insert(path.clone());
                    } else if device_api.as_deref() != Some("mediafoundation") {
                        // If no path and not Media Foundation, it might be a duplicate from DirectShow
                        // Check if we already have a device with the same name
                        if cameras.iter().any(|c| c.name == display_name.to_string()) {
                            continue; // Skip duplicate
                        }
                    }
                    
                    println!("[GStreamer] 🔍 Found: {} ({})", 
                             display_name, 
                             device_api.as_deref().unwrap_or("unknown"));
                    
                    // Use the display name as the ID (works for DirectShow, NDI, etc.)
                    let device_name = display_name.to_string();
                    
                    // Get the device index from properties if available (fallback to sequential)
                    let device_idx = device.properties()
                        .and_then(|props| props.get::<u32>("device.index").ok())
                        .unwrap_or(device_index);

                    // Store device path for mfvideosrc, fallback to name for dshowvideosrc
                    let device_id = device_path.clone().unwrap_or_else(|| device_name.clone());
                    
                    cameras.push(CameraInfo {
                        id: device_id, // Use device path for mfvideosrc
                        name: device_name,
                        description: format!("{} ({})", device_api.as_deref().unwrap_or("unknown"), device_class),
                    });
                    device_index += 1;
                }
            }
            
            monitor.stop();
            
            println!("[GStreamer] 📹 Device monitor found {} cameras", cameras.len());
            
            // Try to enumerate NDI sources explicitly (they might not show up in device monitor)
            println!("[GStreamer] 🔍 Checking for NDI sources...");
            let registry = gst::Registry::get();
            if let Some(_ndi_feature) = registry.find_feature("ndisrc", gst::ElementFactory::static_type()) {
                println!("[GStreamer]   NDI plugin is available, trying to enumerate NDI sources...");
                
                // Add a generic NDI source option if plugin is available
                // Note: NDI source enumeration requires network discovery which is complex
                // For now, we just add a generic option that users can configure
                println!("[GStreamer]   ℹ️ Adding generic NDI source option...");
                cameras.push(CameraInfo {
                    id: "ndi://".to_string(), // Use ndi:// prefix to identify NDI sources
                    name: "NDI Source (Network Device)".to_string(),
                    description: "NewTek NDI Network Video Source".to_string(),
                });
            } else {
                println!("[GStreamer]   ℹ️ NDI plugin not available (install gst-plugins-bad or NDI plugin)");
            }
            
            // Final summary
            if cameras.is_empty() {
                println!("[GStreamer] ❌ No cameras or video sources detected");
                println!("[GStreamer]    Possible reasons:");
                println!("[GStreamer]    1. No physical cameras connected");
                println!("[GStreamer]    2. GStreamer plugins not properly installed");
                println!("[GStreamer]    3. Permissions issue accessing devices");
                println!("[GStreamer]    4. DirectShow/Media Foundation not working");
            } else {
                println!("[GStreamer] ✅ Total video sources available: {}", cameras.len());
            }
        }
        
        // On Linux, enumerate v4l2 devices
        #[cfg(target_os = "linux")]
        {
            for i in 0..10 {
                cameras.push(CameraInfo {
                    id: format!("/dev/video{}", i),
                    name: format!("Video Device {}", i),
                    description: "V4L2 Camera".to_string(),
                });
            }
        }
        
        // On macOS, enumerate AVFoundation devices
        #[cfg(target_os = "macos")]
        {
            for i in 0..10 {
                cameras.push(CameraInfo {
                    id: i.to_string(),
                    name: format!("Camera {}", i),
                    description: "AVFoundation Camera".to_string(),
                });
            }
        }
        
        Ok(cameras)
    }
    
    #[allow(dead_code)]
    pub fn start(&mut self, device_id: &str) -> Result<(), String> {
        self.start_with_quality(device_id, "high")
    }
    
    pub fn start_with_quality(&mut self, device_id: &str, quality: &str) -> Result<(), String> {
        let device_index: u32 = device_id.parse().map_err(|_| "Invalid device ID")?;
        println!("[GStreamer] Starting camera {} with {} quality", device_index, quality);
        
        // Stop existing pipeline if any
        if let Some(pipeline) = &self.pipeline {
            let _ = pipeline.set_state(gst::State::Null);
        }
        
        *self.is_running.write() = true;
        
        // Quality presets (width, height, jpeg_quality, description)
        let (width, height, jpeg_quality) = match quality {
            "low" => (640, 360, 60),      // Low - 360p, lower quality
            "medium" => (1280, 720, 75),  // Medium - 720p, balanced
            "high" => (1280, 720, 90),    // High - 720p, high quality (default)
            "ultra" => (1920, 1080, 95),  // Ultra - 1080p, maximum quality
            _ => (1280, 720, 90),         // Default to high
        };
        
        println!("[GStreamer] Resolution: {}x{}, JPEG quality: {}", width, height, jpeg_quality);
        
        // Build GStreamer pipeline
        // Windows: mfvideosrc (Media Foundation - modern, replaces deprecated ksvideosrc)
        // Linux: v4l2src ! videoconvert ! video/x-raw,format=RGB ! jpegenc ! appsink
        // macOS: avfvideosrc ! videoconvert ! video/x-raw,format=RGB ! jpegenc ! appsink
        
        #[cfg(target_os = "windows")]
        let pipeline_str = format!(
            "mfvideosrc device-index={} ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoconvert ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoscale ! \
             queue leaky=downstream max-size-buffers=3 ! \
             video/x-raw,width={},height={} ! \
             queue leaky=downstream max-size-buffers=2 ! \
             jpegenc quality={} ! \
             appsink name=sink emit-signals=true sync=true max-buffers=2 drop=true",
            device_index, width, height, jpeg_quality
        );
        
        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "v4l2src device=/dev/video{} ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoconvert ! \
             queue leaky=downstream max-size-buffers=3 ! \
             video/x-raw,format=RGB,width=1280,height=720,framerate=30/1 ! \
             queue leaky=downstream max-size-buffers=2 ! \
             jpegenc quality=80 ! \
             appsink name=sink emit-signals=true sync=true max-buffers=2 drop=true",
            device_index
        );
        
        #[cfg(target_os = "macos")]
        let pipeline_str = format!(
            "avfvideosrc device-index={} ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoconvert ! \
             queue leaky=downstream max-size-buffers=3 ! \
             video/x-raw,format=RGB,width=1280,height=720,framerate=30/1 ! \
             queue leaky=downstream max-size-buffers=2 ! \
             jpegenc quality=80 ! \
             appsink name=sink emit-signals=true sync=true max-buffers=2 drop=true",
            device_index
        );
        
        println!("[GStreamer] ⚡ Raw camera pipeline (low-latency with queues): {}", pipeline_str);
        
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;
        
        // Get the appsink element
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        // Set up the appsink callbacks with comprehensive debugging
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();

        use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
        use std::time::Instant;

        let frame_count = Arc::new(AtomicU64::new(0));
        let warned_black = Arc::new(AtomicBool::new(false));
        let start_time = Arc::new(Instant::now());
        let last_log_time = Arc::new(RwLock::new(Instant::now()));
        let last_frame_count = Arc::new(AtomicU64::new(0));

        let frame_count_clone = frame_count.clone();
        let warned_black_clone = warned_black.clone();
        let _start_time_clone = start_time.clone();
        let _last_log_time_clone = last_log_time.clone();
        let _last_frame_count_clone = last_frame_count.clone();
        
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    
                    // Get JPEG data
                    let jpeg_data = map.as_slice();
                    
                    // Validate frame (check if it's not just zeros/black)
                    if jpeg_data.len() > 100 {
                        let count = frame_count_clone.fetch_add(1, Ordering::Relaxed);
                        
                        // Log first successful frame
                        if count == 0 {
                            println!("[GStreamer] ✅ Receiving frames ({} bytes per frame)", jpeg_data.len());
                        }
                        
                        // Check for suspiciously small frames (likely black/empty)
                        if !warned_black_clone.load(Ordering::Relaxed) && jpeg_data.len() < 5000 && count > 10 {
                            println!("[GStreamer] ⚠️ Warning: Receiving very small frames ({} bytes), may be black screen", jpeg_data.len());
                            warned_black_clone.store(true, Ordering::Relaxed);
                        }
                        
                        // Broadcast to WebSocket clients
                        if let Some(sender) = frame_sender.read().as_ref() {
                            let _ = sender.send(jpeg_data.to_vec());
                        }
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Start the pipeline
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {:?}", e))?;
        
        // Wait for pipeline to reach PLAYING state
        let state_result = pipeline.state(Some(gst::ClockTime::from_seconds(5)));
        match state_result.1 {
            gst::State::Playing => {
                println!("[GStreamer] 🚀 Pipeline started - streaming!");
            }
            state => {
                println!("[GStreamer] ⚠️ Pipeline in state {:?}, may not produce frames", state);
            }
        }
        
        // Check for bus messages (errors/warnings)
        let bus = pipeline.bus().ok_or("Pipeline has no bus")?;
        if let Some(msg) = bus.pop() {
            use gst::MessageView;
            match msg.view() {
                MessageView::Error(err) => {
                    println!("[GStreamer] ❌ Pipeline error: {}", err.error());
                }
                MessageView::Warning(warn) => {
                    println!("[GStreamer] ⚠️ Pipeline warning: {}", warn.error());
                }
                _ => {}
            }
        }
        
        self.pipeline = Some(pipeline);
        
        Ok(())
    }
    
    pub fn stop(&mut self) -> Result<(), String> {
        println!("[GStreamer] Stopping camera");
        
        *self.is_running.write() = false;
        
        if let Some(pipeline) = &self.pipeline {
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {:?}", e))?;
        }
        
        self.pipeline = None;
        
        println!("[GStreamer] Camera stopped");
        Ok(())
    }
    
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }
}

impl Drop for GStreamerCamera {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[derive(Clone)]
pub struct CameraInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

