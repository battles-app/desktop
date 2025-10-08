// Screen capture using GStreamer for monitor preview
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

pub struct ScreenCaptureMonitor {
    monitor_index: usize,
    pipeline: Option<gst::Pipeline>,
    frame_sender: Arc<RwLock<Option<broadcast::Sender<Vec<u8>>>>>,
    is_running: Arc<RwLock<bool>>,
}

impl ScreenCaptureMonitor {
    pub fn new(monitor_index: usize) -> Result<Self, String> {
        Ok(Self {
            monitor_index,
            pipeline: None,
            frame_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        })
    }

    pub fn set_frame_sender(&self, sender: broadcast::Sender<Vec<u8>>) {
        *self.frame_sender.write().unwrap() = Some(sender);
    }

    pub fn start(&mut self, monitor_x: i32, monitor_y: i32, monitor_width: u32, monitor_height: u32) -> Result<(), String> {
        println!("[Screen Capture {}] Starting capture at ({}, {}) {}x{}", 
            self.monitor_index, monitor_x, monitor_y, monitor_width, monitor_height);

        // Calculate preview dimensions (max 320x180 for low bandwidth)
        let preview_width = 320u32;
        let preview_height = ((preview_width as f64 / monitor_width as f64) * monitor_height as f64) as u32;

        // Build GStreamer pipeline for screen capture
        // Using dx11screencapturesrc (Windows) or d3d11screencapturesrc
        let pipeline_str = format!(
            "d3d11screencapturesrc monitor-index={} ! \
             video/x-raw(memory:D3D11Memory),format=BGRA ! \
             d3d11convert ! \
             video/x-raw,format=RGBA ! \
             videoscale ! \
             video/x-raw,width={},height={} ! \
             videoconvert ! \
             video/x-raw,format=RGBA ! \
             appsink name=sink",
            self.monitor_index,
            preview_width,
            preview_height
        );

        println!("[Screen Capture {}] Pipeline: {}", self.monitor_index, pipeline_str);

        let pipeline = gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create pipeline: {}", e))?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())?;

        // Get appsink
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink")?
            .dynamic_cast::<gst_app::AppSink>()
            .map_err(|_| "Failed to cast to AppSink")?;

        // Set appsink properties for low latency
        appsink.set_property("max-buffers", 2u32);
        appsink.set_property("drop", true);
        appsink.set_property("emit-signals", false);

        // Set up callbacks for frame delivery
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();
        let monitor_index = self.monitor_index;
        let mut frame_count = 0u64;

        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    if !*is_running.read().unwrap() {
                        return Ok(gst::FlowSuccess::Ok);
                    }

                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;

                    frame_count += 1;

                    // Log every 60 frames (2 seconds at 30fps)
                    if frame_count % 60 == 0 {
                        println!("[Screen Capture {}] Frame {} ({} bytes)", 
                            monitor_index, frame_count, map.len());
                    }

                    // Broadcast frame to WebSocket
                    if let Some(sender) = &*frame_sender.read().unwrap() {
                        let _ = sender.send(map.as_slice().to_vec());
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        // Start pipeline
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start pipeline: {:?}", e))?;

        self.pipeline = Some(pipeline);
        *self.is_running.write().unwrap() = true;

        println!("[Screen Capture {}] ✅ Started successfully", self.monitor_index);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), String> {
        println!("[Screen Capture {}] Stopping...", self.monitor_index);
        *self.is_running.write().unwrap() = false;

        if let Some(pipeline) = &self.pipeline {
            pipeline
                .set_state(gst::State::Null)
                .map_err(|e| format!("Failed to stop pipeline: {:?}", e))?;
        }

        self.pipeline = None;
        *self.frame_sender.write().unwrap() = None;

        println!("[Screen Capture {}] ✅ Stopped", self.monitor_index);
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        *self.is_running.read().unwrap()
    }
}

impl Drop for ScreenCaptureMonitor {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

