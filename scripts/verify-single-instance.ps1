#requires -Version 7
# 实测全局单例：起真实的多个进程，看第二次启动、TortoiseSVN 钩子调用时已有窗口和进程的反应。
# tests/single_instance.rs 只守装配顺序，两个进程之间的行为只能这样测。
#
# 单例只在 release 构建生效（见 src-tauri/src/lib.rs），而且按 identifier 认实例。
# 必须换一个 identifier 构建，否则会撞上本机开着的安装版：第二次启动唤起的是安装版的窗口，
# 也会改写安装版的 session.json。
#
#   1. 在仓库根目录写一个 si.conf.json：{ "identifier": "com.momadacoding.jkk.sitest" }
#   2. pnpm tauri build --no-bundle --config <si.conf.json 的绝对路径>
#   3. pwsh -NoProfile -File scripts/verify-single-instance.ps1 -Exe <src-tauri\target\release\jkk.exe 的绝对路径>
#   4. 删掉 %APPDATA% 与 %LOCALAPPDATA% 下的 com.momadacoding.jkk.sitest
#
# 第 3 步会弹出 jkk 窗口、把它关到托盘再唤出来，跑完自己把进程结束掉。全部 PASS 时退出码 0。
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "找不到 exe：$Exe" }
$Exe = (Resolve-Path -LiteralPath $Exe).Path

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class W {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
}
'@

$results = [System.Collections.Generic.List[string]]::new()
function Check([string]$name, [bool]$ok, [string]$detail) {
  $results.Add(('{0}  {1}  {2}' -f ($(if ($ok) { 'PASS' } else { 'FAIL' })), $name, $detail))
}
function Instances { @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe }) }
function Launch([string[]]$argList) {
  if ($argList) { Start-Process -FilePath $Exe -ArgumentList $argList -PassThru }
  else { Start-Process -FilePath $Exe -PassThru }
}

if (Instances) { throw "已有同路径进程在跑，先关掉：$Exe" }

# 1. 第一个实例
$first = Launch @()
$hwnd = [IntPtr]::Zero
# 认标题：单例插件自己也建了一个顶层的消息窗口（标题是 `<identifier>-siw`），
# MainWindowHandle 可能先落到它上面，往它发 WM_CLOSE 会把插件的接收端销毁
for ($i = 0; $i -lt 60 -and $hwnd -eq [IntPtr]::Zero; $i++) {
  Start-Sleep -Milliseconds 500
  $first.Refresh()
  if ($first.MainWindowTitle -like 'jkk*') { $hwnd = $first.MainWindowHandle }
}
if ($hwnd -eq [IntPtr]::Zero) {
  Stop-Process -Id $first.Id -Force -ErrorAction SilentlyContinue
  throw '第一个实例 30 秒内没出标题为 jkk… 的窗口'
}
Start-Sleep -Seconds 2
Check '第一个实例起来了' ([W]::IsWindowVisible($hwnd)) "pid=$($first.Id)"

try {
  # 2. 点关闭 = 关到托盘（默认设置）。走应用自己的 hide：直接 ShowWindow(SW_HIDE) 会绕过 tao，
  #    tao 仍记着「可见」，之后 show() 什么也不做，测出来的是测试手法的假失败
  [void][W]::PostMessageW($hwnd, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)   # WM_CLOSE
  Start-Sleep -Seconds 1
  Check '点关闭后窗口藏到托盘、进程还在' ((-not [W]::IsWindowVisible($hwnd)) -and -not $first.HasExited) ''
  # 以 --tsvn-hook 调起：必须自己处理完退出，不能把已有窗口唤出来
  $hook = Launch @('--tsvn-hook', 'no_such_kind', '--repo', 'x')
  $exited = $hook.WaitForExit(10000)
  Start-Sleep -Seconds 1
  Check '钩子调用自己退出' $exited "exit=$(if ($exited) { $hook.ExitCode } else { '超时' })"
  Check '钩子调用没唤出已有窗口' (-not [W]::IsWindowVisible($hwnd)) ''
  Check '钩子调用后只剩一个实例' ((Instances).Count -eq 1) "count=$((Instances).Count)"

  # 3. 藏着时再次启动：第二个进程退出，已有窗口被拿出来、到前台
  $second = Launch @()
  $exited = $second.WaitForExit(15000)
  Start-Sleep -Seconds 1
  Check '第二次启动的进程退出' $exited "exit=$(if ($exited) { $second.ExitCode } else { '超时' })"
  Check '藏起来的窗口被显示' ([W]::IsWindowVisible($hwnd)) ''
  Check '只剩一个实例' ((Instances).Count -eq 1) "count=$((Instances).Count)"

  # 4. 最小化时再次启动：还原并到前台。这里可以直接 ShowWindow：最小化 tao 认得出来
  [void][W]::ShowWindow($hwnd, 6)   # SW_MINIMIZE
  Start-Sleep -Milliseconds 500
  $third = Launch @()
  $exited = $third.WaitForExit(15000)
  Start-Sleep -Seconds 1
  Check '最小化时再次启动的进程退出' $exited ''
  Check '最小化的窗口被还原' (-not [W]::IsIconic($hwnd)) ''
  # 前台只报告不判定：Windows 只许持有前台的进程把前台让出去。脚本从后台 shell 起第二个实例，
  # 它自己就没有前台，插件的 AllowSetForegroundWindow 让不出东西；跑的时候动了别的窗口也会抢走。
  # 用户从开始菜单 / 任务栏启动时启动者在前台，不受这个限制——要确认就手动点一次
  $fg = [W]::GetForegroundWindow()
  $results.Add(('INFO  窗口到了前台：{0}（fg={1} main={2}，不计入结果）' -f ($fg -eq $hwnd), $fg, $hwnd))
  Check '仍只有一个实例' ((Instances).Count -eq 1) "count=$((Instances).Count)"
}
finally {
  # 列出来到结束之间进程可能已经自己退了；这里抛错会把下面的结果表吞掉
  Instances | Stop-Process -Force -ErrorAction SilentlyContinue
}

$results
if ($results -match '^FAIL') { exit 1 }
