// GStreamer composite pipeline for OBS-like functionality
use gstreamer::prelude::*;
use gstreamer::{self as gst, Pipeline};
use gstreamer_app::AppSink;
use tokio::sync::broadcast;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use parking_lot::RwLock;

// GPU compositor
use crate::WgpuCompositor;

// Tauri for app handle
use tauri;

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
    // GPU compositor for real-time alpha blending
    wgpu_compositor: Option<Arc<parking_lot::Mutex<WgpuCompositor>>>,
    // Tauri app handle for emitting events
    tauri_app: Option<tauri::AppHandle>,
    // Render task cancellation flag
    render_cancelled: Arc<std::sync::atomic::AtomicBool>,
    // Handle to the render task for cleanup
    render_task_handle: Option<tokio::task::JoinHandle<()>>,
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
    pub cleanup_in_progress: Arc<parking_lot::Mutex<bool>>, // Prevent double cleanup
    pub playback_id: u64, // Unique ID to prevent old EOS probes from interfering
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
            wgpu_compositor: None,
            tauri_app: None,
            render_cancelled: Arc::new(AtomicBool::new(false)),
            render_task_handle: None,
        })
    }
    
    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write() = Some(sender);
    }

    pub fn set_tauri_app(&mut self, app: tauri::AppHandle) {
        self.tauri_app = Some(app);
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
        println!("[Composite] Starting composite pipeline with GPU compositor: {}x{} @ {}fps (rotation: {}°)", width, height, fps, rotation);

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

        // Initialize WGPU compositor
        let wgpu_compositor = Arc::new(parking_lot::Mutex::new(WgpuCompositor::new(width, height)
            .map_err(|e| format!("Failed to create WGPU compositor: {:?}", e))?));
        self.wgpu_compositor = Some(wgpu_compositor.clone());

        // Build simplified GStreamer pipeline - camera only, ends at RGBA appsink
        #[cfg(target_os = "windows")]
        let pipeline_str = if videoflip_method != "none" {
            format!(
                "mfvideosrc device-index={} ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoflip method={} ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoconvert ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoscale ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 video/x-raw,width={},height={},format=RGBA ! \
                 appsink name=camera_sink emit-signals=true sync=false max-buffers=2 drop=true",
                device_index,
                videoflip_method,
                width,
                height
            )
        } else {
            format!(
                "mfvideosrc device-index={} ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoconvert ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 videoscale ! \
                 queue leaky=downstream max-size-buffers=3 ! \
                 video/x-raw,width={},height={},format=RGBA ! \
                 appsink name=camera_sink emit-signals=true sync=false max-buffers=2 drop=true",
                device_index,
                width,
                height
            )
        };

        #[cfg(target_os = "linux")]
        let pipeline_str = format!(
            "v4l2src device=/dev/video{} ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoconvert ! \
             queue leaky=downstream max-size-buffers=3 ! \
             videoscale ! \
             queue leaky=downstream max-size-buffers=3 ! \
             video/x-raw,width={},height={},format=RGBA ! \
             appsink name=camera_sink emit-signals=true sync=false max-buffers=2 drop=true",
            device_index,
            width,
            height
        );

        println!("[Composite] ⚡ Camera pipeline (RGBA output): {}", pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;

        // Get the camera appsink
        let camera_appsink = pipeline
            .by_name("camera_sink")
            .ok_or("Failed to get camera appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;

        // Set up camera appsink callbacks to feed WGPU compositor
        let wgpu_for_camera = wgpu_compositor.clone();
        let is_running = self.is_running.clone();

        camera_appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;

                    // Extract dimensions from caps
                    let structure = caps.structure(0).ok_or(gst::FlowError::Error)?;
                    let width: i32 = structure.get("width").map_err(|_| gst::FlowError::Error)?;
                    let height: i32 = structure.get("height").map_err(|_| gst::FlowError::Error)?;

                    let rgba_data = map.as_slice();

                    // Update camera texture in WGPU compositor
                    if let Some(mut compositor) = wgpu_for_camera.try_lock() {
                        let _ = compositor.update_camera_rgba(width as u32, height as u32, width as u32 * 4, rgba_data);
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Start pipeline
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {}", e))?;

        println!("[Composite] ✅ Camera pipeline started - feeding GPU compositor");

        // Start render loop
        self.start_render_loop();

        self.pipeline = Some(pipeline);
        Ok(())
    }

    fn start_render_loop(&mut self) {
        let wgpu_compositor = match &self.wgpu_compositor {
            Some(c) => c.clone(),
            None => return,
        };

        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        let cancelled = self.render_cancelled.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(33)); // ~30fps

            loop {
                interval.tick().await;

                if cancelled.load(Ordering::Relaxed) || !*is_running.read() {
                    break;
                }

                // Check if there are any active subscribers before rendering
                let has_subscribers = frame_sender.read().as_ref()
                    .map(|sender| sender.receiver_count() > 0)
                    .unwrap_or(false);

                if !has_subscribers {
                    // No subscribers, skip rendering to avoid memory accumulation
                    continue;
                }

                // Render frame only when there are subscribers
                if let Some(mut compositor_guard) = wgpu_compositor.try_lock() {
                    if let Ok(rgba_data) = compositor_guard.render_rgba() {
                        // Send RGBA data over WebSocket broadcast
                        if let Some(sender) = frame_sender.read().as_ref() {
                            // Send frame (broadcast channel will drop if no receivers or channel full)
                            match sender.send(rgba_data) {
                                Ok(_) => {
                                    // Frame sent successfully to at least one receiver
                                }
                                Err(_) => {
                                    // No receivers or channel full - this is expected behavior
                                    // The broadcast channel drops frames when full, preventing accumulation
                                }
                            }
                        }
                    }
                }
            }

            println!("[Composite] Render loop stopped");
        });

        self.render_task_handle = Some(handle);
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

        // Cancel render task
        self.render_cancelled.store(true, Ordering::Relaxed);

        // Don't wait for the task to finish to avoid async issues
        // The task will be cancelled and cleaned up automatically
        self.render_task_handle.take();

        if let Some(pipeline) = &self.pipeline {
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {}", e))?;
        }

        self.pipeline = None;
        self.wgpu_compositor = None; // Drop WGPU resources
        println!("[Composite] Composite pipeline stopped");

        Ok(())
    }
    
    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }

    pub fn get_pipeline_state(&self) -> Option<gst::State> {
        self.pipeline.as_ref().map(|p| p.current_state())
    }

    pub fn get_subscriber_count(&self) -> usize {
        self.frame_sender.read().as_ref()
            .map(|sender| sender.receiver_count())
            .unwrap_or(0)
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
        
        // Stop any existing FX first (simple cleanup since no compositor pads)
        if let Some(existing_fx_bin) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Cleanup of existing FX pipeline...");

            // Cast to Bin and remove it
            if let Ok(bin) = existing_fx_bin.dynamic_cast::<gst::Bin>() {
                let _ = bin.set_state(gst::State::Null);
                let remove_result = std::panic::catch_unwind(|| {
                    pipeline.remove(&bin)
                });

                match remove_result {
                    Ok(result) => {
                        if result.is_ok() {
                            println!("[Composite FX] ✅ FX bin removed from pipeline");
                        } else {
                            println!("[Composite FX] ⚠️ FX bin removal failed");
                        }
                    }
                    Err(e) => println!("[Composite FX] ⚠️ Pipeline removal panicked: {:?}", e),
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

        // RGBA caps for WGPU compositor
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGBA")
            .build();

        println!("[Composite FX] 🎬 Forced 30fps H.264 MP4 playback - RGBA output for GPU compositor");

        let capsfilter = ElementFactory::make("capsfilter")
            .name("fxcaps")
            .property("caps", &caps)
            .build()
            .map_err(|_| "Failed to create capsfilter")?;

        // Create FX appsink
        let fx_appsink = ElementFactory::make("appsink")
            .name("fx_sink")
            .property("emit-signals", true)
            .property("sync", false)
            .property("max-buffers", 2u32)
            .property("drop", true)
            .build()
            .map_err(|_| "Failed to create FX appsink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| "Failed to cast FX appsink")?;

        // Set uridecodebin to async for raw playback
        uridecode.set_property("async-handling", true);

        // Create bin to hold FX elements
        let fx_bin = gst::Bin::builder().name("fxbin").build();

        // Pipeline: uridecodebin -> videorate -> rate_filter -> identity_sync -> videoconvert -> videoscale -> capsfilter -> appsink
        fx_bin.add_many(&[&uridecode, &videorate, &rate_filter, &identity_sync, &videoconvert, &videoscale, &capsfilter, &fx_appsink.upcast_ref()])
            .map_err(|_| "Failed to add elements to FX bin")?;

        // Link elements: videorate enforces 30fps, identity syncs to real-time clock
        gst::Element::link_many(&[&videorate, &rate_filter, &identity_sync, &videoconvert, &videoscale, &capsfilter, &fx_appsink.upcast_ref()])
            .map_err(|_| "Failed to link FX elements")?;

        let final_element = fx_appsink.upcast_ref::<gst::Element>().clone();

        // Create ghost pad on the bin (for EOS probe)
        let final_src_pad = final_element.static_pad("src")
            .ok_or("Failed to get final element src pad")?;
        let ghost_pad = gst::GhostPad::with_target(&final_src_pad)
            .map_err(|_| "Failed to create ghost pad")?;
        ghost_pad.set_active(true).ok();
        fx_bin.add_pad(&ghost_pad).map_err(|_| "Failed to add ghost pad to bin")?;

        // Set up FX appsink callbacks to feed WGPU compositor
        let wgpu_for_fx = self.wgpu_compositor.as_ref().unwrap().clone();
        let is_running_fx = self.is_running.clone();

        fx_appsink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running_fx.read() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let caps = sample.caps().ok_or(gst::FlowError::Error)?;

                    // Extract dimensions from caps
                    let structure = caps.structure(0).ok_or(gst::FlowError::Error)?;
                    let width: i32 = structure.get("width").map_err(|_| gst::FlowError::Error)?;
                    let height: i32 = structure.get("height").map_err(|_| gst::FlowError::Error)?;

                    let rgba_data = map.as_slice();

                    // Update FX texture in WGPU compositor
                    if let Some(mut compositor) = wgpu_for_fx.try_lock() {
                        let _ = compositor.update_fx_rgba(width as u32, height as u32, width as u32 * 4, rgba_data);
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        
        // Add EOS (End-of-Stream) probe to detect when video finishes naturally
        println!("[Composite FX] 📡 Adding EOS probe for auto-cleanup (playback_id: {})...", playback_id);
        let fx_bin_weak = fx_bin.downgrade();
        let pipeline_weak = pipeline.downgrade();
        let fx_state_weak = Arc::downgrade(&self.fx_state);
        let eos_playback_id = playback_id; // Capture current playback ID

        ghost_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
            if let Some(gst::PadProbeData::Event(ref event)) = info.data {
                if event.type_() == gst::EventType::Eos {
                    println!("[Composite FX] 🎬 Video finished (EOS) - auto-cleaning in 100ms...");

                    // Spawn cleanup task (don't block probe callback)
                    let fx_bin_weak_clone = fx_bin_weak.clone();
                    let pipeline_weak_clone = pipeline_weak.clone();
                    let fx_state_weak_clone = fx_state_weak.clone();

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

                        if let (Some(fx_bin), Some(pipeline)) =
                            (fx_bin_weak_clone.upgrade(), pipeline_weak_clone.upgrade()) {

                            // Check if this bin is still actually in the pipeline (might have been manually cleaned up)
                            let has_parent = fx_bin.parent().is_some();

                            if !has_parent {
                                println!("[Composite FX] ⚠️ EOS cleanup skipped - bin already cleaned up (no parent)");
                                return;
                            }

                            println!("[Composite FX] 🧹 EOS Auto-cleanup: Starting cleanup...");

                            // Stop and remove bin
                            let _ = fx_bin.set_state(gst::State::Null);

                            // Remove from pipeline
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

                            // Clear FX state
                            if let Some(fx_state_arc) = fx_state_weak_clone.upgrade() {
                                *fx_state_arc.write() = None;
                                println!("[Composite FX] ✅ FX state cleared");
                            }

                            // Clear FX texture in WGPU compositor (set alpha to 0)
                            // Note: We can't access the compositor from this thread context,
                            // so we rely on the manual stop to handle this

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
        
        // Set FX positioning in WGPU compositor
        let comp_width = *self.pipeline_width.read() as f32;
        let comp_height = *self.pipeline_height.read() as f32;

        // Calculate FX positioning: center and fill height
        // Assume 16:9 FX aspect ratio for horizontal videos
        let fx_aspect = 16.0 / 9.0;
        let comp_aspect = comp_width / comp_height;

        let (fx_x, fx_y, fx_width, fx_height) = if comp_aspect > 1.0 {
            // Horizontal compositor (16:9): Fill full width and height
            (0.0, 0.0, 1.0, 1.0)
        } else {
            // Vertical compositor (9:16): Fill height, center horizontally and crop edges
            let fx_w = comp_height * fx_aspect / comp_width;
            let fx_x_pos = (1.0 - fx_w) / 2.0; // Center horizontally (will crop edges)
            (fx_x_pos, 0.0, fx_w, 1.0)
        };

        println!("[Composite FX] 📐 Positioning: {:.2}x{:.2} at ({:.2}, {:.2}) normalized coords",
                 fx_width, fx_height, fx_x, fx_y);

        // Set FX params in WGPU compositor
        if let Some(wgpu_comp) = &self.wgpu_compositor {
            if let Some(mut comp) = wgpu_comp.try_lock() {
                comp.set_fx_params(fx_x, fx_y, fx_width, fx_height, self.layers.read().overlay_opacity as f32);
            }
        }

        // Sync FX bin state with pipeline
        fx_bin.sync_state_with_parent()
            .map_err(|_| "Failed to sync FX bin state".to_string())?;

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

        // Find and remove FX bin
        if let Some(fx_bin_element) = pipeline.by_name("fxbin") {
            println!("[Composite FX] 🧹 Removing FX bin...");

            // Cast to Bin and remove it
            if let Ok(fx_bin) = fx_bin_element.dynamic_cast::<gst::Bin>() {
                // Stop the bin
                let _ = fx_bin.set_state(gst::State::Null);

                // Remove from pipeline
                let remove_result = std::panic::catch_unwind(|| {
                    pipeline.remove(&fx_bin)
                });

                match remove_result {
                    Ok(result) => {
                        if result.is_ok() {
                            println!("[Composite FX] ✅ FX bin removed successfully");
                        } else {
                            println!("[Composite FX] ⚠️ FX bin removal failed");
                        }
                    }
                    Err(e) => println!("[Composite FX] ⚠️ Pipeline removal panicked: {:?}", e),
                }

                println!("[Composite FX] ✅ FX branch removed and memory freed");
            }
        } else {
            println!("[Composite FX] No FX bin found to remove");
        }

        // Clear FX texture in WGPU compositor (set to transparent)
        if let Some(wgpu_comp) = &self.wgpu_compositor {
            if let Some(mut comp) = wgpu_comp.try_lock() {
                // Set FX alpha to 0 to hide it
                comp.set_fx_params(0.0, 0.0, 1.0, 1.0, 0.0);
            }
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
                    // Stop the bin
                    let set_state_result = std::panic::catch_unwind(|| {
                        bin.set_state(gst::State::Null)
                    });

                    match set_state_result {
                        Ok(_) => println!("[Composite FX] ✅ Emergency: Bin set to NULL"),
                        Err(e) => println!("[Composite FX] ⚠️ Emergency: Set state failed: {:?}", e),
                    }

                    // Remove from pipeline
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
}

impl Drop for GStreamerComposite {
    fn drop(&mut self) {
        // Ensure proper cleanup on drop
        let _ = self.stop();
    }
}



