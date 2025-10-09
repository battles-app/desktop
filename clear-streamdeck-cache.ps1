# Clear Stream Deck image cache to force re-download of thumbnails

$cacheDir = "$env:TEMP\battles_fx_cache"

if (Test-Path $cacheDir) {
    Write-Host "Clearing Stream Deck cache at: $cacheDir" -ForegroundColor Yellow
    Remove-Item -Path "$cacheDir\*" -Force -Recurse
    Write-Host "Cache cleared! Images will be re-downloaded as 96x96 thumbnails" -ForegroundColor Green
} else {
    Write-Host "Cache directory does not exist yet: $cacheDir" -ForegroundColor Cyan
}

Write-Host ""
Write-Host "Now restart your app to download ACTUAL IMAGE thumbnails!" -ForegroundColor Magenta
