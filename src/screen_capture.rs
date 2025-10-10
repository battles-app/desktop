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
        
        // Check if d3d11screencapturesrc is available
        if gst::ElementFactory::find("d3d11screencapturesrc").is_none() {
            // Try alternative: dx9screencapsrc
            if gst::ElementFactory::find("dx9screencapsrc").is_some() {
                return self.start_with_dx9(monitor_width, monitor_height);
            }
            
            // Try alternative: gdiscreencapsrc (GDI)
            if gst::ElementFactory::find("gdiscreencapsrc").is_some() {
                return self.start_with_gdi(monitor_width, monitor_height);
            }
            
            return Err("No screen capture plugin available (tried d3d11screencapturesrc, dx9screencapsrc, gdiscreencapsrc)".to_string());
        }

        // Calculate preview dimensions (max 320x180 for low bandwidth)
        let preview_width = 320u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;

        // Build GStreamer pipeline for screen capture
        // Using d3d11screencapturesrc (Windows) - need to download from GPU memory first
        let pipeline_str = format!(
            "d3d11screencapturesrc monitor-index={} ! \
             video/x-raw(memory:D3D11Memory),format=BGRA ! \
             d3d11download ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
             video/x-raw,width={},height={} ! \
             appsink name=sink",
            self.monitor_index,
            preview_width,
            preview_height
        );

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;

        // Get appsink
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;

        // Set appsink properties for low latency
        appsink.set_property("max-buffers", 2u32);
        appsink.set_property("drop", true);
        appsink.set_property("emit-signals", false);

        // Set up callbacks for frame delivery
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        let _monitor_index = self.monitor_index;
        let mut _frame_count = 0u64;

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read().unwrap() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    _frame_count += 1;

                    // Broadcast frame to WebSocket
                    if let Some(sender) = &*frame_sender.read().unwrap() {
                        let _ = sender.send(map.as_slice().to_vec());
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Start pipeline
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {:?}", e))?;

        self.pipeline = Some(pipeline);
        *self.is_running.write().unwrap() = true;

        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        *self.is_running.write().unwrap() = false;

        if let Some(pipeline) = &self.pipeline {
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {:?}", e))?;
        }

        self.pipeline = None;
        *self.frame_sender.write().unwrap() = None;

        Ok(())
    }

    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        *self.is_running.read().unwrap()
    }
    
    // Fallback: DirectX 9 screen capture
    fn start_with_dx9(&mut self, monitor_width: u32, monitor_height: u32) -> Result<(), String> {
        let preview_width = 320u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;
        
        let pipeline_str = format!(
            "dx9screencapsrc monitor={} ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
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
        let preview_width = 320u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;
        
        let pipeline_str = format!(
            "gdiscreencapsrc monitor={} ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
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

