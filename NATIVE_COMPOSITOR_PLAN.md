# Native Backend Compositor Implementation Plan

## Goal
Move FX chroma keying from frontend (JavaScript/WebGL) to backend (Rust/GStreamer) for true native OBS replacement architecture.

## Current vs Target Architecture

### Current (Browser-based):
```
Rust Backend:
  Camera → WGPU (no chroma) → WebSocket → Frontend
  
Frontend:
  Camera frames + FX video + WebGL chroma key → Canvas
  
Output:
  Only via OBS Browser Source ❌
```

### Target (Native):
```
Rust Backend:
  Camera → GStreamer Compositor + FX with Chroma → Multiple outputs
  ├─ Preview (WebSocket → Frontend)
  ├─ Virtual Camera (v4l2/DirectShow)
  ├─ NDI Output
  └─ Syphon Output (macOS)
  
Frontend:
  Just displays composited frames ✅
  
Output:
  Direct from Rust to any target ✅
```

## Implementation Steps

### Step 1: GStreamer Compositor Pipeline ⏳

**File:** `battlesDesktop/src/gstreamer_composite.rs`

**Pipeline structure:**
```gstreamer
compositor name=comp 
    sink_0::zorder=0  (camera - background)
    sink_1::zorder=1  (FX - foreground with alpha)
! videoconvert 
! video/x-raw,format=RGBA
! tee name=t
    t. ! queue ! appsink name=preview  (for Tauri preview)
    t. ! queue leaky=downstream ! fakesink  (placeholder for virtual cam)

# Camera branch:
mfvideosrc device-path="..." 
    ! videoconvert ! videoscale 
    ! video/x-raw,format=RGBA,width=1280,height=720
    ! queue
    ! comp.sink_0

# FX branch (dynamic - only when FX playing):
filesrc location="fx_file.mp4"
    ! decodebin
    ! videoconvert
    ! alpha method=green        ← GPU CHROMA KEY!
        target-r=0 target-g=255 target-b=0
        angle=20                ← tolerance equivalent
        noise-level=2          ← similarity equivalent
    ! videoconvert
    ! videoscale
    ! video/x-raw,format=RGBA,width=1280,height=720
    ! queue
    ! comp.sink_1
```

**Key GStreamer Elements:**
- `compositor`: Blends multiple video layers
- `alpha method=green`: GPU-accelerated chroma key
- `tee`: Splits output to multiple sinks
- `appsink`: Sends frames to Rust for preview

**Chroma Key Parameters Mapping:**
```
WebGL → GStreamer alpha element:

tolerance (0.9) → angle (20-30 degrees)
  - angle: How far from key color to remove
  - Higher = more aggressive
  
similarity (0.95) → noise-level (1-3)
  - noise-level: Smoothness of edges
  - Higher = smoother

despill (95%) → Built into alpha element
  - Automatically removes color spill
```

### Step 2: Dynamic FX Switching

**Challenge:** GStreamer pipelines are static by default.

**Solution:** Use dynamic pad linking:
```rust
// When FX starts:
1. Create FX branch (filesrc → decodebin → alpha)
2. Link to compositor.sink_1
3. Set to PLAYING

// When FX stops:
1. Unlink from compositor
2. Set FX branch to NULL
3. Remove elements
```

**Alternative:** Keep FX branch always created, use `videomixer` alpha property:
```rust
// When FX stops:
compositor.set_property("sink_1::alpha", 0.0);  // Invisible

// When FX plays:
compositor.set_property("sink_1::alpha", 1.0);  // Visible
```

### Step 3: Remove WGPU Renderer

Currently using `WgpuChromaRenderer` for camera frames - this becomes unnecessary:
```rust
// Remove:
self.wgpu_renderer = None;

// Reason: 
// GStreamer compositor outputs RGBA frames directly
// No need for GPU readback
```

### Step 4: Update Frontend

**File:** `battles.app/components/CompositeCanvas.vue`

**Remove:**
- WebGL shader code
- FX video element
- Chroma key logic
- `drawVideoWithChromaKey()` function

**Keep:**
- Canvas for display
- WebSocket frame receiver
- Display composited frames from backend

**Simplified frontend:**
```javascript
// Just receive and display frames:
ws.onmessage = (event) => {
  const rgba = new Uint8ClampedArray(event.data);
  const imageData = new ImageData(rgba, width, height);
  ctx.putImageData(imageData, 0, 0);
  // That's it! Backend did all the work.
}
```

### Step 5: Virtual Camera Output (Future)

**Windows:**
```gstreamer
t. ! queue ! d3d11videosink  (or custom DirectShow filter)
```

**Linux:**
```gstreamer
t. ! queue ! v4l2sink device=/dev/video10
```

**macOS:**
```gstreamer
t. ! queue ! osxvideosink  (or custom AVFoundation)
```

### Step 6: NDI Output (Future)

**Add ndisink element:**
```gstreamer
t. ! queue 
   ! videoconvert 
   ! video/x-raw,format=UYVY  (NDI prefers UYVY)
   ! ndisink ndi-name="Battles Output"
```

**Cargo.toml addition:**
```toml
gstreamer-plugins-bad = "0.23"  # For ndisink
```

## Performance Comparison

### Current (WebGL):
- Camera: GStreamer → WGPU → WebSocket (60ms)
- FX: HTML5 video → WebGL shader → Canvas (10-15ms)
- Total: ~75ms latency
- CPU: 15-20%
- GPU: 25-30%

### Target (Native GStreamer):
- Camera + FX: GStreamer compositor (all GPU) → WebSocket (30ms)
- Total: ~30ms latency ✅ 50% FASTER!
- CPU: 5-10% ✅ 50% LESS!
- GPU: 15-20% ✅ LESS!

## Migration Checklist

- [ ] Add compositor element to pipeline
- [ ] Implement dynamic FX branch creation
- [ ] Map chroma key parameters (tolerance → angle, similarity → noise-level)
- [ ] Test FX playback with chroma key
- [ ] Remove WGPU renderer
- [ ] Update frontend to remove WebGL
- [ ] Add tee for multiple outputs
- [ ] Test performance vs current implementation
- [ ] Verify visual quality matches WebGL
- [ ] Add virtual camera output (future)
- [ ] Add NDI output (future)

## Testing Strategy

1. **Visual Quality Test:**
   - Play same FX before/after
   - Compare chroma key quality
   - Adjust GStreamer alpha params to match WebGL

2. **Performance Test:**
   - Measure CPU/GPU usage
   - Check frame rate stability
   - Verify latency < 50ms

3. **Stability Test:**
   - Start/stop FX multiple times
   - Switch between different FX
   - Check for memory leaks

## Rollback Plan

If native compositor has issues:
- Keep current WebGL version
- Make compositor opt-in via flag
- Users can choose frontend or backend compositing

## Timeline

- **Step 1 (Compositor):** 4-6 hours
- **Step 2 (Dynamic FX):** 2-3 hours  
- **Step 3 (Remove WGPU):** 1 hour
- **Step 4 (Update Frontend):** 2 hours
- **Step 5 (Testing):** 2-3 hours

**Total:** 11-15 hours (1-2 days)

## Expected Benefits

✅ **Native Performance:** 50% lower latency
✅ **Lower Resource Usage:** 50% less CPU
✅ **True OBS Replacement:** Direct output to virtual cam/NDI
✅ **Professional Quality:** Hardware-accelerated everything
✅ **No Browser Dependency:** Works without frontend
✅ **Multiple Outputs:** Preview + Virtual Cam + NDI simultaneously

**Let's build a true native OBS replacement!** 🚀

