use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use glam::Vec2;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::clock::{FrameClock, SyncClock};
use crate::compositor::{Layer, WgpuCompositor};
use crate::gst::{GstInput, GstOutput, OutputFormat};

/// Represents the WGPU-based compositor
#[derive(Clone)]
pub struct WgpuComposite {
    /// The WGPU compositor
    compositor: Arc<Mutex<WgpuCompositor>>,
    
    /// The input sources
    inputs: Arc<Mutex<HashMap<String, GstInput>>>,
    
    /// The output
    output: Arc<Mutex<Option<GstOutput>>>,
    
    /// The frame sender for the composite output
    frame_sender: Arc<Mutex<Option<broadcast::Sender<Vec<u8>>>>>,
    
    /// The frame sender for the camera layer
    camera_frame_sender: Arc<Mutex<Option<broadcast::Sender<Vec<u8>>>>>,
    
    /// The frame sender for the overlay layer
    overlay_frame_sender: Arc<Mutex<Option<broadcast::Sender<Vec<u8>>>>>,
    
    /// The sync clock
    sync_clock: Arc<SyncClock>,
    
    /// The frame clock
    frame_clock: Arc<Mutex<Option<FrameClock>>>,
    
    /// The compositor task handle
    compositor_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    
    /// Whether the compositor is running
    is_running: Arc<Mutex<bool>>,
    
    /// The target width
    width: u32,
    
    /// The target height
    height: u32,
    
    /// The target FPS
    fps: u32,
}

impl WgpuComposite {
    /// Create a new WGPU compositor
    pub async fn new() -> Result<Self> {
        // Initialize GStreamer
        crate::gst::utils::init()?;
        
        // Create the sync clock
        let sync_clock = Arc::new(SyncClock::new());
        
        // Create the WGPU compositor with default dimensions and FPS
        let width = 1280;
        let height = 720;
        let fps = 30;
        
        let compositor = WgpuCompositor::new(width, height, fps).await?;
        
        Ok(Self {
            compositor: Arc::new(Mutex::new(compositor)),
            inputs: Arc::new(Mutex::new(HashMap::new())),
            output: Arc::new(Mutex::new(None)),
            frame_sender: Arc::new(Mutex::new(None)),
            camera_frame_sender: Arc::new(Mutex::new(None)),
            overlay_frame_sender: Arc::new(Mutex::new(None)),
            sync_clock,
            frame_clock: Arc::new(Mutex::new(None)),
            compositor_task: Arc::new(Mutex::new(None)),
            is_running: Arc::new(Mutex::new(false)),
            width,
            height,
            fps,
        })
    }
    
    /// Set the frame sender for the composite output
    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        // Clone the sender before storing it
        let sender_clone = sender.clone();
        *self.frame_sender.lock().unwrap() = Some(sender);
        
        // Also set it on the compositor
        self.compositor.lock().unwrap().set_frame_sender(sender_clone);
    }
    
    /// Set the frame sender for the camera layer
    pub fn set_camera_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.camera_frame_sender.lock().unwrap() = Some(sender);
    }
    
    /// Set the frame sender for the overlay layer
    pub fn set_overlay_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.overlay_frame_sender.lock().unwrap() = Some(sender);
    }
    
    /// Start the compositor with the specified camera and dimensions
    pub async fn start(
        &mut self,
        camera_device_id: &str,
        width: u32,
        height: u32,
        fps: u32,
        _rotation: u32,
    ) -> Result<()> {
        // Stop any existing compositor
        self.stop()?;
        
        // Update dimensions and FPS
        self.width = width;
        self.height = height;
        self.fps = fps;
        
        // Create a new compositor with the updated dimensions and FPS
        let mut compositor = WgpuCompositor::new(width, height, fps).await?;
        
        // Use the global frame sender to ensure it's synchronized with IPC
        {
            // Get the global frame sender first
            let global_sender_opt = {
                let global_sender_lock = crate::COMPOSITE_FRAME_SENDER.read();
                global_sender_lock.as_ref().cloned()
            };
            
            // Update our local frame sender with the global one
            let mut frame_sender_lock = self.frame_sender.lock().unwrap();
            
            if let Some(global_sender) = global_sender_opt {
                // Use the global sender if it exists
                println!("[WgpuComposite] Using global frame sender with {} receivers", global_sender.receiver_count());
                *frame_sender_lock = Some(global_sender.clone());
                
                // Set the frame sender on the compositor
                compositor.set_frame_sender(global_sender);
            } else {
                // Create a new sender if global doesn't exist (should not happen)
                println!("[WgpuComposite] WARNING: No global frame sender found, creating new one");
                let (tx, _rx) = broadcast::channel::<Vec<u8>>(10);
                
                // Update both local and global senders
                *frame_sender_lock = Some(tx.clone());
                compositor.set_frame_sender(tx.clone());
                
                // Update the global sender
                let mut global_sender_write = crate::COMPOSITE_FRAME_SENDER.write();
                *global_sender_write = Some(tx);
            }
        }
        
        // Use the global camera frame sender
        {
            // Get the global camera frame sender
            let global_sender_opt = {
                let global_sender_lock = crate::CAMERA_LAYER_FRAME_SENDER.read();
                global_sender_lock.as_ref().cloned()
            };
            
            // Update our local camera frame sender
            let mut camera_frame_sender_lock = self.camera_frame_sender.lock().unwrap();
            
            if let Some(global_sender) = global_sender_opt {
                // Use the global sender if it exists
                println!("[WgpuComposite] Using global camera frame sender with {} receivers", global_sender.receiver_count());
                *camera_frame_sender_lock = Some(global_sender);
            } else {
                // Create a new sender if global doesn't exist (should not happen)
                println!("[WgpuComposite] WARNING: No global camera frame sender found, creating new one");
                let (tx, _rx) = broadcast::channel::<Vec<u8>>(10);
                
                // Update both local and global senders
                *camera_frame_sender_lock = Some(tx.clone());
                
                // Update the global sender
                let mut global_sender_write = crate::CAMERA_LAYER_FRAME_SENDER.write();
                *global_sender_write = Some(tx);
            }
            
            println!("[WgpuComposite] Camera frame sender has {} receivers", 
                camera_frame_sender_lock.as_ref().map_or(0, |tx| tx.receiver_count()));
        }
        
        // Use the global overlay frame sender
        {
            // Get the global overlay frame sender
            let global_sender_opt = {
                let global_sender_lock = crate::OVERLAY_LAYER_FRAME_SENDER.read();
                global_sender_lock.as_ref().cloned()
            };
            
            // Update our local overlay frame sender
            let mut overlay_frame_sender_lock = self.overlay_frame_sender.lock().unwrap();
            
            if let Some(global_sender) = global_sender_opt {
                // Use the global sender if it exists
                println!("[WgpuComposite] Using global overlay frame sender with {} receivers", global_sender.receiver_count());
                *overlay_frame_sender_lock = Some(global_sender);
            } else {
                // Create a new sender if global doesn't exist (should not happen)
                println!("[WgpuComposite] WARNING: No global overlay frame sender found, creating new one");
                let (tx, _rx) = broadcast::channel::<Vec<u8>>(10);
                
                // Update both local and global senders
                *overlay_frame_sender_lock = Some(tx.clone());
                
                // Update the global sender
                let mut global_sender_write = crate::OVERLAY_LAYER_FRAME_SENDER.write();
                *global_sender_write = Some(tx);
            }
            
            println!("[WgpuComposite] Overlay frame sender has {} receivers", 
                overlay_frame_sender_lock.as_ref().map_or(0, |tx| tx.receiver_count()));
        }
        
        // Replace the compositor
        *self.compositor.lock().unwrap() = compositor;
        
        // Create the camera input
        let camera_input = GstInput::new_camera(camera_device_id, width, height, fps, "camera")?;
        
        // Create a channel for the camera frames
        let (camera_tx, camera_rx) = broadcast::channel::<(Vec<u8>, u64, u64)>(5);
        camera_input.set_frame_sender(camera_tx);
        
        // Start the camera input
        camera_input.start()?;
        
        // Add the camera input to the inputs map
        self.inputs.lock().unwrap().insert("camera".to_string(), camera_input);
        
        // Create the frame clock
        let frame_clock = FrameClock::new(fps, self.sync_clock.clone());
        *self.frame_clock.lock().unwrap() = Some(frame_clock.clone());
        
        // Mark as running
        *self.is_running.lock().unwrap() = true;
        
        // Create a camera layer in the compositor
        let camera_layer = Layer::new(
            "camera".to_string(),
            Mat4::IDENTITY,
            1.0,
            0.0,
            0,
            true,
            0,
            0,
            width,
            height,
        );
        println!("[WgpuComposite] Adding camera layer to compositor");
        self.compositor.lock().unwrap().add_layer(camera_layer)?;
        
        // Add a debug layer to ensure we have at least one visible layer
        println!("[WgpuComposite] Adding debug layer to compositor");
        let debug_layer = Layer::new(
            "debug_layer".to_string(),
            Mat4::IDENTITY,
            1.0,
            0.0,
            1,
            true,
            0,
            0,
            width,
            height,
        );
        
        // Add the debug layer to the compositor
        match self.compositor.lock().unwrap().add_layer(debug_layer) {
            Ok(_) => println!("[WgpuComposite] Debug layer added successfully"),
            Err(e) => println!("[WgpuComposite] Failed to add debug layer: {}", e),
        };
        
        // Start the compositor task
        let compositor_clone = self.compositor.clone();
        let is_running_clone = self.is_running.clone();
        let frame_clock_clone = Arc::new(frame_clock);
        let camera_frame_sender_clone = self.camera_frame_sender.clone();
        let _overlay_frame_sender_clone = self.overlay_frame_sender.clone();
        
        let task = tokio::spawn(async move {
            // Create a receiver for camera frames
            let mut camera_rx = camera_rx;
            
            while *is_running_clone.lock().unwrap() {
                // Wait for the next frame time
                let frame_time = frame_clock_clone.wait_for_next_frame();
                
                // Try to receive a camera frame
                if let Ok((camera_data, camera_pts, camera_duration)) = camera_rx.try_recv() {
                    // Update the camera layer
                    if let Err(e) = compositor_clone.lock().unwrap().update_layer_texture(
                        "camera",
                        &camera_data,
                        width,
                        height,
                        camera_pts,
                        camera_duration,
                    ) {
                        eprintln!("Failed to update camera layer: {}", e);
                    }
                    
                    // Forward the camera frame to listeners
                    if let Some(sender) = camera_frame_sender_clone.lock().unwrap().as_ref() {
                        println!("[WgpuComposite] Sending camera frame to listeners, size: {}", camera_data.len());
                        match sender.send(camera_data) {
                            Ok(_) => println!("[WgpuComposite] Camera frame sent successfully to {} receivers", sender.receiver_count()),
                            Err(e) => println!("[WgpuComposite] Error sending camera frame: {}", e),
                        }
                    } else {
                        println!("[WgpuComposite] No camera frame sender available");
                    }
                }
                
                // Render the composite frame
                match compositor_clone.lock().unwrap().render(frame_time) {
                    Ok(_frame_data) => {
                        // Frame rendered successfully
                        // The compositor already sends the frame to listeners
                    }
                    Err(e) => {
                        // Skip frame if not ready or no layers
                        if !e.to_string().contains("Not time for a new frame yet")
                            && !e.to_string().contains("No visible layers to render")
                        {
                            eprintln!("Error rendering frame: {}", e);
                        }
                    }
                }
            }
        });
        
        // Store the task handle
        *self.compositor_task.lock().unwrap() = Some(task);
        
        Ok(())
    }
    
    /// Stop the compositor
    pub fn stop(&self) -> Result<()> {
        // Mark as not running
        *self.is_running.lock().unwrap() = false;
        
        // Wait for the compositor task to finish
        let _task = self.compositor_task.lock().unwrap().take();
        // Don't block on the task, just let it finish naturally
        // The is_running flag will cause it to exit
        
        // Stop all inputs
        for (_, input) in self.inputs.lock().unwrap().drain() {
            let _ = input.stop();
        }
        
        // Stop the output
        if let Some(output) = self.output.lock().unwrap().take() {
            let _ = output.stop();
        }
        
        Ok(())
    }
    
    /// Update the layer visibility and opacity
    pub fn update_layers(&self, camera: (bool, f64), overlay: (bool, f64)) -> Result<()> {
        // Update the camera layer
        self.compositor.lock().unwrap().set_layer_visibility("camera", camera.0)?;
        self.compositor.lock().unwrap().set_layer_transform(
            "camera",
            Vec2::ZERO,
            Vec2::ONE,
            0.0,
            camera.1 as f32,
        )?;
        
        // Update the overlay layer if it exists
        if self.compositor.lock().unwrap().set_layer_visibility("overlay", overlay.0).is_ok() {
            self.compositor.lock().unwrap().set_layer_transform(
                "overlay",
                Vec2::ZERO,
                Vec2::ONE,
                0.0,
                overlay.1 as f32,
            )?;
        }
        
        Ok(())
    }
    
    /// Set the output format
    pub fn set_output_format(&self, format: &str, width: u32, height: u32) -> Result<()> {
        // Parse the output format
        let output_format = match format {
            "rtmp" => OutputFormat::Rtmp,
            "virtual_camera" => OutputFormat::VirtualCamera,
            "preview" => OutputFormat::Preview,
            _ => return Err(anyhow!("Invalid output format: {}", format)),
        };
        
        // Stop any existing output
        if let Some(output) = self.output.lock().unwrap().take() {
            let _ = output.stop();
        }
        
        // If preview only, we're done
        if output_format == OutputFormat::Preview {
            return Ok(());
        }
        
        // Create the new output
        let output = GstOutput::new(
            output_format,
            width,
            height,
            self.fps,
            None,
        )?;
        
        // Start the output
        output.start()?;
        
        // Store the output
        *self.output.lock().unwrap() = Some(output);
        
        Ok(())
    }
    
    /// Play an FX from a file
    pub fn play_fx_from_file(
        &self,
        file_path: String,
        _keycolor: String,
        _tolerance: f64,
        _similarity: f64,
        _use_chroma_key: bool,
    ) -> Result<()> {
        // Stop any existing FX
        self.stop_fx()?;
        
        // Create the FX input
        let fx_input = GstInput::new_file(&file_path, self.width, self.height, "overlay")?;
        
        // Create a channel for the FX frames
        let (fx_tx, mut fx_rx) = broadcast::channel::<(Vec<u8>, u64, u64)>(2);
        fx_input.set_frame_sender(fx_tx);
        
        // Start the FX input
        fx_input.start()?;
        
        // Add the FX input to the inputs map
        self.inputs.lock().unwrap().insert("overlay".to_string(), fx_input);
        
        // Create an overlay layer in the compositor
        let mut overlay_layer = Layer::new("overlay");
        overlay_layer.z_order = 1; // Above the camera
        self.compositor.lock().unwrap().add_layer(overlay_layer)?;
        
        // Start a task to process FX frames
        let compositor_clone = self.compositor.clone();
        let is_running_clone = self.is_running.clone();
        let width = self.width;
        let height = self.height;
        let overlay_frame_sender_clone = self.overlay_frame_sender.clone();
        
        tokio::spawn(async move {
            while *is_running_clone.lock().unwrap() {
                // Try to receive an FX frame
                if let Ok((fx_data, fx_pts, fx_duration)) = fx_rx.recv().await {
                    // Update the overlay layer
                    if let Err(e) = compositor_clone.lock().unwrap().update_layer_texture(
                        "overlay",
                        &fx_data,
                        width,
                        height,
                        fx_pts,
                        fx_duration,
                    ) {
                        eprintln!("Failed to update overlay layer: {}", e);
                    }
                    
                    // Forward the overlay frame to listeners
                    if let Some(sender) = overlay_frame_sender_clone.lock().unwrap().as_ref() {
                        let _ = sender.send(fx_data);
                    }
                }
            }
        });
        
        Ok(())
    }
    
    /// Stop the FX
    pub fn stop_fx(&self) -> Result<()> {
        // Remove the overlay input if it exists
        if let Some(overlay_input) = self.inputs.lock().unwrap().remove("overlay") {
            let _ = overlay_input.stop();
        }
        
        // Remove the overlay layer from the compositor
        let _ = self.compositor.lock().unwrap().remove_layer("overlay");
        
        Ok(())
    }
}
