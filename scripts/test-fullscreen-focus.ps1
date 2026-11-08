param([string]$Output = 'target/fullscreen-focus-check.png')
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class FullscreenFocusCheck {
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h,out Rect r);
 [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h,ref Point p);
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr h);
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
 [DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h,int index);
}
'@
$silkHandle = (Get-Process silk).MainWindowHandle
$silkScreen = [System.Windows.Forms.Screen]::FromHandle($silkHandle)
$otherScreen = [System.Windows.Forms.Screen]::AllScreens | Where-Object DeviceName -ne $silkScreen.DeviceName | Select-Object -First 1
if (-not $otherScreen) { throw 'This regression requires two displays and Silk already in fullscreen.' }
function Read-FullscreenGeometry {
  $outer = New-Object FullscreenFocusCheck+Rect
  $client = New-Object FullscreenFocusCheck+Rect
  $origin = New-Object FullscreenFocusCheck+Point
  [FullscreenFocusCheck]::GetWindowRect($silkHandle,[ref]$outer) | Out-Null
  [FullscreenFocusCheck]::GetClientRect($silkHandle,[ref]$client) | Out-Null
  [FullscreenFocusCheck]::ClientToScreen($silkHandle,[ref]$origin) | Out-Null
  [pscustomobject]@{Outer=$outer;Client=$client;Origin=$origin;Maximized=[FullscreenFocusCheck]::IsZoomed($silkHandle);Style=[FullscreenFocusCheck]::GetWindowLongPtrW($silkHandle,-16).ToInt64()}
}
function Assert-FullscreenGeometry($geometry) {
  $bounds=$silkScreen.Bounds
  if ($geometry.Outer.Left -ne $bounds.Left -or $geometry.Outer.Top -ne $bounds.Top -or $geometry.Outer.Right -ne $bounds.Right -or $geometry.Outer.Bottom -ne $bounds.Bottom) { throw 'Fullscreen outer bounds do not match the monitor.' }
  if ($geometry.Origin.X -ne $bounds.Left -or $geometry.Origin.Y -ne $bounds.Top -or $geometry.Client.Right -ne $bounds.Width -or $geometry.Client.Bottom -ne $bounds.Height) { throw 'Fullscreen client area is inset or uses the taskbar work area.' }
  if ($geometry.Maximized -or ($geometry.Style -band 0x00C40000) -ne 0) { throw 'Fullscreen retains a maximized/caption/resizing frame.' }
}
$probe = New-Object System.Windows.Forms.Form
$probe.Text = 'Silk fullscreen focus check'
$probe.StartPosition = 'Manual'
$probe.Location = New-Object System.Drawing.Point(($otherScreen.Bounds.Left+100),($otherScreen.Bounds.Top+100))
$probe.Size = New-Object System.Drawing.Size(340,120)
$probe.ShowInTaskbar = $false
$probe.TopMost = $true
$originalCursor=[System.Windows.Forms.Cursor]::Position
try {
  [FullscreenFocusCheck]::SetForegroundWindow($silkHandle) | Out-Null
  Start-Sleep -Milliseconds 500
  $focused=Read-FullscreenGeometry
  Assert-FullscreenGeometry $focused
  $probe.Show()
  $probe.Activate()
  [FullscreenFocusCheck]::SetForegroundWindow($probe.Handle) | Out-Null
  [System.Windows.Forms.Application]::DoEvents()
  $probeRect=New-Object FullscreenFocusCheck+Rect
  [FullscreenFocusCheck]::GetWindowRect($probe.Handle,[ref]$probeRect) | Out-Null
  [FullscreenFocusCheck]::SetCursorPos(($probeRect.Left+100),($probeRect.Top+70)) | Out-Null
  [FullscreenFocusCheck]::mouse_event(2,0,0,0,[UIntPtr]::Zero)
  [FullscreenFocusCheck]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
  Start-Sleep -Milliseconds 800
  [System.Windows.Forms.Application]::DoEvents()
  if ([FullscreenFocusCheck]::GetForegroundWindow() -ne $probe.Handle) { throw 'Focus did not move to the probe on the other monitor.' }
  $unfocused=Read-FullscreenGeometry
  Assert-FullscreenGeometry $unfocused
  $bounds=$silkScreen.Bounds
  $bitmap=New-Object System.Drawing.Bitmap($bounds.Width,$bounds.Height)
  $graphics=[System.Drawing.Graphics]::FromImage($bitmap)
  try {
    $graphics.CopyFromScreen($bounds.Left,$bounds.Top,0,0,$bitmap.Size)
    $bitmap.Save([IO.Path]::GetFullPath($Output),[System.Drawing.Imaging.ImageFormat]::Png)
  } finally { $graphics.Dispose(); $bitmap.Dispose() }
  [FullscreenFocusCheck]::SetForegroundWindow($silkHandle) | Out-Null
  Start-Sleep -Milliseconds 300
  Assert-FullscreenGeometry (Read-FullscreenGeometry)
  [pscustomobject]@{Passed=$true;Monitor=$silkScreen.DeviceName;Focused=$focused;Unfocused=$unfocused} | ConvertTo-Json -Depth 4
} finally {
  $probe.Close(); $probe.Dispose()
  [System.Windows.Forms.Cursor]::Position=$originalCursor
}
