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
# 用隔离的 identifier 构建，别碰本机的安装版：
#   1. 在仓库根写一个 si.conf.json：{ "identifier": "com.momadacoding.jkk.sitest" }
#   2. pnpm tauri build --no-bundle --config <si.conf.json 的绝对路径>
#   3. pwsh -NoProfile -File scripts/verify-restart-manager-exit.ps1 -Exe <src-tauri\target\release\jkk.exe 的绝对路径>
#   4. 删掉 %APPDATA% 与 %LOCALAPPDATA% 下的 com.momadacoding.jkk.sitest
#
# 全部 PASS 时退出码 0。
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "找不到 exe：$Exe" }
$Exe = (Resolve-Path -LiteralPath $Exe).Path

Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class RM {
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
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
'@

$results = [System.Collections.Generic.List[string]]::new()
function Check([string]$name, [bool]$ok, [string]$detail = '') {
  $results.Add(('{0}  {1}  {2}' -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $detail))
}
function Instances { @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }

function Probe([string]$label, [bool]$hideToTray) {
  if (Instances) { throw "已有同路径进程在跑：$Exe" }
  $errLog = Join-Path ([IO.Path]::GetTempPath()) "jkk-rm-probe-$PID.log"
  $env:RUST_BACKTRACE = '1'
  $p = Start-Process -FilePath $Exe -PassThru -RedirectStandardError $errLog
  try {
    # 认标题：单例插件的隐藏消息窗口也是顶层窗口，MainWindowHandle 可能先落到它上面
    $hwnd = [IntPtr]::Zero
    for ($i = 0; $i -lt 60 -and $hwnd -eq [IntPtr]::Zero; $i++) {
      Start-Sleep -Milliseconds 500; $p.Refresh()
      if ($p.MainWindowTitle -like 'jkk*') { $hwnd = $p.MainWindowHandle }
    }
    if ($hwnd -eq [IntPtr]::Zero) { throw "$label：30 秒内没出标题为 jkk… 的窗口" }
    Start-Sleep -Seconds 2
    if ($hideToTray) {
      [void][RM]::PostMessageW($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)  # WM_CLOSE = 关到托盘
      Start-Sleep -Seconds 1
      Check "$label：关到托盘、进程还在" ((-not [RM]::IsWindowVisible($hwnd)) -and -not $p.HasExited)
    }

    $h = 0; $key = [Text.StringBuilder]::new(64)
    $rc = [RM]::RmStartSession([ref]$h, 0, $key); if ($rc) { throw "RmStartSession=$rc" }
    try {
      $rc = [RM]::RmRegisterResources($h, 1, @($Exe), 0, [IntPtr]::Zero, 0, $null)
      if ($rc) { throw "RmRegisterResources=$rc" }
      $needed = 0; $n = 8; $reasons = 0; $info = [RM+RM_PROCESS_INFO[]]::new(8)
      $rc = [RM]::RmGetList($h, [ref]$needed, [ref]$n, $info, [ref]$reasons)
      $mine = $rc -eq 0 -and $n -ge 1 -and ($info[0..($n - 1)] | Where-Object { $_.Process.dwProcessId -eq $p.Id })
      Check "$label：Restart Manager 认出 jkk 占着 exe" ([bool]$mine) "rc=$rc n=$n"
      $rc = [RM]::RmShutdown($h, 0, [IntPtr]::Zero)   # 0 = 礼貌请求，不强杀
      Check "$label：RmShutdown 成功" ($rc -eq 0) "rc=$rc"
    }
    finally { [void][RM]::RmEndSession($h) }

    $exited = $p.WaitForExit(15000)
    Check "$label：进程退出" $exited
    if ($exited) { Check "$label：正常退出（不是 panic 的 101）" ($p.ExitCode -eq 0) "exit=$($p.ExitCode)" }
    $panic = Get-Content $errLog -ErrorAction SilentlyContinue | Select-String -Pattern 'panicked at' | Select-Object -First 1
    Check "$label：stderr 里没有 panic" (-not $panic) "$panic"
  }
  finally {
    Instances | Stop-Process -Force -ErrorAction SilentlyContinue
    Remove-Item $errLog -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
  }
}

Probe '窗口可见' $false
Probe '关到托盘' $true
$results
if ($results -match '^FAIL') { exit 1 }
