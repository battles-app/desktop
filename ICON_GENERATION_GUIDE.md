# 🎨 Icon Generation Guide for Battles.app

## Quick Steps

### 1. Generate PNG Files
Open `generate-icons.html` in your browser and download all PNG sizes.

### 2. Convert to ICO (Windows)
- Visit [RedKetchup ICO Converter](https://redketchup.io/icon-converter)
- Upload the largest PNG (1024x1024)
- Download as `battles-app-icon.ico`
- Replace `battlesDesktop/favicon.ico` with this file

### 3. Convert to ICNS (macOS)
- Visit [RedKetchup ICNS Converter](https://redketchup.io/icon-converter)
- Upload the largest PNG (1024x1024)
- Download as `icon.icns`
- Save to `battlesDesktop/icon.icns`

### 4. Update Tauri Config
Update `battlesDesktop/tauri.conf.json`:

```json
"icon": [
  "favicon.ico",
  "icon.icns",
  "logo.svg"
]
```

## Alternative: Use Online Tool
Visit [icon.kitchen](https://icon.kitchen/) and upload the `logo.svg` file to generate all required formats automatically.

## Icon Sizes Required

| Platform | Format | Sizes |
|----------|--------|-------|
| Windows | ICO | 16, 32, 48, 64, 128, 256 |
| macOS | ICNS | 16, 32, 64, 128, 256, 512, 1024 |
| Linux | PNG | 32, 128, 256, 512 |

## Current Files
- ✅ `logo.svg` - Source logo (all platforms)
- ✅ `loading.html` - Animated loading screen
- 🔄 `favicon.ico` - Windows icon (needs update)
- ❌ `icon.icns` - macOS icon (needs creation)

