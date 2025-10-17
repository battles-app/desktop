# DMX Hardware Requirements

## Administrator Privileges Required

BattlesDesktop requires **Administrator privileges** to access USB/HID hardware devices including:

- **Enttec USB Pro / Pro Mk2** (DMX controllers)
- **Elgato Stream Deck** (control surface)
- **DMXIS** (USB DMX interface)
- Other USB serial/COM port devices

### Why Administrator Access?

Windows restricts direct access to USB serial ports and HID devices to protect system security. DMX controllers and Stream Decks communicate via these protected interfaces.

## How to Run as Administrator

### Option 1: Right-Click Menu (Recommended)
1. Right-click `BattlesDesktop.exe` or the desktop shortcut
2. Select **"Run as administrator"**
3. Click **Yes** on the UAC prompt

### Option 2: Always Run as Administrator
1. Right-click `BattlesDesktop.exe`
2. Select **Properties**
3. Go to **Compatibility** tab
4. Check **"Run this program as administrator"**
5. Click **OK**

## Troubleshooting

### Error: "Access is denied"
**Cause:** Application doesn't have USB/serial port permissions

**Solution:**
1. Close BattlesDesktop
2. Right-click and select "Run as administrator"
3. If error persists, check that no other DMX software is using the device

### Error: "Device not found"
**Cause:** Missing or incorrect USB drivers

**Solution:**
1. Install **FTDI VCP drivers** from: https://ftdichip.com/drivers/vcp-drivers/
2. For Enttec devices, install **Enttec USB drivers** from: https://www.enttec.com/product/controls/dmx-usb-interfaces/
3. Restart computer after driver installation
4. Run BattlesDesktop as administrator

### Device Already in Use
**Cause:** Another application is using the DMX device

**Solution:**
1. Close other DMX software (LightKey, QLC+, etc.)
2. Unplug and replug the USB device
3. Run BattlesDesktop as administrator

## Network Protocols (No Admin Required)

These protocols work **without** administrator privileges:
- **Art-Net** (network DMX)
- **sACN** (streaming ACN)
- **Enttec ODE** (via Ethernet)

## Logs

Check detailed error logs at:
```
C:\Users\<YourUsername>\AppData\Local\BattlesDesktop\battles-desktop.log
```

## Security Note

The application manifest requests `requireAdministrator` specifically for hardware access. This is standard practice for professional lighting and control software that interfaces with USB/serial devices.


