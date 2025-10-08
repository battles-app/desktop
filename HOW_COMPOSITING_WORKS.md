# How FX Compositing Works - EXPLAINED

## The Confusion

You saw this in the HTML:
```html
<canvas>...</canvas>
<video src="money-gun-001.mp4"></video>
```

And thought "there are 2 elements showing!" but actually...

## What You SEE vs What Exists

### What EXISTS in HTML:
```
1. <canvas> - VISIBLE ✅
2. <video> - HIDDEN (display: none) ❌ Never shows on screen!
```

### What You SEE on screen:
```
Just the <canvas> with camera + FX composited together!
```

## How It Works (Step by Step)

### Step 1: Video Element (Hidden, Just Loads Data)
```javascript
<video 
  ref="fxVideo"
  style="display: none !important"  ← NEVER VISIBLE!
  src="money-gun-001.mp4"
/>
```

This video:
- ❌ **NOT shown on screen**
- ✅ **Just loads the video file**
- ✅ **Acts as a data source**
- It's like loading an image - the `<img>` loads the file, but you can draw it elsewhere

### Step 2: Compositing Function (Runs Every Frame)
```javascript
const compositeFrame = (cameraFrame: ImageData) => {
  const ctx = canvas.getContext('2d')
  
  // Draw camera (background)
  ctx.putImageData(cameraFrame, 0, 0)  ← Camera pixels
  
  // Draw FX on top (foreground) - reads from hidden video element
  if (fxPlaying) {
    ctx.drawImage(fxVideo, 0, 0)  ← FX pixels (copied from hidden video)
  }
}
```

**The `drawImage()` function:**
- Reads pixels from the hidden `<video>` element
- Draws those pixels ONTO the `<canvas>`
- The video itself is never shown - only its pixels are copied

### Step 3: Result
```
┌────────────────────────────┐
│ <canvas> (WHAT YOU SEE)    │
│ ┌────────────────────────┐ │
│ │ Camera pixels          │ │ ← Background
│ │ + FX video pixels      │ │ ← Foreground (copied from hidden video)
│ └────────────────────────┘ │
└────────────────────────────┘
```

## Why Use a Hidden Video?

**Q:** Why not just draw the video file directly?

**A:** Because:
1. HTML5 `<video>` element handles:
   - Video decoding (H.264, VP9, etc.)
   - Frame timing & sync
   - Buffering & streaming
   
2. We just need to **read its current frame** with `drawImage()`
3. It's like using `<img>` to load a PNG, then drawing it on canvas - same concept!

## Live Streaming This Canvas

### Option 1: OBS Browser Source
```
1. Add Browser Source to OBS
2. URL: https://local.battles.app:3000/stream/obs-overlay/USERNAME
3. Point at the canvas element
4. OBS captures the canvas (camera + FX composited)
✅ Perfect for streaming!
```

### Option 2: Virtual Camera (Future)
```javascript
// Capture canvas stream
const stream = canvas.captureStream(30) // 30 FPS

// Send to virtual camera
navigator.mediaDevices.getUserMedia({ video: { mandatory: { chromeMediaSource: 'desktop' } } })

✅ Canvas becomes virtual webcam input
```

### Option 3: NDI Output (Future - Rust Backend)
```rust
// In Rust backend, capture canvas frames and send via NDI
let ndi_sender = NDI::create_sender("Battles Output");
ndi_sender.send_video_frame(canvas_pixels);

✅ Broadcast over network to any NDI receiver
```

## The Key Point

**You only see ONE thing: the canvas.**

Everything else (hidden video, temporary canvases for chroma key) are just **processing steps** that happen behind the scenes.

```
Camera → WebSocket → Canvas (visible)
                       ↑
FX Video (hidden) ─────┘ (copied onto canvas)
```

**Result: Single canvas with perfect compositing ready for streaming!** 🎥✨

## Current Status

✅ Camera renders to canvas  
✅ FX video loads (hidden)  
✅ FX composited onto canvas with chroma key  
✅ Single canvas output ready for broadcast  
🔜 Virtual camera integration (next step)  
🔜 NDI output (future)  

**Everything composites into ONE canvas - that's what you stream!**

