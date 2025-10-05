// Clean GStreamer composite implementation
// Purpose: Combine camera layer (back) + effect layer (front with chroma key) at specified FPS
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline, Element, ElementFactory};
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
    target_fps: Arc<RwLock<u32>>,
    target_width: Arc<RwLock<u32>>,
    target_height: Arc<RwLock<u32>>,
}

impl GStreamerComposite {
    pub fn new() -> Result<Self, String> {
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
        println!("[Composite] Initialized");
        
        Ok(Self {
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            camera_frame_sender: Arc::new(RwLock::new(None)),
            overlay_frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
            target_fps: Arc::new(RwLock::new(30)),
            target_width: Arc::new(RwLock::new(1280)),
            target_height: Arc::new(RwLock::new(720)),
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
    
    pub fn start(&mut self, camera_device_id: &str, width: u32, height: u32, fps: u32, rotation: u32) -> Result<(), String> {
        println!("[Composite] Starting: {}x{} @ {}fps, rotation: {}°", width, height, fps, rotation);
        
        // Store settings
        *self.target_fps.write() = fps;
        *self.target_width.write() = width;
        *self.target_height.write() = height;
        
        // Stop existing pipeline
        if let Some(pipeline) = &self.pipeline {
            let _ = pipeline.set_state(gst::State::Null);
        }
        
        *self.is_running.write() = true;
        
        let device_index: u32 = camera_device_id.parse()
            .map_err(|_| "Invalid camera device ID")?;
        
        // Map rotation to videoflip method
        let videoflip_method = match rotation {
            90 => "clockwise",
            180 => "rotate-180",
            270 => "counterclockwise",
            _ => "none",
        };
        
        // Build clean compositor pipeline:
        // Camera: mfvideosrc → videoconvert → videoscale → videorate → compositor sink_0
        // (FX will be added dynamically to compositor sink_1)
        // Output: compositor → tee → (preview appsink, camera_layer appsink)
        
        #[cfg(target_os = "windows")]
        let pipeline_str = if videoflip_method != "none" {
            format!(
                "compositor name=comp sink_0::zorder=0 sink_0::alpha=1.0 ! \
                 videoconvert ! video/x-raw,format=BGRx,width={},height={},framerate={}/1 ! \
                 tee name=t \
                 t. ! queue ! jpegenc quality=90 ! appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 mfvideosrc device-index={} ! \
                 videoflip method={} ! \
                 videoconvert ! videoscale ! video/x-raw,format=BGRA,width={},height={} ! \
                 videorate ! video/x-raw,framerate={}/1 ! \
                 tee name=camera_tee \
                 camera_tee. ! queue ! videoconvert ! jpegenc quality=90 ! appsink name=camera_layer emit-signals=true sync=false max-buffers=2 drop=true \
                 camera_tee. ! queue ! comp.sink_0",
                width, height, fps,
                device_index,
                videoflip_method,
                width, height,
                fps
            )
        } else {
            format!(
                "compositor name=comp sink_0::zorder=0 sink_0::alpha=1.0 ! \
                 videoconvert ! video/x-raw,format=BGRx,width={},height={},framerate={}/1 ! \
                 tee name=t \
                 t. ! queue ! jpegenc quality=90 ! appsink name=preview emit-signals=true sync=false max-buffers=2 drop=true \
                 mfvideosrc device-index={} ! \
                 videoconvert ! videoscale ! video/x-raw,format=BGRA,width={},height={} ! \
                 videorate ! video/x-raw,framerate={}/1 ! \
                 tee name=camera_tee \
                 camera_tee. ! queue ! videoconvert ! jpegenc quality=90 ! appsink name=camera_layer emit-signals=true sync=false max-buffers=2 drop=true \
                 camera_tee. ! queue ! comp.sink_0",
                width, height, fps,
                device_index,
                width, height,
                fps
            )
        };
        
        println!("[Composite] Pipeline: {}", pipeline_str);
        
        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline")?;
        
        // Set up preview appsink
        let preview_sink = pipeline.by_name("preview")
            .ok_or("Failed to get preview appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        
        preview_sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    
                    if let Some(sender) = frame_sender.read().as_ref() {
                        let _ = sender.send(map.as_slice().to_vec());
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Set up camera layer appsink
        let camera_sink = pipeline.by_name("camera_layer")
            .ok_or("Failed to get camera_layer appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;
        
        let camera_frame_sender = self.camera_frame_sender.clone();
        let is_running_camera = self.is_running.clone();
        
        camera_sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running_camera.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    
                    if let Some(sender) = camera_frame_sender.read().as_ref() {
                        let _ = sender.send(map.as_slice().to_vec());
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Start pipeline
        pipeline.set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {}", e))?;
        
        self.pipeline = Some(pipeline);
        println!("[Composite] Started successfully");
        
        Ok(())
    }
    
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] Playing: {} (chroma: {})", file_path, use_chroma_key);
        
        let pipeline = self.pipeline.as_ref()
            .ok_or("No pipeline running - select camera first")?;
        
        let compositor = pipeline.by_name("comp")
            .ok_or("Compositor not found")?;
        
        // Remove existing FX bin if any
        if let Some(existing_fx) = pipeline.by_name("fxbin") {
            if let Ok(bin) = existing_fx.dynamic_cast::<gst::Bin>() {
                // Unlink from compositor
                if let Some(ghost_pad) = bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                    }
                }
                
                bin.set_state(gst::State::Null).ok();
                pipeline.remove(&bin).ok();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        
        // Create FX bin: filesrc → decodebin → videoconvert → videoscale → [alpha] → videorate → compositor
        let fx_bin = gst::Bin::builder().name("fxbin").build();
        
        let filesrc = ElementFactory::make("filesrc")
            .name("fxfilesrc")
            .property("location", &file_path)
            .build()
            .map_err(|e| format!("Failed to create filesrc: {}", e))?;
        
        let decodebin = ElementFactory::make("decodebin")
            .name("fxdecode")
            .build()
            .map_err(|e| format!("Failed to create decodebin: {}", e))?;
        
        let videoconvert = ElementFactory::make("videoconvert")
            .name("fxconvert")
            .build()
            .map_err(|_| "Failed to create videoconvert")?;
        
        let videoscale = ElementFactory::make("videoscale")
            .name("fxscale")
            .build()
            .map_err(|_| "Failed to create videoscale")?;
        
        // Parse chroma key color
        let rgb = Self::hex_to_rgb(&keycolor)?;
        
        // Create alpha element if chroma keying enabled
        let alpha_elem = if use_chroma_key {
            let alpha = ElementFactory::make("alpha")
                .name("fxalpha")
                .property_from_str("method", "custom")
                .property("target-r", rgb.0 as u32)
                .property("target-g", rgb.1 as u32)
                .property("target-b", rgb.2 as u32)
                .property("angle", (tolerance * 180.0) as f32)
                .build()
                .map_err(|e| format!("Failed to create alpha: {}", e))?;
            Some(alpha)
        } else {
            None
        };
        
        // Videorate to match target FPS
        let videorate = ElementFactory::make("videorate")
            .name("fxvideorate")
            .property("drop-only", true)
            .build()
            .map_err(|_| "Failed to create videorate")?;
        
        // Get target dimensions and FPS
        let target_width = *self.target_width.read();
        let target_height = *self.target_height.read();
        let target_fps = *self.target_fps.read();
        
        // Capsfilter to set output format and framerate
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("width", target_width as i32)
            .field("height", target_height as i32)
            .field("framerate", gst::Fraction::new(target_fps as i32, 1))
            .build();
        
        let capsfilter = ElementFactory::make("capsfilter")
            .name("fxcaps")
            .property("caps", &caps)
            .build()
            .map_err(|_| "Failed to create capsfilter")?;
        
        // Tee for overlay layer debug output
        let overlay_tee = ElementFactory::make("tee")
            .name("overlay_tee")
            .build()
            .map_err(|_| "Failed to create overlay tee")?;
        
        let overlay_queue1 = ElementFactory::make("queue")
            .name("overlay_queue1")
            .property("max-size-buffers", 2u32)
            .build()
            .map_err(|_| "Failed to create overlay_queue1")?;
        
        let overlay_queue2 = ElementFactory::make("queue")
            .name("overlay_queue2")
            .property("max-size-buffers", 2u32)
            .build()
            .map_err(|_| "Failed to create overlay_queue2")?;
        
        let overlay_convert = ElementFactory::make("videoconvert")
            .name("overlay_convert")
            .build()
            .map_err(|_| "Failed to create overlay_convert")?;
        
        let overlay_jpegenc = ElementFactory::make("jpegenc")
            .name("overlay_jpegenc")
            .property("quality", 90i32)
            .build()
            .map_err(|_| "Failed to create overlay_jpegenc")?;
        
        let overlay_appsink = ElementFactory::make("appsink")
            .name("overlay_layer")
            .property("emit-signals", true)
            .property("sync", false)
            .property("max-buffers", 2u32)
            .property_from_str("drop", "true")
            .build()
            .map_err(|_| "Failed to create overlay_appsink")?;
        
        // Add all elements to bin
        if let Some(ref alpha) = alpha_elem {
            fx_bin.add_many(&[&filesrc, &decodebin, &videoconvert, &videoscale, alpha, &videorate, &capsfilter, &overlay_tee, &overlay_queue1, &overlay_queue2, &overlay_convert, &overlay_jpegenc, &overlay_appsink])
                .map_err(|_| "Failed to add elements to FX bin")?;
        } else {
            fx_bin.add_many(&[&filesrc, &decodebin, &videoconvert, &videoscale, &videorate, &capsfilter, &overlay_tee, &overlay_queue1, &overlay_queue2, &overlay_convert, &overlay_jpegenc, &overlay_appsink])
                .map_err(|_| "Failed to add elements to FX bin")?;
        }
        
        // Link static elements
        gst::Element::link_many(&[&filesrc, &decodebin])
            .map_err(|_| "Failed to link filesrc → decodebin")?;
        
        // Link post-decode chain
        if let Some(ref alpha) = alpha_elem {
            gst::Element::link_many(&[&videoconvert, &videoscale, alpha, &videorate, &capsfilter, &overlay_tee])
                .map_err(|_| "Failed to link post-decode chain with alpha")?;
        } else {
            gst::Element::link_many(&[&videoconvert, &videoscale, &videorate, &capsfilter, &overlay_tee])
                .map_err(|_| "Failed to link post-decode chain")?;
        }
        
        // Link tee branches
        let tee_src1 = overlay_tee.request_pad_simple("src_%u")
            .ok_or("Failed to request tee src1")?;
        let tee_src2 = overlay_tee.request_pad_simple("src_%u")
            .ok_or("Failed to request tee src2")?;
        
        tee_src1.link(&overlay_queue1.static_pad("sink").unwrap())
            .map_err(|_| "Failed to link tee → queue1")?;
        
        tee_src2.link(&overlay_queue2.static_pad("sink").unwrap())
            .map_err(|_| "Failed to link tee → queue2")?;
        
        // Link debug branch
        gst::Element::link_many(&[&overlay_queue2, &overlay_convert, &overlay_jpegenc, &overlay_appsink])
            .map_err(|_| "Failed to link debug branch")?;
        
        // Set up overlay appsink callback
        let overlay_frame_sender = self.overlay_frame_sender.clone();
        let is_running_overlay = self.is_running.clone();
        
        let overlay_appsink_cast = overlay_appsink.dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast overlay_appsink")?;
        
        overlay_appsink_cast.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running_overlay.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    
                    if let Some(sender) = overlay_frame_sender.read().as_ref() {
                        let _ = sender.send(map.as_slice().to_vec());
                    }
                    
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Create ghost pad
        let final_src_pad = overlay_queue1.static_pad("src")
            .ok_or("Failed to get overlay_queue1 src pad")?;
        let ghost_pad = gst::GhostPad::with_target(&final_src_pad)
            .map_err(|_| "Failed to create ghost pad")?;
        ghost_pad.set_active(true).ok();
        fx_bin.add_pad(&ghost_pad).map_err(|_| "Failed to add ghost pad")?;
        
        // Connect decodebin dynamic pad
        let videoconvert_clone = videoconvert.clone();
        decodebin.connect_pad_added(move |_dbin, src_pad| {
            let caps = match src_pad.current_caps() {
                Some(caps) => caps,
                None => return,
            };
            
            let structure = match caps.structure(0) {
                Some(s) => s,
                None => return,
            };
            
            if !structure.name().starts_with("video/") {
                return;
            }
            
            let sink_pad = videoconvert_clone.static_pad("sink").expect("No sink pad");
            if sink_pad.is_linked() {
                return;
            }
            
            let _ = src_pad.link(&sink_pad);
        });
        
        // Add bin to pipeline
        pipeline.add(&fx_bin)
            .map_err(|_| "Failed to add FX bin to pipeline")?;
        
        // Request compositor sink_1 pad
        if let Some(existing_pad) = compositor.static_pad("sink_1") {
            compositor.release_request_pad(&existing_pad);
        }
        
        let comp_sink_pad = compositor.request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink_1")?;
        
        // Set compositor properties (zorder=1, alpha=1.0, fill canvas)
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", 1.0f64);
        comp_sink_pad.set_property("xpos", 0i32);
        comp_sink_pad.set_property("ypos", 0i32);
        comp_sink_pad.set_property("width", target_width as i32);
        comp_sink_pad.set_property("height", target_height as i32);
        
        // Link FX bin to compositor
        ghost_pad.link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;
        
        // Sync FX bin with pipeline
        if let Some(base_time) = pipeline.base_time() {
            fx_bin.set_base_time(base_time);
        }
        
        // Start FX bin
        fx_bin.set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start FX bin: {:?}", e))?;
        
        println!("[Composite FX] Started: {}x{} @ {}fps", target_width, target_height, target_fps);
        Ok(())
    }
    
    pub fn stop_fx(&mut self) -> Result<(), String> {
        println!("[Composite FX] Stopping");
        
        let pipeline = match &self.pipeline {
            Some(p) => p,
            None => return Ok(()),
        };
        
        let compositor = match pipeline.by_name("comp") {
            Some(c) => c,
            None => return Ok(()),
        };
        
        if let Some(fx_bin_element) = pipeline.by_name("fxbin") {
            if let Ok(fx_bin) = fx_bin_element.dynamic_cast::<gst::Bin>() {
                // Unlink from compositor
                if let Some(ghost_pad) = fx_bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                    }
                }
                
                fx_bin.set_state(gst::State::Null).ok();
                pipeline.remove(&fx_bin).ok();
            }
        }
        
        println!("[Composite FX] Stopped");
        Ok(())
    }
    
    pub fn stop(&mut self) -> Result<(), String> {
        *self.is_running.write() = false;
        
        if let Some(pipeline) = &self.pipeline {
            pipeline.set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {}", e))?;
        }
        
        self.pipeline = None;
        println!("[Composite] Stopped");
        Ok(())
    }
    
    pub fn set_output_format(&mut self, format: &str) -> Result<(), String> {
        println!("[Composite] Output format: {}", format);
        // Placeholder for virtual camera / NDI output
        Ok(())
    }
    
    pub fn update_layers(&self, camera: (bool, f64), overlay: (bool, f64)) {
        println!("[Composite] Layer visibility updated: camera={}/{:.2}, overlay={}/{:.2}", 
                 camera.0, camera.1, overlay.0, overlay.1);
        // Placeholder for dynamic opacity adjustment
    }
    
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
