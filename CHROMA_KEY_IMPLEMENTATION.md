# Chroma Key Implementation for FX Layer

## Overview

This document describes the implementation of chroma key (green screen) removal for FX video playback in the GStreamer composite pipeline. The feature allows dynamic chroma key removal based on parameters passed from the Nuxt.js frontend to the Tauri/Rust backend.

## Architecture

### Data Flow

```
Nuxt.js Frontend (battles.app)
    ↓ (WebSocket fx_play event)
CompositeCanvas.vue
    ↓ (Tauri invoke: play_composite_fx)
Rust Backend (battlesDesktop)
    ↓ (main.rs: play_composite_fx command)
GStreamerComposite
    ↓ (play_fx_from_file)
GStreamer Pipeline with Alpha Element
```

## Frontend Implementation

### 1. Dashboard FX Buttons (DashboardView.vue)

When a user clicks an FX button, the frontend sends chroma key parameters via WebSocket:

**Lines 2667-2683:**
```javascript
const playCommand = {
  type: 'fx_play',
  fileId: fileId,
  fileUrl: fileUrl,
  filename: filename,
  isVideo: isVideo,
  isAudio: isAudio,
  chromaKey: useChromaKey,              // Boolean: enable/disable
  keycolor: item.keycolor || '#00ff00', // Hex color to remove
  tolerance: item.tolerance || 0.30,    // 0.0 - 1.0
  similarity: item.similarity || 0.45,  // 0.0 - 1.0
  smoothness: item.smoothness || 0.08,
  spill: item.spill || 0.15,
  globalFxId: item.id,
  username: tiktokUsername,
  useCache: true
}
```

**Parameters:**
- `chromaKey` (boolean): Enable/disable chroma key removal
- `keycolor` (string): Hex color to remove (e.g., "#00ff00" for green)
- `tolerance` (float): How much color variation to accept (0.0 - 1.0)
- `similarity` (float): How similar colors need to be to key color (0.0 - 1.0)

### 2. CompositeCanvas.vue

The composite canvas receives WebSocket messages and invokes the Tauri backend:

**Lines 423-431:**
```javascript
await invoke('play_composite_fx', {
  fileUrl: data.fileUrl,
  fileData: null,
  filename: data.filename,
  keycolor: data.keycolor || '#00ff00',
  tolerance: data.tolerance || 0.30,
  similarity: data.similarity || 0.45,
  useChromaKey: data.chromaKey || false
})
```

## Backend Implementation

### 3. Tauri Command (main.rs)

The `play_composite_fx` command receives parameters from the frontend:

**Line 1085:**
```rust
composite.play_fx_from_file(
    file_path_str, 
    keycolor, 
    tolerance, 
    similarity, 
    use_chroma_key
)?;
```

### 4. GStreamer Pipeline (gstreamer_composite.rs)

The core implementation adds an `alpha` element to the GStreamer pipeline when chroma key is enabled.

#### Alpha Element Creation

**Lines 712-747:**
```rust
let alpha_element = if use_chroma_key {
    // Parse hex color (e.g., "#00ff00" -> RGB values)
    let color_hex = keycolor.trim_start_matches('#');
    let r = u8::from_str_radix(&color_hex[0..2], 16).unwrap_or(0) as i32;
    let g = u8::from_str_radix(&color_hex[2..4], 16).unwrap_or(255) as i32;
    let b = u8::from_str_radix(&color_hex[4..6], 16).unwrap_or(0) as i32;
    
    // Calculate angle and noise level
    let angle = (tolerance * 180.0) as i32;  // 0-180 degrees
    let noise_level = ((1.0 - similarity) * 100.0) as i32;  // 0-100
    
    // Create alpha element with chroma key method
    let alpha = ElementFactory::make("alpha")
        .name("fxalpha")
        .property("method", "green")  // green method for chroma keying
        .property("target-r", r)      // Target red component (0-255)
        .property("target-g", g)      // Target green component (0-255)
        .property("target-b", b)      // Target blue component (0-255)
        .property("angle", angle)     // Angle tolerance in degrees
        .property("noise-level", noise_level)  // Noise level (0-100)
        .build()?;
    
    Some(alpha)
} else {
    None
};
```

#### Pipeline Linking

The pipeline is dynamically constructed based on whether chroma key is enabled:

**Lines 763-783 (WITH chroma key):**
```
uridecodebin → videorate → rate_filter → identity_sync → videoconvert → alpha → videoscale → capsfilter → compositor
```

**Lines 775-782 (WITHOUT chroma key):**
```
uridecodebin → videorate → rate_filter → identity_sync → videoconvert → videoscale → capsfilter → compositor
```

## GStreamer Alpha Element

The `alpha` element is GStreamer's built-in chroma keying plugin that makes certain colors transparent based on their similarity to a target color.

### Properties

| Property | Type | Range | Description |
|----------|------|-------|-------------|
| `method` | string | - | Set to "green" for chroma keying |
| `target-r` | int | 0-255 | Red component of key color |
| `target-g` | int | 0-255 | Green component of key color |
| `target-b` | int | 0-255 | Blue component of key color |
| `angle` | int | 0-180 | Color tolerance angle in degrees |
| `noise-level` | int | 0-100 | Noise reduction level |

### Parameter Mapping

Frontend → Backend:

| Frontend Parameter | Backend Calculation | GStreamer Property |
|-------------------|---------------------|-------------------|
| `keycolor: "#00ff00"` | Parse hex to RGB | `target-r`, `target-g`, `target-b` |
| `tolerance: 0.30` | `tolerance * 180.0` | `angle: 54` degrees |
| `similarity: 0.45` | `(1.0 - similarity) * 100.0` | `noise-level: 55` |

## Usage Examples

### Example 1: Green Screen (Default)

```javascript
{
  chromaKey: true,
  keycolor: '#00ff00',  // Pure green
  tolerance: 0.30,       // 30% tolerance
  similarity: 0.45       // 45% similarity
}
```

**Result:** Removes bright green backgrounds with moderate tolerance.

### Example 2: Blue Screen

```javascript
{
  chromaKey: true,
  keycolor: '#0000ff',  // Pure blue
  tolerance: 0.40,       // 40% tolerance
  similarity: 0.50       // 50% similarity
}
```

**Result:** Removes blue backgrounds with higher tolerance.

### Example 3: Disabled

```javascript
{
  chromaKey: false
}
```

**Result:** No chroma key processing, video plays as-is.

## Testing

### Manual Testing Steps

1. **Setup:**
   - Start the Nuxt.js frontend: `cd battles.app && bun run dev`
   - Build and run Tauri app: `cd battlesDesktop && bun run tauri dev`

2. **Test Green Screen Removal:**
   - Upload a video with green screen to the FX library
   - Set `chromaKey: true` in the FX settings
   - Configure `keycolor: #00ff00`
   - Adjust `tolerance` and `similarity` as needed
   - Play the FX and verify green is removed

3. **Test Different Colors:**
   - Try different `keycolor` values
   - Test with blue screens (`#0000ff`)
   - Test with other colors

4. **Test Disable:**
   - Set `chromaKey: false`
   - Verify video plays without chroma key processing

### Debug Logging

The implementation includes comprehensive logging:

```
[Composite FX] 🎨 Creating alpha element for chroma key removal
[Composite FX] 🎨 Settings: keycolor=#00ff00, tolerance=0.3, similarity=0.45
[Composite FX] 🎨 Parsed RGB: R=0, G=255, B=0
[Composite FX] 🎨 Chroma settings: angle=54, noise_level=55
[Composite FX] ✅ Alpha element created with chroma key settings
[Composite FX] 🔗 Pipeline linked WITH chroma key: uridecodebin → videorate → identity_sync → videoconvert → alpha → videoscale → capsfilter
[Composite FX] 🔍 Pipeline: uridecodebin → videorate → identity_sync → videoconvert → alpha (chroma key) → videoscale → capsfilter → compositor
```

## Performance Considerations

1. **CPU Usage:** The alpha element performs real-time chroma key processing, which adds CPU overhead. On modern systems, this should be minimal for 720p/1080p video.

2. **Latency:** The alpha element is placed after `identity_sync` to ensure it doesn't interfere with timing synchronization.

3. **Pipeline Efficiency:** When chroma key is disabled, the alpha element is not added to the pipeline, avoiding any performance overhead.

## Troubleshooting

### Issue: Green not being removed

**Possible causes:**
- Tolerance too low
- Similarity too low
- Wrong keycolor
- Poor green screen lighting

**Solutions:**
- Increase tolerance (try 0.40-0.50)
- Increase similarity (try 0.50-0.60)
- Verify keycolor matches the green in video
- Improve green screen lighting

### Issue: Too much of the image is transparent

**Possible causes:**
- Tolerance too high
- Similarity too high

**Solutions:**
- Decrease tolerance (try 0.20-0.30)
- Decrease similarity (try 0.35-0.45)

### Issue: Pipeline errors

**Check logs for:**
- Alpha element creation errors
- Pipeline linking errors
- Missing GStreamer plugins

**Solutions:**
- Ensure GStreamer alpha plugin is installed
- Check GStreamer environment variables
- Verify pipeline state

## Future Enhancements

1. **Advanced Parameters:**
   - Add `smoothness` and `spill` parameters (currently defined but not used)
   - Implement edge refinement
   - Add despill processing

2. **Real-time Adjustment:**
   - Allow chroma key parameters to be adjusted during playback
   - Add preview mode for testing settings

3. **Presets:**
   - Save commonly used chroma key settings
   - Quick presets for different lighting conditions

4. **GPU Acceleration:**
   - Investigate GPU-accelerated chroma key elements
   - Use OpenGL/Vulkan shaders for better performance

## References

- [GStreamer Alpha Plugin Documentation](https://gstreamer.freedesktop.org/documentation/alpha/index.html)
- [GStreamer Rust Bindings](https://gitlab.freedesktop.org/gstreamer/gstreamer-rs)
- [Tauri IPC Documentation](https://tauri.app/develop/calling-rust/)

