# Stream Deck Image Debug

## What to look for in the console:

### 1. Content-Type (Are they images or videos?)
```
[Stream Deck] Content-Type for x2: image/jpeg  ← ✅ Good (image)
[Stream Deck] Content-Type for galaxy: video/mp4  ← ❌ Problem (video)
```

### 2. Image Loading (Does the image crate load them?)
```
[Stream Deck] ✅ Successfully loaded image for x2: 1920x1080  ← ✅ Good
[Stream Deck] ❌ Failed to load image for galaxy: ... (file might be video)  ← ❌ Problem
```

### 3. Fallback (What happens when loading fails?)
```
[Stream Deck] ⚠️ No cached image found for FX 5  ← Using colored background
```

## Expected Issues

### If Content-Type is `video/*`:
**Problem**: Directus file IDs point to video files, not thumbnails  
**Solution**: We need to either:
1. Extract the actual image thumbnail URL from Directus
2. Generate thumbnails from videos using FFmpeg
3. Use placeholder images for videos

### If Content-Type is `image/*` but loading fails:
**Problem**: Image format not supported by `image` crate  
**Solution**: Check file format and add missing features to Cargo.toml

### If Content-Type is `unknown`:
**Problem**: Proxy not forwarding headers correctly  
**Solution**: Fix Nuxt proxy to forward content-type

## Restart and Report

1. **Clear cache** (optional): Delete `%TEMP%\battles_fx_cache\`
2. **Restart app**: Close and re-run `bun run tauri dev`
3. **Watch console** for the three log types above
4. **Report** what you see for 2-3 example FX items

## Quick Check Commands

**List cached files:**
```powershell
dir $env:TEMP\battles_fx_cache
```

**Check file types:**
```powershell
Get-ChildItem $env:TEMP\battles_fx_cache | Select-Object Name, Length
```

