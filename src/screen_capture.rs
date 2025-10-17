// Screen capture using GStreamer for monitor preview
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

pub struct ScreenCaptureMonitor {
    monitor_index: usize,
    pipeline: Option<gst::Pipeline>,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
}

impl ScreenCaptureMonitor {
    pub fn new(monitor_index: usize) -> Result<Self, String> {
        Ok(Self {
            monitor_index,
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        })
    }

    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write().unwrap() = Some(sender);
    }

    pub fn start(&mut self, _monitor_x: i32, _monitor_y: i32, monitor_width: u32, monitor_height: u32) -> Result<(), String> {
        // Initialize GStreamer if not already initialized
        let _ = gst::init();
        
        crate::file_logger::log("[ScreenCapture] 🎬 Starting screen capture...");
        crate::file_logger::log(&format!("[ScreenCapture]   Monitor: index={}, size={}x{}", 
            self.monitor_index, monitor_width, monitor_height));
        
        // Check available screen capture elements
        let has_d3d11 = gst::ElementFactory::find("d3d11screencapturesrc").is_some();
        let has_d3d11download = gst::ElementFactory::find("d3d11download").is_some();
        let has_dx9 = gst::ElementFactory::find("dx9screencapsrc").is_some();
        let has_gdi = gst::ElementFactory::find("gdiscreencapsrc").is_some();
        
        crate::file_logger::log(&format!("[ScreenCapture]   Available elements: d3d11screencapturesrc={}, d3d11download={}, dx9screencapsrc={}, gdiscreencapsrc={}", 
            has_d3d11, has_d3d11download, has_dx9, has_gdi));
        
        // Check if d3d11screencapturesrc is available
        if !has_d3d11 {
            crate::file_logger::log("[ScreenCapture]   d3d11screencapturesrc NOT available, trying fallbacks...");
            
            // Try alternative: dx9screencapsrc
            if has_dx9 {
                crate::file_logger::log("[ScreenCapture]   Using DX9 fallback");
                return self.start_with_dx9(monitor_width, monitor_height);
            }
            
            // Try alternative: gdiscreencapsrc (GDI)
            if has_gdi {
                crate::file_logger::log("[ScreenCapture]   Using GDI fallback");
                return self.start_with_gdi(monitor_width, monitor_height);
            }
            
            crate::file_logger::log("[ScreenCapture]   ❌ No screen capture plugin available!");
            return Err("No screen capture plugin available (tried d3d11screencapturesrc, dx9screencapsrc, gdiscreencapsrc)".to_string());
        }
        
        if !has_d3d11download {
            crate::file_logger::log("[ScreenCapture]   ⚠️ WARNING: d3d11download element not found, capture may fail!");
        }
        
        crate::file_logger::log("[ScreenCapture]   ✅ Using D3D11 screen capture");

        // Calculate preview dimensions (max 240x135 for low bandwidth and fast startup)
        let preview_width = 240u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;

        // Build GStreamer pipeline for screen capture
        // Using d3d11screencapturesrc (Windows) - need to download from GPU memory first
        // Capped at 5 FPS for nearly instant startup and low CPU usage
        let pipeline_str = format!(
            "d3d11screencapturesrc monitor-index={} ! \
             video/x-raw(memory:D3D11Memory),format=BGRA,framerate=5/1 ! \
             d3d11download ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale method=0 ! \
             video/x-raw,width={},height={} ! \
             appsink name=sink",
            self.monitor_index,
            preview_width,
            preview_height
        );

        crate::file_logger::log(&format!("[ScreenCapture]   Pipeline: {}", pipeline_str));
        
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| {
                let error_msg = format!("Failed to create pipeline: {}", e);
                crate::file_logger::log(&format!("[ScreenCapture]   ❌ {}", error_msg));
                error_msg
            })?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| {
                let error_msg = "Failed to cast to Pipeline";
                crate::file_logger::log(&format!("[ScreenCapture]   ❌ {}", error_msg));
                error_msg.to_string()
            })?;
        
        crate::file_logger::log("[ScreenCapture]   ✅ Pipeline created successfully");

        // Get appsink
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;

        // Set appsink properties for ultra-low latency and instant startup
        appsink.set_property("max-buffers", 1u32);
        appsink.set_property("drop", true);
        appsink.set_property("emit-signals", false);

        // Set up callbacks for frame delivery
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        let monitor_index = self.monitor_index;
        let frame_count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let frame_count_clone = frame_count.clone();

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read().unwrap() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| {
                        crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: ❌ Failed to pull sample", monitor_index));
                        gst::FlowError::Error
                    })?;
                    
                    let buffer = sample.buffer().ok_or_else(|| {
                        crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: ❌ No buffer in sample", monitor_index));
                        gst::FlowError::Error
                    })?;
                    
                    let map = buffer.map_readable().map_err(|_| {
                        crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: ❌ Failed to map buffer", monitor_index));
                        gst::FlowError::Error
                    })?;

                    let count = frame_count_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    
                    // Log first frame and every 30 frames
                    if count == 0 || count % 30 == 0 {
                        crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: 📸 Frame {} ({}x{} bytes)", 
                            monitor_index, count, map.len(), map.len()));
                    }

                    // Broadcast frame to WebSocket (gracefully handle no receivers)
                    if let Some(sender) = &*frame_sender.read().unwrap() {
                        let _ = sender.send(map.as_slice().to_vec()); // Ignore errors - no receivers is OK
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        crate::file_logger::log("[ScreenCapture]   Setting pipeline to PLAYING...");
        
        // Start pipeline
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| {
                let error_msg = format!("Failed to start pipeline: {:?}", e);
                crate::file_logger::log(&format!("[ScreenCapture]   ❌ {}", error_msg));
                error_msg
            })?;

        // Wait a minimal moment for pipeline to start
        std::thread::sleep(std::time::Duration::from_millis(50));
        
        // Check pipeline state
        let (state, pending, _) = pipeline.state(Some(gst::ClockTime::from_seconds(1)));
        crate::file_logger::log(&format!("[ScreenCapture]   Pipeline state: {:?}, pending: {:?}", state, pending));
        
        self.pipeline = Some(pipeline);
        *self.is_running.write().unwrap() = true;
        
        crate::file_logger::log(&format!("[ScreenCapture] ✅ Monitor {} capture started", monitor_index));

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: 🛑 Stopping...", self.monitor_index));
        
        *self.is_running.write().unwrap() = false;

        if let Some(pipeline) = &self.pipeline {
            crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: Setting pipeline to NULL...", self.monitor_index));
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| {
                    let error_msg = format!("Failed to stop pipeline: {:?}", e);
                    crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: ❌ {}", self.monitor_index, error_msg));
                    error_msg
                })?;
            crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: ✅ Pipeline stopped", self.monitor_index));
        }

        self.pipeline = None;
        *self.frame_sender.write().unwrap() = None;

        crate::file_logger::log(&format!("[ScreenCapture] Monitor {}: ✅ Capture stopped completely", self.monitor_index));

        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        *self.is_running.read().unwrap()
    }
    
    // Fallback: DirectX 9 screen capture
    fn start_with_dx9(&mut self, monitor_width: u32, monitor_height: u32) -> Result<(), String> {
        let preview_width = 240u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;
        
        let pipeline_str = format!(
            "dx9screencapsrc monitor={} ! \
             video/x-raw,framerate=5/1 ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale method=0 ! \
             video/x-raw,width={},height={} ! \
             appsink name=sink",
            self.monitor_index,
            preview_width,
            preview_height
        );
        
        self.start_common_pipeline(&pipeline_str)
    }
    
    // Fallback: GDI screen capture (slowest but most compatible)
    fn start_with_gdi(&mut self, monitor_width: u32, monitor_height: u32) -> Result<(), String> {
        let preview_width = 240u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;
        
        let pipeline_str = format!(
            "gdiscreencapsrc monitor={} ! \
             video/x-raw,framerate=5/1 ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale method=0 ! \
             video/x-raw,width={},height={} ! \
             appsink name=sink",
            self.monitor_index,
            preview_width,
            preview_height
        );
        
        self.start_common_pipeline(&pipeline_str)
    }
    
    // Common pipeline setup for fallback methods
    fn start_common_pipeline(&mut self, pipeline_str: &str) -> Result<(), String> {
        let pipeline = gst::parse::launch(pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;
        
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        let frame_sender = Arc::clone(&self.frame_sender);
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    
                    if let Some(sender) = frame_sender.read().unwrap().as_ref() {
                        let _ = sender.send(map.as_slice().to_vec());
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {:?}", e))?;
        
        self.pipeline = Some(pipeline);
        *self.is_running.write().unwrap() = true;
        
        Ok(())
    }
}

impl Drop for ScreenCaptureMonitor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

