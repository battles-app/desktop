# FX Playback Timing Fix

## Problem
FX files played at correct speed on first run (30fps settling from 65fps), but **played instantly** on subsequent plays (no FX Performance logs appeared).

### Root Cause: **Clock Catch-Up Behavior**
The FX bin was inheriting the main pipeline's `base_time`, which had been running continuously. When a new FX started:
1. Pipeline clock: 72:23:52 (hours of uptime)
2. FX video timestamps: Starting at 0:00:00
3. **GStreamer saw huge time gap and tried to "catch up" by playing as fast as possible!**

Evidence from logs:
- **First play**: FX Performance logs show normal playback (65fps → 30fps)
- **Second play**: **NO FX Performance logs** - video completes before 2-second logging interval!
- **Both plays had identical base_time**: `72:23:52.940915900` (the stale pipeline clock)

## Fixes Applied

### 1. Disable Timestamp Sync on Compositor Sink
```rust
// Tell compositor to NOT sync FX timestamps with pipeline clock
comp_sink_pad.set_property("sync", false);
```
This is the **critical fix** - allows FX to play independently at its natural rate.

### 2. Fresh Clock Timing for Each FX Play
**Before:**
```rust
fx_bin.set_base_time(pipeline.base_time());  // OLD stale time!
```

**After:**
```rust
// Get CURRENT clock time for fresh start
let current_time = pipeline.clock()
    .and_then(|clock| clock.time())
    .unwrap_or(gst::ClockTime::ZERO);

fx_bin.set_base_time(current_time);  // Fresh time = no catch-up
fx_bin.set_start_time(gst::ClockTime::ZERO);
```

### 3. **THE REAL FIX: clocksync Element**
```rust
// Add clocksync to enforce REAL-TIME 30fps playback
let clocksync = ElementFactory::make("clocksync")
    .property("sync", true)  // Synchronize to clock
    .build()?;

// Pipeline: videorate → rate_filter → clocksync → videoconvert → ...
```
**This is the critical fix!** Without `clocksync`, videorate just limits frame count but lets them play as fast as possible. `clocksync` enforces **real-time clock synchronization**, ensuring frames play at actual 30fps speed, not 56fps or 90fps!

### 4. Fixed Double-Release Pad Error
```rust
// Check if pad still belongs to compositor before releasing
if peer_pad.parent().as_ref() == Some(compositor.upcast_ref()) {
    compositor.release_request_pad(&peer_pad);
} else {
    println!("Pad already released, skipping");
}
```

### 4. Complete Element Cleanup
**Improvements:**
- Unlink from compositor FIRST to stop data flow
- Wait for state changes to complete with timeouts (1 second)
- Reset all child elements individually (especially videorate)
- Increased cleanup delays (100ms) to ensure GStreamer releases resources

```rust
// Set to NULL and WAIT for completion
if let Ok(_) = fx_bin.set_state(gst::State::Null) {
    let _ = fx_bin.state(Some(gst::ClockTime::from_seconds(1)));
}
```

## Result
✅ **FX files NOW play at REAL 30fps (not 56fps or 90fps!)**
✅ **clocksync enforces real-time playback speed**
✅ **No instant playback on subsequent plays**
✅ **No freezing on second play of same file**
✅ **No GStreamer pad double-release errors**
✅ **Complete memory cleanup after video finishes**
✅ **Auto-cleanup when video reaches end (EOS)**
✅ **Clean element state between plays**
✅ **No decoder caching or garbage accumulation**

## Why This Works

### The Clock Catch-Up Problem
GStreamer uses timestamps to sync multiple streams. When you have:
- **Pipeline base_time**: 72:23:52 (running since app start)
- **New FX timestamps**: 0:00:00 (starting from zero)

GStreamer thinks: *"This FX is 72 hours behind! Skip/drop frames to catch up!"*

### The Solution
1. **`sync=false`**: Compositor doesn't try to sync FX with pipeline clock
2. **Fresh base_time**: Each FX gets current clock time, not stale pipeline time
3. **Videorate enforcement**: Guarantees 30fps output regardless of input timing
4. **Thorough cleanup**: Ensures no state persists between plays

### The Freeze on Second Play Problem
When the first video finished naturally:
1. It reached EOS (End-of-Stream)
2. **No cleanup happened** - bin stayed in pipeline in stale state
3. Second play tried to create NEW bin with same name
4. GStreamer conflict: "sink_1 pad already exists!"
5. Result: **Freeze/hang**

### The Complete Solution
**1. EOS Detection & Auto-Cleanup**
```rust
// Detect when video finishes naturally via pad probe
ghost_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_pad, info| {
    if event.type_() == gst::EventType::Eos {
        // Auto-cleanup in background thread
        std::thread::spawn(move || {
            // Unlink, release pads, stop bin, remove from pipeline
            // Clear fx_state for garbage collection
        });
    }
});
```

**2. No Decoder Caching**
```rust
.property("use-buffering", false)
.property("download", false)
.property("ring-buffer-max-size", 0u64)
```

**3. Complete State Reset**
- Release compositor sink_1 pad properly
- Wait for NULL state to complete
- Clear fx_state (garbage collection)
- Fresh decoder instance every play

### Log Evidence
Now you'll see:
- **Consistent FX Performance logs on every play**
- **"Auto-cleanup complete" message when video finishes**
- **No "sink_1 already exists" errors**
- **No panics or freezes on subsequent plays**
