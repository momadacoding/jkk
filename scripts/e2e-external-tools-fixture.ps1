<#
.SYNOPSIS
    造外部 diff / merge 工具 E2E 用的一次性仓库（docs/superpowers/e2e/2026-10-18-external-tools-e2e.md）。

.DESCRIPTION
    全部落在 $env:TEMP 下一个新目录里，跑完整个目录删掉即可。

    ⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，E2E 里的 stage / 保留 / 删除 / 合并
    **必须**在这种临时仓库里做，不得拿本仓库或任何真实仓库试。

    Conflicts（默认）一个场景一个仓库，目录名以清单里的场景编号开头。
    每个冲突仓库**只有一个冲突文件**，解决完就能 Commit，场景之间互不影响：

      01-merge-text    merge 停在冲突上：Code/P.cs（UU，有冲突标记）
      02-rebase        rebase 停在冲突上：feature 两个提交，第一个重放成功、第二个在 a.txt 冲突
      03a-keep         merge 停在冲突上：Code/Gone.cs（DU，HEAD 删了、feat 改了）
      03b-delete       merge 停在冲突上：Code/Gone.cs（UD，HEAD 改了、feat 删了）
      05-vscode        merge 停在冲突上：Code/P.cs（UU）；另有未暂存的普通改动 notes.txt
      06-scene         merge 停在冲突上：Assets/Scenes/Main.unity，merge=unityyamlmerge（自动 driver 合并失败）
      07-drivers       没有冲突。.gitattributes 引用三个有问题的 driver：完全没配 / 只配了 name / 程序不存在；
                       分支 pick 上有一个提交，用来在应用里 Cherry Pick 触发合并前确认
      08-binary        merge 停在冲突上：img.bin（UU，二进制）
      09-special-path  merge 停在冲突上：`目录 a/-f[1] x.txt`（UU，路径含中文、空格、方括号、开头是 -）
      10-busy          merge 停在冲突上：Code/P.cs（UU）
      diff             普通改动：a.txt 同时有已暂存和未暂存的改动；`目录 a/f[1] x.txt` 只在工作区改；
                       最新提交把 old-name.txt 改名为 new-name.txt（场景 4，2026-10-19 已真机通过，留作回归）

    冲突仓库里，main（HEAD）那边的内容一律带 `from-main`，feat 那边带 `from-feat`，
    共同祖先带 `base`，在外部工具里一眼能分出哪栏是哪边。

    Big 造一个大仓库，记录 merge driver 检查的耗时（场景 12）。

    写文件一律用 .NET API：PowerShell 的 Set-Content -Path 会把 `[1]` 当通配符。

.PARAMETER Shape
    Conflicts 或 Big，见上。

.PARAMETER Files
    Big 形态的文件数，默认 50000。

.PARAMETER UnityYamlMerge
    本机 UnityYAMLMerge.exe 的路径，06-scene 仓库用它做自动 driver。找不到时改用一个必然失败的假 driver。

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

# 三个脚本块依次写出 base / feat / main 三个版本，然后在 main 上 merge feat，停在冲突上。
# 脚本块拿到仓库路径作参数。
function New-MergeConflict([string]$Root, [string]$Name, [scriptblock]$Base, [scriptblock]$Feat, [scriptblock]$Main) {
    $repo = New-Repo $Root $Name
    & $Base $repo
    Save-All $repo 'base'
    Invoke-Git $repo @('checkout', '-q', '-b', 'feat') | Out-Null
    & $Feat $repo
    Save-All $repo 'feat 的改动'
    Invoke-Git $repo @('checkout', '-q', 'main') | Out-Null
    & $Main $repo
    Save-All $repo 'main 的改动'
    Expect-Conflict $repo @('merge', 'feat')
    return $repo
}

# 第一行三边各改各的，其余行相同：标准的 UU，工作区里有冲突标记
function Write-P([string]$Repo, [string]$First) {
    Write-Text (Join-Path $Repo 'Code\P.cs') (Lines @($First, 'line2', 'line3', 'line4', 'line5', 'line6'))
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
    Write-Step '完成。仓库在 big 子目录'
    Write-Output $Root
    return
}

# ---------- 01 / 10：普通文本冲突 ----------
foreach ($name in '01-merge-text', '10-busy') {
    Write-Step "${name}：Code/P.cs UU"
    New-MergeConflict $Root $name `
        { param($r) Write-P $r 'base' } `
        { param($r) Write-P $r 'from-feat' } `
        { param($r) Write-P $r 'from-main' } | Out-Null
}

# ---------- 02：rebase ----------
Write-Step '02-rebase：两个提交，第二个冲突'
$repo = New-Repo $Root '02-rebase'
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

# ---------- 03a / 03b：一边删一边改 ----------
Write-Step '03a-keep：Code/Gone.cs DU（HEAD 删了、feat 改了）'
New-MergeConflict $Root '03a-keep' `
    { param($r) Write-Text (Join-Path $r 'Code\Gone.cs') (Lines @('class Gone { /* base */ }')) } `
    { param($r) Write-Text (Join-Path $r 'Code\Gone.cs') (Lines @('class Gone { /* from-feat */ }')) } `
    { param($r) Invoke-Git $r @('rm', '-q', '--', 'Code/Gone.cs') | Out-Null } | Out-Null

Write-Step '03b-delete：Code/Gone.cs UD（HEAD 改了、feat 删了）'
New-MergeConflict $Root '03b-delete' `
    { param($r) Write-Text (Join-Path $r 'Code\Gone.cs') (Lines @('class Gone { /* base */ }')) } `
    { param($r) Invoke-Git $r @('rm', '-q', '--', 'Code/Gone.cs') | Out-Null } `
    { param($r) Write-Text (Join-Path $r 'Code\Gone.cs') (Lines @('class Gone { /* from-main */ }')) } | Out-Null

# ---------- 05：VS Code ----------
Write-Step '05-vscode：Code/P.cs UU + 未暂存的 notes.txt'
$repo = New-MergeConflict $Root '05-vscode' `
    { param($r) Write-P $r 'base'; Write-Text (Join-Path $r 'notes.txt') (Lines @('note base')) } `
    { param($r) Write-P $r 'from-feat' } `
    { param($r) Write-P $r 'from-main' }
# merge 停下之后再改一个与合并无关的文件，作为 Unstaged 的普通改动
Write-Text (Join-Path $repo 'notes.txt') (Lines @('note changed in workdir'))

# ---------- 06：场景文件 ----------
Write-Step '06-scene：自动 driver 合并失败'
if (Test-Path -LiteralPath $UnityYamlMerge) {
    $exe = $UnityYamlMerge.Replace('\', '/')
    $driver = "'$exe' merge -h -p --force --fallback none %O %B %A %A"
    Write-Step "  driver 用本机 UnityYAMLMerge：$UnityYamlMerge"
} else {
    # 只模拟「driver 失败」：%A 原样留着 ours、以 1 退出。真实 UnityYAMLMerge 合不了时会在冲突处
    # 取对方的值、以 2 退出（2026-10-22 实测），所以 6a 看到的是「与 HEAD 完全相同」，6b 看到的是 diff
    $driver = 'exit 1'
    Write-Warning "找不到 $UnityYamlMerge，06-scene 改用一个必然失败的假 driver"
}
# 必须是 UnityYAMLMerge 解析得了的完整场景：缺 m_Father / m_Component 这些字段时它直接报
# 「Could not determine the transform parent」并失败，连只有一边改的合并都合不了（2026-10-22 实测）
function Scene([string]$X) {
    return Lines @(
        '%YAML 1.1',
        '%TAG !u! tag:unity3d.com,2011:',
        '--- !u!1 &100000',
        'GameObject:',
        '  m_ObjectHideFlags: 0',
        '  m_CorrespondingSourceObject: {fileID: 0}',
        '  m_PrefabInstance: {fileID: 0}',
        '  m_PrefabAsset: {fileID: 0}',
        '  serializedVersion: 6',
        '  m_Component:',
        '  - component: {fileID: 400000}',
        '  m_Layer: 0',
        '  m_Name: Cube',
        '  m_TagString: Untagged',
        '  m_Icon: {fileID: 0}',
        '  m_NavMeshLayer: 0',
        '  m_StaticEditorFlags: 0',
        '  m_IsActive: 1',
        '--- !u!4 &400000',
        'Transform:',
        '  m_ObjectHideFlags: 0',
        '  m_CorrespondingSourceObject: {fileID: 0}',
        '  m_PrefabInstance: {fileID: 0}',
        '  m_PrefabAsset: {fileID: 0}',
        '  m_GameObject: {fileID: 100000}',
        '  serializedVersion: 2',
        '  m_LocalRotation: {x: 0, y: 0, z: 0, w: 1}',
        "  m_LocalPosition: {x: $X, y: 0, z: 0}",
        '  m_LocalScale: {x: 1, y: 1, z: 1}',
        '  m_ConstrainProportionsScale: 0',
        '  m_Children: []',
        '  m_Father: {fileID: 0}',
        '  m_LocalEulerAnglesHint: {x: 0, y: 0, z: 0}',
        '--- !u!1660057539 &9223372036854775807',
        'SceneRoots:',
        '  m_ObjectHideFlags: 0',
        '  m_Roots:',
        '  - {fileID: 400000}'
    )
}
$repo = New-Repo $Root '06-scene'
Invoke-Git $repo @('config', 'merge.unityyamlmerge.name', 'Unity SmartMerge') | Out-Null
Invoke-Git $repo @('config', 'merge.unityyamlmerge.driver', $driver) | Out-Null
Write-Text (Join-Path $repo '.gitattributes') "*.unity merge=unityyamlmerge`n"
Write-Text (Join-Path $repo 'Assets\Scenes\Main.unity') (Scene '1')
Save-All $repo 'base x=1'
Invoke-Git $repo @('checkout', '-q', '-b', 'feat') | Out-Null
Write-Text (Join-Path $repo 'Assets\Scenes\Main.unity') (Scene '5')
Save-All $repo 'feat x=5'
Invoke-Git $repo @('checkout', '-q', 'main') | Out-Null
Write-Text (Join-Path $repo 'Assets\Scenes\Main.unity') (Scene '9')
Save-All $repo 'main x=9'
Expect-Conflict $repo @('merge', 'feat')

# ---------- 07：缺 driver ----------
Write-Step '07-drivers：完全没配 / 只配了 name / 程序不存在'
$repo = New-Repo $Root '07-drivers'
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

# ---------- 08：二进制 ----------
Write-Step '08-binary：img.bin UU'
New-MergeConflict $Root '08-binary' `
    { param($r) Write-Bytes (Join-Path $r 'img.bin') ([byte[]](0x50, 0x4E, 0x47, 0x00, 0x01)) } `
    { param($r) Write-Bytes (Join-Path $r 'img.bin') ([byte[]](0x50, 0x4E, 0x47, 0x00, 0x03)) } `
    { param($r) Write-Bytes (Join-Path $r 'img.bin') ([byte[]](0x50, 0x4E, 0x47, 0x00, 0x02)) } | Out-Null

# ---------- 09：特殊路径 ----------
Write-Step '09-special-path：目录 a/-f[1] x.txt UU'
$weird = '目录 a\-f[1] x.txt'
New-MergeConflict $Root '09-special-path' `
    { param($r) Write-Text (Join-Path $r $weird) (Lines @('base')) } `
    { param($r) Write-Text (Join-Path $r $weird) (Lines @('from-feat')) } `
    { param($r) Write-Text (Join-Path $r $weird) (Lines @('from-main')) } | Out-Null

# ---------- diff（场景 4 回归） ----------
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

Write-Step '完成。每个场景一个仓库，目录名以场景编号开头'
Write-Output $Root
