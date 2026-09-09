param(
    [Parameter(Mandatory = $true)]
    [string]$ClipPath,

    [string]$ExpectedVideoCodec = "h264",

    [string]$ExpectedVideoTag,

    [string]$FfmpegPath = "ffmpeg",

    [string]$FfprobePath = "ffprobe"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $ClipPath -PathType Leaf)) {
    throw "clip does not exist: $ClipPath"
}

$normalizedExpectedCodec = $ExpectedVideoCodec.Trim().ToLowerInvariant()
$canonicalCodecName = switch ($normalizedExpectedCodec) {
    { $_ -in @("h264", "avc", "avc1") } { "h264"; break }
    { $_ -in @("hevc", "h265", "hvc1") } { "hevc"; break }
    { $_ -in @("av1", "av01") } { "av1"; break }
    default { throw "unsupported expected video codec: $ExpectedVideoCodec" }
}

$expectedTag = if ([string]::IsNullOrWhiteSpace($ExpectedVideoTag)) {
    switch ($canonicalCodecName) {
        "h264" { "avc1" }
        "hevc" { "hvc1" }
        "av1" { "av01" }
    }
} else {
    $ExpectedVideoTag.Trim()
}

function Resolve-Tool {
    param([string]$Value)

    if (Test-Path -LiteralPath $Value -PathType Leaf) {
        return (Resolve-Path -LiteralPath $Value).Path
    }
    $command = Get-Command $Value -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        throw "media validation tool was not found: $Value"
    }
    return $command.Source
}

$ffprobe = Resolve-Tool $FfprobePath
$ffmpeg = Resolve-Tool $FfmpegPath

function Invoke-JsonTool {
    param(
        [string]$Executable,
        [string[]]$Arguments
    )

    $output = & $Executable @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Executable failed with exit code $LASTEXITCODE"
    }
    if ([string]::IsNullOrWhiteSpace(($output -join "`n"))) {
        throw "$Executable returned no JSON output"
    }
    return ($output -join "`n" | ConvertFrom-Json)
}

$probe = Invoke-JsonTool $ffprobe @(
    "-v", "error",
    "-show_streams",
    "-show_format",
    "-of", "json",
    $ClipPath
)

$streams = @($probe.streams)
$video = @($streams | Where-Object { $_.codec_type -eq "video" })
$audio = @($streams | Where-Object { $_.codec_type -eq "audio" })
if ($video.Count -eq 0) {
    throw "clip contains no video stream"
}
if ($audio.Count -eq 0) {
    throw "clip contains no audio stream"
}
$actualCodecName = $video[0].codec_name
if ($actualCodecName -ne $canonicalCodecName) {
    throw "unexpected video codec: $actualCodecName (expected: $canonicalCodecName)"
}
$actualTag = $video[0].codec_tag_string
if ($actualTag -ne $expectedTag) {
    throw "unexpected video codec tag: $actualTag (expected: $expectedTag)"
}
if ($video[0].width -le 0 -or $video[0].height -le 0) {
    throw "video stream has invalid dimensions"
}
foreach ($track in $audio) {
    if ($track.codec_name -notin @("aac", "mp4a")) {
        throw "unexpected audio codec: $($track.codec_name)"
    }
    if ($track.sample_rate -le 0 -or $track.channels -le 0) {
        throw "audio stream has invalid format metadata"
    }
}

$duration = [double]$probe.format.duration
if ($duration -le 0) {
    throw "container duration is not positive"
}

$packets = Invoke-JsonTool $ffprobe @(
    "-v", "error",
    "-show_packets",
    "-show_entries", "packet=stream_index,dts_time",
    "-of", "json",
    $ClipPath
)
$lastDts = @{}
foreach ($packet in @($packets.packets)) {
    if ([string]::IsNullOrWhiteSpace([string]$packet.dts_time)) {
        continue
    }
    $streamIndex = [int]$packet.stream_index
    $dts = [double]$packet.dts_time
    if ($lastDts.ContainsKey($streamIndex) -and $dts -lt $lastDts[$streamIndex]) {
        throw "decode timestamps are not monotonic on stream $streamIndex"
    }
    $lastDts[$streamIndex] = $dts
}

$decodeArguments = @("-v", "error", "-i", $ClipPath, "-f", "null", "-")
$null = & $ffmpeg @decodeArguments
if ($LASTEXITCODE -ne 0) {
    throw "ffmpeg full decode failed with exit code $LASTEXITCODE"
}

[pscustomobject]@{
    path = $ClipPath
    duration_seconds = $duration
    video_streams = $video.Count
    audio_streams = $audio.Count
    video_codec = $actualCodecName
    video_tag = $actualTag
    decoded = $true
} | ConvertTo-Json -Compress
