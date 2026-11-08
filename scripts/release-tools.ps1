# Use installed release tools without changing the user's permanent PATH.
$toolDirectories = @(
    (Join-Path $env:LOCALAPPDATA 'tauri\NSIS\Bin'),
    (Join-Path $env:LOCALAPPDATA 'Silk\tools\ffmpeg\bin'),
    (Join-Path $env:USERPROFILE '.cargo\bin')
)
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
if (Test-Path -LiteralPath $sdkRoot) {
    $sdk = Get-ChildItem -LiteralPath $sdkRoot -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending | Select-Object -First 1
    if ($sdk) { $toolDirectories += Join-Path $sdk.FullName 'x64' }
}
$env:PATH = (($toolDirectories | Where-Object { Test-Path -LiteralPath $_ }) -join ';') + ';' + $env:PATH
