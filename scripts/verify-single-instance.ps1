#requires -Version 7
# 实测全局单例：起真实的多个进程，看第二次启动、TortoiseSVN 钩子调用时已有窗口和进程的反应。
# tests/single_instance.rs 只守装配顺序，两个进程之间的行为只能这样测。
#
# 单例按 identifier 认实例：必须换一个 identifier 构建，否则会撞上本机开着的安装版——
# 第二次启动唤起的是安装版的窗口，也会改写安装版的 session.json。构建步骤见 lib/jkk-probe.ps1 开头。
#
#   pwsh -NoProfile -File scripts/verify-single-instance.ps1 -Exe <src-tauri\target\release\jkk.exe>
#
# 会弹出 jkk 窗口、把它关到托盘再唤出来，跑完自己把进程结束掉。全部 PASS 时退出码 0。
param([Parameter(Mandatory)][string]$Exe)
$ErrorActionPreference = 'Stop'
. "$PSScriptRoot/lib/jkk-probe.ps1"
$Exe = Resolve-JkkExe $Exe

function Count-Instances { (Get-JkkInstances $Exe).Count }

if (Get-JkkInstances $Exe) { throw "已有同路径进程在跑，先关掉：$Exe" }

# 1. 第一个实例
$first = Start-Jkk $Exe
$hwnd = Wait-JkkMainWindow $first
if ($hwnd -eq [IntPtr]::Zero) {
  Stop-JkkInstances $Exe
  throw '第一个实例 30 秒内没出标题为 jkk… 的窗口'
}
Start-Sleep -Seconds 2
Add-Check '第一个实例起来了' ([JkkWin]::IsWindowVisible($hwnd)) "pid=$($first.Id)"

try {
  # 2. 关到托盘，再以 --tsvn-hook 调起：钩子必须自己处理完退出，不能把已有窗口唤出来
  Close-JkkToTray $hwnd
  Add-Check '点关闭后窗口藏到托盘、进程还在' ((-not [JkkWin]::IsWindowVisible($hwnd)) -and -not $first.HasExited)
  $hook = Start-Jkk $Exe @('--tsvn-hook', 'no_such_kind', '--repo', 'x')
  $exited = $hook.WaitForExit(10000)
  Start-Sleep -Seconds 1
  Add-Check '钩子调用自己退出' $exited "exit=$(if ($exited) { $hook.ExitCode } else { '超时' })"
  Add-Check '钩子调用没唤出已有窗口' (-not [JkkWin]::IsWindowVisible($hwnd))
  Add-Check '钩子调用后只剩一个实例' ((Count-Instances) -eq 1) "count=$(Count-Instances)"

  # 3. 藏着时再次启动：第二个进程退出，已有窗口被拿出来
  $second = Start-Jkk $Exe
  $exited = $second.WaitForExit(15000)
  Start-Sleep -Seconds 1
  Add-Check '第二次启动的进程退出' $exited "exit=$(if ($exited) { $second.ExitCode } else { '超时' })"
  Add-Check '藏起来的窗口被显示' ([JkkWin]::IsWindowVisible($hwnd))
  Add-Check '只剩一个实例' ((Count-Instances) -eq 1) "count=$(Count-Instances)"

  # 4. 最小化时再次启动：还原。这里可以直接 ShowWindow：最小化 tao 认得出来
  [void][JkkWin]::ShowWindow($hwnd, $SW_MINIMIZE)
  Start-Sleep -Milliseconds 500
  $third = Start-Jkk $Exe
  $exited = $third.WaitForExit(15000)
  Start-Sleep -Seconds 1
  Add-Check '最小化时再次启动的进程退出' $exited
  Add-Check '最小化的窗口被还原' (-not [JkkWin]::IsIconic($hwnd))
  # 前台只报告不判定：Windows 只许持有前台的进程把前台让出去。脚本从后台 shell 起第二个实例，
  # 它自己就没有前台，插件的 AllowSetForegroundWindow 让不出东西；跑的时候动了别的窗口也会抢走。
  # 用户从开始菜单 / 任务栏启动时启动者在前台，不受这个限制——要确认就手动点一次
  $fg = [JkkWin]::GetForegroundWindow()
  Add-Info ('窗口到了前台：{0}（fg={1} main={2}，不计入结果）' -f ($fg -eq $hwnd), $fg, $hwnd)
  Add-Check '仍只有一个实例' ((Count-Instances) -eq 1) "count=$(Count-Instances)"
}
finally {
  Stop-JkkInstances $Exe
}

Complete-Probe
