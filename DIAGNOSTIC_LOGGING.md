# Diagnostic Logging - Production Troubleshooting

## Purpose
Comprehensive logging added to diagnose 3 critical issues reported in production builds that work locally but fail in deployed installers.

---

## Issues Being Diagnosed

### Issue #1: Stream Deck Not Mounting FX
**Symptom**: Stream Deck connects, plays animation, but never shows FX buttons (no names, images, or functionality)

**Logging Added**:
- `update_layout_internal()` - Logs all FX being mounted:
  - Battle Board FX count
  - User FX count
  - Details for each FX: ID, name, image URL
- `download_image_to_cache()` - Logs image download attempts:
  - FX name and image URL for each download
  - Warnings when no image URL provided
  - Download success/failure for each image

**What to Look For**:
```
[Stream Deck] 📊 Updating layout with X battle board + Y user FX items
[Stream Deck] 🔍 Battle Board FX:
[Stream Deck]   1. ID: fx001, Name: Sparkles, Image: Some("/api/fx/sparkles.jpg")
[Stream Deck] 🔍 User FX:
[Stream Deck]   1. ID: uf001, Name: Custom, Image: Some("/api/user/custom.jpg")
[Stream Deck] 📥 Starting download for FX: Sparkles (/api/fx/sparkles.jpg)
```

**Expected Behavior**:
- Should see FX list with valid IDs and image URLs
- Should see download attempts for each FX
- Images should successfully download and cache

**Failure Indicators**:
- Empty FX lists (0 battle board + 0 user FX)
- Missing image URLs (Image: None)
- Download failures or 404 errors
- Wrong base URL being used (check if it's using local vs production)

---

### Issue #2: FX Canvas/Chroma Key Not Showing
**Symptom**: FX overlay canvas with chroma keying works locally but doesn't appear in dashboard in production

**Logging Added**:
- `initialize_composite_system()` - Step-by-step initialization:
  - GStreamer composite creation
  - Global state storage
  - Broadcast channel creation (60 frame buffer)
  - Frame sender connection
  - WebSocket server startup on port 9877
  - Final initialization confirmation

**What to Look For**:
```
[Composite] 🔧 Initializing GStreamer composite pipeline...
[Composite] ✅ GStreamer composite created successfully
[Composite] 📦 Composite stored in global state
[Composite] 📡 Created broadcast channel (60 frame buffer)
[Composite] 🔗 Frame sender connected to composite
[Composite] 🌐 Starting WebSocket server on port 9877...
[Composite] ✅ Composite system initialized
```

**Expected Behavior**:
- All initialization steps should succeed
- WebSocket server should start on port 9877
- Frame sender should connect successfully

**Failure Indicators**:
- `❌ ERROR: Failed to create composite:` - GStreamer initialization failed
- Missing confirmation messages - initialization stopped partway
- WebSocket connection failures
- Frame sender not connecting

---

### Issue #3: Camera Sources Missing from Dropdown
**Symptom**: Camera dropdown list is empty, no cameras show up even though camera is connected

**Logging Added**:
- `get_available_cameras()` - Complete camera enumeration process:
  - Start of enumeration
  - Success/failure of GStreamer camera listing
  - Details for each camera found: ID, name, description
  - Total count of cameras
  - Warning messages if no cameras found with troubleshooting hints

**What to Look For**:
```
[Camera] 📹 Starting camera enumeration...
[Camera] ✅ Successfully listed cameras: 2 found
[Camera]   1. ID: /dev/video0, Name: HD Webcam, Description: USB Camera (HD)
[Camera]   2. ID: /dev/video2, Name: OBS Virtual Camera, Description: OBS Virtual Camera
[Camera] 📊 Total cameras available: 2
```

**Expected Behavior**:
- Should find at least 1 camera if camera is connected
- Each camera should have valid ID and name
- Total count should match actual connected cameras

**Failure Indicators**:
```
[Camera] ❌ ERROR: Failed to list cameras: [error message]
[Camera] 📊 Total cameras available: 0
[Camera] ⚠️  WARNING: No cameras found! Check:
[Camera]     • Camera is connected
[Camera]     • Camera drivers are installed
[Camera]     • No other application is using the camera
```

---

## Log Emoji Guide

| Emoji | Meaning | Used For |
|-------|---------|----------|
| 📊 | Data/Statistics | Counts, totals, summaries |
| 🔍 | Details/Inspection | Detailed item listings |
| ✅ | Success | Successful operations |
| ❌ | Error | Failed operations, critical errors |
| ⚠️  | Warning | Non-critical issues, missing data |
| 📹 | Camera | Camera-related operations |
| 🔧 | Initialization | System/component initialization |
| 📥 | Download | File/image downloads |
| 🌐 | Network | WebSocket, HTTP operations |
| 📦 | Storage | Data storage, global state |
| 📡 | Communication | Channels, broadcasts |
| 🔗 | Connection | Component linking |
| 💡 | Info | Helpful information |

---

## How to Use This Logging

### Step 1: Run the Application
Build and run the application with the new logging:
```bash
bun run release
# Install and run the built installer
```

### Step 2: Reproduce the Issues
1. Launch the app
2. Connect Stream Deck
3. Try to use FX buttons
4. Check camera dropdown
5. Try FX overlay canvas

### Step 3: Collect Logs
**Where to find logs**:
- Windows: Check the console output or terminal where app was launched
- If launched from shortcut: Logs might not be visible (need to run from terminal)

**To see logs on production build**:
1. Open PowerShell/CMD
2. Navigate to installation directory:
   ```
   cd "C:\Program Files\Battles.app Desktop"
   ```
3. Run the exe directly:
   ```
   .\battles-desktop.exe
   ```
4. This will show all console logs in the terminal

### Step 4: Analyze Logs
Look for the patterns described above for each issue:
- Missing FX lists
- Composite initialization failures
- Empty camera lists
- 404 errors or wrong URLs
- Connection failures

### Step 5: Report Findings
When reporting issues, include:
- Complete log output from startup to error
- Specific error messages (❌ lines)
- Any warning messages (⚠️  lines)
- What was expected vs what actually happened

---

## Common Issues and Solutions

### Stream Deck FX Not Loading
**If you see**: Empty FX lists or missing image URLs
**Cause**: Frontend not communicating with backend properly
**Solution**: Check WebSocket connection, verify API endpoints

**If you see**: 404 errors on image downloads
**Cause**: Wrong base URL (using local instead of production)
**Solution**: Verify `#[cfg(debug_assertions)]` properly switches URLs

### Composite Not Showing
**If you see**: GStreamer creation failed
**Cause**: Missing DLLs or GStreamer initialization issue
**Solution**: Verify all GStreamer DLLs are bundled in installer

**If you see**: WebSocket server won't start
**Cause**: Port 9877 might be in use
**Solution**: Check if another process is using the port

### No Cameras Found
**If you see**: Camera enumeration fails
**Cause**: GStreamer can't access camera subsystem
**Solution**: Check camera drivers, permissions, no other app using camera

**If you see**: 0 cameras but camera is connected
**Cause**: Camera drivers not installed or camera in use
**Solution**: Install camera drivers, close other camera apps

---

## Next Steps

After collecting logs:
1. Identify which component is failing
2. Check if it's a URL issue (dev vs prod)
3. Verify DLL bundling if GStreamer fails
4. Check WebSocket connectivity
5. Verify API endpoints are accessible

---

**Version**: 0.0.19  
**Last Updated**: 2025-10-10  
**Purpose**: Production troubleshooting for installer-specific issues

