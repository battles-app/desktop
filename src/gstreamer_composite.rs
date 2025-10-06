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
                   sink_0::zorder=0 sink_0::alpha={} sink_0::sync=false \
                   sink_1::zorder=1 sink_1::alpha={} sink_1::sync=false ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue ! jpegenc quality=90 ! appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue ! {} \
                 mfvideosrc device-index={} ! \
                 videoflip method={} ! \
                 videoconvert ! \
                 videoscale ! \
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
                   sink_0::zorder=0 sink_0::alpha={} sink_0::sync=false \
                   sink_1::zorder=1 sink_1::alpha={} sink_1::sync=false ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue ! jpegenc quality=90 ! appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue ! {} \
                 mfvideosrc device-index={} ! \
                 videoconvert ! \
                 videoscale ! \
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
            "compositor name=comp \
               sink_0::zorder=0 sink_0::alpha={} \
               sink_1::zorder=1 sink_1::alpha={} ! \
             videoconvert ! \
             video/x-raw,format=BGRx,width={},height={} ! \
             tee name=t \
             t. ! queue ! jpegenc quality=90 ! appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
             t. ! queue ! {} \
             v4l2src device=/dev/video{} ! \
             videoconvert ! \
             videoscale ! \
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
        
        println!("[Composite] Pipeline: {}", pipeline_str);
        
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
    
    /// Play an FX file from file path (file already written by main.rs, NO I/O while locked!)
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] 🎬 Playing FX from file (clean playback - no effects)");

        // Store FX state
        *self.fx_state.write() = Some(FxPlaybackState {
            file_url: file_path.clone(),
            keycolor: keycolor.clone(),
            tolerance,
            similarity,
            use_chroma_key,
        });
        
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
        
        // Stop any existing FX first (with proper cleanup)
        if let Some(existing_fx_bin) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Removing existing FX bin and freeing memory...");
            
            // Cast to Bin and set all child elements to NULL to release resources
            if let Ok(bin) = existing_fx_bin.dynamic_cast::<gst::Bin>() {
                let iterator = bin.iterate_elements();
                for item in iterator {
                    if let Ok(element) = item {
                        element.set_state(gst::State::Null).ok();
                    }
                }
                
                // Unlink from compositor
                if let Some(ghost_pad) = bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                    }
                }
                
                // Set bin to NULL and remove
                bin.set_state(gst::State::Null).ok();
                pipeline.remove(&bin).ok();
                
                // Brief pause to let GStreamer cleanup
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        
        println!("[Composite FX] 🚀 Creating uridecodebin (no disk I/O!)...");
        
        // Create filesrc with typefind for instant format detection
        use gstreamer::ElementFactory;
        
        let file_uri = format!("file:///{}", file_path.replace("\\", "/"));
        println!("[Composite FX] 📁 File URI: {}", file_uri);
        
        // Use uridecodebin for reliable decoding
        let uridecode = ElementFactory::make("uridecodebin")
            .name("fxdecode")
            .property("uri", &file_uri)
            .build()
            .map_err(|e| format!("Failed to create uridecodebin: {}", e))?;
        
        // Simple FX pipeline: just decode and convert to BGRA
        let videoconvert = ElementFactory::make("videoconvert")
            .name("fxconvert")
            .build()
            .map_err(|_| "Failed to create videoconvert")?;

        let videoscale = ElementFactory::make("videoscale")
            .name("fxscale")
            .build()
            .map_err(|_| "Failed to create videoscale")?;

        // Create caps filter for BGRA format
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .build();

        println!("[Composite FX] 🎬 Clean playback - no effects, format: BGRA");

        let capsfilter = ElementFactory::make("capsfilter")
            .name("fxcaps")
            .property("caps", &caps)
            .build()
            .map_err(|_| "Failed to create capsfilter")?;

        // Create bin to hold FX elements
        let fx_bin = gst::Bin::builder().name("fxbin").build();

        // Simple pipeline: uridecodebin -> videoconvert -> videoscale -> capsfilter
        fx_bin.add_many(&[&uridecode, &videoconvert, &videoscale, &capsfilter])
            .map_err(|_| "Failed to add elements to FX bin")?;

        // Link elements
        gst::Element::link_many(&[&videoconvert, &videoscale, &capsfilter])
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
        
        // Connect uridecodebin's dynamic pad (for video only)
        let videoconvert_clone = videoconvert.clone();
        uridecode.connect_pad_added(move |_dbin, src_pad| {
            println!("[Composite FX] 🔗 Pad added: {}", src_pad.name());

            // Only link video pads (ignore audio, text, etc.)
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

            if !name.starts_with("video/") {
                // Skip non-video pads (audio, subtitles, etc.)
                println!("[Composite FX] ⏭️ Skipping non-video pad");
                return;
            }

            // Check if sink is already linked (only link once)
            let sink_pad = videoconvert_clone.static_pad("sink").expect("No sink pad");
            if sink_pad.is_linked() {
                println!("[Composite FX] ⚠️ Sink already linked");
                return;
            }

            // Link video pad
            if let Err(e) = src_pad.link(&sink_pad) {
                println!("[Composite FX] ❌ Failed to link video pad: {:?}", e);
            } else {
                println!("[Composite FX] ✅ Video pad linked successfully!");
            }
        });
        
        // Request sink_1 pad from compositor
        let comp_sink_pad = compositor
            .request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink_1 pad")?;
        
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
        
        // Set z-order, alpha, and positioning for overlay layer
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", self.layers.read().overlay_opacity);
        comp_sink_pad.set_property("xpos", fx_xpos);
        comp_sink_pad.set_property("ypos", fx_ypos);
        comp_sink_pad.set_property("width", fx_width);
        comp_sink_pad.set_property("height", fx_height);
        
        // Link FX bin to compositor
        ghost_pad
            .link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;
        
        // Set FX bin base time to match pipeline for proper sync
        if let Some(base_time) = pipeline.base_time() {
            fx_bin.set_base_time(base_time);
            println!("[Composite FX] ⏱️ Set FX base time: {:?}", base_time);
        }
        
        // Sync FX bin state with pipeline
        fx_bin.sync_state_with_parent()
            .map_err(|_| "Failed to sync FX bin state".to_string())?;
        
        println!("[Composite FX] ✅ FX added to pipeline - playing from file");
        
        Ok(())
    }
    
    /// Stop the currently playing FX
    pub fn stop_fx(&mut self) -> Result<(), String> {
        println!("[Composite FX] Stopping FX and cleaning memory...");
        
        *self.fx_state.write() = None;
        
        // Get the pipeline
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => {
                println!("[Composite FX] No pipeline running");
                return Ok(());
            }
        };
        
        // Get compositor element
        let compositor = match pipeline.by_name("comp") {
            Some(c) => c,
            None => {
                println!("[Composite FX] Compositor not found");
                return Ok(());
            }
        };
        
        // Find and remove FX bin
        if let Some(fx_bin_element) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Cleaning up FX bin...");
            
            // Cast to Bin and set all child elements to NULL to release resources
            if let Ok(fx_bin) = fx_bin_element.dynamic_cast::<gst::Bin>() {
                let iterator = fx_bin.iterate_elements();
                for item in iterator {
                    if let Ok(element) = item {
                        element.set_state(gst::State::Null).ok();
                    }
                }
                
                // Unlink from compositor
                if let Some(ghost_pad) = fx_bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                    }
                }
                
                // Set bin to NULL state
                fx_bin.set_state(gst::State::Null).ok();
                
                // Remove bin from pipeline
                pipeline.remove(&fx_bin).ok();
                
                // Give GStreamer time to cleanup
                std::thread::sleep(std::time::Duration::from_millis(10));
                
                println!("[Composite FX] ✅ FX branch removed and memory freed");
            }
        } else {
            println!("[Composite FX] No FX bin found to remove");
        }
        
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



