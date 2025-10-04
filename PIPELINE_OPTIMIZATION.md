# GStreamer Pipeline Optimization - Low-Latency Camera + Overlay Mixing

## Problem
Camera lags and overlays start slow/laggy when mixing live 60/30fps camera with mixed-framerate media files using chroma key.

## Root Causes
1. **Framerate mismatch**: File overlays played at native FPS (not normalized to camera rate)
2. **Queue stalls**: Non-leaky queues allowed overlays to block the camera
3. **No compositor latency tuning**: Default compositor waits indefinitely for all pads
4. **Wrong sync settings**: Files not synced to pipeline clock (`identity sync=false`)
5. **Late scaling**: Overlays scaled after chroma key (wasted work)
6. **Loose queue budgets**: Large buffers cause latency

## Applied Optimizations

### 1. Camera Pipeline (Main/Base)
**Before:**
```
mfvideosrc → videoconvert → videoscale → video/x-raw,BGRA → compositor.sink_0
compositor → videoconvert → tee → queue → appsink(sync=false)
```

**After:**
```
mfvideosrc → videoconvert → videoscale → video/x-raw,framerate=60/1,BGRA → compositor.sink_0
compositor(latency=20ms, ignore-inactive-pads=true) → videoconvert → tee 
  → queue(max-size-time=20ms) → appsink(sync=true, max-lateness=40ms)
```

**Changes:**
- ✅ Explicit camera framerate in caps: `framerate=60/1` (or 30/1 based on user selection)
- ✅ Compositor latency: `20000000` (20ms, nanoseconds)
- ✅ Compositor `ignore-inactive-pads=true` (don't wait for inactive overlays)
- ✅ Appsink: `sync=true` + `max-lateness=40000000` (40ms) → drops late frames instead of stalling
- ✅ Tightened queue budgets: `max-size-buffers=0 max-size-bytes=0 max-size-time=20000000` (20ms)

### 2. Overlay Pipeline (FX/File Branch)
**Before:**
```
filesrc → decodebin → videoconvert → videoscale 
  → [alpha?] → identity(sync=false) → queue(2 buffers, 100ms) → caps(BGRA) → compositor.sink_1
```

**After:**
```
filesrc → decodebin → videoconvert → videoscale → caps(WxH@FPS) → videorate(drop-only) 
  → [alpha?] → identity(sync=true) → queue(leaky, max=1 buffer) → caps(BGRA) → compositor.sink_1
```

**Changes:**
- ✅ **Early scaling** to output size (e.g., 1280x720) BEFORE chroma key
- ✅ **Framerate normalization** via `videorate` (drop-only, skip-to-first) to match camera FPS
- ✅ **Caps enforcement** after scale: `video/x-raw,width=1280,height=720,framerate=60/1`
- ✅ **identity sync=true**: Syncs file to pipeline clock (prevents drift)
- ✅ **Leaky queue** RIGHT BEFORE compositor:
  - `max-size-buffers=1` (only keep latest frame)
  - `max-size-time=0, max-size-bytes=0` (no limits)
  - `leaky=downstream` (drop old frames if full)

### 3. Pipeline Flow Diagram
```
CAMERA (LIVE - provides clock):
  mfvideosrc → videoconvert → videoscale → caps(60fps, BGRA) → compositor.sink_0 ✅

OVERLAY (FILE - synced to clock):
  filesrc → decodebin → videoconvert → videoscale → caps(1280x720@60fps) → videorate(drop-only)
    → [chromakey?] → identity(sync=true) → queue(leaky, 1 buffer) → caps(BGRA) → compositor.sink_1 ✅

COMPOSITOR (low-latency):
  compositor(latency=20ms, ignore-inactive-pads=true) → videoconvert → tee
    → queue(20ms) → appsink(sync=true, max-lateness=40ms) → WebSocket preview ✅
    → queue(20ms) → [virtual camera / NDI output] ✅
```

## Rust Implementation (gstreamer-rs)

### Compositor Properties
```rust
// Main pipeline string (Rust format! macro)
format!(
    "compositor name=comp \
       latency=20000000 \
       ignore-inactive-pads=true \
       sink_0::zorder=0 sink_0::alpha={} \
       sink_1::zorder=1 sink_1::alpha={} ! \
     videoconvert ! \
     video/x-raw,format=BGRx,width={},height={} ! \
     tee name=t \
     t. ! queue max-size-buffers=0 max-size-bytes=0 max-size-time=20000000 ! \
       jpegenc quality=90 ! \
       appsink name=preview emit-signals=true sync=true max-lateness=40000000 max-buffers=2 drop=true \
     ...",
    camera_opacity, overlay_opacity, width, height
)
```

### FX Overlay Elements (Builder Pattern)
```rust
use gstreamer::ElementFactory;

// Early scaling to output size
let videoscale = ElementFactory::make("videoscale")
    .name("fxscale")
    .build()?;

// Normalize framerate to camera FPS BEFORE chroma key
let videorate = ElementFactory::make("videorate")
    .name("fxrate")
    .property("drop-only", true)       // Only drop, never duplicate
    .property("skip-to-first", true)   // Start immediately
    .build()?;

// Caps: Match output dimensions and camera framerate
let caps_after_scale = gst::Caps::builder("video/x-raw")
    .field("width", pipeline_width as i32)
    .field("height", pipeline_height as i32)
    .field("framerate", gst::Fraction::new(pipeline_fps as i32, 1))
    .build();

let capsfilter_scale = ElementFactory::make("capsfilter")
    .name("fxcaps_scale")
    .property("caps", &caps_after_scale)
    .build()?;

// Sync file to pipeline clock
let identity = ElementFactory::make("identity")
    .name("fxidentity")
    .property("sync", true)  // CRITICAL: Lock to pipeline clock
    .build()?;

// Leaky queue RIGHT BEFORE compositor
let queue = ElementFactory::make("queue")
    .name("fxqueue")
    .property("max-size-buffers", 1u32)    // Keep only 1 frame
    .property("max-size-time", 0u64)       // No time limit
    .property("max-size-bytes", 0u32)      // No byte limit
    .property_from_str("leaky", "downstream")  // Drop old frames
    .build()?;

// Final caps for BGRA format (after chroma key)
let caps_final = gst::Caps::builder("video/x-raw")
    .field("format", "BGRA")
    .build();

let capsfilter_final = ElementFactory::make("capsfilter")
    .name("fxcaps_final")
    .property("caps", &caps_final)
    .build()?;
```

### Element Linking (with chroma key)
```rust
// Chain: decode → scale early → normalize FPS → chroma key → sync → leaky queue → compositor
if has_alpha {
    let alpha_elem = chroma_element.as_ref().unwrap();
    gst::Element::link_many(&[
        &videoconvert, &videoscale, &capsfilter_scale, &videorate,
        alpha_elem, &identity, &queue, &capsfilter_final
    ])?;
} else {
    gst::Element::link_many(&[
        &videoconvert, &videoscale, &capsfilter_scale, &videorate,
        &identity, &queue, &capsfilter_final
    ])?;
}
```

## Hardware Decode (Future Enhancement)
For Intel/NVIDIA systems, consider hardware-accelerated decode:

### Intel VA-API (Windows/Linux)
```rust
// Replace: filesrc → decodebin
// With:    filesrc → h264parse → vaapih264dec

let h264parse = ElementFactory::make("h264parse")
    .name("fxparse")
    .build()?;

let vaapidec = ElementFactory::make("vaapih264dec")
    .name("fxdecode")
    .property("low-latency", true)
    .build()?;

// Link: filesrc → h264parse → vaapih264dec → videoscale → ...
```

### NVIDIA NVDEC
```rust
let nvdec = ElementFactory::make("nvh264dec")
    .name("fxdecode")
    .build()?;

// Link: filesrc → h264parse → nvh264dec → videoscale → ...
```

### GL Path (GPU chroma key)
For maximum performance, use OpenGL path:
```
glupload → glvideomixer → glshader(chroma key) → gldownload
```

## Expected Results
✅ **Smooth camera preview** - No lag or stuttering from live camera  
✅ **Instant overlay playback** - Files start immediately at camera FPS  
✅ **Perfect sync** - Overlays locked to camera clock  
✅ **Low latency** - 20-40ms compositor/sink latency  
✅ **No stalls** - Leaky queues drop old overlay frames instead of blocking camera  
✅ **Reduced CPU** - Early scaling reduces chroma key workload  

## Testing
1. Start composite pipeline with 60fps camera
2. Play 30fps overlay with chroma key → should see smooth 60fps camera + overlay normalized to 60fps
3. Play 120fps overlay → should downsample to 60fps (drops frames)
4. Monitor console logs for:
   - `[Composite FX] 📐 Early scaling to 1280x720 @ 60fps`
   - `[Composite FX] 🎬 Normalized to 60fps, synced to pipeline clock`
   - `[Composite FX] ✅ Chain: videoconvert → scale → caps → rate → CHROMA → identity(sync) → queue(leaky) → caps`

## References
- GStreamer compositor: https://gstreamer.freedesktop.org/documentation/compositor/
- videorate element: https://gstreamer.freedesktop.org/documentation/videorate/
- Low-latency mixing best practices: https://gstreamer.freedesktop.org/documentation/application-development/advanced/pipeline-manipulation.html

