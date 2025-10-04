# GStreamer Pipeline - Visual Diagram

## 📊 OPTIMIZED PIPELINE STRING (Camera + Overlay)

### Camera Branch (Live - 60fps or 30fps)
```
mfvideosrc device-index=0 
  ↓
videoflip method=clockwise  (if rotation needed)
  ↓
videoconvert
  ↓
videoscale
  ↓
video/x-raw,width=1280,height=720,framerate=60/1,format=BGRA
  ↓
compositor.sink_0 (zorder=0, alpha=1.0)
```

### Overlay Branch (File - Mixed FPS → Normalized to 60fps)
```
filesrc location=/path/to/fx.mp4
  ↓
decodebin
  ↓ (pad-added dynamic link)
videoconvert
  ↓
videoscale ────────────────┐
  ↓                         │ EARLY SCALING (reduces work)
video/x-raw,width=1280,height=720,framerate=60/1
  ↓
videorate drop-only=true skip-to-first=true ────┐
  ↓                                               │ NORMALIZE FPS
video/x-raw,framerate=60/1                       │ (30fps → 60fps drops duplicates)
  ↓                                               │ (120fps → 60fps drops frames)
[OPTIONAL] alpha method=custom target-r=0 target-g=255 target-b=0 ────┐
  ↓                                                                      │ CHROMA KEY
video/x-raw,format=BGRA (with transparency)                            │
  ↓
identity sync=true ────────────────┐
  ↓                                 │ SYNC TO PIPELINE CLOCK
queue leaky=downstream max-size-buffers=1 max-size-time=0 ────┐
  ↓                                                              │ PREVENT STALLS
video/x-raw,format=BGRA                                         │
  ↓
compositor.sink_1 (zorder=1, alpha=1.0)
```

### Compositor + Output
```
compositor latency=20000000 ignore-inactive-pads=true ────┐
  ↓                                                         │ LOW LATENCY (20ms)
videoconvert                                               │ DON'T WAIT FOR INACTIVE PADS
  ↓
video/x-raw,format=BGRx,width=1280,height=720
  ↓
tee name=t
  ├─→ queue max-size-time=20000000 → jpegenc quality=90 
  │     → appsink sync=true max-lateness=40000000 → WebSocket
  │
  └─→ queue max-size-time=20000000 → [virtual camera / NDI output]
```

## 🔧 MINIMAL RUST SNIPPET

### Complete FX Overlay Pipeline (Dynamic Bin)
```rust
use gstreamer::prelude::*;
use gstreamer::{ElementFactory, Element};

// Get pipeline parameters
let pipeline_fps = 60u32;      // From frontend selection
let pipeline_width = 1280u32;
let pipeline_height = 720u32;

// 1. Source + Decoder
let filesrc = ElementFactory::make("filesrc")
    .name("fxfilesrc")
    .property("location", "/path/to/fx.mp4")
    .build()?;

let decodebin = ElementFactory::make("decodebin")
    .name("fxdecode")
    .property("force-sw-decoders", true)  // Or false for hw decode
    .build()?;

// 2. Early Scaling (reduces chroma key work)
let videoconvert = ElementFactory::make("videoconvert")
    .name("fxconvert")
    .build()?;

let videoscale = ElementFactory::make("videoscale")
    .name("fxscale")
    .build()?;

// 3. Normalize Framerate to Camera FPS BEFORE chroma key
let videorate = ElementFactory::make("videorate")
    .name("fxrate")
    .property("drop-only", true)       // Only drop, never duplicate
    .property("skip-to-first", true)   // Start immediately
    .build()?;

let caps_after_scale = gst::Caps::builder("video/x-raw")
    .field("width", pipeline_width as i32)
    .field("height", pipeline_height as i32)
    .field("framerate", gst::Fraction::new(pipeline_fps as i32, 1))
    .build();

let capsfilter_scale = ElementFactory::make("capsfilter")
    .name("fxcaps_scale")
    .property("caps", &caps_after_scale)
    .build()?;

// 4. Chroma Key (Optional)
let alpha = ElementFactory::make("alpha")
    .name("fxalpha")
    .property_from_str("method", "custom")
    .property("target-r", 0u32)        // Green screen
    .property("target-g", 255u32)
    .property("target-b", 0u32)
    .property("angle", 30.0f32)        // tolerance * 180
    .build()?;

// 5. Sync to Pipeline Clock
let identity = ElementFactory::make("identity")
    .name("fxidentity")
    .property("sync", true)  // CRITICAL: Lock to pipeline clock
    .build()?;

// 6. Leaky Queue RIGHT BEFORE compositor
let queue = ElementFactory::make("queue")
    .name("fxqueue")
    .property("max-size-buffers", 1u32)        // Keep only 1 frame
    .property("max-size-time", 0u64)           // No time limit
    .property("max-size-bytes", 0u32)          // No byte limit
    .property_from_str("leaky", "downstream")  // Drop old frames
    .build()?;

// 7. Final Caps (BGRA format)
let caps_final = gst::Caps::builder("video/x-raw")
    .field("format", "BGRA")
    .build();

let capsfilter_final = ElementFactory::make("capsfilter")
    .name("fxcaps_final")
    .property("caps", &caps_final)
    .build()?;

// Add elements to bin
let fx_bin = gst::Bin::builder().name("fxbin").build();
fx_bin.add_many(&[
    &filesrc, &decodebin, &videoconvert,
    &videoscale, &capsfilter_scale, &videorate,
    &alpha, &identity, &queue, &capsfilter_final
])?;

// Link static elements
gst::Element::link_many(&[&filesrc, &decodebin])?;

// Link processing chain (decodebin connects via pad-added)
gst::Element::link_many(&[
    &videoconvert, &videoscale, &capsfilter_scale, &videorate,
    &alpha, &identity, &queue, &capsfilter_final
])?;

// Connect decodebin dynamically
let videoconvert_clone = videoconvert.clone();
decodebin.connect_pad_added(move |_dbin, src_pad| {
    let caps = src_pad.current_caps()?;
    let structure = caps.structure(0)?;
    
    if structure.name().starts_with("video/") {
        let sink_pad = videoconvert_clone.static_pad("sink")?;
        if !sink_pad.is_linked() {
            src_pad.link(&sink_pad)?;
        }
    }
});

// Create ghost pad and add to pipeline
let final_src_pad = capsfilter_final.static_pad("src")?;
let ghost_pad = gst::GhostPad::with_target(&final_src_pad)?;
ghost_pad.set_active(true)?;
fx_bin.add_pad(&ghost_pad)?;

pipeline.add(&fx_bin)?;

// Link to compositor sink_1
let compositor = pipeline.by_name("comp")?;
let comp_sink_pad = compositor.request_pad_simple("sink_1")?;
comp_sink_pad.set_property("zorder", 1u32);
comp_sink_pad.set_property("alpha", 1.0f64);
ghost_pad.link(&comp_sink_pad)?;

// Set FX bin to PLAYING
fx_bin.set_state(gst::State::Playing)?;
```

### Main Compositor Properties (Pipeline String)
```rust
// Windows pipeline with all optimizations
let pipeline_str = format!(
    "compositor name=comp \
       latency=20000000 \
       ignore-inactive-pads=true \
       sink_0::zorder=0 sink_0::alpha=1.0 \
       sink_1::zorder=1 sink_1::alpha=1.0 ! \
     videoconvert ! \
     video/x-raw,format=BGRx,width={},height={} ! \
     tee name=t \
     t. ! queue max-size-buffers=0 max-size-bytes=0 max-size-time=20000000 ! \
       jpegenc quality=90 ! \
       appsink name=preview emit-signals=true sync=true max-lateness=40000000 max-buffers=2 drop=true \
     t. ! queue max-size-buffers=0 max-size-bytes=0 max-size-time=20000000 ! fakesink \
     mfvideosrc device-index={} ! \
     videoconvert ! \
     videoscale ! \
     video/x-raw,width={},height={},framerate={}/1,format=BGRA ! \
     comp.sink_0",
    width, height, device_index, width, height, fps
);
```

## 🎯 KEY OPTIMIZATIONS SUMMARY

| Component | Property | Value | Purpose |
|-----------|----------|-------|---------|
| **Compositor** | `latency` | `20000000` (20ms) | Reduce mixer wait time |
| **Compositor** | `ignore-inactive-pads` | `true` | Don't wait for inactive overlays |
| **Overlay: videorate** | `drop-only` | `true` | Normalize FPS to camera rate |
| **Overlay: videorate** | `skip-to-first` | `true` | Start immediately, no buffering |
| **Overlay: caps** | `framerate` | `60/1` (or 30/1) | Lock to camera FPS BEFORE keying |
| **Overlay: identity** | `sync` | `true` | Sync file to pipeline clock |
| **Overlay: queue** | `leaky` | `downstream` | Prevent stalling camera |
| **Overlay: queue** | `max-size-buffers` | `1` | Keep only latest frame |
| **Overlay: queue** | `max-size-time` | `0` | No time budget (unlimited) |
| **Appsink** | `sync` | `true` | Enable clock sync |
| **Appsink** | `max-lateness` | `40000000` (40ms) | Drop late frames, don't stall |
| **All queues** | `max-size-time` | `20000000` (20ms) | Tighten budgets |

## 🚀 HARDWARE DECODE (Optional Enhancement)

### Intel VA-API
```rust
// Replace decodebin with:
let h264parse = ElementFactory::make("h264parse").build()?;
let vaapidec = ElementFactory::make("vaapih264dec")
    .property("low-latency", true)
    .build()?;

// Link: filesrc → h264parse → vaapih264dec → videoconvert → ...
```

### NVIDIA NVDEC
```rust
let h264parse = ElementFactory::make("h264parse").build()?;
let nvdec = ElementFactory::make("nvh264dec").build()?;

// Link: filesrc → h264parse → nvh264dec → videoconvert → ...
```

## 📝 PIPELINE FLOW ORDER

```
┌─────────────────────────────────────────────────────────────┐
│ CAMERA (LIVE - provides clock)                             │
│ mfvideosrc → convert → scale → caps(60fps) → compositor.0  │
└─────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────┐
│ OVERLAY (FILE - synced to clock)                           │
│ filesrc → decode → convert → scale(1280x720) → rate(60fps) │
│   → [chroma?] → identity(sync) → queue(leaky) → comp.1     │
└─────────────────────────────────────────────────────────────┘
                              ↓
┌─────────────────────────────────────────────────────────────┐
│ COMPOSITOR (low-latency=20ms, ignore-inactive)             │
│ comp → convert → tee → queue(20ms) → appsink(sync, 40ms)   │
└─────────────────────────────────────────────────────────────┘
```

## ✅ EXPECTED RESULTS

- ✅ Smooth 60fps/30fps camera with no lag
- ✅ Overlays normalized to camera FPS (30fps file → 60fps camera = smooth)
- ✅ Overlays start instantly (no buffering delay)
- ✅ Perfect sync (files locked to camera clock)
- ✅ Camera never stalls (leaky queues drop overlay frames if needed)
- ✅ Low latency: 20-40ms total (compositor + sink)
- ✅ Reduced CPU: Early scaling reduces chroma key workload

## 🔍 DEBUGGING

Enable GStreamer debug logs:
```powershell
$env:GST_DEBUG="3,compositor:5,videorate:5,queue:4"
```

Look for:
- `compositor: Latency: 20000000`
- `videorate: Normalizing from X fps to 60 fps`
- `queue: Dropping buffer (leaky downstream)`
- `appsink: Dropping late buffer (max-lateness=40ms)`

