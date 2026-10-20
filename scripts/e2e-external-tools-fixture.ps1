<#
.SYNOPSIS
    造外部 diff / merge 工具 E2E 用的一次性仓库（docs/superpowers/e2e/2026-10-18-external-tools-e2e.md）。

.DESCRIPTION
    全部落在 $env:TEMP 下一个新目录里，跑完整个目录删掉即可。

    ⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，E2E 里的 stage / 保留 / 删除 / 合并
    **必须**在这种临时仓库里做，不得拿本仓库或任何真实仓库试。

    Conflicts（默认）造五个仓库：

      merge    —— 停在 merge 冲突上：
                  Code/P.cs              UU，有冲突标记（①）
                  Code/Gone.cs           DU，HEAD 删了、feat 改了（③）
                  目录 a/-f[1] x.txt     UU，路径含中文、空格、方括号、开头是 -
                  img.bin                UU，二进制（④c）
      rebase   —— 停在 rebase 冲突上：feature 两个提交，第一个重放成功、第二个冲突
      scene    —— Main.unity 用 merge=unityyamlmerge，停在 merge 冲突上（④b）。
                  本机有 UnityYAMLMerge 时 driver 用它（本地配置）；没有时用一个必然失败的假 driver
      drivers  —— .gitattributes 引用三个有问题的 driver：完全没配 / 只配了 name / 程序不存在。
                  分支 pick 上有一个提交，用来在应用里点 Cherry-pick 触发合并前确认
      diff     —— 普通改动：a.txt 同时有已暂存和未暂存的改动；`目录 a/f[1] x.txt` 只在工作区改；
                  最新提交把 old-name.txt 改名为 new-name.txt

    Big 造一个大仓库，记录 merge driver 检查的首次与命中缓存耗时。

    写文件一律用 .NET API：PowerShell 的 Set-Content -Path 会把 `[1]` 当通配符。

.PARAMETER Shape
    Conflicts 或 Big，见上。

.PARAMETER Files
    Big 形态的文件数，默认 50000。

.PARAMETER UnityYamlMerge
    本机 UnityYAMLMerge.exe 的路径，scene 仓库用它做自动 driver。

.EXAMPLE
    pwsh -File scripts/e2e-external-tools-fixture.ps1
    pwsh -File scripts/e2e-external-tools-fixture.ps1 -Shape Big

.EXAMPLE
    # 用完删掉（脚本会把路径打在最后一行）
    Remove-Item -Recurse -Force C:\Users\me\AppData\Local\Temp\jkk-e2e-ext-ab12cd34
#>
[CmdletBinding()]
param(
    [ValidateSet('Conflicts', 'Big')]
    [string]$Shape = 'Conflicts',
    [ValidateRange(1000, 200000)]
    [int]$Files = 50000,
    [string]$UnityYamlMerge = 'C:\Program Files\Unity\Hub\Editor\6000.6.6f1\Editor\Data\Tools\UnityYAMLMerge.exe'
)

$ErrorActionPreference = 'Stop'

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw 'git 不在 PATH 上。'
}

$Utf8 = New-Object System.Text.UTF8Encoding $false

function Write-Step([string]$Message) {
    Write-Host "==> $Message"
}

# 外部命令的失败不会触发 $ErrorActionPreference，必须自己查 $LASTEXITCODE（`AGENTS.md` §10）。
# 返回退出码：预期会失败的命令（merge / rebase 停在冲突上）由调用方核对具体的码。
function Invoke-Git {
    param(
        [Parameter(Mandatory)][string]$Repo,
        [Parameter(Mandatory)][string[]]$Arguments,
        [int[]]$Allow = @(0)
    )
    $out = & git -C $Repo @Arguments 2>&1
    $code = $LASTEXITCODE
    if ($Allow -notcontains $code) {
        throw "git $($Arguments -join ' ') 失败（exit $code）：`n$out"
    }
    return $code
}

# 内容按字节原样写，LF 行尾；路径按字面量处理（含 [] 也不当通配符）
function Write-Text([string]$Path, [string]$Text) {
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($Path)) | Out-Null
    [System.IO.File]::WriteAllText($Path, $Text, $Utf8)
}

function Write-Bytes([string]$Path, [byte[]]$Bytes) {
    [System.IO.Directory]::CreateDirectory([System.IO.Path]::GetDirectoryName($Path)) | Out-Null
    [System.IO.File]::WriteAllBytes($Path, $Bytes)
}

function New-Repo([string]$Root, [string]$Name) {
    $repo = Join-Path $Root $Name
    [System.IO.Directory]::CreateDirectory($repo) | Out-Null
    Invoke-Git $repo @('init', '-q', '-b', 'main') | Out-Null
    Invoke-Git $repo @('config', 'user.email', 'e2e@example.com') | Out-Null
    Invoke-Git $repo @('config', 'user.name', 'E2E') | Out-Null
    return $repo
}

function Save-All([string]$Repo, [string]$Message) {
    Invoke-Git $Repo @('add', '-A') | Out-Null
    Invoke-Git $Repo @('commit', '-q', '-m', $Message) | Out-Null
}

function Lines([string[]]$Items) {
    return ($Items -join "`n") + "`n"
}

function Expect-Conflict([string]$Repo, [string[]]$Arguments) {
    # 1 = 冲突停下（merge / cherry-pick）；rebase 冲突也是 1
    $code = Invoke-Git $Repo $Arguments -Allow @(0, 1)
    if ($code -ne 1) {
        throw "git $($Arguments -join ' ') 预期停在冲突上（exit 1），实际 exit $code"
    }
}

$Root = Join-Path $env:TEMP ('jkk-e2e-ext-' + [System.Guid]::NewGuid().ToString('N').Substring(0, 8))
[System.IO.Directory]::CreateDirectory($Root) | Out-Null

if ($Shape -eq 'Big') {
    Write-Step "造 $Files 个文件的大仓库"
    $repo = New-Repo $Root 'big'
    Write-Text (Join-Path $repo '.gitattributes') "*.unity merge=e2e-missing`n"
    $perDir = 100
    for ($i = 0; $i -lt $Files; $i++) {
        $dir = Join-Path $repo ('Assets\D{0:D4}' -f [int][Math]::Floor($i / $perDir))
        $ext = if ($i % $perDir -eq 0) { 'unity' } else { 'cs' }
        Write-Text (Join-Path $dir ('F{0:D3}.{1}' -f ($i % $perDir), $ext)) "x`n"
    }
    Save-All $repo 'big'
    Write-Step '完成。打开这个仓库，记录顶部横幅出现的耗时；再保存一个文件，确认横幅不闪、没有明显卡顿'
    Write-Output $Root
    return
}

# ---------- merge ----------
Write-Step 'merge：merge 冲突（标记 / DU / 特殊路径 / 二进制）'
$repo = New-Repo $Root 'merge'
$weird = '目录 a\-f[1] x.txt'
Write-Text (Join-Path $repo 'Code\P.cs') (Lines @('line1', 'line2', 'line3', 'line4', 'line5', 'line6'))
Write-Text (Join-Path $repo 'Code\Gone.cs') (Lines @('class Gone {}'))
Write-Text (Join-Path $repo $weird) (Lines @('base'))
Write-Bytes (Join-Path $repo 'img.bin') ([byte[]](0x50, 0x4E, 0x47, 0x00, 0x01))
Save-All $repo 'base'
Invoke-Git $repo @('checkout', '-q', '-b', 'feat') | Out-Null
Write-Text (Join-Path $repo 'Code\P.cs') (Lines @('patch-local', 'line2', 'line3', 'line4', 'line5', 'line6'))
Write-Text (Join-Path $repo 'Code\Gone.cs') (Lines @('class Gone { int changed; }'))
Write-Text (Join-Path $repo $weird) (Lines @('theirs'))
Write-Bytes (Join-Path $repo 'img.bin') ([byte[]](0x50, 0x4E, 0x47, 0x00, 0x03))
Save-All $repo 'feat 的改动'
Invoke-Git $repo @('checkout', '-q', 'main') | Out-Null
Write-Text (Join-Path $repo 'Code\P.cs') (Lines @('colleague1', 'line2', 'line3', 'line4', 'line5', 'line6'))
Invoke-Git $repo @('--literal-pathspecs', 'rm', '-q', '--', 'Code/Gone.cs') | Out-Null
Write-Text (Join-Path $repo $weird) (Lines @('ours'))
Write-Bytes (Join-Path $repo 'img.bin') ([byte[]](0x50, 0x4E, 0x47, 0x00, 0x02))
Save-All $repo 'main 的改动'
Expect-Conflict $repo @('merge', 'feat')

# ---------- rebase ----------
Write-Step 'rebase：两个提交，第二个冲突'
$repo = New-Repo $Root 'rebase'
Write-Text (Join-Path $repo 'a.txt') (Lines @('line1', 'line2', 'line3', 'line4', 'line5'))
Save-All $repo 'base'
Invoke-Git $repo @('checkout', '-q', '-b', 'feature') | Out-Null
Write-Text (Join-Path $repo 'a.txt') (Lines @('line1', 'line2', 'line3', 'line4', 'line5 c1'))
Save-All $repo 'c1 改第五行'
Write-Text (Join-Path $repo 'a.txt') (Lines @('line1 c2', 'line2', 'line3', 'line4', 'line5 c1'))
Save-All $repo 'c2 改第一行'
Invoke-Git $repo @('checkout', '-q', 'main') | Out-Null
Write-Text (Join-Path $repo 'a.txt') (Lines @('line1 main', 'line2', 'line3', 'line4', 'line5'))
Save-All $repo 'main 改第一行'
Invoke-Git $repo @('checkout', '-q', 'feature') | Out-Null
Expect-Conflict $repo @('rebase', 'main')

# ---------- scene ----------
Write-Step 'scene：自动 driver 合并失败（④b）'
$repo = New-Repo $Root 'scene'
if (Test-Path -LiteralPath $UnityYamlMerge) {
    $exe = $UnityYamlMerge.Replace('\', '/')
    $driver = "'$exe' merge -h -p --force --fallback none %O %B %A %A"
    Write-Step "  driver 用本机 UnityYAMLMerge：$UnityYamlMerge"
} else {
    # 行为与 UnityYAMLMerge 合不了时相同：%A 里留着合法内容（ours 原样）、以 1 退出（spec §4.1）
    $driver = 'exit 1'
    Write-Warning "找不到 $UnityYamlMerge，scene 仓库改用一个必然失败的假 driver"
}
Invoke-Git $repo @('config', 'merge.unityyamlmerge.name', 'Unity SmartMerge') | Out-Null
Invoke-Git $repo @('config', 'merge.unityyamlmerge.driver', $driver) | Out-Null
Write-Text (Join-Path $repo '.gitattributes') "*.unity merge=unityyamlmerge`n"
function Scene([string]$X) {
    return Lines @(
        '%YAML 1.1',
        '%TAG !u! tag:unity3d.com,2011:',
        '--- !u!1 &100000',
        'GameObject:',
        '  m_ObjectHideFlags: 0',
        '  m_Name: Cube',
        '--- !u!4 &400000',
        'Transform:',
        '  m_ObjectHideFlags: 0',
        '  m_GameObject: {fileID: 100000}',
        "  m_LocalPosition: {x: $X, y: 0, z: 0}"
    )
}
Write-Text (Join-Path $repo 'Assets\Scenes\Main.unity') (Scene '1')
Save-All $repo 'base'
Invoke-Git $repo @('checkout', '-q', '-b', 'feat') | Out-Null
Write-Text (Join-Path $repo 'Assets\Scenes\Main.unity') (Scene '5')
Save-All $repo 'theirs x=5'
Invoke-Git $repo @('checkout', '-q', 'main') | Out-Null
Write-Text (Join-Path $repo 'Assets\Scenes\Main.unity') (Scene '9')
Save-All $repo 'ours x=9'
Expect-Conflict $repo @('merge', 'feat')

# ---------- drivers ----------
Write-Step 'drivers：完全没配 / 只配了 name / 程序不存在'
$repo = New-Repo $Root 'drivers'
Invoke-Git $repo @('config', 'merge.e2e-half.name', 'Half') | Out-Null
Invoke-Git $repo @('config', 'merge.e2e-badexe.driver', 'C:/nope/NotThere.exe %O %A %B') | Out-Null
Write-Text (Join-Path $repo '.gitattributes') (Lines @(
    '*.unity  merge=e2e-missing',
    '*.prefab merge=e2e-half',
    '*.asset  merge=e2e-badexe'
))
Write-Text (Join-Path $repo 'Assets\Main.unity') "scene`n"
Write-Text (Join-Path $repo 'Assets\P.prefab') "prefab`n"
Write-Text (Join-Path $repo 'Assets\Data.asset') "asset`n"
Write-Text (Join-Path $repo 'notes.txt') "base`n"
Save-All $repo 'base'
Invoke-Git $repo @('checkout', '-q', '-b', 'pick') | Out-Null
Write-Text (Join-Path $repo 'notes.txt') "picked`n"
Save-All $repo '拿来 cherry-pick 的提交'
Invoke-Git $repo @('checkout', '-q', 'main') | Out-Null

# ---------- diff ----------
Write-Step 'diff：普通改动、已暂存、改名提交'
$repo = New-Repo $Root 'diff'
$weirdDiff = '目录 a\f[1] x.txt'
Write-Text (Join-Path $repo 'a.txt') (Lines @('line 1', 'line 2', 'line 3'))
Write-Text (Join-Path $repo $weirdDiff) (Lines @('base'))
Write-Text (Join-Path $repo 'old-name.txt') (Lines (1..30 | ForEach-Object { "row $_" }))
Save-All $repo 'c1'
Invoke-Git $repo @('mv', 'old-name.txt', 'new-name.txt') | Out-Null
Write-Text (Join-Path $repo 'new-name.txt') (Lines ((1..30 | ForEach-Object { "row $_" }) + 'row added'))
Save-All $repo 'c2 改名并追加一行'
Write-Text (Join-Path $repo 'a.txt') (Lines @('line 1', 'line 2 staged', 'line 3'))
Invoke-Git $repo @('add', 'a.txt') | Out-Null
Write-Text (Join-Path $repo 'a.txt') (Lines @('line 1', 'line 2 staged', 'line 3 unstaged'))
Write-Text (Join-Path $repo $weirdDiff) (Lines @('changed in workdir'))

Write-Step '完成。各仓库：merge / rebase / scene / drivers / diff'
Write-Output $Root
