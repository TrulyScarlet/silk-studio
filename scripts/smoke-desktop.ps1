# Launches the built desktop shell, verifies it stays alive, then kills it.
# This is the automated replacement for "we compiled it" — it proves the
# window/webview/controller wiring starts without crashing on this machine.
# Run from the repository root.

$ErrorActionPreference = "Stop"

Write-Host "== building desktop shell ==" -ForegroundColor Cyan
cargo build -p silk
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$exe = Join-Path (Get-Location) "target\debug\silk.exe"
if (-not (Test-Path $exe)) { throw "built exe not found at $exe" }

Write-Host "== launching $exe ==" -ForegroundColor Cyan
$process = Start-Process -FilePath $exe -PassThru
try {
    Start-Sleep -Seconds 6
    if ($process.HasExited) {
        throw "desktop shell exited early with code $($process.ExitCode)"
    }
    Write-Host "smoke OK: window process alive after 6 s (pid $($process.Id))" -ForegroundColor Green
}
finally {
    if (-not $process.HasExited) {
        Stop-Process -Id $process.Id -Force
        Write-Host "smoke: process stopped"
    }
}
