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
        
        let mut cameras = Vec::new();
        
        // On Windows, use GStreamer device monitor to enumerate real cameras
        #[cfg(target_os = "windows")]
        {
            use gstreamer::DeviceMonitor;
            
            let monitor = DeviceMonitor::new();
            
            // Add filter for video sources
            let caps = gst::Caps::builder("video/x-raw").build();
            monitor.add_filter(Some("Video/Source"), Some(&caps));
            
            // Start monitoring to get active devices
            if monitor.start().is_err() {
                println!("[GStreamer] Failed to start device monitor");
                return Ok(cameras);
            }
            
            let devices = monitor.devices();
            
            let mut device_index = 0;
            // Filter only devices that have valid capabilities (working cameras)
            for device in devices.iter() {
                // Check if device has valid caps (indicates it's actually working)
                if let Some(device_caps) = device.caps() {
                    if device_caps.is_empty() {
                        continue; // Skip devices with no capabilities
                    }
                    
                    let display_name = device.display_name();
                    
                    // Get the device path to verify it's a real device
                    let has_valid_path = device.properties()
                        .and_then(|props| props.get::<String>("device.path").ok())
                        .is_some();

                    // Try to get the actual device index or path for GStreamer
                    let device_id = if let Some(path) = device.properties()
                        .and_then(|props| props.get::<String>("device.path").ok()) {
                        // Use device path if available
                        path
                    } else if let Some(index) = device.properties()
                        .and_then(|props| props.get::<u32>("device.index").ok()) {
                        // Use device index if available
                        index.to_string()
                    } else {
                        // Fallback to sequential index
                        device_index.to_string()
                    };

                    // Only add cameras with valid device paths (skip virtual/unknown devices)
                    if has_valid_path {
                        println!("[GStreamer] Found: {} (device-id: {}, enum-index: {})",
                                 display_name, device_id, device_index);

                        cameras.push(CameraInfo {
                            id: device_id.clone(), // Use actual device index or path
                            name: display_name.to_string(),
                            description: format!("Active Camera (id: {})", device_id),
                        });
                        device_index += 1;
                    } else {
                        println!("[GStreamer] Skipping {} (no valid device path)", display_name);
                    }
                }
            }
            
            monitor.stop();
            
            // If no cameras found, return empty list
            if cameras.is_empty() {
                println!("[GStreamer] No active cameras detected");
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

