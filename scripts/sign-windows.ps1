#Requires -Version 7.0
<#
.SYNOPSIS
  给单个文件做 Authenticode 签名（release.yml 的 tauri signCommand 调用）。
.DESCRIPTION
  证书预先由 CI 导入 CurrentUser\My（pfx 即导即删，不落盘），本脚本只从
  环境变量 CERT_THUMB 读指纹——密码/指纹/证书路径都不进命令行
  （signtool 的 /p 会暴露在进程列表里），signCommand 配置因此可以完全静态。
  时间戳不可省：没有它，证书过期后签名一起作废。
.PARAMETER Target
  待签文件路径（tauri 把 %1 替换成它；exe 与安装包各调一次）。
.PARAMETER SkipVerify
  仅本地自签机制验证用：自签证书不在受信根里，verify /pa 必然失败。
  CI 永远不传这个开关。
#>
param(
    [Parameter(Mandatory)][string]$Target,
    [switch]$SkipVerify
)
$ErrorActionPreference = 'Stop'

$thumbprint = $env:CERT_THUMB
if ([string]::IsNullOrEmpty($thumbprint)) { throw 'CERT_THUMB 未设置（证书未导入？）' }
if (-not (Test-Path $Target)) { throw "待签文件不存在：$Target" }

# 定位 signtool：Windows SDK kits 目录按版本倒序取第一个。
# bin 下混着 x64/x86/arm64 这些非版本目录，且字符串序会把 10.0.9.0 排到
# 10.0.26100.0 前面——必须按 [version] 过滤再排序。
$kitsRoot = 'C:\Program Files (x86)\Windows Kits\10\bin'
$signtool = Get-ChildItem $kitsRoot -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -as [version] } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
    Where-Object { Test-Path $_ } |
    Select-Object -First 1
if (-not $signtool) {
    throw "在 $kitsRoot 下找不到 signtool.exe（需要 Windows SDK；CI runner 自带）"
}

# 原生命令的退出码一律自己判：PS 7.3+ 的 $PSNativeCommandUseErrorActionPreference
# 在部分版本默认为 $true，非零退出会直接抛异常，下面的重试循环就形同虚设
$PSNativeCommandUseErrorActionPreference = $false

# 时间戳服务器会限流、也会瞬时超时。一次失败就让 bundler 失败、进而毁掉整个 tag
# 构建，而 publish 只在 v* tag 上跑，恢复要重跑那个 tag——所以必须重试 + 备用 TSA。
$tsaList = @('http://timestamp.digicert.com', 'http://timestamp.sectigo.com')
$signed = $false
foreach ($attempt in 1..3) {
    foreach ($tsa in $tsaList) {
        & $signtool sign /fd sha256 /td sha256 /tr $tsa /sha1 $thumbprint $Target
        if ($LASTEXITCODE -eq 0) { $signed = $true; break }
        Write-Host "signtool sign 失败（exit $LASTEXITCODE，tsa=$tsa，第 $attempt 轮），重试"
    }
    if ($signed) { break }
    if ($attempt -lt 3) { Start-Sleep -Seconds (5 * $attempt) }
}
if (-not $signed) { throw "signtool sign 连续失败：$Target" }

if (-not $SkipVerify) {
    & $signtool verify /pa $Target | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "签完校验失败：$Target" }
}
