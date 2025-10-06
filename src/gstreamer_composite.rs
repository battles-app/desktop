// GStreamer composite pipeline for OBS-like functionality
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline};
use gstreamer_app::AppSink;
use tokio::sync::broadcast;
use std::sync::Arc;
use parking_lot::RwLock;

pub struct GStreamerComposite {
    pipeline: Option<Pipeline>,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
    output_format: Arc<RwLock<OutputFormat>>,
    layers: Arc<RwLock<LayerSettings>>,
    fx_state: Arc<RwLock<Option<FxPlaybackState>>>,
    pipeline_fps: Arc<RwLock<u32>>,
    pipeline_width: Arc<RwLock<u32>>,
    pipeline_height: Arc<RwLock<u32>>,

    // Persistent FX branch elements (pre-wired to compositor.sink_1)
    fx_uridecodebin: Option<gst::Element>,
    fx_valve: Option<gst::Element>,
    fx_ghost_pad: Option<gst::Pad>,
}

#[derive(Clone, Debug)]
pub enum OutputFormat {
    Preview,
    VirtualCamera,
    NDI,
}

#[derive(Clone, Debug)]
pub struct LayerSettings {
    pub camera_enabled: bool,
    pub camera_opacity: f64,
    pub overlay_enabled: bool,
    pub overlay_opacity: f64,
}

#[derive(Clone, Debug)]
pub struct FxPlaybackState {
    pub file_url: String,
    pub keycolor: String,      // Hex color like "#00ff00"
    pub tolerance: f64,        // 0.0 - 1.0
    pub similarity: f64,       // 0.0 - 1.0
    pub use_chroma_key: bool,
    pub compositor_sink_pad: Option<gst::Pad>, // Store sink pad for proper cleanup
}

impl Default for LayerSettings {
    fn default() -> Self {
        Self {
            camera_enabled: true,
            camera_opacity: 1.0,
            overlay_enabled: true,
            overlay_opacity: 1.0,
        }
    }
}

impl GStreamerComposite {
    pub fn new() -> Result<Self, String> {
        // Initialize GStreamer
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
        
        println!("[Composite] Initialized successfully");
        
        Ok(Self {
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            output_format: Arc::new(RwLock::new(OutputFormat::Preview)),
            layers: Arc::new(RwLock::new(LayerSettings::default())),
            fx_state: Arc::new(RwLock::new(None)),
            pipeline_fps: Arc::new(RwLock::new(30)),
            pipeline_width: Arc::new(RwLock::new(1280)),
            pipeline_height: Arc::new(RwLock::new(720)),

            // Persistent FX elements (initialized when pipeline starts)
            fx_uridecodebin: None,
            fx_valve: None,
            fx_ghost_pad: None,
        })
    }
    
    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write() = Some(sender);
    }

    /// Create persistent FX branch wired to compositor.sink_1
    fn create_persistent_fx_branch(&mut self, pipeline: &Pipeline, compositor: &gst::Element) -> Result<(), String> {
        println!("[Composite FX] 🔧 Building persistent FX branch...");

        // Request sink_1 pad from compositor (persistent connection)
        let comp_sink_pad = compositor
            .request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink_1 pad")?;

        println!("[Composite FX] 📡 Got persistent sink_1 pad from compositor");

        // Create FX elements
        use gstreamer::ElementFactory;

        // Decoder
        let uridecode = ElementFactory::make("uridecodebin")
            .name("fx_uridecodebin")
            .build()
            .map_err(|_| "Failed to create uridecodebin")?;

        // Queues for decoupling
        let decode_queue = ElementFactory::make("queue")
            .name("fx_decode_queue")
            .property("leaky", gst::QueueLeaky::Downstream)
            .property("max-size-buffers", 2u32)
            .build()
            .map_err(|_| "Failed to create decode queue")?;

        let output_queue = ElementFactory::make("queue")
            .name("fx_output_queue")
            .property("leaky", gst::QueueLeaky::Downstream)
            .property("max-size-buffers", 2u32)
            .build()
            .map_err(|_| "Failed to create output queue")?;

        // Video processing
        let videoconvert = ElementFactory::make("videoconvert")
            .name("fx_convert")
            .property_from_str("qos", "false")
            .build()
            .map_err(|_| "Failed to create videoconvert")?;

        // RGBA caps for chroma keyer
        let rgba_caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .build();
        let rgba_filter = ElementFactory::make("capsfilter")
            .name("fx_rgba_filter")
            .property("caps", &rgba_caps)
            .build()
            .map_err(|_| "Failed to create RGBA capsfilter")?;

        // Alpha chroma keyer
        let alpha = ElementFactory::make("alpha")
            .name("fx_alpha")
            .property_from_str("method", "green")
            .property("angle", 20.0f32)  // Default similarity
            .property("noise-level", 2.0f32)  // Default smoothness
            .build()
            .map_err(|_| "Failed to create alpha keyer")?;
        let _ = alpha.set_property_from_str("qos", "false"); // Best effort

        // Video rate control
        let videorate = ElementFactory::make("videorate")
            .name("fx_videorate")
            .property("drop-only", true)
            .build()
            .map_err(|_| "Failed to create videorate")?;

        // 30fps caps
        let fps_caps = gst::Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(30, 1))
            .build();
        let fps_filter = ElementFactory::make("capsfilter")
            .name("fx_fps_filter")
            .property("caps", &fps_caps)
            .build()
            .map_err(|_| "Failed to create fps capsfilter")?;

        // Identity for timeline management
        let identity = ElementFactory::make("identity")
            .name("fx_identity")
            .property("single-segment", true)
            .property("sync", false)  // Don't block, let compositor pace
            .build()
            .map_err(|_| "Failed to create identity")?;

        // Video scaler
        let videoscale = ElementFactory::make("videoscale")
            .name("fx_scale")
            .property_from_str("qos", "false")
            .build()
            .map_err(|_| "Failed to create videoscale")?;

        // BGRA caps for compositor
        let bgra_caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .build();
        let bgra_filter = ElementFactory::make("capsfilter")
            .name("fx_bgra_filter")
            .property("caps", &bgra_caps)
            .build()
            .map_err(|_| "Failed to create BGRA capsfilter")?;

        // Valve for gating during reset
        let valve = ElementFactory::make("valve")
            .name("fx_valve")
            .property("drop", true)
            .property_from_str("drop-mode", "transform-to-gap")
            .build()
            .map_err(|_| "Failed to create valve")?;

        // Add all elements to pipeline
        pipeline.add_many(&[
            &uridecode, &decode_queue, &videoconvert, &rgba_filter, &alpha,
            &videorate, &fps_filter, &identity, &videoscale, &bgra_filter,
            &output_queue, &valve
        ]).map_err(|_| "Failed to add FX elements to pipeline")?;

        // Link the chain (except uridecodebin which connects dynamically)
        gst::Element::link_many(&[
            &decode_queue, &videoconvert, &rgba_filter, &alpha,
            &videorate, &fps_filter, &identity, &videoscale, &bgra_filter,
            &output_queue, &valve
        ]).map_err(|_| "Failed to link FX elements")?;

        // Connect valve to compositor sink_1
        let valve_src_pad = valve.static_pad("src")
            .ok_or("Failed to get valve src pad")?;
        valve_src_pad.link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link valve to compositor: {:?}", e))?;

        // Store persistent elements
        self.fx_uridecodebin = Some(uridecode.clone());
        self.fx_valve = Some(valve.clone());

        // Set compositor sink properties (persistent)
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", self.layers.read().overlay_opacity);
        comp_sink_pad.set_property("xpos", 0i32);
        comp_sink_pad.set_property("ypos", 0i32);
        comp_sink_pad.set_property("width", *self.pipeline_width.read() as i32);
        comp_sink_pad.set_property("height", *self.pipeline_height.read() as i32);

        // Create ghost pad for external control
        let valve_sink_pad = valve.static_pad("sink")
            .ok_or("Failed to get valve sink pad")?;
        let ghost_pad = gst::GhostPad::with_target(&valve_sink_pad)
            .map_err(|_| "Failed to create FX ghost pad")?;
        ghost_pad.set_active(true).ok();

        // Add ghost pad to pipeline (not to a bin, directly to pipeline)
        pipeline.add_pad(&ghost_pad)
            .map_err(|_| "Failed to add FX ghost pad to pipeline")?;

        self.fx_ghost_pad = Some(ghost_pad);

        println!("[Composite FX] ✅ Persistent FX branch wired to compositor.sink_1");
        println!("[Composite FX] 🔗 Chain: uridecodebin → queue → videoconvert → RGBA → alpha → videorate → 30fps → identity → videoscale → BGRA → queue → valve → compositor");

        Ok(())
    }

    /// Add camera branch protection against flush events
    fn protect_camera_from_flush(&self, pipeline: &Pipeline) -> Result<(), String> {
        println!("[Composite] 🛡️ Adding camera flush protection...");

        // Find the camera source element
        let camera_src = pipeline.by_name("mfvideosrc0")
            .or_else(|| pipeline.by_name("v4l2src0"))
            .or_else(|| pipeline.by_name("avfvideosrc0"))
            .ok_or("Failed to find camera source element")?;

        // Get the camera src pad
        let camera_src_pad = camera_src.static_pad("src")
            .ok_or("Failed to get camera src pad")?;

        // Add probe to drop flush events on camera branch
        let probe_id = camera_src_pad.add_probe(
            gst::PadProbeType::EVENT_DOWNSTREAM | gst::PadProbeType::EVENT_FLUSH,
            move |pad, info| {
                if let Some(ev) = info.event() {
                    use gst::EventView::*;
                    match ev.view() {
                        FlushStart(_) | FlushStop(_) => {
                            println!("[Composite] 🛡️ Dropped flush event on camera branch");
                            return gst::PadProbeReturn::Drop;
                        }
                        _ => {}
                    }
                }
                gst::PadProbeReturn::Pass
            }
        );

        if probe_id == gst::PadProbeId::INVALID {
            return Err("Failed to add camera flush protection probe".to_string());
        }

        println!("[Composite] ✅ Camera branch protected from flush events");
        Ok(())
    }
    
    pub fn update_layers(&self, camera: (bool, f64), overlay: (bool, f64)) {
        let mut layers = self.layers.write();
        layers.camera_enabled = camera.0;
        layers.camera_opacity = camera.1 / 100.0;
        layers.overlay_enabled = overlay.0;
        layers.overlay_opacity = overlay.1 / 100.0;
        
        println!("[Composite] Layers updated: camera={}/{:.2}, overlay={}/{:.2}", 
                 camera.0, camera.1, overlay.0, overlay.1);
    }
    
    pub fn start(&mut self, camera_device_id: &str, width: u32, height: u32, fps: u32, rotation: u32) -> Result<(), String> {
        println!("[Composite] Starting composite pipeline: {}x{} @ {}fps (rotation: {}°)", width, height, fps, rotation);
        
        // Stop existing pipeline if any
        if let Some(pipeline) = &self.pipeline {
            let _ = pipeline.set_state(gst::State::Null);
        }
        
        *self.is_running.write() = true;
        
        // Store pipeline dimensions and FPS
        *self.pipeline_fps.write() = fps;
        *self.pipeline_width.write() = width;
        *self.pipeline_height.write() = height;
        
        let device_index: u32 = camera_device_id.parse()
            .map_err(|_| "Invalid camera device ID")?;
        
        // Map rotation degrees to videoflip method
        // 0 = none, 1 = 90° clockwise, 2 = 180°, 3 = 90° counter-clockwise
        let videoflip_method = match rotation {
            90 => "clockwise",
            180 => "rotate-180",
            270 => "counterclockwise",
            _ => "none",
        };
        
        // Build GStreamer composite pipeline with compositor element
        // The compositor element combines multiple video streams with alpha blending
        // See: https://gstreamer.freedesktop.org/documentation/compositor/index.html
        
        #[cfg(target_os = "windows")]
        let pipeline_str = if videoflip_method != "none" {
            format!(
                "compositor name=comp background=black \
                   sink_0::zorder=0 sink_0::alpha={} \
                   sink_1::zorder=1 sink_1::alpha={} ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue leaky=downstream max-size-buffers=2 ! \
                   jpegenc quality=90 ! \
                   appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue leaky=downstream max-size-buffers=2 ! {} \
                 mfvideosrc device-index={} ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoflip method={} ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoconvert ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoscale ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 video/x-raw,width={},height={},format=BGRA ! \
                 comp.sink_0",
                self.layers.read().camera_opacity,
                self.layers.read().overlay_opacity,
                width,
                height,
                self.get_output_branch(),
                device_index,
                videoflip_method,
                width,
                height
            )
        } else {
            format!(
                "compositor name=comp background=black \
                   sink_0::zorder=0 sink_0::alpha={} \
                   sink_1::zorder=1 sink_1::alpha={} ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue leaky=downstream max-size-buffers=2 ! \
                   jpegenc quality=90 ! \
                   appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue leaky=downstream max-size-buffers=2 ! {} \
                 mfvideosrc device-index={} ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoconvert ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoscale ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 video/x-raw,width={},height={},format=BGRA ! \
                 comp.sink_0",
                self.layers.read().camera_opacity,
                self.layers.read().overlay_opacity,
                width,
                height,
                self.get_output_branch(),
                device_index,
                width,
                height
            )
        };
        
        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "compositor name=comp background=black \
               sink_0::zorder=0 sink_0::alpha={} sink_0::sync=true \
               sink_1::zorder=1 sink_1::alpha={} sink_1::sync=true ! \
             videoconvert ! \
             video/x-raw,format=BGRx,width={},height={} ! \
             tee name=t \
             t. ! queue leaky=downstream max-size-buffers=2 ! \
               jpegenc quality=90 ! \
               appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
             t. ! queue leaky=downstream max-size-buffers=2 ! {} \
             v4l2src device=/dev/video{} ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoconvert ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoscale ! \
             queue leaky=downstream max-size-buffers=3 ! \
             video/x-raw,width={},height={},format=BGRA ! \
             comp.sink_0",
            self.layers.read().camera_opacity,
            self.layers.read().overlay_opacity,
            width,
            height,
            self.get_output_branch(),
            device_index,
            width,
            height
        );
        
        println!("[Composite] ⚡ Raw composite pipeline (professional low-latency): {}", pipeline_str);
        
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;

        // Create persistent FX branch wired to compositor.sink_1
        let compositor = pipeline
            .by_name("comp")
            .ok_or("Failed to get compositor element")?;
        self.create_persistent_fx_branch(&pipeline, &compositor)?;

        // Protect camera branch from flush events
        self.protect_camera_from_flush(&pipeline)?;

        // Get the appsink for preview
        let appsink = pipeline
            .by_name("preview")
            .ok_or("Failed to get preview appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        // Set up callbacks for preview frames with comprehensive debugging
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();

        use std::sync::atomic::{AtomicU64, Ordering};
        use std::time::Instant;

        let frame_count = Arc::new(AtomicU64::new(0));
        let start_time = Arc::new(Instant::now());
        let last_log_time = Arc::new(RwLock::new(Instant::now()));
        let last_frame_count = Arc::new(AtomicU64::new(0));

        let frame_count_clone = frame_count.clone();
        let start_time_clone = start_time.clone();
        let last_log_time_clone = last_log_time.clone();
        let last_frame_count_clone = last_frame_count.clone();

        appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    let jpeg_data = map.as_slice();

                    if jpeg_data.len() > 100 {
                        let _count = frame_count_clone.fetch_add(1, Ordering::Relaxed);

                        if let Some(sender) = frame_sender.read().as_ref() {
                            let _ = sender.send(jpeg_data.to_vec());
                        }
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Start pipeline
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {}", e))?;
        
        println!("[Composite] ✅ Composite pipeline started successfully!");
        
        self.pipeline = Some(pipeline);
        Ok(())
    }
    
    fn get_output_branch(&self) -> String {
        match *self.output_format.read() {
            OutputFormat::Preview => {
                // Preview only - no output
                "fakesink".to_string()
            },
            OutputFormat::VirtualCamera => {
                // Virtual camera output
                #[cfg(target_os = "windows")]
                return "videoconvert ! video/x-raw,format=YUY2 ! dshowvideosink".to_string();
                
                #[cfg(target_os = "linux")]
                return "videoconvert ! video/x-raw,format=YUY2 ! v4l2sink device=/dev/video10".to_string();
                
                #[cfg(target_os = "macos")]
                return "fakesink".to_string();
            },
            OutputFormat::NDI => {
                // NDI output (requires gst-ndi plugin)
                "videoconvert ! ndisink".to_string()
            },
        }
    }
    
    pub fn set_output_format(&mut self, format: &str) -> Result<(), String> {
        let new_format = match format {
            "preview" => OutputFormat::Preview,
            "virtual_camera" => OutputFormat::VirtualCamera,
            "ndi" => OutputFormat::NDI,
            _ => return Err(format!("Unknown output format: {}", format)),
        };
        
        *self.output_format.write() = new_format;
        println!("[Composite] Output format changed to: {:?}", format);
        
        // Restart pipeline with new output
        // TODO: Implement dynamic pipeline reconfiguration
        
        Ok(())
    }
    
    pub fn stop(&mut self) -> Result<(), String> {
        println!("[Composite] Stopping composite pipeline");
        
        *self.is_running.write() = false;
        
        if let Some(pipeline) = &self.pipeline {
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {}", e))?;
        }
        
        self.pipeline = None;
        println!("[Composite] Composite pipeline stopped");
        
        Ok(())
    }
    
    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }

    pub fn get_pipeline_state(&self) -> Option<gst::State> {
        self.pipeline.as_ref().map(|p| p.current_state())
    }
    
    /// Play FX using persistent branch with localized flushing seek
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] 🎬 Starting FX playback with localized flush");
        println!("[Composite FX] 📁 File: {}", file_path);
        println!("[Composite FX] ⏰ Start time: {:?}", std::time::Instant::now());

        // Get persistent elements
        let uridecodebin = self.fx_uridecodebin.as_ref()
            .ok_or("FX branch not initialized")?;
        let valve = self.fx_valve.as_ref()
            .ok_or("FX valve not initialized")?;
        let ghost_pad = self.fx_ghost_pad.as_ref()
            .ok_or("FX ghost pad not initialized")?;
        let pipeline = self.pipeline.as_ref()
            .ok_or("Pipeline not running")?;


        // Update FX state
        *self.fx_state.write() = Some(FxPlaybackState {
            file_url: file_path.clone(),
            keycolor: keycolor.clone(),
            tolerance,
            similarity,
            use_chroma_key,
            compositor_sink_pad: None, // Not used in persistent setup
        });

        // Configure chroma key if enabled
        if use_chroma_key {
            println!("[Composite FX] 🎨 Configuring chroma key: color={}, tolerance={:.2}, similarity={:.2}",
                     keycolor, tolerance, similarity);

            // Get alpha element and configure
            let alpha = pipeline.by_name("fx_alpha")
                .ok_or("Alpha element not found")?;

            // Set chroma key parameters
            let rgb_color = GStreamerComposite::hex_to_rgb(&keycolor)?;
            alpha.set_property_from_str("method", "green")?;
            alpha.set_property("angle", (similarity * 2.5) as f32)?; // Map 0-100 to reasonable angle
            alpha.set_property("noise-level", (tolerance / 10.0) as f32)?; // Map to noise level
            let _ = alpha.set_property_from_str("qos", "false");

            println!("[Composite FX] ✅ Chroma key configured");
        } else {
            // Disable chroma key (set method to none or similar)
            if let Some(alpha) = pipeline.by_name("fx_alpha") {
                let _ = alpha.set_property_from_str("method", "none");
            }
        }

        // 0) Close valve to send GAPs during reset
        println!("[Composite FX] 🔒 Closing valve (sending GAPs to compositor)");
        valve.set_property("drop", true)?;

        // 1) Set new file URI
        let file_uri = format!("file:///{}", file_path.replace("\\", "/"));
        println!("[Composite FX] 📁 Setting URI: {}", file_uri);
        uridecodebin.set_property("uri", &file_uri);

        // 2) Perform localized flushing seek on FX decoder
        println!("[Composite FX] 🔄 Performing localized flushing seek...");
        uridecodebin.seek_simple(
            gst::Format::Time,
            gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT | gst::SeekFlags::ACCURATE,
            0.into()
        )?;

        println!("[Composite FX] ✅ Seek issued - FX branch reset locally");

        // 3) Add first-buffer probe to align timestamps and open valve
        let valve_weak = valve.downgrade();
        let pipeline_weak = pipeline.downgrade();
        let ghost_pad_clone = ghost_pad.clone();

        ghost_pad.add_probe(
            gst::PadProbeType::BUFFER,
            move |pad, info| {
                if let Some(gst::PadProbeData::Buffer(ref buf)) = info.data {
                    println!("[Composite FX] 🎬 First buffer received - aligning timestamps...");

                    // Align timestamp to running-time
                    if let Some(pipeline) = pipeline_weak.upgrade() {
                        if let Some(clock) = pipeline.clock() {
                            if let (Some(now), Some(pts), Some(base)) = (clock.time(), buf.pts(), pipeline.base_time()) {
                                let running = now.saturating_sub(base);
                                if running > pts {
                                    let delta = (running.nseconds() - pts.nseconds()) as i64;
                                    pad.set_offset(delta);
                                    println!("[Composite FX] ⏱️ Applied ts-offset {} ns to align FX to running-time", delta);
                                }
                            }
                        }
                    }

                    // 4) Open valve - FX now flows to compositor
                    if let Some(valve) = valve_weak.upgrade() {
                        let _ = valve.set_property("drop", false);
                        println!("[Composite FX] 🔓 Valve opened - FX streaming to compositor");
                    }

                    // Remove this probe (one-time setup)
                    return gst::PadProbeReturn::Remove;
                }
                gst::PadProbeReturn::Ok
            }
        );

        println!("[Composite FX] ✅ FX playback initiated - localized reset complete");
        println!("[Composite FX] 🎯 Camera continues unaffected, FX resets cleanly");

        Ok(())
    }

    /// Stop the currently playing FX
    pub fn stop_fx(&mut self) -> Result<(), String> {
        println!("[Composite FX] 🛑 Stopping FX playback...");

        // Close valve to stop FX flow
        if let Some(valve) = &self.fx_valve {
            valve.set_property("drop", true)?;
            println!("[Composite FX] 🔒 Valve closed - FX flow stopped");
        }

        // Clear FX state
        *self.fx_state.write() = None;

        println!("[Composite FX] ✅ FX stopped - valve closed, ready for next playback");
        Ok(())
    }

    /// Convert hex color to RGB tuple
    fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), String> {
        let hex = hex.trim_start_matches('#');

        if hex.len() != 6 {
            return Err(format!("Invalid hex color: {}", hex));
        }

        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))?;

        Ok((r, g, b))
    }
}
                            println!("[Composite FX] 🔄 Flushed media pad to reset timing");
                        }
                        
                        // Unlink
                        ghost_pad.unlink(&peer_pad).ok();
                        
                        // Release if it belonged to compositor (checked before unlink)
                        if should_release {
                            compositor.release_request_pad(&peer_pad);
                            println!("[Composite FX] ✅ Unlinked and released sink_1 pad");
                        } else {
                            println!("[Composite FX] ⚠️ Pad doesn't belong to compositor, skipping release");
                        }
                    }
                }

                // THEN stop bin (non-blocking for instant cleanup)
                let _ = bin.set_state(gst::State::Null);
                println!("[Composite FX] ✅ FX bin stopped");

                // Force NULL state on all child elements (non-blocking for speed)
                let iterator = bin.iterate_elements();
                for item in iterator {
                    if let Ok(element) = item {
                        let _ = element.set_state(gst::State::Null);
                    }
                }

                // Remove bin from pipeline (instant)
                if pipeline.remove(&bin).is_ok() {
                    println!("[Composite FX] 🧹 Memory cleanup completed");
                }
            }
        }

        // Clear FX state to prevent double-release
        *self.fx_state.write() = None;

        // Ensure pipeline is in playing state after cleanup
        pipeline.set_state(gst::State::Playing).ok();
        
        // Create NEW FX state for this playback (AFTER cleanup, BEFORE pad request)
        *self.fx_state.write() = Some(FxPlaybackState {
            file_url: file_path.clone(),
            keycolor: keycolor.clone(),
            tolerance,
            similarity,
            use_chroma_key,
            compositor_sink_pad: None, // Will be set when pad is requested
        });
        
        println!("[Composite FX] 🚀 Creating uridecodebin (no disk I/O!)...");
        
        // Create filesrc with typefind for instant format detection
        use gstreamer::ElementFactory;
        
        let file_uri = format!("file:///{}", file_path.replace("\\", "/"));
        println!("[Composite FX] 📁 File URI: {}", file_uri);
        
        // Create unique uridecodebin name for each play to prevent state carryover
        use std::time::{SystemTime, UNIX_EPOCH};
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let decode_name = format!("fxdecode_{}", timestamp);

        // Use uridecodebin for reliable decoding - disable buffering to prevent timing issues
        let uridecode = ElementFactory::make("uridecodebin")
            .name(&decode_name)
            .property("uri", &file_uri)
            .property("use-buffering", false)
            .property("download", false)  // Don't cache to disk
            .property("ring-buffer-max-size", 0u64)  // No ring buffer caching
            .build()
            .map_err(|e| format!("Failed to create uridecodebin: {}", e))?;

        // Try to reduce GPU usage by preferring software decoders
        // Note: This may not work on all systems, but worth trying
        let _ = uridecode.set_property("force-sw-decoders", &true);
        
        println!("[Composite FX] 🧹 Fresh decoder created - no caching, clean state");

        // Use identity with sync=true to enforce real-time playback based on buffer timestamps
        // This blocks and waits for each frame's timestamp to arrive in real-time
        let identity_sync = ElementFactory::make("identity")
            .name("fxsync")
            .property("sync", true)              // Block until buffer timestamp arrives in real-time
            .property("single-segment", true)    // Collapse to one timeline (no segment carry-over)
            .build()
            .map_err(|_| "Failed to create identity sync")?;

        // Force consistent 30fps output with videorate
        let videorate = ElementFactory::make("videorate")
            .name("fxvideorate")
            .property("skip-to-first", true)   // Start fresh, ignore previous state
            .property("drop-only", true)       // Only drop, never duplicate
            .build()
            .map_err(|_| "Failed to create videorate")?;

        // Force 30fps output regardless of input framerate
        let rate_caps = gst::Caps::builder("video/x-raw")
            .field("framerate", gst::Fraction::new(30, 1))
            .build();

        let rate_filter = ElementFactory::make("capsfilter")
            .name("fxratefilter")
            .property("caps", &rate_caps)
            .build()
            .map_err(|_| "Failed to create rate capsfilter")?;
        
        println!("[Composite FX] 🕐 identity sync=true added - blocks buffers to enforce real-time playback");

        let videoconvert = ElementFactory::make("videoconvert")
            .name("fxconvert")
            .property_from_str("qos", "false")  // Disable QoS to prevent catch-up
            .build()
            .map_err(|_| "Failed to create videoconvert")?;

        let videoscale = ElementFactory::make("videoscale")
            .name("fxscale")
            .property_from_str("qos", "false")  // Disable QoS to prevent catch-up
            .build()
            .map_err(|_| "Failed to create videoscale")?;

        // Add chroma key element if enabled
        let alphacolor = if use_chroma_key {
            println!("[Composite FX] 🎨 Adding chroma key: color={}, tolerance={:.2}, similarity={:.2}",
                     keycolor, tolerance, similarity);

            // Convert hex color to RGB tuple
            let rgb_color = GStreamerComposite::hex_to_rgb(&keycolor)?;

            // Create alphacolor element for real-time chroma keying
            let chroma_element = ElementFactory::make("alphacolor")
                .name("fxchromakey")
                .property("color", format!("0x{:02x}{:02x}{:02x}ff", rgb_color.0, rgb_color.1, rgb_color.2))
                .property("tolerance", tolerance as f32)
                .property("slope", 0.0f32)  // Hard key for clean edges
                .build()
                .map_err(|_| "Failed to create alphacolor element")?;

            Some(chroma_element)
        } else {
            println!("[Composite FX] 🎬 No chroma key - direct playback");
            None
        };

        // BGRA caps for compositor
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .build();

        println!("[Composite FX] 🎬 Forced 30fps H.264 MP4 playback - videorate ensures consistent timing");

        let capsfilter = ElementFactory::make("capsfilter")
            .name("fxcaps")
            .property("caps", &caps)
            .build()
            .map_err(|_| "Failed to create capsfilter")?;

        // Set uridecodebin to async for raw playback
        uridecode.set_property("async-handling", true);

        // Create bin to hold FX elements
        let fx_bin = gst::Bin::builder().name("fxbin").build();

        // Add elements to bin (conditionally include alphacolor)
        let mut elements: Vec<&gst::Element> = vec![&uridecode, &videorate, &rate_filter, &identity_sync, &videoconvert, &videoscale];
        if let Some(ref chroma) = alphacolor {
            elements.push(chroma);
        }
        elements.push(&capsfilter);

        fx_bin.add_many(&elements)
            .map_err(|_| "Failed to add elements to FX bin")?;

        // Link elements with conditional chroma key
        if let Some(ref chroma) = alphacolor {
            // Pipeline: videorate -> rate_filter -> identity_sync -> videoconvert -> videoscale -> alphacolor -> capsfilter
            gst::Element::link_many(&[&videorate, &rate_filter, &identity_sync, &videoconvert, &videoscale, chroma, &capsfilter])
                .map_err(|_| "Failed to link FX elements with chroma key")?;
        } else {
            // Pipeline: videorate -> rate_filter -> identity_sync -> videoconvert -> videoscale -> capsfilter
            gst::Element::link_many(&[&videorate, &rate_filter, &identity_sync, &videoconvert, &videoscale, &capsfilter])
                .map_err(|_| "Failed to link FX elements")?;
        }

        let final_element = capsfilter.clone();
        
        // Create ghost pad on the bin
        let final_src_pad = final_element.static_pad("src")
            .ok_or("Failed to get final element src pad")?;
        let ghost_pad = gst::GhostPad::with_target(&final_src_pad)
            .map_err(|_| "Failed to create ghost pad")?;
        ghost_pad.set_active(true).ok();
        fx_bin.add_pad(&ghost_pad).map_err(|_| "Failed to add ghost pad to bin")?;
        
        // Add EOS (End-of-Stream) probe to detect when video finishes naturally
        let fx_bin_weak = fx_bin.downgrade();
        let pipeline_weak = pipeline.downgrade();
        let compositor_weak = compositor.downgrade();
        let fx_state_weak = Arc::downgrade(&self.fx_state);
        
        ghost_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
            if let Some(gst::PadProbeData::Event(ref event)) = info.data {
                if event.type_() == gst::EventType::Eos {
                    println!("[Composite FX] 🎬 Video finished (EOS) - auto-cleaning in 100ms...");
                    
                    // Spawn cleanup task (don't block probe callback)
                    let fx_bin_weak_clone = fx_bin_weak.clone();
                    let pipeline_weak_clone = pipeline_weak.clone();
                    let compositor_weak_clone = compositor_weak.clone();
                    let fx_state_weak_clone = fx_state_weak.clone();
                    
                    std::thread::spawn(move || {
                        if let (Some(fx_bin), Some(pipeline), Some(compositor)) = 
                            (fx_bin_weak_clone.upgrade(), pipeline_weak_clone.upgrade(), compositor_weak_clone.upgrade()) {
                            
                            println!("[Composite FX] 🧹 Auto-cleanup: Unlinking from compositor...");
                            
                            // Unlink and release pad (only if still owned by compositor)
                            if let Some(ghost_pad) = fx_bin.static_pad("src") {
                                if let Some(peer_pad) = ghost_pad.peer() {
                                    // Check if pad belongs to compositor BEFORE unlinking
                                    let should_release = peer_pad.parent().as_ref() == Some(compositor.upcast_ref());
                                    
                                    if should_release {
                                        // FLUSH the media pad after EOS to reset timing (no wait = instant)
                                        peer_pad.send_event(gst::event::FlushStart::new());
                                        peer_pad.send_event(gst::event::FlushStop::new(true));
                                        println!("[Composite FX] 🔄 Flushed sink_1 after EOS");
                                    }
                                    
                                    // Unlink
                                    ghost_pad.unlink(&peer_pad).ok();
                                    
                                    // Release if it belonged to compositor (checked before unlink)
                                    if should_release {
                                        compositor.release_request_pad(&peer_pad);
                                        println!("[Composite FX] ✅ Released compositor pad via EOS cleanup");
                                    }
                                }
                            }
                            
                            // Stop and remove bin
                            fx_bin.set_state(gst::State::Null).ok();
                            let _ = fx_bin.state(Some(gst::ClockTime::from_seconds(1)));
                            pipeline.remove(&fx_bin).ok();
                            
                            // Clear FX state (garbage collection)
                            if let Some(fx_state_arc) = fx_state_weak_clone.upgrade() {
                                *fx_state_arc.write() = None;
                            }
                            
                            println!("[Composite FX] ✅ Auto-cleanup complete - memory freed, ready for next FX");
                        }
                    });
                }
            }
            gst::PadProbeReturn::Ok
        });
        
        // Add bin to pipeline
        pipeline.add(&fx_bin)
            .map_err(|_| "Failed to add FX bin to pipeline")?;
        
        // Connect uridecodebin's dynamic pads (video AND audio for proper clock sync)
        let videorate_clone = videorate.clone();

        uridecode.connect_pad_added(move |_dbin, src_pad| {
            println!("[Composite FX] 🔗 Pad added: {}", src_pad.name());

            let caps = match src_pad.current_caps() {
                Some(caps) => caps,
                None => {
                    println!("[Composite FX] ⚠️ Pad has no caps yet");
                    return;
                },
            };

            let structure = match caps.structure(0) {
                Some(s) => s,
                None => {
                    println!("[Composite FX] ⚠️ Caps has no structure");
                    return;
                },
            };

            let name = structure.name();
            println!("[Composite FX] 📹 Pad caps: {}", name);

            if name.starts_with("video/") {
                // Handle video pads - connect to videorate for rate control
                let sink_pad = videorate_clone.static_pad("sink").expect("No videorate sink pad");

                if sink_pad.is_linked() {
                    println!("[Composite FX] ⚠️ Video sink already linked");
                    return;
                }

                if let Err(e) = src_pad.link(&sink_pad) {
                    println!("[Composite FX] ❌ Failed to link video pad: {:?}", e);
                } else {
                    println!("[Composite FX] ✅ Video pad linked successfully!");
                    println!("[Composite FX] 🎬 Video stream connected - playback starting...");
                    println!("[Composite FX] ⏰ Link time: {:?}", std::time::Instant::now());
                }
            } else if name.starts_with("audio/") {
                // Skip audio pads to avoid timing interference
                println!("[Composite FX] 🔇 Audio stream detected - skipping to prevent timing conflicts");
            } else {
                // Skip other pads (subtitles, etc.)
                println!("[Composite FX] ⏭️ Skipping non-media pad: {}", name);
            }
        });
        
        // Request sink_1 pad from compositor
        let comp_sink_pad = compositor
            .request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink_1 pad")?;

        // Store the sink pad for proper cleanup
        if let Some(ref mut fx_state) = *self.fx_state.write() {
            fx_state.compositor_sink_pad = Some(comp_sink_pad.clone());
        }
        
        // Get pipeline dimensions
        let comp_width = *self.pipeline_width.read() as i32;
        let comp_height = *self.pipeline_height.read() as i32;
        
        // Calculate FX positioning: center and fill height
        // Assume 16:9 FX aspect ratio for horizontal videos
        let fx_aspect = 16.0 / 9.0;
        let comp_aspect = comp_width as f64 / comp_height as f64;
        
        let (fx_width, fx_height, fx_xpos, fx_ypos) = if comp_aspect > 1.0 {
            // Horizontal compositor (16:9): Fill full width and height
            (comp_width, comp_height, 0, 0)
        } else {
            // Vertical compositor (9:16): Fill height, center horizontally and crop edges
            let fx_width = (comp_height as f64 * fx_aspect) as i32;
            let fx_xpos = (comp_width - fx_width) / 2; // Center horizontally (will crop edges)
            (fx_width, comp_height, fx_xpos, 0)
        };
        
        println!("[Composite FX] 📐 Positioning: {}x{} at ({}, {}) in {}x{} compositor", 
                 fx_width, fx_height, fx_xpos, fx_ypos, comp_width, comp_height);
        
        // Set compositor sink properties
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", self.layers.read().overlay_opacity);
        comp_sink_pad.set_property("xpos", fx_xpos);
        comp_sink_pad.set_property("ypos", fx_ypos);
        comp_sink_pad.set_property("width", fx_width);
        comp_sink_pad.set_property("height", fx_height);
        

        // CRITICAL: Add timestamp offset probe to align media timestamps to pipeline running-time
        // This prevents "late frames" → "QoS catch-up sprint" on replays
        let pipeline_weak_ts = pipeline.downgrade();
        ghost_pad.add_probe(
            gst::PadProbeType::BUFFER,  // No BLOCK flag = instant start, no delay!
            move |pad, info| {
                if let Some(gst::PadProbeData::Buffer(ref buf)) = info.data {
                    if let Some(pipeline) = pipeline_weak_ts.upgrade() {
                        if let Some(clock) = pipeline.clock() {
                            if let (Some(now), Some(pts), Some(base)) = (clock.time(), buf.pts(), pipeline.base_time()) {
                                // running-time = clock-time - base-time
                                let running = now.saturating_sub(base);
                                
                                if running > pts {
                                    // Align media to "now" - prevents catch-up sprint
                                    let delta = (running.nseconds() - pts.nseconds()) as i64;
                                    pad.set_offset(delta);
                                    println!("[Composite FX] ⏱️ Applied ts-offset {} ns to align FX to running-time", delta);
                                } else {
                                    println!("[Composite FX] ⏱️ No ts-offset needed (pts >= running-time)");
                                }
                            }
                        }
                    }
                    // Remove this probe after first buffer (unblocks flow)
                    return gst::PadProbeReturn::Remove;
                }
                gst::PadProbeReturn::Ok
            },
        );

        // Sync FX bin state with pipeline FIRST (faster than syncing after link)
        fx_bin.sync_state_with_parent()
            .map_err(|_| "Failed to sync FX bin state".to_string())?;

        // Link FX bin to compositor (happens instantly while bin is already playing)
        ghost_pad
            .link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;

        println!("[Composite FX] ✅ FX added to pipeline - playing from file");
        println!("[Composite FX] ⏰ Pipeline ready time: {:?}", std::time::Instant::now());
        println!("[Composite FX] 🔍 Natural pipeline: uridecodebin → videoconvert → videoscale → capsfilter");
        
        Ok(())
    }

    /// Convert hex color to RGB tuple
    fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8), String> {
        let hex = hex.trim_start_matches('#');
        
        if hex.len() != 6 {
            return Err(format!("Invalid hex color: {}", hex));
        }
        
        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))?;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))?;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))?;
        
        Ok((r, g, b))
    }
}



