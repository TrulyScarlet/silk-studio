[CmdletBinding()]
param([Parameter(Mandatory = $true)][int]$GameProcessId, [switch]$Automatic)
$ErrorActionPreference = 'Stop'
$game = Get-Process -Id $GameProcessId
if ($game.ProcessName -ne 'VALORANT-Win64-Shipping' -or $game.MainWindowHandle -eq 0) {
    throw 'Explicit running VALORANT window required.'
}
$output = Join-Path $env:LOCALAPPDATA ('Temp\opencode\valorant-guarded-shield-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $output | Out-Null
$env:SILK_GAME_REPLAY_APPROVAL = 'valorant-guarded-only'
$env:SILK_GAME_PROBE_PID = [string]$game.Id
$env:SILK_GAME_PROBE_HWND = [string]$game.MainWindowHandle
$env:SILK_GAME_PROBE_CREATION = [string]$game.StartTime.ToFileTimeUtc()
$env:SILK_GAME_PROBE_OUTPUT = $output
$env:SILK_GAME_REPLAY_FFMPEG = Join-Path $env:LOCALAPPDATA 'Silk\tools\ffmpeg\bin\ffmpeg.exe'
$env:SILK_EXPECT_CONTINUOUS_FOCUS = '1'
$job = Start-Job -ArgumentList $game.Id, ([long]$game.MainWindowHandle), $output -ScriptBlock {
    param($gamePid, $gameHandle, $output)
    Add-Type -AssemblyName System.Windows.Forms
    $ErrorActionPreference = 'Stop'
    Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public class SilkShieldTrace { [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow(); [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint p); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h); [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int command); }'
    function FocusState {
        $handle = [SilkShieldTrace]::GetForegroundWindow()
        [uint32]$processId = 0
        [void][SilkShieldTrace]::GetWindowThreadProcessId($handle, [ref]$processId)
        $line = "$(Get-Date -Format o) foreground_pid=$processId hwnd=$handle"
        Add-Content -LiteralPath (Join-Path $output 'focus-log.txt') -Value $line
        $line
    }
    function RequireGameFocus {
        [void][SilkShieldTrace]::ShowWindow([IntPtr]$gameHandle, 9)
        [void][SilkShieldTrace]::SetForegroundWindow([IntPtr]$gameHandle)
        $deadline = (Get-Date).AddSeconds(3)
        while ([SilkShieldTrace]::GetForegroundWindow() -ne [IntPtr]$gameHandle -and (Get-Date) -lt $deadline) {
            [System.Windows.Forms.Application]::DoEvents()
            Start-Sleep -Milliseconds 20
        }
        FocusState
        if ([SilkShieldTrace]::GetForegroundWindow() -ne [IntPtr]$gameHandle) { throw 'Windows denied game focus; focus-cycle acceptance not verified' }
    }
    $deadline = (Get-Date).AddSeconds(60)
    while (-not (Test-Path (Join-Path $output 'capture-started'))) {
        if ((Get-Date) -gt $deadline) { throw 'Capture start timeout' }
        Start-Sleep -Milliseconds 100
    }
    RequireGameFocus
    Start-Sleep -Seconds 5
    for ($cycle=0; $cycle -lt 2; $cycle++) {
    $form = New-Object System.Windows.Forms.Form
    try {
        $form.Text = 'Silk guarded focus test'
        $form.Width = 600
        $form.Height = 300
        $form.BackColor = [System.Drawing.Color]::Magenta
        $form.Show()
        $form.Activate()
        "Owned window activation: $([SilkShieldTrace]::SetForegroundWindow($form.Handle))"
        $until = (Get-Date).AddSeconds(2)
        while ((Get-Date) -lt $until) {
            [System.Windows.Forms.Application]::DoEvents()
            if ([SilkShieldTrace]::GetForegroundWindow() -ne $form.Handle) {
                $form.Activate()
                [void][SilkShieldTrace]::SetForegroundWindow($form.Handle)
            }
            Start-Sleep -Milliseconds 20
        }
        FocusState
        if ([SilkShieldTrace]::GetForegroundWindow() -ne $form.Handle) { throw 'Windows denied away focus; acceptance not verified' }
        $until = (Get-Date).AddSeconds(4)
        while ((Get-Date) -lt $until) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20 }
        RequireGameFocus
    } finally { $form.Close(); $form.Dispose() }
    Start-Sleep -Seconds 3
    }
    Set-Content -LiteralPath (Join-Path $output 'focus-cycles-verified') -Value '2'
}
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    Write-Output "Evidence directory: $output"
    $test = if ($Automatic) { 'automatic_game_focus_replay' } else { 'selected_valorant_guarded_hardware_replay' }
    cargo test -p silk --test native_selected_game_replay $test -- --ignored --nocapture
    $testExit = $LASTEXITCODE
    Wait-Job $job -Timeout 30 | Out-Null
    if ($job.State -ne 'Completed') {
        Receive-Job $job -ErrorAction Continue | Out-File (Join-Path $output 'helper-result.txt')
        throw "Focus helper did not complete: $($job.State)"
    }
    Receive-Job $job | Tee-Object -FilePath (Join-Path $output 'helper-result.txt')
    if (-not (Test-Path (Join-Path $output 'focus-cycles-verified'))) { throw 'Two complete focus cycles were not verified' }
    if ($testExit -ne 0) { throw "Replay test failed: $testExit" }
    & $env:SILK_GAME_REPLAY_FFMPEG -nostdin -hide_banner -i (Join-Path $output 'valorant-guarded-replay.mp4') -vf 'blackdetect=d=0.15:pix_th=0.05' -an -fps_mode passthrough -enc_time_base demux -f null - 2>&1 | Tee-Object -FilePath (Join-Path $output 'black-detection.txt')
    if ($LASTEXITCODE -ne 0) { throw 'Black detection decode failed' }
    & $env:SILK_GAME_REPLAY_FFMPEG -nostdin -hide_banner -i (Join-Path $output 'valorant-guarded-replay.mp4') -vf 'freezedetect=n=-50dB:d=1' -an -fps_mode passthrough -enc_time_base demux -f null - 2>&1 | Tee-Object -FilePath (Join-Path $output 'freeze-detection.txt')
    if ($LASTEXITCODE -ne 0) { throw 'Freeze detection decode failed' }
} finally {
    Stop-Job $job -ErrorAction SilentlyContinue
    Remove-Job $job -Force -ErrorAction SilentlyContinue
    Pop-Location
}
