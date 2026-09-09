[CmdletBinding()]
param(
    [ValidateRange(1, 86400)]
    [int] $Seconds = 3600,
    [ValidateSet(30, 60)]
    [int] $Fps = 60
)

# Run the native WGC probe from an interactive Windows desktop. The probe
# reports frame counts, queue drops, resize events, and timestamp violations;
# GPU process counters must be recorded separately for the S2 one-hour gate.
$ErrorActionPreference = "Stop"

cargo run -p capture-windows --release --example capture_probe -- --seconds $Seconds --fps $Fps
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}
