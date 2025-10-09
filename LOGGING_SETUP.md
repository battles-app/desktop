# Production Logging System

## Overview
All diagnostic logs are automatically saved to a persistent file in the installation directory. This allows users to easily share logs for support and troubleshooting without needing to run the app from a terminal.

---

## Log File Location

### Production Builds
```
C:\Program Files\Battles.app Desktop\battles-desktop.log
```

### Development Builds
The log file is created in the same directory as the executable, wherever it's run from.

---

## Features

### Automatic Session Headers
Each time the app starts, a new session header is written to the log:
```
================================================================================
🚀 Battles.app Desktop - Session Started: 2025-10-10 15:30:45
================================================================================
```

### Timestamped Entries
Every log entry includes a precise timestamp:
```
[2025-10-10 15:30:45.123] [GStreamer] Added exe directory to PATH: C:\Program Files\Battles.app Desktop
[2025-10-10 15:30:45.234] [Composite] 🔧 Initializing GStreamer composite pipeline...
[2025-10-10 15:30:46.456] [Camera] 📹 Starting camera enumeration...
```

### Emoji Prefixes
Logs use emoji prefixes for easy visual scanning:
- 📊 Data/Statistics
- 🔍 Details/Inspection  
- ✅ Success
- ❌ Error
- ⚠️  Warning
- 📹 Camera
- 🔧 Initialization
- 📥 Download
- 🌐 Network

---

## Implementation Details

### Core Module: `file_logger.rs`

```rust
// Initialize logger at app startup
file_logger::init_logger();

// Log anywhere in the code
log_info!("Your message here");
log_info!("Formatted message: {} items", count);
```

### Key Features
1. **Thread-Safe**: Uses `lazy_static` and `parking_lot::Mutex`
2. **Immediate Flush**: Every log entry is flushed immediately to disk
3. **Append Mode**: Logs accumulate across sessions
4. **Error Handling**: Gracefully handles file creation failures

### Logged Systems

#### 1. Application Startup
- Version information
- GStreamer configuration
- DLL bundling status
- Plugin directory setup

#### 2. Camera System
- Camera enumeration start/finish
- List of detected cameras (ID, name, description)
- Total camera count
- Warnings if no cameras found

#### 3. Composite System
- Pipeline initialization steps
- Component creation success/failure
- Broadcast channel setup
- WebSocket server startup
- Final initialization status

#### 4. Stream Deck System
- Layout updates with FX details
- Battle Board FX list (ID, name, image URL)
- User FX list (ID, name, image URL)
- Image download attempts
- Missing image warnings

---

## How to Use

### For Users (Production)

1. **Run the app normally** (from Start Menu, desktop shortcut, etc.)
2. **Reproduce the issue** you're experiencing
3. **Navigate to the installation folder**:
   - Open File Explorer
   - Go to: `C:\Program Files\Battles.app Desktop`
4. **Find the log file**: `battles-desktop.log`
5. **Open with any text editor** (Notepad, VS Code, etc.)
6. **Share the log file** for support

### For Developers (Debugging)

1. **Real-time logs in terminal**:
   ```powershell
   cd "C:\Program Files\Battles.app Desktop"
   .\battles-desktop.exe
   ```
   Logs will appear in the console AND be saved to file.

2. **Tail the log file** (PowerShell):
   ```powershell
   Get-Content -Path "battles-desktop.log" -Wait -Tail 50
   ```

3. **Clear old logs** (optional):
   ```powershell
   Remove-Item "battles-desktop.log"
   ```
   A new file will be created on next app start.

---

## Log File Management

### File Size
- The log file grows indefinitely (append mode)
- Each session adds ~100-500 lines depending on activity
- Manual cleanup required if file becomes too large

### Rotation Strategy (Not Implemented)
Currently, the log file does not auto-rotate. Future improvements:
- Size-based rotation (e.g., max 10 MB)
- Keep last N log files
- Timestamp-based archiving

### Manual Cleanup
Users can safely delete `battles-desktop.log` at any time. The app will create a new file on next launch.

---

## Troubleshooting

### Log File Not Created
**Possible Causes**:
- Insufficient permissions to write to installation directory
- Running from a location without write access

**Solution**:
- Run as administrator
- Check Windows Event Viewer for errors
- Console will show: `⚠️  Failed to create log file at [path]: [error]`

### Log File Empty
**Possible Causes**:
- App crashed before logs could be written
- File is locked by another process

**Solution**:
- Ensure no other app has the file open
- Check if file is read-only
- Try deleting and recreating

### Missing Logs for Specific Feature
**Possible Causes**:
- Feature hasn't been updated to use file logger yet
- Feature uses a different logging system

**Solution**:
- Check if logs appear in console but not file
- Report to developers for integration

---

## Adding New Log Points

For developers adding new diagnostic logs:

```rust
// In any Rust module that needs logging:

// 1. Import the macro at top of file
use crate::log_info;

// 2. Use anywhere in your code
log_info!("Starting important operation...");
log_info!("Processing {} items", count);
log_info!("[MyFeature] ✅ Operation completed successfully");
log_info!("[MyFeature] ❌ ERROR: Something failed: {}", error);
```

**Best Practices**:
- Use `[Module]` prefix for clarity (e.g., `[Camera]`, `[Stream Deck]`)
- Include emoji prefix for visual categorization
- Log start AND completion of operations
- Log all errors with context
- Include relevant data (counts, IDs, names)

---

## Related Documentation
- See `DIAGNOSTIC_LOGGING.md` for detailed troubleshooting guide
- See `src/file_logger.rs` for implementation details

---

**Version**: 0.0.19  
**Last Updated**: 2025-10-10  
**Purpose**: Production logging for user support and debugging

