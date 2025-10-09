use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    tauri_build::build();
    
    // Bundle GStreamer DLLs for Windows
    if cfg!(target_os = "windows") {
        bundle_gstreamer_dlls();
    }
}

fn bundle_gstreamer_dlls() {
    println!("cargo:rerun-if-changed=build.rs");
    
    // Get GStreamer path from environment
    let gst_path = env::var("GSTREAMER_1_0_ROOT_MSVC_X86_64")
        .unwrap_or_else(|_| "E:\\gstreamer\\1.0\\msvc_x86_64".to_string());
    
    let gst_bin = PathBuf::from(gst_path).join("bin");
    
    if !gst_bin.exists() {
        eprintln!("Warning: GStreamer bin directory not found at {:?}", gst_bin);
        eprintln!("Skipping DLL bundling. Application may not run on target system.");
        return;
    }
    
    // Get output directory
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
    let target_dir = PathBuf::from(out_dir)
        .ancestors()
        .nth(3)
        .expect("Failed to get target directory")
        .to_path_buf();
    
    println!("cargo:warning=Bundling GStreamer DLLs from {:?} to {:?}", gst_bin, target_dir);
    
    // Essential GStreamer DLLs - only copy what's absolutely needed
    let required_dlls = vec![
        // Core GLib/GObject
        "glib-2.0-0.dll",
        "gobject-2.0-0.dll",
        "gmodule-2.0-0.dll",
        "gio-2.0-0.dll",
        
        // Core GStreamer
        "gstreamer-1.0-0.dll",
        "gstbase-1.0-0.dll",
        "gstapp-1.0-0.dll",
        "gstvideo-1.0-0.dll",
        "gstaudio-1.0-0.dll",
        "gstpbutils-1.0-0.dll",
        "gstcontroller-1.0-0.dll",
        "gstnet-1.0-0.dll",
        "gstgl-1.0-0.dll",
        "gstallocators-1.0-0.dll",
        
        // Required dependencies
        "intl-8.dll",
        "ffi-7.dll",
        "z-1.dll",
        "winpthread-1.dll",
        
        // Video processing
        "orc-0.4-0.dll",
    ];
    
    let mut copied = 0;
    let mut missing = Vec::new();
    
    for dll in &required_dlls {
        let src = gst_bin.join(dll);
        let dst = target_dir.join(dll);
        
        if src.exists() {
            match fs::copy(&src, &dst) {
                Ok(_) => {
                    copied += 1;
                    println!("cargo:warning=  ✓ Copied {}", dll);
                }
                Err(e) => {
                    println!("cargo:warning=  ✗ Failed to copy {}: {}", dll, e);
                }
            }
        } else {
            missing.push(dll);
            println!("cargo:warning=  ⚠ Missing: {}", dll);
        }
    }
    
    println!("cargo:warning=GStreamer DLL bundling complete: {} copied, {} missing", copied, missing.len());
    
    if !missing.is_empty() {
        println!("cargo:warning=Missing DLLs may cause runtime errors: {:?}", missing);
    }
    
    // Tell cargo to link GStreamer
    println!("cargo:rustc-link-search=native={}", gst_bin.display());
}


