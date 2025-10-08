# Stream Deck Diagnostic Test Script
# Run this to check if your Stream Deck is detected

Write-Host "======================================" -ForegroundColor Cyan
Write-Host "Stream Deck Diagnostic Test" -ForegroundColor Cyan
Write-Host "======================================" -ForegroundColor Cyan
Write-Host ""

# Build and run the diagnostic
Write-Host "Building application..." -ForegroundColor Yellow
cargo build --quiet

if ($LASTEXITCODE -ne 0) {
    Write-Host "❌ Build failed!" -ForegroundColor Red
    exit 1
}

Write-Host "✅ Build successful!" -ForegroundColor Green
Write-Host ""
Write-Host "Running Stream Deck diagnostics..." -ForegroundColor Yellow
Write-Host "======================================" -ForegroundColor Cyan
Write-Host ""

# Run the app (it will log diagnostics to console)
# Note: You'll need to actually call the diagnostic command from the frontend
# For now, just start the app and it will attempt to connect

Write-Host "Starting application..." -ForegroundColor Yellow
Write-Host ""
Write-Host "⚠️  The app will now start. Check the terminal output for Stream Deck detection messages." -ForegroundColor Yellow
Write-Host ""
Write-Host "Look for these messages:" -ForegroundColor Cyan
Write-Host "  [Stream Deck] Initializing..." -ForegroundColor Gray
Write-Host "  [Stream Deck] Scanning for devices..." -ForegroundColor Gray
Write-Host "  [Stream Deck] Found X devices" -ForegroundColor Gray
Write-Host "  [Stream Deck] ✅ Connected to..." -ForegroundColor Gray
Write-Host ""
Write-Host "Press Ctrl+C to stop the app" -ForegroundColor Yellow
Write-Host ""

cargo run

