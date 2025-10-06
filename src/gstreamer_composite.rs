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
                        let count = frame_count_clone.fetch_add(1, Ordering::Relaxed);

                        // Log performance metrics every 2 seconds
                        let now = Instant::now();
                        let mut last_log = last_log_time_clone.write();
                        if now.duration_since(*last_log).as_secs() >= 2 {
                            let elapsed = start_time_clone.elapsed();
                            let fps = count as f64 / elapsed.as_secs_f64();
                            let prev_count = last_frame_count_clone.swap(count, Ordering::Relaxed);
                            let recent_frames = count.saturating_sub(prev_count);
                            let recent_fps = recent_frames as f64 / 2.0;

                            println!("[Composite] 📊 Performance - Total: {} frames ({:.1} fps), Recent: {:.1} fps, Buffer: {} bytes",
                                count, fps, recent_fps, jpeg_data.len());

                            *last_log = now;
                        }

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
    
    /// Play an FX file from file path (file already written by main.rs, NO I/O while locked!)
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] 🎬 Playing FX from file (clean playback - no effects)");
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
        
        // Stop any existing FX first (with complete pad cleanup)
        if let Some(existing_fx_bin) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Aggressive cleanup of existing FX pipeline...");

            // Cast to Bin and aggressively cleanup all resources
            if let Ok(bin) = existing_fx_bin.dynamic_cast::<gst::Bin>() {
                // Unlink from compositor FIRST to stop data flow
                if let Some(ghost_pad) = bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        // Release the compositor sink pad (sink_1)
                        compositor.release_request_pad(&peer_pad);
                        println!("[Composite FX] ✅ Unlinked and released sink_1 pad");
                    }
                }

                // Also check stored pad in fx_state and release it
                if let Some(ref fx_state) = *self.fx_state.read() {
                    if let Some(ref sink_pad) = fx_state.compositor_sink_pad {
                        println!("[Composite FX] 🧹 Releasing stored compositor sink pad");
                        compositor.release_request_pad(sink_pad);
                    }
                }

                // THEN stop bin and wait for NULL state to complete
                if let Ok(_) = bin.set_state(gst::State::Null) {
                    // Wait for state change to complete (timeout 1 second)
                    let _ = bin.state(Some(gst::ClockTime::from_seconds(1)));
                    println!("[Composite FX] ✅ FX bin stopped completely");
                }

                // Force cleanup of all child elements (ensure videorate resets)
                let iterator = bin.iterate_elements();
                for item in iterator {
                    if let Ok(element) = item {
                        // Force NULL state and wait for completion
                        if let Ok(_) = element.set_state(gst::State::Null) {
                            let _ = element.state(Some(gst::ClockTime::from_mseconds(100)));
                        }
                    }
                }

                // Remove bin from pipeline
                if pipeline.remove(&bin).is_ok() {
                    println!("[Composite FX] ✅ FX bin removed from pipeline");
                }

                // Give GStreamer time to cleanup and reset element state
                std::thread::sleep(std::time::Duration::from_millis(100));
                println!("[Composite FX] 🧹 Memory cleanup completed");
            }
        }

        // Clear FX state to prevent double-release
        *self.fx_state.write() = None;

        // Ensure pipeline is in playing state after cleanup
        pipeline.set_state(gst::State::Playing).ok();
        
        // Small delay to ensure pipeline is stable before adding new FX
        std::thread::sleep(std::time::Duration::from_millis(50));
        
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
            .build()
            .map_err(|e| format!("Failed to create uridecodebin: {}", e))?;

        // Try to reduce GPU usage by preferring software decoders
        // Note: This may not work on all systems, but worth trying
        let _ = uridecode.set_property("force-sw-decoders", &true);

        // Force consistent 30fps output with videorate in live mode
        let videorate = ElementFactory::make("videorate")
            .name("fxvideorate")
            .property("drop-only", true)       // Only drop frames, never duplicate
            .property("skip-to-first", true)   // Start fresh, ignore previous state
            .property("max-rate", 30i32)       // Hard limit to 30fps
            .property("average-period", 0u64)  // No averaging, immediate rate limiting
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

        let videoconvert = ElementFactory::make("videoconvert")
            .name("fxconvert")
            .build()
            .map_err(|_| "Failed to create videoconvert")?;

        let videoscale = ElementFactory::make("videoscale")
            .name("fxscale")
            .build()
            .map_err(|_| "Failed to create videoscale")?;

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

        // Pipeline: uridecodebin -> videorate -> rate_filter -> videoconvert -> videoscale -> capsfilter
        fx_bin.add_many(&[&uridecode, &videorate, &rate_filter, &videoconvert, &videoscale, &capsfilter])
            .map_err(|_| "Failed to add elements to FX bin")?;

        // Link elements with forced 30fps rate control
        gst::Element::link_many(&[&videorate, &rate_filter, &videoconvert, &videoscale, &capsfilter])
            .map_err(|_| "Failed to link FX elements")?;

        let final_element = capsfilter.clone();
        
        // Create ghost pad on the bin
        let final_src_pad = final_element.static_pad("src")
            .ok_or("Failed to get final element src pad")?;
        let ghost_pad = gst::GhostPad::with_target(&final_src_pad)
            .map_err(|_| "Failed to create ghost pad")?;
        ghost_pad.set_active(true).ok();
        fx_bin.add_pad(&ghost_pad).map_err(|_| "Failed to add ghost pad to bin")?;
        
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
        
        // Add FPS monitoring probe to FX ghost pad
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::time::Instant;

        let fx_frame_count = Arc::new(AtomicUsize::new(0));
        let fx_start_time = Arc::new(Instant::now());
        let fx_last_log_time = Arc::new(std::sync::Mutex::new(Instant::now()));
        let fx_last_frame_count = Arc::new(std::sync::Mutex::new(0usize));

        let fx_frame_count_clone = fx_frame_count.clone();
        let fx_start_time_clone = fx_start_time.clone();
        let fx_last_log_time_clone = fx_last_log_time.clone();
        let fx_last_frame_count_clone = fx_last_frame_count.clone();

        ghost_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
            if let Some(gst::PadProbeData::Buffer(ref _buffer)) = info.data {
                let count = fx_frame_count_clone.fetch_add(1, Ordering::Relaxed);
                let now = Instant::now();
                let mut last_log = fx_last_log_time_clone.lock().unwrap();

                if now.duration_since(*last_log).as_secs() >= 2 {
                    let elapsed = fx_start_time_clone.elapsed();
                    let fps = count as f64 / elapsed.as_secs_f64();
                    let prev_count = fx_last_frame_count_clone.lock().unwrap().clone();
                    let recent_frames = count.saturating_sub(prev_count);
                    let recent_fps = recent_frames as f64 / 2.0; // 2 second window

                    println!("[Composite FX] 🎬 Performance - Total: {} frames ({:.1} fps), Recent: {:.1} fps",
                        count, fps, recent_fps);

                    *fx_last_frame_count_clone.lock().unwrap() = count;
                    *last_log = now;
                }
            }
            gst::PadProbeReturn::Ok
        });

        // Link FX bin to compositor
        ghost_pad
            .link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;

        // Sync FX bin state with pipeline
        fx_bin.sync_state_with_parent()
            .map_err(|_| "Failed to sync FX bin state".to_string())?;

        // Set independent clock timing for FX bin to prevent "catch-up" behavior
        // Each FX playback gets a fresh clock start, preventing speed-up on subsequent plays
        let current_time = pipeline.clock()
            .and_then(|clock| clock.time())
            .unwrap_or(gst::ClockTime::ZERO);
        
        fx_bin.set_base_time(current_time);
        fx_bin.set_start_time(gst::ClockTime::ZERO);
        
        println!("[Composite FX] ⏱️ Independent clock timing - Base: {:?} (prevents catch-up)", current_time);
        println!("[Composite FX] ⏱️ FX will play at natural 30fps regardless of pipeline clock");

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
        
        // Find and remove FX bin
        if let Some(fx_bin_element) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Cleaning up FX bin...");

            // Cast to Bin and set all child elements to NULL to release resources
            if let Ok(fx_bin) = fx_bin_element.dynamic_cast::<gst::Bin>() {
                // Unlink from compositor FIRST to stop data flow
                if let Some(ghost_pad) = fx_bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                        println!("[Composite FX] 🧹 Released compositor sink_1 pad");
                    }
                }

                // Also release stored pad if different
                if let Some(ref fx_state) = *self.fx_state.read() {
                    if let Some(ref sink_pad) = fx_state.compositor_sink_pad {
                        compositor.release_request_pad(sink_pad);
                    }
                }

                // Set bin to NULL state and WAIT for completion
                if let Ok(_) = fx_bin.set_state(gst::State::Null) {
                    // Wait for state change to complete (timeout 1 second)
                    let _ = fx_bin.state(Some(gst::ClockTime::from_seconds(1)));
                }

                // Set all child elements to NULL and wait
                let iterator = fx_bin.iterate_elements();
                for item in iterator {
                    if let Ok(element) = item {
                        if let Ok(_) = element.set_state(gst::State::Null) {
                            let _ = element.state(Some(gst::ClockTime::from_mseconds(100)));
                        }
                    }
                }
                
                // Remove bin from pipeline
                pipeline.remove(&fx_bin).ok();
                
                // Give GStreamer time to cleanup and reset element state
                std::thread::sleep(std::time::Duration::from_millis(100));
                
                println!("[Composite FX] ✅ FX branch removed and memory freed");
            }
        } else {
            println!("[Composite FX] No FX bin found to remove");
        }
        
        // Clear FX state after cleanup complete
        *self.fx_state.write() = None;
        
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



