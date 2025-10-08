# WGPU Readback Bottleneck Analysis

## The Problem

GPU→CPU synchronous readback is **TOO SLOW** for real-time video (30fps):

```
Camera (30fps) → GStreamer RGBA → 
GPU Upload (fast) → 
GPU Shader (fast) → 
GPU→CPU Readback (BLOCKING! ~1000ms) ← BOTTLENECK!
→ WebSocket → Frontend
```

**Result:** 1 FPS instead of 30 FPS

## Why GPU Readback is Slow

1. **Synchronous CPU-GPU transfer**: `copy_texture_to_buffer` + `map_async` blocks until GPU finishes
2. **Memory copy overhead**: 720×1280×4 = 3.6MB per frame copied from VRAM to RAM
3. **Pipeline stall**: CPU waits for GPU to flush entire command queue

## The Solution: Don't Read Back!

### Architecture 1: Direct WGPU Window Rendering (BEST)
```
Camera → GStreamer → 
GPU Texture Upload → 
GPU Chroma Shader → 
Direct Surface Render (Tauri/egui window)
```

**Pros:**
- ✅ Zero CPU-GPU transfer
- ✅ True 30fps GPU-accelerated rendering
- ✅ Low latency (<5ms)
- ✅ Chroma key processed on GPU

**Cons:**
- Requires Tauri window with WGPU surface
- Frontend canvas needs to be replaced with native window

### Architecture 2: Skip WGPU Processing (CURRENT)
```
Camera → GStreamer RGBA → 
WebSocket → Frontend Canvas
```

**Pros:**
- ✅ Fast (30fps+)
- ✅ Works with current frontend

**Cons:**
- ❌ No GPU chroma key
- ❌ No GPU effects

### Architecture 3: Async GPU Readback (COMPLEX)
```
Camera → GStreamer → 
GPU Upload → GPU Render → 
Queue Readback (non-blocking) → 
N frames later → 
Process readback → WebSocket
```

**Pros:**
- GPU processing works
- Can maintain 30fps with latency

**Cons:**
- ❌ Complex triple-buffering needed
- ❌ 3-5 frame latency
- ❌ Still wastes bandwidth on CPU-GPU transfer

## Recommendation

**Use Architecture 1**: Render WGPU directly to a Tauri window surface:

1. Remove WebSocket frame broadcasting
2. Create WGPU surface from Tauri window handle
3. Render directly to window (no readback!)
4. Ultra-low latency GPU preview

This is what you originally wanted: **"direct draw canvas to avoid unnecessary conversions and transfer"**

## Current Status

WGPU renderer is initialized but **disabled** to maintain 30fps.
To enable GPU processing, implement Architecture 1 (direct surface rendering).

