# Comprehensive diagnostic runner for Silk window bottom pixel inspection
# Runs Node.js CDP automation and Win32 pixel inspection across all playback/window states.
param(
    [string]$Target = "debug"
)

$ErrorActionPreference = "Stop"

Write-Host "================================================================================" -ForegroundColor Cyan
Write-Host " [SILK] Running Bottom Pixel & Window Region Diagnostics ($Target build)" -ForegroundColor Cyan
Write-Host "================================================================================" -ForegroundColor Cyan

$exe = if ($Target -eq "release") {
    Join-Path $PSScriptRoot "..\target\release\silk.exe"
} else {
    Join-Path $PSScriptRoot "..\target\debug\silk.exe"
}

if (-not (Test-Path $exe)) {
    Write-Host "Building desktop shell ($Target)..." -ForegroundColor Yellow
    if ($Target -eq "release") {
        cargo build -p silk --release
    } else {
        cargo build -p silk
    }
}

$script = Join-Path $PSScriptRoot "diagnose-bottom-pixels.mjs"
node $script
