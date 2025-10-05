# WGPU-based Compositor for Battles.app

This implementation replaces the GStreamer compositor with a custom Rust compositor built using wgpu, while keeping GStreamer for media I/O.

## Architecture

The architecture follows a modular design:

### Inputs
- GStreamer pipelines for each source (camera, media file, screen)
- Decode to raw RGBA frames via appsink
- Set is-live=true for live sources and use sync=false to prevent blocking

### Compositor (Rust/wgpu)
- WgpuCompositor struct that:
  - Initializes wgpu with an offscreen surface
  - Maintains a list of Layer structs (each with texture, transform, opacity, rotation)
  - Renders all layers each frame using instanced quads and a single render pass
  - Runs a fixed frame clock (e.g., 60 fps) and composites based on each frame's PTS
  - Outputs the rendered frame as RGBA bytes

### Output
- Send the composited frames into a GStreamer appsrc element for encoding and RTMP/WebRTC output
- Set appsrc caps like video/x-raw,format=RGBA,width=1920,height=1080,framerate=60/1 and is-live=true format=time
- Assign correct PTS and duration for each frame to keep sync

### Audio
- GStreamer handles audio input/mixing normally and serves as the master clock
- Query GStreamer's pipeline clock time to drive video frame timestamps

## Folder Structure

```
src/
├── main.rs
├── wgpu_composite.rs
├── compositor/
│   ├── mod.rs
│   ├── layer.rs
│   └── wgpu_compositor.rs
├── gst/
│   ├── mod.rs
│   ├── input.rs
│   ├── output.rs
│   └── utils.rs
└── clock.rs
```

## Key Components

1. **Layer (compositor/layer.rs)**
   - Represents a layer in the compositor
   - Handles texture management and transformation

2. **WgpuCompositor (compositor/wgpu_compositor.rs)**
   - Core rendering engine
   - Manages layers and performs GPU-accelerated compositing

3. **GstInput (gst/input.rs)**
   - Handles input sources (camera, file, screen)
   - Decodes media to raw RGBA frames

4. **GstOutput (gst/output.rs)**
   - Handles output streaming (RTMP, virtual camera)
   - Encodes composited frames

5. **SyncClock and FrameClock (clock.rs)**
   - Synchronizes with GStreamer's pipeline clock
   - Ensures consistent frame timing

6. **WgpuComposite (wgpu_composite.rs)**
   - High-level compositor API
   - Connects inputs, compositor, and outputs

## Benefits Over GStreamer Compositor

1. **Performance**: GPU-accelerated compositing with wgpu is more efficient than GStreamer's CPU-based compositing.
2. **Flexibility**: Custom layer management allows for more complex effects and transformations.
3. **Control**: Fine-grained control over rendering and timing.
4. **Modularity**: Clear separation between I/O (GStreamer) and compositing (wgpu).

## Usage

The API remains compatible with the previous GStreamer implementation, so no changes are needed in the frontend code.

```rust
// Initialize the compositor
let composite = WgpuComposite::new().await?;

// Start the compositor with a camera
composite.start("0", 1280, 720, 30, 0).await?;

// Play an effect
composite.play_fx_from_file("path/to/effect.mp4", "#00ff00", 0.3, 0.5, true)?;

// Set output format
composite.set_output_format("rtmp", 1280, 720)?;

// Stop the compositor
composite.stop()?;
```

## Dependencies

- wgpu: GPU rendering
- glam: Math operations
- gstreamer: Media I/O
- tokio: Async runtime
- bytemuck: Memory manipulation
- anyhow/thiserror: Error handling
