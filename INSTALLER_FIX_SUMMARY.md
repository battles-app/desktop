# Installer Fixes - Summary

## 🔴 Critical Issues Fixed

### 1. Source Code Leakage (CRITICAL SECURITY)
**Problem**: Installer was packaging ALL Rust source code  
**Files Exposed**: 
- src/main.rs
- src/gstreamer_camera.rs
- src/gstreamer_composite.rs  
- src/streamdeck_manager.rs
- src/streamdeck_diagnostics.rs
- src/screen_capture.rs
- src/wgpu_surface_renderer.rs
- src/chroma_key_shader.wgsl
- src/favicon.png

**Fix**: Removed `"src"` from `tauri.conf.json5` resources  
**Status**: ✅ FIXED

**Note**: `favicon.png` and `chroma_key_shader.wgsl` are embedded at compile time using:
- `include_bytes!("../favicon.png")` in streamdeck_manager.rs
- `include_str!("chroma_key_shader.wgsl")` in wgpu_surface_renderer.rs

### 2. Stream Deck FX Not Loading in Production
**Problem**: Released app couldn't download FX button images  
**Root Cause**: Used `battles.app` URL in debug builds instead of `local.battles.app:3000`  
**Fix**: Updated `src/streamdeck_manager.rs` line 641  
**Status**: ✅ FIXED

### 3. FX Canvas/Chroma Key Not Working in Production  
**Problem**: Released app couldn't download FX videos  
**Root Cause**: Used `battles.app` URL in debug builds instead of `local.battles.app:3000`  
**Fix**: Updated `src/main.rs` line 1400  
**Status**: ✅ FIXED

## 📦 DLL Bundling Status

### Current Build Process
1. `build.rs` copies DLLs to `target/release/`
2. Copies plugins to `target/release/gstreamer-1.0/`
3. Tauri NSIS bundler should include all DLLs from target directory

### DLLs Currently in target/release:
✅ Core Libraries (24 DLLs):
- glib-2.0-0.dll
- gobject-2.0-0.dll
- gmodule-2.0-0.dll
- gio-2.0-0.dll
- gstreamer-1.0-0.dll
- gstbase-1.0-0.dll
- gstapp-1.0-0.dll
- gstvideo-1.0-0.dll
- gstaudio-1.0-0.dll
- gstpbutils-1.0-0.dll
- gstcontroller-1.0-0.dll
- gstnet-1.0-0.dll
- gstgl-1.0-0.dll
- gstallocators-1.0-0.dll
- gstrtp-1.0-0.dll
- gstrtsp-1.0-0.dll
- gsttag-1.0-0.dll
- intl-8.dll
- ffi-7.dll
- z-1.dll
- pcre2-8-0.dll
- orc-0.4-0.dll
- pixman-1-0.dll
- graphene-1.0-0.dll

✅ Plugins (14 DLLs):
- gstapp.dll
- gstcoreelements.dll
- gstvideoconvertscale.dll
- gstvideofilter.dll
- gstvideotestsrc.dll
- gstvideoparsersbad.dll
- gstaudioconvert.dll
- gstaudioresample.dll
- gstaudiotestsrc.dll
- gstautodetect.dll
- gstplayback.dll
- gsttypefindfunctions.dll
- gstd3d11.dll
- gstopengl.dll

### Tauri NSIS Bundler Behavior
According to Tauri v2 documentation:
- NSIS bundler automatically includes:
  - Main executable (.exe)
  - All DLLs in the same directory as the exe
  - Subdirectories with files (like gstreamer-1.0/)

**Expected**: DLLs should be automatically bundled  
**To Verify**: Check next installer to ensure all DLLs are present

## 🧪 Testing Checklist for Next Release

### On Fresh Windows Install (No GStreamer):
- [ ] Application launches without DLL errors
- [ ] Stream Deck connects and shows animation
- [ ] Stream Deck loads FX button images
- [ ] FX buttons display correctly
- [ ] Clicking FX button plays effect
- [ ] FX canvas with chroma key works
- [ ] No source code files in installation directory

### Installation Directory Should Contain:
```
C:\Program Files\Battles.app Desktop\
├── battles-desktop.exe
├── glib-2.0-0.dll
├── gobject-2.0-0.dll
├── gmodule-2.0-0.dll
├── gio-2.0-0.dll
├── gstreamer-1.0-0.dll
├── gstbase-1.0-0.dll
├── [... 18 more core DLLs ...]
├── gstreamer-1.0/
│   ├── gstapp.dll
│   ├── gstcoreelements.dll
│   └── [... 12 more plugin DLLs ...]
└── assets/
    └── sounds/
```

### Installation Directory Should NOT Contain:
- ❌ src/ folder
- ❌ *.rs files
- ❌ chroma_key_shader.wgsl (embedded in exe)
- ❌ favicon.png (embedded in exe)

## 📝 Files Modified

1. **tauri.conf.json5** - Removed "src" from resources
2. **src/streamdeck_manager.rs** - Fixed URL for FX image downloads
3. **src/main.rs** - Fixed URL for FX video downloads  
4. **build.rs** - Added bundler documentation

## 🚀 Next Steps

1. Run `bun run release` to build new installer
2. Test on fresh Windows VM without GStreamer
3. Verify all DLLs are present in installer
4. Confirm no source code is included
5. Test Stream Deck and FX functionality

## ⚠️ If DLLs Are Still Missing

If the next installer still doesn't include DLLs, we'll need to:
1. Create custom NSIS script include
2. Explicitly list DLLs in bundle configuration
3. Use Tauri's `externalBin` or additional bundler options

---

**Last Updated**: 2025-10-10  
**Version**: 0.0.17 (pending release)

