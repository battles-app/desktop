# Stream Deck Updates - October 9, 2025

## Changes Implemented

### 1. ✅ Control Buttons Layout (XL Devices)
- **Added 4 control buttons** on the rightmost column of Stream Deck XL:
  - **INTRO** (Purple: #8A2BE2)
  - **PARTY** (Hot Pink: #FF69B4)
  - **BREAK** (Blue: #1E90FF)
  - **END** (Crimson: #DC143C)
- **Layout optimization**:
  - Battle Board: Left 5 columns (max 20 buttons)
  - User FX: Columns 5-6 (max 12 buttons)
  - Control buttons: Column 7 (rightmost)
- Smaller devices maintain standard split layout (left/right)

### 2. ✅ Image Loading Fixes
- **Changed image sources**:
  - Now uses `item.image` field for thumbnails (instead of `item.file`)
  - Falls back to `item.file` if no separate image exists
  - Supports separate `fximage` fields for user FX
- **Correct image sizes** per Elgato documentation:
  - Standard devices (Original, Mk2, Mini): **144x144** (high-DPI)
  - XL devices: **96x96**
  - Plus/Neo: **200x200**
- **Background image downloading**:
  - Non-blocking downloads from Nuxt proxy
  - Caches to `%TEMP%\battles_fx_cache\{name}.jpg`
  - Retries on each layout update if not cached
  - Uses HTTPS with self-signed cert support

### 3. ✅ Play/Stop Signal Synchronization
- **Auto-updates Stream Deck when media finishes**:
  - `onAudioEnded`: Updates all FX and global FX button states
  - `onSoundFxEnded`: Updates only audio FX button states
  - Buttons automatically turn off (remove green border) when playback completes
- **Manual play/stop buttons** in dashboard update Stream Deck in real-time:
  - `playFxFile` / `stopFxFile`: Updates user FX buttons
  - `playGlobalFx` / `stopGlobalFx`: Updates battle board buttons
  - Uses `setButtonState(fxId, isPlaying)` to sync visuals

### 4. ✅ Visual Improvements
- **Control buttons**:
  - Large centered text
  - No borders
  - Full-color backgrounds
  - Reserved for future functionality
- **FX buttons**:
  - Image thumbnails (when available)
  - Text overlay at bottom with semi-transparent background
  - Green border when playing
  - Purple/Blue tint when stopped (indicates button type)

## Technical Details

### Frontend Changes (`battles.app/components/DashboardView.vue`)
```typescript
// Now sends image field instead of file for thumbnails
const battleBoard = globalFxItems.value.map((item, index) => ({
  id: String(item.id),
  name: item.name || `Global FX ${index + 1}`,
  image_url: item.image?.id ? `/directus-assets/${item.image.id}` : 
             (item.file?.id ? `/directus-assets/${item.file.id}` : undefined),
  is_global: true,
  position: index
}))

// Checks for separate image field for user FX
const imageFile = fxFiles.value[fxImageKey]
image_url: imageFile?.id ? `/directus-assets/${imageFile.id}` : 
           (file.id ? `/directus-assets/${file.id}` : undefined)
```

### Backend Changes (`battlesDesktop/src/streamdeck_manager.rs`)
```rust
// High-DPI button sizes per Elgato docs
fn get_button_size(&self) -> u32 {
    match self.device_kind {
        Some(Kind::Original) | Some(Kind::OriginalV2) |
        Some(Kind::Mk2) | Some(Kind::Mk2Scissor) | 
        Some(Kind::Mini) | Some(Kind::MiniMk2) => 144, // High-DPI
        Some(Kind::Xl) | Some(Kind::XlV2) => 96, // XL uses 96x96
        // ...
    }
}

// XL layout with control buttons
if matches!(self.device_kind, Some(Kind::Xl) | Some(Kind::XlV2)) {
    // Control buttons in column 7
    let control_buttons = vec![
        ("INTRO", [138, 43, 226]),
        ("PARTY", [255, 105, 180]),
        ("BREAK", [30, 144, 255]),
        ("END", [220, 20, 60]),
    ];
    // ...
}
```

## Image Download Flow

1. **Dashboard loads** → Fetches global FX and user FX with `image` and `file` fields
2. **Layout update** → Sends `image_url` (from `image.id` or fallback to `file.id`)
3. **Stream Deck receives** → Starts background download for each image URL
4. **Background thread** → Downloads from `https://local.battles.app:3000/directus-assets/{id}`
5. **Cache on disk** → Saves as `{fx_name}.jpg` in temp cache
6. **Button render** → Uses cached image if available, falls back to colored background

## API Structure

### Global FX Item
```json
{
  "id": 13,
  "name": "fireworks",
  "file": {
    "id": "6a348426-3cc7-4e9d-acb9-def9d7cd2015",
    "type": "video/mp4",
    "filename_download": "fireworks-001_1_thm2_apo8_prob4_hyp1-1920h-30fps-double.mp4"
  },
  "image": {
    "id": "abc123-...",
    "type": "image/jpeg",
    "filename_download": "fireworks_thumb.jpg"
  },
  "fxchroma": true
}
```

### User FX Fields
```typescript
fxfile001 // Video/audio file
fxname001 // Display name
fximage001 // Optional thumbnail image (if different from file)
```

## Testing Checklist

- [x] Control buttons appear on Stream Deck XL (column 7)
- [x] Battle board occupies columns 0-4
- [x] User FX occupies columns 5-6
- [x] Images download and cache properly
- [x] Buttons use correct sizes (144x144 standard, 96x96 XL)
- [x] Play button updates Stream Deck to green border
- [x] Stop button removes green border
- [x] Media finish auto-updates Stream Deck state
- [x] Control buttons show colored backgrounds with centered text

## Next Steps (Future Enhancements)

1. **Physical button press detection**: Implement Stream Deck button press handling (currently read-only)
2. **Control button functionality**: Wire up INTRO, PARTY, BREAK, END buttons to trigger events
3. **Button press events**: Send commands back to dashboard when Stream Deck buttons are pressed
4. **WebSocket events**: Subscribe to composite pipeline events for video FX completion
5. **Image preview fallback**: Generate video thumbnails if no separate image provided

