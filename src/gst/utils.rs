use gstreamer as gst;

/// Utility functions for GStreamer operations
pub struct GStreamerUtils;

impl GStreamerUtils {
    /// Convert hex color string to RGB tuple (0.0-1.0 range)
    pub fn hex_to_rgb(hex: &str) -> Result<(f32, f32, f32), String> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return Err(format!("Invalid hex color: {}", hex));
        }

        let r = u8::from_str_radix(&hex[0..2], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))? as f32 / 255.0;
        let g = u8::from_str_radix(&hex[2..4], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))? as f32 / 255.0;
        let b = u8::from_str_radix(&hex[4..6], 16)
            .map_err(|_| format!("Invalid hex color: {}", hex))? as f32 / 255.0;

        Ok((r, g, b))
    }

    /// Create RGBA caps string for given dimensions and framerate
    pub fn create_rgba_caps(width: u32, height: u32, framerate: u32) -> String {
        format!("video/x-raw,format=RGBA,width={},height={},framerate={}/1",
                width, height, framerate)
    }

    /// Get pipeline clock time safely
    pub fn get_pipeline_time(pipeline: &gst::Pipeline) -> Option<gst::ClockTime> {
        pipeline.clock().and_then(|clock| Some(clock.time()))
    }

    /// Check if pipeline is in playing state
    pub fn is_pipeline_playing(pipeline: &gst::Pipeline) -> bool {
        matches!(pipeline.current_state(), gst::State::Playing)
    }

    /// Safely set pipeline state with error handling
    pub fn set_pipeline_state(pipeline: &gst::Pipeline, state: gst::State) -> Result<(), String> {
        pipeline.set_state(state)
            .map_err(|e| format!("Failed to set pipeline state to {:?}: {}", state, e))?;
        Ok(())
    }

    /// Wait for pipeline state change with timeout
    pub fn wait_for_state_change(pipeline: &gst::Pipeline, state: gst::State, timeout_ms: u64) -> Result<(), String> {
        let timeout = gst::ClockTime::from_mseconds(timeout_ms);

        match pipeline.state(Some(timeout)) {
            (Ok(current), _, _) if current == state => Ok(()),
            (result, current, pending) => {
                Err(format!("State change failed: result={:?}, current={:?}, pending={:?}",
                           result, current, pending))
            }
        }
    }

    /// Create a basic video test source pipeline for testing
    pub fn create_test_pipeline(width: u32, height: u32, framerate: u32) -> Result<gst::Pipeline, String> {
        let pipeline_str = format!(
            "videotestsrc is-live=true ! \
             video/x-raw,format=RGBA,width={},height={},framerate={}/1 ! \
             appsink name=sink emit-signals=true sync=false max-buffers=2 drop=true",
            width, height, framerate
        );

        gst::parse::launch(&pipeline_str)
            .map_err(|e| format!("Failed to create test pipeline: {}", e))?
            .dynamic_cast::<gst::Pipeline>()
            .map_err(|_| "Failed to cast to Pipeline".to_string())
    }

    /// Extract video info from caps
    pub fn extract_video_info(caps: &gst::Caps) -> Option<(u32, u32, u32)> {
        caps.structure(0).and_then(|structure| {
            let width = structure.get::<i32>("width").ok()? as u32;
            let height = structure.get::<i32>("height").ok()? as u32;
            let framerate = structure.get::<gst::Fraction>("framerate").ok()?;
            let fps = framerate.numer() as u32 / framerate.denom() as u32;

            Some((width, height, fps))
        })
    }

    /// Check if caps represent video format
    pub fn is_video_caps(caps: &gst::Caps) -> bool {
        caps.structure(0)
            .map(|s| s.name().starts_with("video/"))
            .unwrap_or(false)
    }

    /// Check if caps represent audio format
    pub fn is_audio_caps(caps: &gst::Caps) -> bool {
        caps.structure(0)
            .map(|s| s.name().starts_with("audio/"))
            .unwrap_or(false)
    }
}

/// Frame timing utilities
pub struct FrameTiming;

impl FrameTiming {
    /// Calculate frame duration from framerate
    pub fn frame_duration(framerate: u32) -> gst::ClockTime {
        gst::ClockTime::from_seconds(1) / framerate
    }

    /// Calculate framerate from frame duration
    pub fn framerate_from_duration(duration: gst::ClockTime) -> u32 {
        (gst::ClockTime::from_seconds(1).nseconds() / duration.nseconds()) as u32
    }

    /// Convert PTS to frame number
    pub fn pts_to_frame_number(pts: gst::ClockTime, framerate: u32) -> u64 {
        (pts.seconds() * framerate as u64) + (pts.nseconds() % (1_000_000_000 / framerate as u64)) as u64
    }

    /// Convert frame number to PTS
    pub fn frame_number_to_pts(frame_number: u64, framerate: u32) -> gst::ClockTime {
        gst::ClockTime::from_seconds(frame_number / framerate as u64) +
        gst::ClockTime::from_nseconds((frame_number % framerate as u64) * (1_000_000_000 / framerate as u64))
    }
}

/// Pipeline monitoring utilities
pub struct PipelineMonitor;

impl PipelineMonitor {
    /// Set up basic pipeline message monitoring
    pub fn setup_message_handler(pipeline: &gst::Pipeline, name: &str) {
        let bus = pipeline.bus().unwrap();
        let name = name.to_string();

        std::thread::spawn(move || {
            for msg in bus.iter_timed(gst::ClockTime::NONE) {
                use gst::MessageView;

                match msg.view() {
                    MessageView::Error(err) => {
                        println!("[GST {}] ERROR: {} - {}", name, err.error(), err.debug().unwrap_or_default());
                    }
                    MessageView::Warning(warn) => {
                        println!("[GST {}] WARNING: {} - {}", name, warn.error(), warn.debug().unwrap_or_default());
                    }
                    MessageView::StateChanged(state) => {
                        if let (Some(src), Some(old), Some(new)) = (msg.src(), state.old(), state.current()) {
                            if src.name() == name {
                                println!("[GST {}] State: {:?} -> {:?}", name, old, new);
                            }
                        }
                    }
                    MessageView::Eos(_) => {
                        println!("[GST {}] End of stream", name);
                        break;
                    }
                    _ => {}
                }
            }
        });
    }

    /// Get pipeline position (current playback time)
    pub fn get_position(pipeline: &gst::Pipeline) -> Option<gst::ClockTime> {
        pipeline.query_position::<gst::ClockTime>()
    }

    /// Get pipeline duration
    pub fn get_duration(pipeline: &gst::Pipeline) -> Option<gst::ClockTime> {
        pipeline.query_duration::<gst::ClockTime>()
    }

    /// Check if pipeline is at end of stream
    pub fn is_eos(pipeline: &gst::Pipeline) -> bool {
        Self::get_position(pipeline).map_or(false, |pos| {
            Self::get_duration(pipeline).map_or(false, |dur| pos >= dur)
        })
    }
}
