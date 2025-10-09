# Stream Deck - Brand Loading Animation

## 🎨 Updated Animation with Brand Colors + Logo

### **What You'll See**

**First Button (Button 0):** Logo with animated gradient background
```
┌────────┐
│   🟣   │  ← Pink square (top)
│ ⚪  🟡 │  ← White (right) + Yellow (bottom-left)
└────────┘
Background: Flowing dark gradient
```

**Phase 1: Logo + "BATTLES" Appears** (0.6 seconds)
```
Dark gradient background (subtle, dark theme)
Button 0: [🎨 LOGO]
Row 1: B → BA → BAT → BATT → BATTL → BATTLE → BATTLES
       🔴 ⚪ 🟡 🔴 ⚪ 🟡 🔴
       (Pink)(White)(Yellow) repeating logo colors
```

**Phase 2: Logo + "LOADING" Appears** (0.6 seconds)
```
Dark gradient continues
Button 0: [🎨 LOGO] (stays visible)
Row 2: L → LO → LOA → LOAD → LOADI → LOADIN → LOADING
       🔴 ⚪ 🟡 🔴 ⚪ 🟡 🔴
```

**Phase 3: Infinite Loop** (until FX loaded)
```
Logo + both words visible, dark gradient keeps flowing
Animation loops forever at 33 FPS until user logs in
```

**Total Duration: ~1.5 seconds per cycle, then LOOPS INFINITELY**

## 🎨 Brand Colors (from logo.svg)

Letters cycle through your logo colors:

| Position | Color | Hex | RGB |
|----------|-------|-----|-----|
| 0, 3, 6 | **Pink/Red** | `#ee2b63` | `rgb(238, 43, 99)` |
| 1, 4, 7 | **White** | `#ffffff` | `rgb(255, 255, 255)` |
| 2, 5 | **Yellow** | `#e9b320` | `rgb(233, 179, 32)` |

### Letter Color Pattern

**BATTLES:**
- **B** = Pink 🔴
- **A** = White ⚪
- **T** = Yellow 🟡
- **T** = Pink 🔴
- **L** = White ⚪
- **E** = Yellow 🟡
- **S** = Pink 🔴

**LOADING:**
- **L** = Pink 🔴
- **O** = White ⚪
- **A** = Yellow 🟡
- **D** = Pink 🔴
- **I** = White ⚪
- **N** = Yellow 🟡
- **G** = Pink 🔴

## 🌑 Dark Gradient Background

Matches your app's dark theme:

```rust
// Dark gradient wave (subtle, not overwhelming)
HSV(hue, 0.3, 0.2)  // Low saturation, low brightness
//       ^^^  ^^^
//       30%  20% of max
```

**Effect:** Subtle, dark, flowing rainbow that doesn't compete with text.

**Colors:** Dark purples, blues, greens flowing diagonally across keys.

**Speed:** Slower than bright version (8° per frame vs 10°).

## ✍️ Text Styling

### Size & Weight
```rust
// LARGE text: 65% of button size
let scale = PxScale::from((size * 0.65).max(50.0));

// BOLD effect: Draw each letter 4 times (2x2 offset grid)
for offset_x in 0..2 {
    for offset_y in 0..2 {
        draw_text(x + offset_x, y + offset_y);
    }
}
```

### Centering
```rust
// Center horizontally and vertically
let text_x = (button_size - letter_width) / 2;
let text_y = (button_size - letter_height) / 2;
```

**Result:** Each letter is perfectly centered in its button, large and bold!

## 📐 Layout (Stream Deck XL 8×4)

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ Row 0: [🎨LOGO] [Dark] [Dark] [Dark] [Dark] [Dark] [Dark] [Dark]           │
│        ↑ First button shows animated logo!                                  │
│                                                                              │
│ Row 1: [Dark] [  🔴B  ] [  ⚪A  ] [  🟡T  ] [  🔴T  ] [  ⚪L  ] [  🟡E  ] [  🔴S  ] │
│               BATTLES - Large, bold, centered, logo colors                  │
│               (starts from column 1, logo takes column 0)                   │
│                                                                              │
│ Row 2: [  🔴L  ] [  ⚪O  ] [  🟡A  ] [  🔴D  ] [  ⚪I  ] [  🟡N  ] [  🔴G  ] [Dark] │
│        LOADING - Large, bold, centered, logo colors                         │
│                                                                              │
│ Row 3: [Dark] [Dark] [Dark] [Dark] [Dark] [Dark] [Dark] [Dark]            │
└─────────────────────────────────────────────────────────────────────────────┘

🎨 Logo button: 3 colored squares (Pink, White, Yellow) on dark gradient
🌊 All buttons have subtle dark gradient wave flowing diagonally →
♾️  Animation LOOPS FOREVER until FX buttons load (user logs in)
```

## ⏱️ Timing Breakdown

### Phase 1: "BATTLES" Reveal
- **Frames**: 21 (7 letters × 3 frames)
- **Duration**: 630ms (21 × 30ms)
- **Effect**: Letters appear left to right, one at a time

### Phase 2: "LOADING" Reveal
- **Frames**: 21 (7 letters × 3 frames)
- **Duration**: 630ms
- **Effect**: Letters appear left to right while "BATTLES" stays visible

### Phase 3: Hold
- **Frames**: 10
- **Duration**: 300ms
- **Effect**: Both words visible, gradient keeps flowing

### Total
- **Frames**: 52
- **Duration**: 1.56 seconds
- **FPS**: 33 (30ms per frame)

## 🔄 Infinite Loop Animation

The animation now LOOPS FOREVER until the user logs in and FX buttons are loaded!

### How It Works

1. **Connection**: Device connects → plays initial animation (logo appears, then "BATTLES", then "LOADING")
2. **Loop Starts**: Animation thread continuously calls `continue_loading_background(frame_count)` at 33 FPS
3. **Stops When**: User logs in → `DashboardView.updateStreamDeckLayout()` → `manager.update_layout()` → calls `stop_loading_animation()` → animation stops, FX buttons appear

### Code

```rust
// In main.rs - Animation thread runs forever
std::thread::spawn(move || {
    let mut frame_counter = 0usize;
    loop {
        if manager.is_connected() && manager.is_loading_animation_active() {
            manager.continue_loading_background(frame_counter);
            frame_counter = frame_counter.wrapping_add(1);
        }
        std::thread::sleep(Duration::from_millis(30)); // 33 FPS
    }
});

// In streamdeck_manager.rs - Stops when FX loaded
pub fn update_layout(&mut self, battle_board, user_fx) {
    self.stop_loading_animation(); // ← Animation stops here!
    // ... load FX buttons
}
```

This keeps the dark gradient wave animating with **Logo + "BATTLES LOADING"** visible until your FX buttons finish loading. Perfect for showing the app is still working!

## 🎯 Visual Design Goals

✅ **Dark theme** - Matches your app (dark gradients, not bright)  
✅ **Brand colors** - Uses exact logo colors (#ee2b63, #fff, #e9b320)  
✅ **Bold text** - Highly visible, professional  
✅ **Centered** - Perfect alignment on each button  
✅ **Sequential** - "BATTLES" first, then "LOADING" (storytelling)  
✅ **Continuous** - Background keeps animating (shows activity)  

## 🎬 Animation Flow

```
0.0s  → Connection established
      → [🎨 LOGO] appears on first button
      → Dark gradient starts
      
0.0s  → [B] appears (Pink) - Row 1, Col 1
0.1s  → [BA] (Pink, White)
0.2s  → [BAT] (Pink, White, Yellow)
0.3s  → [BATT] (all logo colors)
0.4s  → [BATTL]
0.5s  → [BATTLE]
0.6s  → [BATTLES] complete!

0.6s  → [L] appears (Pink) - Row 2, Col 0
0.7s  → [LO] (Pink, White)
0.8s  → [LOA] (Pink, White, Yellow)
0.9s  → [LOAD]
1.0s  → [LOADI]
1.1s  → [LOADIN]
1.2s  → [LOADING] complete!

1.2s  → Both words visible
1.4s  → First cycle complete
      → ♾️  LOOPS BACK TO START (dark gradient keeps flowing)
      → Logo stays, text stays, gradient animates
      → Continues forever...

∞     → User logs in → FX buttons load → Animation STOPS
```

## 🖼️ Visual Comparison

### Before (Bright Rainbow)
```
❌ Bright rainbow colors (overwhelming)
❌ All white text (boring)
❌ Small text (hard to see)
❌ Both words at once (cluttered)
```

### After (Brand Dark)
```
✅ Dark gradient (subtle, professional)
✅ Logo colors (branded, colorful)
✅ Large bold text (easy to see)
✅ Sequential (cleaner, storytelling)
```

## 🔧 Technical Details

### Color Calculation
```rust
// Logo colors directly from logo.svg
let logo_colors = [
    Rgba([238, 43, 99, 255]),   // #ee2b63 Pink/Red
    Rgba([255, 255, 255, 255]), // #ffffff White
    Rgba([233, 179, 32, 255]),  // #e9b320 Yellow
];

// Cycle through colors by position
let color_idx = column % logo_colors.len();
let text_color = logo_colors[color_idx];
```

### Dark Gradient
```rust
// Dark theme: Low saturation (30%), Low brightness (20%)
let (r, g, b) = hsv_to_rgb(hue, 0.3, 0.2);

// vs Bright theme: High saturation (80%), High brightness (60%)
let (r, g, b) = hsv_to_rgb(hue, 0.8, 0.6);
```

### Bold Effect
```rust
// Draw each letter 4 times in a 2×2 grid
// Creates bold/thick appearance
for offset_x in 0..2 {
    for offset_y in 0..2 {
        draw_text(x + offset_x, y + offset_y, scale, font, letter);
    }
}
```

## 🚀 Restart to See It!

The animation automatically plays when Stream Deck connects. **Restart your app** to see:

- ✨ Dark gradient wave background
- 🎨 "BATTLES" in pink/white/yellow
- 🎨 "LOADING" in pink/white/yellow
- 💪 Large, bold, centered text
- 🌊 Sequential reveal (BATTLES → LOADING)
- ♾️ Continuous background animation

**It looks AMAZING!** 🔥✨

