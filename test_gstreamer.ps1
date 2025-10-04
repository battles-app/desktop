# GStreamer Installation Verification Script
# Run this after installing GStreamer to verify everything is set up correctly

Write-Host "=== GStreamer Installation Verification ===" -ForegroundColor Cyan
Write-Host ""

# Define GStreamer path (user uses E: drive)
$gstPath = "E:\gstreamer\1.0\msvc_x86_64"
$gstBin = "$gstPath\bin"
$gstInspect = "$gstBin\gst-inspect-1.0.exe"

# Check if GStreamer is installed
Write-Host "[1/6] Checking if GStreamer is installed at $gstPath..." -ForegroundColor Yellow
if (Test-Path $gstInspect) {
    try {
        $gstVersion = & $gstInspect --version 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host "✅ GStreamer found at $gstBin" -ForegroundColor Green
            Write-Host $gstVersion -ForegroundColor Gray
        } else {
            Write-Host "❌ GStreamer executable exists but failed to run" -ForegroundColor Red
            exit 1
        }
    } catch {
        Write-Host "❌ Failed to run GStreamer" -ForegroundColor Red
        Write-Host $_.Exception.Message -ForegroundColor Gray
        exit 1
    }
} else {
    Write-Host "❌ GStreamer NOT INSTALLED at $gstPath" -ForegroundColor Red
    Write-Host "" -ForegroundColor Yellow
    Write-Host "Please install GStreamer first:" -ForegroundColor Yellow
    Write-Host "  1. Download runtime installer from:" -ForegroundColor Gray
    Write-Host "     https://gstreamer.freedesktop.org/download/" -ForegroundColor Gray
    Write-Host "  2. Install to: E:\gstreamer\1.0\msvc_x86_64\" -ForegroundColor Gray
    Write-Host "  3. Choose 'Complete' installation" -ForegroundColor Gray
    Write-Host "  4. Install development package to same location" -ForegroundColor Gray
    Write-Host "" -ForegroundColor Yellow
    Write-Host "See INSTALL_GSTREAMER.md for detailed instructions" -ForegroundColor Cyan
    exit 1
}
Write-Host ""

# Check environment variable
Write-Host "[2/6] Checking GSTREAMER_1_0_ROOT_MSVC_X86_64 environment variable..." -ForegroundColor Yellow
$gstRoot = [System.Environment]::GetEnvironmentVariable("GSTREAMER_1_0_ROOT_MSVC_X86_64", "Machine")
if ($gstRoot -eq "E:\gstreamer\1.0\msvc_x86_64\") {
    Write-Host "✅ Environment variable set correctly: $gstRoot" -ForegroundColor Green
} elseif ($gstRoot) {
    Write-Host "⚠️  Environment variable set to different path: $gstRoot" -ForegroundColor Yellow
    Write-Host "   Expected: E:\gstreamer\1.0\msvc_x86_64\" -ForegroundColor Gray
} else {
    Write-Host "⚠️  Environment variable not set (optional, but recommended)" -ForegroundColor Yellow
    Write-Host "   Set GSTREAMER_1_0_ROOT_MSVC_X86_64=E:\gstreamer\1.0\msvc_x86_64\" -ForegroundColor Gray
}
Write-Host ""

# Check for required plugins
Write-Host "[3/6] Checking required GStreamer plugins..." -ForegroundColor Yellow
$requiredPlugins = @("ksvideosrc", "videoconvert", "jpegenc", "appsink")
$allPluginsFound = $true

foreach ($plugin in $requiredPlugins) {
    try {
        $result = & $gstInspect $plugin 2>&1
        if ($LASTEXITCODE -eq 0) {
            Write-Host "  ✅ $plugin" -ForegroundColor Green
        } else {
            Write-Host "  ❌ $plugin not found" -ForegroundColor Red
            $allPluginsFound = $false
        }
    } catch {
        Write-Host "  ❌ $plugin not found" -ForegroundColor Red
        $allPluginsFound = $false
    }
}

if (-not $allPluginsFound) {
    Write-Host "❌ Some required plugins are missing. Install the complete GStreamer package." -ForegroundColor Red
    exit 1
}
Write-Host ""

# Check for cameras
Write-Host "[4/6] Detecting cameras..." -ForegroundColor Yellow
$gstDeviceMonitor = "$gstBin\gst-device-monitor-1.0.exe"
Write-Host "Running gst-device-monitor-1.0 for 3 seconds..." -ForegroundColor Gray
try {
    $job = Start-Job -ScriptBlock { param($exe) & $exe } -ArgumentList $gstDeviceMonitor
    Start-Sleep -Seconds 3
    Stop-Job $job
    $output = Receive-Job $job
    Remove-Job $job
    
    if ($output -match "Video") {
        Write-Host "✅ Cameras detected:" -ForegroundColor Green
        $output | Where-Object { $_ -match "device.class|display-name" } | ForEach-Object {
            Write-Host "  $_" -ForegroundColor Gray
        }
    } else {
        Write-Host "⚠️  No cameras detected" -ForegroundColor Yellow
        Write-Host "   Make sure your camera is connected and enabled in Windows Settings" -ForegroundColor Gray
    }
} catch {
    Write-Host "⚠️  Could not detect cameras" -ForegroundColor Yellow
}
Write-Host ""

# Test a simple pipeline
Write-Host "[5/6] Testing GStreamer pipeline..." -ForegroundColor Yellow
$gstLaunch = "$gstBin\gst-launch-1.0.exe"
try {
    $testPipeline = "videotestsrc num-buffers=10 ! videoconvert ! fakesink"
    Write-Host "Running: gst-launch-1.0 $testPipeline" -ForegroundColor Gray
    $result = & $gstLaunch videotestsrc num-buffers=10 ! videoconvert ! fakesink 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ GStreamer pipeline works correctly" -ForegroundColor Green
    } else {
        Write-Host "❌ GStreamer pipeline failed" -ForegroundColor Red
        Write-Host $result -ForegroundColor Gray
        exit 1
    }
} catch {
    Write-Host "❌ GStreamer pipeline test failed" -ForegroundColor Red
    exit 1
}
Write-Host ""

# Check Rust compilation
Write-Host "[6/6] Testing Rust GStreamer bindings..." -ForegroundColor Yellow
Write-Host "Running: cargo check" -ForegroundColor Gray
try {
    Push-Location $PSScriptRoot
    $cargoOutput = & cargo check 2>&1
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ Rust GStreamer bindings compile successfully" -ForegroundColor Green
    } else {
        Write-Host "❌ Rust compilation failed" -ForegroundColor Red
        Write-Host $cargoOutput -ForegroundColor Gray
        Pop-Location
        exit 1
    }
    Pop-Location
} catch {
    Write-Host "❌ Cargo check failed" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Gray
    exit 1
}
Write-Host ""

Write-Host "=== ✅ ALL CHECKS PASSED ===" -ForegroundColor Green
Write-Host ""
Write-Host "GStreamer is properly installed and configured!" -ForegroundColor Cyan
Write-Host "You can now build and run the Battles Desktop app with camera support." -ForegroundColor Cyan
Write-Host ""
Write-Host "Next steps:" -ForegroundColor Yellow
Write-Host "  1. cargo build --release" -ForegroundColor Gray
Write-Host "  2. Run the desktop app" -ForegroundColor Gray
Write-Host "  3. Test camera preview" -ForegroundColor Gray
Write-Host "  4. WebSocket streams at ws://127.0.0.1:9876" -ForegroundColor Gray

