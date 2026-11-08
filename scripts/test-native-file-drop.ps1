param([string]$Output = 'target/native-file-drop.json')
# Open Silk's Share dialog with a ready file before running. This sends a real
# mouse drag to a local test window, never to Discord or any external service.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms,System.Drawing,UIAutomationClient,UIAutomationTypes
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class SilkFileDropTest {
 [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
 [DllImport("user32.dll")] public static extern bool GetCursorPos(out Point point);
 [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point p);
 [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h,out uint id);
 [StructLayout(LayoutKind.Sequential)] public struct Point { public int X,Y; }
 [DllImport("user32.dll")] public static extern bool SetCursorPos(int x,int y);
 [DllImport("user32.dll")] public static extern void mouse_event(uint flags,uint x,uint y,uint data,UIntPtr extra);
 [DllImport("user32.dll",SetLastError=true)] public static extern uint SendInput(uint count,INPUT[] inputs,int size);
 [StructLayout(LayoutKind.Sequential)] public struct INPUT { public uint type; public MOUSEINPUT mouse; }
 [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT { public int dx,dy; public uint data,flags,time; public UIntPtr extra; }
 public static void Button(uint flags) { var input=new INPUT(); input.mouse.flags=flags; var sent=SendInput(1,new[]{input},Marshal.SizeOf(typeof(INPUT))); if(sent!=1)throw new Exception("SendInput failed: "+Marshal.GetLastWin32Error()); }
 [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h,out Rect r);
 [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left,Top,Right,Bottom; }
}
'@
$silkProcess = Get-Process silk
$condition = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ProcessIdProperty,[int]$silkProcess.Id), [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'Silk'))
$root = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Children,$condition)
if (-not $root) { throw 'Silk main window unavailable.' }
$handle = [IntPtr]$root.Current.NativeWindowHandle
$label = $root.FindFirst([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,'Drag & drop'))
if (-not $label) { throw 'Open a ready Share clip dialog first.' }
$source = $label.Current.BoundingRectangle
$screen = [System.Windows.Forms.Screen]::FromHandle($handle)
$other = [System.Windows.Forms.Screen]::AllScreens | Where-Object DeviceName -ne $screen.DeviceName | Select-Object -First 1
if (-not $other) { throw 'This test needs a second monitor for the receiving window.' }
$probe = New-Object System.Windows.Forms.Form
$probe.Text = 'Silk local file-drop check'
$probe.StartPosition = 'Manual'
$probe.Location = New-Object System.Drawing.Point(($other.WorkingArea.Left+120),($other.WorkingArea.Top+120))
$probe.Size = New-Object System.Drawing.Size(420,260)
$probe.TopMost = $true
$probe.ShowInTaskbar = $false
$probe.AllowDrop = $true
$script:received = $null
$script:formats = @()
$probe.Add_DragEnter({
  param($sender,$event)
  $script:formats = $event.Data.GetFormats()
  if ($event.Data.GetDataPresent([System.Windows.Forms.DataFormats]::FileDrop)) { $event.Effect = [System.Windows.Forms.DragDropEffects]::Copy }
})
$probe.Add_DragDrop({
  param($sender,$event)
  $paths = $event.Data.GetData([System.Windows.Forms.DataFormats]::FileDrop)
  $script:received = @($paths | ForEach-Object {
    $file=Get-Item -LiteralPath $_
    [pscustomobject]@{Path=$file.FullName;Bytes=$file.Length;Exists=$file.Exists;Effect=$event.Effect.ToString()}
  })
})
$cursor=[System.Windows.Forms.Cursor]::Position
try {
  $probe.Show(); [System.Windows.Forms.Application]::DoEvents()
  $rect=New-Object SilkFileDropTest+Rect
  [SilkFileDropTest]::GetWindowRect($probe.Handle,[ref]$rect) | Out-Null
  $fromX=[int]($source.Left+$source.Width/2); $fromY=[int]($source.Top+$source.Height/2)
  $toX=[int](($rect.Left+$rect.Right)/2); $toY=[int](($rect.Top+$rect.Bottom)/2)
  Write-Output "Drag source: $fromX,$fromY; receiver: $toX,$toY"
  [SilkFileDropTest]::SetForegroundWindow($handle) | Out-Null
  [SilkFileDropTest]::SetCursorPos($fromX,$fromY) | Out-Null
  Start-Sleep -Milliseconds 300
  $point=New-Object SilkFileDropTest+Point; $point.X=$fromX; $point.Y=$fromY
  $targetProcess=[uint32]0
  [SilkFileDropTest]::GetWindowThreadProcessId([SilkFileDropTest]::WindowFromPoint($point),[ref]$targetProcess) | Out-Null
  $targetOwner = Get-CimInstance Win32_Process -Filter "ProcessId = $targetProcess"
  if ($targetProcess -ne $silkProcess.Id -and $targetOwner.ParentProcessId -ne $silkProcess.Id) { throw "Drag source is occluded by process $targetProcess" }
  [SilkFileDropTest]::Button(2)
  Start-Sleep -Milliseconds 100
  $actualCursor=New-Object SilkFileDropTest+Point; [SilkFileDropTest]::GetCursorPos([ref]$actualCursor) | Out-Null
  Write-Output "Foreground $([SilkFileDropTest]::GetForegroundWindow()), Silk $handle, left button $([SilkFileDropTest]::GetAsyncKeyState(1)), cursor $($actualCursor.X),$($actualCursor.Y)"
  [SilkFileDropTest]::SetCursorPos(($fromX+16),$fromY) | Out-Null
  Start-Sleep -Milliseconds 400
  for($step=1;$step -le 40;$step++) {
    [SilkFileDropTest]::SetCursorPos([int]($fromX+($toX-$fromX)*$step/40),[int]($fromY+($toY-$fromY)*$step/40)) | Out-Null
    [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 20
  }
  [SilkFileDropTest]::Button(4)
  for($step=0;$step -lt 30 -and -not $script:received;$step++) { [System.Windows.Forms.Application]::DoEvents(); Start-Sleep -Milliseconds 100 }
  if (-not $script:received) { throw "The receiving window did not get a Windows file drop. Formats: $($script:formats -join ', ')" }
  $script:received | ConvertTo-Json | Set-Content -LiteralPath $Output
  $script:received | ConvertTo-Json
} finally {
  [SilkFileDropTest]::mouse_event(4,0,0,0,[UIntPtr]::Zero)
  $probe.Close(); $probe.Dispose()
  [SilkFileDropTest]::SetCursorPos($cursor.X,$cursor.Y) | Out-Null
}
