[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

& pwsh -NoProfile -File (Join-Path $PSScriptRoot "release-preflight.ps1") -Release
if ($LASTEXITCODE -ne 0) {
    throw "Release preflight failed; no release build was attempted."
}

Push-Location $root
try {
    & cargo tauri build --config "apps\desktop\src-tauri\tauri.conf.json" --ci
    if ($LASTEXITCODE -ne 0) {
        throw "cargo tauri build failed with exit code $LASTEXITCODE."
    }
} finally {
    Pop-Location
}

$bundleDirectory = Join-Path $root "target\release\bundle"
$certificate = [Environment]::GetEnvironmentVariable("SILK_WINDOWS_CERTIFICATE_THUMBPRINT")
$timestampUrl = [Environment]::GetEnvironmentVariable("SILK_WINDOWS_TIMESTAMP_URL")
$artifacts = @(
    Get-ChildItem -LiteralPath (Join-Path $root "target\release") -File -ErrorAction Stop |
        Where-Object { $_.Extension -ieq ".exe" }
    Get-ChildItem -LiteralPath $bundleDirectory -Recurse -File -ErrorAction Stop |
        Where-Object { $_.Extension -ieq ".exe" }
) | Sort-Object -Property FullName -Unique
if ($artifacts.Count -eq 0) {
    throw "No executable release artifacts were found under $bundleDirectory."
}

foreach ($artifact in $artifacts) {
    & signtool sign /sha1 $certificate /fd SHA256 /tr $timestampUrl /td SHA256 $artifact.FullName
    if ($LASTEXITCODE -ne 0) {
        throw "signtool failed for $($artifact.Name) with exit code $LASTEXITCODE."
    }
    & signtool verify /pa /all $artifact.FullName
    if ($LASTEXITCODE -ne 0) {
        throw "signature verification failed for $($artifact.Name)."
    }
}

$manifestPath = Join-Path $bundleDirectory "SHA256SUMS.txt"
$lines = foreach ($artifact in $artifacts | Sort-Object FullName) {
    $relative = [System.IO.Path]::GetRelativePath($root, $artifact.FullName)
    $hash = (Get-FileHash -LiteralPath $artifact.FullName -Algorithm SHA256).Hash
    "$hash  $relative"
}
Set-Content -LiteralPath $manifestPath -Value $lines -Encoding ascii
Write-Output "Signed artifacts and hash manifest written to $manifestPath"
