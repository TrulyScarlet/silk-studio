# Opt-in Windows desktop test: native taskbar pixels, not just HICON handles.
# Run from the repo root: pwsh -File scripts/test-taskbar-themes.ps1
# Uses a separate app identity, no capture, and never edits installed shortcuts.
# Explorer's legacy taskbar UIA proxies require the desktop .NET Framework.
if ($PSVersionTable.PSEdition -eq 'Core') {
    & "$env:WINDIR/System32/WindowsPowerShell/v1.0/powershell.exe" -NoProfile -MTA -ExecutionPolicy Bypass -File $PSCommandPath
    exit $LASTEXITCODE
}
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
Push-Location $repo
$process = $null
try {
    if (Get-Process taskbar_icons -ErrorAction SilentlyContinue) {
        throw 'Close the existing taskbar_icons test window before running this test.'
    }
    cargo build -p silk --example taskbar_icons
    if ($LASTEXITCODE -ne 0) { throw 'Could not build the native test harness.' }
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    Add-Type -AssemblyName Accessibility
    Add-Type -ReferencedAssemblies Accessibility @'
using System;
using System.Runtime.InteropServices;
using Accessibility;
public static class SilkTaskbarAccessibility {
    public class Entry { public string Name; public int X, Y, Width, Height; }
    [DllImport("oleacc.dll")]
    static extern int AccessibleObjectFromWindow(IntPtr hwnd, uint id, ref Guid iid,
        [MarshalAs(UnmanagedType.Interface)] out IAccessible accessible);
    public static Entry[] Read(IntPtr hwnd) {
        var iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
        IAccessible accessible;
        Marshal.ThrowExceptionForHR(AccessibleObjectFromWindow(hwnd, 0xFFFFFFFC, ref iid, out accessible));
        try {
            var entries = new Entry[accessible.accChildCount];
            for (int i = 1; i <= entries.Length; i++) {
                try {
                    var entry = new Entry { Name = accessible.get_accName(i) };
                    accessible.accLocation(out entry.X, out entry.Y, out entry.Width, out entry.Height, i);
                    entries[i - 1] = entry;
                } catch (COMException) { }
            }
            return entries;
        } finally { Marshal.ReleaseComObject(accessible); }
    }
}
'@
    $output = Join-Path $repo 'target/taskbar-theme-test'
    New-Item -ItemType Directory -Force -Path $output | Out-Null
    $stdout = Join-Path $output 'stdout.txt'
    $stderr = Join-Path $output 'stderr.txt'
    $process = Start-Process -FilePath (Join-Path $repo 'target/debug/examples/taskbar_icons.exe') `
        -ArgumentList '--stress' -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Retain the native process handle so Windows PowerShell can read ExitCode
    # even when the harness exits before the final WaitForExit call.
    $null = $process.Handle
    $results = @()
    $deadline = (Get-Date).AddSeconds(110)
    while ($results.Count -lt 24 -and (Get-Date) -lt $deadline) {
        $lines = @(Get-Content $stdout | Where-Object { $_ -like 'CHECK *' })
        if ($lines.Count -le $results.Count) {
            Start-Sleep -Milliseconds 150
            continue
        }
        $step = $results.Count
        $expected = ($lines[$step] -split ' ')[2].ToLowerInvariant()
        Start-Sleep -Milliseconds 750
        $root = [System.Windows.Automation.AutomationElement]::RootElement
        $bar = $root.FindFirst([System.Windows.Automation.TreeScope]::Children,
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ClassNameProperty, 'Shell_TrayWnd'))
        if (-not $bar) { throw 'An interactive Windows Explorer taskbar is required.' }
        $taskList = $bar.FindFirst([System.Windows.Automation.TreeScope]::Descendants,
            [System.Windows.Automation.PropertyCondition]::new(
                [System.Windows.Automation.AutomationElement]::ClassNameProperty, 'MSTaskListWClass'))
        if (-not $taskList) { throw 'This test requires the Windows 10 taskbar accessibility provider.' }
        $settleDeadline = (Get-Date).AddSeconds(2)
        do {
            $elements = [SilkTaskbarAccessibility]::Read([IntPtr]$taskList.Current.NativeWindowHandle)
            # MSAA exposes both a group and its single member at the same bounds.
            $buttons = @($elements | Where-Object {
                $_ -and $_.Name -like 'Silk icon test*running window*' -and $_.Width -gt 0
            } | Group-Object X,Y,Width,Height | ForEach-Object { $_.Group[0] })
            if ($buttons.Count -ne 1) { Start-Sleep -Milliseconds 100 }
        } while ($buttons.Count -ne 1 -and (Get-Date) -lt $settleDeadline)
        if ($buttons.Count -ne 1) {
            $observed = ($elements | ForEach-Object { $_.Name } | Where-Object { $_ -like '*Silk*' }) -join ', '
            throw "Step ${step}: expected one test taskbar button, found $($buttons.Count). Observed: $observed"
        }
        $rect = $buttons[0]
        $bitmap = [System.Drawing.Bitmap]::new([int]$rect.Width, [int]$rect.Height)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $graphics.CopyFromScreen([int]$rect.X, [int]$rect.Y, 0, 0, $bitmap.Size)
            $bitmap.Save((Join-Path $output "step-$step-$expected.png"))
            $counts = @{studio=0; classic=0; ember=0; vamp=0}
            for ($y = [int]($bitmap.Height * .18); $y -lt $bitmap.Height * .82; $y++) {
                for ($x = [int]($bitmap.Width * .25); $x -lt $bitmap.Width * .75; $x++) {
                    $p = $bitmap.GetPixel($x, $y)
                    if ($p.R -gt 180 -and $p.G -gt 180 -and $p.R-$p.B -gt 35 -and $p.G-$p.B -gt 35) {
                        $counts.classic++
                    } elseif ($p.R -gt 150 -and $p.R-$p.G -gt 45 -and $p.G-$p.B -gt 35) {
                        $counts.ember++
                    } elseif ($p.R -gt 150 -and $p.R-$p.G -gt 45 -and [Math]::Abs([int]$p.G-$p.B) -lt 30) {
                        $counts.vamp++
                    } elseif ($p.R -gt 160 -and [Math]::Abs([int]$p.R-$p.G) -lt 12 -and [Math]::Abs([int]$p.G-$p.B) -lt 12) {
                        $counts.studio++
                    }
                }
            }
            $detected = ($counts.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1).Key
            $pass = $detected -eq $expected -and $counts[$expected] -gt 10
            $results += [PSCustomObject]@{Step=$step; Expected=$expected; Detected=$detected; Pass=$pass; Pixels=$counts}
            $results | ConvertTo-Json -Depth 4 | Set-Content (Join-Path $output 'results.json')
            Write-Output "Step ${step}: $expected -> $detected ($pass)"
            if (-not $pass) { throw "Taskbar color mismatch at step $step; see $output" }
        } finally {
            $graphics.Dispose()
            $bitmap.Dispose()
        }
    }
    if ($results.Count -ne 24) { throw 'Timed out before all native theme checks completed.' }
    if (-not $process.WaitForExit(12000)) { throw 'The taskbar test did not exit.' }
    if ($process.ExitCode -ne 0) { throw "Native harness exited with $($process.ExitCode)." }
    Write-Output "PASS: 24 taskbar transitions, including rapid bursts. Evidence: $output"
} finally {
    if ($process -and -not $process.HasExited) {
        $null = $process.CloseMainWindow()
        if (-not $process.WaitForExit(3000)) { $process.Kill() }
    }
    Pop-Location
}
