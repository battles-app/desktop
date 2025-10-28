use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    tauri_build::build();
    
    // Bundle GStreamer DLLs for Windows
    if cfg!(target_os = "windows") {
        bundle_gstreamer_dlls();
        
        // Embed Windows manifest for elevated USB/HID access
        embed_windows_manifest();
    }
}

#[cfg(target_os = "windows")]
fn embed_windows_manifest() {
    // Compile and embed the .rc file which includes the manifest
    embed_resource::compile("app.rc", embed_resource::NONE);
}

#[cfg(not(target_os = "windows"))]
fn embed_windows_manifest() {
    // No-op on non-Windows platforms
}

fn bundle_gstreamer_dlls() {
    
    // Get target directory and project root
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    
    // SKIP bundling in debug/dev builds - use system GStreamer only
    if profile == "debug" {
        return;
    }
    
    // Get GStreamer path from environment
    let gst_path = env::var("GSTREAMER_1_0_ROOT_MSVC_X86_64")
        .unwrap_or_else(|_| "E:\\gstreamer\\1.0\\msvc_x86_64".to_string());
    
    let gst_bin = PathBuf::from(&gst_path).join("bin");
    let gst_plugins = PathBuf::from(&gst_path).join("lib").join("gstreamer-1.0");
    
    if !gst_bin.exists() {
        return;
    }
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let target_dir = PathBuf::from(&manifest_dir).join("target").join(profile);
    let gstreamer_runtime_dir = PathBuf::from(&manifest_dir).join("gstreamer-runtime");
    
    // Ensure gstreamer-runtime directory exists in project root for Tauri resource dir
    let _ = fs::create_dir_all(&gstreamer_runtime_dir);
    
    // CRITICAL: Also create gstreamer-runtime in target directory (for NSIS bundler)
    let target_gstreamer_dir = target_dir.join("gstreamer-runtime");
    let _ = fs::create_dir_all(&target_gstreamer_dir);
    
    // Silent bundling for clean build output
    
    // Copy ALL GStreamer DLLs (not just a subset)
    // GStreamer plugins have complex dependencies - copying everything ensures no missing DLLs
    let mut _copied_count = 0;
    let mut _skipped_count = 0;
    
    // Read all DLLs from GStreamer bin directory
    if let Ok(entries) = fs::read_dir(&gst_bin) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("dll") {
                if let Some(dll_name) = path.file_name().and_then(|s| s.to_str()) {
                    // Copy to FOUR locations:
                    // 1. target/[profile]/ (for dev .exe)
                    let dst_target = target_dir.join(dll_name);
                    if fs::copy(&path, &dst_target).is_ok() {
                        _copied_count += 1;
                    }
                    
                    // 2. gstreamer-runtime/ (for Tauri resource_dir at runtime)
                    let dst_runtime = gstreamer_runtime_dir.join(dll_name);
                    let _ = fs::copy(&path, &dst_runtime);
                    
                    // 3. target/[profile]/gstreamer-runtime/ (organized structure)
                    let dst_target_runtime = target_gstreamer_dir.join(dll_name);
                    let _ = fs::copy(&path, &dst_target_runtime);
                    
                    // 4. PROJECT ROOT (for NSIS bundler to pick up with resources field)
                    let dst_project_root = PathBuf::from(&manifest_dir).join(dll_name);
                    let _ = fs::copy(&path, &dst_project_root);
                }
            }
        }
    } else {
        _skipped_count += 1;
    }
    
    // Bundle ALL GStreamer plugins (not just essential ones)
    // Plugins are already .dll files in the gstreamer-1.0 subdirectory
    // BUT they're also in the main bin directory, so they're already copied above
    // We still copy them to gstreamer-1.0 subdirectories for organized structure
    if gst_plugins.exists() {
        let plugins_dir_target = target_dir.join("gstreamer-1.0");
        let plugins_dir_runtime = gstreamer_runtime_dir.join("gstreamer-1.0");
        let plugins_dir_target_runtime = target_gstreamer_dir.join("gstreamer-1.0");
        let _ = fs::create_dir_all(&plugins_dir_target);
        let _ = fs::create_dir_all(&plugins_dir_runtime);
        let _ = fs::create_dir_all(&plugins_dir_target_runtime);
        
        // Copy ALL plugin DLLs from gstreamer-1.0 directory
        if let Ok(entries) = fs::read_dir(&gst_plugins) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("dll") {
                    if let Some(plugin_name) = path.file_name().and_then(|s| s.to_str()) {
                        // Copy to organized subdirectories
                        let _ = fs::copy(&path, plugins_dir_target.join(plugin_name));
                        let _ = fs::copy(&path, plugins_dir_runtime.join(plugin_name));
                        let _ = fs::copy(&path, plugins_dir_target_runtime.join(plugin_name));
                        
                        // ALSO copy to project root (for Tauri resources field)
                        let dst_project_root = PathBuf::from(&manifest_dir).join(plugin_name);
                        let _ = fs::copy(&path, &dst_project_root);
                    }
                }
            }
        }
    }
    
    // Tell cargo to link GStreamer
}


