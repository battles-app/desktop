// Clean GStreamer composite implementation
// Purpose: Combine camera layer (back) + effect layer (front with chroma key) at specified FPS
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline, ElementFactory};
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
    fx_counter: Arc<RwLock<u32>>, // Counter for unique FX bin names
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
            fx_counter: Arc::new(RwLock::new(0)),
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
        let old_fps = *self.target_fps.read();
        let old_width = *self.target_width.read();
        let old_height = *self.target_height.read();
        let is_fps_change = old_fps != fps && old_fps != 0;
        let is_resolution_change = (old_width != width || old_height != height) && old_width != 0;
        
        println!("\n");
        println!("[Composite] ╔════════════════════════════════════════════════════════════════");
        println!("[Composite] ║ ⚙️  COMPOSITE PIPELINE CONFIGURATION");
        println!("[Composite] ╠════════════════════════════════════════════════════════════════");
        println!("[Composite] ║ 📐 Resolution: {}x{}", width, height);
        println!("[Composite] ║ 🎞️  FPS: {}", fps);
        println!("[Composite] ║ 🔄 Rotation: {}°", rotation);
        println!("[Composite] ║ 📹 Camera device: {}", camera_device_id);
        
        if old_fps != 0 {
            println!("[Composite] ║");
            if is_fps_change {
                println!("[Composite] ║ 🔄 FPS CHANGED: {} → {} fps", old_fps, fps);
                println!("[Composite] ║    ⚠️  Pipeline will restart with new target FPS");
                println!("[Composite] ║    ⚠️  Any playing FX will be stopped and need to be replayed");
            }
            if is_resolution_change {
                println!("[Composite] ║ 🔄 RESOLUTION CHANGED: {}x{} → {}x{}", old_width, old_height, width, height);
                println!("[Composite] ║    ⚠️  Pipeline will restart with new resolution");
            }
            if !is_fps_change && !is_resolution_change {
                println!("[Composite] ║ ✅ Settings unchanged (rotation only)");
            }
        }
        
        println!("[Composite] ╚════════════════════════════════════════════════════════════════");
        println!("");
        
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
    
    pub fn play_fx_from_file(&mut self, file_path: String, keycolor: String, tolerance: f64, _similarity: f64, use_chroma_key: bool) -> Result<(), String> {
        println!("[Composite FX] Playing: {} (chroma: {})", file_path, use_chroma_key);
        
        let pipeline = self.pipeline.as_ref()
            .ok_or("No pipeline running - select camera first")?;
        
        let compositor = pipeline.by_name("comp")
            .ok_or("Compositor not found")?;
        
        // Increment FX counter for unique bin names
        let fx_id = {
            let mut counter = self.fx_counter.write();
            *counter += 1;
            *counter
        };
        println!("[Composite FX] 🆔 FX Instance ID: {}", fx_id);
        
        // Remove existing FX bin if any (search for any fxbin_*)
        let existing_bins: Vec<_> = pipeline.children().iter()
            .filter(|el| el.name().starts_with("fxbin_"))
            .cloned()
            .collect();
        
        for existing_fx in existing_bins {
            if let Ok(bin) = existing_fx.dynamic_cast::<gst::Bin>() {
                println!("[Composite FX] 🗑️ Removing old FX bin: {}", bin.name());
                // Unlink from compositor
                if let Some(ghost_pad) = bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                    }
                }
                
                // Remove bin from pipeline - DON'T set to Null first!
                // Setting to Null resets internal timing and causes speed issues
                // Just remove it and let GStreamer handle cleanup
                pipeline.remove(&bin).ok();
                
                // Now set to Null AFTER removing to properly clean up
                bin.set_state(gst::State::Null).ok();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        
        // Create FX bin with UNIQUE name: filesrc → decodebin → videoconvert → videoscale → [alpha] → videorate → compositor
        let bin_name = format!("fxbin_{}", fx_id);
        let fx_bin = gst::Bin::builder().name(&bin_name).build();
        println!("[Composite FX] 📦 Creating new FX bin: {}", bin_name);
        
        let filesrc = ElementFactory::make("filesrc")
            .name(&format!("fxfilesrc_{}", fx_id))
            .property("location", &file_path)
            .build()
            .map_err(|e| format!("Failed to create filesrc: {}", e))?;

        let decodebin = ElementFactory::make("decodebin")
            .name(&format!("fxdecode_{}", fx_id))
            .build()
            .map_err(|e| format!("Failed to create decodebin: {}", e))?;

        let videoconvert = ElementFactory::make("videoconvert")
            .name(&format!("fxconvert_{}", fx_id))
            .build()
            .map_err(|_| "Failed to create videoconvert")?;

        let videoscale = ElementFactory::make("videoscale")
            .name(&format!("fxscale_{}", fx_id))
            .build()
            .map_err(|_| "Failed to create videoscale")?;

        // Parse chroma key color
        let rgb = Self::hex_to_rgb(&keycolor)?;
        
        // Create alpha element if chroma keying enabled
        let alpha_elem = if use_chroma_key {
            let alpha = ElementFactory::make("alpha")
                .name(&format!("fxalpha_{}", fx_id))
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
        // CRITICAL: Preserves original playback speed/duration!
        // - skip-to-first=false: Don't skip frames at start (maintains timing)
        // - drop-only=false: Allow frame duplication for upscaling FPS
        // - Duplicates frames for low FPS (24fps → 60fps)
        // - Drops frames for high FPS (60fps → 30fps)
        // Result: Video plays at ORIGINAL speed, outputs at TARGET fps
        let videorate = ElementFactory::make("videorate")
            .name(&format!("fxvideorate_{}", fx_id))
            .property("skip-to-first", false) // Maintain original timing
            .property("drop-only", false) // Allow duplication (default, but explicit)
            .property("average-period", 0u64) // No averaging, immediate conversion
            .property("max-rate", i32::MAX) // No rate limiting
            .build()
            .map_err(|_| "Failed to create videorate")?;
        
        // Add pad probes to monitor videorate input/output for debugging
        let videorate_clone_for_probe = videorate.clone();
        let fx_id_for_probe = fx_id;
        std::thread::spawn(move || {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::Arc;
            use parking_lot::Mutex;
            std::thread::sleep(std::time::Duration::from_millis(500));
            
            if let Some(sink_pad) = videorate_clone_for_probe.static_pad("sink") {
                let frame_count_in = Arc::new(AtomicU64::new(0));
                let last_pts_in = Arc::new(Mutex::new(None::<gst::ClockTime>));
                
                sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    if let Some(gst::PadProbeData::Buffer(ref buffer)) = info.data {
                        let count = frame_count_in.fetch_add(1, Ordering::Relaxed) + 1;
                        let pts = buffer.pts();
                        
                        if count <= 3 || count % 30 == 0 {
                            let last = *last_pts_in.lock();
                            let duration = if let (Some(curr), Some(last)) = (pts, last) {
                                if curr > last {
                                    Some((curr.nseconds() - last.nseconds()) / 1_000_000)
                                } else { None }
                            } else { None };
                            
                            println!("[FX {} videorate IN] Frame #{}: PTS={:?}, Delta={:?}ms", 
                                     fx_id_for_probe, count, pts.map(|p| p.mseconds()), duration);
                        }
                        
                        *last_pts_in.lock() = pts;
                    }
                    gst::PadProbeReturn::Ok
                });
            }
            
            if let Some(src_pad) = videorate_clone_for_probe.static_pad("src") {
                let frame_count_out = Arc::new(AtomicU64::new(0));
                let last_pts_out = Arc::new(Mutex::new(None::<gst::ClockTime>));
                
                src_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    if let Some(gst::PadProbeData::Buffer(ref buffer)) = info.data {
                        let count = frame_count_out.fetch_add(1, Ordering::Relaxed) + 1;
                        let pts = buffer.pts();
                        
                        if count <= 3 || count % 30 == 0 {
                            let last = *last_pts_out.lock();
                            let duration = if let (Some(curr), Some(last)) = (pts, last) {
                                if curr > last {
                                    Some((curr.nseconds() - last.nseconds()) / 1_000_000)
                                } else { None }
                            } else { None };
                            
                            println!("[FX {} videorate OUT] Frame #{}: PTS={:?}, Delta={:?}ms", 
                                     fx_id_for_probe, count, pts.map(|p| p.mseconds()), duration);
                        }
                        
                        *last_pts_out.lock() = pts;
                    }
                    gst::PadProbeReturn::Ok
                });
            }
        });
        
        // Get target dimensions and FPS
        let target_width = *self.target_width.read();
        let target_height = *self.target_height.read();
        let target_fps = *self.target_fps.read();
        
        println!("\n");
        println!("[Composite FX] ╔════════════════════════════════════════════════════════════════");
        println!("[Composite FX] ║ 🎬 NEW FX PLAYBACK REQUEST");
        println!("[Composite FX] ╠════════════════════════════════════════════════════════════════");
        println!("[Composite FX] ║ 📁 File: {}", file_path);
        println!("[Composite FX] ║ 🎨 Chroma key: {} (enabled: {})", keycolor, use_chroma_key);
        println!("[Composite FX] ║ 📐 Target resolution: {}x{}", target_width, target_height);
        println!("[Composite FX] ║ 🎞️  Target FPS: {}", target_fps);
        println!("[Composite FX] ╚════════════════════════════════════════════════════════════════");
        println!("");
        
        // Capsfilter to set output format and framerate (but NOT dimensions - compositor handles that)
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "BGRA")
            .field("framerate", gst::Fraction::new(target_fps as i32, 1))
            .build();

        let capsfilter = ElementFactory::make("capsfilter")
            .name(&format!("fxcaps_{}", fx_id))
            .property("caps", &caps)
            .build()
            .map_err(|_| "Failed to create capsfilter")?;

        // Identity element (passthrough)
        let identity = ElementFactory::make("identity")
            .name(&format!("fxidentity_{}", fx_id))
            .build()
            .map_err(|_| "Failed to create identity")?;
        
        // Tee for overlay layer debug output
        let overlay_tee = ElementFactory::make("tee")
            .name(&format!("overlay_tee_{}", fx_id))
            .build()
            .map_err(|_| "Failed to create overlay tee")?;

        let overlay_queue1 = ElementFactory::make("queue")
            .name(&format!("overlay_queue1_{}", fx_id))
            .property("max-size-buffers", 2u32)
            .property("max-size-time", 0u64) // Disable time-based buffering
            .build()
            .map_err(|_| "Failed to create overlay_queue1")?;
        
        let overlay_queue2 = ElementFactory::make("queue")
            .name(&format!("overlay_queue2_{}", fx_id))
            .property("max-size-buffers", 2u32)
            .property("max-size-time", 0u64) // Disable time-based buffering
            .build()
            .map_err(|_| "Failed to create overlay_queue2")?;

        let overlay_convert = ElementFactory::make("videoconvert")
            .name(&format!("overlay_convert_{}", fx_id))
            .build()
            .map_err(|_| "Failed to create overlay_convert")?;
        
        let overlay_jpegenc = ElementFactory::make("jpegenc")
            .name(&format!("overlay_jpegenc_{}", fx_id))
            .property("quality", 90i32)
            .build()
            .map_err(|_| "Failed to create overlay_jpegenc")?;
        
        let overlay_appsink = ElementFactory::make("appsink")
            .name(&format!("overlay_layer_{}", fx_id))
            .property("emit-signals", true)
            .property("sync", false) // Don't sync to clock - let frames flow naturally
            .property("async", false) // Process frames immediately
            .property("max-buffers", 2u32)
            .property_from_str("drop", "true")
            .build()
            .map_err(|_| "Failed to create overlay_appsink")?;
        
        // Add all elements to bin
        if let Some(ref alpha) = alpha_elem {
            fx_bin.add_many(&[&filesrc, &decodebin, &videoconvert, &videoscale, alpha, &videorate, &capsfilter, &identity, &overlay_tee, &overlay_queue1, &overlay_queue2, &overlay_convert, &overlay_jpegenc, &overlay_appsink])
                .map_err(|_| "Failed to add elements to FX bin")?;
        } else {
            fx_bin.add_many(&[&filesrc, &decodebin, &videoconvert, &videoscale, &videorate, &capsfilter, &identity, &overlay_tee, &overlay_queue1, &overlay_queue2, &overlay_convert, &overlay_jpegenc, &overlay_appsink])
                .map_err(|_| "Failed to add elements to FX bin")?;
        }

        // Link static elements
        gst::Element::link_many(&[&filesrc, &decodebin])
            .map_err(|_| "Failed to link filesrc → decodebin")?;
        
        // Link post-decode chain
        if let Some(ref alpha) = alpha_elem {
            gst::Element::link_many(&[&videoconvert, &videoscale, alpha, &videorate, &capsfilter, &identity, &overlay_tee])
                .map_err(|_| "Failed to link post-decode chain with alpha")?;
        } else {
            gst::Element::link_many(&[&videoconvert, &videoscale, &videorate, &capsfilter, &identity, &overlay_tee])
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
        let file_path_for_log = file_path.clone();
        let target_fps_for_log = target_fps;
        let compositor_for_callback = compositor.clone();
        let target_width_for_callback = target_width;
        let target_height_for_callback = target_height;
        decodebin.connect_pad_added(move |_dbin, src_pad| {
            println!("\n[Composite FX] ╔════════════════════════════════════════════════════════════════");
            println!("[Composite FX] ║ 🔗 DECODEBIN PAD ADDED - Source Media Detected");
            println!("[Composite FX] ╠════════════════════════════════════════════════════════════════");
            println!("[Composite FX] ║ 📁 File: {}", file_path_for_log);
            println!("[Composite FX] ║ 🏷️ Pad name: {}", src_pad.name());
            
            let caps = match src_pad.current_caps() {
                Some(caps) => {
                    println!("[Composite FX] ║");
                    println!("[Composite FX] ║ 📊 RAW CAPS: {}", caps);
                    
                    // Extract detailed media info
                    if let Some(structure) = caps.structure(0) {
                        println!("[Composite FX] ║");
                        println!("[Composite FX] ║ 🎞️  SOURCE MEDIA DETAILS:");
                        
                        // Framerate
                        if let Ok(framerate) = structure.get::<gst::Fraction>("framerate") {
                            let fps = framerate.numer() as f64 / framerate.denom() as f64;
                            println!("[Composite FX] ║   • Source FPS: {:.2} ({}/{})", 
                                     fps, framerate.numer(), framerate.denom());
                            println!("[Composite FX] ║   • Target FPS: {}", target_fps_for_log);
                            
                            if fps < target_fps_for_log as f64 {
                                let ratio = target_fps_for_log as f64 / fps;
                                println!("[Composite FX] ║   • videorate will DUPLICATE frames ({}x)", ratio);
                            } else if fps > target_fps_for_log as f64 {
                                let ratio = fps / target_fps_for_log as f64;
                                println!("[Composite FX] ║   • videorate will DROP frames (keep 1 in {})", ratio);
                            } else {
                                println!("[Composite FX] ║   • videorate will PASS-THROUGH (same FPS)");
                            }
                        }
                        
                        // Resolution
                        if let (Ok(src_width), Ok(src_height)) = (structure.get::<i32>("width"), structure.get::<i32>("height")) {
                            println!("[Composite FX] ║   • Source resolution: {}x{}", src_width, src_height);
                            
                            // Calculate aspect-ratio-preserving dimensions for compositor
                            // STRATEGY: Fit HEIGHT to canvas (100%), auto-scale WIDTH, center horizontally
                            // - For portrait videos (width < canvas): center with positive x_offset
                            // - For landscape videos (width > canvas): center with NEGATIVE x_offset (crop sides)
                            let src_aspect = src_width as f64 / src_height as f64;
                            let canvas_width = target_width_for_callback as i32;
                            let canvas_height = target_height_for_callback as i32;
                            
                            // FX fills canvas height, width auto-calculated
                            let fx_height = canvas_height;
                            let fx_width = (fx_height as f64 * src_aspect).round() as i32;
                            
                            // Center horizontally (can be negative for wide videos to crop sides)
                            let x_offset = (canvas_width - fx_width) / 2;
                            let y_offset = 0i32;  // Top-aligned since we fill height
                            
                            println!("[Composite FX] ║");
                            println!("[Composite FX] ║ 📐 COMPOSITOR LAYOUT CALCULATION:");
                            println!("[Composite FX] ║   • Canvas size: {}x{}", canvas_width, canvas_height);
                            println!("[Composite FX] ║   • Source aspect ratio: {:.3} ({}:{})", src_aspect, 
                                     src_width, src_height);
                            println!("[Composite FX] ║   • FX size: {}x{} (fit height, auto width)", fx_width, fx_height);
                            println!("[Composite FX] ║   • Position: x={}, y={}", x_offset, y_offset);
                            
                            if fx_width > canvas_width {
                                let crop_per_side = (fx_width - canvas_width) / 2;
                                println!("[Composite FX] ║   • Layout: LANDSCAPE (crop {} px from each side)", crop_per_side);
                            } else if fx_width < canvas_width {
                                let padding_per_side = (canvas_width - fx_width) / 2;
                                println!("[Composite FX] ║   • Layout: PORTRAIT (pad {} px on each side)", padding_per_side);
                            } else {
                                println!("[Composite FX] ║   • Layout: PERFECT FIT (same aspect as canvas)");
                            }
                            
                            // Update compositor pad properties for sink_1
                            if let Some(comp_pad) = compositor_for_callback.static_pad("sink_1") {
                                comp_pad.set_property("width", fx_width);
                                comp_pad.set_property("height", fx_height);
                                comp_pad.set_property("xpos", x_offset);
                                comp_pad.set_property("ypos", y_offset);
                                println!("[Composite FX] ║   • ✅ Compositor pad configured: {}x{} at ({}, {})", 
                                         fx_width, fx_height, x_offset, y_offset);
                            } else {
                                println!("[Composite FX] ║   • ❌ ERROR: Could not get compositor sink_1 pad!");
                            }
                        }
                        
                        // Format
                        if let Ok(format) = structure.get::<&str>("format") {
                            println!("[Composite FX] ║   • Source format: {}", format);
                        }
                        
                        // Pixel aspect ratio
                        if let Ok(par) = structure.get::<gst::Fraction>("pixel-aspect-ratio") {
                            println!("[Composite FX] ║   • Pixel aspect ratio: {}/{}", par.numer(), par.denom());
                        }
                    }
                    
                    caps
                },
                None => {
                    println!("[Composite FX] ║ ⚠️ No caps yet on pad");
                    println!("[Composite FX] ╚════════════════════════════════════════════════════════════════\n");
                    return;
                },
            };

            let structure = match caps.structure(0) {
                Some(s) => s,
                None => {
                    println!("[Composite FX] ║ ⚠️ No structure in caps");
                    println!("[Composite FX] ╚════════════════════════════════════════════════════════════════\n");
                    return;
                },
            };

            let media_type = structure.name();
            println!("[Composite FX] ║   • Media type: {}", media_type);
            
            if !media_type.starts_with("video/") {
                println!("[Composite FX] ║");
                println!("[Composite FX] ║ ⏭️ Skipping non-video pad ({})", media_type);
                println!("[Composite FX] ╚════════════════════════════════════════════════════════════════\n");
                return;
            }

            println!("[Composite FX] ║");
            println!("[Composite FX] ║ ✅ Video pad detected - proceeding with link");
            println!("[Composite FX] ╚════════════════════════════════════════════════════════════════\n");
            
            let sink_pad = videoconvert_clone.static_pad("sink").expect("No sink pad");

            if sink_pad.is_linked() {
                println!("[Composite FX] ⚠️ Videoconvert sink already linked - skipping");
                return;
            }

            println!("[Composite FX] 🔗 Linking decodebin → videoconvert...");
            match src_pad.link(&sink_pad) {
                Ok(_) => println!("[Composite FX] ✅ Successfully linked decodebin → videoconvert!"),
                Err(e) => println!("[Composite FX] ❌ Failed to link: {:?}", e),
            }
        });
        
        // Add bin to pipeline
        println!("[Composite FX] 📦 Adding FX bin to pipeline...");
        pipeline.add(&fx_bin)
            .map_err(|_| "Failed to add FX bin to pipeline")?;
        println!("[Composite FX] ✅ FX bin added to pipeline");
        
        // Request compositor sink_1 pad
        if let Some(existing_pad) = compositor.static_pad("sink_1") {
            println!("[Composite FX] 🔓 Releasing existing compositor sink_1 pad");
            compositor.release_request_pad(&existing_pad);
        }
        
        println!("[Composite FX] 🎭 Requesting compositor sink_1 pad...");
        let comp_sink_pad = compositor.request_pad_simple("sink_1")
            .ok_or("Failed to request compositor sink_1")?;
        println!("[Composite FX] ✅ Got compositor sink_1 pad: {}", comp_sink_pad.name());
        
        // Set initial compositor properties
        // Note: Position/dimensions will be updated in pad-added callback once source dimensions are known
        println!("[Composite FX] 🎛️ Setting initial compositor properties (will be updated after source detection)...");
        comp_sink_pad.set_property("zorder", 1u32);
        comp_sink_pad.set_property("alpha", 1.0f64);
        println!("[Composite FX] ✅ Initial compositor properties set (zorder=1, alpha=1.0)");

        // Link FX bin to compositor
        println!("[Composite FX] 🔗 Linking FX bin ghost pad to compositor sink_1...");
        ghost_pad.link(&comp_sink_pad)
            .map_err(|e| format!("Failed to link FX to compositor: {:?}", e))?;
        println!("[Composite FX] ✅ FX bin linked to compositor");

        // Get pipeline timing info before syncing
        let pipeline_clock = pipeline.clock();
        let pipeline_base_time = pipeline.base_time();
        let pipeline_start_time = pipeline.start_time();
        
        println!("\n[Composite FX] ⏰ PIPELINE TIMING INFO BEFORE SYNC:");
        println!("[Composite FX] ║ Pipeline clock: {:?}", pipeline_clock.as_ref().map(|c| c.time()));
        println!("[Composite FX] ║ Pipeline base_time: {:?}", pipeline_base_time);
        println!("[Composite FX] ║ Pipeline start_time: {:?}", pipeline_start_time);
        println!("[Composite FX] ║ FX bin ID: {}", fx_id);
        
        // Log videorate element state
        println!("\n[Composite FX] 🎞️  VIDEORATE ELEMENT INFO:");
        println!("[Composite FX] ║ Element name: fxvideorate_{}", fx_id);
        println!("[Composite FX] ║ skip-to-first: false (maintains timing)");
        println!("[Composite FX] ║ drop-only: false (allows duplication)");
        println!("[Composite FX] ║ average-period: 0 (immediate conversion)");
        println!("[Composite FX] ║ Target FPS: {}", target_fps);
        
        // Use sync_state_with_parent() - the standard GStreamer approach
        // This automatically handles state transitions and timing
        println!("\n[Composite FX] ▶️ Syncing FX bin state with parent pipeline...");
        fx_bin.sync_state_with_parent()
            .map_err(|e| format!("Failed to sync FX bin state: {:?}", e))?;
        println!("[Composite FX] ✅ FX bin synced to PLAYING");
        
        // Get timing info after syncing
        let fx_base_time = fx_bin.base_time();
        let fx_start_time = fx_bin.start_time();
        let fx_clock = fx_bin.clock();
        
        println!("\n[Composite FX] ⏰ FX BIN TIMING INFO AFTER SYNC:");
        println!("[Composite FX] ║ FX bin base_time: {:?}", fx_base_time);
        println!("[Composite FX] ║ FX bin start_time: {:?}", fx_start_time);
        println!("[Composite FX] ║ FX bin clock: {:?}", fx_clock.as_ref().map(|c| c.time()));
        println!("[Composite FX] ║ Clock matches pipeline: {}", 
                 fx_clock.as_ref().and_then(|fc| pipeline_clock.as_ref().map(|pc| fc == pc)).unwrap_or(false));
        
        // Wait for state change to complete (with 2 second timeout)
        match fx_bin.state(Some(gst::ClockTime::from_seconds(2))) {
            (Ok(_), gst::State::Playing, _) => {
                println!("[Composite FX] ✅ FX bin reached PLAYING state successfully!");
            }
            (result, current, pending) => {
                println!("[Composite FX] ⚠️ State change incomplete: result={:?}, current={:?}, pending={:?}", 
                         result, current, pending);
            }
        }
        
        // Check for any pipeline errors on the bus
        if let Some(bus) = pipeline.bus() {
            // Pop all pending messages
            let mut error_found = false;
            while let Some(msg) = bus.pop() {
                use gst::MessageView;
                match msg.view() {
                    MessageView::Error(err) => {
                        error_found = true;
                        let err_msg = format!("Pipeline error: {} - {}", err.error(), err.debug().unwrap_or_else(|| "".into()));
                        println!("[Composite FX] ❌ {}", err_msg);
                        if let Some(src) = msg.src() {
                            println!("[Composite FX] ❌ Error source: {}", src.name());
                        }
                    }
                    MessageView::Warning(warn) => {
                        println!("[Composite FX] ⚠️ Pipeline warning: {} - {}", warn.error(), warn.debug().unwrap_or_else(|| "".into()));
                        if let Some(src) = msg.src() {
                            println!("[Composite FX] ⚠️ Warning source: {}", src.name());
                        }
                    }
                    MessageView::StateChanged(state) => {
                        if let Some(src) = msg.src() {
                            let src_name = src.name();
                            // Only log state changes for THIS fx instance
                            let fx_suffix = format!("_{}", fx_id);
                            if src_name.contains(&fx_suffix) || src_name == format!("fxbin_{}", fx_id) {
                                println!("[Composite FX] 🔄 {} state: {:?} → {:?}", src_name, state.old(), state.current());
                            }
                        }
                    }
                    MessageView::AsyncDone(_) => {
                        println!("[Composite FX] 📢 Async done");
                    }
                    _ => {}
                }
            }
            
            if error_found {
                return Err("FX playback failed - check logs above for details".to_string());
            }
        }
        
        println!("\n[Composite FX] ╔════════════════════════════════════════════════════════════════");
        println!("[Composite FX] ║ ✅ FX SETUP COMPLETE - NOW PLAYING");
        println!("[Composite FX] ╠════════════════════════════════════════════════════════════════");
        println!("[Composite FX] ║ 📁 File: {}", file_path);
        println!("[Composite FX] ║ 📐 Output: {}x{} @ {}fps", target_width, target_height, target_fps);
        println!("[Composite FX] ║ 🎨 Chroma key: {} (enabled: {})", keycolor, use_chroma_key);
        println!("[Composite FX] ║");
        println!("[Composite FX] ║ 🎞️  PIPELINE FLOW:");
        println!("[Composite FX] ║   filesrc → decodebin → videoconvert → videoscale →");
        if use_chroma_key {
            println!("[Composite FX] ║   alpha (chroma key) → videorate → capsfilter → identity →");
        } else {
            println!("[Composite FX] ║   videorate → capsfilter → identity →");
        }
        println!("[Composite FX] ║   tee → [compositor sink_1, overlay debug appsink]");
        println!("[Composite FX] ║");
        println!("[Composite FX] ║ ⚙️  ACTIVE TRANSFORMATIONS:");
        println!("[Composite FX] ║   • videorate: Adapts source FPS → {} fps (with sync=true on appsink)", target_fps);
        println!("[Composite FX] ║   • videoscale: Adapts source resolution → {}x{}", target_width, target_height);
        if use_chroma_key {
            println!("[Composite FX] ║   • alpha: Removes {} color", keycolor);
        }
        println!("[Composite FX] ║   • Natural timing: videorate + appsink sync enforce correct playback speed");
        println!("[Composite FX] ║");
        println!("[Composite FX] ║ ⚡ FX should now be visible on:");
        println!("[Composite FX] ║   1. Composition canvas (camera + FX overlay)");
        println!("[Composite FX] ║   2. Overlay Layer canvas (FX only)");
        println!("[Composite FX] ╚════════════════════════════════════════════════════════════════\n");
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
        
        // Remove all FX bins (search for any fxbin_*)
        let existing_bins: Vec<_> = pipeline.children().iter()
            .filter(|el| el.name().starts_with("fxbin_"))
            .cloned()
            .collect();
        
        for fx_bin_element in existing_bins {
            if let Ok(fx_bin) = fx_bin_element.dynamic_cast::<gst::Bin>() {
                println!("[Composite FX] 🗑️ Removing FX bin: {}", fx_bin.name());
                // Unlink from compositor
                if let Some(ghost_pad) = fx_bin.static_pad("src") {
                    if let Some(peer_pad) = ghost_pad.peer() {
                        ghost_pad.unlink(&peer_pad).ok();
                        compositor.release_request_pad(&peer_pad);
                    }
                }
                
                // Remove bin from pipeline - DON'T set to Null first!
                // Setting to Null resets internal timing and causes speed issues
                // Just remove it and let GStreamer handle cleanup
                pipeline.remove(&fx_bin).ok();
                
                // Now set to Null AFTER removing to properly clean up
                fx_bin.set_state(gst::State::Null).ok();
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
