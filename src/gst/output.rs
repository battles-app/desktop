use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline, ElementFactory, Element};
use gstreamer_app::AppSrc;
use tokio::sync::broadcast;
use std::sync::Arc;
use parking_lot::RwLock;

/// Output configuration
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub width: u32,
    pub height: u32,
    pub framerate: u32,
    pub format: OutputFormat,
}

/// Output format options
#[derive(Debug, Clone)]
pub enum OutputFormat {
    Preview,
    RTMP { url: String },
    WebRTC,
    File { path: String },
}

/// GStreamer output pipeline that encodes from RGBA frames via appsrc
pub struct GStreamerOutput {
    pipeline: Option<Pipeline>,
    appsrc: Option<AppSrc>,
    config: OutputConfig,
    frame_receiver: Arc<RwLock<Option<broadcast::Receiver<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
    id: String,
}

impl GStreamerOutput {
    pub fn new(id: String, config: OutputConfig) -> Result<Self, String> {
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;

        println!("[GST Output {}] Created: {:?}", id, config.format);

        Ok(Self {
            pipeline: None,
            appsrc: None,
            config,
            frame_receiver: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            id,
        })
    }

    pub fn set_frame_receiver(&mut self, receiver: broadcast::Receiver<Vec<u8>>) {
        *self.frame_receiver.write() = Some(receiver);
    }

    pub fn start(&mut self) -> Result<(), String> {
        println!("[GST Output {}] Starting pipeline", self.id);

        let pipeline_str = match &self.config.format {
            OutputFormat::Preview => {
                // Preview pipeline: appsrc → videoconvert → autovideosink
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     videoconvert ! autovideosink",
                    self.config.width, self.config.height, self.config.framerate
                )
            }
            OutputFormat::RTMP { url } => {
                // RTMP streaming pipeline: appsrc → videoconvert → x264enc → flvmux → rtmpsink
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     videoconvert ! x264enc tune=zerolatency bitrate=4000 speed-preset=ultrafast ! \
                     flvmux ! rtmpsink location=\"{}\"",
                    self.config.width, self.config.height, self.config.framerate, url
                )
            }
            OutputFormat::WebRTC => {
                // WebRTC pipeline: appsrc → videoconvert → vp8enc → webrtcbin
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     videoconvert ! vp8enc ! webrtcbin name=webrtc",
                    self.config.width, self.config.height, self.config.framerate
                )
            }
            OutputFormat::File { path } => {
                // File output pipeline: appsrc → videoconvert → x264enc → mp4mux → filesink
                format!(
                    "appsrc name=src is-live=true format=time caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" ! \
                     videoconvert ! x264enc ! mp4mux ! filesink location=\"{}\"",
                    self.config.width, self.config.height, self.config.framerate, path
                )
            }
        };

        println!("[GST Output {}] Pipeline: {}", self.id, pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;

        // Get appsrc element
        let appsrc = pipeline.by_name("src")
            .ok_or("Failed to get appsrc")?
            .dynamic_cast::<AppSrc>()
            .map_err(|_| "Failed to cast to AppSrc")?;

        // Configure appsrc
        appsrc.set_format(gst::Format::Time);
        appsrc.set_is_live(true);

        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .field("width", self.config.width as i32)
            .field("height", self.config.height as i32)
            .field("framerate", gst::Fraction::new(self.config.framerate as i32, 1))
            .build();

        appsrc.set_caps(Some(&caps));

        self.appsrc = Some(appsrc);
        self.pipeline = Some(pipeline);

        // Start frame receiver task
        self.start_frame_receiver();

        // Start pipeline
        if let Some(pipeline) = &self.pipeline {
            pipeline.set_state(gst::State::Playing)
                .map_err(|e| format!("Failed to start pipeline: {}", e))?;
        }

        *self.is_running.write() = true;
        println!("[GST Output {}] Started successfully", self.id);
        Ok(())
    }

    fn start_frame_receiver(&self) {
        let appsrc = match &self.appsrc {
            Some(src) => src.clone(),
            None => return,
        };

        let frame_receiver = self.frame_receiver.clone();
        let is_running = self.is_running.clone();
        let output_id = self.id.clone();
        let framerate = self.config.framerate;

        tokio::spawn(async move {
            let mut receiver = match frame_receiver.write().as_mut() {
                Some(rx) => rx.resubscribe(),
                None => {
                    println!("[GST Output {}] No frame receiver set", output_id);
                    return;
                }
            };

            let frame_duration = gst::ClockTime::from_seconds(1) / framerate;
            let mut pts = gst::ClockTime::ZERO;

            println!("[GST Output {}] Frame receiver started, frame duration: {:?}", output_id, frame_duration);

            while *is_running.read() {
                match tokio::time::timeout(std::time::Duration::from_millis(100), receiver.recv()).await {
                    Ok(Ok(frame_data)) => {
                        // Create buffer from frame data
                        let mut buffer = gst::Buffer::with_size(frame_data.len())
                            .expect("Failed to create buffer");

                        {
                            let buffer_mut = buffer.get_mut().unwrap();
                            buffer_mut.set_pts(pts);
                            buffer_mut.set_duration(frame_duration);

                            let mut map = buffer_mut.map_writable().expect("Failed to map buffer");
                            map.as_mut_slice().copy_from_slice(&frame_data);
                        }

                        // Push buffer to appsrc
                        match appsrc.push_buffer(buffer) {
                            Ok(_) => {
                                // println!("[GST Output {}] Pushed frame at PTS: {:?}", output_id, pts);
                            }
                            Err(gst::FlowError::Flushing) => {
                                println!("[GST Output {}] Pipeline flushing", output_id);
                                break;
                            }
                            Err(e) => {
                                println!("[GST Output {}] Failed to push buffer: {:?}", output_id, e);
                                break;
                            }
                        }

                        pts += frame_duration;
                    }
                    Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                        println!("[GST Output {}] Frame receiver lagged by {} frames", output_id, n);
                    }
                    Ok(Err(broadcast::error::RecvError::Closed)) => {
                        println!("[GST Output {}] Frame receiver closed", output_id);
                        break;
                    }
                    Err(_) => {
                        // Timeout - continue loop to check is_running
                        continue;
                    }
                }
            }

            println!("[GST Output {}] Frame receiver stopped", output_id);
        });
    }

    pub fn stop(&mut self) -> Result<(), String> {
        *self.is_running.write() = false;

        // End of stream for appsrc
        if let Some(appsrc) = &self.appsrc {
            appsrc.end_of_stream().ok();
        }

        if let Some(pipeline) = &self.pipeline {
            pipeline.set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {}", e))?;
        }

        self.pipeline = None;
        self.appsrc = None;
        println!("[GST Output {}] Stopped", self.id);
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }

    pub fn get_config(&self) -> &OutputConfig {
        &self.config
    }

    pub fn update_config(&mut self, config: OutputConfig) -> Result<(), String> {
        let was_running = self.is_running();
        if was_running {
            self.stop()?;
        }

        self.config = config;

        if was_running {
            self.start()?;
        }

        Ok(())
    }
}

/// Manager for output destinations
pub struct OutputManager {
    outputs: std::collections::HashMap<String, GStreamerOutput>,
}

impl OutputManager {
    pub fn new() -> Self {
        Self {
            outputs: std::collections::HashMap::new(),
        }
    }

    pub fn add_output(&mut self, output: GStreamerOutput) -> Result<(), String> {
        let id = output.get_id().to_string();
        if self.outputs.contains_key(&id) {
            return Err(format!("Output with ID '{}' already exists", id));
        }
        self.outputs.insert(id, output);
        Ok(())
    }

    pub fn remove_output(&mut self, id: &str) -> Result<GStreamerOutput, String> {
        self.outputs.remove(id)
            .ok_or_else(|| format!("Output with ID '{}' not found", id))
    }

    pub fn get_output(&self, id: &str) -> Option<&GStreamerOutput> {
        self.outputs.get(id)
    }

    pub fn get_output_mut(&mut self, id: &str) -> Option<&mut GStreamerOutput> {
        self.outputs.get_mut(id)
    }

    pub fn start_output(&mut self, id: &str) -> Result<(), String> {
        let output = self.outputs.get_mut(id)
            .ok_or_else(|| format!("Output with ID '{}' not found", id))?;
        output.start()
    }

    pub fn stop_output(&mut self, id: &str) -> Result<(), String> {
        let output = self.outputs.get_mut(id)
            .ok_or_else(|| format!("Output with ID '{}' not found", id))?;
        output.stop()
    }

    pub fn stop_all(&mut self) -> Result<(), String> {
        for (id, output) in self.outputs.iter_mut() {
            if output.is_running() {
                println!("[Output Manager] Stopping output: {}", id);
                output.stop()?;
            }
        }
        Ok(())
    }

    pub fn list_outputs(&self) -> Vec<String> {
        self.outputs.keys().cloned().collect()
    }
}

impl Default for OutputManager {
    fn default() -> Self {
        Self::new()
    }
}
