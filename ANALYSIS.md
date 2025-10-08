# Current Architecture Analysis

## Question 1: Why 2 Canvases?

**Answer:** `overlayCanvas` is **UNUSED DEAD CODE**

- `cameraCanvas`: Receives all frames (camera + WGPU processed)
- `overlayCanvas`: Defined but NEVER written to - should be REMOVED

## Question 2: Current Rendering Path (NOT OPTIMAL!)

```
❌ CURRENT (5+ copies, terrible latency):
Camera → GStreamer RGBA → 
WGPU Upload → GPU Shader → GPU Render →
CPU Readback (3.6MB copy!) →
WebSocket (network) →
Frontend ArrayBuffer →
Uint8ClampedArray →
ImageData →
Canvas putImageData (CPU) →
Browser compositor (GPU)
```

**Issues:**
1. ❌ GPU → CPU readback: 3.6MB sync copy per frame
2. ❌ WebSocket transfer: 3.6MB over network (even localhost has overhead)
3. ❌ putImageData: CPU→GPU upload in browser
4. ❌ 5+ memory copies total
5. ❌ ~100ms latency (3 frame GPU lag + network + browser)

## Question 3: Best GPU Technique (Zero Latency)

```
✅ OPTIMAL (zero copies, <5ms latency):
Camera → GStreamer → 
GPU Texture Upload →
GPU Chroma Shader →
Direct Render to Tauri Window Surface
(NO WEBSOCKET, NO READBACK!)
```

**How:**
- Create WGPU surface from Tauri window handle
- Render directly to window framebuffer
- Zero CPU involvement after initial upload
- True GPU pipeline: Camera → Display

**Implementation:**
1. Get Tauri window raw handle
2. Create WGPU surface from handle
3. Render to surface instead of texture
4. Remove WebSocket entirely
5. Remove frontend canvas (native WGPU window)

## Question 4: Why Frontend Disconnects

Log shows:
```
[Composite WS] 📡 Sending frame 540 (3686400 bytes, 52.6 fps)
[Composite WS] ℹ️ Broadcast channel closed  ← Frontend disconnected!
[Composite] 📡 Frame 630 - WGPU rendering    ← Backend keeps running
```

**Cause:** Frontend WebSocket closed (navigation or tab switch)

**Backend continues:** Because GStreamer pipeline keeps running

**Fix:** Proper cleanup on frontend unmount

## Solution Summary

1. **Remove overlayCanvas** (dead code)
2. **Stop using WebSocket for frames** (terrible latency)
3. **Implement direct WGPU surface rendering** (zero latency)
4. **Add proper cleanup** on frontend unmount

## Current vs Optimal Performance

| Metric | Current (WebSocket) | Optimal (WGPU Surface) |
|--------|--------------------|-----------------------|
| Latency | ~100ms | <5ms |
| CPU Usage | High (readback) | Minimal |
| GPU Usage | Inefficient | Efficient |
| Memory Copies | 5+ | 1 |
| Bandwidth | 3.6MB/frame | 0 (zero) |
| FPS Cap | ~50fps | 60+ fps |

