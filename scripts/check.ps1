# Development gate script (plan §15 verification commands).
# Run from the repository root.

$ErrorActionPreference = "Stop"

Write-Host "== cargo fmt ==" -ForegroundColor Cyan
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== cargo clippy ==" -ForegroundColor Cyan
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Write-Host "== cargo test ==" -ForegroundColor Cyan
cargo test --workspace
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Push-Location apps/desktop
try {
    Write-Host "== frontend deps ==" -ForegroundColor Cyan
    if (-not (Test-Path node_modules)) { npm install --no-audit --no-fund }
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "== tsc typecheck ==" -ForegroundColor Cyan
    npm run typecheck
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "== eslint ==" -ForegroundColor Cyan
    npm run lint
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    Write-Host "== vite build ==" -ForegroundColor Cyan
    npm run build
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}
finally {
    Pop-Location
}

Write-Host "All gates passed." -ForegroundColor Green
