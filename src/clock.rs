use gstreamer as gst;
use gstreamer::prelude::*;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::broadcast;

/// Master clock system for synchronizing video compositing
pub struct MasterClock {
    pipeline: Option<gst::Pipeline>,
    clock: Option<gst::Clock>,
    target_fps: u32,
    frame_interval: gst::ClockTime,
    frame_count: Arc<RwLock<u64>>,
}

impl MasterClock {
    pub fn new(target_fps: u32) -> Self {
        let frame_interval = gst::ClockTime::from_nseconds(1_000_000_000 / target_fps as u64);

        Self {
            pipeline: None,
            clock: None,
            target_fps,
            frame_interval,
            frame_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Set the master GStreamer pipeline for clock synchronization
    pub fn set_master_pipeline(&mut self, pipeline: gst::Pipeline) {
        self.pipeline = Some(pipeline.clone());

        // Use pipeline's clock as master clock
        if let Some(clock) = pipeline.clock() {
            self.clock = Some(clock.clone());
            println!("[Master Clock] Using pipeline clock");
        } else {
            println!("[Master Clock] Warning: Pipeline has no clock");
        }
    }

    /// Get current master time
    pub fn current_time(&self) -> Option<gst::ClockTime> {
        self.clock.as_ref().and_then(|clock| clock.time())
    }

    /// Calculate the next frame PTS based on frame count
    pub fn next_frame_pts(&self) -> gst::ClockTime {
        let frame_count = *self.frame_count.read();
        gst::ClockTime::from_nseconds(frame_count * self.frame_interval.nseconds())
    }

    /// Advance to next frame
    pub fn advance_frame(&self) {
        let mut frame_count = self.frame_count.write();
        *frame_count += 1;
    }

    /// Get current frame number
    pub fn current_frame(&self) -> u64 {
        *self.frame_count.read()
    }

    /// Check if it's time to render the next frame
    pub fn should_render_frame(&self) -> bool {
        if let Some(current_time) = self.current_time() {
            let next_pts = self.next_frame_pts();
            current_time >= next_pts
        } else {
            // No clock available, render continuously
            true
        }
    }

    /// Wait until next frame time (async)
    pub async fn wait_for_next_frame(&self) {
        if let Some(current_time) = self.current_time() {
            let next_pts = self.next_frame_pts();
            if current_time < next_pts {
                let wait_time = next_pts - current_time;
                tokio::time::sleep(std::time::Duration::from_nanos(wait_time.nseconds())).await;
            }
        } else {
            // No clock, just sleep for frame interval
            tokio::time::sleep(std::time::Duration::from_nanos(self.frame_interval.nseconds())).await;
        }
    }

    /// Reset frame counter
    pub fn reset(&self) {
        *self.frame_count.write() = 0;
    }

    /// Get frame interval
    pub fn frame_interval(&self) -> gst::ClockTime {
        self.frame_interval
    }

    /// Get target FPS
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }
}

/// Frame scheduler that coordinates rendering with the master clock
#[derive(Clone)]
pub struct FrameScheduler {
    master_clock: Arc<RwLock<MasterClock>>,
    frame_sender: broadcast::Sender<FrameEvent>,
    is_running: Arc<RwLock<bool>>,
}

#[derive(Debug, Clone)]
pub enum FrameEvent {
    Render { pts: gst::ClockTime, frame_number: u64 },
    Stop,
}

impl FrameScheduler {
    pub fn new(master_clock: Arc<RwLock<MasterClock>>) -> (Self, broadcast::Receiver<FrameEvent>) {
        let (tx, rx) = broadcast::channel(16);

        let scheduler = Self {
            master_clock,
            frame_sender: tx,
            is_running: Arc::new(RwLock::new(false)),
        };

        (scheduler, rx)
    }

    /// Start the frame scheduling loop
    pub fn start(&self) {
        *self.is_running.write() = true;

        let master_clock = self.master_clock.clone();
        let frame_sender = self.frame_sender.clone();
        let is_running = self.is_running.clone();

        tokio::spawn(async move {
            println!("[Frame Scheduler] Started at {} fps", master_clock.read().target_fps());

            master_clock.write().reset();

            while *is_running.read() {
                if master_clock.read().should_render_frame() {
                    let pts = master_clock.read().next_frame_pts();
                    let frame_number = master_clock.read().current_frame();

                    // Send render event
                    let event = FrameEvent::Render { pts, frame_number };
                    let _ = frame_sender.send(event);

                    // Advance to next frame
                    master_clock.write().advance_frame();
                }

                // Small sleep to prevent busy waiting
                tokio::time::sleep(std::time::Duration::from_micros(500)).await;
            }

            // Send stop event
            let _ = frame_sender.send(FrameEvent::Stop);
            println!("[Frame Scheduler] Stopped");
        });
    }

    /// Stop the frame scheduling loop
    pub fn stop(&self) {
        *self.is_running.write() = false;
    }

    /// Check if scheduler is running
    pub fn is_running(&self) -> bool {
        *self.is_running.read()
    }
}

/// Synchronized frame buffer for handling frame data with timestamps
pub struct SynchronizedFrameBuffer {
    buffer: Arc<RwLock<Vec<FrameData>>>,
    max_size: usize,
}

#[derive(Debug, Clone)]
pub struct FrameData {
    pub pts: gst::ClockTime,
    pub data: Vec<u8>,
    pub source_id: String,
}

impl SynchronizedFrameBuffer {
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: Arc::new(RwLock::new(Vec::new())),
            max_size,
        }
    }

    /// Add a frame to the buffer
    pub fn push_frame(&self, frame: FrameData) {
        let mut buffer = self.buffer.write();

        // Remove old frames if buffer is full
        while buffer.len() >= self.max_size {
            buffer.remove(0);
        }

        buffer.push(frame);

        // Sort by PTS to maintain chronological order
        buffer.sort_by_key(|f| f.pts);
    }

    /// Get the latest frame for a specific source
    pub fn get_latest_frame(&self, source_id: &str) -> Option<FrameData> {
        let buffer = self.buffer.read();
        buffer.iter()
            .rev()
            .find(|f| f.source_id == source_id)
            .cloned()
    }

    /// Get frames that are at or before the given PTS
    pub fn get_frames_before_pts(&self, pts: gst::ClockTime) -> Vec<FrameData> {
        let buffer = self.buffer.read();
        buffer.iter()
            .filter(|f| f.pts <= pts)
            .cloned()
            .collect()
    }

    /// Clear all frames for a specific source
    pub fn clear_source(&self, source_id: &str) {
        let mut buffer = self.buffer.write();
        buffer.retain(|f| f.source_id != source_id);
    }

    /// Clear all frames
    pub fn clear_all(&self) {
        let mut buffer = self.buffer.write();
        buffer.clear();
    }

    /// Get buffer size
    pub fn len(&self) -> usize {
        self.buffer.read().len()
    }

    /// Check if buffer is empty
    pub fn is_empty(&self) -> bool {
        self.buffer.read().is_empty()
    }
}

/// Clock drift compensator for handling timing variations between sources
pub struct ClockDriftCompensator {
    source_clocks: Arc<RwLock<std::collections::HashMap<String, SourceClock>>>,
}

#[derive(Debug, Clone)]
struct SourceClock {
    last_pts: gst::ClockTime,
    drift_accumulator: f64,
    adjustment_factor: f64,
}

impl ClockDriftCompensator {
    pub fn new() -> Self {
        Self {
            source_clocks: Arc::new(RwLock::new(std::collections::HashMap::new())),
        }
    }

    /// Adjust PTS for clock drift compensation
    pub fn adjust_pts(&self, source_id: &str, original_pts: gst::ClockTime, expected_interval: gst::ClockTime) -> gst::ClockTime {
        let mut source_clocks = self.source_clocks.write();

        let source_clock = source_clocks.entry(source_id.to_string()).or_insert(SourceClock {
            last_pts: gst::ClockTime::ZERO,
            drift_accumulator: 0.0,
            adjustment_factor: 1.0,
        });

        if source_clock.last_pts != gst::ClockTime::ZERO {
            let actual_interval = original_pts - source_clock.last_pts;
            let expected_ns = expected_interval.nseconds() as f64;
            let actual_ns = actual_interval.nseconds() as f64;

            if expected_ns > 0.0 {
                let drift = (actual_ns - expected_ns) / expected_ns;
                source_clock.drift_accumulator = source_clock.drift_accumulator * 0.9 + drift * 0.1;

                // Adjust factor based on accumulated drift
                source_clock.adjustment_factor = 1.0 - (source_clock.drift_accumulator * 0.1).clamp(-0.1, 0.1);
            }
        }

        source_clock.last_pts = original_pts;

        // Apply adjustment
        let adjusted_ns = (original_pts.nseconds() as f64 * source_clock.adjustment_factor) as u64;
        gst::ClockTime::from_nseconds(adjusted_ns)
    }

    /// Reset drift compensation for a source
    pub fn reset_source(&self, source_id: &str) {
        let mut source_clocks = self.source_clocks.write();
        source_clocks.remove(source_id);
    }
}
