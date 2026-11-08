[CmdletBinding()]
param([switch]$WithoutPublisherSignature)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'release-tools.ps1')
Push-Location $root
try {
    $sourceChanges = git status --porcelain | Where-Object {
        $_ -ne '?? .agents/' -and $_ -ne '?? skills-lock.json'
    }
    if ($sourceChanges) { throw 'Commit the release source before building distributable artifacts.' }
    $env:SILK_BUILD_COMMIT = (git rev-parse HEAD).Trim()
    $env:SILK_BUILD_DATE = (git show -s --format=%cI HEAD).Trim()
    $env:SILK_RELEASE_BUILD = '1'
    if (-not $env:TAURI_SIGNING_PRIVATE_KEY) {
        $localKey = Join-Path $env:LOCALAPPDATA 'Silk\signing\updater.key'
        if (Test-Path -LiteralPath $localKey) { $env:TAURI_SIGNING_PRIVATE_KEY = $localKey }
    }
    & (Join-Path $PSScriptRoot 'release-preflight.ps1') -Release -RunAudits -WithoutPublisherSignature:$WithoutPublisherSignature
    if ($LASTEXITCODE -ne 0) { throw 'Release preflight failed; no release was built.' }
    # Sign the exe before bundling, then the installer before the updater signature.
    $signingConfig = Join-Path $root 'target\release-signing.json'
    if (-not $WithoutPublisherSignature) { @{ bundle = @{ windows = @{
        certificateThumbprint = $env:SILK_WINDOWS_CERTIFICATE_THUMBPRINT
        timestampUrl = $env:SILK_WINDOWS_TIMESTAMP_URL
        tsp = $true
    } } } | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $signingConfig -Encoding utf8 }
    Push-Location (Join-Path $root 'apps\desktop')
    try {
        $buildArgs = @('tauri', 'build', '--bundles', 'nsis', '--ci')
        if (-not $WithoutPublisherSignature) { $buildArgs += @('--config', $signingConfig) }
        & cargo @buildArgs
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    } finally { Pop-Location }
    $version = (Get-Content apps/desktop/src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
    $installer = Join-Path $root "target\release\bundle\nsis\Silk_${version}_x64-setup.exe"
    if (-not $WithoutPublisherSignature) { foreach ($artifact in @((Join-Path $root 'target\release\silk.exe'), $installer)) {
        & signtool verify /pa /all $artifact
        if ($LASTEXITCODE -ne 0) { throw "Publisher signature verification failed: $artifact" }
    } }
    & (Join-Path $PSScriptRoot 'write-update-manifest.ps1') -Installer $installer -Version $version
    Write-Output "Release ready: $installer and latest.json. Publish both with the .sig and SHA256SUMS.txt on GitHub release v$version."
} finally { Pop-Location }
