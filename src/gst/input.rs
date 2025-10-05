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
    frame_sender: Arc<RwLock<Option<broadcast::Sender<(Vec<u8>, u32, u32)>>>>,
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

    pub fn set_frame_sender(&self, sender: broadcast::Sender<(Vec<u8>, u32, u32)>) {
        *self.frame_sender.write() = Some(sender);
    }

    pub fn start(&mut self) -> Result<(), String> {
        println!("[GST Input {}] Starting pipeline", self.id);

        let pipeline_str = match &self.config.input_type {
            InputType::Camera { device_index } => {
                // Platform-specific webcam input pipeline
                #[cfg(target_os = "linux")]
                let source_element = format!("v4l2src device=/dev/video{} is-live=true", device_index);

                #[cfg(target_os = "windows")]
                let source_element = format!("ksvideosrc device-index={}", device_index);

                #[cfg(target_os = "macos")]
                let source_element = format!("avfvideosrc device-index={} is-live=true", device_index);

                format!(
                    "{} ! videoconvert ! video/x-raw,format=RGBA ! \
                     appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
                    source_element
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

                    println!("[GST Input {}] Received camera frame", input_id);

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    // Get actual frame dimensions from caps
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;
                    let structure = caps.structure(0).ok_or(gst::FlowError::Error)?;
                    let width: i32 = structure.get("width").map_err(|_| gst::FlowError::Error)?;
                    let height: i32 = structure.get("height").map_err(|_| gst::FlowError::Error)?;

                    println!("[GST Input {}] Frame buffer size: {} bytes, dimensions: {}x{}", input_id, map.size(), width, height);

                    // Send frame data to WGPU compositor
                    if let Some(sender) = frame_sender.read().as_ref() {
                        let frame_data = map.as_slice().to_vec();
                        let _ = sender.send((frame_data, width as u32, height as u32));
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
        let state_result = pipeline.set_state(gst::State::Playing);
        println!("[GST Input {}] Set pipeline state to Playing: {:?}", self.id, state_result);

        state_result.map_err(|e| format!("Failed to start pipeline: {}", e))?;

        // Wait for the pipeline to actually reach Playing state
        println!("[GST Input {}] Waiting for pipeline to reach Playing state...", self.id);
        let mut attempts = 0;
        const MAX_ATTEMPTS: u32 = 50; // 5 seconds max

        loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let (state_change_result, current_state, pending_state) = pipeline.state(None);
            println!("[GST Input {}] Pipeline state check - current: {:?}, pending: {:?}, result: {:?}", self.id, current_state, pending_state, state_change_result);

            match current_state {
                gst::State::Playing => {
                    println!("[GST Input {}] Pipeline reached Playing state successfully", self.id);
                    break;
                }
                gst::State::Paused => {
                    if pending_state == gst::State::Playing {
                        println!("[GST Input {}] Pipeline is transitioning to Playing...", self.id);
                    } else {
                        println!("[GST Input {}] Pipeline stuck in Paused state", self.id);
                    }
                }
                gst::State::Ready | gst::State::Null => {
                    println!("[GST Input {}] Pipeline in unexpected state: {:?}", self.id, current_state);
                }
                gst::State::VoidPending => {
                    println!("[GST Input {}] Pipeline state void pending", self.id);
                }
            }

            attempts += 1;
            if attempts >= MAX_ATTEMPTS {
                println!("[GST Input {}] Timeout waiting for Playing state after {} attempts", self.id, attempts);
                return Err("Timeout waiting for pipeline to reach Playing state".to_string());
            }
        }

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
