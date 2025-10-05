use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;

/// A clock that synchronizes with GStreamer's pipeline clock
pub struct SyncClock {
    /// The GStreamer pipeline clock
    gst_clock: Option<gst::Clock>,
    
    /// The base time of the pipeline
    base_time: Arc<Mutex<gst::ClockTime>>,
    
    /// The reference time for the local clock
    reference_time: Arc<Mutex<Instant>>,
    
    /// The offset between the GStreamer clock and the local clock
    offset: Arc<Mutex<i64>>,
}

impl SyncClock {
    /// Create a new synchronized clock
    pub fn new() -> Self {
        Self {
            gst_clock: None,
            base_time: Arc::new(Mutex::new(gst::ClockTime::ZERO)),
            reference_time: Arc::new(Mutex::new(Instant::now())),
            offset: Arc::new(Mutex::new(0)),
        }
    }
    
    /// Set the GStreamer pipeline clock
    pub fn set_pipeline_clock(&mut self, pipeline: &gst::Pipeline) -> Result<()> {
        let clock = pipeline.clock().ok_or_else(|| anyhow!("No pipeline clock"))?;
        let base_time = pipeline.base_time();
        
        *self.base_time.lock().unwrap() = base_time;
        *self.reference_time.lock().unwrap() = Instant::now();
        
        // Calculate the offset between the GStreamer clock and the local clock
        let gst_time = clock.time().ok_or_else(|| anyhow!("Failed to get clock time"))?;
        let local_time = Instant::now();
        let reference_time = *self.reference_time.lock().unwrap();
        let local_elapsed = local_time.duration_since(reference_time).as_nanos() as i64;
        
        let gst_elapsed = (gst_time - base_time).nseconds() as i64;
        let offset = gst_elapsed - local_elapsed;
        
        *self.offset.lock().unwrap() = offset;
        self.gst_clock = Some(clock);
        
        Ok(())
    }
    
    /// Get the current time in nanoseconds
    pub fn time(&self) -> u64 {
        if self.gst_clock.is_none() {
            // Fall back to local time if no GStreamer clock is available
            let now = Instant::now();
            let reference_time = *self.reference_time.lock().unwrap();
            return now.duration_since(reference_time).as_nanos() as u64;
        }
        
        // Get the GStreamer clock time
        let gst_time = match self.gst_clock.as_ref().unwrap().time() {
            Some(time) => time,
            None => return Instant::now().elapsed().as_nanos() as u64,
        };
        
        // Adjust for the base time
        let base_time = *self.base_time.lock().unwrap();
        if gst_time < base_time {
            return 0;
        }
        
        (gst_time - base_time).nseconds()
    }
    
    /// Sleep until the specified time in nanoseconds
    pub fn sleep_until(&self, target_time_ns: u64) {
        let current_time_ns = self.time();
        
        if current_time_ns >= target_time_ns {
            // Target time has already passed
            return;
        }
        
        let sleep_duration_ns = target_time_ns - current_time_ns;
        std::thread::sleep(Duration::from_nanos(sleep_duration_ns));
    }
    
    /// Calculate the presentation timestamp (PTS) for a frame
    pub fn calculate_pts(&self, frame_index: u64, fps: u32) -> u64 {
        let frame_duration_ns = 1_000_000_000 / fps as u64;
        frame_index * frame_duration_ns
    }
}

/// A frame clock that ticks at a fixed rate
#[derive(Clone)]
pub struct FrameClock {
    /// The target frames per second
    fps: u32,
    
    /// The frame interval in nanoseconds
    frame_interval_ns: u64,
    
    /// The last frame time in nanoseconds
    last_frame_time: Arc<Mutex<u64>>,
    
    /// The frame counter
    frame_counter: Arc<Mutex<u64>>,
    
    /// The sync clock
    sync_clock: Arc<SyncClock>,
}

impl FrameClock {
    /// Create a new frame clock
    pub fn new(fps: u32, sync_clock: Arc<SyncClock>) -> Self {
        let frame_interval_ns = 1_000_000_000 / fps as u64;
        
        Self {
            fps,
            frame_interval_ns,
            last_frame_time: Arc::new(Mutex::new(0)),
            frame_counter: Arc::new(Mutex::new(0)),
            sync_clock,
        }
    }
    
    /// Wait for the next frame
    pub fn wait_for_next_frame(&self) -> u64 {
        let mut last_frame_time = self.last_frame_time.lock().unwrap();
        let mut frame_counter = self.frame_counter.lock().unwrap();
        
        // Calculate the target time for the next frame
        let current_time = self.sync_clock.time();
        let target_time = *last_frame_time + self.frame_interval_ns;
        
        // If we're behind, catch up
        let next_frame_time = if current_time >= target_time + self.frame_interval_ns {
            // We're more than one frame behind, skip to the current time
            current_time
        } else {
            // Wait until the target time
            self.sync_clock.sleep_until(target_time);
            target_time
        };
        
        // Update the last frame time and frame counter
        *last_frame_time = next_frame_time;
        *frame_counter += 1;
        
        next_frame_time
    }
    
    /// Get the current frame index
    pub fn frame_index(&self) -> u64 {
        *self.frame_counter.lock().unwrap()
    }
    
    /// Get the frame duration in nanoseconds
    pub fn frame_duration_ns(&self) -> u64 {
        self.frame_interval_ns
    }
    
    /// Get the frames per second
    pub fn fps(&self) -> u32 {
        self.fps
    }
    
    /// Reset the frame counter
    pub fn reset(&self) {
        *self.last_frame_time.lock().unwrap() = self.sync_clock.time();
        *self.frame_counter.lock().unwrap() = 0;
    }
}
