#requires -Version 7
# 起真实 jkk 进程做实测的脚本共用的部分：verify-single-instance.ps1、verify-restart-manager-exit.ps1
# 都 dot-source 它（`. "$PSScriptRoot/lib/jkk-probe.ps1"`）。单独运行没有意义。
#
# 被测的 exe 必须用隔离的 identifier 构建，别碰本机的安装版：
#   1. 在 %TEMP% 下写一个 si.conf.json：{ "identifier": "com.momadacoding.jkk.sitest" }
#      （别放进仓库：AGENTS.md 不许在根目录加文件，git add -A 也会把它带进提交）
#   2. pnpm tauri build --no-bundle --config <si.conf.json 的绝对路径>
#      （tauri build 的产物才开单例；tauri dev 不开，见 src-tauri/src/lib.rs）
#   3. 跑脚本，-Exe 给 src-tauri\target\release\jkk.exe
#   4. 删掉 si.conf.json，以及 %APPDATA% 与 %LOCALAPPDATA% 下的 com.momadacoding.jkk.sitest

if (-not ('JkkWin' -as [type])) {
  Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class JkkWin {
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [DllImport("user32.dll")] public static extern bool PostMessageW(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] public static extern uint GetLongPathNameW(string shortPath, StringBuilder longPath, uint size);
}
'@
}

$WM_CLOSE = 0x0010
$SW_MINIMIZE = 6

$script:ProbeResults = [System.Collections.Generic.List[string]]::new()

function Add-Check([string]$Name, [bool]$Ok, [string]$Detail = '') {
  $script:ProbeResults.Add(('{0}  {1}  {2}' -f ($(if ($Ok) { 'PASS' } else { 'FAIL' })), $Name, $Detail))
}

function Add-Info([string]$Text) {
  $script:ProbeResults.Add("INFO  $Text")
}

# 打印结果表，有 FAIL 时退出码 1
function Complete-Probe {
  $script:ProbeResults
  if ($script:ProbeResults -match '^FAIL') { exit 1 }
}

# 展开成长路径：进程的 Path 是长路径，用 8.3 短路径（LIUHAO~1）或别的写法传进来时，
# 按路径找进程会找不到——残留进程既拦不住、也清不掉，下一轮会被单例插件转给它
function Resolve-JkkExe([string]$Exe) {
  if (-not (Test-Path -LiteralPath $Exe -PathType Leaf)) { throw "找不到 exe：$Exe" }
  $path = (Resolve-Path -LiteralPath $Exe).ProviderPath
  $long = [Text.StringBuilder]::new(32768)
  if ([JkkWin]::GetLongPathNameW($path, $long, [uint32]$long.Capacity) -gt 0) { $path = $long.ToString() }
  $path
}

function Get-JkkInstances([string]$Exe) {
  @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $Exe })
}

# 结束所有同路径的进程。列出来到结束之间进程可能已经自己退了，不能抛错，否则结果表会被吞掉
function Stop-JkkInstances([string]$Exe) {
  Get-JkkInstances $Exe | Stop-Process -Force -ErrorAction SilentlyContinue
}

function Start-Jkk([string]$Exe, [string[]]$ArgList = @(), [string]$StderrLog) {
  $params = @{ FilePath = $Exe; PassThru = $true }
  if ($ArgList.Count -gt 0) { $params.ArgumentList = $ArgList }
  if ($StderrLog) { $params.RedirectStandardError = $StderrLog }
  $p = Start-Process @params
  # 立刻取一次句柄：Start-Process 返回的对象是按 PID 查来的，不先缓存句柄的话，
  # 进程退出后 ExitCode 可能读出 $null，正常退出也会被判成 FAIL
  $null = $p.Handle
  $p
}

# 等主窗口出来，返回句柄；超时返回 [IntPtr]::Zero。
# 认标题：单例插件自己也建了一个顶层的消息窗口（标题是 `<identifier>-siw`），
# MainWindowHandle 可能先落到它上面，往它发 WM_CLOSE 会把插件的接收端销毁
function Wait-JkkMainWindow([System.Diagnostics.Process]$Process, [int]$TimeoutSeconds = 30) {
  for ($i = 0; $i -lt $TimeoutSeconds * 2; $i++) {
    Start-Sleep -Milliseconds 500
    $Process.Refresh()
    if ($Process.MainWindowTitle -like 'jkk*') { return $Process.MainWindowHandle }
  }
  [IntPtr]::Zero
}

# 点关闭 = 关到托盘（默认设置）。走应用自己的 hide：直接 ShowWindow(SW_HIDE) 会绕过 tao，
# tao 仍记着「可见」，之后 show() 什么也不做，测出来的是测试手法的假失败
function Close-JkkToTray([IntPtr]$Hwnd) {
  [void][JkkWin]::PostMessageW($Hwnd, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
  Start-Sleep -Seconds 1
}
