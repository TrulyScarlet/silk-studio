[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Installer,
    [Parameter(Mandatory)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [string]$Notes = 'Silk capture, playback, sharing, and reliability improvements.'
)
$ErrorActionPreference = 'Stop'
$file = Get-Item -LiteralPath $Installer
if ($file.Name -ne "Silk_${Version}_x64-setup.exe") { throw 'Installer filename must match the release version.' }
$signature = (Get-Content -LiteralPath "$Installer.sig" -Raw).Trim()
if (-not $signature) { throw 'Updater signature missing.' }
$decoded = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($signature))
if ($decoded -notmatch "(?m)^trusted comment:.*version[:=]$([regex]::Escape($Version))(?:\s|$)") {
    throw 'Signature must bind the exact release version. Rebuild with the current Tauri CLI.'
}
$manifest = @{
    version=$Version; notes=$Notes; pub_date=[DateTime]::UtcNow.ToString('o')
    platforms=@{'windows-x86_64'=@{
        url="https://github.com/TrulyScarlet/silk-studio/releases/download/v$Version/$($file.Name)"
        signature=$signature
    }}
}
$manifest | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $file.DirectoryName 'latest.json') -Encoding utf8
$hashes = foreach ($path in @($file.FullName,"$Installer.sig",(Join-Path $file.DirectoryName 'latest.json'))) {
    "{0}  {1}" -f (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash, [IO.Path]::GetFileName($path)
}
$hashes | Set-Content -LiteralPath (Join-Path $file.DirectoryName 'SHA256SUMS.txt') -Encoding ascii
