use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline, ElementFactory, Element};
use gstreamer_app::AppSink;
use tokio::sync::broadcast;
use std::sync::Arc;
use parking_lot::RwLock;

/// Input source types
#[derive(Debug, Clone)]
pub enum InputType {
    Camera { device_index: u32 },
    File { uri: String },
}

/// Input configuration
#[derive(Debug, Clone)]
pub struct InputConfig {
    pub input_type: InputType,
    pub width: u32,
    pub height: u32,
    pub framerate: u32,
    pub is_live: bool,
}

/// GStreamer input pipeline that decodes to RGBA frames via appsink
pub struct GStreamerInput {
    pipeline: Option<Pipeline>,
    config: InputConfig,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
    id: String,
}

impl GStreamerInput {
    pub fn new(id: String, config: InputConfig) -> Result<Self, String> {
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;

        println!("[GST Input {}] Created: {:?}", id, config.input_type);

        Ok(Self {
            pipeline: None,
            config,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            id,
        })
    }

    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write() = Some(sender);
    }

    pub fn start(&mut self) -> Result<(), String> {
        println!("[GST Input {}] Starting pipeline", self.id);

        let pipeline_str = match &self.config.input_type {
            InputType::Camera { device_index } => {
                // Webcam input pipeline - platform specific
                #[cfg(target_os = "linux")]
                let source_element = format!("v4l2src device=/dev/video{} is-live=true", device_index);

                #[cfg(target_os = "windows")]
                let source_element = format!("ksvideosrc device-index={}", device_index);

                #[cfg(target_os = "macos")]
                let source_element = format!("avfvideosrc device-index={} is-live=true", device_index);

                format!(
                    "{} ! videoconvert ! video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
                     appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
                    source_element, self.config.width, self.config.height, self.config.framerate
                )
            }
            InputType::File { uri } => {
                // File input pipeline: uridecodebin → videoconvert → videoscale → videorate → appsink
                format!(
                    "uridecodebin uri=\"{}\" ! \
                     videoconvert ! videoscale ! video/x-raw,format=RGBA,width={},height={} ! \
                     videorate ! video/x-raw,framerate={}/1 ! \
                     appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
                    uri, self.config.width, self.config.height, self.config.framerate
                )
            }
        };

        println!("[GST Input {}] Pipeline: {}", self.id, pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;

        // Set up appsink
        let sink = pipeline.by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;

        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        let input_id = self.id.clone();

        sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    // Send frame data to WGPU compositor
                    if let Some(sender) = frame_sender.read().as_ref() {
                        let frame_data = map.as_slice().to_vec();
                        let _ = sender.send(frame_data);
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // For file inputs, handle dynamic pad linking
        if let InputType::File { .. } = &self.config.input_type {
            let videoconvert = pipeline.by_name("videoconvert")
                .ok_or("Failed to get videoconvert element")?;

            let input_id_clone = self.id.clone();
            pipeline.by_name("uridecodebin").unwrap().connect_pad_added(move |_dbin, src_pad| {
                println!("[GST Input {}] Pad added: {}", input_id_clone, src_pad.name());

                let caps = match src_pad.current_caps() {
                    Some(caps) => caps,
                    None => {
                        println!("[GST Input {}] No caps yet", input_id_clone);
                        return;
                    }
                };

                let structure = match caps.structure(0) {
                    Some(s) => s,
                    None => {
                        println!("[GST Input {}] No structure in caps", input_id_clone);
                        return;
                    }
                };

                let media_type = structure.name();
                if !media_type.starts_with("video/") {
                    println!("[GST Input {}] Skipping non-video pad: {}", input_id_clone, media_type);
                    return;
                }

                println!("[GST Input {}] Linking video pad to videoconvert", input_id_clone);

                let sink_pad = videoconvert.static_pad("sink").unwrap();
                if sink_pad.is_linked() {
                    println!("[GST Input {}] Sink pad already linked", input_id_clone);
                    return;
                }

                match src_pad.link(&sink_pad) {
                    Ok(_) => println!("[GST Input {}] Successfully linked pad", input_id_clone),
                    Err(e) => println!("[GST Input {}] Failed to link pad: {:?}", input_id_clone, e),
                }
            });
        }

        // Start pipeline
        pipeline.set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {}", e))?;

        *self.is_running.write() = true;
        self.pipeline = Some(pipeline);

        println!("[GST Input {}] Started successfully", self.id);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        *self.is_running.write() = false;

        if let Some(pipeline) = &self.pipeline {
            pipeline.set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {}", e))?;
        }

        self.pipeline = None;
        println!("[GST Input {}] Stopped", self.id);
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }

    pub fn get_id(&self) -> &str {
        &self.id
    }

    pub fn get_config(&self) -> &InputConfig {
        &self.config
    }
}

/// Manager for multiple input sources
pub struct InputManager {
    inputs: std::collections::HashMap<String, GStreamerInput>,
}

impl InputManager {
    pub fn new() -> Self {
        Self {
            inputs: std::collections::HashMap::new(),
        }
    }

    pub fn add_input(&mut self, input: GStreamerInput) -> Result<(), String> {
        let id = input.get_id().to_string();
        if self.inputs.contains_key(&id) {
            return Err(format!("Input with ID '{}' already exists", id));
        }
        self.inputs.insert(id, input);
        Ok(())
    }

    pub fn remove_input(&mut self, id: &str) -> Result<GStreamerInput, String> {
        self.inputs.remove(id)
            .ok_or_else(|| format!("Input with ID '{}' not found", id))
    }

    pub fn get_input(&self, id: &str) -> Option<&GStreamerInput> {
        self.inputs.get(id)
    }

    pub fn get_input_mut(&mut self, id: &str) -> Option<&mut GStreamerInput> {
        self.inputs.get_mut(id)
    }

    pub fn start_input(&mut self, id: &str) -> Result<(), String> {
        let input = self.inputs.get_mut(id)
            .ok_or_else(|| format!("Input with ID '{}' not found", id))?;
        input.start()
    }

    pub fn stop_input(&mut self, id: &str) -> Result<(), String> {
        let input = self.inputs.get_mut(id)
            .ok_or_else(|| format!("Input with ID '{}' not found", id))?;
        input.stop()
    }

    pub fn stop_all(&mut self) -> Result<(), String> {
        for (id, input) in self.inputs.iter_mut() {
            if input.is_running() {
                println!("[Input Manager] Stopping input: {}", id);
                input.stop()?;
            }
        }
        Ok(())
    }

    pub fn list_inputs(&self) -> Vec<String> {
        self.inputs.keys().cloned().collect()
    }
}

impl Default for InputManager {
    fn default() -> Self {
        Self::new()
    }
}
