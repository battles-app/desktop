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
    
    let gst_bin = PathBuf::from(&gst_path).join("bin");
    let gst_plugins = PathBuf::from(&gst_path).join("lib").join("gstreamer-1.0");
    
    if !gst_bin.exists() {
        eprintln!("Warning: GStreamer bin directory not found at {:?}", gst_bin);
        eprintln!("Skipping DLL bundling. Application may not run on target system.");
        return;
    }
    
    // Get target directory - use the actual target/release directory
    // NSIS bundler will automatically include DLLs from this location
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let target_dir = PathBuf::from(manifest_dir).join("target").join(profile);
    
    println!("cargo:warning=━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("cargo:warning=  📦 Bundling GStreamer Dependencies");
    println!("cargo:warning=━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("cargo:warning=From: {:?}", gst_bin);
    println!("cargo:warning=  To: {:?}", target_dir);
    println!("cargo:warning=");
    
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
        "gstrtp-1.0-0.dll",
        "gstrtsp-1.0-0.dll",
        "gsttag-1.0-0.dll",
        
        // Required dependencies
        "intl-8.dll",
        "ffi-7.dll",
        "z-1.dll",
        "winpthread-1.dll",
        "pcre2-8-0.dll",
        
        // Video processing
        "orc-0.4-0.dll",
        
        // Graphics
        "pixman-1-0.dll",
        "png16-16.dll",
        "graphene-1.0-0.dll",
    ];
    
    // Create gstreamer-runtime directory in project root for Tauri bundler
    let runtime_dir = PathBuf::from(&manifest_dir).join("gstreamer-runtime");
    let _ = fs::create_dir_all(&runtime_dir);
    
    println!("cargo:warning=📚 Core Libraries:");
    let mut copied = 0;
    let mut missing = Vec::new();
    
    for dll in &required_dlls {
        let src = gst_bin.join(dll);
        
        // Copy to both target dir (for dev/testing) and runtime dir (for bundling)
        let dst_target = target_dir.join(dll);
        let dst_runtime = runtime_dir.join(dll);
        
        if src.exists() {
            let mut success = false;
            
            // Copy to target directory
            if let Ok(_) = fs::copy(&src, &dst_target) {
                success = true;
            }
            
            // Copy to runtime directory for bundler
            if let Ok(_) = fs::copy(&src, &dst_runtime) {
                success = true;
            }
            
            if success {
                copied += 1;
                println!("cargo:warning=  ✓ {}", dll);
            } else {
                println!("cargo:warning=  ✗ {} (copy failed)", dll);
            }
        } else {
            missing.push(dll);
            println!("cargo:warning=  ⚠ {} (not found)", dll);
        }
    }
    
    // Bundle essential GStreamer plugins
    if gst_plugins.exists() {
        let plugins_dir_target = target_dir.join("gstreamer-1.0");
        let plugins_dir_runtime = runtime_dir.join("gstreamer-1.0");
        let _ = fs::create_dir_all(&plugins_dir_target);
        let _ = fs::create_dir_all(&plugins_dir_runtime);
        
        let essential_plugins = vec![
            "gstapp.dll",
            "gstcoreelements.dll",
            "gstvideoconvertscale.dll",
            "gstvideofilter.dll",
            "gstvideotestsrc.dll",
            "gstvideoparsersbad.dll",
            "gstaudioconvert.dll",
            "gstaudioresample.dll",
            "gstaudiotestsrc.dll",
            "gstautodetect.dll",
            "gstplayback.dll",
            "gsttypefindfunctions.dll",
            "gstd3d11.dll",
            "gstopengl.dll",
            "gstd3dvideosink.dll",
        ];
        
        println!("cargo:warning=");
        println!("cargo:warning=🔌 GStreamer Plugins:");
        let mut plugins_copied = 0;
        
        for plugin in &essential_plugins {
            let src = gst_plugins.join(plugin);
            
            if src.exists() {
                let mut success = false;
                
                // Copy to target directory
                if let Ok(_) = fs::copy(&src, plugins_dir_target.join(plugin)) {
                    success = true;
                }
                
                // Copy to runtime directory for bundler
                if let Ok(_) = fs::copy(&src, plugins_dir_runtime.join(plugin)) {
                    success = true;
                }
                
                if success {
                    plugins_copied += 1;
                    println!("cargo:warning=  ✓ {}", plugin);
                } else {
                    println!("cargo:warning=  ✗ {} (copy failed)", plugin);
                }
            } else {
                println!("cargo:warning=  ⚠ {} (not found)", plugin);
            }
        }
        
        println!("cargo:warning=");
        println!("cargo:warning=✅ Plugins: {} bundled", plugins_copied);
    }
    
    println!("cargo:warning=");
    println!("cargo:warning=━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("cargo:warning=✅ Libraries: {} bundled", copied);
    if !missing.is_empty() {
        println!("cargo:warning=⚠️  Missing: {} (may cause runtime errors)", missing.len());
    }
    println!("cargo:warning=━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    
    // Tell cargo to link GStreamer
    println!("cargo:rustc-link-search=native={}", gst_bin.display());
}


