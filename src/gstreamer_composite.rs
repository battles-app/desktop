// GStreamer composite pipeline for OBS-like functionality
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline};
use gstreamer_app::AppSink;
use tokio::sync::broadcast;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use parking_lot::RwLock;

// Real-time GPU-accelerated chroma key FX source bin
use anyhow::{Result, anyhow};

// Global counter for unique FX playback IDs
static FX_PLAYBACK_COUNTER: AtomicU64 = AtomicU64::new(1);

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
    // Add mutex for pad operations to prevent race conditions
    pad_operation_mutex: Arc<parking_lot::Mutex<()>>,
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
    pub cleanup_in_progress: Arc<parking_lot::Mutex<bool>>, // Prevent double cleanup
    pub playback_id: u64, // Unique ID to prevent old EOS probes from interfering
}
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
    pub fn new(name: &str) -> Result<Self> {
        let bin = gst::Bin::new(Some(name));

        // Use decodebin3 for better preroll/low-latency behavior
        let decode = gst::ElementFactory::make("decodebin3").name(&format!("{name}_decode")).build()?;
        // Prefer hardware video decoders when available:
        // decodebin3::connect::autoplug-select to prefer d3d11/nv/vaapi
        decode.connect_autoplug_select(|_dbin, _pad, _caps, factory| {
            let fname = factory.name();
            if fname.starts_with("d3d11") || fname.starts_with("nv") || fname.starts_with("vaapi") {
                gst::AutoplugSelectResult::Try
            } else {
                gst::AutoplugSelectResult::Try
            }
        });

        let vconv = gst::ElementFactory::make("videoconvert").name(&format!("{name}_vconv")).build()?;
        let caps_rgba = gst::ElementFactory::make("capsfilter").name(&format!("{name}_capsrgba")).build()?;
        caps_rgba.set_property("caps", &gst::Caps::builder("video/x-raw").field("format", &"RGBA").build())?;

        let glupload = gst::ElementFactory::make("glupload").name(&format!("{name}_glup")).build()?;
        let glconv   = gst::ElementFactory::make("glcolorconvert").name(&format!("{name}_glconv")).build()?;
        let tee      = gst::ElementFactory::make("tee").name(&format!("{name}_tee")).build()?;

        // Tiny, leaky queues cut latency and avoid buildup
        let q_key   = make_leaky_queue(&format!("{name}_q_key"))?;
        let glalpha = gst::ElementFactory::make("glalpha").name(&format!("{name}_glalpha")).build()?;
        glalpha.set_property_from_str("method", "green");
        glalpha.set_property("angle", &18i32)?;
        glalpha.set_property("noise-level", &1i32)?;
        glalpha.set_property("black-sensitivity", &80i32)?;
        glalpha.set_property("white-sensitivity", &80i32)?;

        let q_clean = make_leaky_queue(&format!("{name}_q_clean"))?;

        let selector   = gst::ElementFactory::make("inputselector").name(&format!("{name}_sel")).build()?;
        let out_glconv = gst::ElementFactory::make("glcolorconvert").name(&format!("{name}_outgl")).build()?;
        let out_caps   = gst::ElementFactory::make("capsfilter").name(&format!("{name}_outcaps")).build()?;
        // Keep it in GL memory & RGBA for glvideomixer
        out_caps.set_property("caps", &gst::Caps::builder("video/x-raw")
            .features(&gst::CapsFeatures::new(&[gst::CAPS_FEATURE_MEMORY_GL]))
            .field("format", &"RGBA")
            .build()
        )?;

        bin.add_many(&[
            &decode, &vconv, &caps_rgba, &glupload, &glconv, &tee,
            &q_key, &glalpha, &q_clean, &selector, &out_glconv, &out_caps,
        ])?;

        // Shared pre-branch: CPU RGBA → GL
        gst::Element::link_many(&[&vconv, &caps_rgba, &glupload, &glconv, &tee])?;

        // Request tee pads and link branches
        let tee_key = tee.request_pad_simple("src_%u").ok_or_else(|| anyhow!("tee src pad A"))?;
        let tee_clean = tee.request_pad_simple("src_%u").ok_or_else(|| anyhow!("tee src pad B"))?;
        tee_key.link(&q_key.static_pad("sink").unwrap())?;
        tee_clean.link(&q_clean.static_pad("sink").unwrap())?;

        // Branch A (keyed): q_key → glalpha → selector
        gst::Element::link_many(&[&q_key, &glalpha, &selector])?;
        // Branch B (clean): q_clean → selector
        q_clean.link(&selector)?;

        // Selector → out_gl → out_caps → (ghost src pad)
        gst::Element::link_many(&[&selector, &out_glconv, &out_caps])?;

        // Ghost pad (src) that you'll link to glvideomixer.sink_1
        let src_pad = out_caps.static_pad("src").unwrap();
        let ghost_src = gst::GhostPad::with_target(Some("src"), &src_pad)?;
        ghost_src.set_active(true)?;
        bin.add_pad(&ghost_src)?;

        // Dynamic link: decodebin3 video pad → vconv.sink
        let vconv_sink = vconv.static_pad("sink").unwrap();
        decode.connect_pad_added(move |_dbin, src_pad| {
            let is_video = src_pad
                .current_caps()
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
                .unwrap_or(false);
            if is_video && !vconv_sink.is_linked() {
                let _ = src_pad.link(&vconv_sink);
            }
        });

        Ok(Self {
            bin, decode, vconv, caps_rgba, glupload, glconv, tee,
            q_key, glalpha, q_clean, selector, out_glconv, out_caps,
        })
    }

    /// Point to a new file. Call `flush()` before/after to reset timing.
    pub fn set_uri(&self, uri: &str) -> Result<()> {
        self.decode.set_property("uri", &uri)?;
        Ok(())
    }

    /// Choose keyed (true) or clean (false) **before** preroll (and you can live-switch if needed).
    pub fn set_key_enabled(&self, enabled: bool) -> Result<()> {
        let pads = self.selector.iterate_sink_pads().collect::<Result<Vec<_>, _>>()?;
        if pads.len() != 2 { return Err(anyhow!("selector expects 2 sink pads")); }
        // Consistent mapping: first added is keyed (q_key → glalpha), second is clean (q_clean).
        let target = if enabled { &pads[0] } else { &pads[1] };
        self.selector.set_property("active-pad", target)?;
        Ok(())
    }

    /// Your NO-LAG ritual: flush downstream of the selector output (out_glconv sink pad).
    pub fn flush(&self) -> Result<()> {
        let sink_pad = self.out_glconv.static_pad("sink").ok_or_else(|| anyhow!("no out_glconv sink"))?;
        if !sink_pad.send_event(gst::event::FlushStart::new()) {
            return Err(anyhow!("FlushStart not accepted"));
        }
        if !sink_pad.send_event(gst::event::FlushStop::new(true)) {
            return Err(anyhow!("FlushStop not accepted"));
        }
        Ok(())
    }

    /// Optional: tweak chroma at runtime like OBS
    pub fn set_key_params(&self, method: &str, angle: i32, noise: i32, black: i32, white: i32) -> Result<()> {
        self.glalpha.set_property_from_str("method", method);
        self.glalpha.set_property("angle", &angle)?;
        self.glalpha.set_property("noise-level", &noise)?;
        self.glalpha.set_property("black-sensitivity", &black)?;
        self.glalpha.set_property("white-sensitivity", &white)?;
        Ok(())
    }
}

fn make_leaky_queue(name: &str) -> Result<gst::Element> {
    let q = gst::ElementFactory::make("queue").name(name).build()?;
    q.set_property("leaky", &2i32)?;               // downstream
    q.set_property("max-size-buffers", &2u32)?;
    q.set_property("max-size-bytes", &0u32)?;
    q.set_property("max-size-time", &0u64)?;
    q.set_property("silent", &true)?;
    Ok(q)
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
            pad_operation_mutex: Arc::new(parking_lot::Mutex::new(()))
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

    /// Safely flush a media pad with proper synchronization
    fn safe_flush_pad(&self, pad: &gst::Pad, compositor: &gst::Element) -> Result<(), String> {
        // Acquire mutex to prevent concurrent pad operations
        let _guard = self.pad_operation_mutex.lock();

        // Double-check pad is still valid and belongs to compositor
        if pad.parent().as_ref() != Some(compositor.upcast_ref()) {
            println!("[Composite FX] ⚠️ Pad no longer belongs to compositor, skipping flush");
            return Ok(());
        }

        // Send flush events - these must be done atomically
        match pad.send_event(gst::event::FlushStart::new()) {
            true => {
                println!("[Composite FX] 🔄 FlushStart sent successfully");
            },
            false => {
                println!("[Composite FX] ❌ Failed to send FlushStart");
                return Err("Failed to send FlushStart event".to_string());
            }
        }

        // Small delay to ensure FlushStart is processed
        std::thread::sleep(std::time::Duration::from_millis(1));

        match pad.send_event(gst::event::FlushStop::new(true)) {
            true => {
                println!("[Composite FX] 🔄 FlushStop sent successfully");
            },
            false => {
                println!("[Composite FX] ❌ Failed to send FlushStop");
                return Err("Failed to send FlushStop event".to_string());
            }
        }

        Ok(())
    }

    /// Safely perform FX cleanup with double-cleanup prevention
    fn safe_cleanup_fx(&self, fx_bin: &gst::Bin, compositor: &gst::Element) -> Result<(), String> {
        // Check if cleanup is already in progress
        if let Some(fx_state) = self.fx_state.read().as_ref() {
            let already_cleaning = *fx_state.cleanup_in_progress.lock();
            if already_cleaning {
                println!("[Composite FX] ⚠️ Cleanup already in progress, skipping duplicate cleanup");
                return Ok(());
            }
            // Mark cleanup as in progress
            *fx_state.cleanup_in_progress.lock() = true;
        }

        println!("[Composite FX] 🧹 Performing safe cleanup...");

        // EXTRA DEFENSIVE: Check if bin is still valid and has the expected pad
        let ghost_pad = match fx_bin.static_pad("src") {
            Some(pad) => pad,
            None => {
                println!("[Composite FX] ⚠️ FX bin has no src pad, cleanup not needed");
                return Ok(());
            }
        };

        // Check if ghost pad has a peer (is linked)
        let peer_pad = match ghost_pad.peer() {
            Some(pad) => pad,
            None => {
                println!("[Composite FX] ⚠️ Ghost pad not linked, cleanup not needed");
                return Ok(());
            }
        };

        // MULTIPLE SAFETY CHECKS for pad validity
        let compositor_ref = compositor.upcast_ref();

        // Check 1: Pad still belongs to compositor
        let should_release = peer_pad.parent().as_ref() == Some(compositor_ref);

        // Check 2: Pad is still active/linked
        let is_linked = ghost_pad.is_linked();

        // Check 3: Compositor still owns this pad (check if pad parent is still compositor)
        let compositor_owns_pad = peer_pad.parent().as_ref() == Some(compositor.upcast_ref());

        println!("[Composite FX] 🛡️ Safety checks: should_release={}, is_linked={}, compositor_owns={}",
                 should_release, is_linked, compositor_owns_pad);

        if !is_linked {
            println!("[Composite FX] ⚠️ Pad not linked, skipping cleanup");
            return Ok(());
        }

        if should_release && compositor_owns_pad {
            // FLUSH the media pad to reset timing - with extra safety
            if let Err(e) = self.safe_flush_pad(&peer_pad, compositor) {
                println!("[Composite FX] ❌ Safe flush failed during cleanup: {}", e);
                // Don't fail the entire cleanup just because flush failed
            }
        }

        // Unlink pads safely
        let unlink_result = ghost_pad.unlink(&peer_pad);
        match unlink_result {
            Ok(_) => println!("[Composite FX] ✅ Successfully unlinked pads"),
            Err(e) => {
                println!("[Composite FX] ⚠️ Unlink failed (might already be unlinked): {:?}", e);
            }
        }

        // Release pad only if all safety checks pass - with extra validation
        println!("[Composite FX] 🔍 RELEASE CHECK: should_release={}, compositor_owns_pad={}, parent_check={}",
                 should_release, compositor_owns_pad, peer_pad.parent().as_ref() == Some(compositor_ref));

        if should_release && compositor_owns_pad && peer_pad.parent().as_ref() == Some(compositor_ref) {
            println!("[Composite FX] 📤 ATTEMPTING PAD RELEASE...");

            // Check what pads the compositor currently has
            println!("[Composite FX] 📊 Checking compositor pads...");
            // Note: We can't easily enumerate all pads, but we can check our specific pad

            // Extra safety: try to release the pad but don't crash if it fails
            let release_result = std::panic::catch_unwind(|| {
                println!("[Composite FX] 📤 Calling compositor.release_request_pad()...");
                let result = compositor.release_request_pad(&peer_pad);
                println!("[Composite FX] 📤 release_request_pad() returned: {:?}", result);
                result
            });

            match release_result {
                Ok(_) => {
                    println!("[Composite FX] ✅ Released compositor pad during cleanup");

                    // Verify the pad was actually released by checking if it still has a parent
                    let pad_parent = peer_pad.parent();
                    let pad_parent_after = pad_parent.as_ref();
                    println!("[Composite FX] 📊 After release: Pad parent is {:?}", pad_parent_after);
                    let pad_still_has_parent = pad_parent_after.is_some();
                    println!("[Composite FX] 📊 Pad still has parent after release: {}", pad_still_has_parent);
                },
                Err(e) => {
                    println!("[Composite FX] ❌ Pad release panicked: {:?}", e);
                    // Even if it panicked, check the pad's parent
                    let pad_parent = peer_pad.parent();
                    let pad_parent_after_panic = pad_parent.as_ref();
                    println!("[Composite FX] 📊 After panic: Pad parent is {:?}", pad_parent_after_panic);
                },
            }
        } else {
            println!("[Composite FX] ⚠️ Pad already released or safety checks failed - not attempting release");
        }

        Ok(())
    }
    
    /// Play an FX file from file path (file already written by main.rs, NO I/O while locked!)
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] 🎬 Playing FX from file (CPU-based with timer fixes)");
        println!("[Composite FX] 📁 File: {}", file_path);
        println!("[Composite FX] ⏰ Start time: {:?}", std::time::Instant::now());
        
        // Get the pipeline
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => {
                return Err("[Composite FX] ❌ No pipeline running - please select a camera first!".to_string());
            }
        };
        
        // Get compositor element
        let compositor = pipeline
            .by_name("comp")
            .ok_or("Failed to get compositor element")?;
        
        // Stop any existing FX first (proper cleanup with safe pad operations)
        if let Some(existing_fx_bin) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Proper cleanup of existing FX pipeline (manual)...");

            // Cast to Bin and perform complete cleanup including pad release
            if let Ok(bin) = existing_fx_bin.dynamic_cast::<gst::Bin>() {
                // First try safe cleanup with pad operations
                if let Err(e) = self.safe_cleanup_fx(&bin, &compositor) {
                    println!("[Composite FX] ❌ Safe cleanup failed: {}, trying emergency cleanup", e);

                    // Emergency cleanup: force removal without pad operations
                    let _ = bin.set_state(gst::State::Null);
                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] 🧹 Emergency: FX bin removed from pipeline");
                        } else {
                                println!("[Composite FX] ⚠️ Emergency: FX bin removal failed");
                        }
                    }
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Pipeline removal panicked: {:?}", e),
                }
                } else {
                    // Safe cleanup succeeded, now remove the bin
                let _ = bin.set_state(gst::State::Null);
                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] 🧹 FX bin removed from pipeline after safe cleanup");
                            } else {
                                println!("[Composite FX] ⚠️ FX bin removal failed after safe cleanup");
                            }
                        }
                        Err(e) => println!("[Composite FX] ⚠️ Pipeline removal panicked after safe cleanup: {:?}", e),
                    }
                }
            }
        }

        // Clear FX state to prevent double-release
        *self.fx_state.write() = None;

        // Ensure pipeline is in playing state after cleanup
        pipeline.set_state(gst::State::Playing).ok();
        
        // Create NEW FX state for this playback (AFTER cleanup, BEFORE pad request)
        // Generate unique playback ID for this FX
        let playback_id = FX_PLAYBACK_COUNTER.fetch_add(1, Ordering::SeqCst);

        *self.fx_state.write() = Some(FxPlaybackState {
            file_url: file_path.clone(),
            keycolor: keycolor.clone(),
            tolerance,
            similarity,
            use_chroma_key,
            compositor_sink_pad: None, // Will be set when pad is requested
            cleanup_in_progress: Arc::new(parking_lot::Mutex::new(false)),
            playback_id,
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
        let bin_name = format!("fxbin_{}", timestamp);
        println!("[Composite FX] 🏷️ Unique bin name: {}", bin_name);

        // Build ultra-low-latency FX pipeline: decode → convert → scale → filter
        let fx_bin = gst::Bin::new(Some(&bin_name));

        // Create elements with unique names to prevent state carryover
        let decode = gst::ElementFactory::make("uridecodebin").name(&format!("decode_{}", timestamp)).build()
            .map_err(|_| "Failed to create uridecodebin")?;
        let vconv = gst::ElementFactory::make("videoconvert").name(&format!("vconv_{}", timestamp)).build()
            .map_err(|_| "Failed to create videoconvert")?;
        let vscale = gst::ElementFactory::make("videoscale").name(&format!("vscale_{}", timestamp)).build()
            .map_err(|_| "Failed to create videoscale")?;
        let vrate = gst::ElementFactory::make("videorate").name(&format!("vrate_{}", timestamp)).build()
            .map_err(|_| "Failed to create videorate")?;
        let caps = gst::ElementFactory::make("capsfilter").name(&format!("caps_{}", timestamp)).build()
            .map_err(|_| "Failed to create capsfilter")?;
        let identity = gst::ElementFactory::make("identity").name(&format!("identity_{}", timestamp)).build()
            .map_err(|_| "Failed to create identity")?;

        // Configure elements for low latency
        decode.set_property("uri", &file_uri);
        decode.set_property("use-buffering", &false); // No buffering for instant playback

        // Force 30fps playback for consistent timing
        caps.set_property("caps", &gst::Caps::builder("video/x-raw")
            .field("framerate", &gst::Fraction::new(30, 1))
            .build());

        // Configure identity for real-time sync (blocks buffers to enforce timing)
        identity.set_property("sync", &true);

        // Add elements to bin
        fx_bin.add_many(&[&decode, &vconv, &vscale, &vrate, &caps, &identity])
            .map_err(|_| "Failed to add elements to FX bin")?;

        // Link static elements (dynamic decode pad links to vconv later)
        gst::Element::link_many(&[&vconv, &vscale, &vrate, &caps, &identity])
            .map_err(|_| "Failed to link FX elements")?;

        // Create ghost src pad from identity
        let final_src_pad = identity.static_pad("src").unwrap();
        let ghost_pad = gst::GhostPad::with_target(&final_src_pad)
            .map_err(|_| "Failed to create ghost pad")?;
        ghost_pad.set_active(true).map_err(|_| "Failed to activate ghost pad")?;
        fx_bin.add_pad(&ghost_pad).map_err(|_| "Failed to add ghost pad")?;

        // Dynamic link: decodebin3 video pad → vconv.sink
        let vconv_sink = vconv.static_pad("sink").unwrap();
        decode.connect_pad_added(move |_dbin, src_pad| {
            let is_video = src_pad
                .current_caps()
                .and_then(|c| c.structure(0).map(|s| s.name().starts_with("video/")))
                .unwrap_or(false);
            if is_video && !vconv_sink.is_linked() {
                let _ = src_pad.link(&vconv_sink);
            }
        });

        // Add FX bin to pipeline
        pipeline.add(&fx_bin).map_err(|e| format!("Failed to add FX bin to pipeline: {:?}", e))?;

        // Configure compositor pad properties BEFORE linking
        let comp_sink_pad = compositor
            .request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink pad")?;

        // Set pad properties for positioning and opacity
        let comp_width = *self.pipeline_width.read() as i32;
        let comp_height = *self.pipeline_height.read() as i32;

        // Calculate FX positioning: center and fill height
        // Assume 16:9 FX aspect ratio for horizontal videos
        let fx_aspect = 16.0 / 9.0;
        let comp_aspect = comp_width as f64 / comp_height as f64;

        let (fx_width, fx_height, fx_xpos, fx_ypos) = if comp_aspect > fx_aspect {
            // Compositor is wider than FX aspect ratio: fit to height, center horizontally
            let fx_height = comp_height;
            let fx_width = (comp_height as f64 * fx_aspect) as i32;
            let fx_xpos = (comp_width - fx_width) / 2;
            (fx_width, fx_height, fx_xpos, 0)
        } else {
            // Compositor is taller than FX aspect ratio: fit to width, center vertically
            let fx_width = comp_width;
            let fx_height = (comp_width as f64 / fx_aspect) as i32;
            let fx_ypos = (comp_height - fx_height) / 2;
            (fx_width, fx_height, 0, fx_ypos)
        };

        // Set pad properties
        comp_sink_pad.set_property("zorder", &1i32);
        comp_sink_pad.set_property("alpha", &1.0f64);
        comp_sink_pad.set_property("xpos", &fx_xpos);
        comp_sink_pad.set_property("ypos", &fx_ypos);
        comp_sink_pad.set_property("width", &fx_width);
        comp_sink_pad.set_property("height", &fx_height);

        // Store the sink pad for proper cleanup
        if let Some(ref mut fx_state) = *self.fx_state.write() {
            fx_state.compositor_sink_pad = Some(comp_sink_pad.clone());
        }

        // Verify properties were set
        println!("[Composite FX] ✅ Pad properties set: zorder=1, alpha={:.2}, pos=({}, {}), size={}x{}",
                 1.0, fx_xpos, fx_ypos, fx_width, fx_height);

        // CRITICAL: Add timestamp offset probe to align media timestamps to pipeline running-time
        // This prevents "late frames" → "QoS catch-up sprint" on replays
        println!("[Composite FX] ⏱️ Setting up timestamp offset probe...");
        let pipeline_weak_ts = pipeline.downgrade();

        let probe_result = std::panic::catch_unwind(|| {
            ghost_pad.add_probe(
                gst::PadProbeType::BUFFER,  // No BLOCK flag = instant start, no delay!
                move |pad, info| {
                    // Add panic protection inside the probe callback too
                    let result = std::panic::catch_unwind(|| {
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
                            gst::PadProbeReturn::Remove
                        } else {
                            gst::PadProbeReturn::Ok
                        }
                    });

                    match result {
                        Ok(ret) => ret,
                        Err(e) => {
                            println!("[Composite FX] ❌ Timestamp probe panicked: {:?}", e);
                            gst::PadProbeReturn::Remove
                        }
                    }
                },
            )
        });

        match probe_result {
            Ok(_) => println!("[Composite FX] ✅ Timestamp offset probe added successfully"),
            Err(e) => println!("[Composite FX] ⚠️ Failed to add timestamp probe: {:?}", e),
        }

        // Sync FX bin state with pipeline FIRST (faster than syncing after link)
        fx_bin.sync_state_with_parent()
            .map_err(|_| "Failed to sync FX bin state".to_string())?;

        // Link FX bin to compositor (happens instantly while bin is already playing)
        println!("[Composite FX] 🔗 Linking ghost pad to compositor sink pad...");
        ghost_pad
            .link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;

        println!("[Composite FX] ✅ Pad linking successful!");
        println!("[Composite FX] 🔗 Link status: ghost_pad.is_linked()={}, comp_sink_pad.is_linked()={}",
                 ghost_pad.is_linked(), comp_sink_pad.is_linked());
        
        // Add EOS (End-of-Stream) probe to detect when video finishes naturally
        println!("[Composite FX] 📡 Adding EOS probe for auto-cleanup (playback_id: {})...", playback_id);
        let fx_bin_weak = fx_bin.downgrade();
        let pipeline_weak = pipeline.downgrade();
        let fx_state_weak = Arc::downgrade(&self.fx_state);
        let pad_mutex_weak = Arc::downgrade(&self.pad_operation_mutex);
        let eos_playback_id = playback_id; // Capture current playback ID
        
        ghost_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
            if let Some(gst::PadProbeData::Event(ref event)) = info.data {
                if event.type_() == gst::EventType::Eos {
                    println!("[Composite FX] 🎬 Video finished (EOS) - auto-cleaning in 100ms...");
                    
                    // Spawn cleanup task (don't block probe callback)
                    let fx_bin_weak_clone = fx_bin_weak.clone();
                    let pipeline_weak_clone = pipeline_weak.clone();
                    let fx_state_weak_clone = fx_state_weak.clone();
                    let pad_mutex_weak_clone = pad_mutex_weak.clone();
                    
                    std::thread::spawn(move || {
                        // Check if this EOS event is for the current FX playback
                        let is_current_fx = if let Some(fx_state_arc) = fx_state_weak_clone.upgrade() {
                            let fx_state = fx_state_arc.read();
                            if let Some(state) = fx_state.as_ref() {
                                if state.playback_id != eos_playback_id {
                                    println!("[Composite FX] ⚠️ EOS cleanup skipped - this is for old FX playback (got {}, current {})",
                                             eos_playback_id, state.playback_id);
                                    return;
                                }
                                true
                            } else {
                                false // No current FX state
                            }
                        } else {
                            false // State was dropped
                        };

                        if !is_current_fx {
                            println!("[Composite FX] ⚠️ EOS cleanup skipped - no current FX state");
                            return;
                        }

                        // Check if cleanup is already in progress to prevent double cleanup
                        let cleanup_already_started = if let Some(fx_state_arc) = fx_state_weak_clone.upgrade() {
                            let fx_state = fx_state_arc.read();
                            if let Some(state) = fx_state.as_ref() {
                                let already_cleaning = *state.cleanup_in_progress.lock();
                                if already_cleaning {
                                    println!("[Composite FX] ⚠️ EOS cleanup skipped - cleanup already in progress");
                                    return;
                                }
                                // Mark cleanup as started
                                *state.cleanup_in_progress.lock() = true;
                                false // Not already started, we just marked it
                            } else {
                                true // No state, assume cleanup already happened
                            }
                        } else {
                            true // State was dropped, assume cleanup already happened
                        };

                        if cleanup_already_started {
                            return;
                        }

                        if let (Some(fx_bin), Some(pipeline), Some(pad_mutex)) =
                            (fx_bin_weak_clone.upgrade(), pipeline_weak_clone.upgrade(), pad_mutex_weak_clone.upgrade()) {

                            // Check if this bin is still actually in the pipeline (might have been manually cleaned up)
                            // First check if bin still has a parent (basic check)
                            let has_parent = fx_bin.parent().is_some();

                            // Also check if the bin's ghost pad is still linked (more reliable indicator of cleanup status)
                            let ghost_pad_still_linked = if let Some(ghost_pad) = fx_bin.static_pad("src") {
                                ghost_pad.is_linked()
                            } else {
                                false
                            };

                            let bin_still_active = has_parent && ghost_pad_still_linked;

                            if !bin_still_active {
                                println!("[Composite FX] ⚠️ EOS cleanup skipped - bin already cleaned up (no parent or ghost pad not linked)");
                                return;
                            }

                            println!("[Composite FX] 🧹 EOS Auto-cleanup: Clearing FX state...");

                            // Clear FX state (garbage collection)
                            if let Some(fx_state_arc) = fx_state_weak_clone.upgrade() {
                                *fx_state_arc.write() = None;
                                println!("[Composite FX] ✅ FX state cleared");
                            }
                        }
                    });

                    gst::PadProbeReturn::Ok
                } else {
                    gst::PadProbeReturn::Ok
                }
            } else {
                gst::PadProbeReturn::Ok
            }
        });

        println!("[Composite FX] ✅ FX added to pipeline - playing from file");
        println!("[Composite FX] ⏰ Pipeline ready time: {:?}", std::time::Instant::now());
        println!("[Composite FX] 🔍 Natural pipeline: uridecodebin → videoconvert → videoscale → capsfilter");

        Ok(())
    }

    /// Stop the currently playing FX
        println!("[Composite FX] 🛑 Stopping FX and cleaning memory...");

        // Get the pipeline
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => {
                println!("[Composite FX] No pipeline running");
                *self.fx_state.write() = None;
                return Ok(());
            }
        };

        // Get compositor element
        let compositor = match pipeline.by_name("comp") {
            Some(c) => c,
            None => {
                println!("[Composite FX] No compositor found");
                *self.fx_state.write() = None;
                return Ok(());
            }
        };

        // Clear FX state first to prevent interference
        *self.fx_state.write() = None;

        // If using FxKeyBin, just clear the state (the bin stays in pipeline for reuse)
        if self.fx_bin.is_some() {
            println!("[Composite FX] ✅ GPU FX state cleared - bin remains for reuse");
            return Ok(());
        }

        // Legacy cleanup for old uridecodebin approach (if fx_bin is None)
        if let Some(existing_fx_bin) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Legacy cleanup of existing FX pipeline (manual)...");

            if let Ok(bin) = existing_fx_bin.dynamic_cast::<gst::Bin>() {
                // First try safe cleanup with pad operations
                if let Err(e) = self.safe_cleanup_fx(&bin, &compositor) {
                    println!("[Composite FX] ❌ Safe cleanup failed: {}, trying emergency cleanup", e);

                    // Emergency cleanup: force removal without pad operations
                    let _ = bin.set_state(gst::State::Null);
                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] 🧹 Emergency: FX bin removed from pipeline");
                            } else {
                                println!("[Composite FX] ⚠️ Emergency: FX bin removal failed");
                            }
                        }
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Pipeline removal panicked: {:?}", e),
                    }
                } else {
                    // Safe cleanup succeeded, now remove the bin
                    let _ = bin.set_state(gst::State::Null);
                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] 🧹 FX bin removed from pipeline after safe cleanup");
                            } else {
                                println!("[Composite FX] ⚠️ FX bin removal failed after safe cleanup");
                            }
                        }
                        Err(e) => println!("[Composite FX] ⚠️ Pipeline removal panicked after safe cleanup: {:?}", e),
                    }
                }
            }
        }

        Ok(())
    }

    /// Perform emergency cleanup of any orphaned FX resources
    /// This can be called periodically to ensure no resources leak
    pub fn emergency_cleanup(&self) -> Result<(), String> {
        println!("[Composite FX] 🚨 Emergency cleanup check...");

        if let Some(pipeline) = &self.pipeline {
            // Look for any orphaned FX bins
            if let Some(found_bin) = pipeline.by_name("fxbin") {
                // Check if this bin is the current active FX
                // We can't directly compare bins, so we check if there's any current FX state
                // If there's no current FX state, then any found bin is orphaned
                let has_current_fx = self.fx_state.read().is_some();

                if has_current_fx {
                    println!("[Composite FX] ✅ Current FX is active - bin might be legitimate, skipping emergency cleanup");
                    return Ok(());
                }

                println!("[Composite FX] 🚨 Found orphaned FX bin during emergency cleanup");

                if let Ok(bin) = found_bin.dynamic_cast::<gst::Bin>() {
                    // Try safe cleanup first
                    if let Some(compositor) = pipeline.by_name("comp") {
                        let cleanup_result = self.safe_cleanup_fx(&bin, &compositor);
                        if cleanup_result.is_err() {
                            println!("[Composite FX] 🚨 Safe cleanup failed, forcing removal");
                        }
                    }

                    // Force removal regardless
                    let set_state_result = std::panic::catch_unwind(|| {
                        bin.set_state(gst::State::Null)
                    });

                    match set_state_result {
                        Ok(_) => println!("[Composite FX] ✅ Emergency: Bin set to NULL"),
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Set state panicked: {:?}", e),
                    }

                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] ✅ Emergency: Orphaned bin removed");
                            } else {
                                println!("[Composite FX] ⚠️ Emergency: Bin removal failed");
                            }
                        }
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Removal panicked: {:?}", e),
                    }

                    // Clear any stale FX state
                    *self.fx_state.write() = None;
                    println!("[Composite FX] ✅ Emergency: Stale FX state cleared");
                }
            } else {
                println!("[Composite FX] ✅ Emergency: No orphaned bins found");
            }
        }

        println!("[Composite FX] ✅ Emergency cleanup complete");
        Ok(())
    }
                            if let Some(state) = fx_state.as_ref() {
                                let already_cleaning = *state.cleanup_in_progress.lock();
                                if already_cleaning {
                                    println!("[Composite FX] ⚠️ EOS cleanup skipped - cleanup already in progress");
                                    return;
                                }
                                // Mark cleanup as started
                                *state.cleanup_in_progress.lock() = true;
                                false // Not already started, we just marked it
                            } else {
                                true // No state, assume cleanup already happened
                            }
                        } else {
                            true // State was dropped, assume cleanup already happened
                        };

                        if cleanup_already_started {
                            return;
                        }

                        if let (Some(fx_bin), Some(pipeline), Some(compositor), Some(pad_mutex)) =
                            (fx_bin_weak_clone.upgrade(), pipeline_weak_clone.upgrade(), compositor_weak_clone.upgrade(), pad_mutex_weak_clone.upgrade()) {

                            // Check if this bin is still actually in the pipeline (might have been manually cleaned up)
                            // First check if bin still has a parent (basic check)
                            let has_parent = fx_bin.parent().is_some();

                            // Also check if the bin's ghost pad is still linked (more reliable indicator of cleanup status)
                            let ghost_pad_still_linked = if let Some(ghost_pad) = fx_bin.static_pad("src") {
                                ghost_pad.is_linked()
                            } else {
                                false
                            };

                            let bin_still_active = has_parent && ghost_pad_still_linked;

                            if !bin_still_active {
                                println!("[Composite FX] ⚠️ EOS cleanup skipped - bin already cleaned up (no parent or ghost pad not linked)");
                                return;
                            }

                            println!("[Composite FX] 🧹 EOS Auto-cleanup: Starting defensive cleanup...");

                            // DEFENSIVE CLEANUP: Multiple safety checks before touching pads
                            let ghost_pad = match fx_bin.static_pad("src") {
                                Some(pad) => pad,
                                None => {
                                    println!("[Composite FX] ⚠️ EOS: FX bin has no src pad");
                                    return;
                                }
                            };

                            let peer_pad = match ghost_pad.peer() {
                                Some(pad) => pad,
                                None => {
                                    println!("[Composite FX] ⚠️ EOS: Ghost pad not linked");
                                    return;
                                }
                            };

                            // MULTIPLE SAFETY CHECKS (same as safe_cleanup_fx)
                            let compositor_ref = compositor.upcast_ref();
                            let should_release = peer_pad.parent().as_ref() == Some(compositor_ref);
                            let is_linked = ghost_pad.is_linked();
                            let compositor_owns_pad = peer_pad.parent().as_ref() == Some(compositor_ref);

                            println!("[Composite FX] 🛡️ EOS Safety checks: should_release={}, is_linked={}, compositor_owns={}",
                                     should_release, is_linked, compositor_owns_pad);

                            // MINIMAL EOS CLEANUP: Only set bin state and remove from pipeline
                            // Avoid pad operations during EOS as they can cause crashes
                            println!("[Composite FX] 🔄 EOS: Minimal cleanup - avoiding pad operations to prevent crashes");

                            // Just ensure the bin gets cleaned up later by the manual cleanup
                            // Don't touch pads during EOS callback to avoid race conditions

                            // Stop and remove bin - with extra safety
                            let _ = fx_bin.set_state(gst::State::Null);
                            let _ = fx_bin.state(Some(gst::ClockTime::from_seconds(1)));

                            // Safe pipeline removal with panic protection
                            let remove_result = std::panic::catch_unwind(|| {
                                pipeline.remove(&fx_bin)
                            });

                            match remove_result {
                                Ok(result) => {
                                    if result.is_ok() {
                                        println!("[Composite FX] ✅ FX bin removed from pipeline");
                                    } else {
                                        println!("[Composite FX] ⚠️ FX bin removal returned error");
                                    }
                                }
                                Err(e) => println!("[Composite FX] ⚠️ Pipeline removal panicked (but continuing): {:?}", e),
                            }

                            // Check for timeout before finishing
                            
                            // Clear FX state (garbage collection)
                            if let Some(fx_state_arc) = fx_state_weak_clone.upgrade() {
                                *fx_state_arc.write() = None;
                                println!("[Composite FX] ✅ FX state cleared");
                            }

                            // Schedule a delayed cleanup for any remaining resources
                            let pipeline_weak_delayed = pipeline_weak_clone.clone();
                            std::thread::spawn(move || {
                                std::thread::sleep(std::time::Duration::from_millis(100)); // Small delay
                                if let Some(pipeline) = pipeline_weak_delayed.upgrade() {
                                    // Check if there are any orphaned FX bins that need cleanup
                                    if let Some(orphaned_bin) = pipeline.by_name("fxbin") {
                                        println!("[Composite FX] 🧹 Found orphaned FX bin, performing delayed cleanup...");
                                        if let Ok(bin) = orphaned_bin.dynamic_cast::<gst::Bin>() {
                                            let _ = bin.set_state(gst::State::Null);
                                            let _ = pipeline.remove(&bin);
                                            println!("[Composite FX] ✅ Orphaned FX bin cleaned up");
                                        }
                                    }
                                }
                            });

                            println!("[Composite FX] ✅ EOS Auto-cleanup complete - memory freed, ready for next FX");
                        } else {
                            println!("[Composite FX] ⚠️ EOS cleanup: Some weak references were dropped, cleanup may have already happened");
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
        
        // Request sink_1 pad from compositor (standard naming)
        // The key is to ensure proper cleanup so this pad can be reused
        let sink_pad_name = "sink_1";

        println!("[Composite FX] 🔌 Requesting sink pad: {}", sink_pad_name);

        let comp_sink_pad = compositor
            .request_pad_simple(sink_pad_name)
            .ok_or(format!("Failed to request compositor sink pad: {}", sink_pad_name))?;

        println!("[Composite FX] ✅ Successfully requested sink pad: {}", comp_sink_pad.name());

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
        
        // Set compositor sink properties on the actual pad object
        println!("[Composite FX] 🎨 Setting pad properties on: {}", comp_sink_pad.name());
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", self.layers.read().overlay_opacity);
        comp_sink_pad.set_property("xpos", fx_xpos);
        comp_sink_pad.set_property("ypos", fx_ypos);
        comp_sink_pad.set_property("width", fx_width);
        comp_sink_pad.set_property("height", fx_height);

        // Verify properties were set
        println!("[Composite FX] ✅ Pad properties set: zorder=1, alpha={:.2}, pos=({}, {}), size={}x{}",
                 self.layers.read().overlay_opacity, fx_xpos, fx_ypos, fx_width, fx_height);
        

        // CRITICAL: Add timestamp offset probe to align media timestamps to pipeline running-time
        // This prevents "late frames" → "QoS catch-up sprint" on replays
        println!("[Composite FX] ⏱️ Setting up timestamp offset probe...");
        let pipeline_weak_ts = pipeline.downgrade();

        let probe_result = std::panic::catch_unwind(|| {
        ghost_pad.add_probe(
            gst::PadProbeType::BUFFER,  // No BLOCK flag = instant start, no delay!
            move |pad, info| {
                    // Add panic protection inside the probe callback too
                    let result = std::panic::catch_unwind(|| {
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
                            gst::PadProbeReturn::Remove
                        } else {
                gst::PadProbeReturn::Ok
                        }
                    });

                    match result {
                        Ok(ret) => ret,
                        Err(e) => {
                            println!("[Composite FX] ❌ Timestamp probe panicked: {:?}", e);
                            gst::PadProbeReturn::Remove
                        }
                    }
                },
            )
        });

        match probe_result {
            Ok(_) => println!("[Composite FX] ✅ Timestamp offset probe added successfully"),
            Err(e) => println!("[Composite FX] ⚠️ Failed to add timestamp probe: {:?}", e),
        }

        // Sync FX bin state with pipeline FIRST (faster than syncing after link)
        fx_bin.sync_state_with_parent()
            .map_err(|_| "Failed to sync FX bin state".to_string())?;

        // Link FX bin to compositor (happens instantly while bin is already playing)
        println!("[Composite FX] 🔗 Linking ghost pad to compositor sink pad...");
        ghost_pad
            .link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;

        println!("[Composite FX] ✅ Pad linking successful!");
        println!("[Composite FX] 🔗 Link status: ghost_pad.is_linked()={}, comp_sink_pad.is_linked()={}",
                 ghost_pad.is_linked(), comp_sink_pad.is_linked());

        println!("[Composite FX] ✅ FX added to pipeline - playing from file");
        println!("[Composite FX] ⏰ Pipeline ready time: {:?}", std::time::Instant::now());
        println!("[Composite FX] 🔍 Natural pipeline: uridecodebin → videoconvert → videoscale → capsfilter");
        
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
                *self.fx_state.write() = None;
                return Ok(());
            }
        };
        
        // Get compositor element
        let compositor = match pipeline.by_name("comp") {
            Some(c) => c,
            None => {
                println!("[Composite FX] Compositor not found");
                *self.fx_state.write() = None;
                return Ok(());
            }
        };
        
        // Find and remove FX bin (proper cleanup with safe pad operations)
        if let Some(fx_bin_element) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Manual stop: Proper cleanup of FX bin...");

            // Cast to Bin and perform complete cleanup
            if let Ok(fx_bin) = fx_bin_element.dynamic_cast::<gst::Bin>() {
                // Try safe cleanup with pad operations first
                if let Err(e) = self.safe_cleanup_fx(&fx_bin, &compositor) {
                    println!("[Composite FX] ❌ Safe cleanup failed during manual stop: {}, trying emergency", e);

                    // Emergency cleanup: force removal without pad operations
                    let _ = fx_bin.set_state(gst::State::Null);
                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&fx_bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] 🧹 Emergency: FX bin removed during manual stop");
                        } else {
                                println!("[Composite FX] ⚠️ Emergency: FX bin removal failed during manual stop");
                        }
                    }
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Pipeline removal panicked during manual stop: {:?}", e),
                }
                } else {
                    // Safe cleanup succeeded, now remove the bin
                let _ = fx_bin.set_state(gst::State::Null);
                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&fx_bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] 🧹 FX bin removed after safe cleanup (manual stop)");
                            } else {
                                println!("[Composite FX] ⚠️ FX bin removal failed after safe cleanup (manual stop)");
                            }
                        }
                        Err(e) => println!("[Composite FX] ⚠️ Pipeline removal panicked after safe cleanup (manual stop): {:?}", e),
                    }
                }
                
                println!("[Composite FX] ✅ FX branch removed and memory freed");
            }
        } else {
            println!("[Composite FX] No FX bin found to remove");
        }
        
        // Clear FX state after cleanup complete
        *self.fx_state.write() = None;
        println!("[Composite FX] ✅ FX state cleared after manual stop");
        
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
                let has_current_fx = self.fx_state.read().is_some();

                if has_current_fx {
                    println!("[Composite FX] ✅ Current FX is active - bin might be legitimate, skipping emergency cleanup");
                    return Ok(());
                }

                println!("[Composite FX] 🚨 Found orphaned FX bin during emergency cleanup");

                if let Ok(bin) = found_bin.dynamic_cast::<gst::Bin>() {
                    // Try safe cleanup first
                    if let Some(compositor) = pipeline.by_name("comp") {
                        let cleanup_result = self.safe_cleanup_fx(&bin, &compositor);
                        if cleanup_result.is_err() {
                            println!("[Composite FX] 🚨 Safe cleanup failed, forcing removal");
                        }
                    }

                    // Force removal regardless
                    let set_state_result = std::panic::catch_unwind(|| {
                        bin.set_state(gst::State::Null)
                    });

                    match set_state_result {
                        Ok(_) => println!("[Composite FX] ✅ Emergency: Bin set to NULL"),
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Set state failed: {:?}", e),
                    }

                    let remove_result = std::panic::catch_unwind(|| {
                        pipeline.remove(&bin)
                    });

                    match remove_result {
                        Ok(result) => {
                            if result.is_ok() {
                                println!("[Composite FX] ✅ Emergency: Orphaned bin removed");
                            } else {
                                println!("[Composite FX] ⚠️ Emergency: Bin removal failed");
                            }
                        }
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Remove panicked: {:?}", e),
                    }
                }
            } else {
                println!("[Composite FX] ✅ Emergency: No orphaned bins found");
            }

            // Clear any stale FX state
            if self.fx_state.read().is_some() {
                *self.fx_state.write() = None;
                println!("[Composite FX] ✅ Emergency: Stale FX state cleared");
            }
        }

        println!("[Composite FX] ✅ Emergency cleanup complete");
        Ok(())
    }



