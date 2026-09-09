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
    $lock = Get-Content -LiteralPath $packageLock -Raw | ConvertFrom-Json -AsHashtable
    $rootPackage = $lock.packages['']
    $packageManifest = Get-Content -LiteralPath (Join-Path $root "apps\desktop\package.json") -Raw | ConvertFrom-Json
    $declared = @(
        $packageManifest.dependencies.PSObject.Properties | ForEach-Object { $_.Name }
        $packageManifest.devDependencies.PSObject.Properties | ForEach-Object { $_.Name }
    ) | Sort-Object -Unique
    foreach ($name in $declared) {
        $lockKey = "node_modules/$name"
        if (-not $lock.packages.ContainsKey($lockKey)) {
            Add-Failure "Frontend dependency '$name' is not represented in package-lock.json."
        }
    }
    Write-Output "Frontend direct dependencies checked: $($declared.Count)"
}

if (Test-Path -LiteralPath $inventoryPath -PathType Leaf) {
    $inventory = Get-Content -LiteralPath $inventoryPath -Raw
    if ($inventory -notmatch "tauri-plugin-opener") {
        Add-Failure "The Tauri opener dependency is missing from the license inventory."
    }
    if ($inventory -notmatch "FFmpeg") {
        Add-Failure "The FFmpeg decision row is missing from the license inventory."
    }
    if ($Strict -and $inventory -match "Provisional") {
        Add-Failure "License inventory still contains provisional approvals."
    }
}

if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Error $failure -ErrorAction Continue
    }
    exit 1
}

Write-Output "Dependency verification passed."
