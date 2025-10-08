# Clean restart script for Stream Deck testing

Write-Host "🧹 Cleaning Tauri dev cache..." -ForegroundColor Yellow

# Kill any running Tauri instances
Get-Process | Where-Object { $_.ProcessName -like "*battles-desktop*" } | Stop-Process -Force -ErrorAction SilentlyContinue

# Clear Tauri cache
if (Test-Path "target/debug") {
    Write-Host "  Clearing debug build..." -ForegroundColor Gray
}

# Wait a moment
Start-Sleep -Seconds 1

Write-Host "✅ Cache cleared!" -ForegroundColor Green
Write-Host ""
Write-Host "🚀 Starting Tauri app..." -ForegroundColor Yellow
Write-Host ""

# Start with clean cache
cargo run

