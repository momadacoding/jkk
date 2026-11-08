#Requires -Version 7.4
<#
.SYNOPSIS
  盘点已经没用的分支与 worktree：只读，只打印建议的清理命令，不执行任何一条。
.DESCRIPTION
  为什么需要它：本仓库用 squash 合并，分支上的提交永远不会出现在 master 历史里，
  `git branch --merged` 认不出来；叠加 PR（base 是另一条 PR 的分支）更只能间接进 master。
  以前每次判断「这个分支还有没有用」都要手工查 PR、逐文件比内容。

  判定为「可以清理」必须是：分支最新的 PR 已合并，并且满足其一：
    - tip 在 master（优先 origin/master）的历史里
    - tip 等于或早于 PR 合并时的 headRefOid（合并之后没有再提交；squash 合并走这条）
    - 叠加 PR：tip 同上，且 base 分支的 PR 随后合进 master 时的 headRefOid
      包含这次合并的提交
  没有已合并 PR、但 tip 在 master 历史里的分支只列为「需人工确认」：刚建还没提交的分支
  也是这个形态，不能据此判可删。
  worktree 有未提交改动（含 untracked）、被 lock（Claude Code 会给活跃会话的 worktree
  加锁）、或是当前所在的 worktree 时，不给删除命令。

  打印出的命令仍属 AGENTS.md 开头列出的破坏性操作：执行前要用户针对具体命令和目标授权。
  同一仓库有并发会话时不要批量执行（AGENTS.md §4.3）。

  需要网络与 gh 已登录（读 PR 列表、`git ls-remote`）。默认不 fetch；本地缺少判断所需的
  提交时会标「无法判断」，加 -Fetch 先跑一次 `git fetch origin`（不带 --prune）再判断。
  用 `pwsh -NoProfile -File scripts/stale-branches.ps1` 运行。
.PARAMETER Base
  主干分支名，默认 master。
.PARAMETER Fetch
  判断前先 `git fetch origin`。只更新远端跟踪分支，不删除任何引用。
.EXAMPLE
  pwsh -NoProfile -File scripts/stale-branches.ps1
#>
param(
    [string]$Base = 'master',
    [switch]$Fetch
)
$ErrorActionPreference = 'Stop'
# 下面的原生命令都自己看 $LASTEXITCODE（`merge-base --is-ancestor` 返回 1 是正常结论）
$PSNativeCommandUseErrorActionPreference = $false

function Invoke-Checked {
    param([Parameter(Mandatory)][string]$What, [Parameter(Mandatory)][scriptblock]$Cmd)
    $out = & $Cmd
    if ($LASTEXITCODE -ne 0) { throw "$What 失败（exit $LASTEXITCODE）" }
    $out
}

# Windows 上路径大小写不敏感，porcelain 输出用正斜杠：统一后再比
function Format-PathKey([string]$p) {
    [IO.Path]::GetFullPath($p).TrimEnd('\', '/').Replace('\', '/').ToLowerInvariant()
}

function Test-Commit([string]$sha) {
    if (-not $sha) { return $false }
    git cat-file -e "$sha^{commit}" 2>$null
    $LASTEXITCODE -eq 0
}

# 返回 'yes' / 'no' / 'unknown'（本地缺提交）。a 等于 b 也算 yes。
function Test-AncestorOrEqual([string]$a, [string]$b) {
    if (-not (Test-Commit $a) -or -not (Test-Commit $b)) { return 'unknown' }
    git merge-base --is-ancestor $a $b 2>$null
    switch ($LASTEXITCODE) {
        0 { 'yes' }
        1 { 'no' }
        default { throw "git merge-base --is-ancestor $a $b 失败（exit $LASTEXITCODE）" }
    }
}

# 对某个提交（本地或远端分支的 tip）给出结论：@{ Verdict = safe|keep|review|unknown; Reason = ... }
function Get-Verdict([string]$name, [string]$tip) {
    $inBase = Test-AncestorOrEqual $tip $script:baseRef
    if ($inBase -eq 'unknown') { return @{ Verdict = 'unknown'; Reason = '本地缺少这个提交，加 -Fetch 重跑' } }
    $pr = $script:prByHead[$name]
    if (-not $pr -or $pr.state -eq 'CLOSED') {
        $what = if ($pr) { "PR 第 $($pr.number) 条关闭但没合并" } else { '没有 PR' }
        # 刚建、还没提交的分支 tip 就是主干上的某个提交，同样「在主干历史里」：不能据此判可删
        if ($inBase -eq 'yes') { return @{ Verdict = 'review'; Reason = "$what，tip 在 $Base 历史里：可能是刚建还没提交的分支，也可能是直接合并的" } }
        return @{ Verdict = 'keep'; Reason = "$what，也不在 $Base 历史里" }
    }
    $tag = "PR 第 $($pr.number) 条"
    if ($pr.state -eq 'OPEN') { return @{ Verdict = 'keep'; Reason = "$tag 还开着$(if ($pr.isDraft) { '（draft）' })" } }
    if ($inBase -eq 'yes') { return @{ Verdict = 'safe'; Reason = "$tag 已合并，tip 在 $Base 历史里" } }

    switch (Test-AncestorOrEqual $tip $pr.headRefOid) {
        'no' { return @{ Verdict = 'keep'; Reason = "$tag 已合并，但这里有合并之后的新提交" } }
        'unknown' { return @{ Verdict = 'unknown'; Reason = "$tag 已合并，本地缺少 PR 的提交，加 -Fetch 重跑" } }
    }
    if ($pr.baseRefName -eq $Base) { return @{ Verdict = 'safe'; Reason = "$tag 已合并进 $Base" } }

    # 叠加 PR：合进的是另一条分支，要确认那条分支的 PR 把这次合并一起带进了主干
    $basePr = $script:prByHead[$pr.baseRefName]
    $via = "$tag 合进的是 $($pr.baseRefName)（叠加 PR）"
    if (-not $basePr -or $basePr.state -ne 'MERGED' -or $basePr.baseRefName -ne $Base) {
        return @{ Verdict = 'keep'; Reason = "$via，那条分支还没有合进 $Base 的 PR" }
    }
    switch (Test-AncestorOrEqual $pr.mergeCommit.oid $basePr.headRefOid) {
        'yes' { @{ Verdict = 'safe'; Reason = "$via，随 PR 第 $($basePr.number) 条进了 $Base" } }
        'no' { @{ Verdict = 'keep'; Reason = "$via，PR 第 $($basePr.number) 条合进 $Base 时还没包含它" } }
        default { @{ Verdict = 'unknown'; Reason = "$via，本地缺少判断所需的提交，加 -Fetch 重跑" } }
    }
}

# ---------- 收集 ----------
$current = Format-PathKey (Invoke-Checked '读取当前 worktree' { git rev-parse --show-toplevel })
if ($Fetch) { Invoke-Checked 'git fetch origin' { git fetch origin } | Out-Null }

$script:baseRef = "refs/remotes/origin/$Base"
git rev-parse --verify --quiet $script:baseRef *> $null
if ($LASTEXITCODE -ne 0) { $script:baseRef = "refs/heads/$Base" }

# worktree：第一条是主工作区
$worktrees = @()
$wt = $null
foreach ($line in (Invoke-Checked 'git worktree list' { git worktree list --porcelain })) {
    if ($line -like 'worktree *') { $wt = [ordered]@{ Path = $line.Substring(9); Branch = $null; Locked = $false; Prunable = $false }; $worktrees += $wt }
    elseif ($line -like 'branch refs/heads/*') { $wt.Branch = $line.Substring(18) }
    elseif ($line -like 'locked*') { $wt.Locked = $true }
    elseif ($line -like 'prunable*') { $wt.Prunable = $true }
}
$mainRoot = $worktrees[0].Path
$wtByBranch = @{}
foreach ($w in $worktrees) { if ($w.Branch) { $wtByBranch[$w.Branch] = $w } }

$remote = @{}
foreach ($line in (Invoke-Checked 'git ls-remote --heads origin' { git ls-remote --heads origin })) {
    $sha, $ref = $line -split "`t"
    $remote[$ref.Substring(11)] = $sha
}

# 同一个分支名可能开过多条 PR，取编号最大的那条
$prJson = Invoke-Checked 'gh pr list' {
    gh pr list --state all --limit 1000 --json number,headRefName,headRefOid,baseRefName,state,isDraft,mergeCommit
}
$script:prByHead = @{}
foreach ($pr in ($prJson | ConvertFrom-Json | Sort-Object number)) { $script:prByHead[$pr.headRefName] = $pr }

$locals = Invoke-Checked 'git for-each-ref' { git for-each-ref --format='%(refname:short)%09%(objectname)' refs/heads }

# ---------- 判定 ----------
$rows = @()
$commands = @()
foreach ($line in $locals) {
    $name, $tip = $line -split "`t"
    if ($name -eq $Base) { continue }
    $v = Get-Verdict $name $tip
    $w = $wtByBranch[$name]
    $notes = @()
    $canRemove = $v.Verdict -eq 'safe'
    if ($w) {
        $key = Format-PathKey $w.Path
        if ($key -eq (Format-PathKey $mainRoot)) { $notes += '签出在主工作区'; $canRemove = $false }
        elseif ($key -eq $current) { $notes += '是当前所在的 worktree'; $canRemove = $false }
        if ($w.Locked) { $notes += 'worktree 被 lock'; $canRemove = $false }
        if ($w.Prunable) { $notes += 'worktree 目录已不存在'; $canRemove = $false }
        elseif (Test-Path -LiteralPath $w.Path) {
            $dirty = Invoke-Checked "git status（$($w.Path)）" { git -C $w.Path status --porcelain }
            if ($dirty) { $notes += "worktree 有 $(@($dirty).Count) 处未提交改动"; $canRemove = $false }
        }
    }
    $verdict = if ($v.Verdict -eq 'safe' -and -not $canRemove) { 'blocked' } else { $v.Verdict }
    $rows += [pscustomobject]@{ Kind = 'local'; Name = $name; Verdict = $verdict; Reason = $v.Reason; Worktree = $w.Path; Notes = ($notes -join '；') }
    if ($canRemove) {
        if ($w) { $commands += "git worktree remove `"$($w.Path)`"" }
        $commands += "git branch -D $name"
    }
}

foreach ($name in ($remote.Keys | Sort-Object)) {
    if ($name -eq $Base) { continue }
    $v = Get-Verdict $name $remote[$name]
    $rows += [pscustomobject]@{ Kind = 'origin'; Name = $name; Verdict = $v.Verdict; Reason = $v.Reason; Worktree = $null; Notes = '' }
    if ($v.Verdict -eq 'safe') { $commands += "git push origin --delete $name" }
}

# 放在 worktree 惯用目录、但没登记在 git 里的目录：可能是 remove 删到一半，也可能是登记丢了的
# 工作，git 已经看不出里面有没有未提交改动，所以只列出、不给命令
$registered = @{}
foreach ($w in $worktrees) { $registered[(Format-PathKey $w.Path)] = $true }
$orphans = @()
foreach ($dir in '.claude/worktrees', '.worktrees') {
    $parent = Join-Path $mainRoot $dir
    if (-not (Test-Path -LiteralPath $parent)) { continue }
    foreach ($d in (Get-ChildItem -LiteralPath $parent -Directory -Force)) {
        if (-not $registered[(Format-PathKey $d.FullName)]) { $orphans += $d }
    }
}

# ---------- 输出 ----------
$label = @{
    safe = '可以清理'
    blocked = '分支已合并，但 worktree 现在不能删'
    review = '需人工确认'
    keep = '保留'
    unknown = '无法判断'
}
foreach ($verdict in 'safe', 'blocked', 'review', 'keep', 'unknown') {
    $group = @($rows | Where-Object Verdict -eq $verdict)
    if (-not $group) { continue }
    Write-Host "`n== $($label[$verdict])（$($group.Count)）" -ForegroundColor Cyan
    foreach ($r in $group) {
        $where = if ($r.Kind -eq 'origin') { 'origin/' } else { '' }
        Write-Host "  $where$($r.Name)"
        Write-Host "      $($r.Reason)"
        if ($r.Worktree) { Write-Host "      worktree：$($r.Worktree)" }
        if ($r.Notes) { Write-Host "      注意：$($r.Notes)" -ForegroundColor Yellow }
    }
}

if ($orphans) {
    Write-Host "`n== 不归 git 管的 worktree 目录（$($orphans.Count)）：先人工确认里面没有要留的东西" -ForegroundColor Cyan
    foreach ($d in $orphans) { Write-Host "  $($d.FullName)（最后修改 $($d.LastWriteTime.ToString('yyyy-MM-dd HH:mm'))）" }
}
$prunable = @($worktrees | Where-Object Prunable)
if ($prunable) {
    Write-Host "`n== 登记了但目录已不存在的 worktree（$($prunable.Count)）" -ForegroundColor Cyan
    foreach ($w in $prunable) { Write-Host "  $($w.Path)" }
}

$agents = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.Name -in 'claude', 'codex' })
if ($agents.Count -gt 1) {
    Write-Host "`n注意：本机有 $($agents.Count) 个 claude/codex 进程，可能有别的会话在动同一个仓库。" -ForegroundColor Yellow
}

Write-Host "`n== 建议的清理命令（只打印，不执行；在主工作区运行，执行前需要明确授权）" -ForegroundColor Cyan
if ($commands) { $commands | ForEach-Object { Write-Host "  $_" } } else { Write-Host '  （没有）' }
