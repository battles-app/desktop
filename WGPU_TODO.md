# WGPU Integration TODO

## The Problem

I initialized WgpuChromaRenderer but I'm NOT USING IT! The frames go straight from GStreamer to WebSocket without GPU processing.

## What Needs to Happen

### Current (WRONG):
```
Camera → GStreamer RGBA → 
appsink callback → 
WebSocket → Frontend
```

### What You Want (RIGHT):
```
Camera → GStreamer RGBA → 
appsink callback → 
WGPU texture upload → 
WGPU shader (chroma key) → 
WGPU render to surface → 
Direct display (NO WEBSOCKET!)
```

## The Issue

The `appsink` callback is in a closure that doesn't have access to `self.wgpu_renderer`. I need to:

1. **Pass wgpu_renderer into the callback** via Arc/Mutex
2. **Upload RGBA to GPU texture** using `update_texture_from_rgba()`
3. **Render with shader** using `render_frame()`
4. **Output to surface** - either:
   - Direct to Tauri window
   - Or via egui
   - Or to shared texture

## Solution

### Option 1: Arc<Mutex<WgpuChromaRenderer>> in callback
```rust
let wgpu_renderer = Arc::new(Mutex::new(self.wgpu_renderer.take().unwrap()));
let wgpu_clone = wgpu_renderer.clone();

appsink.set_callbacks(
    AppSinkCallbacks::builder()
        .new_sample(move |appsink| {
            // ... get rgba_data ...
            
            let mut renderer = wgpu_clone.lock().unwrap();
            renderer.update_texture_from_rgba(rgba_data, width, height)?;
            let processed = renderer.render_frame()?;
            
            // Send processed frame
            sender.send(processed);
        })
);
```

### Option 2: Separate rendering thread
```rust
// Main thread: WGPU rendering loop
// Callback thread: Just queues raw frames
```

## Next Steps

1. Refactor to use Arc<Mutex<>> for wgpu_renderer
2. Actually call update_texture_from_rgba() in callback
3. Actually call render_frame() to apply shader
4. Remove WebSocket or send GPU-processed frames only

## The Real Goal

Eventually: NO WEBSOCKET AT ALL
- WGPU renders to egui window
- OR WGPU renders to Tauri webview surface
- Direct GPU pipeline, <5ms latency

