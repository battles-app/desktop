# Black Canvas Diagnostic Guide

## Your Current Issue
Pipeline starts successfully but no frames are being captured - resulting in a black canvas.

## What Changed
I've added **extensive debugging** to track exactly where frames are getting stuck:

### New Debug Messages
1. `[Composite] ✅ AppSink callbacks configured` - Confirms callback is registered
2. `[Composite] ⏳ Waiting for pipeline to reach PAUSED state...` - State transition tracking
3. `[Composite] 📊 Pipeline state: Playing (pending: VoidPending)` - Final state verification
4. `[Composite] 🎬 FIRST FRAME CAPTURED! (X bytes)` - **THIS IS WHAT WE'RE MISSING**
5. `[Composite] 📡 Frame 30 captured (X bytes)` - Ongoing frame capture
6. `[Composite] ✅ Frame broadcast to X receivers` - WebSocket delivery confirmation

## Step-by-Step Diagnosis

### Step 1: Restart the App with Test Pattern
```bash
cd battlesDesktop
bun run tauri dev
```

**In the Dashboard:**
1. Select "🎨 Test Pattern (Debug)" from the dropdown
2. Watch the console closely

**Expected Output (if pipeline works):**
```
[Composite] ✅ AppSink callbacks configured
[Composite] 🔄 Setting pipeline to PAUSED state...
[Composite] ⏳ Waiting for pipeline to reach PAUSED state...
[Composite] ✅ Pipeline is PAUSED and ready
[Composite] 🔄 Setting pipeline to PLAYING state...
[Composite] ✅ Pipeline set to PLAYING
[Composite] 📊 Pipeline state: Playing (pending: VoidPending)
[Composite] ✅ Pipeline fully initialized and running
[Composite WS] ✅ Client connected
[Composite] 🎬 FIRST FRAME CAPTURED! (15234 bytes)  ← KEY LINE!
[Composite] ✅ Frame broadcast to 1 receivers
[Composite] 📡 Frame 30 captured (15234 bytes)
```

**If you see the FIRST FRAME message:** ✅ Pipeline works! The issue is camera-specific
**If you DON'T see FIRST FRAME:** ❌ General pipeline issue

### Step 2: Check for Errors/Warnings
Look for any of these messages:
- `[Composite] ❌ Failed to pull sample`
- `[Composite] ❌ Sample has no buffer`
- `[Composite] ❌ Failed to map buffer`
- `[Composite] ❌ Pipeline error:`
- `[Composite] ⚠️ Pipeline warning:`
- `[Composite] ⚠️ Pipeline in unexpected state:`

### Step 3: Test with Real Camera
If test pattern works, select your camera again:
1. Select your camera from dropdown
2. Compare console output

**Camera-specific issues to look for:**
- `[Composite] ⚠️ Warning: Camera device path format looks unusual`
- State stuck in "Paused" instead of "Playing"
- Bus error messages about device access

## Common Issues & Solutions

### Issue 1: Camera in Use by Another App
**Symptoms:**
- Test pattern works ✅
- Camera fails ❌
- May see "device busy" in logs

**Solution:**
- Close OBS, Zoom, Teams, browser tabs with camera access
- Try again

### Issue 2: Camera Resolution Not Supported
**Symptoms:**
- Test pattern works ✅
- Camera fails ❌
- Pipeline state stuck or warning about format negotiation

**Solution:**
We're requesting 720x1280 @ 30fps. Camera may not support this exact resolution.
Try modifying in CompositeCanvas.vue:
```javascript
// Try 640x360 instead (line ~214)
const compositeWidth = computed(() => isVertical.value ? 360 : 640)
const compositeHeight = computed(() => isVertical.value ? 640 : 360)
```

### Issue 3: mfvideosrc Plugin Missing/Broken
**Symptoms:**
- Test pattern works ✅
- Camera fails immediately ❌
- Error about "mfvideosrc" element

**Solution:**
GStreamer Windows plugin issue. Try:
```powershell
# Reinstall GStreamer plugins
choco install gstreamer -y
choco install gstreamer-plugins-good -y
choco install gstreamer-plugins-bad -y
```

### Issue 4: General GStreamer Issue
**Symptoms:**
- Test pattern fails ❌
- Camera fails ❌
- No frames at all

**Possible causes:**
1. **jpegenc plugin missing:** JPEG encoder not found
2. **videoconvert plugin missing:** Format converter not found
3. **appsink not working:** Frame delivery broken

**Check GStreamer installation:**
```powershell
gst-inspect-1.0 jpegenc
gst-inspect-1.0 videoconvert
gst-inspect-1.0 appsink
```

All should return plugin info, not "No such element".

### Issue 5: WebSocket Connection Timing
**Symptoms:**
- Frames are being captured (you see the logs)
- But canvas is still black
- WebSocket connects but no data received

**Check:**
1. WebSocket client connects AFTER pipeline starts
2. Broadcast channel has receivers
3. Frontend is listening on correct port (9877)

**Frontend log should show:**
```
[Composite] WebSocket connected
[Composite] WebSocket message received: { type: 'Blob', size: 15234 }
[Composite] 🎬 Rendering frame from WebSocket
```

## Quick Test: Command Line GStreamer

Test GStreamer directly (bypasses Rust code):

```powershell
# Test pattern to window (should show animated ball)
gst-launch-1.0 videotestsrc pattern=ball ! videoconvert ! autovideosink

# Test pattern to JPEG (should see file sizes printed)
gst-launch-1.0 videotestsrc pattern=ball num-buffers=10 ! video/x-raw,width=1280,height=720 ! jpegenc ! filesink location=test_%d.jpg
```

If these fail, GStreamer installation is broken.

## What to Send Me

When you test, please send me:

1. **Full console output** from restart until 5 seconds after connecting
2. **Which test worked:**
   - ✅ Test pattern
   - ✅ Camera
   - ❌ Neither
3. **Key lines to include:**
   - "FIRST FRAME CAPTURED" (if present)
   - Any ❌ error messages
   - Any ⚠️ warning messages
   - "Pipeline state: X" line
4. **Camera details:**
   - Camera name/model
   - Is it USB or built-in?
   - Does it work in other apps right now?

## Expected Behavior (Working System)

When everything works, you should see:
- Console: `FIRST FRAME CAPTURED` within 1 second of pipeline start
- Canvas: Animated ball (test pattern) or camera feed appears immediately
- Smooth 30 FPS video with no stuttering
- Console updates every 30 frames (~1 second)

Let's diagnose this together! 🔍

