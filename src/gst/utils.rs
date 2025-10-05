use anyhow::{anyhow, Result};
use gstreamer as gst;
use gstreamer::prelude::*;

/// Camera information
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Camera ID
    pub id: String,
    
    /// Camera name
    pub name: String,
    
    /// Camera description
    pub description: String,
}

/// Initialize GStreamer
pub fn init() -> Result<()> {
    if !gst::is_initialized() {
        gst::init()?;
    }
    
    Ok(())
}

/// List available cameras
pub fn list_cameras() -> Result<Vec<CameraInfo>> {
    // Initialize GStreamer if not already initialized
    init()?;
    
    let mut cameras = Vec::new();
    
    #[cfg(target_os = "windows")]
    {
        use gstreamer::DeviceMonitor;
        
        let monitor = DeviceMonitor::new();
        let caps = gst::Caps::builder("video/x-raw").build();
        monitor.add_filter(Some("Video/Source"), Some(&caps));
        
        if monitor.start().is_err() {
            return Ok(cameras);
        }
        
        let devices = monitor.devices();
        let mut device_index = 0;
        
        for device in devices.iter() {
            if let Some(device_caps) = device.caps() {
                if device_caps.is_empty() {
                    continue;
                }
                
                let display_name = device.display_name();
                let has_valid_path = device.properties()
                    .and_then(|props| props.get::<String>("device.path").ok())
                    .is_some();
                
                if has_valid_path {
                    cameras.push(CameraInfo {
                        id: device_index.to_string(),
                        name: display_name.to_string(),
                        description: "Camera".to_string(),
                    });
                    device_index += 1;
                }
            }
        }
        
        monitor.stop();
    }
    
    #[cfg(target_os = "linux")]
    {
        use std::path::Path;
        use std::fs;
        
        // On Linux, enumerate /dev/video* devices
        for entry in fs::read_dir("/dev")? {
            let entry = entry?;
            let path = entry.path();
            
            if let Some(filename) = path.file_name() {
                if let Some(filename_str) = filename.to_str() {
                    if filename_str.starts_with("video") {
                        if let Some(index_str) = filename_str.strip_prefix("video") {
                            if let Ok(index) = index_str.parse::<u32>() {
                                cameras.push(CameraInfo {
                                    id: index.to_string(),
                                    name: format!("Camera {}", index),
                                    description: format!("/dev/{}", filename_str),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    
    #[cfg(target_os = "macos")]
    {
        use gstreamer::DeviceMonitor;
        
        let monitor = DeviceMonitor::new();
        let caps = gst::Caps::builder("video/x-raw").build();
        monitor.add_filter(Some("Video/Source"), Some(&caps));
        
        if monitor.start().is_err() {
            return Ok(cameras);
        }
        
        let devices = monitor.devices();
        let mut device_index = 0;
        
        for device in devices.iter() {
            if let Some(device_caps) = device.caps() {
                if device_caps.is_empty() {
                    continue;
                }
                
                let display_name = device.display_name();
                
                cameras.push(CameraInfo {
                    id: device_index.to_string(),
                    name: display_name.to_string(),
                    description: "Camera".to_string(),
                });
                device_index += 1;
            }
        }
        
        monitor.stop();
    }
    
    Ok(cameras)
}

/// List available monitors
pub fn list_monitors() -> Result<Vec<(u32, String)>> {
    // Initialize GStreamer if not already initialized
    init()?;
    
    let mut monitors = Vec::new();
    
    #[cfg(target_os = "windows")]
    {
        // On Windows, we can use the dx9screencapsrc element
        // But we need to query the system for monitor information
        use winit::event_loop::EventLoop;
        
        let event_loop = EventLoop::new().map_err(|e| anyhow!("Failed to create event loop: {}", e))?;
        let monitors_list = event_loop.available_monitors();
        
        for (i, monitor) in monitors_list.enumerate() {
            monitors.push((i as u32, monitor.name().unwrap_or_else(|| format!("Monitor {}", i))));
        }
    }
    
    #[cfg(target_os = "linux")]
    {
        // On Linux, we can use the ximagesrc element
        // But we need to query the X server for monitor information
        // This is a simplified implementation
        monitors.push((0, "Primary Monitor".to_string()));
    }
    
    #[cfg(target_os = "macos")]
    {
        // On macOS, we can use the avfvideosrc element with capture-screen=true
        // But we need to query the system for monitor information
        monitors.push((0, "Primary Monitor".to_string()));
    }
    
    Ok(monitors)
}

/// Convert RGB to YUV
pub fn rgb_to_yuv(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let r_f = r as f32 / 255.0;
    let g_f = g as f32 / 255.0;
    let b_f = b as f32 / 255.0;
    
    let y = 0.299 * r_f + 0.587 * g_f + 0.114 * b_f;
    let u = -0.14713 * r_f - 0.28886 * g_f + 0.436 * b_f;
    let v = 0.615 * r_f - 0.51499 * g_f - 0.10001 * b_f;
    
    let y = (y * 255.0) as u8;
    let u = ((u + 0.5) * 255.0) as u8;
    let v = ((v + 0.5) * 255.0) as u8;
    
    (y, u, v)
}

/// Convert YUV to RGB
pub fn yuv_to_rgb(y: u8, u: u8, v: u8) -> (u8, u8, u8) {
    let y_f = y as f32 / 255.0;
    let u_f = (u as f32 / 255.0) - 0.5;
    let v_f = (v as f32 / 255.0) - 0.5;
    
    let r = y_f + 1.13983 * v_f;
    let g = y_f - 0.39465 * u_f - 0.58060 * v_f;
    let b = y_f + 2.03211 * u_f;
    
    let r = (r * 255.0).clamp(0.0, 255.0) as u8;
    let g = (g * 255.0).clamp(0.0, 255.0) as u8;
    let b = (b * 255.0).clamp(0.0, 255.0) as u8;
    
    (r, g, b)
}

/// Parse hex color string to RGB
pub fn hex_to_rgb(hex: &str) -> Result<(u8, u8, u8)> {
    let hex = hex.trim_start_matches('#');
    
    if hex.len() != 6 {
        return Err(anyhow!("Invalid hex color: {}", hex));
    }
    
    let r = u8::from_str_radix(&hex[0..2], 16)
        .map_err(|_| anyhow!("Invalid hex color: {}", hex))?;
    let g = u8::from_str_radix(&hex[2..4], 16)
        .map_err(|_| anyhow!("Invalid hex color: {}", hex))?;
    let b = u8::from_str_radix(&hex[4..6], 16)
        .map_err(|_| anyhow!("Invalid hex color: {}", hex))?;
    
    Ok((r, g, b))
}
