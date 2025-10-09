# Stream Deck XL Layout Update

## New Layout Configuration

### Stream Deck XL (8 columns × 4 rows = 32 buttons):

```
┌────┬────┬────┬────┬────┬────┬────┬──────┐
│ B1 │ B2 │ B3 │ B4 │ B5 │ F1 │ F5 │INTRO │
├────┼────┼────┼────┼────┼────┼────┼──────┤
│ B6 │ B7 │ B8 │ B9 │B10 │ F2 │ F6 │PARTY │
├────┼────┼────┼────┼────┼────┼────┼──────┤
│B11 │B12 │B13 │B14 │B15 │ F3 │ F7 │BREAK │
├────┼────┼────┼────┼────┼────┼────┼──────┤
│B16 │B17 │B18 │B19 │B20 │ F4 │ F8 │ END  │
└────┴────┴────┴────┴────┴────┴────┴──────┘

B = Battle Board (Global FX)
F = User FX (Sound & Video Board)
```

### Column Layout:

- **Columns 0-4** (5 columns): Battle Board - Up to 20 global FX
- **Columns 5-6** (2 columns): User FX - Up to 8 buttons (limited to 12 total)
- **Column 7** (rightmost): Control Buttons (4 buttons)

### Control Buttons (Column 7):

1. **INTRO** (Row 0)
   - Color: Purple `rgb(138, 43, 226)`
   - ID: `control_intro`
   - Status: Unassigned (reserved for future use)

2. **PARTY** (Row 1)
   - Color: Hot Pink `rgb(255, 105, 180)`
   - ID: `control_party`
   - Status: Unassigned (reserved for future use)

3. **BREAK** (Row 2)
   - Color: Blue `rgb(30, 144, 255)`
   - ID: `control_break`
   - Status: Unassigned (reserved for future use)

4. **END** (Row 3)
   - Color: Crimson `rgb(220, 20, 60)`
   - ID: `control_end`
   - Status: Unassigned (reserved for future use)

## Visual Differences

### Control Buttons:
- ✅ **Solid colored backgrounds** (no images)
- ✅ **Large centered text** (18% of button size)
- ✅ **No borders** (clean solid color)
- ✅ **White text** on colored background

### FX Buttons (Battle Board & User FX):
- ✅ **Images from browser cache** (or colored backgrounds if no image)
- ✅ **Text bar at bottom** (semi-transparent black, 25% height)
- ✅ **Small text** (13% of button size, truncated if > 10 chars)
- ✅ **Colored borders**:
  - Purple border for battle board (global FX)
  - Blue border for user FX
  - **Thick green border when playing**

## Implementation Details

### Layout Code (`update_layout`):
```rust
// For XL devices, special layout with control buttons
if matches!(self.device_kind, Some(Kind::Xl) | Some(Kind::XlV2)) {
    // Place battle board (columns 0-4, up to 20 buttons)
    // Place user FX (columns 5-6, limit to 12 buttons)
    // Place control buttons (column 7, 4 buttons)
}
```

### Control Button Rendering (`create_button_image`):
```rust
// Control buttons have specific colors
if fx_button.id.starts_with("control_") {
    match fx_button.name.as_str() {
        "INTRO" => Rgba([138, 43, 226, 255]), // Purple
        "PARTY" => Rgba([255, 105, 180, 255]), // Hot pink
        "BREAK" => Rgba([30, 144, 255, 255]),  // Blue
        "END" => Rgba([220, 20, 60, 255]),     // Crimson
    }
}
```

### Control Button Text:
```rust
if fx_button.id.starts_with("control_") {
    // Large centered text (18% font size)
    // Center both horizontally and vertically
    // No text background bar
}
```

## Benefits

### ✅ More Organized Layout
- Battle board has dedicated left side (20 slots)
- User FX compact on right-center (8 visible slots)
- Control buttons clearly separated on far right

### ✅ Future Extensibility
- Control buttons ready for scene management
- Easy to add functionality later
- Clear visual separation between FX and controls

### ✅ Better User Experience
- Quick access to scene controls on the right
- More space for battle board effects
- Clear color coding for different button types

## Non-XL Devices

For Stream Deck Original, Mini, and other models:
- **Standard split layout** (50/50 left-right)
- **No control buttons** (not enough space)
- Battle board on left, user FX on right

## Testing

```powershell
bun run tauri dev
```

### Expected Behavior:

**Column 7 (rightmost) should show:**
```
┌──────┐
│INTRO │ ← Purple
├──────┤
│PARTY │ ← Hot pink
├──────┤
│BREAK │ ← Blue
├──────┤
│ END  │ ← Crimson
└──────┘
```

**Columns 5-6 should show:**
- First 8 user FX (limited to 12 total)
- Images + text at bottom
- Blue borders

**Columns 0-4 should show:**
- Battle board global FX
- Images + text at bottom
- Purple borders

## Future Enhancements

Control buttons are ready for:
- Scene transitions (INTRO starts show)
- Party mode activation
- Break/intermission screens
- End of show sequences
- Custom event triggers
- Macro functionality

## Summary

✅ **Reorganized layout** for Stream Deck XL  
✅ **4 control buttons** on the right (INTRO, PARTY, BREAK, END)  
✅ **User FX limited to 12** (displayed in columns 5-6)  
✅ **Battle board** uses left 5 columns (up to 20 buttons)  
✅ **Color-coded** control buttons for easy identification  
✅ **Large centered text** on control buttons  
✅ **Future-ready** for scene management features  

**Your Stream Deck XL now has a professional control panel layout!** 🎛️✨

