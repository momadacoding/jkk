#requires -Version 7
# 实测 jkk 被 Restart Manager 请求关闭时能不能正常退出——MSI 升级时安装程序就是这样关掉运行中的
# jkk 的，Windows 关机 / 注销走的也是同一个 WM_ENDSESSION。
#
# 背景：Tauri 2.11（tao 0.35）收到 WM_ENDSESSION 后在 tao 的 runner 里 panic
# （`cannot move state from Destroyed`），退出码 101。升到 Tauri 2.12（tao 0.37）修掉。
# 以后再升 Tauri / tao，用这个脚本复验。
#
# 不装 MSI：直接调 Restart Manager API，做与安装程序相同的事——登记 exe、列出占用者、
# RmShutdown（不强杀）。窗口可见、关到托盘两种状态各测一次。
#
# 用隔离 identifier 构建的 exe，构建步骤见 lib/jkk-probe.ps1 开头：
#
#   pwsh -NoProfile -File scripts/verify-restart-manager-exit.ps1 -Exe <src-tauri\target\release\jkk.exe>
#
# 全部 PASS 时退出码 0。
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/lib/jkk-probe.ps1"
$Exe = Resolve-JkkExe $Exe

if (-not ('JkkRM' -as [type])) {
  Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class JkkRM {
  [StructLayout(LayoutKind.Sequential)]
  public struct RM_UNIQUE_PROCESS { public int dwProcessId; public System.Runtime.InteropServices.ComTypes.FILETIME ProcessStartTime; }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  public struct RM_PROCESS_INFO {
    public RM_UNIQUE_PROCESS Process;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string strAppName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 64)] public string strServiceShortName;
    public int ApplicationType; public uint AppStatus; public uint TSSessionId;
    [MarshalAs(UnmanagedType.Bool)] public bool bRestartable;
  }
  [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)] public static extern int RmStartSession(out uint h, int flags, StringBuilder key);
  [DllImport("rstrtmgr.dll")] public static extern int RmEndSession(uint h);
  [DllImport("rstrtmgr.dll", CharSet = CharSet.Unicode)] public static extern int RmRegisterResources(uint h, uint nFiles, string[] files, uint nApps, IntPtr apps, uint nSvc, string[] svcs);
  [DllImport("rstrtmgr.dll")] public static extern int RmGetList(uint h, out uint needed, ref uint n, [In, Out] RM_PROCESS_INFO[] info, ref uint reasons);
  [DllImport("rstrtmgr.dll")] public static extern int RmShutdown(uint h, uint flags, IntPtr cb);
}
'@
}

function Probe([string]$label, [bool]$hideToTray) {
  if (Get-JkkInstances $Exe) { throw "已有同路径进程在跑：$Exe" }
  $errLog = Join-Path ([IO.Path]::GetTempPath()) "jkk-rm-probe-$PID.log"
  $env:RUST_BACKTRACE = '1'
  $p = Start-Jkk $Exe -StderrLog $errLog
  try {
    $hwnd = Wait-JkkMainWindow $p
    if ($hwnd -eq [IntPtr]::Zero) { throw "$label：30 秒内没出标题为 jkk… 的窗口" }
    Start-Sleep -Seconds 2
    if ($hideToTray) {
      Close-JkkToTray $hwnd
      Add-Check "$label：关到托盘、进程还在" ((-not [JkkWin]::IsWindowVisible($hwnd)) -and -not $p.HasExited)
    }

    $h = 0; $key = [Text.StringBuilder]::new(64)
    $rc = [JkkRM]::RmStartSession([ref]$h, 0, $key); if ($rc) { throw "RmStartSession=$rc" }
    try {
      $rc = [JkkRM]::RmRegisterResources($h, 1, @($Exe), 0, [IntPtr]::Zero, 0, $null)
      if ($rc) { throw "RmRegisterResources=$rc" }
      $needed = 0; $n = 8; $reasons = 0; $info = [JkkRM+RM_PROCESS_INFO[]]::new(8)
      $rc = [JkkRM]::RmGetList($h, [ref]$needed, [ref]$n, $info, [ref]$reasons)
      $mine = $rc -eq 0 -and $n -ge 1 -and ($info[0..($n - 1)] | Where-Object { $_.Process.dwProcessId -eq $p.Id })
      Add-Check "$label：Restart Manager 认出 jkk 占着 exe" ([bool]$mine) "rc=$rc n=$n"
      $rc = [JkkRM]::RmShutdown($h, 0, [IntPtr]::Zero)   # 0 = 礼貌请求，不强杀
      Add-Check "$label：RmShutdown 成功" ($rc -eq 0) "rc=$rc"
    }
    finally { [void][JkkRM]::RmEndSession($h) }

    $exited = $p.WaitForExit(15000)
    Add-Check "$label：进程退出" $exited
    if ($exited) { Add-Check "$label：正常退出（不是 panic 的 101）" ($p.ExitCode -eq 0) "exit=$($p.ExitCode)" }
    $panic = Get-Content $errLog -ErrorAction SilentlyContinue | Select-String -Pattern 'panicked at' | Select-Object -First 1
    Add-Check "$label：stderr 里没有 panic" (-not $panic) "$panic"
  }
  finally {
    Stop-JkkInstances $Exe
    Remove-Item $errLog -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
  }
}

# 一轮抛错不能让另一轮不跑、结果表不打印：记成 FAIL 接着走
foreach ($case in @(@('窗口可见', $false), @('关到托盘', $true))) {
  try { Probe $case[0] $case[1] }
  catch { Add-Check "$($case[0])：探测本身出错" $false "$_" }
}
Complete-Probe
