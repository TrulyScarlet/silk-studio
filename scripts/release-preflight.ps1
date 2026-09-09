[CmdletBinding()]
param(
    [switch]$Release,
    [switch]$RunAudits,
    [switch]$SkipMediaTools
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$failures = [System.Collections.Generic.List[string]]::new()
$warnings = [System.Collections.Generic.List[string]]::new()

function Add-Failure([string]$Message) {
    $failures.Add($Message)
}

function Add-Warning([string]$Message) {
    $warnings.Add($Message)
}

function Has-Command([string]$Name) {
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Require-Command([string]$Name, [bool]$ForRelease) {
    if (-not (Has-Command $Name)) {
        if ($ForRelease) {
            Add-Failure "Required release tool '$Name' is not installed or not on PATH."
        } else {
            Add-Warning "Release tool '$Name' is not installed or not on PATH."
        }
        return $false
    }
    return $true
}

foreach ($relativePath in @(
        "Cargo.lock",
        "apps\desktop\package-lock.json",
        "apps\desktop\src-tauri\tauri.conf.json",
        "LICENSE",
        "THIRD_PARTY_LICENSES.md",
        "rust-toolchain.toml",
        "scripts\build-release.ps1"
    )) {
    if (-not (Test-Path -LiteralPath (Join-Path $root $relativePath) -PathType Leaf)) {
        Add-Failure "Required release input is missing: $relativePath"
    }
}

$tauriPath = Join-Path $root "apps\desktop\src-tauri\tauri.conf.json"
if (Test-Path -LiteralPath $tauriPath -PathType Leaf) {
    $tauri = Get-Content -LiteralPath $tauriPath -Raw | ConvertFrom-Json
    if ([string]::IsNullOrWhiteSpace([string]$tauri.app.security.csp)) {
        Add-Failure "Tauri production CSP is missing."
    }
    if (-not ($tauri.bundle.targets -contains "nsis")) {
        Add-Failure "Tauri bundle must retain the NSIS target."
    }
    if ([string]::IsNullOrWhiteSpace([string]$tauri.bundle.licenseFile)) {
        Add-Failure "Tauri bundle licenseFile is missing."
    } else {
        $licensePath = Join-Path (Split-Path $tauriPath -Parent) ([string]$tauri.bundle.licenseFile)
        if (-not (Test-Path -LiteralPath $licensePath -PathType Leaf)) {
            Add-Failure "Tauri bundle licenseFile does not resolve: $($tauri.bundle.licenseFile)"
        }
    }
    if ([string]$tauri.bundle.windows.nsis.installMode -ne "currentUser") {
        Add-Failure "NSIS installMode must remain currentUser until per-machine elevation is reviewed."
    }
    if ([string]$tauri.bundle.windows.webviewInstallMode.type -ne "downloadBootstrapper") {
        Add-Failure "WebView2 install mode must be explicitly configured."
    }
}

$toolchainPath = Join-Path $root "rust-toolchain.toml"
if (Test-Path -LiteralPath $toolchainPath -PathType Leaf) {
    $toolchain = Get-Content -LiteralPath $toolchainPath -Raw
    if ($toolchain -notmatch 'channel\s*=\s*"1\.98\.0"') {
        Add-Failure "rust-toolchain.toml must pin Rust 1.98.0 for release builds."
    }
}

$packagePath = Join-Path $root "apps\desktop\package.json"
$cargoPath = Join-Path $root "Cargo.toml"
$tauriVersion = if (Test-Path -LiteralPath $tauriPath -PathType Leaf) { [string]$tauri.version } else { "" }
$packageVersion = if (Test-Path -LiteralPath $packagePath -PathType Leaf) {
    [string](Get-Content -LiteralPath $packagePath -Raw | ConvertFrom-Json).version
} else {
    ""
}
$cargoVersion = if (Test-Path -LiteralPath $cargoPath -PathType Leaf) {
    $cargoMatch = Select-String -LiteralPath $cargoPath -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
    if ($null -ne $cargoMatch) { $cargoMatch.Matches[0].Groups[1].Value } else { "" }
} else {
    ""
}
if ($tauriVersion -ne $packageVersion -or $tauriVersion -ne $cargoVersion) {
    Add-Failure "Version mismatch: Cargo=$cargoVersion package.json=$packageVersion tauri=$tauriVersion"
}

if (Require-Command "cargo" $Release) {
    try {
        Push-Location $root
        & cargo metadata --locked --no-deps --format-version 1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Add-Failure "Cargo.lock is not usable with --locked (exit code $LASTEXITCODE)."
        }
    } catch {
        Add-Failure "Cargo.lock is not usable with --locked: $($_.Exception.Message)"
    } finally {
        Pop-Location
    }
}
Require-Command "rustc" $Release | Out-Null
Require-Command "node" $Release | Out-Null
Require-Command "npm" $Release | Out-Null
Require-Command "cargo-tauri" $Release | Out-Null
Require-Command "makensis" $Release | Out-Null
Require-Command "signtool" $Release | Out-Null

$signingVariables = @(
    "SILK_WINDOWS_CERTIFICATE_THUMBPRINT",
    "SILK_WINDOWS_TIMESTAMP_URL"
)
foreach ($name in $signingVariables) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        if ($Release) {
            Add-Failure "Release signing variable '$name' is not configured; unsigned artifacts are not releasable."
        } else {
            Add-Warning "Release signing variable '$name' is not configured; this checkout cannot produce a signed release."
        }
    }
}

foreach ($name in @("SILK_BUILD_COMMIT", "SILK_BUILD_DATE")) {
    if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name))) {
        if ($Release) {
            Add-Failure "Build provenance variable '$name' is not configured."
        } else {
            Add-Warning "Build provenance variable '$name' is not configured; metadata will remain unset."
        }
    }
}

if (-not $SkipMediaTools) {
    foreach ($name in @("ffmpeg", "ffprobe")) {
        if (-not (Has-Command $name)) {
            if ($Release) {
                Add-Failure "Media validation tool '$name' is unavailable. Use -SkipMediaTools only for a non-media package build."
            } else {
                Add-Warning "Media validation tool '$name' is unavailable; media acceptance remains unverified."
            }
        }
    }
}

if ($RunAudits -or $Release) {
    $dependencyScript = Join-Path $PSScriptRoot "verify-dependencies.ps1"
    if (Test-Path -LiteralPath $dependencyScript -PathType Leaf) {
        try {
            $dependencyArgs = @("-NoProfile", "-File", $dependencyScript)
            if ($Release) {
                $dependencyArgs += "-Strict"
            }
            & pwsh @dependencyArgs
            if ($LASTEXITCODE -ne 0) {
                Add-Failure "Dependency verification failed."
            }
        } catch {
            Add-Failure "Dependency verification failed: $($_.Exception.Message)"
        }
    }
    if (Require-Command "cargo-audit" $Release) {
        try {
            Push-Location $root
            & cargo audit
            if ($LASTEXITCODE -ne 0) {
                Add-Failure "cargo audit exited with code $LASTEXITCODE."
            }
        } catch {
            Add-Failure "cargo audit failed: $($_.Exception.Message)"
        } finally {
            Pop-Location
        }
    }
    if (Require-Command "npm" $Release) {
        try {
            Push-Location (Join-Path $root "apps\desktop")
            & npm audit --omit=dev --audit-level=high
            if ($LASTEXITCODE -ne 0) {
                Add-Failure "npm audit exited with code $LASTEXITCODE."
            }
        } catch {
            Add-Failure "npm audit failed: $($_.Exception.Message)"
        } finally {
            Pop-Location
        }
    }
}

foreach ($warning in $warnings) {
    Write-Warning $warning
}
foreach ($failure in $failures) {
    Write-Error $failure -ErrorAction Continue
}

if ($failures.Count -gt 0) {
    exit 1
}

Write-Output "Release preflight passed with $($warnings.Count) warning(s)."
