use lazy_static::lazy_static;
use parking_lot::Mutex;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use chrono::Local;

lazy_static! {
    static ref LOG_FILE: Mutex<Option<File>> = Mutex::new(None);
}

/// Initialize the file logger in the executable's directory
pub fn init_logger() {
    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let exe_dir = exe_path.parent().unwrap_or_else(|| std::path::Path::new("."));
    
    let log_file_path = exe_dir.join("battles-desktop.log");
    
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
    {
        Ok(mut file) => {
            let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
            let separator = "=".repeat(80);
            let header = format!("\n\n{}\n🚀 Battles.app Desktop - Session Started: {}\n{}\n", separator, timestamp, separator);
            let _ = file.write_all(header.as_bytes());
            let _ = file.flush();
            
            *LOG_FILE.lock() = Some(file);
            
            // Print to console where logs are being saved
            println!("📝 Logging to: {}", log_file_path.display());
        }
        Err(e) => {
            eprintln!("⚠️  Failed to create log file at {}: {}", log_file_path.display(), e);
        }
    }
}

/// Log a message to both console and file
pub fn log(message: &str) {
    // Print to console
    println!("{}", message);
    
    // Write to file with timestamp
    if let Some(ref mut file) = *LOG_FILE.lock() {
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let log_line = format!("[{}] {}\n", timestamp, message);
        let _ = file.write_all(log_line.as_bytes());
        let _ = file.flush(); // Flush immediately to ensure logs are written
    }
}

/// Macro for easy logging that goes to both console and file
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        {
            let msg = format!($($arg)*);
            $crate::file_logger::log(&msg);
        }
    };
}

