# Battles.app Desktop Installer Branding

## Enhanced Windows Installer Customization

### ✅ Completed Customizations

#### Product Information
- **Product Name**: "Battles.app Desktop" (updated from "battles.app")
- **App Identifier**: `app.battles.desktop` (changed from `io.battles.app` to avoid macOS conflicts)
- **Copyright**: "© 2025 BATTLES.app™. All rights reserved."
- **Category**: Utility
- **License**: BSL-1.1
- **Repository**: https://github.com/battles-app/desktop
- **Homepage**: https://battles.app

#### NSIS Installer Features
- **Product Name**: "Battles.app Desktop" (shown in installer title and shortcuts)
- **Installer Filename**: `Battles.app Desktop_X.X.X_x64-setup.exe`
- **Install Mode**: Default (Tauri managed)
- **Language**: English
- **Icon**: Uses app icon from bundle configuration
- **Compression**: Default (Tauri managed)

#### Installer Branding
The installer displays:
- Professional product name: "Battles.app Desktop"
- Proper copyright: "© 2025 BATTLES.app™. All rights reserved."
- Category: Utility
- Complete descriptions and metadata
- Desktop and Start Menu shortcuts with proper branding

#### Product Descriptions
- **Short**: "Pro TikTok Live Tools with Stream Deck"
- **Long**: "Professional TikTok Live streaming tools with Elgato Stream Deck integration. Real-time FX control, GPU-accelerated chroma key, and professional broadcast features for content creators."

### 🎨 Brand Colors Reference
From `tailwind.config.ts`:
- **Background**: #0b0f1a (dark navy)
- **Panel**: #101726 (lighter dark blue)
- **Neon**: #00f3ff (cyan/aqua)
- **Neon2**: #ff00e6 (magenta/pink)
- **Gold**: #ffd166 (gold)

### 📦 Installer Assets Structure
```
battlesDesktop/
├── icons/
│   ├── icon.ico (installer icon)
│   ├── icon.png (app icon)
│   └── ... (various sizes)
├── installer-assets/
│   └── installer.nsi (custom NSIS template - future use)
└── LICENSE (included in installer)
```

### 🚀 Build Commands

#### Development Build
```bash
bun run tauri build
```

#### Production Release (with signing & updates)
```bash
bun run release
```

### 📋 Installer Features
- ✅ Custom product branding
- ✅ Professional welcome/finish messages
- ✅ Emoji-enhanced feature callouts
- ✅ Desktop shortcut creation
- ✅ Start menu shortcut
- ✅ Uninstaller with proper registry cleanup
- ✅ GStreamer DLL bundling for standalone operation
- ✅ Webview bootstrapper for optimal size
- ✅ LZMA compression for smaller download
- ✅ License agreement display
- ✅ Professional product metadata

### 🎯 Future Enhancements (Optional)
- Custom installer banner images (164x314 px)
- Custom sidebar images (150x57 px)
- Custom installer background color
- Multi-language support
- Custom uninstaller messages
- Install location customization UI

### 📝 Notes
- The installer will be named: `battles.app_X.X.X_x64-setup.exe`
- All branding is consistent across installer, application, and documentation
- Webview runtime is downloaded during installation if needed
- GStreamer dependencies are bundled for offline operation

