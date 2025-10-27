use lazy_static::lazy_static;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use chrono::Local;

lazy_static! {
    static ref LOG_FILE: Mutex<Option<File>> = Mutex::new(None);
}

/// Initialize the file logger in AppData\Local
pub fn init_logger() {
    // Determine log directory based on OS
    let log_dir = if cfg!(target_os = "windows") {
        // Use %LOCALAPPDATA%\BattlesDesktop\ on Windows
        match std::env::var("LOCALAPPDATA") {
            Ok(local_app_data) => {
                PathBuf::from(local_app_data).join("BattlesDesktop")
            }
            Err(_) => {
                // Fallback to executable directory if LOCALAPPDATA not available
                let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
                exe_path.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf()
            }
        }
    } else {
        // For non-Windows, use executable directory
        let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        exe_path.parent().unwrap_or_else(|| std::path::Path::new(".")).to_path_buf()
    };
    
    // Ensure the log directory exists
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        return;
    }
    
    let log_file_path = log_dir.join("battles-desktop.log");
    
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
    {
        Ok(mut file) => {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let separator = "=".repeat(80);
            let header = format!("\n\n{}\n🚀 BattlesDesktop - Session Started: {}\n{}\n", separator, timestamp, separator);
            let _ = file.write_all(header.as_bytes());
            let _ = file.flush();
            
            *LOG_FILE.lock() = Some(file);
            
            // Print to console where logs are being saved
        }
        Err(e) => {
        }
    }
}

/// Log a message to both console and file
pub fn log(message: &str) {
    // Print to console
    
    // Write to file with timestamp (buffered - no immediate flush for performance)
    if let Some(ref mut file) = *LOG_FILE.lock() {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_line = format!("[{}] {}\n", timestamp, message);
        let _ = file.write_all(log_line.as_bytes());
        // REMOVED: Immediate flush - let OS buffer handle it for performance
        // Logs will still be written, just batched by the OS (much faster)
    }
}

/// Macro for easy logging that goes to both console and file
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        {
            let msg = format!($($arg)*);
            $crate::file_logger::
        }
    };
}

