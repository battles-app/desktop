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
    camera_frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    overlay_frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
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
            camera_frame_sender: Arc::new(RwLock::new(None)),
            overlay_frame_sender: Arc::new(RwLock::new(None)),
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

    pub fn set_camera_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.camera_frame_sender.write() = Some(sender);
    }

    pub fn set_overlay_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.overlay_frame_sender.write() = Some(sender);
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
                "compositor name=comp \
                   sink_0::zorder=0 sink_0::alpha={} \
                   sink_1::zorder=1 sink_1::alpha={} ! \
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
                 tee name=camera_tee \
                 camera_tee. ! queue ! videoconvert ! video/x-raw,format=BGRx ! jpegenc quality=90 ! appsink name=camera_layer emit-signals=true sync=false max-buffers=2 drop=true \
                 camera_tee. ! comp.sink_0",
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
                "compositor name=comp \
                   sink_0::zorder=0 sink_0::alpha={} \
                   sink_1::zorder=1 sink_1::alpha={} ! \
                 videoconvert ! \
                 video/x-raw,format=BGRx,width={},height={} ! \
                 tee name=t \
                 t. ! queue ! jpegenc quality=90 ! appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 t. ! queue ! {} \
                 mfvideosrc device-index={} ! \
                 videoconvert ! \
                 videoscale ! \
                 video/x-raw,width={},height={},format=BGRA ! \
                 tee name=camera_tee \
                 camera_tee. ! queue ! videoconvert ! video/x-raw,format=BGRx ! jpegenc quality=90 ! appsink name=camera_layer emit-signals=true sync=false max-buffers=2 drop=true \
                 camera_tee. ! comp.sink_0",
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
             tee name=camera_tee \
             camera_tee. ! queue ! videoconvert ! video/x-raw,format=BGRx ! jpegenc quality=90 ! appsink name=camera_layer emit-signals=true sync=false max-buffers=2 drop=true \
             camera_tee. ! comp.sink_0",
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

        // Get the appsink for camera layer (always available in main pipeline)
        let camera_appsink = pipeline
            .by_name("camera_layer")
            .ok_or("Failed to get camera layer appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast camera layer to AppSink")?;

        // Note: overlay_appsink is created dynamically in the FX bin when FX is played
        // It will be set up in play_fx_from_file();

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

        // Set up callbacks for camera layer frames
        let camera_frame_sender = self.camera_frame_sender.clone();
        let is_running_camera = self.is_running.clone();

        camera_appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running_camera.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    let jpeg_data = map.as_slice();
                    if jpeg_data.len() > 100 {
                        if let Some(sender) = camera_frame_sender.read().as_ref() {
                            let _ = sender.send(jpeg_data.to_vec());
                        }
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Note: overlay layer callbacks are set up in play_fx_from_file() when FX bin is created
        
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
        println!("[Composite FX] ⚡ ===== STARTING FX PLAYBACK =====");
        println!("[Composite FX] ⚡ File: {}", file_path);
        println!("[Composite FX] ⚡ Chroma key: {} (color: {}, tolerance: {:.2}, similarity: {:.2})",
                 use_chroma_key, keycolor, tolerance, similarity);
        println!("[Composite FX] ⚡ Pipeline FPS: {}, Width: {}, Height: {}",
                 *self.pipeline_fps.read(), *self.pipeline_width.read(), *self.pipeline_height.read());
        
        // Store FX state
        *self.fx_state.write() = Some(FxPlaybackState {
            file_url: file_path.clone(),
            keycolor: keycolor.clone(),
            tolerance,
            similarity,
            use_chroma_key,
        });
        
        // Parse hex color to RGB (e.g., "#00ff00" -> R=0, G=255, B=0)
        let rgb = Self::hex_to_rgb(&keycolor)?;
        println!("[Composite FX] 📊 Chroma key RGB: R={}, G={}, B={} (tolerance={:.3}, similarity={:.3})",
                 rgb.0, rgb.1, rgb.2, tolerance, similarity);
        
        // Get the pipeline
        let pipeline = match &self.pipeline {
            Some(p) => {
                println!("[Composite FX] ✅ Pipeline found - state: {:?}", p.current_state());
                p
            },
            None => {
                println!("[Composite FX] ❌ No pipeline running - please select a camera first!");
                return Err("[Composite FX] ❌ No pipeline running - please select a camera first!".to_string());
            }
        };

        // Get compositor element
        let compositor = match pipeline.by_name("comp") {
            Some(c) => {
                println!("[Composite FX] ✅ Compositor element found");
                c
            },
            None => {
                println!("[Composite FX] ❌ Compositor element not found!");
                return Err("Failed to get compositor element".to_string());
            }
        };
        
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
        
        println!("[Composite FX] 🚀 ===== CREATING FX PIPELINE ELEMENTS =====");
        println!("[Composite FX] 📁 File path: {}", file_path);

        // Create filesrc with typefind for instant format detection
        use gstreamer::ElementFactory;

        // Use filesrc + decodebin for direct file access
        println!("[Composite FX] 📂 Creating filesrc element...");
        let filesrc = ElementFactory::make("filesrc")
            .name("fxfilesrc")
            .property("location", &file_path)
            .build()
            .map_err(|e| format!("Failed to create filesrc: {}", e))?;
        println!("[Composite FX] ✅ Filesrc created successfully");

        // Create decodebin for decoding with timestamp preservation
        println!("[Composite FX] 🔧 Creating decodebin element...");
        let decodebin = ElementFactory::make("decodebin")
            .name("fxdecode")
            .property("force-sw-decoders", true) // Ensure consistent decoding
            .build()
            .map_err(|e| format!("Failed to create decodebin: {}", e))?;
        println!("[Composite FX] ✅ Decodebin created successfully");

        // Create post-decode elements
        let videoconvert = ElementFactory::make("videoconvert")
            .name("fxconvert")
            .build()
            .map_err(|_| "Failed to create videoconvert")?;

        let videoscale = ElementFactory::make("videoscale")
            .name("fxscale")
            .build()
            .map_err(|_| "Failed to create videoscale")?;

        // Add identity element with sync=false to allow natural FX playback speed
        // Compositor handles timing - FX should play at native frame rate without clock interference
        let identity = ElementFactory::make("identity")
            .name("fxidentity")
            .property("sync", false) // Don't sync to pipeline clock - natural playback speed
            .build()
            .map_err(|_| "Failed to create identity")?;

        // Add queue for buffering - minimal for low latency FX playback
        let queue = ElementFactory::make("queue")
            .name("fxqueue")
            .property("max-size-buffers", 2u32) // Minimal buffer for low latency
            .property("max-size-time", 100000000u64) // 100ms buffer only
            .property_from_str("leaky", "downstream") // Drop old frames if buffer full
            .build()
            .map_err(|_| "Failed to create queue")?;

        // Create caps filter to match compositor format (BGRA with alpha channel)
        // NO FRAME RATE specification - FX plays at natural speed!
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .build();

        println!("[Composite FX] 🎬 Natural FPS without clock sync, format: BGRA");

        let capsfilter = ElementFactory::make("capsfilter")
            .name("fxcaps")
            .property("caps", &caps)
            .build()
            .map_err(|_| "Failed to create capsfilter")?;

        println!("[Composite FX] 🎭 Creating overlay debug elements...");

        // Create overlay layer tee and appsink for debugging
        let overlay_tee = ElementFactory::make("tee")
            .name("overlay_tee")
            .build()
            .map_err(|_| "Failed to create overlay tee")?;

        let overlay_appsink = ElementFactory::make("appsink")
            .name("overlay_layer")
            .property("emit-signals", true)
            .property("sync", false)
            .property("max-buffers", 2u32)
            .property_from_str("drop", "true")
            .build()
            .map_err(|_| "Failed to create overlay appsink")?;

        // Create JPEG encoder for overlay layer
        let overlay_jpegenc = ElementFactory::make("jpegenc")
            .name("overlay_jpegenc")
            .property("quality", 90i32)
            .build()
            .map_err(|_| "Failed to create overlay jpeg encoder")?;

        let overlay_videoconvert = ElementFactory::make("videoconvert")
            .name("overlay_convert")
            .build()
            .map_err(|_| "Failed to create overlay videoconvert")?;

        // Note: We'll use the ghost pad directly instead of a separate queue

        println!("[Composite FX] ✅ Overlay debug elements created");

        // Create bin to hold FX elements
        let fx_bin = gst::Bin::builder().name("fxbin").build();

        // Add alpha element if chroma keying is enabled
        let (chroma_element, has_alpha) = if use_chroma_key {
            let alpha = ElementFactory::make("alpha")
                .name("fxalpha")
                .property_from_str("method", "custom") // Use string for enum
                .property("target-r", rgb.0 as u32)
                .property("target-g", rgb.1 as u32)
                .property("target-b", rgb.2 as u32)
                .property("angle", (tolerance * 180.0) as f32)
                .build()
                .map_err(|e| format!("Failed to create alpha element: {}", e))?;
            (Some(alpha), true)
        } else {
            (None, false)
        };

        // Add all elements to bin (filesrc + decodebin pipeline)
        if let Some(ref alpha) = chroma_element {
            fx_bin.add_many(&[&filesrc, &decodebin, &videoconvert, &videoscale, alpha, &identity, &queue, &capsfilter, &overlay_tee, &overlay_videoconvert, &overlay_jpegenc, &overlay_appsink, &overlay_queue])
                .map_err(|_| "Failed to add elements to FX bin")?;
        } else {
            fx_bin.add_many(&[&filesrc, &decodebin, &videoconvert, &videoscale, &identity, &queue, &capsfilter, &overlay_tee, &overlay_videoconvert, &overlay_jpegenc, &overlay_appsink, &overlay_queue])
                .map_err(|_| "Failed to add elements to FX bin")?;
        }

        // Link static elements (decodebin will link dynamically)
        println!("[Composite FX] 🔗 Linking filesrc → decodebin...");
        gst::Element::link_many(&[&filesrc, &decodebin])
            .map_err(|_| "Failed to link filesrc to decodebin")?;
        println!("[Composite FX] ✅ Filesrc → decodebin linked");

        println!("[Composite FX] 🔗 Linking videoconvert → videoscale...");
        gst::Element::link_many(&[&videoconvert, &videoscale])
            .map_err(|_| "Failed to link videoconvert to post-processing")?;
        println!("[Composite FX] ✅ Videoconvert → videoscale linked");

        println!("[Composite FX] 🔗 Linking post-processing chain...");
        if has_alpha {
            let alpha_elem = chroma_element.as_ref().unwrap();
            gst::Element::link_many(&[&videoscale, alpha_elem, &identity, &queue, &capsfilter])
                .map_err(|_| "Failed to link FX elements with alpha")?;
            println!("[Composite FX] ✅ Chain linked with alpha: videoscale → alpha → identity → queue → capsfilter");
        } else {
            gst::Element::link_many(&[&videoscale, &identity, &queue, &capsfilter])
                .map_err(|_| "Failed to link FX elements")?;
            println!("[Composite FX] ✅ Chain linked: videoscale → identity → queue → capsfilter");
        }

        // Link capsfilter to overlay tee
        println!("[Composite FX] 🔗 Linking capsfilter → overlay_tee...");
        gst::Element::link(&capsfilter, &overlay_tee)
            .map_err(|_| "Failed to link capsfilter to overlay tee")?;
        println!("[Composite FX] ✅ Capsfilter → overlay_tee linked");

        // Request pads from overlay tee for branching
        println!("[Composite FX] 🔌 Requesting overlay tee pads...");
        let overlay_tee_src1 = overlay_tee.request_pad_simple("src_%u")
            .ok_or("Failed to request overlay tee src1 pad")?;
        let overlay_tee_src2 = overlay_tee.request_pad_simple("src_%u")
            .ok_or("Failed to request overlay tee src2 pad")?;
        println!("[Composite FX] ✅ Overlay tee pads requested: src1={}, src2={}",
                 overlay_tee_src1.name(), overlay_tee_src2.name());

        // For the compositor branch, we'll use the capsfilter directly
        // The tee src1 will connect to the compositor via the ghost pad
        println!("[Composite FX] 🔗 Setting up overlay tee branching...");

        // Link tee src1 to a queue that will connect to compositor
        let compositor_queue = ElementFactory::make("queue")
            .name("compositor_queue")
            .property("max-size-buffers", 2u32)
            .build()
            .map_err(|_| "Failed to create compositor queue")?;

        overlay_tee_src1.link(&compositor_queue.static_pad("sink").unwrap())
            .map_err(|_| "Failed to link overlay tee src1 to compositor queue")?;
        println!("[Composite FX] ✅ Overlay tee src1 → compositor_queue linked");

        // Link tee src2 to debug branch
        overlay_tee_src2.link(&overlay_videoconvert.static_pad("sink").unwrap())
            .map_err(|_| "Failed to link overlay tee src2 to videoconvert")?;
        println!("[Composite FX] ✅ Overlay tee src2 → overlay_videoconvert linked");

        // Link debug branch elements
        gst::Element::link_many(&[&overlay_videoconvert, &overlay_jpegenc, &overlay_appsink])
            .map_err(|_| "Failed to link overlay debug branch")?;
        println!("[Composite FX] ✅ Overlay debug branch linked: videoconvert → jpegenc → appsink");

        // Add compositor queue to bin
        fx_bin.add(&compositor_queue)
            .map_err(|_| "Failed to add compositor queue to FX bin")?;

        let final_element = compositor_queue;
        
        // Create ghost pad on the bin
        let final_src_pad = final_element.static_pad("src")
            .ok_or("Failed to get final element src pad")?;
        let ghost_pad = gst::GhostPad::with_target(&final_src_pad)
            .map_err(|_| "Failed to create ghost pad")?;
        ghost_pad.set_active(true).ok();
        fx_bin.add_pad(&ghost_pad).map_err(|_| "Failed to add ghost pad to bin")?;
        
        // Set up callbacks for overlay layer frames
        let overlay_frame_sender_clone = self.overlay_frame_sender.clone();
        let overlay_is_running = self.is_running.clone();

        let overlay_appsink_cast = overlay_appsink.dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast overlay appsink to AppSink")?;

        overlay_appsink_cast.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*overlay_is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    let jpeg_data = map.as_slice();
                    if jpeg_data.len() > 100 {
                        println!("[Overlay Layer] 📸 Sending frame: {} bytes", jpeg_data.len());
                        if let Some(sender) = overlay_frame_sender_clone.read().as_ref() {
                            let _ = sender.send(jpeg_data.to_vec());
                        }
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Add bin to pipeline
        pipeline.add(&fx_bin)
            .map_err(|_| "Failed to add FX bin to pipeline")?;
        
        // Connect decodebin's dynamic pads to videoconvert (for decoded video)
        println!("[Composite FX] 👂 Setting up decodebin pad-added handler...");
        let videoconvert_clone = videoconvert.clone();
        let filesrc_name = filesrc.name().to_string();
        decodebin.connect_pad_added(move |_dbin, src_pad| {
            println!("[Composite FX] 🔗 ===== DECODEBIN PAD ADDED =====");
            println!("[Composite FX] 🔗 Pad name: {}", src_pad.name());
            println!("[Composite FX] 🔗 Pad caps: {:?}", src_pad.current_caps());
            println!("[Composite FX] 🔗 Filesrc: {}", filesrc_name);

            // Only link video pads (ignore audio, text, etc.)
            let caps = match src_pad.current_caps() {
                Some(caps) => {
                    println!("[Composite FX] 📊 Decodebin caps: {}", caps);
                    caps
                },
                None => {
                    println!("[Composite FX] ⚠️ Decodebin pad has no caps yet - waiting...");
                    return;
                },
            };

            let structure = match caps.structure(0) {
                Some(s) => s,
                None => {
                    println!("[Composite FX] ⚠️ Decodebin pad caps has no structure");
                    return;
                },
            };

            let name = structure.name();
            println!("[Composite FX] 🎥 Media type: {}", name);

            if !name.starts_with("video/") {
                // Skip non-video pads (audio, subtitles, etc.)
                println!("[Composite FX] ⏭️ Skipping non-video pad from decodebin: {}", name);
                return;
            }

            // Check if videoconvert sink is already linked (only link once)
            let sink_pad = videoconvert_clone.static_pad("sink").expect("No sink pad");
            println!("[Composite FX] 🔗 Videoconvert sink pad exists: {}", sink_pad.is_linked());

            if sink_pad.is_linked() {
                println!("[Composite FX] ⚠️ Videoconvert sink already linked - ignoring duplicate");
                return;
            }

            // Link decoded video pad to videoconvert
            match src_pad.link(&sink_pad) {
                Ok(_) => {
                    println!("[Composite FX] ✅ ===== SUCCESS: DECODEBIN → VIDEOCONVERT LINKED =====");
                    println!("[Composite FX] 🎬 ===== FX PIPELINE READY - SHOULD BE PLAYING =====");
                },
                Err(e) => {
                    println!("[Composite FX] ❌ ===== FAILED TO LINK DECODEBIN → VIDEOCONVERT =====");
                    println!("[Composite FX] ❌ Error: {:?}", e);
                }
            }
        });
        
        // Request sink_1 pad from compositor
        println!("[Composite FX] 🎭 Requesting compositor sink_1 pad...");

        // Debug: List all pads on compositor
        println!("[Composite FX] 📋 Current compositor pads:");
        let pads = compositor.pads();
        for pad in pads {
            println!("[Composite FX]   - Pad: {}", pad.name());
        }

        // First check if sink_1 already exists and try to release it
        if let Some(existing_pad) = compositor.static_pad("sink_1") {
            println!("[Composite FX] ⚠️ Found existing sink_1 pad, attempting to release...");
            compositor.release_request_pad(&existing_pad);
            println!("[Composite FX] ✅ Existing sink_1 pad released");
        }

        let comp_sink_pad = compositor
            .request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink_1 pad")?;
        println!("[Composite FX] ✅ Compositor sink_1 pad obtained: {}", comp_sink_pad.name());

        // Get pipeline dimensions
        let comp_width = *self.pipeline_width.read() as i32;
        let comp_height = *self.pipeline_height.read() as i32;
        println!("[Composite FX] 📐 Canvas dimensions: {}x{}", comp_width, comp_height);

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

        println!("[Composite FX] 📐 FX positioning: {}x{} at ({}, {}) in {}x{} canvas",
                 fx_width, fx_height, fx_xpos, fx_ypos, comp_width, comp_height);

        // Set z-order, alpha, and positioning for overlay layer
        println!("[Composite FX] 🎛️ Setting compositor properties...");
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", self.layers.read().overlay_opacity);
        comp_sink_pad.set_property("xpos", fx_xpos);
        comp_sink_pad.set_property("ypos", fx_ypos);
        comp_sink_pad.set_property("width", fx_width);
        comp_sink_pad.set_property("height", fx_height);
        println!("[Composite FX] ✅ Compositor properties set (zorder=1, alpha={:.2})",
                 self.layers.read().overlay_opacity);

        // Link FX bin to compositor
        println!("[Composite FX] 🔗 Linking FX bin ghost pad to compositor sink_1...");
        ghost_pad
            .link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;
        println!("[Composite FX] ✅ FX bin linked to compositor");

        // Set FX bin base time to match pipeline for proper sync
        if let Some(base_time) = pipeline.base_time() {
            fx_bin.set_base_time(base_time);
            println!("[Composite FX] ⏱️ Set FX base time: {:?}", base_time);
        } else {
            println!("[Composite FX] ⚠️ No base time available from pipeline");
        }

        // Set FX bin to run asynchronously for natural playback
        // From GStreamer docs: use async state changes for independent elements
        println!("[Composite FX] 🔄 Setting FX bin to async Playing state...");

        // Set base time to match pipeline for proper timing reference
        if let Some(pipeline_base_time) = pipeline.base_time() {
            fx_bin.set_base_time(pipeline_base_time);
            println!("[Composite FX] ⏱️ FX bin base time set to match pipeline");
        }

        // Try to set clock to NULL first (may be refused by some elements)
        let clock_result = fx_bin.set_clock(None::<&gst::Clock>);
        match clock_result {
            Ok(_) => println!("[Composite FX] 🕒 FX bin clock set to NULL successfully"),
            Err(e) => println!("[Composite FX] ⚠️ FX bin clock NULL refused: {:?} (expected for some elements)", e),
        }

        // Set state for independent playback
        let _ = fx_bin.set_state(gst::State::Playing);
        println!("[Composite FX] ✅ FX bin state change initiated to Playing");

        // Check final states
        println!("[Composite FX] 📊 Final states:");
        println!("[Composite FX] 📊 Pipeline state: {:?}", pipeline.current_state());
        println!("[Composite FX] 📊 FX bin state: {:?}", fx_bin.current_state());

        println!("[Composite FX] 🎉 ===== FX PLAYBACK STARTED SUCCESSFULLY =====");
        println!("[Composite FX] 📋 Summary: {} | {}x{} canvas | Chroma: {} | Pipeline: {}fps",
                 file_path.split('\\').last().unwrap_or(&file_path),
                 comp_width, comp_height, use_chroma_key, *self.pipeline_fps.read());
        
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
                        println!("[Composite FX] 🔗 Unlinking ghost pad from compositor sink");
                        ghost_pad.unlink(&peer_pad).ok();
                        println!("[Composite FX] 🔓 Releasing compositor request pad");
                        compositor.release_request_pad(&peer_pad);
                        println!("[Composite FX] ✅ Compositor pad released");
                    }
                }
                
                // Set bin to NULL state
                fx_bin.set_state(gst::State::Null).ok();
                
                // Remove bin from pipeline
                pipeline.remove(&fx_bin).ok();
                println!("[Composite FX] 🗑️ FX bin removed from pipeline");

                // Give GStreamer more time to cleanup
                std::thread::sleep(std::time::Duration::from_millis(50));
                println!("[Composite FX] ⏳ Cleanup delay completed");

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

