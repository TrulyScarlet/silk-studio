param([string]$ArchivePath = "$PSScriptRoot/../target/ffmpeg-9.0.1-full_build.zip")
$ErrorActionPreference = 'Stop'
$installer = Join-Path $PSScriptRoot '../apps/desktop/src-tauri/windows/install-media-tools.ps1'
$testRoot = Join-Path $PSScriptRoot ('../target/media-test-' + [guid]::NewGuid().ToString('N'))
$validRoot = Join-Path $testRoot 'Silk with spaces'
$badRoot = Join-Path $testRoot 'failure'
[IO.Directory]::CreateDirectory($testRoot) | Out-Null
$badArchive = Join-Path $testRoot 'corrupt.zip'
[IO.File]::WriteAllText($badArchive, 'corrupt archive')
function Run-Setup([string]$folder, [string]$archive, [int]$expectedExit) {
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $installer -InstallDir $folder -ArchivePath $archive
    if ($LASTEXITCODE -ne $expectedExit) { throw "Unexpected setup exit: $LASTEXITCODE (expected $expectedExit)" }
}
Run-Setup $validRoot $ArchivePath 0
# Reproduce an installer launched with another PowerShell version's module path.
$savedModules = $env:PSModulePath
try { $env:PSModulePath = 'C:\nonexistent-module-path'; Run-Setup $validRoot $badArchive 0 }
finally { $env:PSModulePath = $savedModules }
# A valid installed pair must not download or read the supplied bad archive.
Run-Setup $validRoot $badArchive 0
Run-Setup $badRoot $badArchive 1
if (Test-Path (Join-Path $badRoot 'tools/ffmpeg/bin/ffmpeg.exe')) { throw 'Bad archive installed an executable' }
$exe = Join-Path $validRoot 'tools/ffmpeg/bin/ffmpeg.exe'
[IO.File]::WriteAllText($exe, 'damaged installed file')
Run-Setup $validRoot $badArchive 1
if ([IO.File]::ReadAllText($exe) -ne 'damaged installed file') { throw 'Failed repair modified the previous installation' }
Run-Setup $validRoot $ArchivePath 0
if ((Get-FileHash $exe).Hash -ne '57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A') { throw 'Repair failed' }
if (Get-ChildItem $testRoot -Directory -Recurse -Filter 'media-setup-*') { throw 'Staging directories leaked' }
Write-Output 'PASS: fresh install, spaces in paths, offline reuse, corrupt archive rejection, failed repair preservation, successful repair, and staging cleanup.'
