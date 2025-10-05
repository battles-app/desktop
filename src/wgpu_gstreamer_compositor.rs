use crate::compositor::{Layer, WgpuCompositor};
use crate::gst::{InputManager, OutputManager, InputType, InputConfig, OutputFormat, OutputConfig};
use crate::clock::{MasterClock, FrameScheduler, SynchronizedFrameBuffer, ClockDriftCompensator, FrameEvent, FrameData};
use gstreamer as gst;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::broadcast;
use tauri::{AppHandle, Emitter};
use base64::Engine;

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
    pub async fn new(width: u32, height: u32, fps: u32, app_handle: AppHandle) -> Result<Self, String> {
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
            app_handle,
            width,
            height,
            fps,
        })
    }

    /// Add a camera input source
    pub async fn add_camera_input(&self, id: String, device_index: u32) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Adding camera input: {} (device {}) at {}x{} @ {}fps", id, device_index, self.width, self.height, self.fps);

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
        layer = layer.with_position(0.0, 0.0); // Position at top-left
        layer = layer.with_scale(1.0, 1.0); // Full size
        layer = layer.with_opacity(1.0); // Fully opaque

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
        let app_handle = self.app_handle.clone();
        let camera_width = self.width;
        let camera_height = self.height;

        tokio::spawn(async move {
            let mut rx = rx;
            while let Ok(frame_data) = rx.recv().await {
                // Send camera frames directly to frontend for immediate display
                if input_id.starts_with("camera_") {
                    println!("[Camera] Sending frame to frontend: {}x{} ({} bytes)", camera_width, camera_height, frame_data.len());
                    Self::send_frame_to_frontend(&app_handle, "camera-layer-frame", &frame_data, camera_width, camera_height);
                }

                // Also store in frame buffer for compositing (optional for now)
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

    /// Remove a camera input source
    pub async fn remove_camera_input(&self, id: String) -> Result<(), String> {
        println!("[WGPU-GST Compositor] Removing camera input: {}", id);

        // Remove from input manager
        {
            let mut input_mgr = self.input_manager.write();
            input_mgr.remove_input(&id)
                .map_err(|e| format!("Failed to remove input: {}", e))?;
        }

        // Remove layer from WGPU compositor
        {
            let mut wgpu = self.wgpu_compositor.write();
            if !wgpu.remove_layer(&id) {
                println!("[WGPU-GST Compositor] Warning: Layer {} was not found", id);
            }
        }

        println!("[WGPU-GST Compositor] Camera input removed: {}", id);
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
        let app_handle = self.app_handle.clone();
        let width = self.width;
        let height = self.height;

        tokio::spawn(async move {
            println!("[WGPU-GST Compositor] Compositing loop started");

            while *is_running.read() {
                // Wait for next frame event
                match frame_rx.recv().await {
                    Ok(FrameEvent::Render { pts, frame_number }) => {
                        // Update textures for all layers
                        {
                            let output_size = {
                                let wgpu = wgpu_compositor.read();
                                wgpu.output_size()
                            };

                            let mut wgpu = wgpu_compositor.write();
                            let layer_ids: Vec<String> = wgpu.layers().keys().cloned().collect();

                            // Update textures for each layer
                            for layer_id in &layer_ids {
                                if let Some(frame) = frame_buffer.get_latest_frame(layer_id) {
                                    println!("[WGPU-GST Compositor] Processing frame for layer {}: {} bytes", layer_id, frame.data.len());

                                    // Send raw frame to frontend for debugging (camera layers only)
                                    if layer_id.starts_with("camera") {
                                        Self::send_frame_to_frontend(&app_handle, "camera-layer-frame", &frame.data, 1920, 1080); // Camera native resolution
                                    }

                                    // Use actual frame dimensions (camera is 1920x1080)
                                    let texture = wgpu.create_texture_from_rgba(1920, 1080, &frame.data);
                                    if let Some(layer) = wgpu.get_layer_mut(layer_id) {
                                        layer.update_texture(texture);
                                        // Update the texture array so bind group gets created
                                        wgpu.update_texture_array();
                                    }
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
                                Ok(rgba_data) => {
                                    println!("[WGPU-GST Compositor] Read back {} bytes of composited data", rgba_data.len());

                                    // Check if data is not all zeros (first 100 bytes)
                                    let first_bytes: Vec<u8> = rgba_data.iter().take(100).cloned().collect();
                                    let has_content = first_bytes.iter().any(|&b| b != 0);
                                    println!("[WGPU-GST Compositor] First 100 bytes have content: {}", has_content);

                                    // Send frames to frontend for debugging
                                    println!("[WGPU-GST Compositor] Sending composite-frame to frontend");
                                    Self::send_frame_to_frontend(&app_handle, "composite-layer-frame", &rgba_data, width, height);
                                    Self::send_frame_to_frontend(&app_handle, "composite-frame", &rgba_data, width, height);
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

    /// Get compositor dimensions
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Get compositor fps
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Get current frame count
    pub fn current_frame(&self) -> u64 {
        self.master_clock.read().current_frame()
    }

    /// Send frame data to frontend via Tauri events
    fn send_frame_to_frontend(app_handle: &AppHandle, event_name: &str, rgba_data: &[u8], width: u32, height: u32) {
        println!("[WGPU-GST Compositor] send_frame_to_frontend: {} ({}x{} = {} bytes)", event_name, width, height, rgba_data.len());

        // Create RGBA image from raw data
        if let Some(img) = image::RgbaImage::from_raw(width, height, rgba_data.to_vec()) {
            println!("[WGPU-GST Compositor] Created RGBA image for {}", event_name);

            // Convert to RGB for JPEG encoding
            let rgb_img = image::DynamicImage::ImageRgba8(img).to_rgb8();

            // Encode as JPEG
            let mut jpeg_data = Vec::new();
            if let Ok(_) = rgb_img.write_to(&mut std::io::Cursor::new(&mut jpeg_data), image::ImageFormat::Jpeg) {
                println!("[WGPU-GST Compositor] Encoded JPEG for {}: {} bytes", event_name, jpeg_data.len());

                // Encode as base64
                let base64_frame = base64::engine::general_purpose::STANDARD.encode(&jpeg_data);
                println!("[WGPU-GST Compositor] Encoded base64 for {}: {} bytes", event_name, base64_frame.len());

                // Send to frontend
                match app_handle.emit(event_name, base64_frame) {
                    Ok(_) => println!("[WGPU-GST Compositor] Successfully emitted {} event", event_name),
                    Err(e) => println!("[WGPU-GST Compositor] Failed to emit {} event: {:?}", event_name, e),
                }
            } else {
                println!("[WGPU-GST Compositor] Failed to encode JPEG for {}", event_name);
            }
        } else {
            println!("[WGPU-GST Compositor] Failed to create RGBA image for {} ({}x{})", event_name, width, height);
        }
    }
}
