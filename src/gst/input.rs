use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use tokio::sync::broadcast;

/// Represents a GStreamer input source
pub struct GstInput {
    /// The GStreamer pipeline
    pipeline: gst::Pipeline,
    
    /// The appsink element
    appsink: AppSink,
    
    /// Whether the input is running
    is_running: Arc<Mutex<bool>>,
    
    /// The frame sender
    frame_sender: Arc<Mutex<Option<broadcast::Sender<(Vec<u8>, u64, u64)>>>>,
    
    /// The input type
    input_type: InputType,
    
    /// The input ID
    id: String,
    
    /// The input width
    width: u32,
    
    /// The input height
    height: u32,
}

/// Input type enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputType {
    /// Camera input
    Camera,
    
    /// File input
    File,
    
    /// Screen input
    Screen,
}

impl GstInput {
    /// Create a new camera input
    pub fn new_camera(
        device_id: &str,
        width: u32,
        height: u32,
        fps: u32,
        id: &str,
    ) -> Result<Self> {
        // Initialize GStreamer if not already initialized
        crate::gst::utils::init()?;
        
        // Build the pipeline string
        #[cfg(target_os = "windows")]
        let pipeline_str = format!(
            "mfvideosrc device-index={} is-live=true ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            device_id, width, height, fps
        );
        
        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "v4l2src device=/dev/video{} is-live=true ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            device_id, width, height, fps
        );
        
        #[cfg(target_os = "macos")]
        let pipeline_str = format!(
            "avfvideosrc device-index={} is-live=true ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            device_id, width, height, fps
        );
        
        // Create the pipeline
        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast pipeline"))?;
        
        // Get the appsink element
        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Failed to get appsink"))?
            .downcast::<AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;
        
        Ok(Self {
            pipeline,
            appsink,
            is_running: Arc::new(Mutex::new(false)),
            frame_sender: Arc::new(Mutex::new(None)),
            input_type: InputType::Camera,
            id: id.to_string(),
            width,
            height,
        })
    }
    
    /// Create a new file input
    pub fn new_file(
        file_path: &str,
        width: u32,
        height: u32,
        id: &str,
    ) -> Result<Self> {
        // Initialize GStreamer if not already initialized
        crate::gst::utils::init()?;
        
        // Build the pipeline string
        let pipeline_str = format!(
            "filesrc location=\"{}\" ! \
             decodebin ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={} ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            file_path, width, height
        );
        
        // Create the pipeline
        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast pipeline"))?;
        
        // Get the appsink element
        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Failed to get appsink"))?
            .downcast::<AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;
        
        Ok(Self {
            pipeline,
            appsink,
            is_running: Arc::new(Mutex::new(false)),
            frame_sender: Arc::new(Mutex::new(None)),
            input_type: InputType::File,
            id: id.to_string(),
            width,
            height,
        })
    }
    
    /// Create a new screen input
    pub fn new_screen(
        monitor_index: u32,
        width: u32,
        height: u32,
        fps: u32,
        id: &str,
    ) -> Result<Self> {
        // Initialize GStreamer if not already initialized
        crate::gst::utils::init()?;
        
        // Build the pipeline string
        #[cfg(target_os = "windows")]
        let pipeline_str = format!(
            "dx9screencapsrc monitor={} is-live=true ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            monitor_index, width, height, fps
        );
        
        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "ximagesrc xid={} is-live=true ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            monitor_index, width, height, fps
        );
        
        #[cfg(target_os = "macos")]
        let pipeline_str = format!(
            "avfvideosrc capture-screen=true capture-screen-cursor=true device-index={} is-live=true ! \
             videoconvert ! \
             videoscale ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            monitor_index, width, height, fps
        );
        
        // Create the pipeline
        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast pipeline"))?;
        
        // Get the appsink element
        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| anyhow!("Failed to get appsink"))?
            .downcast::<AppSink>()
            .map_err(|_| anyhow!("Failed to downcast to AppSink"))?;
        
        Ok(Self {
            pipeline,
            appsink,
            is_running: Arc::new(Mutex::new(false)),
            frame_sender: Arc::new(Mutex::new(None)),
            input_type: InputType::Screen,
            id: id.to_string(),
            width,
            height,
        })
    }
    
    /// Set the frame sender
    pub fn set_frame_sender(&self, sender: broadcast::Sender<(Vec<u8>, u64, u64)>) {
        *self.frame_sender.lock().unwrap() = Some(sender);
    }
    
    /// Start the input
    pub fn start(&self) -> Result<()> {
        // Set up the appsink callbacks
        let is_running = self.is_running.clone();
        let frame_sender = self.frame_sender.clone();
        
        self.appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.lock().unwrap() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    
                    // Get the PTS and duration
                    let pts = buffer.pts().unwrap_or(gst::ClockTime::ZERO).nseconds();
                    let duration = buffer.duration().unwrap_or(gst::ClockTime::ZERO).nseconds();
                    
                    // Map the buffer to read the data
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let data = map.as_slice().to_vec();
                    
                    // Send the frame to listeners
                    if let Some(sender) = frame_sender.lock().unwrap().as_ref() {
                        let _ = sender.send((data, pts, duration));
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Set the pipeline to playing
        self.pipeline.set_state(gst::State::Playing)?;
        
        // Mark as running
        *self.is_running.lock().unwrap() = true;
        
        Ok(())
    }
    
    /// Stop the input
    pub fn stop(&self) -> Result<()> {
        // Mark as not running
        *self.is_running.lock().unwrap() = false;
        
        // Set the pipeline to null
        self.pipeline.set_state(gst::State::Null)?;
        
        Ok(())
    }
    
    /// Get the input ID
    pub fn id(&self) -> &str {
        &self.id
    }
    
    /// Get the input width
    pub fn width(&self) -> u32 {
        self.width
    }
    
    /// Get the input height
    pub fn height(&self) -> u32 {
        self.height
    }
    
    /// Get the input type
    pub fn input_type(&self) -> InputType {
        self.input_type
    }
}

impl Drop for GstInput {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
