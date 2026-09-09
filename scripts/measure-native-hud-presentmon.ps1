<#
.SYNOPSIS
    Orchestrates PresentMon v2 frame-time capture during native HUD benchmark stimulation.

.DESCRIPTION
    Developer-only measurement orchestrator for Phase 3/4 native HUD gating.
    Enforces administrator elevation, verifies running target processes, starts a dedicated
    PresentMon v2 session with unique session isolation and timed capture, runs benchmark stimulation,
    captures markers, and automatically runs analyze-native-hud-presentmon.ps1.

.PARAMETER PresentMonPath
    Path to the official PresentMon v2 executable.

.PARAMETER ProcessName
    Target running game process name to trace (mutually exclusive with ProcessId). Target process must already be running.

.PARAMETER ProcessId
    Target running game process ID to trace (mutually exclusive with ProcessName). Target process must already be running.

.PARAMETER BenchmarkPath
    Optional path to benchmark executable (affordance stimulus). Target process must already be running.

.PARAMETER BenchmarkArgs
    Optional additional arguments passed to the benchmark executable.

.PARAMETER MarkerCsvPath
    Optional path to an existing or generated marker CSV. If BenchmarkPath is specified,
    defaults to markers.csv inside the session directory.

.PARAMETER OutputDir
    Directory under which the unique session evidence directory will be created.

.PARAMETER State
    HUD state under test: no-hud, idle, saved, queued-to-saved, failed, rejected. Default: 'saved'.

.PARAMETER Mode
    HUD presentation mode: full, compact. Default: 'full'.

.PARAMETER Anchor
    HUD anchor position (e.g. 'bottom-center', 'top-left', 'top-right'). Default: 'bottom-center'.

.PARAMETER CaptureExclusion
    HUD capture exclusion (WDA_EXCLUDEFROMCAPTURE): on, off. Default: 'on'.

.PARAMETER LifecyclePolicy
    HUD diagnostic lifecycle policy: attached-shown, detached-shown, attached-hidden. Default: 'attached-shown'.

.PARAMETER Runs
    Number of measured cycles (must be > 0). Default: 10.

.PARAMETER Warmup
    Number of warmup cycles before measured cycles. Default: 2.

.PARAMETER IntervalMs
    Interval between marker cycles in milliseconds (minimum 5000). Default: 5000.

.PARAMETER TransitionMs
    Duration of HUD visibility / transition in milliseconds for queued-to-saved state. Default: 250.

.PARAMETER StartupLeadSeconds
    Lead time in seconds before first marker trigger to ensure baseline pre-window capture. Default: 2.0.

.PARAMETER TailSeconds
    Safety tail time in seconds after benchmark completes to ensure exit-window capture. Default: 3.0.

.PARAMETER SwapChainAddress
    Optional explicit swapchain selector passed to analyzer.

.PARAMETER AnalyzerPath
    Path to analyze-native-hud-presentmon.ps1. Defaults to sibling script.

.PARAMETER SelfTest
    Runs built-in verification of argument construction and duration math without requiring elevation.
#>
[CmdletBinding(DefaultParameterSetName = "ByName")]
param(
    [Parameter(Mandatory = $true, ParameterSetName = "ByName")]
    [Parameter(Mandatory = $true, ParameterSetName = "ById")]
    [string]$PresentMonPath,

    [Parameter(Mandatory = $true, ParameterSetName = "ByName")]
    [string]$ProcessName,

    [Parameter(Mandatory = $true, ParameterSetName = "ById")]
    [int]$ProcessId,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [string]$BenchmarkPath,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [string[]]$BenchmarkArgs = @(),

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [string]$MarkerCsvPath,

    [Parameter(Mandatory = $true, ParameterSetName = "ByName")]
    [Parameter(Mandatory = $true, ParameterSetName = "ById")]
    [string]$OutputDir,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateSet("no-hud", "idle", "saved", "queued-to-saved", "failed", "rejected")]
    [string]$State = "saved",

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateSet("full", "compact")]
    [string]$Mode = "full",

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateSet("top-left", "top-center", "top-right", "center-left", "center-right", "bottom-left", "bottom-center", "bottom-right", "top_left", "top_center", "top_right", "center_left", "center_right", "bottom_left", "bottom_center", "bottom_right")]
    [string]$Anchor = "bottom-center",

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateSet("on", "off")]
    [string]$CaptureExclusion = "on",

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateSet("attached-shown", "detached-shown", "attached-hidden")]
    [string]$LifecyclePolicy = "attached-shown",

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateRange(1, 1000)]
    [int]$Runs = 10,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [Alias("Warmups")]
    [ValidateRange(0, 100)]
    [int]$Warmup = 2,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [Alias("IntervalSeconds")]
    [ValidateRange(5000, 600000)]
    [int]$IntervalMs = 5000,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [Alias("TransitionSeconds")]
    [ValidateRange(1, 60000)]
    [int]$TransitionMs = 250,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateRange(1.0, 30.0)]
    [double]$StartupLeadSeconds = 2.0,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [ValidateRange(3.0, 60.0)]
    [double]$TailSeconds = 3.0,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [string]$SwapChainAddress,

    [Parameter(ParameterSetName = "ByName")]
    [Parameter(ParameterSetName = "ById")]
    [string]$AnalyzerPath,

    [Parameter(Mandatory = $true, ParameterSetName = "SelfTest")]
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"

function Get-CaptureDurationSeconds {
    <#
    .SYNOPSIS
        Calculates total PresentMon capture duration required to span startup lead,
        benchmark steady states, inter-trigger intervals, and analyzer exit windows.
    #>
    param(
        [int]$Runs,
        [int]$Warmup,
        [int]$IntervalMs,
        [int]$TransitionMs,
        [string]$State,
        [double]$StartupLeadSeconds,
        [double]$TailSeconds
    )

    $totalCycles = $Runs + $Warmup
    $transitionSec = if ($State -eq "queued-to-saved") { [double]($TransitionMs / 1000.0) } else { 0.0 }
    $cycleSpacingSec = [Math]::Max([double]($IntervalMs / 1000.0), $transitionSec)
    $interCycleSpanSec = if ($totalCycles -gt 1) { ($totalCycles - 1) * $cycleSpacingSec } else { 0.0 }
    $benchmarkPreSteadySec = 1.0
    $benchmarkPostSteadySec = 5.0
    $benchmarkActiveSec = $benchmarkPreSteadySec + $interCycleSpanSec + $benchmarkPostSteadySec
    $totalSec = $StartupLeadSeconds + $benchmarkActiveSec + $TailSeconds
    return [int][Math]::Ceiling($totalSec)
}

function Build-BenchmarkArguments {
    param(
        [string]$MarkersPath,
        [int]$Runs,
        [int]$Warmup,
        [int]$IntervalMs,
        [int]$TransitionMs,
        [string]$State,
        [string]$Mode,
        [string]$Anchor,
        [string]$CaptureExclusion = "on",
        [string]$LifecyclePolicy = "attached-shown",
        [string[]]$AdditionalArgs = @()
    )

    $benchArgsList = [System.Collections.Generic.List[string]]::new()
    $benchArgsList.AddRange([string[]]@(
        "--markers", $MarkersPath,
        "--runs", "$Runs",
        "--warmup", "$Warmup",
        "--interval-ms", "$IntervalMs",
        "--transition-ms", "$TransitionMs",
        "--state", $State,
        "--mode", $Mode,
        "--anchor", $Anchor,
        "--capture-exclusion", $CaptureExclusion,
        "--lifecycle-policy", $LifecyclePolicy
    ))
    if ($AdditionalArgs) {
        $benchArgsList.AddRange([string[]]$AdditionalArgs)
    }
    return $benchArgsList.ToArray()
}

function Build-PresentMonArguments {
    param(
        [string]$SessionName,
        [string]$OutputFile,
        [int]$TimedDurationSeconds,
        [string]$ProcessName,
        [int]$ProcessId
    )

    $pmArgs = [System.Collections.Generic.List[string]]::new()
    $pmArgs.AddRange([string[]]@(
        "--session_name", $SessionName,
        "--stop_existing_session",
        "--output_file", $OutputFile,
        "--qpc_time",
        "--v2_metrics",
        "--timed", "$TimedDurationSeconds",
        "--terminate_after_timed"
    ))

    if ($ProcessId -gt 0) {
        $pmArgs.Add("--process_id")
        $pmArgs.Add("$ProcessId")
    } elseif (-not [string]::IsNullOrWhiteSpace($ProcessName)) {
        $pmArgs.Add("--process_name")
        $pmArgs.Add("$ProcessName.exe")
    }

    return $pmArgs.ToArray()
}

function Invoke-SelfTest {
    Write-Host "Running measure-native-hud-presentmon SelfTest suite..." -ForegroundColor Cyan

    # 1. Test Duration Calculation
    $dur1 = Get-CaptureDurationSeconds -Runs 10 -Warmup 2 -IntervalMs 5000 -TransitionMs 250 -State "saved" -StartupLeadSeconds 2.0 -TailSeconds 3.0
    if ($dur1 -ne 66) {
        throw "SelfTest Failed: Expected duration 66, got $dur1"
    }

    $dur2 = Get-CaptureDurationSeconds -Runs 5 -Warmup 1 -IntervalMs 5000 -TransitionMs 6000 -State "queued-to-saved" -StartupLeadSeconds 1.5 -TailSeconds 4.0
    if ($dur2 -ne 42) {
        throw "SelfTest Failed: Expected duration 42, got $dur2"
    }

    # 2. Test Benchmark CLI Argument Construction (Default, Explicit Off, and Lifecycle Policy)
    $argsDefault = Build-BenchmarkArguments -MarkersPath "C:\test\markers.csv" -Runs 10 -Warmup 2 -IntervalMs 5000 -TransitionMs 250 -State "saved" -Mode "full" -Anchor "bottom-center" -CaptureExclusion "on" -LifecyclePolicy "attached-shown"
    $expectedDefault = @("--markers", "C:\test\markers.csv", "--runs", "10", "--warmup", "2", "--interval-ms", "5000", "--transition-ms", "250", "--state", "saved", "--mode", "full", "--anchor", "bottom-center", "--capture-exclusion", "on", "--lifecycle-policy", "attached-shown")
    if ($argsDefault.Length -ne $expectedDefault.Length) {
        throw "SelfTest Failed: Default argument count mismatch: expected $($expectedDefault.Length), got $($argsDefault.Length)"
    }
    for ($i = 0; $i -lt $expectedDefault.Length; $i++) {
        if ($argsDefault[$i] -ne $expectedDefault[$i]) {
            throw "SelfTest Failed: Argument mismatch at index $($i): expected '$($expectedDefault[$i])', got '$($argsDefault[$i])'"
        }
    }

    $argsExplicitDetached = Build-BenchmarkArguments -MarkersPath "C:\test\markers.csv" -Runs 5 -Warmup 1 -IntervalMs 5000 -TransitionMs 250 -State "no-hud" -Mode "compact" -Anchor "top-left" -CaptureExclusion "off" -LifecyclePolicy "detached-shown"
    $expectedDetached = @("--markers", "C:\test\markers.csv", "--runs", "5", "--warmup", "1", "--interval-ms", "5000", "--transition-ms", "250", "--state", "no-hud", "--mode", "compact", "--anchor", "top-left", "--capture-exclusion", "off", "--lifecycle-policy", "detached-shown")
    if ($argsExplicitDetached.Length -ne $expectedDetached.Length) {
        throw "SelfTest Failed: Explicit detached argument count mismatch: expected $($expectedDetached.Length), got $($argsExplicitDetached.Length)"
    }
    for ($i = 0; $i -lt $expectedDetached.Length; $i++) {
        if ($argsExplicitDetached[$i] -ne $expectedDetached[$i]) {
            throw "SelfTest Failed: Argument mismatch at index $($i): expected '$($expectedDetached[$i])', got '$($argsExplicitDetached[$i])'"
        }
    }

    $argsExplicitHidden = Build-BenchmarkArguments -MarkersPath "C:\test\markers.csv" -Runs 5 -Warmup 1 -IntervalMs 5000 -TransitionMs 250 -State "saved" -Mode "full" -Anchor "bottom-center" -CaptureExclusion "on" -LifecyclePolicy "attached-hidden"
    $expectedHidden = @("--markers", "C:\test\markers.csv", "--runs", "5", "--warmup", "1", "--interval-ms", "5000", "--transition-ms", "250", "--state", "saved", "--mode", "full", "--anchor", "bottom-center", "--capture-exclusion", "on", "--lifecycle-policy", "attached-hidden")
    if ($argsExplicitHidden.Length -ne $expectedHidden.Length) {
        throw "SelfTest Failed: Explicit hidden argument count mismatch: expected $($expectedHidden.Length), got $($argsExplicitHidden.Length)"
    }
    for ($i = 0; $i -lt $expectedHidden.Length; $i++) {
        if ($argsExplicitHidden[$i] -ne $expectedHidden[$i]) {
            throw "SelfTest Failed: Argument mismatch at index $($i): expected '$($expectedHidden[$i])', got '$($argsExplicitHidden[$i])'"
        }
    }

    # 3. Test PresentMon Argument Construction
    $pmArgsByName = Build-PresentMonArguments -SessionName "silk_pm_test_123" -OutputFile "C:\test\out.csv" -TimedDurationSeconds 64 -ProcessName "game" -ProcessId 0
    if (-not ($pmArgsByName -contains "--session_name") -or -not ($pmArgsByName -contains "silk_pm_test_123")) {
        throw "SelfTest Failed: Missing unique session_name in PresentMon arguments"
    }
    if (-not ($pmArgsByName -contains "--stop_existing_session")) {
        throw "SelfTest Failed: Missing --stop_existing_session in PresentMon arguments"
    }
    if (-not ($pmArgsByName -contains "--process_name") -or -not ($pmArgsByName -contains "game.exe")) {
        throw "SelfTest Failed: Missing --process_name game.exe in PresentMon arguments"
    }

    $pmArgsById = Build-PresentMonArguments -SessionName "silk_pm_test_123" -OutputFile "C:\test\out.csv" -TimedDurationSeconds 64 -ProcessName "" -ProcessId 12345
    if (-not ($pmArgsById -contains "--process_id") -or -not ($pmArgsById -contains "12345")) {
        throw "SelfTest Failed: Missing --process_id 12345 in PresentMon arguments"
    }

    Write-Host "SelfTest completed successfully. All assertions passed." -ForegroundColor Green
}

if ($SelfTest) {
    Invoke-SelfTest
    exit 0
}

# 1. Elevation Verification
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]$identity
$isAdmin = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (-not $isAdmin) {
    throw "Administrator elevation required. PresentMon requires an elevated shell to configure ETW sessions for real-time graphics tracing. Please re-run from an elevated PowerShell prompt."
}

# 2. Input Verification
if (-not (Test-Path -LiteralPath $PresentMonPath)) {
    throw "PresentMon executable not found at '$PresentMonPath'. Please provide the path to an official PresentMon v2 binary."
}

if ($BenchmarkPath -and -not (Test-Path -LiteralPath $BenchmarkPath)) {
    throw "Benchmark executable not found at '$BenchmarkPath'."
}

if (-not $AnalyzerPath) {
    $AnalyzerPath = Join-Path $PSScriptRoot "analyze-native-hud-presentmon.ps1"
}
if (-not (Test-Path -LiteralPath $AnalyzerPath)) {
    throw "Telemetry analyzer script not found at '$AnalyzerPath'."
}

# Verify target process is ALREADY running (BenchmarkPath is stimulus, not target launch)
$resolvedProcessName = $null
if ($PSCmdlet.ParameterSetName -eq "ById") {
    $proc = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if ($null -eq $proc) {
        throw "Target process with PID $ProcessId is not running. The target game process must already be running before starting measurement."
    }
    $resolvedProcessName = $proc.ProcessName
} else {
    $cleanProcName = $ProcessName -replace "\.exe$", ""
    $matching = Get-Process -Name $cleanProcName -ErrorAction SilentlyContinue
    if ($null -eq $matching) {
        throw "Target process '$ProcessName' is not running. The target game process must already be running before starting measurement."
    }
    $resolvedProcessName = $cleanProcName
}

# 3. Session Directory & Unique Session Setup
$sessionTimestamp = [DateTime]::UtcNow.ToString("yyyyMMdd_HHmmss")
$sessionGuid = [System.Guid]::NewGuid().ToString("N").Substring(0, 8)
$sessionDirName = "pm_hud_${sessionTimestamp}_${sessionGuid}"
$sessionDir = Join-Path $OutputDir $sessionDirName
New-Item -ItemType Directory -Path $sessionDir -Force | Out-Null

$sessionName = "silk_pm_${sessionTimestamp}_${sessionGuid}"
$presentMonCsvPath = Join-Path $sessionDir "presentmon_raw.csv"
$resolvedMarkerCsvPath = if ($MarkerCsvPath) { $MarkerCsvPath } else { Join-Path $sessionDir "markers.csv" }
$outputJsonPath = Join-Path $sessionDir "analysis_summary.json"
$outputMdPath = Join-Path $sessionDir "analysis_summary.md"

# 4. Duration Calculation
$timedCaptureSeconds = Get-CaptureDurationSeconds `
    -Runs $Runs `
    -Warmup $Warmup `
    -IntervalMs $IntervalMs `
    -TransitionMs $TransitionMs `
    -State $State `
    -StartupLeadSeconds $StartupLeadSeconds `
    -TailSeconds $TailSeconds

Write-Host "=== Native HUD PresentMon Measurement Orchestrator ===" -ForegroundColor Cyan
Write-Host "Session Name:      $sessionName"
Write-Host "Target Process:    $(if ($ProcessId) { "PID $ProcessId ($resolvedProcessName)" } else { $ProcessName })"
Write-Host "Stimulus State:    $State (mode: $Mode, anchor: $Anchor, capture-exclusion: $CaptureExclusion, lifecycle-policy: $LifecyclePolicy)"
Write-Host "Cycles:            $Runs runs + $Warmup warmup (interval: ${IntervalMs}ms)"
Write-Host "Capture Duration:  ${timedCaptureSeconds}s (Lead: ${StartupLeadSeconds}s, Tail: ${TailSeconds}s)"
Write-Host "Session Dir:       $sessionDir"

# 5. Launch PresentMon v2 with unique session
$pmArgs = Build-PresentMonArguments `
    -SessionName $sessionName `
    -OutputFile $presentMonCsvPath `
    -TimedDurationSeconds $timedCaptureSeconds `
    -ProcessName $(if ($PSCmdlet.ParameterSetName -eq "ByName") { $resolvedProcessName } else { "" }) `
    -ProcessId $(if ($PSCmdlet.ParameterSetName -eq "ById") { $ProcessId } else { 0 })

$pmStartInfo = [System.Diagnostics.ProcessStartInfo]::new()
$pmStartInfo.FileName = (Resolve-Path $PresentMonPath).Path
$pmStartInfo.Arguments = ($pmArgs | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '
$pmStartInfo.UseShellExecute = $false
$pmStartInfo.CreateNoWindow = $true

Write-Host "Starting PresentMon v2 session ($sessionName)..." -ForegroundColor Cyan
$pmProc = [System.Diagnostics.Process]::Start($pmStartInfo)
if ($null -eq $pmProc) {
    throw "Failed to start PresentMon process."
}

$benchmarkProc = $null

try {
    # Wait startup lead so PresentMon ETW session is initialized and pre-window baseline frames are recorded
    Write-Host "Waiting startup lead time (${StartupLeadSeconds}s)..."
    Start-Sleep -Milliseconds ([int]($StartupLeadSeconds * 1000))

    if ($BenchmarkPath) {
        Write-Host "Launching benchmark stimulus: $BenchmarkPath" -ForegroundColor Cyan
        $benchArgs = Build-BenchmarkArguments `
            -MarkersPath $resolvedMarkerCsvPath `
            -Runs $Runs `
            -Warmup $Warmup `
            -IntervalMs $IntervalMs `
            -TransitionMs $TransitionMs `
            -State $State `
            -Mode $Mode `
            -Anchor $Anchor `
            -CaptureExclusion $CaptureExclusion `
            -LifecyclePolicy $LifecyclePolicy `
            -AdditionalArgs $BenchmarkArgs

        $benchStartInfo = [System.Diagnostics.ProcessStartInfo]::new()
        $benchStartInfo.FileName = (Resolve-Path $BenchmarkPath).Path
        $benchStartInfo.Arguments = ($benchArgs | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '
        $benchStartInfo.UseShellExecute = $false

        $benchmarkProc = [System.Diagnostics.Process]::Start($benchStartInfo)
        if ($null -eq $benchmarkProc) {
            throw "Failed to start benchmark process."
        }

        $benchmarkProc.WaitForExit()
        if ($benchmarkProc.ExitCode -ne 0) {
            throw "Benchmark process exited with non-zero exit code: $($benchmarkProc.ExitCode)"
        }
        Write-Host "Benchmark stimulus completed successfully." -ForegroundColor Green
    } else {
        Write-Host "External stimulation mode: waiting for PresentMon capture timer (${timedCaptureSeconds}s)..."
    }

    # Wait for PresentMon capture to complete
    $timeoutMs = ($timedCaptureSeconds + 15) * 1000
    $exited = $pmProc.WaitForExit($timeoutMs)
    if (-not $exited) {
        Write-Warning "PresentMon did not exit within timeout ($timeoutMs ms); terminating process."
        $pmProc.Kill()
        throw "PresentMon timed capture exceeded timeout."
    }

    if ($pmProc.ExitCode -ne 0) {
        throw "PresentMon exited with error code: $($pmProc.ExitCode)"
    }
    Write-Host "PresentMon capture completed." -ForegroundColor Green
}
finally {
    # Cleanup only the unique PresentMon process and ETW session started by this invocation
    if ($pmProc -and -not $pmProc.HasExited) {
        try {
            $pmProc.Kill()
        } catch {}
    }
    if ($sessionName) {
        try {
            $cleanupInfo = [System.Diagnostics.ProcessStartInfo]::new()
            $cleanupInfo.FileName = (Resolve-Path $PresentMonPath).Path
            $cleanupInfo.Arguments = "--session_name $sessionName --terminate_existing_session --no_csv"
            $cleanupInfo.UseShellExecute = $false
            $cleanupInfo.CreateNoWindow = $true
            $cleanupProc = [System.Diagnostics.Process]::Start($cleanupInfo)
            if ($cleanupProc) {
                $cleanupProc.WaitForExit(5000)
            }
        } catch {}
    }
}

# 6. Verify Captured Files
if (-not (Test-Path -LiteralPath $presentMonCsvPath)) {
    throw "PresentMon output CSV was not created: $presentMonCsvPath"
}
if (-not (Test-Path -LiteralPath $resolvedMarkerCsvPath)) {
    throw "Marker CSV file not found: $resolvedMarkerCsvPath. Ensure the benchmark wrote marker timings."
}

# 7. Invoke Analyzer
Write-Host "Invoking Telemetry Analyzer..." -ForegroundColor Cyan
$analyzerParams = @{
    PresentMonCsv  = $presentMonCsvPath
    MarkerCsv      = $resolvedMarkerCsvPath
    OutputJson     = $outputJsonPath
    OutputMarkdown = $outputMdPath
}
if ($SwapChainAddress) {
    $analyzerParams["SwapChainAddress"] = $SwapChainAddress
}

$analysis = & $AnalyzerPath @analyzerParams

Write-Host "Session evidence written to: $sessionDir" -ForegroundColor Green
return $analysis
