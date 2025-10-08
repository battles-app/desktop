# Stream Deck Image Loading Fix

## Problem
Images were not downloading correctly because:
1. **Incomplete URLs**: Frontend was passing relative paths like `/directus-assets/xxx`
2. **Tauri Limitation**: Rust backend couldn't access these paths (they're Nuxt proxy routes)
3. **Race Condition**: Images were downloaded AFTER buttons rendered

## Solution

### Frontend Changes (`battles.app/components/DashboardView.vue`)
**Generated full URLs** that can be accessed from outside the browser:

```typescript
const baseUrl = window.location.origin // e.g., https://local.battles.app:3000

// Battle board images
image_url: `${baseUrl}/directus-assets/${item.file.id}`

// User FX images
image_url: `${baseUrl}/directus-assets/${file.id}`
```

### Backend Changes (`battlesDesktop/src/streamdeck_manager.rs`)

1. **Direct URL usage**: Use the full URL from frontend directly
   ```rust
   // Before: format!("https://local.battles.app:3000{}", image_url)
   // After: image_url (already complete)
   ```

2. **Download before rendering**: Images are now downloaded **BEFORE** buttons render
   ```rust
   // Check which images need downloading
   // Download them synchronously with rate limiting (150ms delays)
   // THEN render the layout
   ```

## How It Works Now

1. **Frontend loads FX data** from Directus API
2. **Generates full URLs** like `https://local.battles.app:3000/directus-assets/f1bd0750-...`
3. **Sends to Rust backend** via Tauri IPC
4. **Rust downloads missing images** to `%TEMP%/battles_fx_cache/`
   - Sequential download with 150ms delays between requests
   - Robust HTTP client with timeouts and error handling
   - Images cached by FX name (e.g., `x2.jpg`, `galaxy.jpg`)
5. **Renders buttons** with downloaded images

## URL Flow

```
Directus (tiktok.b4battle.com)
    ↓
Nuxt Proxy (/directus-assets/[id])
    ↓
Full URL (https://local.battles.app:3000/directus-assets/[id])
    ↓
Tauri Rust (downloads with reqwest)
    ↓
Local Cache (%TEMP%/battles_fx_cache/)
    ↓
Stream Deck Buttons (with images!)
```

## Test Results Expected

**First Load:**
```
[Stream Deck] Downloading 12 missing images...
[Stream Deck] Downloading image from: https://local.battles.app:3000/directus-assets/f1bd0750-...
[Stream Deck] ✅ Cached x2 (45231 bytes)
[Stream Deck] ✅ Cached x3 (52341 bytes)
...
[Stream Deck] ✅ Image download complete, rendering layout...
[Stream Deck] ✅ Found cached image for x2: Some("x2.jpg")
```

**Subsequent Loads:**
```
[Stream Deck] ✅ Found cached image for x2: Some("x2.jpg")
[Stream Deck] ✅ Found cached image for x3: Some("x3.jpg")
[Stream Deck] ✅ Layout updated successfully
```

## Benefits

✅ **Works from outside browser** - Full URLs accessible from Tauri  
✅ **Reliable downloads** - Sequential with rate limiting, no connection exhaustion  
✅ **Instant subsequent loads** - Images cached locally  
✅ **Proper error handling** - Detailed logs for debugging  
✅ **No IPC timeout** - Downloads happen before rendering completes  

## Testing

1. **Clear cache**: Delete `%TEMP%/battles_fx_cache/` folder
2. **Restart app**: `bun run tauri dev`
3. **Watch console**: Look for download progress and success messages
4. **Check Stream Deck**: Should show actual FX images!

## Next Steps

- Monitor download logs for any errors
- Verify all images appear correctly on Stream Deck
- Test with different network conditions

