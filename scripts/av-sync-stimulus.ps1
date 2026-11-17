param([Parameter(Mandatory=$true)][string]$Wave)
$ErrorActionPreference='Stop'
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$form=[Windows.Forms.Form]::new()
$form.FormBorderStyle='None'
$form.Bounds=[Windows.Forms.Screen]::PrimaryScreen.Bounds
$form.BackColor=[Drawing.Color]::Black
$form.TopMost=$true
Add-Type @'
using System;
using System.IO;
using System.Runtime.InteropServices;
public sealed class StimulusAudio : IDisposable {
 [StructLayout(LayoutKind.Sequential, Pack=2)] struct Format { public ushort tag, channels; public uint rate, bytes; public ushort align, bits, extra; }
 [StructLayout(LayoutKind.Sequential)] struct Header { public IntPtr data; public uint length, recorded; public IntPtr user; public uint flags, loops; public IntPtr next, reserved; }
 [StructLayout(LayoutKind.Explicit, Size=12)] struct Position { [FieldOffset(0)] public uint type; [FieldOffset(4)] public uint value; }
 [DllImport("winmm.dll")] static extern uint waveOutOpen(out IntPtr handle,uint device,ref Format format,IntPtr callback,IntPtr instance,uint flags);
 [DllImport("winmm.dll")] static extern uint waveOutPrepareHeader(IntPtr h,IntPtr header,uint size);
 [DllImport("winmm.dll")] static extern uint waveOutWrite(IntPtr h,IntPtr header,uint size);
 [DllImport("winmm.dll")] static extern uint waveOutGetPosition(IntPtr h,ref Position position,uint size);
 [DllImport("winmm.dll")] static extern uint waveOutReset(IntPtr h);
 [DllImport("winmm.dll")] static extern uint waveOutUnprepareHeader(IntPtr h,IntPtr header,uint size);
 [DllImport("winmm.dll")] static extern uint waveOutClose(IntPtr h);
 IntPtr handle, data, header; uint size=(uint)Marshal.SizeOf<Header>();
 static void Check(uint code) { if(code!=0) throw new Exception("waveOut error "+code); }
 public StimulusAudio(string path) {
   byte[] wav=File.ReadAllBytes(path);
   Format f=new Format{tag=1,channels=1,rate=48000,bytes=96000,align=2,bits=16};
   Check(waveOutOpen(out handle,0xffffffff,ref f,IntPtr.Zero,IntPtr.Zero,0));
   data=Marshal.AllocHGlobal(wav.Length-44); Marshal.Copy(wav,44,data,wav.Length-44);
   header=Marshal.AllocHGlobal((int)size); Marshal.StructureToPtr(new Header{data=data,length=(uint)(wav.Length-44)},header,false);
   Check(waveOutPrepareHeader(handle,header,size));
 }
 public void Play() { Check(waveOutWrite(handle,header,size)); }
 public double Milliseconds { get { Position p=new Position{type=2}; Check(waveOutGetPosition(handle,ref p,12)); if(p.type!=2) throw new Exception("Sample position unavailable"); return p.value/48.0; } }
 public void Dispose(){ waveOutReset(handle); waveOutUnprepareHeader(handle,header,size); waveOutClose(handle); Marshal.FreeHGlobal(header); Marshal.FreeHGlobal(data); }
}
'@
$player=[StimulusAudio]::new($Wave)
$clock=[Diagnostics.Stopwatch]::new()
$timer=[Windows.Forms.Timer]::new()
$timer.Interval=5
$timer.Add_Tick({
    $ms=$player.Milliseconds
    $form.BackColor=if(($ms % 2000) -ge 500 -and ($ms % 2000) -lt 700){[Drawing.Color]::White}else{[Drawing.Color]::Black}
    if($ms -gt 22000){$form.Close()}
})
$form.Add_Shown({$clock.Start();$player.Play();$timer.Start()})
try{[Windows.Forms.Application]::Run($form)}finally{$timer.Dispose();$player.Dispose();$form.Dispose()}
