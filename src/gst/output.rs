use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;

/// Output format enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// RTMP streaming
    Rtmp,
    
    /// Virtual camera
    VirtualCamera,
    
    /// Preview only
    Preview,
}

impl OutputFormat {
    /// Convert to string
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rtmp => "rtmp",
            Self::VirtualCamera => "virtual_camera",
            Self::Preview => "preview",
        }
    }
    
    /// Parse from string
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "rtmp" => Ok(Self::Rtmp),
            "virtual_camera" => Ok(Self::VirtualCamera),
            "preview" => Ok(Self::Preview),
            _ => Err(anyhow!("Invalid output format: {}", s)),
        }
    }
}

/// Represents a GStreamer output pipeline
pub struct GstOutput {
    /// The GStreamer pipeline
    pipeline: gst::Pipeline,
    
    /// The appsrc element
    appsrc: AppSrc,
    
    /// Whether the output is running
    is_running: Arc<Mutex<bool>>,
    
    /// The output format
    format: OutputFormat,
    
    /// The output width
    width: u32,
    
    /// The output height
    height: u32,
    
    /// The output framerate
    fps: u32,
    
    /// The output URL (for RTMP)
    url: Option<String>,
}

impl GstOutput {
    /// Create a new output
    pub fn new(
        format: OutputFormat,
        width: u32,
        height: u32,
        fps: u32,
        url: Option<String>,
    ) -> Result<Self> {
        // Initialize GStreamer if not already initialized
        crate::gst::utils::init()?;
        
        // Build the pipeline string based on the format
        let pipeline_str = match format {
            OutputFormat::Rtmp => {
                let rtmp_url = url.as_deref().unwrap_or("rtmp://localhost/live/stream");
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     videoconvert ! \
                     x264enc tune=zerolatency bitrate=4000 speed-preset=ultrafast ! \
                     flvmux ! \
                     rtmpsink location=\"{}\"",
                    width, height, fps, rtmp_url
                )
            }
            OutputFormat::VirtualCamera => {
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     videoconvert ! \
                     dshowsink",
                    width, height, fps
                )
            }
            OutputFormat::Preview => {
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     fakesink",
                    width, height, fps
                )
            }
        };
        
        // Create the pipeline
        let pipeline = gst::parse::launch(&pipeline_str)?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Failed to downcast pipeline"))?;
        
        // Get the appsrc element
        let appsrc = pipeline
            .by_name("src")
            .ok_or_else(|| anyhow!("Failed to get appsrc"))?
            .downcast::<AppSrc>()
            .map_err(|_| anyhow!("Failed to downcast to AppSrc"))?;
        
        // Configure the appsrc
        appsrc.set_format(gst::Format::Time);
        appsrc.set_is_live(true);
        appsrc.set_do_timestamp(true);
        
        Ok(Self {
            pipeline,
            appsrc,
            is_running: Arc::new(Mutex::new(false)),
            format,
            width,
            height,
            fps,
            url,
        })
    }
    
    /// Start the output
    pub fn start(&self) -> Result<()> {
        // Set the pipeline to playing
        self.pipeline.set_state(gst::State::Playing)?;
        
        // Mark as running
        *self.is_running.lock().unwrap() = true;
        
        Ok(())
    }
    
    /// Stop the output
    pub fn stop(&self) -> Result<()> {
        // Mark as not running
        *self.is_running.lock().unwrap() = false;
        
        // Set the pipeline to null
        self.pipeline.set_state(gst::State::Null)?;
        
        Ok(())
    }
    
    /// Push a frame to the output
    pub fn push_frame(&self, data: &[u8], pts: u64, duration: u64) -> Result<()> {
        if !*self.is_running.lock().unwrap() {
            return Ok(());
        }
        
        // Create a buffer from the data
        let mut buffer = gst::Buffer::with_size(data.len())?;
        
        {
            let buffer_ref = buffer.get_mut().unwrap();
            
            // Set the PTS and duration
            if pts > 0 {
                buffer_ref.set_pts(gst::ClockTime::from_nseconds(pts));
            }
            
            if duration > 0 {
                buffer_ref.set_duration(gst::ClockTime::from_nseconds(duration));
            }
            
            // Copy the data to the buffer
            let mut map = buffer_ref.map_writable().map_err(|_| anyhow!("Failed to map buffer"))?;
            map.copy_from_slice(data);
        }
        
        // Push the buffer to the appsrc
        self.appsrc.push_buffer(buffer)?;
        
        Ok(())
    }
    
    /// Get the output format
    pub fn format(&self) -> OutputFormat {
        self.format
    }
    
    /// Get the output width
    pub fn width(&self) -> u32 {
        self.width
    }
    
    /// Get the output height
    pub fn height(&self) -> u32 {
        self.height
    }
    
    /// Get the output framerate
    pub fn fps(&self) -> u32 {
        self.fps
    }
    
    /// Get the output URL
    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }
}

impl Drop for GstOutput {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
