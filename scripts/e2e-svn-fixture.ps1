<#
.SYNOPSIS
    造一个临时 SVN fixture 仓库，给 Phase 1c 的真实窗口 E2E 用。

.DESCRIPTION
    全部落在 $env:TEMP 下，跑完整个目录删掉即可。

    ⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，E2E 里那些要发 SVN 写操作的场景
    **必须**在这种临时仓库里做，不得拿本仓库或任何真实仓库试。

    两种形态，对应清单里不同的场景：

      Simple（默认）—— 一个 revision，wc1 已经 update 到 HEAD。
        给场景 1-7、9、10 用。想造「外部提交」就去 wc2 里提交。

      Deep —— 服务器走到 rN（默认 230，> PAGE_SIZE 200），而 **wc1 停在 r1**。
        给场景 8 用。这是「同事在推进、我还没 update」的日常形态，也是
        第二轮 review 那条翻页缺陷唯一能暴露出来的形状：
        首屏只覆盖最新 200 条，基线 r1 靠单点锚点补进来，
        中间 r2..r30 是空的 —— 翻页必须能把那一段拿回来。

.PARAMETER Shape
    Simple 或 Deep，见上。

.PARAMETER Revisions
    Deep 形态下服务器上的总 revision 数。默认 230。

.EXAMPLE
    pwsh -File scripts/e2e-svn-fixture.ps1
    pwsh -File scripts/e2e-svn-fixture.ps1 -Shape Deep

.EXAMPLE
    # 用完删掉（脚本会把路径打在最后一行）
    Remove-Item -Recurse -Force C:\Users\me\AppData\Local\Temp\phase1c_ab12cd34
#>
[CmdletBinding()]
param(
    [ValidateSet('Simple', 'Deep')]
    [string]$Shape = 'Simple',
    [ValidateRange(3, 2000)]
    [int]$Revisions = 230
)

$ErrorActionPreference = 'Stop'

# 外部命令的失败不会触发 $ErrorActionPreference，必须自己查 $LASTEXITCODE
# （`AGENTS.md` §10）。不查的话 fixture 会带着半截状态交给测试，
# 而测试失败时看上去像是程序的问题。
function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments, [string]$WorkDir)
    $out = & $Program @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "$Program $($Arguments -join ' ') 失败（exit $LASTEXITCODE）：`n$out"
    }
}

foreach ($exe in @('svn', 'svnadmin')) {
    if (-not (Get-Command $exe -ErrorAction SilentlyContinue)) {
        throw "$exe 不在 PATH 上。spec §17 要求命令行 SVN 客户端可用（如 SlikSvn / TortoiseSVN 勾选 command line tools）。"
    }
}

$root = Join-Path $env:TEMP ("phase1c_" + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $root | Out-Null
Write-Host "[1/5] 建仓库 $root\repo"
Invoke-Checked svnadmin @('create', "$root\repo")

# Windows 上 file:// URL 是 file:///C:/path（三斜杠 + 盘符 + 正斜杠）。
# 这是已知跨平台坑，不能靠路径拼接碰运气（spec §14.1）。
$url = 'file:///' + (Resolve-Path "$root\repo").Path.Replace('\', '/')

Write-Host "[2/5] checkout wc1"
Invoke-Checked svn @('checkout', '--non-interactive', $url, "$root\wc1")

Push-Location "$root\wc1"
try {
    New-Item -ItemType Directory -Path 'Code', 'Code\Deep', 'Art' | Out-Null
    Set-Content 'Code\A.cs'      -Value 'class A {}' -NoNewline
    Set-Content 'Code\Deep\D.cs' -Value 'class D {}' -NoNewline
    # 白名单内但扩展名不符 + 白名单外各一个：这两个**本来就不该**进 git，
    # 场景 6 的漏收提示不能把它们报出来（报了就是几千条噪音）。
    Set-Content 'Code\Art.psd'   -Value 'PSD' -NoNewline
    Set-Content 'Art\boss.psd'   -Value 'PSD' -NoNewline
    Write-Host "[3/5] 提交 r1"
    Invoke-Checked svn @('add', 'Code', 'Art', '--non-interactive')
    Invoke-Checked svn @('commit', '-m', 'r1', '--non-interactive')
    # **两种形态都要 update。** wc1 是在 r1 之前 checkout 的，工作副本根还停在
    # r0；不 update 的话 Deep 形态造出来的是「wc 停在 r0」，与它要复现的
    # 「停在一个真实的旧 revision」不是同一个形状。
    # Deep 形态之后就再也不 update 了 —— 那才是「同事在推进、我还没 update」。
    Invoke-Checked svn @('update', '--non-interactive')
} finally {
    Pop-Location
}

Write-Host "[4/5] checkout wc2（扮演同事的机器）"
Invoke-Checked svn @('checkout', '--non-interactive', $url, "$root\wc2")

if ($Shape -eq 'Deep') {
    Write-Host "[5/5] 由 wc2 提交 r2..r$Revisions（wc1 故意不 update，停在 r1）"
    Push-Location "$root\wc2"
    try {
        for ($n = 2; $n -le $Revisions; $n++) {
            Set-Content 'Code\A.cs' -Value "class A { int v$n; }" -NoNewline
            Invoke-Checked svn @('commit', '-m', "r$n", '--non-interactive')
            Invoke-Checked svn @('update', '--non-interactive')
            if ($n % 25 -eq 0) { Write-Host "      ... r$n" }
        }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "[5/5] Simple 形态，服务器停在 r1"
}

Write-Host ''
Write-Host "形态       : $Shape"
if ($Shape -eq 'Deep') {
    Write-Host "服务器     : r$Revisions"
    Write-Host "wc1        : 停在 r1（基线会记 r1，首屏拉不到它，靠锚点补）"
} else {
    Write-Host "服务器     : r1"
    Write-Host "wc1        : r1"
}
Write-Host "同事的副本 : $root\wc2   （在这里提交 = 外部变更）"
Write-Host ''
Write-Host "在应用里打开这个目录：" -ForegroundColor Cyan
Write-Host "  $root\wc1" -ForegroundColor Cyan
Write-Host ''
Write-Host "用完删掉整个 fixture："
Write-Host "  Remove-Item -Recurse -Force '$root'"
