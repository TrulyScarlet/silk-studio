<#
.SYNOPSIS
    Analyzes PresentMon v2 CSV capture and benchmark marker CSV data for native HUD telemetry gating.

.DESCRIPTION
    Developer-only telemetry analyzer for native HUD Phase 3/4 validation. Slices PresentMon v2
    frame timing data against timestamped HUD markers (pre, post, exit windows), evaluates paired
    render/display pacing deltas, dropped frames, hardware-independent flip mode ratios,
    capture exclusion, and lifecycle policy metadata against formal telemetry gate thresholds.

.PARAMETER PresentMonCsv
    Path to PresentMon v2 CSV capture file. Must contain CPUStartQPC column.

.PARAMETER MarkerCsv
    Path to benchmark marker CSV file. Schema: run_index,warmup,state,mode,anchor,[capture_exclusion,][lifecycle_policy,]trigger_qpc,qpc_frequency,submit_result

.PARAMETER OutputJson
    Optional destination path to write structured analysis JSON.

.PARAMETER OutputMarkdown
    Optional destination path to write human-readable Markdown summary.

.PARAMETER SwapChainAddress
    Optional explicit SwapChainAddress filter. If omitted, automatically selects the swapchain with the most frame rows.

.PARAMETER SelfTest
    Runs built-in test suite with synthetic CSV data and exits.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, ParameterSetName = "Analyze")]
    [string]$PresentMonCsv,

    [Parameter(Mandatory = $true, ParameterSetName = "Analyze")]
    [string]$MarkerCsv,

    [Parameter(ParameterSetName = "Analyze")]
    [string]$OutputJson,

    [Parameter(ParameterSetName = "Analyze")]
    [string]$OutputMarkdown,

    [Parameter(ParameterSetName = "Analyze")]
    [string]$SwapChainAddress,

    [Parameter(Mandatory = $true, ParameterSetName = "SelfTest")]
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"

function Get-Quantile {
    <#
    .SYNOPSIS
        Calculates quantile using Linear Interpolation (Type 7 / Excel PERCENTILE.INC).
    #>
    param(
        [double[]]$Values,
        [ValidateRange(0.0, 1.0)]
        [double]$Quantile
    )

    if ($null -eq $Values -or $Values.Count -eq 0) {
        return $null
    }
    if ($Values.Count -eq 1) {
        return [double]$Values[0]
    }

    $sorted = [double[]]($Values | Sort-Object)
    $n = $sorted.Count
    $index = ($n - 1) * $Quantile
    $lowIndex = [int][Math]::Floor($index)
    $fraction = $index - $lowIndex

    if ($lowIndex -ge ($n - 1)) {
        return [double]$sorted[$n - 1]
    }

    return [double]($sorted[$lowIndex] + ($fraction * ($sorted[$lowIndex + 1] - $sorted[$lowIndex])))
}

function Test-IsHardwareIndependentMode {
    param([string]$ModeName)
    if ([string]::IsNullOrWhiteSpace($ModeName)) {
        return $false
    }
    if ($ModeName -match "Independent\s+Flip") {
        return $true
    }
    return $false
}

function Get-WindowStats {
    param(
        [PSCustomObject[]]$Frames,
        [string]$RenderMetricName,
        [string]$DisplayMetricName,
        [bool]$HasDroppedColumn = $false,
        [bool]$HasDisplayedTimeColumn = $false
    )

    $dropAvailable = ($HasDroppedColumn -or $HasDisplayedTimeColumn)
    $count = if ($Frames) { $Frames.Count } else { 0 }
    if ($count -eq 0) {
        return [PSCustomObject]@{
            RowCount                 = 0
            RenderMetric             = $RenderMetricName
            RenderMin                = $null
            RenderMean               = $null
            RenderP95                = $null
            RenderP99                = $null
            RenderMax                = $null
            DisplayMetric            = $DisplayMetricName
            DisplayMin               = $null
            DisplayMean              = $null
            DisplayP95               = $null
            DisplayP99               = $null
            DisplayMax               = $null
            DropMetricAvailable      = $dropAvailable
            DroppedCount             = if ($dropAvailable) { 0 } else { $null }
            DroppedRatio             = if ($dropAvailable) { 0.0 } else { $null }
            HardwareIndependentCount = 0
            HardwareIndependentRatio = 0.0
            PresentModeDistribution  = @{}
        }
    }

    $renderValues = [System.Collections.Generic.List[double]]::new()
    $displayValues = [System.Collections.Generic.List[double]]::new()
    $droppedCount = 0
    $hwIndepCount = 0
    $modeDistribution = @{}

    foreach ($frame in $Frames) {
        # Render pacing metric
        if ($RenderMetricName -and $null -ne $frame.$RenderMetricName -and -not [double]::IsNaN($frame.$RenderMetricName)) {
            $renderValues.Add([double]$frame.$RenderMetricName)
        }

        # Display pacing metric
        if ($DisplayMetricName -and $null -ne $frame.$DisplayMetricName -and -not [double]::IsNaN($frame.$DisplayMetricName)) {
            $displayValues.Add([double]$frame.$DisplayMetricName)
        }

        # Dropped frame check
        if ($dropAvailable) {
            $isDropped = $false
            if ($HasDroppedColumn -and $frame.PSObject.Properties.Match("Dropped").Count -gt 0 -and $null -ne $frame.Dropped) {
                $dropVal = [string]$frame.Dropped
                if ($dropVal -eq "1" -or $dropVal -eq "true" -or $dropVal -eq "True") {
                    $isDropped = $true
                } elseif ($dropVal -eq "0" -or $dropVal -eq "false" -or $dropVal -eq "False") {
                    $isDropped = $false
                }
            }
            if (-not $isDropped -and $HasDisplayedTimeColumn) {
                if ($null -eq $frame.DisplayedTime -or [double]::IsNaN($frame.DisplayedTime)) {
                    $isDropped = $true
                }
            }
            if ($isDropped) {
                $droppedCount++
            }
        }

        # PresentMode classification
        $mode = if ($frame.PresentMode) { [string]$frame.PresentMode } else { "Unknown" }
        if (-not $modeDistribution.ContainsKey($mode)) {
            $modeDistribution[$mode] = 0
        }
        $modeDistribution[$mode]++

        if (Test-IsHardwareIndependentMode -ModeName $mode) {
            $hwIndepCount++
        }
    }

    $renderMean = if ($renderValues.Count -gt 0) { ($renderValues | Measure-Object -Average).Average } else { $null }
    $renderMin = if ($renderValues.Count -gt 0) { ($renderValues | Measure-Object -Minimum).Minimum } else { $null }
    $renderMax = if ($renderValues.Count -gt 0) { ($renderValues | Measure-Object -Maximum).Maximum } else { $null }
    $renderP95 = if ($renderValues.Count -gt 0) { Get-Quantile -Values $renderValues.ToArray() -Quantile 0.95 } else { $null }
    $renderP99 = if ($renderValues.Count -gt 0) { Get-Quantile -Values $renderValues.ToArray() -Quantile 0.99 } else { $null }

    $displayMean = if ($displayValues.Count -gt 0) { ($displayValues | Measure-Object -Average).Average } else { $null }
    $displayMin = if ($displayValues.Count -gt 0) { ($displayValues | Measure-Object -Minimum).Minimum } else { $null }
    $displayMax = if ($displayValues.Count -gt 0) { ($displayValues | Measure-Object -Maximum).Maximum } else { $null }
    $displayP95 = if ($displayValues.Count -gt 0) { Get-Quantile -Values $displayValues.ToArray() -Quantile 0.95 } else { $null }
    $displayP99 = if ($displayValues.Count -gt 0) { Get-Quantile -Values $displayValues.ToArray() -Quantile 0.99 } else { $null }

    $finalDroppedCount = if ($dropAvailable) { $droppedCount } else { $null }
    $finalDroppedRatio = if ($dropAvailable) { [double]($droppedCount / $count) } else { $null }

    return [PSCustomObject]@{
        RowCount                 = $count
        RenderMetric             = $RenderMetricName
        RenderMin                = $renderMin
        RenderMean               = $renderMean
        RenderP95                = $renderP95
        RenderP99                = $renderP99
        RenderMax                = $renderMax
        DisplayMetric            = $DisplayMetricName
        DisplayMin               = $displayMin
        DisplayMean              = $displayMean
        DisplayP95               = $displayP95
        DisplayP99               = $displayP99
        DisplayMax               = $displayMax
        DropMetricAvailable      = $dropAvailable
        DroppedCount             = $finalDroppedCount
        DroppedRatio             = $finalDroppedRatio
        HardwareIndependentCount = $hwIndepCount
        HardwareIndependentRatio = [double]($hwIndepCount / $count)
        PresentModeDistribution  = $modeDistribution
    }
}

function Parse-PresentMonCsvData {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "PresentMon CSV file not found: $Path"
    }

    $rawText = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
    if ([string]::IsNullOrWhiteSpace($rawText)) {
        throw "PresentMon CSV file is empty: $Path"
    }

    $lines = ($rawText -split "\r?\n") | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and -not ($_ -match "^\s*(//|#)") }
    if ($lines.Length -lt 2) {
        throw "PresentMon CSV file is empty or missing data: $Path"
    }

    $filteredCsv = $lines -join "`r`n"
    $rawRecords = ConvertFrom-Csv -InputObject $filteredCsv
    if ($null -eq $rawRecords -or $rawRecords.Count -eq 0) {
        throw "PresentMon CSV contained no data rows: $Path"
    }

    $firstRow = $rawRecords[0]
    $propNames = @($firstRow.PSObject.Properties | ForEach-Object { $_.Name })

    # Validate CPUStartQPC requirement
    if (-not ($propNames -contains "CPUStartQPC")) {
        throw "PresentMon CSV missing mandatory 'CPUStartQPC' column. Ensure PresentMon was executed with --qpc_time."
    }

    # Identify available pacing metric columns
    # Priority for Render: MsBetweenPresents > MsBetweenAppStart > FrameTime
    $renderMetric = $null
    if ($propNames -contains "MsBetweenPresents") {
        $renderMetric = "MsBetweenPresents"
    } elseif ($propNames -contains "MsBetweenAppStart") {
        $renderMetric = "MsBetweenAppStart"
    } elseif ($propNames -contains "FrameTime") {
        $renderMetric = "FrameTime"
    } else {
        throw "PresentMon CSV missing render pacing metric (MsBetweenPresents, MsBetweenAppStart, or FrameTime)."
    }

    # Priority for Display: MsBetweenDisplayChange > DisplayedTime
    $displayMetric = $null
    if ($propNames -contains "MsBetweenDisplayChange") {
        $displayMetric = "MsBetweenDisplayChange"
    } elseif ($propNames -contains "DisplayedTime") {
        $displayMetric = "DisplayedTime"
    }

    $hasSwapChain = $propNames -contains "SwapChainAddress"
    $hasDisplayedTime = $propNames -contains "DisplayedTime"
    $hasPresentMode = $propNames -contains "PresentMode"
    $hasDropped = $propNames -contains "Dropped"

    $culture = [System.Globalization.CultureInfo]::InvariantCulture
    $frames = [System.Collections.Generic.List[PSCustomObject]]::new()
    $swapChainCounts = @{}
    $lineNum = 1

    foreach ($row in $rawRecords) {
        $lineNum++

        # Parse CPUStartQPC
        $qpcStr = [string]$row.CPUStartQPC
        $qpcVal = 0L
        if (-not [long]::TryParse($qpcStr.Trim(), [System.Globalization.NumberStyles]::Integer, $culture, [ref]$qpcVal)) {
            throw "Malformed CPUStartQPC '$qpcStr' at row $lineNum."
        }

        # Parse Render metric
        $renderVal = [double]::NaN
        $renderStr = [string]$row.$renderMetric
        if (-not [string]::IsNullOrWhiteSpace($renderStr) -and $renderStr.Trim() -ne "NA" -and $renderStr.Trim() -ne "N/A") {
            $parsedR = 0.0
            if ([double]::TryParse($renderStr.Trim(), [System.Globalization.NumberStyles]::Float, $culture, [ref]$parsedR)) {
                $renderVal = $parsedR
            } else {
                throw "Malformed numeric value '$renderStr' for column $renderMetric at row $lineNum."
            }
        }

        # Parse Display metric
        $displayVal = [double]::NaN
        if ($displayMetric) {
            $displayStr = [string]$row.$displayMetric
            if (-not [string]::IsNullOrWhiteSpace($displayStr) -and $displayStr.Trim() -ne "NA" -and $displayStr.Trim() -ne "N/A") {
                $parsedD = 0.0
                if ([double]::TryParse($displayStr.Trim(), [System.Globalization.NumberStyles]::Float, $culture, [ref]$parsedD)) {
                    $displayVal = $parsedD
                }
            }
        }

        # Parse DisplayedTime
        $dispTimeVal = [double]::NaN
        if ($hasDisplayedTime) {
            $dispTimeStr = [string]$row.DisplayedTime
            if (-not [string]::IsNullOrWhiteSpace($dispTimeStr) -and $dispTimeStr.Trim() -ne "NA" -and $dispTimeStr.Trim() -ne "N/A") {
                $parsedDT = 0.0
                if ([double]::TryParse($dispTimeStr.Trim(), [System.Globalization.NumberStyles]::Float, $culture, [ref]$parsedDT)) {
                    $dispTimeVal = $parsedDT
                }
            }
        }

        # Parse SwapChainAddress
        $swapChain = "N/A"
        if ($hasSwapChain) {
            $scStr = [string]$row.SwapChainAddress
            if (-not [string]::IsNullOrWhiteSpace($scStr)) {
                $swapChain = $scStr.Trim()
            }
        }
        if (-not $swapChainCounts.ContainsKey($swapChain)) {
            $swapChainCounts[$swapChain] = 0
        }
        $swapChainCounts[$swapChain]++

        # PresentMode
        $presentMode = "Unknown"
        if ($hasPresentMode -and -not [string]::IsNullOrWhiteSpace($row.PresentMode)) {
            $presentMode = ([string]$row.PresentMode).Trim()
        }

        # Dropped
        $dropped = $null
        if ($hasDropped) {
            $dropped = ([string]$row.Dropped).Trim()
        }

        $frameObj = [PSCustomObject]@{
            LineNumber       = $lineNum
            CPUStartQPC      = $qpcVal
            SwapChainAddress = $swapChain
            PresentMode      = $presentMode
            DisplayedTime    = $dispTimeVal
            Dropped          = $dropped
        }

        if ($renderMetric -and -not $frameObj.PSObject.Properties.Match($renderMetric).Count) {
            $frameObj | Add-Member -NotePropertyName $renderMetric -NotePropertyValue $renderVal
        }
        if ($displayMetric -and -not $frameObj.PSObject.Properties.Match($displayMetric).Count) {
            $frameObj | Add-Member -NotePropertyName $displayMetric -NotePropertyValue $displayVal
        }

        $frames.Add($frameObj)
    }

    return [PSCustomObject]@{
        Frames                 = $frames
        RenderMetric           = $renderMetric
        DisplayMetric          = $displayMetric
        HasDroppedColumn       = $hasDropped
        HasDisplayedTimeColumn = $hasDisplayedTime
        DropMetricAvailable    = ($hasDropped -or $hasDisplayedTime)
        SwapChainCounts        = $swapChainCounts
        TotalFrames            = $frames.Count
    }
}

function Parse-MarkerCsvData {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        throw "Marker CSV file not found: $Path"
    }

    $rawText = [System.IO.File]::ReadAllText($Path, [System.Text.Encoding]::UTF8)
    if ([string]::IsNullOrWhiteSpace($rawText)) {
        throw "Marker CSV file is empty: $Path"
    }

    $lines = ($rawText -split "\r?\n") | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and -not ($_ -match "^\s*#") }
    if ($lines.Length -lt 2) {
        throw "Marker CSV file is empty or missing data: $Path"
    }

    $filteredCsv = $lines -join "`r`n"
    $rawRecords = ConvertFrom-Csv -InputObject $filteredCsv
    if ($null -eq $rawRecords -or $rawRecords.Count -eq 0) {
        throw "Marker CSV contained no parseable rows: $Path"
    }

    $firstRow = $rawRecords[0]
    $propNames = @($firstRow.PSObject.Properties | ForEach-Object { $_.Name.Trim().ToLowerInvariant() })

    # Required core schema columns (backward compatible: capture_exclusion and lifecycle_policy are optional)
    $requiredCols = @("run_index", "warmup", "state", "mode", "anchor", "trigger_qpc", "qpc_frequency", "submit_result")
    foreach ($req in $requiredCols) {
        if (-not ($propNames -contains $req)) {
            throw "Marker CSV missing required column '$req'. Expected schema: run_index,warmup,state,mode,anchor,[capture_exclusion,][lifecycle_policy,]trigger_qpc,qpc_frequency,submit_result"
        }
    }

    $hasCaptureExclusion = $propNames -contains "capture_exclusion"
    $hasLifecyclePolicy = $propNames -contains "lifecycle_policy"
    $hasNominalDuration = ($propNames -contains "nominal_duration_ms") -or ($propNames -contains "cue_duration_ms") -or ($propNames -contains "duration_ms")
    $hasIntervalMs = $propNames -contains "interval_ms"

    $culture = [System.Globalization.CultureInfo]::InvariantCulture
    $markers = [System.Collections.Generic.List[PSCustomObject]]::new()
    $frequencies = [System.Collections.Generic.HashSet[long]]::new()
    $lineIndex = 1

    foreach ($row in $rawRecords) {
        $lineIndex++

        # run_index
        $runIndexStr = [string]$row.run_index
        $runIndex = 0
        if (-not [int]::TryParse($runIndexStr.Trim(), [System.Globalization.NumberStyles]::Integer, $culture, [ref]$runIndex)) {
            throw "Malformed run_index '$runIndexStr' at marker row $lineIndex."
        }

        # warmup boolean
        $warmupStr = ([string]$row.warmup).Trim().ToLowerInvariant()
        $isWarmup = ($warmupStr -eq "1" -or $warmupStr -eq "true")

        # state, mode, anchor
        $state = ([string]$row.state).Trim()
        $mode = ([string]$row.mode).Trim()
        $anchor = ([string]$row.anchor).Trim()

        # capture_exclusion (optional, backward compatible, default 'unspecified')
        $captureExclusion = "unspecified"
        if ($hasCaptureExclusion -and $null -ne $row.capture_exclusion) {
            $ceRaw = ([string]$row.capture_exclusion).Trim().ToLowerInvariant()
            if ($ceRaw -eq "on" -or $ceRaw -eq "true" -or $ceRaw -eq "1") {
                $captureExclusion = "on"
            } elseif ($ceRaw -eq "off" -or $ceRaw -eq "false" -or $ceRaw -eq "0") {
                $captureExclusion = "off"
            } elseif (-not [string]::IsNullOrWhiteSpace($ceRaw)) {
                $captureExclusion = $ceRaw
            }
        }

        # lifecycle_policy (optional, backward compatible, default 'attached-shown')
        $lifecyclePolicy = "attached-shown"
        if ($hasLifecyclePolicy -and $null -ne $row.lifecycle_policy) {
            $lpRaw = ([string]$row.lifecycle_policy).Trim().ToLowerInvariant()
            if (-not [string]::IsNullOrWhiteSpace($lpRaw)) {
                $lifecyclePolicy = $lpRaw
            }
        }

        # trigger_qpc
        $triggerQpcStr = [string]$row.trigger_qpc
        $triggerQpc = 0L
        if (-not [long]::TryParse($triggerQpcStr.Trim(), [System.Globalization.NumberStyles]::Integer, $culture, [ref]$triggerQpc)) {
            throw "Malformed trigger_qpc '$triggerQpcStr' at marker row $lineIndex."
        }

        # qpc_frequency
        $qpcFreqStr = [string]$row.qpc_frequency
        $qpcFreq = 0L
        if (-not [long]::TryParse($qpcFreqStr.Trim(), [System.Globalization.NumberStyles]::Integer, $culture, [ref]$qpcFreq) -or $qpcFreq -le 0) {
            throw "Malformed or non-positive qpc_frequency '$qpcFreqStr' at marker row $lineIndex."
        }
        $null = $frequencies.Add($qpcFreq)

        # submit_result
        $submitResult = ([string]$row.submit_result).Trim().ToLowerInvariant()

        # nominal_duration_ms & interval_ms metadata parsing with legacy fallback
        $timingSource = "legacy_derived"
        $nominalDuration = 0L
        $intervalMs = 5000L # conservative infer/use 5000ms

        $matchedDurationProp = $null
        foreach ($p in @("nominal_duration_ms", "cue_duration_ms", "duration_ms")) {
            if ($propNames -contains $p) {
                $matchedDurationProp = $p
                break
            }
        }

        if ($matchedDurationProp) {
            $durStr = [string]$row.$matchedDurationProp
            $parsedDur = 0L
            if ([string]::IsNullOrWhiteSpace($durStr) -or -not [long]::TryParse($durStr.Trim(), [System.Globalization.NumberStyles]::Integer, $culture, [ref]$parsedDur) -or $parsedDur -lt 0) {
                throw "Malformed or negative $matchedDurationProp '$durStr' at marker row $lineIndex. Duration must be an integer >= 0 ms."
            }
            $nominalDuration = $parsedDur
            $timingSource = "marker_metadata"
        }

        if ($hasIntervalMs) {
            $intStr = [string]$row.interval_ms
            $parsedInt = 0L
            if ([string]::IsNullOrWhiteSpace($intStr) -or -not [long]::TryParse($intStr.Trim(), [System.Globalization.NumberStyles]::Integer, $culture, [ref]$parsedInt) -or $parsedInt -lt 5000) {
                throw "Malformed or invalid interval_ms '$intStr' at marker row $lineIndex. Interval must be an integer >= 5000 ms to match benchmark floor."
            }
            $intervalMs = $parsedInt
        }

        if ($timingSource -eq "legacy_derived") {
            # Derive standard normal-motion durations from state
            $nominalDuration = switch -Regex ($state.ToLowerInvariant()) {
                "^(no-hud|no_hud|idle)$" { 0L }
                "^saved$" { 180L + 2200L + 200L } # 2580 ms
                "^(queued-to-saved|queued_to_saved)$" { 250L + 2200L + 200L } # 2650 ms
                "^(failed|rejected)$" { 180L + 3200L + 200L } # 3580 ms
                default { 0L }
            }
        }

        $markers.Add([PSCustomObject]@{
            LineNumber        = $lineIndex
            RunIndex          = $runIndex
            Warmup            = $isWarmup
            State             = $state
            Mode              = $mode
            Anchor            = $anchor
            CaptureExclusion  = $captureExclusion
            LifecyclePolicy   = $lifecyclePolicy
            TriggerQpc        = $triggerQpc
            QpcFrequency      = $qpcFreq
            SubmitResult      = $submitResult
            TimingSource      = $timingSource
            NominalDurationMs = $nominalDuration
            IntervalMs        = $intervalMs
        })
    }

    if ($frequencies.Count -gt 1) {
        $freqList = [string]::Join(", ", $frequencies)
        throw "Mixed QPC frequencies detected across markers ($freqList). Capture timestamps must use a uniform timer frequency."
    }

    return [PSCustomObject]@{
        Markers      = $markers
        QpcFrequency = ($markers[0].QpcFrequency)
        TotalMarkers = $markers.Count
    }
}

function Invoke-PresentMonAnalysis {
    param(
        [string]$PresentMonCsvPath,
        [string]$MarkerCsvPath,
        [string]$SelectedSwapChain,
        [string]$OutputJsonPath,
        [string]$OutputMarkdownPath
    )

    $pmData = Parse-PresentMonCsvData -Path $PresentMonCsvPath
    $markerData = Parse-MarkerCsvData -Path $MarkerCsvPath

    # Swapchain selection
    $activeSwapChain = $SelectedSwapChain
    if ([string]::IsNullOrWhiteSpace($activeSwapChain)) {
        $maxCount = -1
        foreach ($sc in $pmData.SwapChainCounts.Keys) {
            if ($pmData.SwapChainCounts[$sc] -gt $maxCount) {
                $maxCount = $pmData.SwapChainCounts[$sc]
                $activeSwapChain = $sc
            }
        }
    } else {
        if (-not $pmData.SwapChainCounts.ContainsKey($activeSwapChain)) {
            $available = [string]::Join(", ", $pmData.SwapChainCounts.Keys)
            throw "Requested SwapChainAddress '$activeSwapChain' not found in PresentMon capture. Available swapchains: $available"
        }
    }

    # Filter PresentMon frames to selected SwapChain
    $targetFrames = $pmData.Frames
    if ($activeSwapChain -ne "N/A" -and $pmData.SwapChainCounts.Count -gt 1) {
        $targetFrames = [System.Collections.Generic.List[PSCustomObject]]::new(
            [PSCustomObject[]]($pmData.Frames | Where-Object { $_.SwapChainAddress -eq $activeSwapChain })
        )
    }

    # Filter markers: exclude warmup and rejected/error submissions
    # Valid submission results: 'accepted', 'not_submitted', with legacy aliases 'success', 'ok', 'submitted'
    $validSubmitResults = @("accepted", "not_submitted", "success", "ok", "submitted")
    $measuredMarkers = [System.Collections.Generic.List[PSCustomObject]]::new()
    $excludedWarmups = 0
    $excludedErrors = 0

    foreach ($m in $markerData.Markers) {
        if ($m.Warmup) {
            $excludedWarmups++
            continue
        }
        if (-not ($validSubmitResults -contains $m.SubmitResult)) {
            $excludedErrors++
            continue
        }
        $measuredMarkers.Add($m)
    }

    if ($measuredMarkers.Count -eq 0) {
        throw "No valid measured markers remain after filtering (total markers: $($markerData.TotalMarkers), warmup excluded: $excludedWarmups, errors excluded: $excludedErrors)."
    }

    $qpcFreq = $markerData.QpcFrequency
    $oneSecondTicks = [long]$qpcFreq

    $markerDetails = [System.Collections.Generic.List[PSCustomObject]]::new()
    $pairedRenderP99Deltas = [System.Collections.Generic.List[double]]::new()
    $pairedDroppedDeltas = [System.Collections.Generic.List[int]]::new()
    $postHwRatios = [System.Collections.Generic.List[double]]::new()
    $preHwRatios = [System.Collections.Generic.List[double]]::new()
    $activeRecoveryLatencies = [System.Collections.Generic.List[double]]::new()
    $activeRecoveryLowerBounds = [System.Collections.Generic.List[double]]::new()
    $unrecoveredMarkerCount = 0

    for ($idx = 0; $idx -lt $measuredMarkers.Count; $idx++) {
        $marker = $measuredMarkers[$idx]
        $t0 = $marker.TriggerQpc
        $isControlState = ($marker.State -in @("no-hud", "no_hud", "idle"))

        # Window Slicing:
        # Pre: [T0 - 1.0s, T0)
        # Post: [T0, T0 + 1.0s]
        $preFrames = [PSCustomObject[]]($targetFrames | Where-Object { $_.CPUStartQPC -ge ($t0 - $oneSecondTicks) -and $_.CPUStartQPC -lt $t0 })
        $postFrames = [PSCustomObject[]]($targetFrames | Where-Object { $_.CPUStartQPC -ge $t0 -and $_.CPUStartQPC -le ($t0 + $oneSecondTicks) })

        if ($preFrames.Length -eq 0) {
            throw "Marker #$($marker.RunIndex) (T0=$t0) pre-window [T0-1s, T0) contains 0 frames. Insufficient capture lead time."
        }
        if ($postFrames.Length -eq 0) {
            throw "Marker #$($marker.RunIndex) (T0=$t0) post-window [T0, T0+1s] contains 0 frames. Insufficient capture duration or frozen presentation."
        }

        $preStats = Get-WindowStats -Frames $preFrames -RenderMetricName $pmData.RenderMetric -DisplayMetricName $pmData.DisplayMetric -HasDroppedColumn $pmData.HasDroppedColumn -HasDisplayedTimeColumn $pmData.HasDisplayedTimeColumn
        $postStats = Get-WindowStats -Frames $postFrames -RenderMetricName $pmData.RenderMetric -DisplayMetricName $pmData.DisplayMetric -HasDroppedColumn $pmData.HasDroppedColumn -HasDisplayedTimeColumn $pmData.HasDisplayedTimeColumn

        $pairedRenderDelta = [double]($postStats.RenderP99 - $preStats.RenderP99)
        $pairedDroppedDelta = if ($pmData.DropMetricAvailable -and $null -ne $preStats.DroppedCount -and $null -ne $postStats.DroppedCount) {
            [int]($postStats.DroppedCount - $preStats.DroppedCount)
        } else {
            $null
        }

        $pairedRenderP99Deltas.Add($pairedRenderDelta)
        if ($null -ne $pairedDroppedDelta) {
            $pairedDroppedDeltas.Add($pairedDroppedDelta)
        }
        $preHwRatios.Add($preStats.HardwareIndependentRatio)
        $postHwRatios.Add($postStats.HardwareIndependentRatio)

        $expectedCompletionQpc = $null
        $probeStats = $null
        $recoveryObserved = $null
        $recoveryLatencyMs = $null
        $recoveryLowerBoundMs = $null
        $postRecStats = $null
        $steadyStats = $null

        if (-not $isControlState) {
            $durationTicks = [long]($qpcFreq * ($marker.NominalDurationMs / 1000.0))
            $expectedCompletionQpc = $t0 + $durationTicks
            $probeEndTicks = $expectedCompletionQpc + [long]($qpcFreq * 0.200) # completion + 200ms
            $postRecEndTicks = $expectedCompletionQpc + [long]($qpcFreq * 0.700) # completion + 700ms
            $intervalTicks = [long]($qpcFreq * ($marker.IntervalMs / 1000.0))
            $steadyEndTicks = $t0 + $intervalTicks # trigger + interval

            # Slicing:
            # RecoveryProbe: [completion, completion + 200ms)
            # PostRecovery500: [completion + 200ms, completion + 700ms)
            # SteadyState: [completion + 700ms, trigger + interval)
            $probeFrames = [PSCustomObject[]]($targetFrames | Where-Object { $_.CPUStartQPC -ge $expectedCompletionQpc -and $_.CPUStartQPC -lt $probeEndTicks })
            $postRecFrames = [PSCustomObject[]]($targetFrames | Where-Object { $_.CPUStartQPC -ge $probeEndTicks -and $_.CPUStartQPC -lt $postRecEndTicks })
            $steadyFrames = [PSCustomObject[]]($targetFrames | Where-Object { $_.CPUStartQPC -ge $postRecEndTicks -and $_.CPUStartQPC -lt $steadyEndTicks })

            if ($probeFrames.Length -eq 0) {
                throw "Marker #$($marker.RunIndex) (T0=$t0) RecoveryProbe window [completion, completion+200ms) contains 0 frames. Insufficient capture duration or presentation ended prematurely."
            }
            if ($postRecFrames.Length -eq 0) {
                throw "Marker #$($marker.RunIndex) (T0=$t0) PostRecovery500 window [completion+200ms, completion+700ms) contains 0 frames. Insufficient capture duration or presentation ended prematurely."
            }
            if ($steadyFrames.Length -eq 0) {
                throw "Marker #$($marker.RunIndex) (T0=$t0) SteadyState window [completion+700ms, trigger+interval) contains 0 frames. Insufficient capture duration or presentation ended prematurely."
            }

            $probeStats = Get-WindowStats -Frames $probeFrames -RenderMetricName $pmData.RenderMetric -DisplayMetricName $pmData.DisplayMetric -HasDroppedColumn $pmData.HasDroppedColumn -HasDisplayedTimeColumn $pmData.HasDisplayedTimeColumn
            $postRecStats = Get-WindowStats -Frames $postRecFrames -RenderMetricName $pmData.RenderMetric -DisplayMetricName $pmData.DisplayMetric -HasDroppedColumn $pmData.HasDroppedColumn -HasDisplayedTimeColumn $pmData.HasDisplayedTimeColumn
            $steadyStats = Get-WindowStats -Frames $steadyFrames -RenderMetricName $pmData.RenderMetric -DisplayMetricName $pmData.DisplayMetric -HasDroppedColumn $pmData.HasDroppedColumn -HasDisplayedTimeColumn $pmData.HasDisplayedTimeColumn

            # Calculate conservative recovery latency or censored lower bound
            # Look at all frames at/after expected completion up to steadyEndTicks
            $evalFrames = [PSCustomObject[]]($targetFrames | Where-Object { $_.CPUStartQPC -ge $expectedCompletionQpc -and $_.CPUStartQPC -lt $steadyEndTicks })
            $lastDirtyIndex = -1

            for ($fIdx = 0; $fIdx -lt $evalFrames.Length; $fIdx++) {
                $f = $evalFrames[$fIdx]
                $isDirty = $false
                if (-not (Test-IsHardwareIndependentMode -ModeName $f.PresentMode)) {
                    $isDirty = $true
                } elseif ($pmData.DropMetricAvailable) {
                    if ($pmData.HasDroppedColumn -and $null -ne $f.Dropped) {
                        $dropVal = [string]$f.Dropped
                        if ($dropVal -eq "1" -or $dropVal -eq "true" -or $dropVal -eq "True") {
                            $isDirty = $true
                        }
                    }
                    if (-not $isDirty -and $pmData.HasDisplayedTimeColumn) {
                        if ($null -eq $f.DisplayedTime -or [double]::IsNaN($f.DisplayedTime)) {
                            $isDirty = $true
                        }
                    }
                }
                if ($isDirty) {
                    $lastDirtyIndex = $fIdx
                }
            }

            $obsDurationMs = [Math]::Max(0.0, [double](($steadyEndTicks - $expectedCompletionQpc) / $qpcFreq * 1000.0))

            if ($lastDirtyIndex -eq -1) {
                $recoveryObserved = $true
                $recoveryLatencyMs = 0.0
                $recoveryLowerBoundMs = $null
                $activeRecoveryLatencies.Add($recoveryLatencyMs)
            } elseif ($lastDirtyIndex + 1 -lt $evalFrames.Length) {
                $cleanFrame = $evalFrames[$lastDirtyIndex + 1]
                $recoveryObserved = $true
                $recoveryLatencyMs = [Math]::Max(0.0, [double](($cleanFrame.CPUStartQPC - $expectedCompletionQpc) / $qpcFreq * 1000.0))
                $recoveryLowerBoundMs = $null
                $activeRecoveryLatencies.Add($recoveryLatencyMs)
            } else {
                $recoveryObserved = $false
                $recoveryLatencyMs = $null
                $recoveryLowerBoundMs = $obsDurationMs
                $unrecoveredMarkerCount++
                $activeRecoveryLowerBounds.Add($recoveryLowerBoundMs)
            }
        }

        $markerDetails.Add([PSCustomObject]@{
            RunIndex               = $marker.RunIndex
            State                  = $marker.State
            Mode                   = $marker.Mode
            Anchor                 = $marker.Anchor
            CaptureExclusion       = $marker.CaptureExclusion
            LifecyclePolicy        = $marker.LifecyclePolicy
            TimingSource           = $marker.TimingSource
            NominalDurationMs      = $marker.NominalDurationMs
            IntervalMs             = $marker.IntervalMs
            TriggerQpc             = $marker.TriggerQpc
            ExpectedCompletionQpc  = $expectedCompletionQpc
            SubmitResult           = $marker.SubmitResult
            Pre                    = $preStats
            Post                   = $postStats
            RecoveryProbe          = $probeStats
            RecoveryObserved       = $recoveryObserved
            RecoveryLatencyMs      = if ($null -ne $recoveryLatencyMs) { [Math]::Round($recoveryLatencyMs, 3) } else { $null }
            RecoveryLowerBoundMs   = if ($null -ne $recoveryLowerBoundMs) { [Math]::Round($recoveryLowerBoundMs, 3) } else { $null }
            PostRecovery500        = $postRecStats
            SteadyState            = $steadyStats
            PairedRenderP99DeltaMs = $pairedRenderDelta
            PairedDroppedDelta     = $pairedDroppedDelta
        })
    }

    # Aggregate headline metrics
    $meanPairedRenderDelta = ($pairedRenderP99Deltas | Measure-Object -Average).Average
    $maxPairedRenderDelta = ($pairedRenderP99Deltas | Measure-Object -Maximum).Maximum
    $meanPairedDroppedDelta = if ($pairedDroppedDeltas.Count -gt 0) { ($pairedDroppedDeltas | Measure-Object -Average).Average } else { $null }
    $maxPairedDroppedDelta = if ($pairedDroppedDeltas.Count -gt 0) { ($pairedDroppedDeltas | Measure-Object -Maximum).Maximum } else { $null }
    $meanPreHwRatio = ($preHwRatios | Measure-Object -Average).Average
    $meanPostHwRatio = ($postHwRatios | Measure-Object -Average).Average
    $maxRecLatency = if ($activeRecoveryLatencies.Count -gt 0) { ($activeRecoveryLatencies | Measure-Object -Maximum).Maximum } else { $null }
    $meanRecLatency = if ($activeRecoveryLatencies.Count -gt 0) { ($activeRecoveryLatencies | Measure-Object -Average).Average } else { $null }
    $maxRecLowerBound = if ($activeRecoveryLowerBounds.Count -gt 0) { ($activeRecoveryLowerBounds | Measure-Object -Maximum).Maximum } else { $null }

    # Determine unique or mixed CaptureExclusion across measured markers
    $exclusionValues = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($m in $measuredMarkers) {
        $null = $exclusionValues.Add($m.CaptureExclusion)
    }
    $topLevelExclusion = if ($exclusionValues.Count -eq 1) {
        $exclusionValues | Select-Object -First 1
    } elseif ($exclusionValues.Count -gt 1) {
        "mixed"
    } else {
        "unspecified"
    }

    # Determine unique or mixed LifecyclePolicy across measured markers
    $policyValues = [System.Collections.Generic.HashSet[string]]::new()
    foreach ($m in $measuredMarkers) {
        $null = $policyValues.Add($m.LifecyclePolicy)
    }
    $topLevelPolicy = if ($policyValues.Count -eq 1) {
        $policyValues | Select-Object -First 1
    } elseif ($policyValues.Count -gt 1) {
        "mixed"
    } else {
        "attached-shown"
    }

    # Telemetry Gate Evaluation:
    # 1. Drop telemetry must be available (Dropped column or DisplayedTime) to verify zero dropped/undisplayed frames
    # 2. Mean paired render-p99 delta <= 1.0 ms
    # 3. Max paired render-p99 delta <= 2.0 ms (diagnostic warning)
    # 4. No positive post-vs-pre dropped-frame delta
    # 5. Active cues: Pre window >= 99.5% Hardware-Independent Flip and <= 1.0% undisplayed
    # 6. Active cues: Recovery latency <= 200 ms post expected cue completion
    # 7. Active cues: PostRecovery500 and SteadyState are 100% Hardware-Independent Flip with 0 drops
    # 8. Control states (no-hud/idle): Pre and Post windows >= 99.5% Hardware-Independent Flip and <= 1.0% undisplayed
    $gateFailures = [System.Collections.Generic.List[string]]::new()
    $gateWarnings = [System.Collections.Generic.List[string]]::new()

    if (-not $pmData.DropMetricAvailable) {
        $gateFailures.Add("Drop telemetry (Dropped column or DisplayedTime) is unavailable in capture; zero-drop / undisplayed frame recovery cannot be verified.")
    }

    if ($meanPairedRenderDelta -gt 1.0) {
        $gateFailures.Add("Mean paired render-p99 delta ($([Math]::Round($meanPairedRenderDelta, 3)) ms) exceeded threshold (<= 1.0 ms)")
    }
    if ($maxPairedRenderDelta -gt 2.0) {
        $gateWarnings.Add("Max paired render-p99 delta ($([Math]::Round($maxPairedRenderDelta, 3)) ms) exceeded diagnostic threshold (<= 2.0 ms)")
    }
    if ($null -ne $maxPairedDroppedDelta -and $null -ne $meanPairedDroppedDelta) {
        if ($maxPairedDroppedDelta -gt 0 -or $meanPairedDroppedDelta -gt 0) {
            $gateFailures.Add("Positive post-vs-pre dropped frame delta detected (mean delta: $([Math]::Round($meanPairedDroppedDelta, 3)), max delta: $maxPairedDroppedDelta)")
        }
    }

    # Evaluate per-marker gating criteria
    for ($i = 0; $i -lt $markerDetails.Count; $i++) {
        $m = $markerDetails[$i]
        $isControl = ($m.State -in @("no-hud", "no_hud", "idle"))

        if ($isControl) {
            if ($m.Pre.HardwareIndependentRatio -lt 0.995) {
                $gateFailures.Add("Control marker #$($m.RunIndex) ($($m.State)) Pre Hardware-Independent ratio ($([Math]::Round($m.Pre.HardwareIndependentRatio * 100, 2))%) was below required 99.5%")
            }
            if ($m.Post.HardwareIndependentRatio -lt 0.995) {
                $gateFailures.Add("Control marker #$($m.RunIndex) ($($m.State)) Post Hardware-Independent ratio ($([Math]::Round($m.Post.HardwareIndependentRatio * 100, 2))%) was below required 99.5%")
            }
            if ($pmData.DropMetricAvailable) {
                if ($null -ne $m.Pre.DroppedRatio -and $m.Pre.DroppedRatio -gt 0.01) {
                    $gateFailures.Add("Control marker #$($m.RunIndex) ($($m.State)) Pre dropped frame ratio ($([Math]::Round($m.Pre.DroppedRatio * 100, 2))%) exceeded 1.0%")
                }
                if ($null -ne $m.Post.DroppedRatio -and $m.Post.DroppedRatio -gt 0.01) {
                    $gateFailures.Add("Control marker #$($m.RunIndex) ($($m.State)) Post dropped frame ratio ($([Math]::Round($m.Post.DroppedRatio * 100, 2))%) exceeded 1.0%")
                }
            }
        } else {
            # Active cue state
            if ($m.Pre.HardwareIndependentRatio -lt 0.995) {
                $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) Pre Hardware-Independent ratio ($([Math]::Round($m.Pre.HardwareIndependentRatio * 100, 2))%) was below required 99.5% baseline")
            }
            if ($pmData.DropMetricAvailable -and $null -ne $m.Pre.DroppedRatio -and $m.Pre.DroppedRatio -gt 0.01) {
                $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) Pre dropped frame ratio ($([Math]::Round($m.Pre.DroppedRatio * 100, 2))%) exceeded 1.0%")
            }
            if ($m.RecoveryObserved -eq $false) {
                $boundStr = if ($null -ne $m.RecoveryLowerBoundMs) { "$([Math]::Round($m.RecoveryLowerBoundMs, 1)) ms" } else { "observation window" }
                $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) no clean recovery observed for at least $boundStr (required <= 200 ms)")
            } elseif ($null -ne $m.RecoveryLatencyMs -and $m.RecoveryLatencyMs -gt 200.0) {
                $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) recovery latency ($([Math]::Round($m.RecoveryLatencyMs, 1)) ms) exceeded threshold (<= 200 ms)")
            }
            if ($m.PostRecovery500.HardwareIndependentRatio -lt 1.0) {
                $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) PostRecovery500 Hardware-Independent ratio ($([Math]::Round($m.PostRecovery500.HardwareIndependentRatio * 100, 1))%) was not 100%")
            }
            if ($m.SteadyState.HardwareIndependentRatio -lt 1.0) {
                $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) SteadyState Hardware-Independent ratio ($([Math]::Round($m.SteadyState.HardwareIndependentRatio * 100, 1))%) was not 100%")
            }
            if ($pmData.DropMetricAvailable) {
                if ($null -ne $m.PostRecovery500.DroppedCount -and $m.PostRecovery500.DroppedCount -gt 0) {
                    $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) PostRecovery500 had $($m.PostRecovery500.DroppedCount) dropped frame(s) (required 0)")
                }
                if ($null -ne $m.SteadyState.DroppedCount -and $m.SteadyState.DroppedCount -gt 0) {
                    $gateFailures.Add("Active marker #$($m.RunIndex) ($($m.State)) SteadyState had $($m.SteadyState.DroppedCount) dropped frame(s) (required 0)")
                }
            }
        }
    }

    $gatePassed = ($gateFailures.Count -eq 0)

    $disclaimer = if ($pmData.DropMetricAvailable) {
        "This telemetry gate evaluates frame pacing, dropped frames, and swapchain presentation modes. It does not prove focus, click-through, visual fidelity, or capture exclusion."
    } else {
        "This telemetry gate evaluates frame pacing and swapchain presentation modes; drop telemetry was unavailable so zero-drop recovery could not be proved. It does not prove focus, click-through, visual fidelity, or capture exclusion."
    }

    $result = [PSCustomObject]@{
        Timestamp            = [DateTime]::UtcNow.ToString("o")
        QuantileMethod       = "Linear Interpolation (Type 7 / Excel PERCENTILE.INC)"
        PresentMonCsv        = (Resolve-Path $PresentMonCsvPath).Path
        MarkerCsv            = (Resolve-Path $MarkerCsvPath).Path
        SelectedSwapChain    = $activeSwapChain
        DetectedSwapChains   = $pmData.SwapChainCounts
        RenderMetricChosen   = $pmData.RenderMetric
        DisplayMetricChosen  = $pmData.DisplayMetric
        DropMetricAvailable  = $pmData.DropMetricAvailable
        CaptureExclusion     = $topLevelExclusion
        LifecyclePolicy      = $topLevelPolicy
        QpcFrequencyHz       = $qpcFreq
        TotalFramesInCapture = $pmData.TotalFrames
        TotalMarkersInFile   = $markerData.TotalMarkers
        WarmupsExcluded      = $excludedWarmups
        ErrorsExcluded       = $excludedErrors
        MeasuredMarkersCount = $measuredMarkers.Count
        HeadlineSummary      = [PSCustomObject]@{
            MeanPairedRenderP99DeltaMs = [Math]::Round($meanPairedRenderDelta, 4)
            MaxPairedRenderP99DeltaMs  = [Math]::Round($maxPairedRenderDelta, 4)
            MeanPairedDroppedDelta     = if ($null -ne $meanPairedDroppedDelta) { [Math]::Round($meanPairedDroppedDelta, 4) } else { $null }
            MaxPairedDroppedDelta      = $maxPairedDroppedDelta
            MeanPreHardwareIndepRatio  = [Math]::Round($meanPreHwRatio, 4)
            MeanPostHardwareIndepRatio = [Math]::Round($meanPostHwRatio, 4)
            MaxRecoveryLatencyMs       = if ($null -ne $maxRecLatency) { [Math]::Round($maxRecLatency, 4) } else { $null }
            MeanRecoveryLatencyMs      = if ($null -ne $meanRecLatency) { [Math]::Round($meanRecLatency, 4) } else { $null }
            UnrecoveredMarkerCount     = $unrecoveredMarkerCount
            MaxRecoveryLowerBoundMs    = if ($null -ne $maxRecLowerBound) { [Math]::Round($maxRecLowerBound, 4) } else { $null }
        }
        TelemetryGate        = [PSCustomObject]@{
            Status   = if ($gatePassed) { "PASSED" } else { "FAILED" }
            Passed   = $gatePassed
            Failures = $gateFailures.ToArray()
            Warnings = $gateWarnings.ToArray()
            Rules    = @(
                "Drop telemetry must be available (Dropped column or DisplayedTime)",
                "Mean paired render-p99 delta <= 1.0 ms",
                "Max paired render-p99 delta <= 2.0 ms (diagnostic warning)",
                "No positive post-vs-pre dropped-frame delta",
                "Active cues: Pre window >= 99.5% Hardware-Independent Flip and <= 1.0% undisplayed",
                "Active cues: Recovery latency <= 200 ms post expected cue completion",
                "Active cues: PostRecovery500 and SteadyState 100% Hardware-Independent Flip with 0 drops",
                "Control states (no-hud/idle): Pre and Post windows >= 99.5% Hardware-Independent Flip and <= 1.0% undisplayed"
            )
        }
        Disclaimer           = $disclaimer
        MarkerDetails        = $markerDetails
    }

    # Write Output JSON if requested
    if ($OutputJsonPath) {
        $jsonDir = Split-Path -Parent $OutputJsonPath
        if ($jsonDir -and -not (Test-Path -LiteralPath $jsonDir)) {
            New-Item -ItemType Directory -Path $jsonDir -Force | Out-Null
        }
        $jsonStr = $result | ConvertTo-Json -Depth 8
        [System.IO.File]::WriteAllText($OutputJsonPath, $jsonStr, [System.Text.Encoding]::UTF8)
    }

    # Write Output Markdown if requested
    if ($OutputMarkdownPath) {
        $mdDir = Split-Path -Parent $OutputMarkdownPath
        if ($mdDir -and -not (Test-Path -LiteralPath $mdDir)) {
            New-Item -ItemType Directory -Path $mdDir -Force | Out-Null
        }
        $md = Generate-MarkdownReport -Result $result
        [System.IO.File]::WriteAllText($OutputMarkdownPath, $md, [System.Text.Encoding]::UTF8)
    }

    return $result
}

function Generate-MarkdownReport {
    param([PSCustomObject]$Result)

    $sb = [System.Text.StringBuilder]::new()
    $null = $sb.AppendLine("# Native HUD Telemetry Analysis Report")
    $null = $sb.AppendLine()
    $null = $sb.AppendLine("**Generated:** $($Result.Timestamp)")
    $null = $sb.AppendLine("**Quantile Method:** $($Result.QuantileMethod)")
    $null = $sb.AppendLine("**Render Metric:** $($Result.RenderMetricChosen)")
    $null = $sb.AppendLine("**Display Metric:** $($Result.DisplayMetricChosen)")
    $null = $sb.AppendLine("**SwapChain:** $($Result.SelectedSwapChain)")
    $null = $sb.AppendLine("**Capture Exclusion:** $($Result.CaptureExclusion)")
    $null = $sb.AppendLine("**Lifecycle Policy:** $($Result.LifecyclePolicy)")
    $null = $sb.AppendLine("**Measured Markers:** $($Result.MeasuredMarkersCount) (Excluded: $($Result.WarmupsExcluded) warmups, $($Result.ErrorsExcluded) errors)")
    $null = $sb.AppendLine()

    $gateBadge = if ($Result.TelemetryGate.Passed) { "## Gating Result: PASSED" } else { "## Gating Result: FAILED" }
    $null = $sb.AppendLine($gateBadge)
    $null = $sb.AppendLine()

    if (-not $Result.TelemetryGate.Passed) {
        $null = $sb.AppendLine("### Gate Violations:")
        foreach ($fail in $Result.TelemetryGate.Failures) {
            $null = $sb.AppendLine("- :x: $fail")
        }
        $null = $sb.AppendLine()
    }

    if ($Result.TelemetryGate.Warnings -and $Result.TelemetryGate.Warnings.Count -gt 0) {
        $null = $sb.AppendLine("### Gate Warnings:")
        foreach ($warn in $Result.TelemetryGate.Warnings) {
            $null = $sb.AppendLine("- :warning: $warn")
        }
        $null = $sb.AppendLine()
    }

    $null = $sb.AppendLine("### Headline Paired Summary (Per-Marker Paired Deltas)")
    $null = $sb.AppendLine("| Metric | Value | Threshold | Status |")
    $null = $sb.AppendLine("| :--- | :--- | :--- | :--- |")

    $meanRenderStatus = if ($Result.HeadlineSummary.MeanPairedRenderP99DeltaMs -le 1.0) { "PASS" } else { "FAIL" }
    $maxRenderStatus = if ($Result.HeadlineSummary.MaxPairedRenderP99DeltaMs -le 2.0) { "PASS" } else { "WARN" }
    $droppedStatus = if ($null -ne $Result.HeadlineSummary.MaxPairedDroppedDelta -and $null -ne $Result.HeadlineSummary.MeanPairedDroppedDelta) {
        if ($Result.HeadlineSummary.MaxPairedDroppedDelta -le 0 -and $Result.HeadlineSummary.MeanPairedDroppedDelta -le 0) { "PASS" } else { "FAIL" }
    } else {
        "N/A"
    }
    $hwRatioStatus = if ($Result.HeadlineSummary.MeanPreHardwareIndepRatio -lt 0.995 -or $Result.HeadlineSummary.MeanPostHardwareIndepRatio -ge 0.995) { "PASS" } else { "FAIL" }

    $maxDropStr = if ($null -ne $Result.HeadlineSummary.MaxPairedDroppedDelta) { "$($Result.HeadlineSummary.MaxPairedDroppedDelta)" } else { "N/A" }
    $meanDropStr = if ($null -ne $Result.HeadlineSummary.MeanPairedDroppedDelta) { "$($Result.HeadlineSummary.MeanPairedDroppedDelta)" } else { "N/A" }

    $null = $sb.AppendLine("| Mean Paired Render-p99 Delta | $($Result.HeadlineSummary.MeanPairedRenderP99DeltaMs) ms | <= 1.0 ms | $meanRenderStatus |")
    $null = $sb.AppendLine("| Max Paired Render-p99 Delta | $($Result.HeadlineSummary.MaxPairedRenderP99DeltaMs) ms | <= 2.0 ms (diagnostic) | $maxRenderStatus |")
    $null = $sb.AppendLine("| Max Paired Dropped Delta | $maxDropStr | <= 0 | $droppedStatus |")
    $null = $sb.AppendLine("| Mean Paired Dropped Delta | $meanDropStr | <= 0 | $droppedStatus |")
    $null = $sb.AppendLine("| Mean Hardware-Independent Ratio | Pre: $([Math]::Round($Result.HeadlineSummary.MeanPreHardwareIndepRatio * 100, 1))% -> Post: $([Math]::Round($Result.HeadlineSummary.MeanPostHardwareIndepRatio * 100, 1))% | Baseline >= 99.5% | $hwRatioStatus |")
    if ($null -ne $Result.HeadlineSummary.MaxRecoveryLatencyMs) {
        $recStatus = if ($Result.HeadlineSummary.MaxRecoveryLatencyMs -le 200.0) { "PASS" } else { "FAIL" }
        $null = $sb.AppendLine("| Max Recovery Latency | $($Result.HeadlineSummary.MaxRecoveryLatencyMs) ms | <= 200.0 ms | $recStatus |")
    }
    if ($null -ne $Result.HeadlineSummary.UnrecoveredMarkerCount -and $Result.HeadlineSummary.UnrecoveredMarkerCount -gt 0) {
        $null = $sb.AppendLine("| Unrecovered Markers | $($Result.HeadlineSummary.UnrecoveredMarkerCount) | 0 | FAIL |")
        if ($null -ne $Result.HeadlineSummary.MaxRecoveryLowerBoundMs) {
            $null = $sb.AppendLine("| Max Recovery Lower Bound | >= $($Result.HeadlineSummary.MaxRecoveryLowerBoundMs) ms | <= 200.0 ms | FAIL |")
        }
    }
    $null = $sb.AppendLine()

    $null = $sb.AppendLine("### Per-Marker Details")
    $null = $sb.AppendLine("| Run | State | Exclusion | Policy | Timing Source | Pre p99 | Post p99 | Paired Delta | Pre HW% | Rec Latency | PostRec500 HW% | Steady HW% | Pre Drops | Post Drops |")
    $null = $sb.AppendLine("| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |")

    foreach ($m in $Result.MarkerDetails) {
        $preP99 = [Math]::Round($m.Pre.RenderP99, 3)
        $postP99 = [Math]::Round($m.Post.RenderP99, 3)
        $delta = [Math]::Round($m.PairedRenderP99DeltaMs, 3)
        $preHw = [Math]::Round($m.Pre.HardwareIndependentRatio * 100, 1)
        $timing = $m.TimingSource
        $recLatStr = if ($m.RecoveryObserved -eq $false -and $null -ne $m.RecoveryLowerBoundMs) {
            ">= $([Math]::Round($m.RecoveryLowerBoundMs, 1)) ms (unrecovered)"
        } elseif ($null -ne $m.RecoveryLatencyMs) {
            "$([Math]::Round($m.RecoveryLatencyMs, 1)) ms"
        } else {
            "N/A"
        }
        $postRecHwStr = if ($null -ne $m.PostRecovery500) { "$([Math]::Round($m.PostRecovery500.HardwareIndependentRatio * 100, 1))%" } else { "N/A" }
        $steadyHwStr = if ($null -ne $m.SteadyState) { "$([Math]::Round($m.SteadyState.HardwareIndependentRatio * 100, 1))%" } else { "N/A" }
        $preDrop = if ($null -ne $m.Pre.DroppedCount) { "$($m.Pre.DroppedCount)" } else { "N/A" }
        $postDrop = if ($null -ne $m.Post.DroppedCount) { "$($m.Post.DroppedCount)" } else { "N/A" }
        $null = $sb.AppendLine("| $($m.RunIndex) | $($m.State) | $($m.CaptureExclusion) | $($m.LifecyclePolicy) | $timing | $preP99 ms | $postP99 ms | $delta ms | $preHw% | $recLatStr | $postRecHwStr | $steadyHwStr | $preDrop | $postDrop |")
    }

    $null = $sb.AppendLine()
    $null = $sb.AppendLine("> **Notice & Disclaimer:** $($Result.Disclaimer)")
    return $sb.ToString()
}

function Invoke-SelfTest {
    Write-Host "Running analyze-native-hud-presentmon SelfTest suite..." -ForegroundColor Cyan

    $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("presentmon_selftest_" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

    try {
        # 1. Test Quantile Calculation with known distribution
        $sampleVals = [double[]]@(1..100)
        $p99 = Get-Quantile -Values $sampleVals -Quantile 0.99
        $expectedP99 = 99.01
        if ([Math]::Abs($p99 - $expectedP99) -gt 0.001) {
            throw "SelfTest Failed: Get-Quantile expected $expectedP99, got $p99"
        }

        # 2. Test Slicing Boundaries, Real Schema with duration/interval metadata, transient composition in Post with clean recovery <= 200ms (PASS)
        $qpcFreq = 10000000L # 10 MHz
        $markerCsvPath = Join-Path $tempDir "markers.csv"
        $pmCsvPath = Join-Path $tempDir "presentmon.csv"
        $outJsonPath = Join-Path $tempDir "analysis.json"
        $outMdPath = Join-Path $tempDir "analysis.md"

        # Markers: Run 0 (warmup), Run 1 (saved, duration=2580ms, interval=5000ms), Run 2 (rejected error), Run 3 (idle control, duration=0, interval=5000ms)
        $markerCsvContent = @"
run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms
0,true,saved,full,bottom-center,on,attached-shown,10000000,10000000,accepted,2580,5000
1,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,2580,5000
2,false,saved,full,bottom-center,on,attached-shown,40000000,10000000,rejected,2580,5000
3,false,idle,full,bottom-center,on,attached-shown,80000000,10000000,not_submitted,0,5000
"@
        [System.IO.File]::WriteAllText($markerCsvPath, $markerCsvContent, [System.Text.Encoding]::UTF8)

        $pmLines = [System.Collections.Generic.List[string]]::new()
        $pmLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")

        # Boundary test frame at 9,999,999 (should NOT be in pre-window of Run 1)
        $pmLines.Add("9999999,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")

        # Exact pre-window frames for Run 1: 10,000,000 to 19,833,333 (60 frames) -> 100% Independent Flip
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $pmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }

        # Exact post-window frames for Run 1: 20,000,000 to 30,000,000 -> Composed: Flip (transient composition during active cue allowed!)
        for ($q = 20000000L; $q -le 30000000L; $q += 166666L) {
            $pmLines.Add("$q,17.10,17.10,17.10,Composed: Flip,0x1000")
        }

        # Active cue continues composed until completion at 45,800,000 (T0 + 2580ms = 45,800,000 ticks)
        for ($q = 30166666L; $q -lt 45800000L; $q += 166666L) {
            $pmLines.Add("$q,16.66,16.66,16.66,Composed: Flip,0x1000")
        }

        # At completion (45,800,000): one composed frame
        $pmLines.Add("45800000,16.66,16.66,16.66,Composed: Flip,0x1000")
        # At completion + 20ms (46,000,000): recovered to clean Independent Flip! (Recovery latency = 20ms <= 200ms)
        $pmLines.Add("46000000,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")

        # Rest of RecoveryProbe [45.8M, 47.8M], PostRecovery500 [47.8M, 52.8M], and SteadyState [52.8M, 70.0M): 100% Independent Flip
        for ($q = 46166666L; $q -lt 70000000L; $q += 166666L) {
            $pmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }

        # Pre and Post frames for Run 3 (idle control state, T0 = 80,000,000): 100% Independent Flip
        for ($q = 70000000L; $q -lt 80000000L; $q += 166666L) {
            $pmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        for ($q = 80000000L; $q -le 90000000L; $q += 166666L) {
            $pmLines.Add("$q,16.80,16.80,16.80,Hardware: Independent Flip,0x1000")
        }

        # Secondary swapchain (0x2000) to test auto-selection
        $pmLines.Add("15000000,16.66,16.66,16.66,Hardware: Independent Flip,0x2000")
        $pmLines.Add("25000000,16.66,16.66,16.66,Hardware: Independent Flip,0x2000")

        [System.IO.File]::WriteAllLines($pmCsvPath, $pmLines)

        $res = Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $markerCsvPath -OutputJsonPath $outJsonPath -OutputMarkdownPath $outMdPath

        if ($res.MeasuredMarkersCount -ne 2) {
            throw "SelfTest Failed: Expected 2 measured markers, got $($res.MeasuredMarkersCount)"
        }
        if ($res.WarmupsExcluded -ne 1) {
            throw "SelfTest Failed: Expected 1 warmup excluded, got $($res.WarmupsExcluded)"
        }
        if ($res.ErrorsExcluded -ne 1) {
            throw "SelfTest Failed: Expected 1 error excluded, got $($res.ErrorsExcluded)"
        }
        if ($res.CaptureExclusion -ne "on") {
            throw "SelfTest Failed: Expected top-level CaptureExclusion 'on', got '$($res.CaptureExclusion)'"
        }
        if ($res.LifecyclePolicy -ne "attached-shown") {
            throw "SelfTest Failed: Expected top-level LifecyclePolicy 'attached-shown', got '$($res.LifecyclePolicy)'"
        }
        if ($res.SelectedSwapChain -ne "0x1000") {
            throw "SelfTest Failed: Expected auto-selected swapchain '0x1000', got '$($res.SelectedSwapChain)'"
        }
        if (-not $res.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected telemetry gate to PASS with transient composition and <=200ms recovery, but failed with: $($res.TelemetryGate.Failures -join '; ')"
        }
        $m1 = $res.MarkerDetails | Where-Object { $_.RunIndex -eq 1 }
        if ($m1.TimingSource -ne "marker_metadata") {
            throw "SelfTest Failed: Expected TimingSource 'marker_metadata', got '$($m1.TimingSource)'"
        }
        if ($m1.NominalDurationMs -ne 2580) {
            throw "SelfTest Failed: Expected NominalDurationMs 2580, got $($m1.NominalDurationMs)"
        }
        if ($m1.ExpectedCompletionQpc -ne 45800000L) {
            throw "SelfTest Failed: Expected ExpectedCompletionQpc 45800000, got $($m1.ExpectedCompletionQpc)"
        }
        if ($m1.RecoveryObserved -ne $true) {
            throw "SelfTest Failed: Expected RecoveryObserved true, got $($m1.RecoveryObserved)"
        }
        if ($null -ne $m1.RecoveryLowerBoundMs) {
            throw "SelfTest Failed: Expected RecoveryLowerBoundMs null when clean recovery observed, got $($m1.RecoveryLowerBoundMs)"
        }
        if ($m1.RecoveryLatencyMs -gt 25.0 -or $m1.RecoveryLatencyMs -lt 15.0) {
            throw "SelfTest Failed: Expected RecoveryLatencyMs ~20ms, got $($m1.RecoveryLatencyMs)"
        }
        if ($res.HeadlineSummary.UnrecoveredMarkerCount -ne 0) {
            throw "SelfTest Failed: Expected UnrecoveredMarkerCount 0, got $($res.HeadlineSummary.UnrecoveredMarkerCount)"
        }
        if ($null -ne $res.HeadlineSummary.MaxRecoveryLowerBoundMs) {
            throw "SelfTest Failed: Expected MaxRecoveryLowerBoundMs null when all recovered, got $($res.HeadlineSummary.MaxRecoveryLowerBoundMs)"
        }
        if ($m1.PostRecovery500.HardwareIndependentRatio -ne 1.0) {
            throw "SelfTest Failed: Expected PostRecovery500 HW ratio 1.0, got $($m1.PostRecovery500.HardwareIndependentRatio)"
        }
        if ($m1.SteadyState.HardwareIndependentRatio -ne 1.0) {
            throw "SelfTest Failed: Expected SteadyState HW ratio 1.0, got $($m1.SteadyState.HardwareIndependentRatio)"
        }
        if ($m1.Pre.PresentModeDistribution.Count -eq 0) {
            throw "SelfTest Failed: PresentModeDistribution was empty map in Pre stats"
        }

        # Check Control Marker (Run 3)
        $m3 = $res.MarkerDetails | Where-Object { $_.RunIndex -eq 3 }
        if ($null -ne $m3.ExpectedCompletionQpc -or $null -ne $m3.RecoveryProbe -or $null -ne $m3.RecoveryLatencyMs -or $null -ne $m3.RecoveryObserved -or $null -ne $m3.RecoveryLowerBoundMs) {
            throw "SelfTest Failed: Control marker should not have active completion or recovery probe"
        }

        if (-not (Test-Path -LiteralPath $outJsonPath) -or -not (Test-Path -LiteralPath $outMdPath)) {
            throw "SelfTest Failed: Output JSON or Markdown was not created."
        }

        # 3. Test Backward-Compatibility With Old Marker Schema (without duration/interval/capture_exclusion/lifecycle_policy)
        $oldMarkerCsv = Join-Path $tempDir "old_schema_markers.csv"
        $oldMarkerContent = @"
run_index,warmup,state,mode,anchor,trigger_qpc,qpc_frequency,submit_result
0,false,saved,full,bottom-center,20000000,10000000,accepted
"@
        [System.IO.File]::WriteAllText($oldMarkerCsv, $oldMarkerContent, [System.Text.Encoding]::UTF8)
        $oldRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $oldMarkerCsv
        if ($oldRes.CaptureExclusion -ne "unspecified") {
            throw "SelfTest Failed: Expected CaptureExclusion 'unspecified' for legacy marker schema, got '$($oldRes.CaptureExclusion)'"
        }
        if ($oldRes.LifecyclePolicy -ne "attached-shown") {
            throw "SelfTest Failed: Expected LifecyclePolicy 'attached-shown' for legacy marker schema fallback, got '$($oldRes.LifecyclePolicy)'"
        }
        if ($oldRes.MarkerDetails[0].TimingSource -ne "legacy_derived") {
            throw "SelfTest Failed: Expected TimingSource 'legacy_derived' for legacy marker, got '$($oldRes.MarkerDetails[0].TimingSource)'"
        }
        if ($oldRes.MarkerDetails[0].NominalDurationMs -ne 2580) {
            throw "SelfTest Failed: Expected derived NominalDurationMs 2580 for legacy saved marker, got $($oldRes.MarkerDetails[0].NominalDurationMs)"
        }
        if ($oldRes.MarkerDetails[0].IntervalMs -ne 5000) {
            throw "SelfTest Failed: Expected inferred IntervalMs 5000 for legacy marker, got $($oldRes.MarkerDetails[0].IntervalMs)"
        }
        if (-not $oldRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected legacy marker run to PASS."
        }

        # 4. Test Recovery > 200ms (FAIL)
        $slowRecPmPath = Join-Path $tempDir "slow_rec_pm.csv"
        $slowLines = [System.Collections.Generic.List[string]]::new()
        $slowLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $slowLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        # Composed until 48,500,000 (which is completion 45.8M + 270ms > 200ms)
        for ($q = 20000000L; $q -le 48500000L; $q += 166666L) {
            $slowLines.Add("$q,16.66,16.66,16.66,Composed: Flip,0x1000")
        }
        # Clean recovery only at 48.66M
        for ($q = 48666666L; $q -lt 70000000L; $q += 166666L) {
            $slowLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        [System.IO.File]::WriteAllLines($slowRecPmPath, $slowLines)

        $singleSavedMarkerCsv = Join-Path $tempDir "single_saved.csv"
        [System.IO.File]::WriteAllText($singleSavedMarkerCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,2580,5000", [System.Text.Encoding]::UTF8)

        $slowRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $slowRecPmPath -MarkerCsvPath $singleSavedMarkerCsv
        if ($slowRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected telemetry gate to FAIL when recovery latency > 200ms."
        }
        $hasRecFailure = ($slowRes.TelemetryGate.Failures | Where-Object { $_ -match "recovery latency|PostRecovery500" }).Count -gt 0
        if (-not $hasRecFailure) {
            throw "SelfTest Failed: Expected gate failure message to mention recovery latency or PostRecovery500, got: $($slowRes.TelemetryGate.Failures -join '; ')"
        }
        $slowMarker = $slowRes.MarkerDetails[0]
        if ($slowMarker.RecoveryObserved -ne $true) {
            throw "SelfTest Failed: Expected RecoveryObserved true for slow recovery, got $($slowMarker.RecoveryObserved)"
        }
        if ($null -eq $slowMarker.RecoveryLatencyMs -or $slowMarker.RecoveryLatencyMs -le 200.0) {
            throw "SelfTest Failed: Expected RecoveryLatencyMs > 200ms for slow recovery, got $($slowMarker.RecoveryLatencyMs)"
        }
        if ($null -ne $slowMarker.RecoveryLowerBoundMs) {
            throw "SelfTest Failed: Expected RecoveryLowerBoundMs null for slow recovery, got $($slowMarker.RecoveryLowerBoundMs)"
        }
        if ($slowRes.HeadlineSummary.UnrecoveredMarkerCount -ne 0) {
            throw "SelfTest Failed: Expected UnrecoveredMarkerCount 0 for slow recovery, got $($slowRes.HeadlineSummary.UnrecoveredMarkerCount)"
        }

        # 5. Test Persistent Composed Latch (FAIL)
        $latchPmPath = Join-Path $tempDir "latch_pm.csv"
        $latchLines = [System.Collections.Generic.List[string]]::new()
        $latchLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $latchLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        for ($q = 20000000L; $q -lt 70000000L; $q += 166666L) {
            $latchLines.Add("$q,16.66,16.66,16.66,Composed: Flip,0x1000")
        }
        [System.IO.File]::WriteAllLines($latchPmPath, $latchLines)

        $latchRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $latchPmPath -MarkerCsvPath $singleSavedMarkerCsv
        if ($latchRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected telemetry gate to FAIL on persistent composed latch."
        }
        $hasLatchFail = ($latchRes.TelemetryGate.Failures | Where-Object { $_ -match "SteadyState|PostRecovery500|recovery latency|no clean recovery observed" }).Count -gt 0
        if (-not $hasLatchFail) {
            throw "SelfTest Failed: Expected failure message to mention SteadyState or PostRecovery500 or recovery latency for latch."
        }
        $hasCleanRecFail = ($latchRes.TelemetryGate.Failures | Where-Object { $_ -match "no clean recovery observed for at least" }).Count -gt 0
        if (-not $hasCleanRecFail) {
            throw "SelfTest Failed: Expected failure message to explicitly contain 'no clean recovery observed for at least X ms', got: $($latchRes.TelemetryGate.Failures -join '; ')"
        }
        $latchMarker = $latchRes.MarkerDetails[0]
        if ($latchMarker.RecoveryObserved -ne $false) {
            throw "SelfTest Failed: Expected RecoveryObserved false for persistent latch, got $($latchMarker.RecoveryObserved)"
        }
        if ($null -ne $latchMarker.RecoveryLatencyMs) {
            throw "SelfTest Failed: Expected RecoveryLatencyMs null for persistent latch, got $($latchMarker.RecoveryLatencyMs)"
        }
        if ($null -eq $latchMarker.RecoveryLowerBoundMs -or [Math]::Abs($latchMarker.RecoveryLowerBoundMs - 2420.0) -gt 0.1) {
            throw "SelfTest Failed: Expected RecoveryLowerBoundMs ~2420ms for persistent latch, got $($latchMarker.RecoveryLowerBoundMs)"
        }
        if ($latchRes.HeadlineSummary.UnrecoveredMarkerCount -ne 1) {
            throw "SelfTest Failed: Expected UnrecoveredMarkerCount 1 for persistent latch, got $($latchRes.HeadlineSummary.UnrecoveredMarkerCount)"
        }
        if ($null -ne $latchRes.HeadlineSummary.MaxRecoveryLatencyMs) {
            throw "SelfTest Failed: Expected MaxRecoveryLatencyMs null for persistent latch, got $($latchRes.HeadlineSummary.MaxRecoveryLatencyMs)"
        }
        if ($null -eq $latchRes.HeadlineSummary.MaxRecoveryLowerBoundMs -or [Math]::Abs($latchRes.HeadlineSummary.MaxRecoveryLowerBoundMs - 2420.0) -gt 0.1) {
            throw "SelfTest Failed: Expected MaxRecoveryLowerBoundMs ~2420ms for persistent latch, got $($latchRes.HeadlineSummary.MaxRecoveryLowerBoundMs)"
        }

        # 6. Test Control Window Mode & Dropped Thresholds
        $idleMarkerCsv = Join-Path $tempDir "idle_single.csv"
        [System.IO.File]::WriteAllText($idleMarkerCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,idle,full,bottom-center,on,attached-shown,20000000,10000000,not_submitted,0,5000", [System.Text.Encoding]::UTF8)

        # 6a: Control Pre HW ratio < 99.5% -> FAIL
        $lowHwControlLines = [System.Collections.Generic.List[string]]::new()
        $lowHwControlLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")
        $firstLow = $true
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            if ($firstLow) {
                $lowHwControlLines.Add("$q,16.66,16.66,16.66,Composed: Flip,0x1000")
                $firstLow = $false
            } else {
                $lowHwControlLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
            }
        }
        for ($q = 20000000L; $q -lt 70000000L; $q += 166666L) {
            $lowHwControlLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        $lowHwControlPath = Join-Path $tempDir "low_hw_control_pm.csv"
        [System.IO.File]::WriteAllLines($lowHwControlPath, $lowHwControlLines)

        $lowHwControlRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $lowHwControlPath -MarkerCsvPath $idleMarkerCsv
        if ($lowHwControlRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected control marker with <99.5% Pre HW flip to FAIL."
        }

        # 6b: Control Post dropped ratio > 1.0% -> FAIL
        $highDropControlLines = [System.Collections.Generic.List[string]]::new()
        $highDropControlLines.Add("CPUStartQPC,MsBetweenPresents,Dropped,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $highDropControlLines.Add("$q,16.66,0,Hardware: Independent Flip,0x1000")
        }
        $dropCount = 0
        for ($q = 20000000L; $q -le 30000000L; $q += 166666L) {
            if ($dropCount -lt 2) {
                $highDropControlLines.Add("$q,16.66,1,Hardware: Independent Flip,0x1000")
                $dropCount++
            } else {
                $highDropControlLines.Add("$q,16.66,0,Hardware: Independent Flip,0x1000")
            }
        }
        $highDropControlPath = Join-Path $tempDir "high_drop_control_pm.csv"
        [System.IO.File]::WriteAllLines($highDropControlPath, $highDropControlLines)

        $highDropControlRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $highDropControlPath -MarkerCsvPath $idleMarkerCsv
        if ($highDropControlRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected control marker with >1% drops in Post to FAIL."
        }

        # 6c: Active Pre window < 99.5% baseline latch FAIL
        $activeLowPreRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $lowHwControlPath -MarkerCsvPath $singleSavedMarkerCsv
        if ($activeLowPreRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected active marker with <99.5% Pre HW flip to FAIL."
        }
        $hasActivePreFail = ($activeLowPreRes.TelemetryGate.Failures | Where-Object { $_ -match "Pre Hardware-Independent ratio" }).Count -gt 0
        if (-not $hasActivePreFail) {
            throw "SelfTest Failed: Expected active pre failure message, got: $($activeLowPreRes.TelemetryGate.Failures -join '; ')"
        }

        # 7. Test Diagnostic Lifecycle Policy (detached-shown / attached-hidden)
        $diagMarkerCsv = Join-Path $tempDir "diag_markers.csv"
        $diagMarkerContent = @"
run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms
0,false,saved,full,bottom-center,off,detached-shown,20000000,10000000,accepted,2580,5000
1,false,queued-to-saved,full,bottom-center,off,detached-shown,80000000,10000000,"queued_error: bad status, code 400; saved_error: timeout",2650,5000
"@
        [System.IO.File]::WriteAllText($diagMarkerCsv, $diagMarkerContent, [System.Text.Encoding]::UTF8)
        $diagRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $diagMarkerCsv
        if ($diagRes.MeasuredMarkersCount -ne 1 -or $diagRes.ErrorsExcluded -ne 1) {
            throw "SelfTest Failed: Diagnostic marker result parsing failed. Measured: $($diagRes.MeasuredMarkersCount), Excluded: $($diagRes.ErrorsExcluded)"
        }
        if ($diagRes.LifecyclePolicy -ne "detached-shown") {
            throw "SelfTest Failed: Expected LifecyclePolicy 'detached-shown', got '$($diagRes.LifecyclePolicy)'"
        }

        # 8. Test DisplayedTime as Display Metric Fallback (MsBetweenDisplayChange column absent)
        $dispFallbackLines = [System.Collections.Generic.List[string]]::new()
        $dispFallbackLines.Add("CPUStartQPC,MsBetweenPresents,DisplayedTime,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $dispFallbackLines.Add("$q,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        for ($q = 20000000L; $q -lt 70000000L; $q += 166666L) {
            $dispFallbackLines.Add("$q,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        $dispFallbackPath = Join-Path $tempDir "disp_fallback_pm.csv"
        [System.IO.File]::WriteAllLines($dispFallbackPath, $dispFallbackLines)

        $fallbackRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $dispFallbackPath -MarkerCsvPath $singleSavedMarkerCsv
        if ($fallbackRes.DisplayMetricChosen -ne "DisplayedTime") {
            throw "SelfTest Failed: Expected DisplayMetricChosen to be 'DisplayedTime', got '$($fallbackRes.DisplayMetricChosen)'"
        }
        if ([Math]::Abs($fallbackRes.MarkerDetails[0].Pre.DisplayMean - 16.66) -gt 0.001) {
            throw "SelfTest Failed: Display metric values not correctly populated under DisplayedTime fallback (got $($fallbackRes.MarkerDetails[0].Pre.DisplayMean))."
        }

        # 9. Test Explicit Dropped Column Without DisplayedTime Column
        $explicitDropLines = [System.Collections.Generic.List[string]]::new()
        $explicitDropLines.Add("CPUStartQPC,MsBetweenPresents,Dropped,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $explicitDropLines.Add("$q,16.66,0,Hardware: Independent Flip,0x1000")
        }
        $first = $true
        for ($q = 20000000L; $q -le 30000000L; $q += 166666L) {
            if ($first) {
                $explicitDropLines.Add("$q,16.66,1,Hardware: Independent Flip,0x1000")
                $first = $false
            } else {
                $explicitDropLines.Add("$q,16.66,0,Hardware: Independent Flip,0x1000")
            }
        }
        for ($q = 30166666L; $q -lt 70000000L; $q += 166666L) {
            $explicitDropLines.Add("$q,16.66,0,Hardware: Independent Flip,0x1000")
        }
        $explicitDropPath = Join-Path $tempDir "explicit_drop_pm.csv"
        [System.IO.File]::WriteAllLines($explicitDropPath, $explicitDropLines)

        $explicitDropRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $explicitDropPath -MarkerCsvPath $singleSavedMarkerCsv
        if ($explicitDropRes.MarkerDetails[0].Pre.DroppedCount -ne 0) {
            throw "SelfTest Failed: Expected 0 pre drops when Dropped=0 without DisplayedTime, got $($explicitDropRes.MarkerDetails[0].Pre.DroppedCount)"
        }
        if ($explicitDropRes.MarkerDetails[0].Post.DroppedCount -ne 1) {
            throw "SelfTest Failed: Expected 1 post drop when Dropped=1 without DisplayedTime, got $($explicitDropRes.MarkerDetails[0].Post.DroppedCount)"
        }
        if ($explicitDropRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected telemetry gate to FAIL on positive post dropped delta."
        }

        # 10. Test Empty Probe Window Rejection
        $emptyProbeLines = [System.Collections.Generic.List[string]]::new()
        $emptyProbeLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -le 30000000L; $q += 166666L) {
            $emptyProbeLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        $emptyProbePath = Join-Path $tempDir "empty_probe_pm.csv"
        [System.IO.File]::WriteAllLines($emptyProbePath, $emptyProbeLines)

        $caughtEmptyProbe = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $emptyProbePath -MarkerCsvPath $singleSavedMarkerCsv
        } catch {
            $caughtEmptyProbe = $true
        }
        if (-not $caughtEmptyProbe) {
            throw "SelfTest Failed: Expected empty RecoveryProbe window to throw error."
        }

        # 11. Test Empty Pre-Window Rejection
        $emptyPreLines = [System.Collections.Generic.List[string]]::new()
        $emptyPreLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")
        for ($q = 20000000L; $q -lt 70000000L; $q += 166666L) {
            $emptyPreLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        $emptyPrePath = Join-Path $tempDir "empty_pre_pm.csv"
        [System.IO.File]::WriteAllLines($emptyPrePath, $emptyPreLines)

        $caughtEmptyPre = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $emptyPrePath -MarkerCsvPath $singleSavedMarkerCsv
        } catch {
            $caughtEmptyPre = $true
        }
        if (-not $caughtEmptyPre) {
            throw "SelfTest Failed: Expected empty pre-window to throw error."
        }

        # 12. Test Missing CPUStartQPC Rejection
        $noQpcCsv = Join-Path $tempDir "no_qpc.csv"
        [System.IO.File]::WriteAllText($noQpcCsv, "Time,MsBetweenPresents`n1,16.6`n", [System.Text.Encoding]::UTF8)
        $caughtNoQpc = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $noQpcCsv -MarkerCsvPath $singleSavedMarkerCsv
        } catch {
            $caughtNoQpc = $true
        }
        if (-not $caughtNoQpc) {
            throw "SelfTest Failed: Expected missing CPUStartQPC to throw error."
        }

        # 13. Test Mixed QPC Frequencies Rejection
        $mixedMarkerCsv = Join-Path $tempDir "mixed_markers.csv"
        $mixedContent = @"
run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms
0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,2580,5000
1,false,saved,full,bottom-center,on,attached-shown,60000000,20000000,accepted,2580,5000
"@
        [System.IO.File]::WriteAllText($mixedMarkerCsv, $mixedContent, [System.Text.Encoding]::UTF8)
        $caughtMixed = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $mixedMarkerCsv
        } catch {
            $caughtMixed = $true
        }
        if (-not $caughtMixed) {
            throw "SelfTest Failed: Expected mixed QPC frequencies to throw error."
        }

        # 14. Test All-Excluded Markers Rejection
        $noMarkersCsv = Join-Path $tempDir "no_markers.csv"
        $noMarkersContent = @"
run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms
0,true,saved,full,bottom-center,on,attached-shown,10000000,10000000,accepted,2580,5000
1,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,rejected,2580,5000
"@
        [System.IO.File]::WriteAllText($noMarkersCsv, $noMarkersContent, [System.Text.Encoding]::UTF8)
        $caughtNoMarkers = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $noMarkersCsv
        } catch {
            $caughtNoMarkers = $true
        }
        if (-not $caughtNoMarkers) {
            throw "SelfTest Failed: Expected all-excluded markers to throw error."
        }

        # 15. Test Unavailable Drop Telemetry (Fail-Closed Gate Status)
        $noDropPmPath = Join-Path $tempDir "no_drop_pm.csv"
        $noDropLines = [System.Collections.Generic.List[string]]::new()
        $noDropLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $noDropLines.Add("$q,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        for ($q = 20000000L; $q -lt 70000000L; $q += 166666L) {
            $noDropLines.Add("$q,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        [System.IO.File]::WriteAllLines($noDropPmPath, $noDropLines)

        $noDropRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $noDropPmPath -MarkerCsvPath $singleSavedMarkerCsv
        if ($noDropRes.DropMetricAvailable) {
            throw "SelfTest Failed: Expected DropMetricAvailable to be false when Dropped and DisplayedTime are absent."
        }
        if ($noDropRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected TelemetryGate to FAIL closed when drop telemetry is unavailable."
        }
        if ($noDropRes.TelemetryGate.Status -ne "FAILED") {
            throw "SelfTest Failed: Expected gate status FAILED, got '$($noDropRes.TelemetryGate.Status)'."
        }
        $hasDropFailMsg = ($noDropRes.TelemetryGate.Failures | Where-Object { $_ -match "Drop telemetry .* unavailable" }).Count -gt 0
        if (-not $hasDropFailMsg) {
            throw "SelfTest Failed: Expected gate failure message for unavailable drop telemetry, got: $($noDropRes.TelemetryGate.Failures -join '; ')"
        }
        if (-not ($noDropRes.Disclaimer -match "drop telemetry was unavailable")) {
            throw "SelfTest Failed: Expected disclaimer to mention unavailable drop telemetry."
        }

        # 16. Test Present-But-Invalid Metadata Rejection
        # 16a: Negative nominal_duration_ms
        $negDurCsv = Join-Path $tempDir "neg_dur.csv"
        [System.IO.File]::WriteAllText($negDurCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,-10,5000", [System.Text.Encoding]::UTF8)
        $caughtNegDur = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $negDurCsv
        } catch {
            $caughtNegDur = $_.Exception.Message -match "Malformed or negative"
        }
        if (-not $caughtNegDur) {
            throw "SelfTest Failed: Expected negative nominal_duration_ms to be rejected."
        }

        # 16b: Malformed nominal_duration_ms
        $malDurCsv = Join-Path $tempDir "mal_dur.csv"
        [System.IO.File]::WriteAllText($malDurCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,not_a_number,5000", [System.Text.Encoding]::UTF8)
        $caughtMalDur = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $malDurCsv
        } catch {
            $caughtMalDur = $_.Exception.Message -match "Malformed or negative"
        }
        if (-not $caughtMalDur) {
            throw "SelfTest Failed: Expected non-numeric nominal_duration_ms to be rejected."
        }

        # 16c: Interval below floor (< 5000ms)
        $shortIntCsv = Join-Path $tempDir "short_int.csv"
        [System.IO.File]::WriteAllText($shortIntCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,2580,4999", [System.Text.Encoding]::UTF8)
        $caughtShortInt = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $shortIntCsv
        } catch {
            $caughtShortInt = $_.Exception.Message -match "Malformed or invalid interval_ms"
        }
        if (-not $caughtShortInt) {
            throw "SelfTest Failed: Expected interval_ms < 5000 to be rejected."
        }

        # 16d: Malformed interval_ms
        $malIntCsv = Join-Path $tempDir "mal_int.csv"
        [System.IO.File]::WriteAllText($malIntCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,2580,bad_interval", [System.Text.Encoding]::UTF8)
        $caughtMalInt = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $malIntCsv
        } catch {
            $caughtMalInt = $_.Exception.Message -match "Malformed or invalid interval_ms"
        }
        if (-not $caughtMalInt) {
            throw "SelfTest Failed: Expected non-numeric interval_ms to be rejected."
        }

        # 16e: Empty duration cell with header present
        $emptyDurCsv = Join-Path $tempDir "empty_dur.csv"
        [System.IO.File]::WriteAllText($emptyDurCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,saved,full,bottom-center,on,attached-shown,20000000,10000000,accepted,,5000", [System.Text.Encoding]::UTF8)
        $caughtEmptyDur = $false
        try {
            Invoke-PresentMonAnalysis -PresentMonCsvPath $pmCsvPath -MarkerCsvPath $emptyDurCsv
        } catch {
            $caughtEmptyDur = $_.Exception.Message -match "Malformed or negative"
        }
        if (-not $caughtEmptyDur) {
            throw "SelfTest Failed: Expected empty nominal_duration_ms to be rejected."
        }

        # 17. Test Lone Max-p99 Outlier Diagnostic Warning (Warns but Gate PASSES)
        $outlierMarkerCsv = Join-Path $tempDir "outlier_markers.csv"
        $outlierMarkerContent = @"
run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms
0,false,idle,full,bottom-center,on,attached-shown,20000000,10000000,not_submitted,0,5000
1,false,idle,full,bottom-center,on,attached-shown,40000000,10000000,not_submitted,0,5000
2,false,idle,full,bottom-center,on,attached-shown,60000000,10000000,not_submitted,0,5000
3,false,idle,full,bottom-center,on,attached-shown,80000000,10000000,not_submitted,0,5000
4,false,idle,full,bottom-center,on,attached-shown,100000000,10000000,not_submitted,0,5000
"@
        [System.IO.File]::WriteAllText($outlierMarkerCsv, $outlierMarkerContent, [System.Text.Encoding]::UTF8)

        $outlierPmLines = [System.Collections.Generic.List[string]]::new()
        $outlierPmLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")

        # Runs 0..3: perfect 16.66ms pre and post (delta = 0.0ms)
        for ($mIdx = 0; $mIdx -lt 4; $mIdx++) {
            $t = (20000000L + ($mIdx * 20000000L))
            # Pre: [t - 1s, t)
            for ($q = ($t - 10000000L); $q -lt $t; $q += 166666L) {
                $outlierPmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
            }
            # Post: [t, t + 1s]
            for ($q = $t; $q -le ($t + 10000000L); $q += 166666L) {
                $outlierPmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
            }
        }

        # Run 4: Pre 16.66ms, Post has an outlier delta of 3.5ms (20.16ms)
        $t4 = 100000000L
        for ($q = ($t4 - 10000000L); $q -lt $t4; $q += 166666L) {
            $outlierPmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        for ($q = $t4; $q -le ($t4 + 10000000L); $q += 166666L) {
            $outlierPmLines.Add("$q,20.16,20.16,20.16,Hardware: Independent Flip,0x1000")
        }

        $outlierPmPath = Join-Path $tempDir "outlier_pm.csv"
        $outlierJsonPath = Join-Path $tempDir "outlier_analysis.json"
        $outlierMdPath = Join-Path $tempDir "outlier_analysis.md"
        [System.IO.File]::WriteAllLines($outlierPmPath, $outlierPmLines)

        $outlierRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $outlierPmPath -MarkerCsvPath $outlierMarkerCsv -OutputJsonPath $outlierJsonPath -OutputMarkdownPath $outlierMdPath
        if (-not $outlierRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected telemetry gate to PASS with lone max-p99 outlier when mean delta <= 1ms and 100% HW flip."
        }
        if ($outlierRes.TelemetryGate.Status -ne "PASSED") {
            throw "SelfTest Failed: Expected TelemetryGate.Status 'PASSED', got '$($outlierRes.TelemetryGate.Status)'."
        }
        if ($outlierRes.TelemetryGate.Failures.Count -ne 0) {
            throw "SelfTest Failed: Expected 0 failures, got $($outlierRes.TelemetryGate.Failures.Count): $($outlierRes.TelemetryGate.Failures -join '; ')"
        }
        if ($outlierRes.TelemetryGate.Warnings.Count -ne 1) {
            throw "SelfTest Failed: Expected 1 warning for max paired delta outlier, got $($outlierRes.TelemetryGate.Warnings.Count)"
        }
        if (-not ($outlierRes.TelemetryGate.Warnings[0] -match "Max paired render-p99 delta")) {
            throw "SelfTest Failed: Expected warning to mention 'Max paired render-p99 delta', got: '$($outlierRes.TelemetryGate.Warnings[0])'"
        }
        if ($outlierRes.HeadlineSummary.MeanPairedRenderP99DeltaMs -gt 1.0) {
            throw "SelfTest Failed: Expected MeanPairedRenderP99DeltaMs <= 1.0 ms, got $($outlierRes.HeadlineSummary.MeanPairedRenderP99DeltaMs)"
        }
        if ($outlierRes.HeadlineSummary.MaxPairedRenderP99DeltaMs -le 2.0) {
            throw "SelfTest Failed: Expected MaxPairedRenderP99DeltaMs > 2.0 ms, got $($outlierRes.HeadlineSummary.MaxPairedRenderP99DeltaMs)"
        }

        # Verify Markdown and JSON contain Warnings
        $outlierMdContent = [System.IO.File]::ReadAllText($outlierMdPath, [System.Text.Encoding]::UTF8)
        if (-not ($outlierMdContent -match "### Gate Warnings:") -or -not ($outlierMdContent -match ":warning:")) {
            throw "SelfTest Failed: Expected Markdown report to contain '### Gate Warnings:' section with :warning: emoji."
        }
        if (-not ($outlierMdContent -match "\|\s*Max Paired Render-p99 Delta\s*\|.*\|\s*WARN\s*\|")) {
            throw "SelfTest Failed: Expected Markdown Headline Summary table to mark Max Paired Render-p99 Delta as WARN."
        }

        $outlierJsonContent = [System.IO.File]::ReadAllText($outlierJsonPath, [System.Text.Encoding]::UTF8)
        $parsedOutlierJson = $outlierJsonContent | ConvertFrom-Json
        if ($null -eq $parsedOutlierJson.TelemetryGate.Warnings -or $parsedOutlierJson.TelemetryGate.Warnings.Count -ne 1) {
            throw "SelfTest Failed: Expected JSON output to contain TelemetryGate.Warnings array with 1 item."
        }

        # 18. Test Mean Paired Render-p99 Delta > 1.0ms (Gate FAILS)
        $highMeanMarkerCsv = Join-Path $tempDir "high_mean_markers.csv"
        [System.IO.File]::WriteAllText($highMeanMarkerCsv, "run_index,warmup,state,mode,anchor,capture_exclusion,lifecycle_policy,trigger_qpc,qpc_frequency,submit_result,nominal_duration_ms,interval_ms`n0,false,idle,full,bottom-center,on,attached-shown,20000000,10000000,not_submitted,0,5000", [System.Text.Encoding]::UTF8)

        $highMeanPmLines = [System.Collections.Generic.List[string]]::new()
        $highMeanPmLines.Add("CPUStartQPC,MsBetweenPresents,MsBetweenDisplayChange,DisplayedTime,PresentMode,SwapChainAddress")
        for ($q = 10000000L; $q -lt 20000000L; $q += 166666L) {
            $highMeanPmLines.Add("$q,16.66,16.66,16.66,Hardware: Independent Flip,0x1000")
        }
        for ($q = 20000000L; $q -le 30000000L; $q += 166666L) {
            $highMeanPmLines.Add("$q,18.00,18.00,18.00,Hardware: Independent Flip,0x1000")
        }
        $highMeanPmPath = Join-Path $tempDir "high_mean_pm.csv"
        [System.IO.File]::WriteAllLines($highMeanPmPath, $highMeanPmLines)

        $highMeanRes = Invoke-PresentMonAnalysis -PresentMonCsvPath $highMeanPmPath -MarkerCsvPath $highMeanMarkerCsv
        if ($highMeanRes.TelemetryGate.Passed) {
            throw "SelfTest Failed: Expected telemetry gate to FAIL when Mean paired render-p99 delta > 1.0 ms."
        }
        $hasMeanFailMsg = ($highMeanRes.TelemetryGate.Failures | Where-Object { $_ -match "Mean paired render-p99 delta" }).Count -gt 0
        if (-not $hasMeanFailMsg) {
            throw "SelfTest Failed: Expected failure message to mention 'Mean paired render-p99 delta', got: $($highMeanRes.TelemetryGate.Failures -join '; ')"
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
    $analysis = Invoke-PresentMonAnalysis `
        -PresentMonCsvPath $PresentMonCsv `
        -MarkerCsvPath $MarkerCsv `
        -SelectedSwapChain $SwapChainAddress `
        -OutputJsonPath $OutputJson `
        -OutputMarkdownPath $OutputMarkdown

    Write-Host "=== PresentMon Telemetry Analysis ===" -ForegroundColor Cyan
    Write-Host "Status:            $($analysis.TelemetryGate.Status)" -ForegroundColor $(if ($analysis.TelemetryGate.Passed) { "Green" } else { "Red" })
    Write-Host "Capture Exclusion: $($analysis.CaptureExclusion)"
    Write-Host "Lifecycle Policy:  $($analysis.LifecyclePolicy)"
    Write-Host "Mean Paired Render p99 Delta: $($analysis.HeadlineSummary.MeanPairedRenderP99DeltaMs) ms"
    Write-Host "Max Paired Render p99 Delta:  $($analysis.HeadlineSummary.MaxPairedRenderP99DeltaMs) ms"
    Write-Host "Mean Paired Dropped Delta:    $(if ($null -ne $analysis.HeadlineSummary.MeanPairedDroppedDelta) { $analysis.HeadlineSummary.MeanPairedDroppedDelta } else { 'N/A' })"
    Write-Host "Mean HW-Independent Ratio:    Pre: $([Math]::Round($analysis.HeadlineSummary.MeanPreHardwareIndepRatio * 100, 1))% -> Post: $([Math]::Round($analysis.HeadlineSummary.MeanPostHardwareIndepRatio * 100, 1))%"
    if ($null -ne $analysis.HeadlineSummary.MaxRecoveryLatencyMs) {
        Write-Host "Max Recovery Latency:         $($analysis.HeadlineSummary.MaxRecoveryLatencyMs) ms"
    }
    if ($analysis.HeadlineSummary.UnrecoveredMarkerCount -gt 0) {
        Write-Host "Unrecovered Markers:          $($analysis.HeadlineSummary.UnrecoveredMarkerCount)" -ForegroundColor Red
        if ($null -ne $analysis.HeadlineSummary.MaxRecoveryLowerBoundMs) {
            Write-Host "Max Recovery Lower Bound:     >= $($analysis.HeadlineSummary.MaxRecoveryLowerBoundMs) ms" -ForegroundColor Red
        }
    }
    if ($analysis.TelemetryGate.Failures -and $analysis.TelemetryGate.Failures.Count -gt 0) {
        Write-Host "Failures:" -ForegroundColor Red
        foreach ($fail in $analysis.TelemetryGate.Failures) {
            Write-Host "  - $fail" -ForegroundColor Red
        }
    }
    if ($analysis.TelemetryGate.Warnings -and $analysis.TelemetryGate.Warnings.Count -gt 0) {
        Write-Host "Warnings:" -ForegroundColor Yellow
        foreach ($warn in $analysis.TelemetryGate.Warnings) {
            Write-Host "  - $warn" -ForegroundColor Yellow
        }
    }
    Write-Host "Disclaimer: $($analysis.Disclaimer)" -ForegroundColor Gray

    return $analysis
}
