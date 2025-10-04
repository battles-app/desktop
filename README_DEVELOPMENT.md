# Development Setup - Battles Desktop

## 🚀 Quick Start

```bash
# Development (hot reload)
bun run dev

# Production build
bun run build

# Check compilation only
bun run check
```

That's it! No special scripts needed.

---

## ❓ Differences: `tauri dev` vs `cargo tauri dev`

### `tauri dev` (via bun/npm) ✅ **Recommended**
- Uses **local** `@tauri-apps/cli` from `node_modules`
- Version controlled in `package.json`
- Team uses same version
- Standard for Node.js/Tauri projects
- **Run via:** `bun run dev`

### `cargo tauri dev` ⚠️
- Uses **global** Tauri CLI installed system-wide
- Version might differ between developers
- Requires: `cargo install tauri-cli`
- Less common in modern Tauri projects

### TL;DR
Use `bun run dev` - it calls `tauri dev` from your local node_modules! ✅

---

## 🔧 How Environment Variables Work

### For Development (Automatic!)

The `.cargo/config.toml` file automatically sets GStreamer env vars:

```toml
# .cargo/config.toml
[env]
PKG_CONFIG_PATH = "E:\\gstreamer\\1.0\\msvc_x86_64\\lib\\pkgconfig"
GSTREAMER_1_0_ROOT_MSVC_X86_64 = "E:\\gstreamer\\1.0\\msvc_x86_64\\"
```

This means **all cargo commands** (including those called by Tauri) get these variables automatically.

No scripts needed! ✅

---

## 📦 For Production (Clients)

### What Gets Shipped

When you run `bun run build`, Tauri creates:
```
target/release/
├── battles-desktop.exe        ← Main executable
└── [bundled installer/msi]
```

### Client Requirements

**ZERO setup required!** 🎉

- ❌ No GStreamer installation needed
- ❌ No environment variables needed
- ❌ No PATH configuration needed
- ✅ Just run the `.exe` or install the `.msi`

### How It Works

Tauri automatically:
1. Bundles GStreamer DLLs with your app
2. Includes them in the installer/portable exe
3. Loads them at runtime from the app directory

Your client literally just **double-clicks the exe** and it works!

---

## 🛠️ Development Requirements

### One-Time Setup

1. **Install GStreamer** (for development only)
   - Download from: https://gstreamer.freedesktop.org/download/
   - Install to: `E:\gstreamer\1.0\msvc_x86_64\`
   - See `INSTALL_GSTREAMER.md` for details

2. **That's it!** The `.cargo/config.toml` handles the rest.

### Verify Setup

```bash
# Should work without errors
bun run check

# Should compile and run
bun run dev
```

---

## 📁 Project Structure

```
battlesDesktop/
├── .cargo/
│   └── config.toml          ← Auto-sets env vars for cargo
├── src/
│   ├── main.rs              ← Tauri backend (Rust)
│   └── gstreamer_camera.rs  ← Camera implementation
├── package.json             ← Scripts: dev, build, check
├── Cargo.toml               ← Rust dependencies
└── tauri.conf.json          ← Tauri configuration
```

---

## 🔄 Workflow

### Day-to-Day Development

```bash
# Start dev server (auto-reload)
bun run dev

# Make changes to:
# - src/*.rs (Rust backend)
# - ../battles.app/* (Nuxt frontend)

# Save files → Auto recompiles & reloads
```

### Building for Production

```bash
# Create optimized build
bun run build

# Find output in:
# target/release/battles-desktop.exe
# target/release/bundle/msi/...
```

### Quick Checks

```bash
# Check if code compiles (fast)
bun run check

# Full build without running
cargo build --release
```

---

## 🎯 Common Commands

| Command | What It Does | Speed |
|---------|-------------|-------|
| `bun run dev` | Start dev server with hot reload | Slow first time, fast after |
| `bun run build` | Build production exe/installer | Slow (~2-5 min) |
| `bun run check` | Check if code compiles | Fast (~1-3 sec) |
| `cargo clean` | Clean build artifacts | N/A |

---

## 🐛 Troubleshooting

### "pkg-config not found" or "GStreamer not found"

**Cause**: GStreamer not installed or `.cargo/config.toml` has wrong path

**Fix**:
1. Verify GStreamer installed at: `E:\gstreamer\1.0\msvc_x86_64\`
2. Check `.cargo/config.toml` paths are correct
3. If you moved GStreamer, update paths in `.cargo/config.toml`

### "Command 'tauri' not found"

**Cause**: Missing `@tauri-apps/cli` dependency

**Fix**:
```bash
bun install
```

### "Camera doesn't work in dev but works in production"

**Cause**: Missing GStreamer DLLs in PATH for dev runtime

**Fix**: Run this ONCE as admin:
```bash
.\setup_permanent_env.ps1
```
Then restart terminal.

---

## 💡 Pro Tips

1. **First build is slow** (~2-5 min) - Rust compiles everything. Subsequent builds are much faster!

2. **Use `bun run check`** frequently - It's fast and catches compilation errors without full rebuild.

3. **Dev server auto-reloads** - Save Rust files and it recompiles automatically.

4. **Frontend is separate** - The Nuxt app (`battles.app`) runs embedded in Tauri window.

5. **Production builds are optimized** - They're much faster than dev builds.

---

## 🎊 Summary

### For Development
```bash
bun run dev     # That's it!
```

### For Production
```bash
bun run build   # Creates exe with everything bundled
```

### For Clients
```bash
[No setup needed - just run the exe!]
```

**No build.ps1 needed!** ✅  
**No manual env vars needed!** ✅  
**No complicated setup!** ✅

Everything is handled automatically by:
- `.cargo/config.toml` (dev compilation)
- Tauri bundler (production distribution)

Happy coding! 🚀





