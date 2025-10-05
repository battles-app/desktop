// Clean GStreamer camera capture implementation
// Purpose: Provide high-quality camera feed with minimal overhead
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

#[derive(Clone)]
pub struct CameraInfo {
    pub id: String,
    pub name: String,
    pub description: String,
}

impl GStreamerCamera {
    pub fn new() -> Result<Self, String> {
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
        println!("[Camera] Initialized");
        
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
        
        #[cfg(target_os = "windows")]
        {
            use gstreamer::DeviceMonitor;
            
            let monitor = DeviceMonitor::new();
            let caps = gst::Caps::builder("video/x-raw").build();
            monitor.add_filter(Some("Video/Source"), Some(&caps));
            
            if monitor.start().is_err() {
                return Ok(cameras);
            }
            
            let devices = monitor.devices();
            let mut device_index = 0;
            
            for device in devices.iter() {
                if let Some(device_caps) = device.caps() {
                    if device_caps.is_empty() {
                        continue;
                    }
                    
                    let display_name = device.display_name();
                    let has_valid_path = device.properties()
                        .and_then(|props| props.get::<String>("device.path").ok())
                        .is_some();
                    
                    if has_valid_path {
                        cameras.push(CameraInfo {
                            id: device_index.to_string(),
                            name: display_name.to_string(),
                            description: "Camera".to_string(),
                        });
                        device_index += 1;
                    }
                }
            }
            
            monitor.stop();
        }
        
        Ok(cameras)
    }
    
    #[allow(dead_code)]
    pub fn start(&mut self, device_id: &str) -> Result<(), String> {
        self.start_with_quality(device_id, "high")
    }
    
    pub fn start_with_quality(&mut self, device_id: &str, quality: &str) -> Result<(), String> {
        let device_index: u32 = device_id.parse().map_err(|_| "Invalid device ID")?;
        
        // Stop existing pipeline
        if let Some(pipeline) = &self.pipeline {
            let _ = pipeline.set_state(gst::State::Null);
        }
        
        *self.is_running.write() = true;
        
        // Quality presets
        let (width, height, jpeg_quality) = match quality {
            "low" => (640, 360, 60),
            "medium" => (1280, 720, 75),
            "high" => (1280, 720, 90),
            "ultra" => (1920, 1080, 95),
            _ => (1280, 720, 90),
        };
        
        #[cfg(target_os = "windows")]
        let pipeline_str = format!(
            "mfvideosrc device-index={} ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,width={},height={} ! \
             jpegenc quality={} ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            device_index, width, height, jpeg_quality
        );
        
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;
        
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        
        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    
                    let jpeg_data = map.as_slice();
                    if jpeg_data.len() > 100 {
                        if let Some(sender) = frame_sender.read().as_ref() {
                            let _ = sender.send(jpeg_data.to_vec());
                        }
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        pipeline.set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {:?}", e))?;
        
        self.pipeline = Some(pipeline);
        println!("[Camera] Started: {}x{} @ {}% quality", width, height, jpeg_quality);
        
        Ok(())
    }
    
    pub fn stop(&mut self) -> Result<(), String> {
        *self.is_running.write() = false;
        
        if let Some(pipeline) = &self.pipeline {
            pipeline.set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {:?}", e))?;
        }
        
        self.pipeline = None;
        println!("[Camera] Stopped");
        Ok(())
    }
}

impl Drop for GStreamerCamera {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
