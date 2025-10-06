# FX Playback Timing Fix

## Problem
FX files played at correct speed on first run (30fps), but sped up progressively on subsequent plays of the same file.

### Root Causes
1. **Clock Desynchronization**: Setting `base_time = ZERO` and `start_time = NONE` caused timestamp confusion
2. **Videorate State Retention**: The `videorate` element wasn't properly resetting between plays
3. **Incomplete Cleanup**: Elements weren't fully stopped before being removed

## Fixes Applied

### 1. Improved Videorate Configuration
```rust
// Added properties to ensure proper rate limiting and state reset
.property("drop-only", true)        // Only drop frames, never duplicate
.property("skip-to-first", true)    // Start fresh, ignore previous state  
.property("max-rate", 30i32)        // Hard limit to 30fps
```

### 2. Proper Clock Synchronization
**Before:**
```rust
fx_bin.set_base_time(gst::ClockTime::ZERO);
fx_bin.set_start_time(gst::ClockTime::NONE);
```

**After:**
```rust
// Sync with parent pipeline first
fx_bin.sync_state_with_parent()?;

// Use pipeline's clock for consistent timing
if let Some(pipeline_base_time) = pipeline.base_time() {
    fx_bin.set_base_time(pipeline_base_time);
}
fx_bin.set_start_time(gst::ClockTime::ZERO);
```

### 3. Complete Element Cleanup
**Improvements:**
- Unlink from compositor FIRST to stop data flow
- Wait for state changes to complete with timeouts
- Reset all child elements (especially videorate)
- Increased cleanup delays to ensure GStreamer releases resources

```rust
// Set to NULL and WAIT for completion
if let Ok(_) = fx_bin.set_state(gst::State::Null) {
    let _ = fx_bin.state(Some(gst::ClockTime::from_seconds(1)));
}
```

## Result
- FX files now play at consistent 30fps every time
- No speed-up on subsequent plays
- Clean element state between plays
- Proper timestamp handling across multiple playbacks

## Technical Details
- Videorate enforces 30fps by dropping excess frames
- Pipeline clock synchronization ensures consistent timestamps
- Thorough cleanup prevents state carryover
- Proper state change waiting ensures GStreamer completes operations
