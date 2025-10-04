# Run this ONCE as Administrator to set permanent environment variables
# Then you can delete build.ps1 and use regular commands!

Write-Host "=== Setting Permanent GStreamer Environment Variables ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "This will set SYSTEM environment variables permanently." -ForegroundColor Yellow
Write-Host "You'll need to restart your terminal/IDE after this." -ForegroundColor Yellow
Write-Host ""

$gstPath = "E:\gstreamer\1.0\msvc_x86_64"
$gstBin = "$gstPath\bin"
$pkgConfigPath = "$gstPath\lib\pkgconfig"

try {
    # 1. Add GStreamer bin to PATH
    Write-Host "[1/3] Adding to system PATH..." -ForegroundColor Yellow
    $currentPath = [Environment]::GetEnvironmentVariable("Path", "Machine")
    if (-not $currentPath.Contains($gstBin)) {
        [Environment]::SetEnvironmentVariable("Path", "$currentPath;$gstBin", "Machine")
        Write-Host "✅ Added $gstBin to PATH" -ForegroundColor Green
    } else {
        Write-Host "✅ Already in PATH" -ForegroundColor Green
    }
    
    # 2. Set PKG_CONFIG_PATH
    Write-Host "[2/3] Setting PKG_CONFIG_PATH..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable("PKG_CONFIG_PATH", $pkgConfigPath, "Machine")
    Write-Host "✅ Set to $pkgConfigPath" -ForegroundColor Green
    
    # 3. Set GSTREAMER_1_0_ROOT_MSVC_X86_64
    Write-Host "[3/3] Setting GSTREAMER_1_0_ROOT_MSVC_X86_64..." -ForegroundColor Yellow
    [Environment]::SetEnvironmentVariable("GSTREAMER_1_0_ROOT_MSVC_X86_64", "$gstPath\", "Machine")
    Write-Host "✅ Set to $gstPath\" -ForegroundColor Green
    
    Write-Host ""
    Write-Host "=== ✅ SUCCESS ===" -ForegroundColor Green
    Write-Host ""
    Write-Host "Environment variables set permanently!" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Next steps:" -ForegroundColor Yellow
    Write-Host "  1. Close ALL terminals and restart them" -ForegroundColor Gray
    Write-Host "  2. You can now delete build.ps1" -ForegroundColor Gray
    Write-Host "  3. Use regular commands:" -ForegroundColor Gray
    Write-Host "     - cargo tauri dev" -ForegroundColor White
    Write-Host "     - cargo tauri build" -ForegroundColor White
    Write-Host "     - cargo check" -ForegroundColor White
    Write-Host ""
    
} catch {
    Write-Host ""
    Write-Host "=== ❌ ERROR ===" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
    Write-Host ""
    Write-Host "This script requires Administrator privileges!" -ForegroundColor Yellow
    Write-Host "Right-click PowerShell → Run as Administrator" -ForegroundColor Yellow
    Write-Host ""
    exit 1
}





