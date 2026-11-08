# Opt-in real Windows test: replaces the installed app with a test baseline,
# rejects two bad updates, then upgrades back to the production-configured build.
# Run serve-update-test.mjs first. Never upload the baseline installer.
[CmdletBinding()]
param([switch]$Live)
if (-not $Live) { throw 'Pass -Live explicitly; this test restarts the installed Silk application.' }
$ErrorActionPreference='Stop'
Add-Type -AssemblyName UIAutomationClient,UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SilkUpgradeWindow {
 [DllImport("user32.dll")] public static extern bool ShowWindowAsync(IntPtr window,int command);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
}
'@
$root=Split-Path -Parent $PSScriptRoot
$installed=Join-Path $env:LOCALAPPDATA 'Silk\silk.exe'
$baseline=Join-Path $root 'target\release\bundle\nsis\Silk_0.0.9_x64-setup.exe'
$production=Join-Path $root 'target\release\bundle\nsis\Silk_0.1.1_x64-setup.exe'
$config=Join-Path $env:APPDATA 'dev.silk.project\config.json'
$catalog=Join-Path (Split-Path $config) 'library.json'
$mode=Join-Path $root 'target\updater-test-mode.txt'
foreach ($path in @($baseline,$production,"$production.sig",$config,$catalog)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Missing test input: $path" }
}
function Silk-Process { Get-Process silk -ErrorAction SilentlyContinue | Where-Object {$_.Path -eq $installed} }
function Silk-Root {
    $process=Silk-Process
    if (-not $process) { return $null }
    $condition=[System.Windows.Automation.AndCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$process.Id),
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'Silk'))
    [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children,$condition)
}
function Find-Control([string]$Name) {
    $window=Silk-Root
    if (-not $window) {return $null}
    $condition=[System.Windows.Automation.AndCondition]::new(
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$Name),
        [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,[System.Windows.Automation.ControlType]::Button))
    $window.FindFirst([System.Windows.Automation.TreeScope]::Descendants,$condition)
}
function Invoke-Control([string]$Name) {
    $control=Find-Control $Name
    if (-not $control -or -not $control.Current.IsEnabled) {throw "Control unavailable: $Name"}
    $pattern=$null
    if($control.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern,[ref]$pattern)){$pattern.Invoke()}
    elseif($control.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern,[ref]$pattern)){$pattern.Expand()}
    else{throw "Control has no supported action: $Name"}
    Start-Sleep -Milliseconds 300
}
function Wait-Until([scriptblock]$Condition,[string]$Description,[int]$Seconds=60) {
    $deadline=[DateTime]::UtcNow.AddSeconds($Seconds)
    do { try { if (& $Condition) {return} } catch {} ; Start-Sleep -Milliseconds 350 } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out: $Description"
}
function UI-Text {
    $window=Silk-Root
    if(-not $window){return ''}
    (($window.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition) | ForEach-Object {$_.Current.Name}) -join "`n")
}
function Show-Silk {
    $window=Silk-Root
    [SilkUpgradeWindow]::ShowWindowAsync([IntPtr]$window.Current.NativeWindowHandle,9) | Out-Null
    [SilkUpgradeWindow]::SetForegroundWindow([IntPtr]$window.Current.NativeWindowHandle) | Out-Null
}
function Snapshot-Data {
    $settings=Get-Content -LiteralPath $config -Raw | ConvertFrom-Json
    $output=$settings.settings.output.directory
    if((Split-Path $output -Leaf) -ne 'Silk'){$output=Join-Path $output 'Silk'}
    $paths=@($config,$catalog)+@(Get-ChildItem -LiteralPath $output -Filter '*.mp4' -Recurse -File | Select-Object -ExpandProperty FullName)
    $hashes=@{}
    foreach($path in $paths){$hashes[$path]=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash}
    $hashes
}
if(Find-Control 'Close share dialog'){Invoke-Control 'Close share dialog'}
if((Find-Control 'Stop').Current.IsEnabled){Invoke-Control 'Stop'; Start-Sleep -Seconds 2}
$before=Snapshot-Data
$before | ConvertTo-Json | Set-Content (Join-Path $root 'target\updater-data-before.json')
Write-Output "Recorded hashes for $($before.Count) settings/catalog/clip files."
'corrupt' | Set-Content -LiteralPath $mode
Silk-Process | Stop-Process
$setup=Start-Process -FilePath $baseline -ArgumentList '/S','/UPDATE' -WindowStyle Hidden -Wait -PassThru
if($setup.ExitCode -ne 0){throw "Baseline installation failed: $($setup.ExitCode)"}
Start-Process -FilePath $installed -WindowStyle Hidden
Wait-Until {Find-Control 'New update'} 'baseline update notification'
Show-Silk
$baselinePid=(Silk-Process).Id
$baselineHash=(Get-FileHash -LiteralPath $installed).Hash
Invoke-Control 'New update'
Invoke-Control 'Update & restart'
Wait-Until {(UI-Text) -match 'Update verification failed'} 'corrupt artifact rejection'
if((Silk-Process).Id -ne $baselinePid -or (Get-FileHash -LiteralPath $installed).Hash -ne $baselineHash){throw 'Rejected update changed the installed app'}
Write-Output 'PASS: Corrupted installer rejected; current process and executable preserved.'

'wrong-version' | Set-Content -LiteralPath $mode
Invoke-Control 'Open Settings dialog'
Invoke-Control 'System'
Invoke-Control 'Check for updates'
Wait-Until {(UI-Text) -match 'Silk 0.1.2 is available'} 'wrong-version manifest loaded'
Invoke-Control 'Update & restart'
Wait-Until {(UI-Text) -match 'Update verification failed'} 'signed version mismatch rejected'
if((Silk-Process).Id -ne $baselinePid){throw 'Version mismatch restarted app'}
Write-Output 'PASS: Signed artifact with a mismatched announced version rejected.'

'good' | Set-Content -LiteralPath $mode
Invoke-Control 'Check for updates'
Wait-Until {(UI-Text) -match 'Silk 0.1.1 is available'} 'valid update available'
Invoke-Control 'Update & restart'
Wait-Until { $process=Silk-Process; $process -and $process.Id -ne $baselinePid -and (Find-Control 'Open Settings dialog') } 'automatic upgrade and restart' 120
Show-Silk
Wait-Until {(Get-Item -LiteralPath $installed).VersionInfo.ProductVersion -eq '0.1.1'} 'installed production version'
$after=Snapshot-Data
foreach($path in $before.Keys){if($after[$path] -ne $before[$path]){throw "Data changed across upgrade: $path"}}
if($after.Count -ne $before.Count){throw 'Clip file count changed across upgrade'}
foreach($name in @('config.json','library.json')){
    $original=Join-Path (Split-Path $config) $name
    $backup=Join-Path (Split-Path $config) "update-backup\$name"
    if((Get-FileHash -LiteralPath $backup).Hash -ne $before[$original]){throw "Invalid upgrade backup: $name"}
}
Write-Output "PASS: 0.0.9 -> 0.1.1 downloaded, installed and restarted from one update action; all $($after.Count) data-file hashes and backups match."
