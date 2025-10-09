# Stream Deck Debugging Guide

## 🔍 Comprehensive Logging Added

### Issue 1: Button Presses Not Working
### Issue 2: Green Border Not Appearing

## Complete Event Flow with Logging

### 1. **Stream Deck Button Press** → Dashboard

```
[RUST] Stream Deck Watcher (main.rs)
  ↓
🔍 Detected N button press(es): [idx]
🔘 Button X pressed - processing...
🎮 Button X toggled FX 'fxfile001' to PLAYING ▶
📤 Emitting event to frontend: streamdeck://button_press
   → fx_id: fxfile001, is_playing: true, button_idx: 0
✅ Event emitted successfully

[FRONTEND] useStreamDeck.ts
  ↓
📥 Received button press event from Tauri!
   → Payload: { fxId, isPlaying, buttonIdx }
   → Parsed: fxId=fxfile001, isPlaying=true, buttonIdx=0
📤 Dispatching custom window event: streamdeck-button-press
✅ Custom event dispatched

[FRONTEND] DashboardView.vue
  ↓
📥 ============ STREAM DECK BUTTON PRESS EVENT ============
   → Parsed: fxId=fxfile001, isPlaying=true, buttonIdx=0
   → Available global FX: X
   → Available user FX: Y
🔍 Checking if user FX... (pattern: fxfile###)
✅ Matched user FX pattern!
   → FX Number: 1
   → FX Index: 0
   → FX Key: fxfile001
   → Action: PLAY
✅ FX exists, triggering action...
✅ Action completed
```

### 2. **Dashboard Play** → Stream Deck

```
[FRONTEND] DashboardView.vue
  ↓
playFxFile(index) or playGlobalFx(item)
  ↓
streamDeck.setButtonState(fxId, true)

[FRONTEND] useStreamDeck.ts
  ↓
🎨 setButtonState called: { fxId, isPlaying }
   → isTauri: true
   → isInitialized: true
   → isConnected: true
📤 Invoking streamdeck_set_button_state...
✅ Button state updated successfully: fxId -> PLAYING (GREEN)

[RUST] main.rs (Tauri Command)
  ↓
📥 streamdeck_set_button_state called
   → fx_id: fxfile001
   → is_playing: true
✅ Manager found, calling set_button_state...
✅ Button state updated successfully: fxfile001 -> PLAYING (GREEN)

[RUST] streamdeck_manager.rs
  ↓
🎨 set_button_state called
   → fx_id: fxfile001
   → is_playing: true
   → button_layout.len: 32
🔍 Searching for FX ID in button layout...
✅ Found FX at button index 5: My Effect
🔄 Updating button state...
✅ State updated in memory
🎨 Creating button image...
✅ Image created, setting on device...
✅ Button image set, flushing...
✅ Flushed! Button 5 now shows: GREEN BORDER
✅ set_button_state completed successfully
```

## How to Debug

### Step 1: Restart the App with Logging
```bash
cd battlesDesktop
cargo run
```

### Step 2: Test Button Press
1. **Press a button on Stream Deck**
2. **Look for these logs in order:**
   - `[Stream Deck Watcher] 🔍 Detected 1 button press(es)`
   - `[Stream Deck Watcher] 📤 Emitting event to frontend`
   - `[Stream Deck] 📥 Received button press event from Tauri!`
   - `[Dashboard] 📥 ============ STREAM DECK BUTTON PRESS EVENT`
   - `[Dashboard] ✅ FX exists, triggering action...`

### Step 3: Test Dashboard Play
1. **Click play on dashboard**
2. **Look for these logs in order:**
   - `[Stream Deck] 🎨 setButtonState called`
   - `[Stream Deck] 📤 Invoking streamdeck_set_button_state...`
   - `[Stream Deck Command] 📥 streamdeck_set_button_state called`
   - `[Stream Deck Manager] 🎨 set_button_state called`
   - `[Stream Deck Manager] ✅ Flushed! Button X now shows: GREEN BORDER`

## Common Issues & Fixes

### Issue: Button Press Logs Show "⚠️ No FX info"
**Cause:** Button is empty or a control button
**Solution:** This is expected for empty slots and control buttons (INTRO, PARTY, etc.)

### Issue: "❌ FX ID not found in layout"
**Cause:** FX ID mismatch between dashboard and Stream Deck layout
**Solution:** Check the "Available FX IDs" log to see what IDs are actually in the layout

### Issue: "⚠️ Not initialized" or "⚠️ Not connected"
**Cause:** Stream Deck not properly initialized
**Solution:** Look for `[Stream Deck] ✅ Connected to XlV2` earlier in logs

### Issue: "⚠️ Window object not available!"
**Cause:** Event received before DOM ready
**Solution:** Wait for page load, should auto-recover

### Issue: Green border doesn't appear
**Causes:**
1. **Check if setButtonState is being called:**
   - Look for `[Stream Deck] 🎨 setButtonState called`
   - If missing → Dashboard not calling it
   
2. **Check if it reaches Rust:**
   - Look for `[Stream Deck Command] 📥 streamdeck_set_button_state called`
   - If missing → Frontend invoke failing
   
3. **Check if FX is found:**
   - Look for `[Stream Deck Manager] ✅ Found FX at button index`
   - If missing → FX ID mismatch
   
4. **Check if device update succeeds:**
   - Look for `[Stream Deck Manager] ✅ Flushed!`
   - If missing → Device communication issue

## Debug Checklist

- [ ] Stream Deck physically connected?
- [ ] Logs show "Connected to XlV2"?
- [ ] Event listener added: `🎧 Adding event listener`?
- [ ] Layout loaded: `[Stream Deck] Updating layout with X battle board + Y user FX`?
- [ ] Button press detected: `🔍 Detected N button press(es)`?
- [ ] Event reached frontend: `📥 Received button press event`?
- [ ] Event reached dashboard: `📥 ============ STREAM DECK BUTTON PRESS EVENT`?
- [ ] setButtonState called when playing: `🎨 setButtonState called`?
- [ ] Rust command received: `📥 streamdeck_set_button_state called`?
- [ ] Manager updated: `✅ Flushed! Button X now shows: GREEN BORDER`?

## Expected Log Volume

**Normal Operation:**
- Button press: ~15-20 log lines
- Dashboard play: ~10-15 log lines
- Every 2 seconds: Device connection check (minimal)

**If logs are missing:**
- Button press logs stop at watcher → Check HID device connection
- Button press logs stop at frontend → Check Tauri event system
- setButtonState logs missing → Check if `streamDeck.setButtonState()` is called
- Rust logs missing → Check if Tauri command is registered

## Quick Test

1. **Press Stream Deck button** → Should see `🎮 Button X toggled FX` within 1 second
2. **Click dashboard play** → Should see `✅ Flushed! Button X now shows: GREEN BORDER` within 1 second
3. **Click dashboard stop** → Should see `✅ Flushed! Button X now shows: NO BORDER` within 1 second

If any of these fail, the logs will show exactly where the chain breaks!

## Log Search Patterns

```bash
# Search for button press detection
grep "Detected.*button press"

# Search for event emissions
grep "Emitting event to frontend"

# Search for setButtonState calls
grep "setButtonState called"

# Search for green border updates
grep "GREEN BORDER"

# Search for errors
grep "❌"

# Search for warnings
grep "⚠️"
```

## Next Steps

1. **Run the app**
2. **Try pressing a Stream Deck button**
3. **Try playing from dashboard**
4. **Share the logs** - they will show exactly where the problem is!

The comprehensive logging will reveal:
- ✅ Which events are firing
- ✅ Which functions are being called
- ✅ Where the data flow stops
- ✅ What values are being passed
- ✅ Whether device updates succeed

