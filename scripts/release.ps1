#Requires -Version 7.4
<#
.SYNOPSIS
  发版：改版本号 → 推 master → 先在 CI 上预检（workflow_dispatch）→ 绿了才打 tag → 等正式构建。
.DESCRIPTION
  为什么要先预检：CI runner 和本机的差异（8.3 短路径的 %TEMP%、全新的 git 配置、
  SlikSvn 版本……）本地复现不了，以前只能打了 tag 才发现，挂一次就废一个版本号
  （v0.7.0、v0.7.1 都是这么废的）。预检和正式构建跑的是同一个 workflow、同一个提交。

  可重入——中途失败后用同一个版本号重跑，按远端的实际状态续上：
    - 版本号已提交但没推上去（本地只领先一个本脚本的发版提交）：推送后继续
    - 同一提交已有成功或进行中的预检：直接复用 / 接着等
    - 预检失败：照常走 worktree + PR 修复，合进 master 后重跑，会对新 master 重新预检
    - 本地 tag 已打在该提交上但没推上去：推送后继续
    - tag 已推送（构建没等完、watch 断线、转正式发布失败）：跳过前面所有步骤，
      只等那次 tag 构建、按需转正式发布
  无法自动续上的情况（比如版本号提交没推上去而 master 已前进）会停下并说明怎么处理，
  不会替你做破坏性操作。

  只改发版提交例外允许的内容（AGENTS.md §4.1）：三个清单与 Cargo.lock 各恰好一行版本号，
  外加重新生成的 THIRD_PARTY_NOTICES.md；按 numstat 校验，多出任何改动直接失败。

  全程约 80 分钟（预检、正式构建各约 40 分钟）。需要 gh 已登录。
  用 `pwsh -NoProfile -File scripts/release.ps1 ...` 运行，避免 profile 改了原生命令的错误处理。
.PARAMETER Version
  要发的版本号，形如 0.7.3（不带 v）。
.PARAMETER Publish
  正式构建成功后直接把 draft Release 转成正式发布并标为 latest。
  不传时停在 draft，由人去 Release 页面过目后再点发布。
.PARAMETER DryRun
  只做检查并打印将要做的事：不写文件、不移动本地分支、不提交、不碰 CI。
.EXAMPLE
  pwsh -NoProfile -File scripts/release.ps1 -Version 0.7.3 -Publish
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [switch]$Publish,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
# 下面所有原生命令都自己看 $LASTEXITCODE；profile 若打开了这个开关，
# `git rev-parse --verify` 查一个本来就不存在的 tag 这种正常情况也会被当成异常抛出
$PSNativeCommandUseErrorActionPreference = $false

function Step([string]$msg) { Write-Host "`n==> $msg" -ForegroundColor Cyan }

# 外部命令失败时 PowerShell 不会抛，必须逐个看 $LASTEXITCODE
function Invoke-Checked {
    param([Parameter(Mandatory)][string]$What, [Parameter(Mandatory)][scriptblock]$Cmd)
    $out = & $Cmd
    if ($LASTEXITCODE -ne 0) { throw "$What 失败（exit $LASTEXITCODE）" }
    $out
}

function Show-RunFailure([string]$runId) {
    gh run view $runId --json jobs --jq '.jobs[] | select(.conclusion=="failure") | "失败的 job：" + .name + "，步骤：" + ([.steps[] | select(.conclusion=="failure") | .name] | join(", "))'
    gh run view $runId --log-failed 2>&1 |
        ForEach-Object { ($_ -split "`t")[-1] -replace '^\S+Z ', '' } |
        Select-String -Pattern 'FAILED|panicked|^error|-->|::error' -Context 0, 3 |
        Select-Object -First 30
}

# 等某个 run 真正结束再判结论。`gh run watch` 网络一抖就会提前退出，
# 那时 conclusion 还是空的——把它当失败，会让人以为 tag 构建挂了而白白废掉一个版本号
function Wait-Run([string]$runId, [string]$label) {
    Write-Host "$label：https://github.com/$script:repo/actions/runs/$runId"
    gh run watch $runId --interval 30
    $errors = 0
    while ($true) {
        $json = gh run view $runId --json status,conclusion
        if ($LASTEXITCODE -eq 0) {
            $errors = 0
            $s = $json | ConvertFrom-Json
            if ($s.status -eq 'completed') { break }
        } else {
            # 查不动（登录过期、长时间断网）时别无声地卡住；连续 30 分钟都查不动就放弃，
            # run 本身不受影响，重跑脚本会接着等它
            $errors++
            Write-Host "查询 run $runId 失败（连续第 $errors 次），30 秒后重试" -ForegroundColor Yellow
            if ($errors -ge 60) { throw "连续 30 分钟查不到 run $runId 的状态。修好 gh 登录 / 网络后重跑本脚本即可续上" }
        }
        Start-Sleep 30
    }
    if ($s.conclusion -ne 'success') {
        Show-RunFailure $runId
        throw "$label 未通过（$($s.conclusion)）"
    }
    Write-Host "$label 通过" -ForegroundColor Green
}

# 某个提交上、某种事件触发、指定分支（tag 推送时是 tag 名）的 release run，按创建时间排序
function Get-Runs([string]$event, [string]$sha, [string]$branch) {
    # 过滤放在 PowerShell 里做，不往原生命令的参数里塞带引号的 jq
    $json = gh run list --workflow release.yml --event $event --commit $sha --limit 50 `
        --json databaseId,headBranch,status,conclusion,createdAt
    if ($LASTEXITCODE -ne 0) { throw '查询 workflow run 失败' }
    @($json | ConvertFrom-Json) | Where-Object headBranch -EQ $branch | Sort-Object createdAt
}

# 触发之后 run 要过几秒才出现在列表里。只认 $exclude 之外的新 run——
# 同一提交上旧的失败 run 满足其它所有条件，不排除就会被错认成刚触发的这次
function Find-NewRun([string]$event, [string]$sha, [string]$branch, [string[]]$exclude) {
    for ($i = 0; $i -lt 30; $i++) {
        $hit = Get-Runs $event $sha $branch | Where-Object { [string]$_.databaseId -notin $exclude } |
            Select-Object -Last 1
        if ($hit) { return [string]$hit.databaseId }
        Start-Sleep 5
    }
    throw "等了 150 秒也没找到新的 $event run（提交 $sha、分支 $branch）。master 可能在推送后又动了，或 dispatch 没生效"
}

# 把文件里恰好一处 $pattern 的版本号换掉；匹配数不是 1 就失败，宁可停也不猜
function Set-VersionIn([string]$path, [string]$pattern, [string]$from, [string]$to) {
    # [IO.File] 认的是 .NET 进程的当前目录，Set-Location 改不到它：相对路径会读写到
    # 启动脚本时所在的目录（实测踩过）。一律拼成仓库根下的绝对路径
    $full = Join-Path $script:root $path
    $text = [IO.File]::ReadAllText($full)
    $re = [regex]::new($pattern.Replace('{V}', [regex]::Escape($from)), 'Multiline')
    $n = $re.Matches($text).Count
    if ($n -ne 1) { throw "$path 里匹配到 $n 处版本号 $from（期望恰好 1 处），不改" }
    if (-not $script:DryRun -and $from -ne $to) {
        $new = $re.Replace($text, { param($m) $m.Groups[1].Value + $to + $m.Groups[2].Value }, 1)
        [IO.File]::WriteAllText($full, $new, [Text.UTF8Encoding]::new($false))
    }
    Write-Host ("  {0,-28} {1} -> {2}" -f $path, $from, $to)
}

# 发版提交例外只许改版本号：四个版本文件必须恰好各一行增一行删，许可清单随依赖变化可以多改；
# 除此之外任何改动都不该搭车（cargo 改写了 Cargo.lock、手工提交借用了发版标题都算）
function Assert-ReleaseOnly([string[]]$numstat) {
    foreach ($line in $numstat) {
        $added, $deleted, $path = $line -split "`t", 3
        if ($path -eq $script:notices) { continue }
        if ($path -notin $script:versionFiles) { throw "发版提交里有例外之外的文件：$path" }
        if ($added -ne '1' -or $deleted -ne '1') { throw "$path 的改动不只是一行版本号（+$added -$deleted）" }
    }
}

# 远端 tag 指向的提交（附注 tag 取 ^{} 解引用后的那一行）；不存在时返回空
function Get-RemoteTagCommit([string]$tag) {
    $lines = Invoke-Checked '查询远端 tag' { git ls-remote origin "refs/tags/$tag" "refs/tags/$tag^{}" }
    $peeled = $lines | Where-Object { $_ -match "`trefs/tags/$([regex]::Escape($tag))\^\{\}$" }
    $plain = $lines | Where-Object { $_ -match "`trefs/tags/$([regex]::Escape($tag))$" }
    $line = if ($peeled) { $peeled } elseif ($plain) { $plain } else { $null }
    if ($line) { ($line -split "`t")[0] } else { '' }
}

# 等 tag 构建、按需转正式发布。tag 已推送后的所有续跑都走这里
function Complete-Release([string]$tag, [string]$sha) {
    Step "正式构建（$tag @ $($sha.Substring(0, 8))）"
    $runs = @(Get-Runs 'push' $sha $tag)
    $run = if ($runs) { [string]$runs[-1].databaseId } else { Find-NewRun 'push' $sha $tag @() }
    Wait-Run $run '正式构建'

    $url = "https://github.com/$script:repo/releases/tag/$tag"
    if (-not $Publish) {
        Write-Host "`ndraft 已建好，过目后在页面上点发布：$url" -ForegroundColor Green
        return
    }
    Step '转为正式发布'
    $isDraft = Invoke-Checked '查询 Release' { gh release view $tag --json isDraft --jq .isDraft }
    if ($isDraft -eq 'true') {
        Invoke-Checked 'gh release edit' { gh release edit $tag --draft=false --latest }
    } else {
        Write-Host '已经是正式发布，跳过'
    }
    gh release view $tag --json assets --jq '.assets[] | "  " + .name + "  " + .url'
    Write-Host "`n已发布：$url" -ForegroundColor Green
}

# ---------------------------------------------------------------------------

if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw "版本号要形如 0.7.3：$Version" }
$tag = "v$Version"
$releaseSubject = "chore(release): 版本号升到 $Version"
$files = @(
    @{ Path = 'package.json'; Pattern = '^(\s*"version":\s*"){V}(")' }
    @{ Path = 'src-tauri/tauri.conf.json'; Pattern = '^(\s*"version":\s*"){V}(")' }
    # 只认 [package] 段里的那一行，依赖表里的 version = 不碰
    @{ Path = 'src-tauri/Cargo.toml'; Pattern = '(?s)(\[package\][^\[]*?^version = "){V}(")' }
    @{ Path = 'src-tauri/Cargo.lock'; Pattern = '(^name = "app"\r?\nversion = "){V}(")' }
)
$script:versionFiles = $files | ForEach-Object { $_.Path }
$script:notices = 'THIRD_PARTY_NOTICES.md'

# 不用 `git rev-parse --show-toplevel`：它的输出按控制台代码页解码，中文路径会乱码
$script:root = Split-Path $PSScriptRoot -Parent
Set-Location $script:root
# .NET 文件 API 与子进程按进程当前目录解析相对路径，和 PowerShell 的位置对齐
[Environment]::CurrentDirectory = $script:root

Step '检查前置条件'
Invoke-Checked 'gh 登录检查（先 gh auth login）' { gh auth status *> $null }
$originUrl = Invoke-Checked '读取 origin 地址' { git remote get-url origin }
# gh 子命令操作的仓库来自 GH_REPO 或 `gh repo set-default`，都可能不是 origin；
# 不一致就会推一个仓库、在另一个仓库上预检和发版
$script:repo = if ($env:GH_REPO) { $env:GH_REPO } else {
    Invoke-Checked '读取仓库名' { gh repo view --json nameWithOwner --jq .nameWithOwner }
}
if (($originUrl -replace '\\', '/') -notmatch "[/:]$([regex]::Escape($script:repo))(\.git)?/?$") {
    throw "gh 操作的仓库 $script:repo 与 origin（$originUrl）不是同一个"
}

$branch = Invoke-Checked '读取当前分支' { git symbolic-ref --short -q HEAD }
if ($branch -ne 'master') { throw "要在 master 上发版，当前是 $branch" }
$dirty = Invoke-Checked 'git status' { git status --porcelain }
if ($dirty) { throw "工作区不干净，先处理：`n$($dirty -join "`n")" }
Invoke-Checked 'git fetch' { git fetch -q origin master }

# --- tag 已经推上去：前面的步骤都做过了，只剩等构建 / 转正式发布 ---
$remoteTagSha = Get-RemoteTagCommit $tag
if ($remoteTagSha) {
    Invoke-Checked "确认 $tag 在 master 上" { git merge-base --is-ancestor $remoteTagSha origin/master }
    Write-Host "origin 上已有 $tag（@ $($remoteTagSha.Substring(0, 8))），从等待正式构建续上"
    if ($DryRun) { Write-Host "`n[DryRun] 到此为止"; exit 0 }
    Complete-Release $tag $remoteTagSha
    exit 0
}

# --- 本地 master 与 origin 的关系 ---
$ahead = [int](Invoke-Checked '比对 origin/master' { git rev-list --count origin/master..HEAD })
$behind = [int](Invoke-Checked '比对 origin/master' { git rev-list --count HEAD..origin/master })
$pendingPush = $false
if ($ahead -gt 0) {
    $subject = Invoke-Checked '读取 HEAD 标题' { git log -1 --format=%s HEAD }
    if ($ahead -eq 1 -and $subject -eq $releaseSubject) {
        if ($behind -gt 0) {
            throw "上次的版本号提交没推上去，而 origin/master 已经前进了 $behind 个提交。" +
                '确认那个发版提交里没有别的东西后，把本地 master 对齐 origin/master（会丢弃那个本地提交）再重跑'
        }
        # 标题对得上不够：手工提交也可能借用这个标题。内容同样按发版提交例外核一遍
        Assert-ReleaseOnly (Invoke-Checked '核对未推送的发版提交' { git diff --numstat HEAD~1 HEAD })
        Write-Host '上次的版本号提交还没推上去，这次接着推'
        $pendingPush = $true
    } else {
        # 领先的提交没经过 review，不能跟着发版带出去
        throw "本地 master 比 origin 多 $ahead 个提交，先确认它们该不该发"
    }
} elseif ($behind -gt 0) {
    if ($DryRun) { Write-Host "[DryRun] 实际运行时会把本地 master 快进 $behind 个提交到 origin/master" }
    else { Invoke-Checked '快进到 origin/master' { git merge -q --ff-only origin/master } }
}

# 远端没有这个 tag，本地却有：只有「上次打好了没推上去」这一种合法情况
$localTagSha = git rev-parse -q --verify "refs/tags/$tag^{commit}"
if ($LASTEXITCODE -ne 0) { $localTagSha = '' }
if ($localTagSha -and -not $pendingPush) {
    $head = Invoke-Checked '读取 HEAD' { git rev-parse HEAD }
    if ($localTagSha -ne $head) {
        throw "本地已有 $tag，但指向 $localTagSha 而不是 HEAD。它从没推上去过，确认后删掉本地 tag 再重跑"
    }
}

# --- 版本号 ---
Step "版本号 -> $Version"
$current = (Get-Content package.json -Raw | ConvertFrom-Json).version
if ($DryRun -and $behind -gt 0) {
    Write-Host "[DryRun] 本地落后 origin，下面按本地的 $current 推算；实际运行时以快进后的版本为准" -ForegroundColor Yellow
}
# 本地 tag 打在 HEAD 上、版本号却还要 bump：bump 会产生新提交，tag 必然对不上。
# 在这里就停，别等 40 分钟预检跑完才发现
if ($localTagSha -and $current -ne $Version) {
    throw "本地已有 $tag（指向 HEAD），但版本号还是 $current。确认后删掉本地 tag 再重跑"
}

if ($current -eq $Version) {
    # 重跑：版本号已经是目标值（已提交，可能还没推）。核对四处一致
    foreach ($f in $files) { Set-VersionIn $f.Path $f.Pattern $Version $Version }
    Write-Host "版本号已经是 $Version，跳过 bump"
    if (-not $pendingPush -and $ahead -eq 0) {
        # 版本号已在 origin/master 上：要么上次推完就停了，要么有人手工改过。确认那是发版提交
        $last = Invoke-Checked '查找发版提交' { git log -1 --format=%s -S "`"version`": `"$Version`"" -- package.json }
        if ($last -ne $releaseSubject) { Write-Host "注意：引入 $Version 的提交标题是「$last」，不是本脚本的发版提交" -ForegroundColor Yellow }
    }
} else {
    if ($pendingPush) { throw "本地未推送的发版提交与 -Version $Version 对不上（package.json 是 $current）" }
    if ([version]$Version -le [version]$current) { throw "新版本 $Version 不大于当前 $current" }
    foreach ($f in $files) { Set-VersionIn $f.Path $f.Pattern $current $Version }

    if ($DryRun) { Write-Host "`n[DryRun] 到此为止，没有写任何文件"; exit 0 }

    try {
        Invoke-Checked '重新生成许可清单' { node scripts/gen-notices.mjs }
        Invoke-Checked '许可清单校验' { node scripts/gen-notices.mjs --check }

        # 本地先拦一道快的：钉了 rust-toolchain.toml 之后，这里和 CI 用的是同一个 clippy
        Step '本地 cargo fmt --check / clippy'
        Push-Location src-tauri
        try {
            Invoke-Checked 'cargo fmt --check' { cargo fmt --check }
            Invoke-Checked 'cargo clippy' { cargo clippy --all-targets -- -D warnings }
        } finally { Pop-Location }

        Step '提交版本号'
        Invoke-Checked 'git add' { git add -- $script:versionFiles $script:notices }
        Assert-ReleaseOnly (Invoke-Checked 'git diff --cached --numstat' { git diff --cached --numstat })
        Invoke-Checked 'git commit' { git commit -q -m $releaseSubject }
    } catch {
        # 不替用户撤销：只提示。开跑前已确认工作区干净，这些文件里只有本脚本的改动
        Write-Host "`n版本号改动还留在工作区或暂存区。修好问题后先撤掉再重跑：" -ForegroundColor Yellow
        Write-Host "  git restore --staged --worktree $($script:versionFiles -join ' ') $script:notices"
        throw
    }
    $pendingPush = $true
}

if ($DryRun) { Write-Host "`n[DryRun] 到此为止"; exit 0 }

if ($pendingPush) {
    Step '推送版本号'
    Invoke-Checked 'git push master' { git push -q origin master }
}

# 预检、打 tag 都钉在这个提交上，之后 HEAD 怎么动都不影响
$sha = Invoke-Checked '读取 HEAD' { git rev-parse HEAD }

# --- 预检 ---
Step "CI 预检（workflow_dispatch @ $($sha.Substring(0, 8))）"
# 同一提交上已经在 master 跑过的预检：成功的直接复用，进行中的接着等
$dispatchRuns = @(Get-Runs 'workflow_dispatch' $sha 'master')
$reuse = $dispatchRuns | Where-Object conclusion -EQ 'success' | Select-Object -Last 1
if (-not $reuse) { $reuse = $dispatchRuns | Where-Object status -NE 'completed' | Select-Object -Last 1 }
if ($reuse) {
    $preflight = [string]$reuse.databaseId
    Write-Host "同一提交已有预检 run $preflight（$($reuse.status) $($reuse.conclusion)），直接复用"
} else {
    # `--ref master` 构建的是触发那一刻的 master；推送之后 master 若又动了，预检的就不是 $sha
    $remoteMaster = (Invoke-Checked '查询远端 master' { git ls-remote origin refs/heads/master }) -split "`t" | Select-Object -First 1
    if ($remoteMaster -ne $sha) { throw "origin/master（$remoteMaster）已经不是 $sha，master 在推送后又动了。拉取后用新版本号或等它稳定再重跑" }
    $before = @($dispatchRuns | ForEach-Object { [string]$_.databaseId })
    Invoke-Checked '触发预检' { gh workflow run release.yml --ref master }
    $preflight = Find-NewRun 'workflow_dispatch' $sha 'master' $before
}
Wait-Run $preflight '预检'

# --- 打 tag ---
Step "打 tag $tag 并推送"
$head = Invoke-Checked '读取 HEAD' { git rev-parse HEAD }
if ($head -ne $sha) { Write-Host "注意：等预检期间 HEAD 动了，tag 仍打在通过预检的 $sha 上" -ForegroundColor Yellow }
if ($localTagSha) {
    if ($localTagSha -ne $sha) { throw "本地 $tag 指向 $localTagSha，不是通过预检的 $sha" }
    Write-Host '本地 tag 上次已打好，直接推送'
} else {
    Invoke-Checked 'git tag' { git tag -a $tag -m $tag $sha }
}
Invoke-Checked 'git push tag' { git push -q origin "refs/tags/$tag" }

Complete-Release $tag $sha
