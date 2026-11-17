[CmdletBinding()]
param(
    [switch]$Strict
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$failures = [System.Collections.Generic.List[string]]::new()

function Add-Failure([string]$Message) {
    $failures.Add($Message)
}

$manifest = Join-Path $root "Cargo.toml"
$inventoryPath = Join-Path $root "THIRD_PARTY_LICENSES.md"
$packageLock = Join-Path $root "apps\desktop\package-lock.json"

if (-not (Test-Path -LiteralPath $inventoryPath -PathType Leaf)) {
    Add-Failure "Third-party license inventory is missing."
}
if (-not (Test-Path -LiteralPath $packageLock -PathType Leaf)) {
    Add-Failure "Frontend lockfile is missing."
}

$metadata = $null
try {
    Push-Location $root
    $metadata = (& cargo metadata --locked --format-version 1 | ConvertFrom-Json)
    if ($LASTEXITCODE -ne 0) {
        Add-Failure "cargo metadata --locked failed."
    }
} catch {
    Add-Failure "Could not inspect Cargo.lock: $($_.Exception.Message)"
} finally {
    Pop-Location
}

if ($null -ne $metadata) {
    $external = @($metadata.packages | Where-Object { $_.source })
    $missingLicenses = @($external | Where-Object {
            [string]::IsNullOrWhiteSpace([string]$_.license)
        })
    if ($missingLicenses.Count -gt 0) {
        $names = ($missingLicenses | ForEach-Object { $_.name } | Sort-Object -Unique) -join ", "
        Add-Failure "Cargo packages without license metadata: $names"
    }
    Write-Output "Cargo registry packages checked: $($external.Count)"
}

if (Test-Path -LiteralPath $packageLock -PathType Leaf) {
    $script = @"
const fs = require('fs');
const lock = JSON.parse(fs.readFileSync('$($packageLock.Replace('\', '/'))', 'utf8'));
const manifest = JSON.parse(fs.readFileSync('$((Join-Path $root "apps/desktop/package.json").Replace('\', '/'))', 'utf8'));
const declared = new Set([...Object.keys(manifest.dependencies || {}), ...Object.keys(manifest.devDependencies || {})]);
let missing = [];
for (const name of declared) {
    if (!lock.packages['node_modules/' + name]) missing.push(name);
}
if (missing.length > 0) {
    console.error('MISSING:' + missing.join(', '));
    process.exit(1);
}
console.log(declared.size);
"@
    $nodeRes = & node -e $script 2>&1
    if ($LASTEXITCODE -ne 0) {
        Add-Failure "Frontend dependencies missing in lockfile: $nodeRes"
    } else {
        Write-Output "Frontend direct dependencies checked: $nodeRes"
    }
}

if (Test-Path -LiteralPath $inventoryPath -PathType Leaf) {
    $inventory = Get-Content -LiteralPath $inventoryPath -Raw
    if ($inventory -notmatch "tauri-plugin-opener") {
        Add-Failure "The Tauri opener dependency is missing from the license inventory."
    }
    if ($inventory -notmatch "FFmpeg") {
        Add-Failure "The FFmpeg decision row is missing from the license inventory."
    }
    if ($Strict -and $inventory -match '(?m)^\|.*\|\s*Provisional\s*\|\s*$') {
        Add-Failure "License inventory still contains provisional approvals."
    }
    if ($Strict -and -not (Test-Path -LiteralPath (Join-Path $root 'licenses/THIRD_PARTY_NOTICES.txt'))) {
        Add-Failure 'Generated third-party notices are missing.'
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Error $failure -ErrorAction Continue
    }
    exit 1
}

Write-Output "Dependency verification passed."
