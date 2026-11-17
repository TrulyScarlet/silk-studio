[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Executable,
    [string[]]$Arguments = @(),
    [Parameter(Mandatory)][string]$OutputDirectory,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 300
)

$ErrorActionPreference = 'Stop'
$directory = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $directory) { throw 'Use a new output directory for every run; existing evidence is never overwritten.' }
$null = New-Item -ItemType Directory -Path $directory
$statePath = Join-Path $directory 'result.json'
$state = [ordered]@{
    schemaVersion = 1
    status = 'starting'
    startedUtc = [DateTime]::UtcNow.ToString('o')
    finishedUtc = $null
    runnerPid = $PID
    runnerStartedUtc = (Get-Process -Id $PID).StartTime.ToUniversalTime().ToString('o')
    childPid = $null
    executable = $Executable
    arguments = $Arguments
    timeoutSeconds = $TimeoutSeconds
    exitCode = $null
    error = $null
}
function Save-State {
    $temporary = "$statePath.tmp"
    [System.IO.File]::WriteAllText($temporary, ($state | ConvertTo-Json -Depth 5))
    [System.IO.File]::Move($temporary, $statePath, $true)
}
Save-State
$process = [System.Diagnostics.Process]::new()
try {
    $process.StartInfo.FileName = $Executable
    $process.StartInfo.UseShellExecute = $false
    $process.StartInfo.RedirectStandardOutput = $true
    $process.StartInfo.RedirectStandardError = $true
    foreach ($argument in $Arguments) { $process.StartInfo.ArgumentList.Add($argument) }
    # Disable FileStream's managed write buffer: sparse fifteen-minute progress
    # lines must reach the file while the test runs, not only on runner exit.
    $stdout = [System.IO.FileStream]::new((Join-Path $directory 'stdout.log'), [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::ReadWrite, 1, [System.IO.FileOptions]::Asynchronous)
    $stderr = [System.IO.FileStream]::new((Join-Path $directory 'stderr.log'), [System.IO.FileMode]::CreateNew, [System.IO.FileAccess]::Write, [System.IO.FileShare]::ReadWrite, 1, [System.IO.FileOptions]::Asynchronous)
    try {
        if (-not $process.Start()) { throw 'Failed to start test process.' }
        $state.childPid = $process.Id
        $state.status = 'running'
        Save-State
        # Stream directly to files: no unbounded accumulation of test output.
        $outTask = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $errTask = $process.StandardError.BaseStream.CopyToAsync($stderr)
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            $state.status = 'timed_out'
            $process.Kill($true)
            $process.WaitForExit()
        } else {
            $state.status = if ($process.ExitCode -eq 0) { 'passed' } else { 'failed' }
        }
        $state.exitCode = $process.ExitCode
        if (-not [System.Threading.Tasks.Task]::WaitAll([System.Threading.Tasks.Task[]]@($outTask, $errTask), 5000)) {
            throw 'Output pipes did not close within five seconds; completion cannot be certified.'
        }
    } finally {
        $stdout.Dispose()
        $stderr.Dispose()
    }
} catch {
    $state.status = 'runner_error'
    $state.error = $_.Exception.Message
    if ($state.childPid -and -not $process.HasExited) {
        try { $process.Kill($true); $null = $process.WaitForExit(5000) } catch { $state.error += "; cleanup: $($_.Exception.Message)" }
    }
} finally {
    $state.finishedUtc = [DateTime]::UtcNow.ToString('o')
    Save-State
    $process.Dispose()
}
if ($state.status -ne 'passed') { throw "Tracked test ended with status $($state.status). Evidence: $statePath" }
