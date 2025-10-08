# Troubleshooting Direct Surface Rendering

## Expected Behavior

**IMPORTANT:** Video does **NOT** render to the canvas anymore!

### Where Video Appears

The video now renders **DIRECTLY to the native Tauri window** using WGPU surface rendering. This means:

1. ✅ Video bypasses the WebView completely
2. ✅ Video renders to the native OS window
3. ✅ Zero latency, direct GPU → display
4. ❌ Video is NOT visible in the `<canvas>` element

### Current Issue: Video Behind WebView

**Problem:** The WGPU surface renders to the Tauri window, but the WebView (your Vue UI) is rendered **on top** of it, blocking the video.

**Solutions:**

### Solution 1: Transparent Canvas Area (Quick Fix)

Make the canvas area of your Vue component transparent so video shows through:

```vue
<template>
  <div class="composite-container">
    <!-- Make this div transparent to see WGPU surface below -->
    <div class="video-viewport" style="background: transparent !important;">
      <!-- Controls overlay OVER the native video -->
      <div class="controls-overlay">
        <!-- Camera selector, FPS, rotation controls -->
      </div>
    </div>
  </div>
</template>

<style scoped>
.video-viewport {
  background: transparent !important;
  -webkit-app-region: no-drag;
}
</style>
```

### Solution 2: Remove Canvas Element

Since video doesn't use canvas anymore, remove it:

```vue
<!-- REMOVE THIS -->
<!-- <canvas ref="cameraCanvas" /> -->

<!-- KEEP THIS (UI overlay) -->
<div class="controls-overlay">
  <!-- Camera selector, etc. -->
</div>
```

### Solution 3: Check Tauri Window Config

Ensure window is configured correctly in `tauri.conf.json`:

```json
{
  "tauri": {
    "windows": [
      {
        "label": "main",
        "transparent": false,
        "decorations": true
      }
    ]
  }
}
```

## Diagnostic Steps

### Step 1: Check Initialization Logs

Restart app and look for these logs in console:

```
[WGPU Surface] 🚀 Initializing direct surface renderer (720x1280)
[WGPU Surface] 🔧 Creating surface from Tauri window...
[WGPU Surface] ✅ Surface created successfully
[WGPU Surface] 🔧 Requesting GPU adapter...
[WGPU Surface] ✅ GPU adapter found
[WGPU Surface] 🔧 Requesting GPU device...
[WGPU Surface] ✅ GPU device created
[WGPU Surface] 🔧 Configuring surface...
[WGPU Surface] 📊 Surface format: Bgra8UnormSrgb
[WGPU Surface] ✅ Surface configured (720x1280)
[WGPU Surface] ✅ Surface renderer initialized
[Composite] ✅ Surface renderer initialized - ready for ZERO-LATENCY rendering!
```

**If ANY step fails, report the error.**

### Step 2: Check Camera Pipeline Logs

After starting camera:

```
[Composite] Creating pipeline: mfvideosrc device-path=...
[Composite] 🎬 FIRST FRAME! Processing with WGPU (720x1280)
[Composite] ✅ Frame 90 → DIRECT TO SCREEN (zero-latency)
```

**If no frames appear, check GStreamer logs.**

### Step 3: Look for Errors

Common errors:

**"Failed to get surface target"**
- Tauri window not ready
- Try delaying surface creation

**"Failed to find suitable GPU adapter"**
- WGPU can't find compatible GPU
- Check GPU drivers

**"Failed to create surface"**
- Window handle invalid
- Tauri WebView window incompatibility

## Alternative: Fallback to Canvas

If WGPU surface doesn't work with WebView, we can fall back to a **hybrid approach**:

1. Keep WGPU rendering (for chroma key)
2. Use **shared memory** instead of WebSocket
3. Render to canvas (but with zero-copy)

This gives you:
- ✅ Fast rendering (<50ms)
- ✅ Visible in browser
- ✅ Debuggable
- ❌ Not as fast as direct surface (but 3× faster than current)

## What to Report

When debugging, provide:

1. **All logs** from app restart
2. **First error** that appears
3. **OS and GPU info** (Windows + GPU model)
4. **Screenshot** of what you see
5. **Console errors** in DevTools

## Expected Timeline

- If surface creation **succeeds**: Video should appear behind WebView (need CSS fix)
- If surface creation **fails**: Need to diagnose error and find solution
- If fundamentally incompatible: Fall back to shared memory + canvas approach

