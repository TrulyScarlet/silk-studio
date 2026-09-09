param(
    [Parameter(Mandatory = $true)]
    [string]$FfmpegPath,

    [Parameter(Mandatory = $true)]
    [string]$FfprobePath
)

$ErrorActionPreference = "Stop"

function Resolve-ExecutablePath {
    param([string]$Value)

    if (-not (Test-Path -LiteralPath $Value -PathType Leaf)) {
        throw "FFmpeg tool does not exist: $Value"
    }
    return (Resolve-Path -LiteralPath $Value).Path
}

$ffmpeg = Resolve-ExecutablePath $FfmpegPath
$ffprobe = Resolve-ExecutablePath $FfprobePath

Write-Output "ffmpeg_sha256=$((Get-FileHash -LiteralPath $ffmpeg -Algorithm SHA256).Hash)"
Write-Output "ffprobe_sha256=$((Get-FileHash -LiteralPath $ffprobe -Algorithm SHA256).Hash)"

& cargo run --quiet -p encoder-ffmpeg --bin verify-ffmpeg-toolchain -- --ffmpeg $ffmpeg --ffprobe $ffprobe
if ($LASTEXITCODE -ne 0) {
    throw "FFmpeg toolchain verification failed with exit code $LASTEXITCODE"
}
