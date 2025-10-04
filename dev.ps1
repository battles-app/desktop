# Development launch script with GStreamer in PATH
$env:Path = "E:\gstreamer\1.0\msvc_x86_64\bin;$env:Path"
$env:GSTREAMER_1_0_ROOT_MSVC_X86_64 = "E:\gstreamer\1.0\msvc_x86_64\"

Write-Host "Starting Tauri dev with GStreamer in PATH..." -ForegroundColor Green
bun run dev





