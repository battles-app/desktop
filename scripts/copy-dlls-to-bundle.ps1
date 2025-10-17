# Copy GStreamer DLLs to NSIS bundle for re-packaging
$projectRoot = Split-Path -Parent $PSScriptRoot
$bundleDir = Join-Path $projectRoot "target\release\bundle\nsis"

if (Test-Path $bundleDir) {
    Write-Host "📦 Copying GStreamer DLLs to bundle directory..."
    
    # Copy all DLLs from project root to bundle
    Get-ChildItem -Path $projectRoot -Filter "*.dll" | ForEach-Object {
        Copy-Item $_.FullName -Destination $bundleDir -Force
        Write-Host "  ✅ Copied $($_.Name)"
    }
    
    # Create gstreamer-runtime subfolder in bundle
    $bundleGstDir = Join-Path $bundleDir "gstreamer-runtime"
    if (!(Test-Path $bundleGstDir)) {
        New-Item -ItemType Directory -Path $bundleGstDir | Out-Null
    }
    
    # Copy gstreamer-runtime folder contents
    $gstRuntimeDir = Join-Path $projectRoot "gstreamer-runtime"
    if (Test-Path $gstRuntimeDir) {
        Copy-Item -Path "$gstRuntimeDir\*" -Destination $bundleGstDir -Recurse -Force
        Write-Host "  ✅ Copied gstreamer-runtime folder"
    }
    
    Write-Host "✅ GStreamer DLLs copied to bundle successfully!"
} else {
    Write-Host "⚠️  Bundle directory not found: $bundleDir"
    exit 1
}

