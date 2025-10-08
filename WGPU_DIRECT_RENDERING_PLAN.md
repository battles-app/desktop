# WGPU Direct Rendering Implementation Plan

## Current Problem

Using WebSocket for frame delivery:
- **Latency:** 50-100ms due to JPEG encoding/decoding + network stack
- **CPU overhead:** JPEG encoding at 30 FPS
- **Memory copies:** Multiple copies through WebSocket → Frontend → Canvas
- **Not scalable:** Won't work well at 60 FPS or higher resolutions

## Proposed Solution

### Architecture: Direct WGPU Rendering

```
┌─────────────────────────────────────────────────────────────┐
│ Camera (Elgato Cam Link)                                    │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ GStreamer Pipeline (Rust)                                   │
│ - mfvideosrc (native format, e.g., NV12/YUY2)             │
│ - videoconvert → RGBA (GPU if available)                   │
│ - appsink (raw RGBA bytes)                                 │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ WGPU Renderer (Rust)                                        │
│ - Upload RGBA to GPU texture                                │
│ - Apply chroma key shader (if FX active)                    │
│ - Composite with FX layer                                   │
└────────────────────┬────────────────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────────────────┐
│ Tauri Window (Native)                                       │
│ - WGPU surface rendering                                    │
│ - OR: egui integration                                      │
│ - OR: raw-window-handle for custom rendering               │
└─────────────────────────────────────────────────────────────┘
```

## Implementation Steps

### Step 1: Modify GStreamer Pipeline
Remove JPEG encoding, output raw RGBA:

```rust
// Current (BAD - encoding to JPEG)
"... ! jpegenc quality=85 ! appsink ..."

// New (GOOD - raw RGBA)
"... ! videoconvert ! video/x-raw,format=RGBA ! appsink ..."
```

### Step 2: Use Existing WgpuChromaRenderer
We already have `WgpuChromaRenderer` in `gstreamer_composite.rs`!

```rust
// In the appsink callback:
let rgba_data = map.as_slice(); // Raw RGBA bytes

// Upload to WGPU texture
wgpu_renderer.update_texture_from_rgba(rgba_data, width, height)?;

// Render (with chroma key if needed)
let rendered_frame = wgpu_renderer.render_frame()?;
```

### Step 3: Choose Rendering Method

#### Option A: Embedded Canvas in Tauri Window (Easiest)
Keep the Vue frontend but use more efficient data transfer:

**Use SharedArrayBuffer:**
```rust
// Rust: Write to shared memory
#[command]
fn get_frame_buffer() -> Vec<u8> {
    // Return raw RGBA buffer
    rendered_frame
}
```

```javascript
// Frontend: Direct putImageData (no decode)
const buffer = await invoke('get_frame_buffer');
const imageData = new ImageData(
    new Uint8ClampedArray(buffer),
    width,
    height
);
ctx.putImageData(imageData, 0, 0);
```

**Pros:** 
- Minimal code changes
- Still use Vue UI
- ~10ms latency (vs 50-100ms current)

**Cons:**
- Still copies data (but no encoding/decoding)
- Not as fast as pure WGPU

#### Option B: Native WGPU Window (Fastest)
Create a separate native window for video preview:

```rust
use wgpu::Surface;
use raw_window_handle::HasRawWindowHandle;

struct VideoPreviewWindow {
    surface: Surface,
    device: Device,
    queue: Queue,
    // ... render pipeline
}

impl VideoPreviewWindow {
    fn render(&mut self, texture: &Texture) {
        // Direct GPU rendering to window surface
        let frame = self.surface.get_current_texture()?;
        let view = frame.texture.create_view(&Default::default());
        
        // Render texture to surface
        // ... render pass
        
        frame.present();
    }
}
```

**Pros:**
- Zero-copy GPU pipeline
- <5ms latency
- Maximum performance
- Can render at 120+ FPS

**Cons:**
- More complex
- Separate window (or need to embed in Tauri)
- UI controls separate from video

#### Option C: egui Integration (Best Balance)
Use egui (immediate mode GUI) for both UI and video:

```rust
use eframe::egui;

struct BattlesApp {
    wgpu_renderer: WgpuChromaRenderer,
    video_texture: egui::TextureId,
}

impl eframe::App for BattlesApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            // Render video
            ui.image(self.video_texture, [1280.0, 720.0]);
            
            // UI controls
            if ui.button("Change Camera").clicked() {
                // ...
            }
        });
    }
}
```

**Pros:**
- Single window
- Immediate mode (easy to use)
- WGPU accelerated
- UI + video integrated
- ~5-10ms latency

**Cons:**
- Replace Vue UI with Rust/egui
- Different UI paradigm

## Recommended Approach

**Start with Option A (SharedArrayBuffer) as MVP:**

1. Remove JPEG encoding from pipeline
2. Send raw RGBA via Tauri commands (not WebSocket)
3. Use `putImageData()` on frontend
4. Measure latency improvement

**Then evolve to Option C (egui) for production:**

1. Move UI to egui
2. Direct WGPU texture rendering
3. Full GPU pipeline
4. Sub-10ms latency

## Performance Comparison

| Method | Latency | CPU Usage | GPU Usage | Implementation |
|--------|---------|-----------|-----------|----------------|
| Current (WebSocket+JPEG) | 50-100ms | High (encoding) | Low | ❌ Done |
| SharedArrayBuffer | ~10ms | Medium | Low | ⚡ Quick win |
| WGPU Direct | <5ms | Very Low | Medium | 🎯 Best |
| egui Integration | <10ms | Low | Medium | 🏆 Recommended |

## Code Changes Required

### Minimal (Option A):

1. **gstreamer_composite.rs** - Remove jpegenc, output RGBA
2. **main.rs** - Add command to get frame buffer
3. **CompositeCanvas.vue** - Use `putImageData()` instead of ImageBitmap

**Time:** ~2 hours
**Improvement:** 5-10x lower latency

### Full (Option C):

1. Create new `src/ui/` module with egui
2. Integrate `WgpuChromaRenderer`
3. Replace Vue with Rust UI
4. Direct texture rendering

**Time:** ~2-3 days
**Improvement:** 10-20x lower latency, much lower CPU

## Next Steps

1. **Measure current latency** (add timestamps)
2. **Implement Option A** (quick win with minimal changes)
3. **Benchmark improvement**
4. **Decide on Option C** (if worth the refactor)

## References

- Tauri WGPU example: https://github.com/tauri-apps/tauri/discussions/4860
- egui + WGPU: https://github.com/emilk/egui/tree/master/crates/egui-wgpu
- Raw window handle: https://github.com/rust-windowing/raw-window-handle

## Questions to Resolve

1. Do you want to keep Vue UI or switch to egui?
2. What's your target latency? (10ms? 5ms?)
3. Are you planning to add more GPU effects (chroma key, overlays)?
4. Do you need the UI in the same window as video?

Let me know which approach you prefer and I'll implement it!

