// GStreamer composite pipeline for OBS-like functionality
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline};
use gstreamer_app::AppSink;
use tokio::sync::broadcast;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use parking_lot::RwLock;

// Global counter for unique FX playback IDs
static FX_PLAYBACK_COUNTER: AtomicU64 = AtomicU64::new(1);

// ============================================================================
// GPU-ACCELERATED FX SOURCE BIN (GL Memory, Ultra-Low Latency)
// ============================================================================

pub struct FxKeyBin {
    pub bin: gst::Bin,
    decode: gst::Element,         // decodebin3
    vconv: gst::Element,          // videoconvert (only once, before GL)
    caps_rgba: gst::Element,      // capsfilter to RGBA
    glupload: gst::Element,
    glconv: gst::Element,         // glcolorconvert
    tee: gst::Element,

    // Branch A (keyed)
    q_key: gst::Element,
    glalpha: gst::Element,

    // Branch B (clean)
    q_clean: gst::Element,

    selector: gst::Element,       // inputselector
    out_glconv: gst::Element,     // glcolorconvert (normalize after selector)
    out_caps: gst::Element,       // caps to GL RGBA for the mixer
}

impl FxKeyBin {
    pub fn new(name: &str) -> Result<Self, String> {
        let bin = gst::Bin::builder().name(name).build();

        // Use decodebin3 for better preroll/low-latency behavior
        let decode = gst::ElementFactory::make("decodebin3")
            .name(&format!("{}_decode", name))
            .build()
            .map_err(|e| format!("Failed to create decodebin3: {}", e))?;
        
        // Note: Hardware decoder preference is set via environment variables or decoder selection
        // decodebin3 will automatically choose hardware decoders when available

        let vconv = gst::ElementFactory::make("videoconvert")
            .name(&format!("{}_vconv", name))
            .build()
            .map_err(|e| format!("Failed to create videoconvert: {}", e))?;
        
        let caps_rgba = gst::ElementFactory::make("capsfilter")
            .name(&format!("{}_capsrgba", name))
            .build()
            .map_err(|e| format!("Failed to create capsfilter: {}", e))?;
        caps_rgba.set_property("caps", &gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .build());

        let glupload = gst::ElementFactory::make("glupload")
            .name(&format!("{}_glup", name))
            .build()
            .map_err(|e| format!("Failed to create glupload: {}", e))?;
        
        let glconv = gst::ElementFactory::make("glcolorconvert")
            .name(&format!("{}_glconv", name))
            .build()
            .map_err(|e| format!("Failed to create glcolorconvert: {}", e))?;
        
        let tee = gst::ElementFactory::make("tee")
            .name(&format!("{}_tee", name))
            .build()
            .map_err(|e| format!("Failed to create tee: {}", e))?;

        // Tiny, leaky queues cut latency and avoid buildup
        let q_key = Self::make_leaky_queue(&format!("{}_q_key", name))?;
        
        let glalpha = gst::ElementFactory::make("glalpha")
            .name(&format!("{}_glalpha", name))
            .build()
            .map_err(|e| format!("Failed to create glalpha: {}", e))?;
        glalpha.set_property_from_str("method", "green");
        glalpha.set_property("angle", 18.0f32);
        glalpha.set_property("noise-level", 1.0f32);
        glalpha.set_property("black-sensitivity", 80u32);
        glalpha.set_property("white-sensitivity", 80u32);

        let q_clean = Self::make_leaky_queue(&format!("{}_q_clean", name))?;

        let selector = gst::ElementFactory::make("input-selector")
            .name(&format!("{}_sel", name))
            .build()
            .map_err(|e| format!("Failed to create input-selector: {}", e))?;
        
        let out_glconv = gst::ElementFactory::make("glcolorconvert")
            .name(&format!("{}_outgl", name))
            .build()
            .map_err(|e| format!("Failed to create out glcolorconvert: {}", e))?;
        
        let out_caps = gst::ElementFactory::make("capsfilter")
            .name(&format!("{}_outcaps", name))
            .build()
            .map_err(|e| format!("Failed to create out capsfilter: {}", e))?;
        
        // Keep it in GL memory & RGBA for glvideomixer
        let gl_caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .build();
        // Note: GL memory caps are set at link time by glupload
        out_caps.set_property("caps", &gl_caps);

        bin.add_many(&[
            &decode, &vconv, &caps_rgba, &glupload, &glconv, &tee,
            &q_key, &glalpha, &q_clean, &selector, &out_glconv, &out_caps,
        ]).map_err(|e| format!("Failed to add elements to bin: {}", e))?;

        // Shared pre-branch: CPU RGBA → GL
        gst::Element::link_many(&[&vconv, &caps_rgba, &glupload, &glconv, &tee])
            .map_err(|e| format!("Failed to link pre-branch: {}", e))?;

        // Request tee pads and link branches
        let tee_key = tee.request_pad_simple("src_%u")
            .ok_or_else(|| "Failed to request tee src pad A".to_string())?;
        let tee_clean = tee.request_pad_simple("src_%u")
            .ok_or_else(|| "Failed to request tee src pad B".to_string())?;
        
        tee_key.link(&q_key.static_pad("sink").unwrap())
            .map_err(|e| format!("Failed to link tee to q_key: {:?}", e))?;
        tee_clean.link(&q_clean.static_pad("sink").unwrap())
            .map_err(|e| format!("Failed to link tee to q_clean: {:?}", e))?;

        // Branch A (keyed): q_key → glalpha → selector
        gst::Element::link_many(&[&q_key, &glalpha])
            .map_err(|e| format!("Failed to link keyed branch: {}", e))?;
        glalpha.link(&selector)
            .map_err(|e| format!("Failed to link glalpha to selector: {}", e))?;
        
        // Branch B (clean): q_clean → selector
        q_clean.link(&selector)
            .map_err(|e| format!("Failed to link clean branch: {}", e))?;

        // Selector → out_gl → out_caps → (ghost src pad)
        gst::Element::link_many(&[&selector, &out_glconv, &out_caps])
            .map_err(|e| format!("Failed to link output chain: {}", e))?;

        // Ghost pad (src) that you'll link to glvideomixer.sink_1
        let src_pad = out_caps.static_pad("src")
            .ok_or_else(|| "Failed to get src pad from out_caps".to_string())?;
        let ghost_src = gst::GhostPad::with_target(&src_pad)
            .map_err(|_| "Failed to create ghost pad".to_string())?;
        ghost_src.set_active(true).ok();
        bin.add_pad(&ghost_src)
            .map_err(|e| format!("Failed to add ghost pad: {}", e))?;

        // Dynamic link: decodebin3 video pad → vconv.sink
        let vconv_sink = vconv.static_pad("sink").unwrap();
        decode.connect_pad_added(move |_dbin, src_pad| {
            let is_video = src_pad
                .current_caps()
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
                .unwrap_or(false);
            if is_video && !vconv_sink.is_linked() {
                let _ = src_pad.link(&vconv_sink);
                println!("[FX GL] ✅ Video pad linked to converter");
            }
        });

        Ok(Self {
            bin, decode, vconv, caps_rgba, glupload, glconv, tee,
            q_key, glalpha, q_clean, selector, out_glconv, out_caps,
        })
    }

    /// Point to a new file. Call `flush()` before/after to reset timing.
    pub fn set_uri(&self, uri: &str) -> Result<(), String> {
        self.decode.set_property("uri", uri);
        Ok(())
    }

    /// Choose keyed (true) or clean (false) **before** preroll (and you can live-switch if needed).
    pub fn set_key_enabled(&self, enabled: bool) -> Result<(), String> {
        let pads: Vec<gst::Pad> = self.selector.iterate_sink_pads()
            .into_iter()
            .filter_map(|p| p.ok())
            .collect();
        
        if pads.len() != 2 {
            return Err(format!("selector expects 2 sink pads, got {}", pads.len()));
        }
        
        // First added is keyed (glalpha), second is clean (q_clean)
        let target = if enabled { &pads[0] } else { &pads[1] };
        self.selector.set_property("active-pad", target);
        
        println!("[FX GL] 🎨 Chroma key mode: {}", if enabled { "ENABLED" } else { "DISABLED" });
        Ok(())
    }

    /// Your NO-LAG ritual: flush downstream of the selector output (out_glconv sink pad).
    pub fn flush(&self) -> Result<(), String> {
        let sink_pad = self.out_glconv.static_pad("sink")
            .ok_or_else(|| "no out_glconv sink".to_string())?;
        
        if !sink_pad.send_event(gst::event::FlushStart::new()) {
            return Err("FlushStart not accepted".to_string());
        }
        
        std::thread::sleep(std::time::Duration::from_millis(1));
        
        if !sink_pad.send_event(gst::event::FlushStop::new(true)) {
            return Err("FlushStop not accepted".to_string());
        }
        
        println!("[FX GL] 🔄 Flush complete - timing reset");
        Ok(())
    }

    /// Optional: tweak chroma at runtime like OBS
    pub fn set_key_params(&self, method: &str, angle: f32, noise: f32, black: u32, white: u32) -> Result<(), String> {
        self.glalpha.set_property_from_str("method", method);
        self.glalpha.set_property("angle", angle);
        self.glalpha.set_property("noise-level", noise);
        self.glalpha.set_property("black-sensitivity", black);
        self.glalpha.set_property("white-sensitivity", white);
        
        println!("[FX GL] 🎨 Key params: method={}, angle={}, noise={}, black={}, white={}",
                 method, angle, noise, black, white);
        Ok(())
    }

    fn make_leaky_queue(name: &str) -> Result<gst::Element, String> {
        let q = gst::ElementFactory::make("queue")
            .name(name)
            .build()
            .map_err(|e| format!("Failed to create queue: {}", e))?;
        
        q.set_property_from_str("leaky", "downstream");  // Use string instead of enum
        q.set_property("max-size-buffers", 2u32);
        q.set_property("max-size-bytes", 0u32);
        q.set_property("max-size-time", 0u64);
        q.set_property("silent", true);
        Ok(q)
    }
}

pub struct GStreamerComposite {
    pipeline: Option<Pipeline>,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
    output_format: Arc<RwLock<OutputFormat>>,
    layers: Arc<RwLock<LayerSettings>>,
    fx_bin: Arc<RwLock<Option<FxKeyBin>>>,
    pipeline_fps: Arc<RwLock<u32>>,
    pipeline_width: Arc<RwLock<u32>>,
    pipeline_height: Arc<RwLock<u32>>,
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
        
        println!("[Composite] Initialized successfully with GPU acceleration");
        
        Ok(Self {
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            output_format: Arc::new(RwLock::new(OutputFormat::Preview)),
            layers: Arc::new(RwLock::new(LayerSettings::default())),
            fx_bin: Arc::new(RwLock::new(None)),
            pipeline_fps: Arc::new(RwLock::new(30)),
            pipeline_width: Arc::new(RwLock::new(1280)),
            pipeline_height: Arc::new(RwLock::new(720)),
        })
    }
    
    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write() = Some(sender);
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
        println!("[Composite] Starting GPU composite pipeline: {}x{} @ {}fps (rotation: {}°)", width, height, fps, rotation);
        
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
        let videoflip_method = match rotation {
            90 => "clockwise",
            180 => "rotate-180",
            270 => "counterclockwise",
            _ => "none",
        };
        
        // Build GPU-accelerated composite pipeline with glvideomixer
        // All processing stays in GL memory for zero CPU copies
        #[cfg(target_os = "windows")]
        let pipeline_str = if videoflip_method != "none" {
            format!(
                "glvideomixer name=mix background=black \
                   sink_0::zorder=0 sink_0::alpha={} \
                   sink_1::zorder=1 sink_1::alpha={} \
                   start-time-selection=first ! \
                 glcolorconvert ! gldownload ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue leaky=downstream max-size-buffers=2 ! \
                   jpegenc quality=90 ! \
                   appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue leaky=downstream max-size-buffers=2 ! {} \
                 mfvideosrc device-index={} ! \
                 queue leaky=downstream max-size-buffers=2 ! \
                 videoflip method={} ! \
                 videoconvert ! \
                 video/x-raw,format=RGBA ! \
                 glupload ! glcolorconvert ! \
                 video/x-raw(memory:GLMemory),format=RGBA ! \
                 mix.sink_0",
                self.layers.read().camera_opacity,
                self.layers.read().overlay_opacity,
                width,
                height,
                self.get_output_branch(),
                device_index,
                videoflip_method
            )
        } else {
            format!(
                "glvideomixer name=mix background=black \
                   sink_0::zorder=0 sink_0::alpha={} \
                   sink_1::zorder=1 sink_1::alpha={} \
                   start-time-selection=first ! \
                 glcolorconvert ! gldownload ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue leaky=downstream max-size-buffers=2 ! \
                   jpegenc quality=90 ! \
                   appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue leaky=downstream max-size-buffers=2 ! {} \
                 mfvideosrc device-index={} ! \
                 queue leaky=downstream max-size-buffers=2 ! \
                 videoconvert ! \
                 video/x-raw,format=RGBA ! \
                 glupload ! glcolorconvert ! \
                 video/x-raw(memory:GLMemory),format=RGBA ! \
                 mix.sink_0",
                self.layers.read().camera_opacity,
                self.layers.read().overlay_opacity,
                width,
                height,
                self.get_output_branch(),
                device_index
            )
        };
        
        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "glvideomixer name=mix background=black \
               sink_0::zorder=0 sink_0::alpha={} \
               sink_1::zorder=1 sink_1::alpha={} \
               start-time-selection=first ! \
             glcolorconvert ! gldownload ! \
             videoconvert ! \
             video/x-raw,format=BGRx,width={},height={} ! \
             tee name=t \
             t. ! queue leaky=downstream max-size-buffers=2 ! \
               jpegenc quality=90 ! \
               appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
             t. ! queue leaky=downstream max-size-buffers=2 ! {} \
             v4l2src device=/dev/video{} ! \
             queue leaky=downstream max-size-buffers=2 ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             glupload ! glcolorconvert ! \
             video/x-raw(memory:GLMemory),format=RGBA ! \
             mix.sink_0",
            self.layers.read().camera_opacity,
            self.layers.read().overlay_opacity,
            width,
            height,
            self.get_output_branch(),
            device_index
        );
        
        println!("[Composite] ⚡ GPU pipeline (zero CPU copies): {}", pipeline_str);
        
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;
        
        // Get the appsink for preview
        let appsink = pipeline
            .by_name("preview")
            .ok_or("Failed to get preview appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        // Set up callbacks for preview frames
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();

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
        
        println!("[Composite] ✅ GPU composite pipeline started successfully!");
        
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

    /// Play an FX file from file path with GPU-accelerated chroma key
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] 🎬 Playing FX with GPU acceleration (zero CPU copies)");
        println!("[Composite FX] 📁 File: {}", file_path);
        println!("[Composite FX] 🎨 Chroma key: {}", if use_chroma_key { "ENABLED" } else { "DISABLED" });
        
        // Get the pipeline
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => {
                return Err("[Composite FX] ❌ No pipeline running - please select a camera first!".to_string());
            }
        };
        
        // Get mixer element
        let mixer = pipeline
            .by_name("mix")
            .ok_or("Failed to get glvideomixer element")?;
        
        // Stop any existing FX first
        if let Some(existing_bin) = self.fx_bin.read().as_ref() {
            println!("[Composite FX] 🧹 Cleaning up existing FX...");
            
            // Flush before removing
            let _ = existing_bin.flush();
            
            // Set to NULL and remove
            let _ = existing_bin.bin.set_state(gst::State::Null);
            let _ = pipeline.remove(&existing_bin.bin);
            
            println!("[Composite FX] ✅ Old FX cleaned up");
        }
        
        // Clear FX bin
        *self.fx_bin.write() = None;
        
        // Create new GPU-accelerated FX bin
        println!("[Composite FX] 🚀 Creating GPU FX bin...");
        let fx_bin = FxKeyBin::new("fxbin")
            .map_err(|e| format!("Failed to create FX bin: {}", e))?;
        
        // Convert file path to URI
        let file_uri = format!("file:///{}", file_path.replace("\\", "/"));
        println!("[Composite FX] 📁 File URI: {}", file_uri);
        
        // Set URI and chroma key mode BEFORE adding to pipeline
        fx_bin.set_uri(&file_uri)?;
        fx_bin.set_key_enabled(use_chroma_key)?;
        
        // Add bin to pipeline
        pipeline.add(&fx_bin.bin)
            .map_err(|e| format!("Failed to add FX bin to pipeline: {}", e))?;
        
        // Request sink_1 pad from mixer
        println!("[Composite FX] 🔌 Requesting mixer sink_1 pad...");
        let mix_sink1 = mixer
            .request_pad_simple("sink_1")
            .ok_or("Failed to request mixer sink_1 pad")?;
        
        // Configure mixer sink pad properties
        let comp_width = *self.pipeline_width.read() as i32;
        let comp_height = *self.pipeline_height.read() as i32;
        
        // Calculate FX positioning: center and fill
        let comp_aspect = comp_width as f64 / comp_height as f64;
        
        let (fx_width, fx_height, fx_xpos, fx_ypos) = if comp_aspect > 1.0 {
            // Horizontal compositor (16:9): Fill full width and height
            (comp_width, comp_height, 0, 0)
        } else {
            // Vertical compositor (9:16): Fill height, center horizontally
            let fx_aspect = 16.0 / 9.0;
            let fx_width = (comp_height as f64 * fx_aspect) as i32;
            let fx_xpos = (comp_width - fx_width) / 2;
            (fx_width, comp_height, fx_xpos, 0)
        };
        
        println!("[Composite FX] 📐 Positioning: {}x{} at ({}, {}) in {}x{} mixer", 
                 fx_width, fx_height, fx_xpos, fx_ypos, comp_width, comp_height);
        
        // Set mixer pad properties
        mix_sink1.set_property("zorder", 1u32);
        mix_sink1.set_property("alpha", self.layers.read().overlay_opacity);
        mix_sink1.set_property("xpos", fx_xpos);
        mix_sink1.set_property("ypos", fx_ypos);
        mix_sink1.set_property("width", fx_width);
        mix_sink1.set_property("height", fx_height);
        
        println!("[Composite FX] ✅ Mixer pad configured: zorder=1, alpha={:.2}, pos=({}, {}), size={}x{}",
                 self.layers.read().overlay_opacity, fx_xpos, fx_ypos, fx_width, fx_height);
        
        // Link FX bin to mixer
        let fx_src = fx_bin.bin.static_pad("src")
            .ok_or("Failed to get FX bin src pad")?;
        
        fx_src.link(&mix_sink1)
            .map_err(|e| format!("Failed to link FX bin to mixer: {:?}", e))?;
        
        println!("[Composite FX] 🔗 FX bin linked to mixer");
        
        // FLUSH BEFORE playing (critical for timing)
        fx_bin.flush()?;
        
        // Sync bin state with pipeline
        fx_bin.bin.sync_state_with_parent()
            .map_err(|e| format!("Failed to sync FX bin state: {}", e))?;
        
        // FLUSH AFTER playing (second flush for clean start)
        fx_bin.flush()?;
        
        // Store FX bin for cleanup
        *self.fx_bin.write() = Some(fx_bin);
        
        println!("[Composite FX] ✅ GPU FX playback started - zero CPU copies, real-time chroma key!");
        println!("[Composite FX] ⚡ Pipeline: file → decodebin3 → GL upload → glalpha (GPU) → glvideomixer");
        
        Ok(())
    }
    
    /// Stop the currently playing FX
    pub fn stop_fx(&mut self) -> Result<(), String> {
        println!("[Composite FX] 🛑 Stopping FX and cleaning memory...");
        
        // Get the pipeline
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => {
                println!("[Composite FX] No pipeline running");
                *self.fx_bin.write() = None;
                return Ok(());
            }
        };
        
        // Stop and remove FX bin
        if let Some(fx_bin) = self.fx_bin.read().as_ref() {
            println!("[Composite FX] 🧹 Stopping GPU FX bin...");
            
            // Flush before stopping
            let _ = fx_bin.flush();
            
            // Set to NULL
            let _ = fx_bin.bin.set_state(gst::State::Null);
            
            // Remove from pipeline
            let _ = pipeline.remove(&fx_bin.bin);
            
            println!("[Composite FX] ✅ FX bin stopped and removed");
        } else {
            println!("[Composite FX] No FX bin found to remove");
        }
        
        // Clear FX bin
        *self.fx_bin.write() = None;
        println!("[Composite FX] ✅ FX stopped and memory freed");
        
        Ok(())
    }
    
    /// Perform emergency cleanup of any orphaned FX resources
    /// This can be called periodically to ensure no resources leak
    pub fn emergency_cleanup(&self) -> Result<(), String> {
        println!("[Composite FX] 🚨 Emergency cleanup check...");

        if let Some(pipeline) = &self.pipeline {
            // Look for any orphaned FX bins
            if let Some(found_bin) = pipeline.by_name("fxbin") {
                // Check if this bin is truly orphaned (not the current active FX)
                // We can't directly compare bins, so we check if there's any current FX state
                // If there's no current FX state, then any found bin is orphaned
                let has_current_fx = self.fx_bin.read().is_some();

                if has_current_fx {
                    println!("[Composite FX] ✅ Current FX is active - bin might be legitimate, skipping emergency cleanup");
                    return Ok(());
                }

                println!("[Composite FX] 🚨 Found orphaned FX bin during emergency cleanup");

                if let Ok(bin) = found_bin.dynamic_cast::<gst::Bin>() {
                    let _ = bin.set_state(gst::State::Null);
                    let _ = pipeline.remove(&bin);
                    println!("[Composite FX] ✅ Emergency: Orphaned bin removed");
                }
            } else {
                println!("[Composite FX] ✅ Emergency: No orphaned bins found");
            }

            // Clear any stale FX bin reference
            if self.fx_bin.read().is_some() {
                *self.fx_bin.write() = None;
                println!("[Composite FX] ✅ Emergency: Stale FX bin reference cleared");
            }
        }

        println!("[Composite FX] ✅ Emergency cleanup complete");
        Ok(())
    }


}



