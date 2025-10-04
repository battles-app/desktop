# Quick GStreamer Installation Guide

## ⚠️ Current Status
Your PATH is configured for: `E:\gstreamer\1.0\msvc_x86_64\bin`
But GStreamer is **NOT YET INSTALLED** at that location.

## 📥 Download GStreamer

### Option 1: Direct Download Links (Recommended)
Download these two files:

**Runtime Installer:**
https://gstreamer.freedesktop.org/data/pkg/windows/1.24.8/msvc/gstreamer-1.0-msvc-x86_64-1.24.8.msi

**Development Installer:**
https://gstreamer.freedesktop.org/data/pkg/windows/1.24.8/msvc/gstreamer-1.0-devel-msvc-x86_64-1.24.8.msi

### Option 2: Official Website
Go to: https://gstreamer.freedesktop.org/download/

1. Scroll to **Windows binaries**
2. Find **MSVC 64-bit (VS 2019, x86_64)** section
3. Download both:
   - Runtime installer (e.g., `gstreamer-1.0-msvc-x86_64-1.24.8.msi`)
   - Development installer (e.g., `gstreamer-1.0-devel-msvc-x86_64-1.24.8.msi`)

## 🔧 Installation Steps

### 1. Install Runtime (First)
1. Double-click `gstreamer-1.0-msvc-x86_64-1.24.8.msi`
2. Click "Next"
3. **IMPORTANT:** Change installation path to: `E:\gstreamer\1.0\msvc_x86_64\`
4. Select **"Complete"** installation (not Typical)
5. Click "Next" and "Install"
6. Wait for installation to complete
7. Click "Finish"

### 2. Install Development Package (Second)
1. Double-click `gstreamer-1.0-devel-msvc-x86_64-1.24.8.msi`
2. Click "Next"
3. **Use the SAME path:** `E:\gstreamer\1.0\msvc_x86_64\`
4. Select **"Complete"** installation
5. Click "Next" and "Install"
6. Wait for installation to complete
7. Click "Finish"

## ✅ Verify Installation

After installation, open a **NEW PowerShell window** and run:

```powershell
# Method 1: Using full path
E:\gstreamer\1.0\msvc_x86_64\bin\gst-inspect-1.0.exe --version

# Method 2: Using PATH (after restarting PowerShell)
gst-inspect-1.0 --version
```

You should see:
```
gst-inspect-1.0 version 1.24.8
GStreamer 1.24.8
```

## 🔍 Test Camera Detection

```powershell
# Detect available cameras
E:\gstreamer\1.0\msvc_x86_64\bin\gst-device-monitor-1.0.exe

# Check for Windows camera source plugin
E:\gstreamer\1.0\msvc_x86_64\bin\gst-inspect-1.0.exe ksvideosrc
```

## 🎯 Environment Variables (Already Set)

You've already added to PATH:
✅ `E:\gstreamer\1.0\msvc_x86_64\bin`

Also add this **System Variable** (if not already set):
- Variable name: `GSTREAMER_1_0_ROOT_MSVC_X86_64`
- Variable value: `E:\gstreamer\1.0\msvc_x86_64\`

## 🚀 After Installation

Once GStreamer is installed, go back to your project:

```powershell
cd d:\Works\B4\Scripts\tiktok\battlesDesktop

# Run the verification script
.\test_gstreamer.ps1

# Build the Tauri app
cargo build --release

# Run the app
cargo tauri dev
```

## 📂 Expected Directory Structure

After installation, you should have:
```
E:\gstreamer\1.0\msvc_x86_64\
├── bin\                      (200+ executables and DLLs)
│   ├── gst-inspect-1.0.exe
│   ├── gst-launch-1.0.exe
│   ├── gst-device-monitor-1.0.exe
│   ├── gstreamer-1.0-0.dll
│   └── ... (many more DLLs)
├── include\                  (header files)
├── lib\                      (libraries)
├── libexec\
└── share\
```

## 🐛 Troubleshooting

### "Installation failed"
- Run the installer as Administrator
- Make sure you have enough space on E: drive (~500MB needed)

### "Still can't find gst-inspect-1.0"
- Close ALL PowerShell/Command Prompt windows
- Open a NEW PowerShell window
- The PATH changes require a fresh shell

### "Wrong architecture" error
- Make sure you downloaded the **x86_64** (64-bit) version
- NOT the x86 (32-bit) version

## 💡 Why E: Drive?
You chose E: drive instead of the default C: drive. That's fine! Just make sure:
- The E: drive has enough space (~500MB)
- The path is consistent in both installations (runtime + development)
- Your environment variables point to E: drive


