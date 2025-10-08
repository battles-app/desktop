# Direct WGPU Surface Rendering Implementation Plan

## Overview

Transform from: `Camera → GPU → CPU → WebSocket → Browser → GPU → Display`
To: `Camera → GPU → Display` (native window rendering)

## Implementation Steps

### Phase 1: Add Window Handle Support ✅

1. Add `raw-window-handle` to Cargo.toml
2. Create new Tauri command to get window handle
3. Pass window handle to WGPU renderer

### Phase 2: Create Surface-Based Renderer

1. Modify `WgpuChromaRenderer::new()` to accept surface
2. Replace output texture with surface texture
3. Remove readback buffers (no longer needed)
4. Add surface configuration

### Phase 3: Update Rendering Loop

1. Modify GStreamer callback to render to surface
2. Remove GPU→CPU readback code
3. Call `surface.present()` instead of `broadcast`
4. Remove WebSocket frame sending

### Phase 4: Frontend Changes

1. Remove canvas element (WGPU renders to window now)
2. Keep only UI overlay (controls, status)
3. Position UI over native rendering
4. Update WebSocket to control-only (no frames)

### Phase 5: Cleanup

1. Remove `COMPOSITE_FRAME_SENDER`
2. Remove WebSocket server for frames
3. Remove `start_composite_ws_server`
4. Keep only FX command WebSocket

## Code Changes Required

### File: `battlesDesktop/src/main.rs`

**Add command:**
```rust
#[command]
fn get_composite_window_handle(app: tauri::AppHandle) -> Result<String, String> {
    // Return window handle as hex string
}
```

### File: `battlesDesktop/src/gstreamer_composite.rs`

**Major changes:**
- `WgpuChromaRenderer` now stores `Surface` instead of output texture
- Remove all readback code (900+ lines!)
- `render_to_surface()` instead of `render_frame_async()`
- Call `present()` on each frame

### File: `battles.app/components/CompositeCanvas.vue`

**Simplify:**
- Remove `<canvas>` element
- Keep only controls UI
- Remove frame receiving WebSocket
- Keep FX command WebSocket

## Expected Results

- Latency: 130ms → <20ms (8× improvement)
- CPU: 15-25% → 3-5% (5× improvement)  
- GPU: More efficient (no wasted transfers)
- Code: Simpler (remove 1000+ lines)
- FPS: 30fps → 60fps capable

## Challenges

1. **Window Sizing**: Surface must match window size
2. **Resizing**: Need to handle window resize events
3. **UI Overlay**: Need transparent Tauri window or CSS positioning
4. **Testing**: Can't inspect frames in DevTools anymore

## Alternative: Hybrid Approach

If direct surface is too complex:
- Keep WebSocket for development/debugging
- Add feature flag to switch between modes
- Production uses surface, dev uses WebSocket

