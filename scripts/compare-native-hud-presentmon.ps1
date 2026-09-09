<#
.SYNOPSIS
    Compares two native HUD PresentMon telemetry analysis JSON reports (e.g. baseline vs active HUD).

.DESCRIPTION
    Developer-only comparison utility for native HUD Phase 3/4 telemetry.
    Reads analyzer JSON results from baseline (no-hud, idle, or product disabled) and comparison
    (active HUD or product enabled) sessions, computes comparative differential metrics without
    pooling individual frame rows, and outputs structured Markdown and JSON reports.

.PARAMETER BaselineJson
    Path to baseline analyzer JSON output (e.g. no-hud or product disabled run).

.PARAMETER ComparisonJson
    Path to comparison analyzer JSON output (e.g. active HUD or product enabled run).

.PARAMETER OutputMarkdown
    Optional destination path to write comparison Markdown summary.

.PARAMETER OutputJson
    Optional destination path to write comparison JSON summary.

.PARAMETER SelfTest
    Runs built-in test suite with synthetic comparison JSON data and exits.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, ParameterSetName = "Compare")]
    [string]$BaselineJson,

    [Parameter(Mandatory = $true, ParameterSetName = "Compare")]
    [string]$ComparisonJson,

    [Parameter(ParameterSetName = "Compare")]
    [string]$OutputMarkdown,

    [Parameter(ParameterSetName = "Compare")]
    [string]$OutputJson,

    [Parameter(Mandatory = $true, ParameterSetName = "SelfTest")]
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"

function Invoke-TelemetryComparison {
    param(
        [string]$BaselinePath,
        [string]$ComparisonPath,
        [string]$OutputMarkdownPath,
        [string]$OutputJsonPath
    )

    if (-not (Test-Path -LiteralPath $BaselinePath)) {
        throw "Baseline JSON file not found: $BaselinePath"
    }
    if (-not (Test-Path -LiteralPath $ComparisonPath)) {
        throw "Comparison JSON file not found: $ComparisonPath"
    }

    $baseRaw = [System.IO.File]::ReadAllText($BaselinePath, [System.Text.Encoding]::UTF8)
    $compRaw = [System.IO.File]::ReadAllText($ComparisonPath, [System.Text.Encoding]::UTF8)

    $base = $baseRaw | ConvertFrom-Json
    $comp = $compRaw | ConvertFrom-Json

    if ($null -eq $base.HeadlineSummary -or $null -eq $comp.HeadlineSummary) {
        throw "Malformed analysis JSON: Missing HeadlineSummary in baseline or comparison input."
    }

    if ($null -eq $base.TelemetryGate -or $null -eq $comp.TelemetryGate) {
        throw "Malformed analysis JSON: Missing TelemetryGate in baseline or comparison input."
    }

    # Strict Render Metric Validation
    if ($base.RenderMetricChosen -ne $comp.RenderMetricChosen) {
        throw "Cannot compare sessions with differing render pacing metrics: baseline used '$($base.RenderMetricChosen)', comparison used '$($comp.RenderMetricChosen)'."
    }

    # Compatibility Diagnostics
    if ($base.MeasuredMarkersCount -ne $comp.MeasuredMarkersCount) {
        Write-Warning "Measured marker count mismatch: baseline has $($base.MeasuredMarkersCount) markers, comparison has $($comp.MeasuredMarkersCount) markers."
    }

    if ($base.QpcFrequencyHz -and $comp.QpcFrequencyHz -and $base.QpcFrequencyHz -ne $comp.QpcFrequencyHz) {
        Write-Warning "QPC timer frequency mismatch: baseline has $($base.QpcFrequencyHz) Hz, comparison has $($comp.QpcFrequencyHz) Hz."
    }

    $baseExclusion = if ($base.CaptureExclusion) { [string]$base.CaptureExclusion } else { "unspecified" }
    $compExclusion = if ($comp.CaptureExclusion) { [string]$comp.CaptureExclusion } else { "unspecified" }

    $basePolicy = if ($base.LifecyclePolicy) { [string]$base.LifecyclePolicy } else { "unspecified" }
    $compPolicy = if ($comp.LifecyclePolicy) { [string]$comp.LifecyclePolicy } else { "unspecified" }

    # Calculate comparative deltas (without pooling raw frames)
    $meanPairedRenderDeltaDiff = [Math]::Round(($comp.HeadlineSummary.MeanPairedRenderP99DeltaMs - $base.HeadlineSummary.MeanPairedRenderP99DeltaMs), 4)
    $maxPairedRenderDeltaDiff = [Math]::Round(($comp.HeadlineSummary.MaxPairedRenderP99DeltaMs - $base.HeadlineSummary.MaxPairedRenderP99DeltaMs), 4)

    $meanPairedDroppedDeltaDiff = if ($null -ne $comp.HeadlineSummary.MeanPairedDroppedDelta -and $null -ne $base.HeadlineSummary.MeanPairedDroppedDelta) {
        [Math]::Round(($comp.HeadlineSummary.MeanPairedDroppedDelta - $base.HeadlineSummary.MeanPairedDroppedDelta), 4)
    } else {
        $null
    }

    $maxPairedDroppedDeltaDiff = if ($null -ne $comp.HeadlineSummary.MaxPairedDroppedDelta -and $null -ne $base.HeadlineSummary.MaxPairedDroppedDelta) {
        ($comp.HeadlineSummary.MaxPairedDroppedDelta - $base.HeadlineSummary.MaxPairedDroppedDelta)
    } else {
        $null
    }

    $postHwRatioDiff = [Math]::Round(($comp.HeadlineSummary.MeanPostHardwareIndepRatio - $base.HeadlineSummary.MeanPostHardwareIndepRatio), 4)

    $disclaimer = "Comparison does not pool individual frame rows across runs. Telemetry comparison evaluates frame pacing deltas and presentation mode stability; it does not prove focus, click-through, visual fidelity, or capture exclusion."

    $comparisonResult = [PSCustomObject]@{
        Timestamp                   = [DateTime]::UtcNow.ToString("o")
        BaselinePath                = (Resolve-Path $BaselinePath).Path
        ComparisonPath              = (Resolve-Path $ComparisonPath).Path
        BaselineGateStatus          = $base.TelemetryGate.Status
        ComparisonGateStatus        = $comp.TelemetryGate.Status
        RenderMetric                = $comp.RenderMetricChosen
        BaselineCaptureExclusion    = $baseExclusion
        ComparisonCaptureExclusion  = $compExclusion
        BaselineLifecyclePolicy     = $basePolicy
        ComparisonLifecyclePolicy   = $compPolicy
        BaselineMarkersCount        = $base.MeasuredMarkersCount
        ComparisonMarkersCount      = $comp.MeasuredMarkersCount
        Summary                     = [PSCustomObject]@{
            BaselineMeanPairedRenderDeltaMs   = $base.HeadlineSummary.MeanPairedRenderP99DeltaMs
            ComparisonMeanPairedRenderDeltaMs = $comp.HeadlineSummary.MeanPairedRenderP99DeltaMs
            MeanPairedRenderDeltaDiffMs       = $meanPairedRenderDeltaDiff

            BaselineMaxPairedRenderDeltaMs   = $base.HeadlineSummary.MaxPairedRenderP99DeltaMs
            ComparisonMaxPairedRenderDeltaMs = $comp.HeadlineSummary.MaxPairedRenderP99DeltaMs
            MaxPairedRenderDeltaDiffMs       = $maxPairedRenderDeltaDiff

            BaselineMeanPairedDroppedDelta   = $base.HeadlineSummary.MeanPairedDroppedDelta
            ComparisonMeanPairedDroppedDelta = $comp.HeadlineSummary.MeanPairedDroppedDelta
            MeanPairedDroppedDeltaDiff       = $meanPairedDroppedDeltaDiff

            BaselineMaxPairedDroppedDelta   = $base.HeadlineSummary.MaxPairedDroppedDelta
            ComparisonMaxPairedDroppedDelta = $comp.HeadlineSummary.MaxPairedDroppedDelta
            MaxPairedDroppedDeltaDiff       = $maxPairedDroppedDeltaDiff

            BaselineMeanPostHardwareIndepRatio   = $base.HeadlineSummary.MeanPostHardwareIndepRatio
            ComparisonMeanPostHardwareIndepRatio = $comp.HeadlineSummary.MeanPostHardwareIndepRatio
            MeanPostHardwareIndepRatioDiff       = $postHwRatioDiff
        }
        Disclaimer                  = $disclaimer
    }

    if ($OutputJsonPath) {
        $jsonDir = Split-Path -Parent $OutputJsonPath
        if ($jsonDir -and -not (Test-Path -LiteralPath $jsonDir)) {
            New-Item -ItemType Directory -Path $jsonDir -Force | Out-Null
        }
        $jsonStr = $comparisonResult | ConvertTo-Json -Depth 6
        [System.IO.File]::WriteAllText($OutputJsonPath, $jsonStr, [System.Text.Encoding]::UTF8)
    }

    $baseMeanDropStr = if ($null -ne $base.HeadlineSummary.MeanPairedDroppedDelta) { "$($base.HeadlineSummary.MeanPairedDroppedDelta)" } else { "N/A" }
    $compMeanDropStr = if ($null -ne $comp.HeadlineSummary.MeanPairedDroppedDelta) { "$($comp.HeadlineSummary.MeanPairedDroppedDelta)" } else { "N/A" }
    $diffMeanDropStr = if ($null -ne $meanPairedDroppedDeltaDiff) { "$meanPairedDroppedDeltaDiff" } else { "N/A" }

    $baseMaxDropStr = if ($null -ne $base.HeadlineSummary.MaxPairedDroppedDelta) { "$($base.HeadlineSummary.MaxPairedDroppedDelta)" } else { "N/A" }
    $compMaxDropStr = if ($null -ne $comp.HeadlineSummary.MaxPairedDroppedDelta) { "$($comp.HeadlineSummary.MaxPairedDroppedDelta)" } else { "N/A" }
    $diffMaxDropStr = if ($null -ne $maxPairedDroppedDeltaDiff) { "$maxPairedDroppedDeltaDiff" } else { "N/A" }

    $sb = [System.Text.StringBuilder]::new()
    $null = $sb.AppendLine("# Native HUD Telemetry Comparison Report")
    $null = $sb.AppendLine()
    $null = $sb.AppendLine("**Generated:** $($comparisonResult.Timestamp)")
    $null = $sb.AppendLine("**Baseline:** $($comparisonResult.BaselinePath)")
    $null = $sb.AppendLine("**Comparison:** $($comparisonResult.ComparisonPath)")
    $null = $sb.AppendLine("**Render Metric:** $($comparisonResult.RenderMetric)")
    $null = $sb.AppendLine("**Capture Exclusion:** Baseline=$($comparisonResult.BaselineCaptureExclusion), Comparison=$($comparisonResult.ComparisonCaptureExclusion)")
    $null = $sb.AppendLine("**Lifecycle Policy:** Baseline=$($comparisonResult.BaselineLifecyclePolicy), Comparison=$($comparisonResult.ComparisonLifecyclePolicy)")
    $null = $sb.AppendLine("**Measured Markers:** Baseline=$($comparisonResult.BaselineMarkersCount), Comparison=$($comparisonResult.ComparisonMarkersCount)")
    $null = $sb.AppendLine()
    $null = $sb.AppendLine("## Gate Results")
    $null = $sb.AppendLine("- **Baseline Gate:** $($comparisonResult.BaselineGateStatus)")
    $null = $sb.AppendLine("- **Comparison Gate:** $($comparisonResult.ComparisonGateStatus)")
    $null = $sb.AppendLine()
    $null = $sb.AppendLine("## Comparative Metrics Summary")
    $null = $sb.AppendLine("| Metric | Baseline | Comparison | Difference (Comp - Base) |")
    $null = $sb.AppendLine("| :--- | :--- | :--- | :--- |")
    $null = $sb.AppendLine("| Mean Paired Render-p99 Delta | $($base.HeadlineSummary.MeanPairedRenderP99DeltaMs) ms | $($comp.HeadlineSummary.MeanPairedRenderP99DeltaMs) ms | $meanPairedRenderDeltaDiff ms |")
    $null = $sb.AppendLine("| Max Paired Render-p99 Delta | $($base.HeadlineSummary.MaxPairedRenderP99DeltaMs) ms | $($comp.HeadlineSummary.MaxPairedRenderP99DeltaMs) ms | $maxPairedRenderDeltaDiff ms |")
    $null = $sb.AppendLine("| Mean Paired Dropped Delta | $baseMeanDropStr | $compMeanDropStr | $diffMeanDropStr |")
    $null = $sb.AppendLine("| Max Paired Dropped Delta | $baseMaxDropStr | $compMaxDropStr | $diffMaxDropStr |")
    $null = $sb.AppendLine("| Post HW-Independent Flip Ratio | $([Math]::Round($base.HeadlineSummary.MeanPostHardwareIndepRatio * 100, 1))% | $([Math]::Round($comp.HeadlineSummary.MeanPostHardwareIndepRatio * 100, 1))% | $([Math]::Round($postHwRatioDiff * 100, 1))% |")
    $null = $sb.AppendLine()
    $null = $sb.AppendLine("> **Notice & Disclaimer:** $disclaimer")

    $mdOutput = $sb.ToString()

    if ($OutputMarkdownPath) {
        $mdDir = Split-Path -Parent $OutputMarkdownPath
        if ($mdDir -and -not (Test-Path -LiteralPath $mdDir)) {
            New-Item -ItemType Directory -Path $mdDir -Force | Out-Null
        }
        [System.IO.File]::WriteAllText($OutputMarkdownPath, $mdOutput, [System.Text.Encoding]::UTF8)
    }

    return $comparisonResult
}

function Invoke-SelfTest {
    Write-Host "Running compare-native-hud-presentmon SelfTest suite..." -ForegroundColor Cyan

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("compare_selftest_" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        $baseJsonPath = Join-Path $tempDir "base.json"
        $compJsonPath = Join-Path $tempDir "comp.json"
        $diffMdPath = Join-Path $tempDir "diff.md"
        $diffJsonPath = Join-Path $tempDir "diff.json"

        $baseData = [PSCustomObject]@{
            Timestamp            = [DateTime]::UtcNow.ToString("o")
            RenderMetricChosen   = "MsBetweenPresents"
            DisplayMetricChosen  = "MsBetweenDisplayChange"
            CaptureExclusion     = "on"
            LifecyclePolicy      = "attached-shown"
            QpcFrequencyHz       = 10000000
            MeasuredMarkersCount = 10
            TelemetryGate        = [PSCustomObject]@{ Status = "PASSED"; Passed = $true }
            HeadlineSummary      = [PSCustomObject]@{
                MeanPairedRenderP99DeltaMs = 0.05
                MaxPairedRenderP99DeltaMs  = 0.20
                MeanPairedDroppedDelta     = 0.0
                MaxPairedDroppedDelta      = 0
                MeanPreHardwareIndepRatio  = 1.0
                MeanPostHardwareIndepRatio = 1.0
            }
        }

        $compData = [PSCustomObject]@{
            Timestamp            = [DateTime]::UtcNow.ToString("o")
            RenderMetricChosen   = "MsBetweenPresents"
            DisplayMetricChosen  = "MsBetweenDisplayChange"
            CaptureExclusion     = "on"
            LifecyclePolicy      = "detached-shown"
            QpcFrequencyHz       = 10000000
            MeasuredMarkersCount = 10
            TelemetryGate        = [PSCustomObject]@{ Status = "PASSED"; Passed = $true }
            HeadlineSummary      = [PSCustomObject]@{
                MeanPairedRenderP99DeltaMs = 0.15
                MaxPairedRenderP99DeltaMs  = 0.45
                MeanPairedDroppedDelta     = 0.0
                MaxPairedDroppedDelta      = 0
                MeanPreHardwareIndepRatio  = 1.0
                MeanPostHardwareIndepRatio = 1.0
            }
        }

        [System.IO.File]::WriteAllText($baseJsonPath, ($baseData | ConvertTo-Json -Depth 6), [System.Text.Encoding]::UTF8)
        [System.IO.File]::WriteAllText($compJsonPath, ($compData | ConvertTo-Json -Depth 6), [System.Text.Encoding]::UTF8)

        # 1. Test normal comparison
        $res = Invoke-TelemetryComparison -BaselinePath $baseJsonPath -ComparisonPath $compJsonPath -OutputMarkdownPath $diffMdPath -OutputJsonPath $diffJsonPath

        if ([Math]::Abs($res.Summary.MeanPairedRenderDeltaDiffMs - 0.10) -gt 0.001) {
            throw "SelfTest Failed: Expected MeanPairedRenderDeltaDiffMs 0.10, got $($res.Summary.MeanPairedRenderDeltaDiffMs)"
        }
        if ([Math]::Abs($res.Summary.MaxPairedRenderDeltaDiffMs - 0.25) -gt 0.001) {
            throw "SelfTest Failed: Expected MaxPairedRenderDeltaDiffMs 0.25, got $($res.Summary.MaxPairedRenderDeltaDiffMs)"
        }
        if ($res.BaselineLifecyclePolicy -ne "attached-shown" -or $res.ComparisonLifecyclePolicy -ne "detached-shown") {
            throw "SelfTest Failed: Policy metadata not correctly preserved in comparison result."
        }
        if (-not (Test-Path -LiteralPath $diffMdPath) -or -not (Test-Path -LiteralPath $diffJsonPath)) {
            throw "SelfTest Failed: Expected output Markdown and JSON files to be created."
        }

        # 2. Test Mismatched Render Metric Rejection
        $incompatCompData = [PSCustomObject]@{
            Timestamp            = [DateTime]::UtcNow.ToString("o")
            RenderMetricChosen   = "FrameTime"
            DisplayMetricChosen  = "MsBetweenDisplayChange"
            QpcFrequencyHz       = 10000000
            MeasuredMarkersCount = 10
            TelemetryGate        = [PSCustomObject]@{ Status = "PASSED"; Passed = $true }
            HeadlineSummary      = [PSCustomObject]@{
                MeanPairedRenderP99DeltaMs = 0.15
                MaxPairedRenderP99DeltaMs  = 0.45
                MeanPairedDroppedDelta     = 0.0
                MaxPairedDroppedDelta      = 0
                MeanPreHardwareIndepRatio  = 1.0
                MeanPostHardwareIndepRatio = 1.0
            }
        }
        $incompatCompPath = Join-Path $tempDir "incompat_comp.json"
        [System.IO.File]::WriteAllText($incompatCompPath, ($incompatCompData | ConvertTo-Json -Depth 6), [System.Text.Encoding]::UTF8)

        $caughtIncompat = $false
        try {
            Invoke-TelemetryComparison -BaselinePath $baseJsonPath -ComparisonPath $incompatCompPath
        } catch {
            $caughtIncompat = $true
        }
        if (-not $caughtIncompat) {
            throw "SelfTest Failed: Expected mismatched render metric to throw error."
        }

        Write-Host "SelfTest completed successfully. All assertions passed." -ForegroundColor Green
    }
    finally {
        if (Test-Path -LiteralPath $tempDir) {
            Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

# Entrypoint logic
if ($SelfTest) {
    Invoke-SelfTest
    exit 0
} else {
    $comparisonResult = Invoke-TelemetryComparison `
        -BaselinePath $BaselineJson `
        -ComparisonPath $ComparisonJson `
        -OutputMarkdownPath $OutputMarkdown `
        -OutputJsonPath $OutputJson

    Write-Host "=== PresentMon Telemetry Comparison ===" -ForegroundColor Cyan
    Write-Host "Baseline Gate:   $($comparisonResult.BaselineGateStatus)"
    Write-Host "Comparison Gate: $($comparisonResult.ComparisonGateStatus)"
    Write-Host "Mean Paired Render p99 Diff: $($comparisonResult.Summary.MeanPairedRenderDeltaDiffMs) ms"
    Write-Host "Max Paired Render p99 Diff:  $($comparisonResult.Summary.MaxPairedRenderDeltaDiffMs) ms"
    Write-Host "Post HW-Indep Ratio Diff:    $([Math]::Round($comparisonResult.Summary.MeanPostHardwareIndepRatioDiff * 100, 1))%"
    Write-Host "Disclaimer: $($comparisonResult.Disclaimer)" -ForegroundColor Gray

    return $comparisonResult
}
