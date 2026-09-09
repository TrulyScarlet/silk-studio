# Silk Fullscreen Rect & WorkArea Gap Test Script
# Runs test-fullscreen-rects.mjs to query GetWindowRect, GetClientRect, SPI_GETWORKAREA
# before fullscreen, in fullscreen, and after exiting fullscreen, and tests Win32 API calls.

param(
    [string]$Target = "debug"
)

$ErrorActionPreference = "Stop"

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

$script = Join-Path $PSScriptRoot "test-fullscreen-rects.mjs"
node $script
