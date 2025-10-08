# FX Compositor Implementation Status

## Current State

The `play_fx_from_file` method in `gstreamer_composite.rs` **does NOT actually composite video FX**. 

Lines 1070-1082 show it just stores the FX file path but doesn't rebuild the pipeline:

```rust
if is_video {
    println!("[Composite] 🎥 Video FX detected - this will be implemented with GStreamer overlay");
    // For now, log that we received the FX command
    // Full GStreamer overlay integration would require rebuilding the pipeline with compositor
    
    self.current_fx_file = Some(file_path.clone());
    self.current_chroma_params = Some((keycolor, tolerance, similarity, use_chroma_key));
    
    println!("[Composite] ✅ Video FX stored (overlay implementation in progress)");
    Ok(())
}
```

## What's Needed

To make FX compositing work properly for broadcasting/virtual camera/NDI, we need to:

### Option 1: GStreamer Compositor (Complex but Professional)

Rebuild the pipeline with a `compositor` element that blends camera + FX video:

```gstreamer
compositor name=comp sink_0::alpha=1.0 sink_0::zorder=0 sink_1::alpha=1.0 sink_1::zorder=1 ! 
videoconvert ! video/x-raw,format=RGBA ! appsink

mfvideosrc ! ... ! comp.sink_0  (camera - background)
filesrc location=fx.mp4 ! decodebin ! alpha ! ... ! comp.sink_1  (FX - foreground)
```

**Pros:**
- GPU-accelerated
- Professional quality
- Built-in chroma key (`alpha` element)
- Works for broadcasting

**Cons:**
- Must rebuild entire pipeline when FX changes
- Complex state management
- Pipeline might drop frames during rebuild

### Option 2: Canvas-Based Compositing (Simpler, Works Now)

Keep camera → canvas as-is, add FX compositing in JavaScript:

```javascript
// Hidden video element for FX
const fxVideo = document.createElement('video')
fxVideo.src = fxFileUrl
fxVideo.play()

// Composite on canvas
function render() {
  ctx.putImageData(cameraImageData, 0, 0)  // Camera background
  ctx.drawImage(fxVideo, 0, 0)              // FX foreground
  // Apply chroma key with WebGL if needed
}
```

**Pros:**
- Much simpler
- No pipeline rebuild
- Can use existing WebGL chroma key code
- Works immediately

**Cons:**
- JavaScript compositing overhead
- May have slight performance impact
- Needs FX files accessible to frontend

## Recommendation

For **fastest implementation** that works for broadcasting:

**Use Canvas-Based Compositing** because:
1. Can be implemented in 50 lines of code
2. Works immediately
3. Canvas output can be captured for virtual camera/NDI
4. Reuses existing WebGL chroma key if needed
5. No complex GStreamer pipeline rebuild

The canvas content is what gets broadcast/captured, so compositing there works perfectly for your use case.

## Implementation Plan (Canvas Compositing)

1. Add hidden `<video>` element to `CompositeCanvas.vue`
2. When FX play event received, set `video.src` and play
3. In render loop, draw camera ImageData first, then drawImage(video)
4. Optional: Apply WebGL chroma key to FX video before drawing
5. Result: Single composited canvas ready for broadcast!

