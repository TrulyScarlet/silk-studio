$ErrorActionPreference = 'Stop'
$root = Join-Path ([System.IO.Path]::GetTempPath()) "silk-tracked-$([Guid]::NewGuid().ToString('N'))"
$pwsh = (Get-Process -Id $PID).Path
function Invoke-Case([string]$Name, [string]$Command, [string]$Expected, [int]$Timeout = 10) {
    $directory = Join-Path $root $Name
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Command))
    try {
        & "$PSScriptRoot/run-tracked-test.ps1" -Executable $pwsh -Arguments @('-NoProfile', '-EncodedCommand', $encoded) -OutputDirectory $directory -TimeoutSeconds $Timeout
    } catch {
        if ($Expected -eq 'passed') { throw }
    }
    $state = Get-Content -LiteralPath (Join-Path $directory 'result.json') -Raw | ConvertFrom-Json
    if ($state.status -ne $Expected) { throw "$Name expected $Expected, got $($state.status)" }
    if (-not $state.finishedUtc) { throw "$Name missing completion timestamp" }
    return $directory
}
$passed = Invoke-Case 'success' "[Console]::Out.WriteLine('output'); [Console]::Error.WriteLine('diagnostic'); exit 0" 'passed'
if ((Get-Content (Join-Path $passed 'stdout.log') -Raw).Trim() -ne 'output') { throw 'stdout missing' }
if ((Get-Content (Join-Path $passed 'stderr.log') -Raw).Trim() -ne 'diagnostic') { throw 'stderr missing' }
$failed = Invoke-Case 'failure' 'exit 7' 'failed'
$failedState = Get-Content (Join-Path $failed 'result.json') -Raw | ConvertFrom-Json
if ($failedState.exitCode -ne 7) { throw 'Actual child exit code was not preserved' }
$null = Invoke-Case 'timeout' 'Start-Sleep -Seconds 30' 'timed_out' 1
$missing = Join-Path $root 'missing'
try {
    & "$PSScriptRoot/run-tracked-test.ps1" -Executable (Join-Path $root 'missing.exe') -OutputDirectory $missing
} catch { }
$missingState = Get-Content (Join-Path $missing 'result.json') -Raw | ConvertFrom-Json
if ($missingState.status -ne 'runner_error' -or -not $missingState.error) { throw 'Launch error was not recorded' }
try {
    & "$PSScriptRoot/run-tracked-test.ps1" -Executable $pwsh -OutputDirectory $passed
    throw 'Existing evidence was accepted'
} catch { if ($_.Exception.Message -eq 'Existing evidence was accepted') { throw } }
$interrupted = Join-Path $root 'interrupted'
$null = New-Item -ItemType Directory -Path $interrupted
@{ status = 'running'; runnerPid = $PID; runnerStartedUtc = '2000-01-01T00:00:00Z' } | ConvertTo-Json | Set-Content (Join-Path $interrupted 'result.json')
$observed = & "$PSScriptRoot/get-tracked-test.ps1" -OutputDirectory $interrupted | ConvertFrom-Json
if ($observed.status -ne 'interrupted_unconfirmed') { throw 'Recycled PID not rejected' }
$unchanged = Get-Content (Join-Path $interrupted 'result.json') -Raw | ConvertFrom-Json
if ($unchanged.status -ne 'running') { throw 'Reader overwrote original evidence' }
@{ status = 'running'; runnerPid = $PID; runnerStartedUtc = (Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o') } | ConvertTo-Json | Set-Content (Join-Path $interrupted 'result.json')
$live = & "$PSScriptRoot/get-tracked-test.ps1" -OutputDirectory $interrupted | ConvertFrom-Json
if ($live.status -ne 'running') { throw 'Live runner incorrectly marked interrupted' }

# Observe tiny output while the child is still running; final-output-only tests
# would miss FileStream retaining an entire sparse endurance log in its buffer.
$liveDirectory = Join-Path $root 'live-output'
$childCode = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes("[Console]::Out.WriteLine('live-output'); [Console]::Error.WriteLine('live-error'); Start-Sleep -Seconds 30"))
$runnerPath = "$PSScriptRoot/run-tracked-test.ps1".Replace("'", "''")
$quotedPwsh = $pwsh.Replace("'", "''")
$quotedDirectory = $liveDirectory.Replace("'", "''")
$runnerCode = "try { & '$runnerPath' -Executable '$quotedPwsh' -Arguments @('-NoProfile','-EncodedCommand','$childCode') -OutputDirectory '$quotedDirectory' -TimeoutSeconds 5 } catch { exit 1 }"
$runner = [System.Diagnostics.Process]::new()
$runner.StartInfo.FileName = $pwsh
$runner.StartInfo.UseShellExecute = $false
$runner.StartInfo.ArgumentList.Add('-NoProfile')
$runner.StartInfo.ArgumentList.Add('-EncodedCommand')
$runner.StartInfo.ArgumentList.Add([Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($runnerCode)))
try {
    $null = $runner.Start()
    $deadline = [DateTime]::UtcNow.AddSeconds(4)
    $visible = $false
    while ([DateTime]::UtcNow -lt $deadline -and -not $runner.HasExited) {
        $out = Join-Path $liveDirectory 'stdout.log'
        $err = Join-Path $liveDirectory 'stderr.log'
        if ((Test-Path $out) -and (Test-Path $err)) {
            $visible = (Get-Content $out -Raw) -match 'live-output' -and (Get-Content $err -Raw) -match 'live-error'
            if ($visible) { break }
        }
        Start-Sleep -Milliseconds 10
    }
    if (-not $visible -or $runner.HasExited) { throw 'Sparse progress output was not visible before completion' }
    if (-not $runner.WaitForExit(10000)) { throw 'Live-output timeout case did not finish' }
    $result = Get-Content (Join-Path $liveDirectory 'result.json') -Raw | ConvertFrom-Json
    if ($result.status -ne 'timed_out') { throw 'Live-output child did not retain timeout result' }
} finally {
    if (-not $runner.HasExited) { $runner.Kill($true); $null = $runner.WaitForExit(5000) }
    $runner.Dispose()
}
Write-Output "PASS: success/failure/timeout, split output, immutable evidence and stale-run detection. Evidence: $root"
