[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstallDir,
    # Offline archive injection is for packaging tests; the installer never supplies it.
    [string]$ArchivePath
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$expectedArchive = '2E8E28AF97C2AE338CCEF92E36DA9B2A4CD21D0CAD9DDE093545606CB07F5B00'
$expected = @{
    'ffmpeg.exe' = '57C56E369D5B4873B4D93FC1A1D833CB7CD8BC9325C14B05C34CE60B22842D8A'
    'ffprobe.exe' = 'AFE05347CAAABE479B3C4EAE71992B6EC1E11C57266A1D665DEB0F9FE9847208'
}
$root = [IO.Path]::GetFullPath($InstallDir)
$tools = Join-Path $root 'tools'
$destination = Join-Path $tools 'ffmpeg'
function Test-Pair([string]$folder) {
    foreach ($name in $expected.Keys) {
        $file = Join-Path $folder "bin\$name"
        if (-not (Test-Path -LiteralPath $file) -or (Get-FileHash -LiteralPath $file).Hash -ne $expected[$name]) { return $false }
    }
    return $true
}
if (Test-Pair $destination) { Write-Output 'Verified sharing tools are already installed.'; exit 0 }
[IO.Directory]::CreateDirectory($tools) | Out-Null
$stage = Join-Path $tools ('media-setup-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($stage) | Out-Null
$backup = Join-Path $stage 'previous'
try {
    if (-not $ArchivePath) {
        Write-Output 'Downloading sharing tools (about 240 MB)...'
        $ArchivePath = Join-Path $stage 'ffmpeg.zip'
        Add-Type -AssemblyName System.Net.Http
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $client = [Net.Http.HttpClient]::new()
        $client.Timeout = [TimeSpan]::FromMinutes(10)
        $cancel = [Threading.CancellationTokenSource]::new([TimeSpan]::FromMinutes(10))
        try {
            $response = $client.GetAsync('https://github.com/GyanD/codexffmpeg/releases/download/9.0.1/ffmpeg-9.0.1-full_build.zip', [Net.Http.HttpCompletionOption]::ResponseHeadersRead, $cancel.Token).GetAwaiter().GetResult()
            $response.EnsureSuccessStatusCode() | Out-Null
            if ($response.Content.Headers.ContentLength -gt 260000000) { throw 'Sharing tools download is too large.' }
            $inputStream = $response.Content.ReadAsStreamAsync().GetAwaiter().GetResult()
            $outputStream = [IO.File]::Create($ArchivePath)
            try {
                $buffer = New-Object byte[] 65536
                [long]$total = 0
                while (($count = $inputStream.ReadAsync($buffer, 0, $buffer.Length, $cancel.Token).GetAwaiter().GetResult()) -gt 0) {
                    $total += $count
                    if ($total -gt 260000000) { throw 'Sharing tools download exceeded its limit.' }
                    $outputStream.Write($buffer, 0, $count)
                }
            } finally { $outputStream.Dispose(); $inputStream.Dispose(); $response.Dispose() }
        } finally { $client.Dispose(); $cancel.Dispose() }
    }
    if ((Get-FileHash -LiteralPath $ArchivePath).Hash -ne $expectedArchive) { throw 'Sharing tools archive verification failed.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($ArchivePath)
    $prepared = Join-Path $stage 'prepared'
    [IO.Directory]::CreateDirectory((Join-Path $prepared 'bin')) | Out-Null
    try {
        # Extract only these fixed paths; never trust arbitrary archive entry paths.
        foreach ($relative in @('bin/ffmpeg.exe', 'bin/ffprobe.exe', 'LICENSE', 'README.txt')) {
            $entry = $zip.GetEntry("ffmpeg-9.0.1-full_build/$relative")
            if (-not $entry -or $entry.Length -gt 230000000) { throw "Invalid sharing tools archive entry: $relative" }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $prepared $relative))
        }
    } finally { $zip.Dispose() }
    if (-not (Test-Pair $prepared)) { throw 'Sharing executable verification failed.' }
    if (Test-Path -LiteralPath $destination) { Move-Item -LiteralPath $destination -Destination $backup }
    try { Move-Item -LiteralPath $prepared -Destination $destination }
    catch {
        if (Test-Path -LiteralPath $backup) { Move-Item -LiteralPath $backup -Destination $destination }
        throw
    }
    Write-Output 'Sharing tools installed and verified.'
} catch {
    Write-Error "Silk could not install its sharing tools. Check your internet connection and rerun setup. $($_.Exception.Message)" -ErrorAction Continue
    exit 1
} finally {
    # Only delete the uniquely created staging folder beneath this install's tools.
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if ($resolvedStage.StartsWith([IO.Path]::GetFullPath($tools) + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $resolvedStage -Recurse -Force -ErrorAction SilentlyContinue
    }
}
