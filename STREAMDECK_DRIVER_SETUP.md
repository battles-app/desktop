# Stream Deck Driver Setup & Troubleshooting

## Quick Diagnostic Check

The app now includes built-in diagnostics to help you troubleshoot Stream Deck connection issues.

### Run Diagnostics from Code

```typescript
import { invoke } from '@tauri-apps/api/tauri';

// Run diagnostics
const diagnostics = await invoke('streamdeck_run_diagnostics');
console.log(diagnostics);

// Get driver download info
const driverInfo = await invoke('streamdeck_get_driver_info');
console.log(driverInfo);
```

### Diagnostic Information Provided

The diagnostics will tell you:
- ✅ Whether HID API initialized successfully
- ✅ How many Stream Deck devices were found
- ✅ Device details (model, serial number, VID/PID)
- ✅ Driver status
- ✅ Step-by-step recommendations to fix issues

## Windows Setup

### Option 1: Built-in Windows HID Drivers (Recommended for Development)

On Windows 10/11, Stream Deck devices should work automatically with built-in HID drivers. No additional software needed!

**If your device isn't detected:**

1. **Check Device Manager**
   - Press `Win + X` → Device Manager
   - Look under "Human Interface Devices"
   - You should see entries for your Stream Deck
   - If you see yellow warning icons, drivers may need updating

2. **Update Windows**
   ```
   Settings → Windows Update → Check for updates
   ```
   This ensures you have the latest HID drivers.

3. **Try Different USB Port/Cable**
   - Use a USB 3.0 port (blue port) if available
   - Try a different high-quality USB cable
   - Avoid USB hubs if possible

### Option 2: Official Elgato Software (Includes Drivers)

If built-in drivers don't work, install the official software:

1. **Download Elgato Stream Deck Software**
   - Visit: https://www.elgato.com/downloads
   - Download "Stream Deck" software for Windows
   - Version 6.0+ recommended

2. **Install**
   - Run the installer
   - The installer will automatically install necessary drivers
   - **Important:** Restart your computer after installation

3. **Verify Installation**
   - Plug in your Stream Deck
   - Open the Elgato Stream Deck software
   - If it detects your device, drivers are working

4. **Use with Battles.app**
   - You can now close the Elgato software
   - Battles.app will use the installed drivers
   - Both apps can coexist (but don't run simultaneously)

## macOS Setup

macOS includes HID support by default, but you may need the official software for best results.

1. **Download Stream Deck Software**
   - Visit: https://www.elgato.com/downloads
   - Download "Stream Deck" for macOS

2. **Grant Permissions**
   - During first launch, macOS will ask for permissions
   - Grant "Input Monitoring" permission
   - This allows the app to communicate with the Stream Deck

## Linux Setup

Linux requires udev rules to grant user access to the Stream Deck.

### 1. Copy udev Rules

```bash
# Copy the included rules file
sudo cp 40-streamdeck.rules /etc/udev/rules.d/

# Or create it manually:
sudo nano /etc/udev/rules.d/40-streamdeck.rules
```

Paste the contents from the included `40-streamdeck.rules` file.

### 2. Reload udev Rules

```bash
sudo udevadm control --reload-rules
sudo udevadm trigger
```

### 3. Add User to plugdev Group

```bash
# Create group if it doesn't exist
sudo groupadd plugdev

# Add yourself to the group
sudo usermod -aG plugdev $USER
```

### 4. Restart

Log out and log back in (or restart) for group changes to take effect.

### 5. Verify

```bash
# Unplug and replug your Stream Deck
# Check if it's visible
lsusb | grep 0fd9
```

You should see your Stream Deck listed.

## Troubleshooting Guide

### Device Not Detected

**Run the diagnostic command first:**

```typescript
const diagnostics = await invoke('streamdeck_run_diagnostics');
console.log('Devices found:', diagnostics.devices_found);
console.log('Recommendations:', diagnostics.recommendations);
```

**Common Issues:**

1. **USB Connection**
   - ❌ Bad cable
   - ❌ USB hub with insufficient power
   - ❌ Damaged USB port
   - ✅ Try different cable
   - ✅ Use direct USB 3.0 port
   - ✅ Test on different computer to rule out hardware

2. **Driver Issues (Windows)**
   - ❌ Outdated Windows
   - ❌ Missing HID drivers
   - ✅ Run Windows Update
   - ✅ Install official Elgato software
   - ✅ Restart after driver installation

3. **Permission Issues (Linux)**
   - ❌ Missing udev rules
   - ❌ User not in plugdev group
   - ✅ Follow Linux setup steps above
   - ✅ Restart after setup

4. **Conflicting Software**
   - ❌ Another app is using the device
   - ✅ Close Elgato Stream Deck software
   - ✅ Close other Stream Deck apps
   - ✅ Restart computer

### "HidAPI Failed to Initialize"

This is a rare error that usually indicates system driver problems.

**Windows:**
```
1. Run Windows Update
2. Install official Elgato software
3. Restart computer
4. Check Device Manager for any driver warnings
```

**macOS:**
```
1. Grant Input Monitoring permission
2. Install official Elgato software
3. Restart computer
```

**Linux:**
```
1. Install hidapi library:
   sudo apt install libhidapi-libusb0  # Debian/Ubuntu
   sudo dnf install hidapi             # Fedora
   sudo pacman -S hidapi               # Arch
2. Setup udev rules (see above)
3. Restart computer
```

### Device Detected But Buttons Don't Work

If diagnostics show the device but buttons don't respond:

1. **Firmware Update**
   - Install official Elgato software
   - It will prompt for firmware update if needed
   - Update firmware
   - Restart device

2. **Button Press Detection**
   - Note: Physical button press detection is not yet implemented
   - Current version is display-only (shows FX status)
   - Buttons will light up when you play FX from dashboard
   - Full button input coming in future update

## Testing Your Setup

### Quick Test

1. Run the app
2. Open the console/terminal
3. Look for Stream Deck messages:
   ```
   [Stream Deck] Initializing...
   [Stream Deck] Scanning for devices...
   [Stream Deck] Found X devices
   [Stream Deck] ✅ Connected to Stream Deck Original (Serial: ABC123)
   ```

### Visual Test

If connected successfully:
- Stream Deck buttons should light up
- Buttons should show colored squares (purple/blue/green)
- When you play FX from dashboard, corresponding button should turn green

## Getting Help

If you're still having issues after trying the above:

1. **Run Full Diagnostics**
   ```typescript
   const diagnostics = await invoke('streamdeck_run_diagnostics');
   const driverInfo = await invoke('streamdeck_get_driver_info');
   
   console.log('=== DIAGNOSTICS ===');
   console.log(JSON.stringify(diagnostics, null, 2));
   console.log('=== DRIVER INFO ===');
   console.log(JSON.stringify(driverInfo, null, 2));
   ```

2. **Check Console Logs**
   - Look for error messages in the terminal
   - Share the full output when asking for help

3. **System Information**
   - OS version
   - Stream Deck model
   - USB connection type
   - Any error messages

## Driver Download Links

### Official Elgato Software
- **Windows:** https://www.elgato.com/downloads
- **macOS:** https://www.elgato.com/downloads
- **Note:** Includes all necessary drivers

### Linux Libraries (if needed)
- **Debian/Ubuntu:** `sudo apt install libhidapi-libusb0 libhidapi-hidraw0`
- **Fedora:** `sudo dnf install hidapi`
- **Arch:** `sudo pacman -S hidapi`

## Summary

✅ **Windows:** Usually works out-of-the-box. Install official software if needed.
✅ **macOS:** May need official software for permissions.
✅ **Linux:** Requires udev rules (provided in repo).

The built-in diagnostics will guide you through any issues! 🎮

