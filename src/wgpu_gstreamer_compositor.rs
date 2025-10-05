use crate::compositor::{Layer, WgpuCompositor};
use crate::gst::{InputManager, OutputManager, InputType, InputConfig, OutputFormat, OutputConfig};
use crate::clock::{MasterClock, FrameScheduler, SynchronizedFrameBuffer, ClockDriftCompensator, FrameEvent, FrameData};
use gstreamer as gst;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tauri::AppHandle;

/// Main compositor that integrates WGPU rendering with GStreamer I/O
#[derive(Clone)]
pub struct WgpuGStreamerCompositor {
    wgpu_compositor: Arc<RwLock<WgpuCompositor>>,
    input_manager: Arc<RwLock<InputManager>>,
    output_manager: Arc<RwLock<OutputManager>>,
    master_clock: Arc<RwLock<MasterClock>>,
    frame_scheduler: Option<FrameScheduler>,
    frame_buffer: Arc<SynchronizedFrameBuffer>,
    drift_compensator: Arc<ClockDriftCompensator>,
    is_running: Arc<RwLock<bool>>,
    app_handle: AppHandle,

    // Compositing settings
    width: u32,
    height: u32,
    fps: u32,
}

impl WgpuGStreamerCompositor {
    pub async fn new(width: u32, height: u32, fps: u32) -> Result<Self, String> {
        println!("[WGPU-GST Compositor] Creating compositor: {}x{} @ {}fps", width, height, fps);

        // Initialize GStreamer
        gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;

        // Create WGPU compositor
        let wgpu_compositor = WgpuCompositor::new_offscreen(width, height, fps).await?;
        let wgpu_compositor = Arc::new(RwLock::new(wgpu_compositor));

        // Create managers
        let input_manager = Arc::new(RwLock::new(InputManager::new()));
        let output_manager = Arc::new(RwLock::new(OutputManager::new()));

        // Create clock system
        let master_clock = Arc::new(RwLock::new(MasterClock::new(fps)));
        let frame_buffer = Arc::new(SynchronizedFrameBuffer::new(32)); // Buffer up to 32 frames per source
        let drift_compensator = Arc::new(ClockDriftCompensator::new());

        Ok(Self {
            wgpu_compositor,
            input_manager,
            output_manager,
            master_clock,
            frame_scheduler: None,
            frame_buffer,
            drift_compensator,
            is_running: Arc::new(RwLock::new(false)),
            width,
            height,
            fps,
        })
    }

    /// Add a camera input source
    pub async fn add_camera_input(&self, id: String, device_index: u32) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Adding camera input: {} (device {})", id, device_index);

        let config = InputConfig {
            input_type: InputType::Camera { device_index },
            width: self.width,
            height: self.height,
            framerate: self.fps,
            is_live: true,
        };

        let mut input = crate::gst::GStreamerInput::new(id.clone(), config)?;

        // Set up frame receiver
        let (tx, rx) = broadcast::channel::<Vec<u8>>(32);
        input.set_frame_sender(tx);

        // Start input
        input.start()?;

        // Create layer for this input
        let mut layer = Layer::new(id.clone());
        layer = layer.with_z_order(0); // Camera on bottom

        // Add to managers
        {
            let mut input_mgr = self.input_manager.write();
            input_mgr.add_input(input)?;
        }

        {
            let mut wgpu = self.wgpu_compositor.write();
            wgpu.add_layer(layer);
        }

        // Set up frame buffer receiver
        let frame_buffer = self.frame_buffer.clone();
        let drift_compensator = self.drift_compensator.clone();
        let input_id = id.clone();

        tokio::spawn(async move {
            let mut rx = rx;
            while let Ok(frame_data) = rx.recv().await {
                let pts = gst::ClockTime::from_nseconds(
                    (chrono::Utc::now().timestamp_nanos() % 1_000_000_000) as u64
                );

                let adjusted_pts = drift_compensator.adjust_pts(
                    &input_id,
                    pts,
                    gst::ClockTime::from_seconds(1) / 60 // Assume 60fps for drift calculation
                );

                let frame = FrameData {
                    pts: adjusted_pts,
                    data: frame_data,
                    source_id: input_id.clone(),
                };

                frame_buffer.push_frame(frame);
            }
        });

        Ok(())
    }

    /// Add a media file input source
    pub async fn add_media_input(&self, id: String, file_path: String) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Adding media input: {} ({})", id, file_path);

        let uri = format!("file://{}", file_path);
        let config = InputConfig {
            input_type: InputType::File { uri },
            width: self.width,
            height: self.height,
            framerate: self.fps,
            is_live: false,
        };

        let mut input = crate::gst::GStreamerInput::new(id.clone(), config)?;

        // Set up frame receiver
        let (tx, rx) = broadcast::channel::<Vec<u8>>(32);
        input.set_frame_sender(tx);

        // Start input
        input.start()?;

        // Create layer for this input
        let mut layer = Layer::new(id.clone());
        layer = layer.with_z_order(1); // Media files above camera

        // Add to managers
        {
            let mut input_mgr = self.input_manager.write();
            input_mgr.add_input(input)?;
        }

        {
            let mut wgpu = self.wgpu_compositor.write();
            wgpu.add_layer(layer);
        }

        // Set up frame buffer receiver
        let frame_buffer = self.frame_buffer.clone();
        let input_id = id.clone();

        tokio::spawn(async move {
            let mut rx = rx;
            while let Ok(frame_data) = rx.recv().await {
                let pts = gst::ClockTime::from_nseconds(
                    (chrono::Utc::now().timestamp_nanos() % 1_000_000_000) as u64
                );

                let frame = FrameData {
                    pts,
                    data: frame_data,
                    source_id: input_id.clone(),
                };

                frame_buffer.push_frame(frame);
            }
        });

        Ok(())
    }

    /// Add an output destination
    pub async fn add_output(&self, id: String, format: OutputFormat) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Adding output: {} ({:?})", id, format);

        let config = OutputConfig {
            width: self.width,
            height: self.height,
            framerate: self.fps,
            format,
        };

        let mut output = crate::gst::GStreamerOutput::new(id, config)?;

        // Set up frame receiver for composited output
        let (tx, rx) = broadcast::channel::<Vec<u8>>(32);
        output.set_frame_receiver(rx);

        // Start output
        output.start()?;

        // Add to manager
        let mut output_mgr = self.output_manager.write();
        output_mgr.add_output(output)?;

        // Store sender for composited frames
        // TODO: Store this for the rendering loop

        Ok(())
    }

    /// Start the compositor
    pub async fn start(&mut self) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Starting compositor");

        *self.is_running.write() = true;

        // Create master pipeline for clock synchronization
        let master_pipeline = gst::Pipeline::new();
        self.master_clock.write().set_master_pipeline(master_pipeline);

        // Create frame scheduler
        let (scheduler, mut frame_rx) = FrameScheduler::new(self.master_clock.clone());
        self.frame_scheduler = Some(scheduler);
        self.frame_scheduler.as_ref().unwrap().start();

        // Start compositing loop
        let wgpu_compositor = self.wgpu_compositor.clone();
        let frame_buffer = self.frame_buffer.clone();
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            println!("[WGPU-GST Compositor] Compositing loop started");

            while *is_running.read() {
                // Wait for next frame event
                match frame_rx.recv().await {
                    Ok(FrameEvent::Render { pts, frame_number }) => {
                        // Get latest frames for all sources
                        let camera_frame = frame_buffer.get_latest_frame("camera");
                        let media_frame = frame_buffer.get_latest_frame("media");

                        // Update WGPU textures and render
                        let (camera_texture, media_texture) = {
                            let output_size = {
                                let wgpu = wgpu_compositor.read();
                                wgpu.output_size()
                            };

                            let camera_texture = camera_frame.as_ref().map(|frame| {
                                let mut wgpu = wgpu_compositor.write();
                                wgpu.create_texture_from_rgba(output_size.0, output_size.1, &frame.data)
                            });

                            let media_texture = media_frame.as_ref().map(|frame| {
                                let mut wgpu = wgpu_compositor.write();
                                wgpu.create_texture_from_rgba(output_size.0, output_size.1, &frame.data)
                            });

                            (camera_texture, media_texture)
                        };

                        // Update layer textures
                        {
                            let mut wgpu = wgpu_compositor.write();
                            if let Some(texture) = camera_texture {
                                if let Some(layer) = wgpu.get_layer_mut("camera") {
                                    layer.update_texture(texture);
                                }
                            }
                            if let Some(texture) = media_texture {
                                if let Some(layer) = wgpu.get_layer_mut("media") {
                                    layer.update_texture(texture);
                                }
                            }

                            // Render frame
                            let output_texture = wgpu.create_output_texture();
                            if let Err(e) = wgpu.render_frame(&output_texture) {
                                println!("[WGPU-GST Compositor] Render error: {:?}", e);
                                continue;
                            }

                            // Read back composited frame
                            match wgpu.read_output_texture(&output_texture) {
                                Ok(_data) => {
                                    // TODO: Send to output channels
                                }
                                Err(e) => {
                                    println!("[WGPU-GST Compositor] Readback error: {:?}", e);
                                    continue;
                                }
                            }
                        }

                        if frame_number % 60 == 0 {
                            println!("[WGPU-GST Compositor] Rendered frame {}", frame_number);
                        }
                    }
                    Ok(FrameEvent::Stop) => {
                        println!("[WGPU-GST Compositor] Received stop event");
                        break;
                    }
                    Err(e) => {
                        match e {
                            tokio::sync::broadcast::error::RecvError::Lagged(lag) => {
                                println!("[WGPU-GST Compositor] Frame receiver lagged by {} frames, catching up...", lag);
                                // Continue the loop - this is recoverable
                                continue;
                            }
                            _ => {
                                println!("[WGPU-GST Compositor] Frame receiver error: {:?}", e);
                                break;
                            }
                        }
                    }
                }
            }

            println!("[WGPU-GST Compositor] Compositing loop stopped");
        });

        Ok(())
    }

    /// Stop the compositor
    pub async fn stop(&mut self) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Stopping compositor");

        *self.is_running.write() = false;

        if let Some(scheduler) = &self.frame_scheduler {
            scheduler.stop();
        }

        // Stop all inputs
        {
            let mut input_mgr = self.input_manager.write();
            input_mgr.stop_all()?;
        }

        // Stop all outputs
        {
            let mut output_mgr = self.output_manager.write();
            output_mgr.stop_all()?;
        }

        Ok(())
    }

    /// Set layer transform
    pub fn set_layer_transform(&self, layer_id: &str, position: glam::Vec2, scale: glam::Vec2, rotation: f32) {
        let mut wgpu = self.wgpu_compositor.write();
        wgpu.set_transform(layer_id, position, scale, rotation);
    }

    /// Set layer opacity
    pub fn set_layer_opacity(&self, layer_id: &str, opacity: f32) {
        let mut wgpu = self.wgpu_compositor.write();
        if let Some(layer) = wgpu.get_layer_mut(layer_id) {
            layer.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    /// Set chroma key for a layer
    pub fn set_chroma_key(&self, layer_id: &str, r: f32, g: f32, b: f32, tolerance: f32) {
        let mut wgpu = self.wgpu_compositor.write();
        if let Some(layer) = wgpu.get_layer_mut(layer_id) {
            layer.chroma_key = Some([r, g, b]);
            layer.chroma_tolerance = tolerance;
        }
    }

    /// Enable/disable layer
    pub fn set_layer_visible(&self, layer_id: &str, visible: bool) {
        let mut wgpu = self.wgpu_compositor.write();
        if let Some(layer) = wgpu.get_layer_mut(layer_id) {
            layer.visible = visible;
        }
    }

    /// Get compositor status
    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }

    /// Get current frame count
    pub fn current_frame(&self) -> u64 {
        self.master_clock.read().current_frame()
    }
}
