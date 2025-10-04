# Set GStreamer Environment Variables Permanently
# Run this once: .\set_gstreamer_env.ps1

Write-Host "Setting GStreamer environment variables..." -ForegroundColor Cyan

$gstPath = "E:\gstreamer\1.0\msvc_x86_64"
$gstBin = "$gstPath\bin"
$pkgConfigPath = "$gstPath\lib\pkgconfig"

# Set system environment variables (requires admin)
try {
    # Add GStreamer bin to PATH if not already there
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if (-not $currentPath.Contains($gstBin)) {
        Write-Host "Adding $gstBin to system PATH..." -ForegroundColor Yellow
        [Environment]::SetEnvironmentVariable("Path", "$currentPath;$gstBin", "Machine")
        Write-Host "✅ Added to PATH" -ForegroundColor Green
    } else {
        Write-Host "✅ Already in PATH" -ForegroundColor Green
    }
    
    # Set GSTREAMER_1_0_ROOT_MSVC_X86_64
    Write-Host "Setting GSTREAMER_1_0_ROOT_MSVC_X86_64..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable("GSTREAMER_1_0_ROOT_MSVC_X86_64", "$gstPath\", "Machine")
    Write-Host "✅ Set" -ForegroundColor Green
    
    # Set PKG_CONFIG_PATH
    Write-Host "Setting PKG_CONFIG_PATH..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable("PKG_CONFIG_PATH", $pkgConfigPath, "Machine")
    Write-Host "✅ Set" -ForegroundColor Green
    
    Write-Host ""
    Write-Host "=== Environment Variables Set Successfully ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "IMPORTANT: You need to restart your terminal/IDE for changes to take effect!" -ForegroundColor Yellow
    Write-Host ""
    Write-Host "After restarting, you can use:" -ForegroundColor Cyan
    Write-Host "  cargo build" -ForegroundColor Gray
    Write-Host "  cargo tauri dev" -ForegroundColor Gray
    Write-Host ""
    
} catch {
    Write-Host "❌ Error: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host ""
    Write-Host "This script requires Administrator privileges." -ForegroundColor Yellow
    Write-Host "Please run PowerShell as Administrator and try again." -ForegroundColor Yellow
    Write-Host ""
    Write-Host "Or use the .\build.ps1 script which sets environment variables temporarily." -ForegroundColor Cyan
}


