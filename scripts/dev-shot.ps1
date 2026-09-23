# Launch an AddonForge exe, wait, screenshot its window, close it.
#   powershell -File scripts\dev-shot.ps1 -Exe <path> -Out <png> [-Tab browse|settings] [-Wait 9]
#   Extra env for the launched app can be set by the caller (ADDONFORGE_UPDATE_URL, ADDONFORGE_JUST_UPDATED).
param([string]$Exe, [string]$Out, [string]$Tab = "", [int]$Wait = 9)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public class DevShot {
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [StructLayout(LayoutKind.Sequential)] public struct R { public int L, T, Rt, B; }
}
"@
[DevShot]::SetProcessDPIAware() | Out-Null
if ($Tab) { $env:ADDONFORGE_TAB = $Tab }
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds $Wait
$p.Refresh(); $h = $p.MainWindowHandle
[DevShot]::SetForegroundWindow($h) | Out-Null; Start-Sleep -Milliseconds 800
$r = New-Object DevShot+R; [DevShot]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Rt - $r.L; $ht = $r.B - $r.T
$bmp = New-Object System.Drawing.Bitmap $w, $ht
$g = [System.Drawing.Graphics]::FromImage($bmp)
$dc = $g.GetHdc(); [DevShot]::PrintWindow($h, $dc, 2) | Out-Null; $g.ReleaseHdc($dc)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Stop-Process -Id $p.Id -Force
if ($Tab) { Remove-Item Env:ADDONFORGE_TAB }
"captured ${w}x${ht} -> $Out"
