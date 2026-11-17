[CmdletBinding()]
param([Parameter(Mandatory)][string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
$state = Get-Content -LiteralPath (Join-Path $OutputDirectory 'result.json') -Raw | ConvertFrom-Json
if ($state.status -in @('starting', 'running')) {
    $runner = Get-Process -Id $state.runnerPid -ErrorAction SilentlyContinue
    # PowerShell may deserialize ISO JSON strings as DateTime automatically.
    # Compare instants, not culture-dependent string conversions of that object.
    $sameRunner = $runner -and $runner.StartTime.ToUniversalTime().Ticks -eq ([DateTimeOffset]$state.runnerStartedUtc).UtcTicks
    if (-not $sameRunner) {
        # Never turn missing runner evidence into a pass or overwrite the record.
        $state.status = 'interrupted_unconfirmed'
    }
}
$state | ConvertTo-Json -Depth 5
