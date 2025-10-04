# Why build.ps1 instead of bun run dev?

## Two Different Apps

You have **TWO separate applications** in your project:

### 1. `battles.app` (Nuxt Frontend)
**Location**: `D:\Works\B4\Scripts\tiktok\battles.app\`
**Technology**: Vue 3 + Nuxt 3 (JavaScript/TypeScript)
**Command**: `bun run dev`
**Purpose**: Web frontend, runs in browser

### 2. `battlesDesktop` (Tauri Desktop App)
**Location**: `D:\Works\B4\Scripts\tiktok\battlesDesktop\`
**Technology**: Rust + Tauri + Nuxt embedded
**Command**: `.\build.ps1 dev` or `cargo tauri dev`
**Purpose**: Desktop app with native features (camera, system access)

## Why build.ps1 for battlesDesktop?

The `build.ps1` script is **only for the Rust/Tauri desktop app** because:

1. **GStreamer Environment Variables**
   ```powershell
   $env:PKG_CONFIG_PATH = "E:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
   $env:PATH = "E:\gstreamer\1.0\msvc_x86_64\bin;$env:PATH"
   ```
   Rust needs these to find GStreamer libraries during compilation.

2. **Rust Compilation**
   The desktop app is written in Rust, which needs `cargo` to build, not `bun`.

3. **Different Build System**
   - `battles.app` uses Bun/Vite (JavaScript bundler)
   - `battlesDesktop` uses Cargo (Rust compiler) + Tauri

## What Command Should You Use?

### For Web Development (Nuxt app):
```powershell
cd D:\Works\B4\Scripts\tiktok\battles.app
bun run dev
```
This runs the Nuxt app in your browser.

### For Desktop App Development (with camera):
```powershell
cd D:\Works\B4\Scripts\tiktok\battlesDesktop
.\build.ps1 dev
```
This compiles the Rust backend AND runs the Nuxt frontend inside the desktop window.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  battlesDesktop (Desktop App)                               │
│  ┌───────────────────────────────────────────────────────┐  │
│  │  Tauri Window                                         │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │  battles.app (Nuxt/Vue)                         │  │  │
│  │  │  Your frontend UI                               │  │  │
│  │  │                                                  │  │  │
│  │  │  Calls: invoke('start_camera_preview')          │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  │                          ↕                             │  │
│  │  ┌─────────────────────────────────────────────────┐  │  │
│  │  │  Rust Backend (GStreamer)                       │  │  │
│  │  │  - Camera capture                                │  │  │
│  │  │  - WebSocket server                              │  │  │
│  │  │  - Native system access                          │  │  │
│  │  └─────────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Can I Use Regular Commands?

### Option 1: Use build.ps1 (Recommended)
```powershell
.\build.ps1 dev    # Development
.\build.ps1 build  # Production
```

### Option 2: Set Environment Variables Permanently
```powershell
# Run ONCE as Administrator
.\set_gstreamer_env.ps1

# Then you can use normal commands
cargo tauri dev
cargo tauri build
```

### Option 3: Manual (Not Recommended)
```powershell
$env:PKG_CONFIG_PATH = "E:\gstreamer\1.0\msvc_x86_64\lib\pkgconfig"
$env:PATH = "E:\gstreamer\1.0\msvc_x86_64\bin;$env:PATH"
cargo tauri dev
```

## TL;DR

- **Web app** (`battles.app`): Use `bun run dev` ✅
- **Desktop app** (`battlesDesktop`): Use `.\build.ps1 dev` ✅
- They're two different things with different build systems!


