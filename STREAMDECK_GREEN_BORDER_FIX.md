# Stream Deck - Green Border Cleanup Fix

## Issue
When you press button A on Stream Deck (turns green), then press button B, button B starts playing but **button A stays green**. This is wrong because most FX don't play simultaneously - the old one should stop and lose its green border.

## Root Cause
The dashboard code already stops other FX when a new one plays:
```typescript
// This was already working:
fxPlaying.value[parseInt(playingIndex)] = false  // Stops in dashboard state
globalFxPlaying.value[playingId] = false         // Stops in dashboard state

// But this was MISSING:
// streamDeck.setButtonState(stoppedFxKey, false) ❌ NOT CALLED!
```

The internal state was updated, but the Stream Deck was never told to remove the green borders.

## Fix Applied

### In `playFxFile` (User FX):
```typescript
// Hide controls from ALL other playing items
for (const playingIndex of currentlyPlayingFx) {
  if (playingIndex !== index.toString()) {
    fxPlaying.value[parseInt(playingIndex)] = false
    
    // ✅ NEW: Update Stream Deck - remove green border from stopped FX
    if (streamDeck.isAvailable && streamDeck.isConnected.value) {
      const stoppedFxKey = `fxfile${(parseInt(playingIndex) + 1).toString().padStart(3, '0')}`
      await streamDeck.setButtonState(stoppedFxKey, false)
    }
  }
}
for (const playingId of currentlyPlayingGlobal) {
  globalFxPlaying.value[playingId] = false
  
  // ✅ NEW: Update Stream Deck - remove green border from stopped global FX
  if (streamDeck.isAvailable && streamDeck.isConnected.value) {
    await streamDeck.setButtonState(String(playingId), false)
  }
}
```

### In `playGlobalFx` (Battle Board FX):
Same fix applied - now updates Stream Deck for all stopped FX.

## Behavior

### Before Fix ❌
1. Press button A → Button A turns green, FX A plays
2. Press button B → Button B turns green, FX B plays, **Button A stays green** (wrong!)
3. Dashboard knows FX A stopped, but Stream Deck doesn't

### After Fix ✅
1. Press button A → Button A turns green, FX A plays
2. Press button B → Button B turns green, **Button A green removed**, FX B plays
3. Dashboard AND Stream Deck both show only FX B is playing

## Important Notes

### Simultaneous Playback Exception
The code already has logic to allow certain FX to play simultaneously:
- **Audio FX** can play over **Video FX** (they don't stop each other)
- Only **Video FX** stop other **Video FX**

This is handled by the existing logic and the green border fix respects this:
```typescript
if (isVideoFile) {
  // Only stop other VIDEO files, not audio
  // ...
}
```

### When Green Borders Are Removed
Green borders are now removed when:
1. ✅ You press another button on Stream Deck
2. ✅ You play another FX from dashboard
3. ✅ You manually stop an FX from dashboard
4. ✅ Media finishes naturally (existing `onAudioEnded`, `onSoundFxEnded`, `handleFxEndedEvent`)

## Testing

### Test Case 1: Stream Deck → Stream Deck
1. Press button A (should turn green)
2. Press button B (should turn green, A should lose green)
3. ✅ Only button B should be green

### Test Case 2: Stream Deck → Dashboard
1. Press button A on Stream Deck (should turn green)
2. Click play on FX B in dashboard (should turn B's button green, A should lose green)
3. ✅ Only button B should be green

### Test Case 3: Dashboard → Stream Deck
1. Click play on FX A in dashboard (A's button should turn green)
2. Press button B on Stream Deck (should turn green, A should lose green)
3. ✅ Only button B should be green

### Test Case 4: Dashboard → Dashboard
1. Click play on FX A (A's button should turn green)
2. Click play on FX B (B's button should turn green, A should lose green)
3. ✅ Only button B should be green

All test cases should now work correctly! 🎉

## Files Modified
- `battles.app/components/DashboardView.vue`:
  - Updated `playFxFile()` to call `streamDeck.setButtonState(stoppedFxKey, false)` for all stopped FX
  - Updated `playGlobalFx()` to call `streamDeck.setButtonState(String(playingId), false)` for all stopped FX

## No Restart Required
This is a frontend-only change, so just **refresh the page** in your browser!

