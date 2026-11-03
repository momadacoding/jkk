#Requires -Version 7.0
<#
.SYNOPSIS
  发版：改版本号 → 推 master → 先在 CI 上预检（workflow_dispatch）→ 绿了才打 tag → 等正式构建。
.DESCRIPTION
  为什么要先预检：CI runner 和本机的差异（8.3 短路径的 %TEMP%、全新的 git 配置、
  SlikSvn 版本……）本地复现不了，以前只能打了 tag 才发现，挂一次就废一个版本号
  （v0.7.0、v0.7.1 都是这么废的）。预检和正式构建跑的是同一个 workflow、同一个提交，
  预检绿了，打 tag 后剩下的风险基本只有 runner 偶发故障。

  可重入：中途失败（预检挂了、网络断了）修好后，用同一个版本号重跑即可——
  版本号已经提交的跳过 bump，同一提交已有成功预检的直接复用。
  预检失败时的修复照常走 worktree + PR；合进 master 后重跑，脚本会在新的 master 上
  重新预检（版本号提交已在 master 里，不会重复 bump）。

  只改发版提交例外允许的文件（AGENTS.md §4.1）：三个清单的 version、Cargo.lock 里
  app 那一条、THIRD_PARTY_NOTICES.md。暂存区里出现其它文件直接失败。

  全程约 80 分钟（预检、正式构建各约 40 分钟）。需要 gh 已登录。
.PARAMETER Version
  要发的版本号，形如 0.7.3（不带 v）。
.PARAMETER Publish
  正式构建成功后直接把 draft Release 转成正式发布并标为 latest。
  不传时停在 draft，由人去 Release 页面过目后再点发布。
.PARAMETER DryRun
  只做检查并打印将要改的版本号，不写文件、不提交、不碰 CI。
.EXAMPLE
  pwsh scripts/release.ps1 -Version 0.7.3 -Publish
#>
param(
    [Parameter(Mandatory)][string]$Version,
    [switch]$Publish,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'

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

# 等某个 run 跑完；失败时打印失败步骤与日志摘要再抛
function Wait-Run([string]$runId, [string]$label) {
    Write-Host "$label：https://github.com/$script:repo/actions/runs/$runId"
    # 进度直接打到终端；结论另查，不靠 watch 的退出码（网络抖一下它也会非零）
    gh run watch $runId --interval 30
    $conclusion = Invoke-Checked '查询 run 结论' { gh run view $runId --json conclusion --jq .conclusion }
    if ($conclusion -ne 'success') {
        Show-RunFailure $runId
        throw "$label 未通过（$conclusion）"
    }
    Write-Host "$label 通过" -ForegroundColor Green
}

# dispatch / 推 tag 之后，run 要过几秒才出现在列表里；按提交 SHA 认领，不按「最新一条」猜
function Find-Run([string]$event, [string]$sha, [string]$branch) {
    for ($i = 0; $i -lt 30; $i++) {
        # 过滤放在 PowerShell 里做：把含引号的 jq 表达式传给原生命令，Windows 上容易被吃掉引号
        $json = gh run list --workflow release.yml --event $event --commit $sha --limit 20 `
            --json databaseId,headBranch,createdAt
        if ($LASTEXITCODE -eq 0 -and $json) {
            $hit = @($json | ConvertFrom-Json) | Where-Object headBranch -EQ $branch |
                Sort-Object createdAt | Select-Object -Last 1
            if ($hit) { return [string]$hit.databaseId }
        }
        Start-Sleep 5
    }
    throw "等了 150 秒也没找到 $event 触发、提交为 $sha 的 release workflow run"
}

# 把文件里恰好一处 $pattern 的版本号换掉；匹配数不是 1 就失败，宁可停也不猜
function Set-VersionIn([string]$path, [string]$pattern, [string]$from, [string]$to) {
    $text = [IO.File]::ReadAllText($path)
    $re = [regex]::new($pattern.Replace('{V}', [regex]::Escape($from)), 'Multiline')
    $n = $re.Matches($text).Count
    if ($n -ne 1) { throw "$path 里匹配到 $n 处版本号（期望恰好 1 处），不改" }
    if (-not $script:DryRun -and $from -ne $to) {
        $new = $re.Replace($text, { param($m) $m.Groups[1].Value + $to + $m.Groups[2].Value }, 1)
        [IO.File]::WriteAllText($path, $new, [Text.UTF8Encoding]::new($false))
    }
    Write-Host ("  {0,-28} {1} -> {2}" -f $path, $from, $to)
}

# ---------------------------------------------------------------------------

if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') { throw "版本号要形如 0.7.3：$Version" }
$tag = "v$Version"

$root = Invoke-Checked '定位仓库根' { git rev-parse --show-toplevel }
Set-Location $root

Step '检查前置条件'
Invoke-Checked 'gh 登录检查（先 gh auth login）' { gh auth status *> $null }
$script:repo = Invoke-Checked '读取仓库名' { gh repo view --json nameWithOwner --jq .nameWithOwner }

$branch = Invoke-Checked '读取当前分支' { git symbolic-ref --short -q HEAD }
if ($branch -ne 'master') { throw "要在 master 上发版，当前是 $branch" }
$dirty = Invoke-Checked 'git status' { git status --porcelain }
if ($dirty) { throw "工作区不干净，先处理：`n$($dirty -join "`n")" }

Invoke-Checked 'git fetch' { git fetch -q origin master --tags }
# 落后就快进；领先说明有没推上去的提交——那些没经过 review，不能跟着发版带出去
Invoke-Checked '快进到 origin/master' { git merge -q --ff-only origin/master }
$ahead = Invoke-Checked '比对 origin/master' { git rev-list --count origin/master..HEAD }
if ([int]$ahead -ne 0) { throw "本地 master 比 origin 多 $ahead 个提交，先确认它们该不该发" }

if (git rev-parse -q --verify "refs/tags/$tag") { throw "本地已有 tag $tag" }
$remoteTag = Invoke-Checked '查询远端 tag' { git ls-remote --tags origin "refs/tags/$tag" }
if ($remoteTag) { throw "origin 上已有 tag $tag。挂掉的版本不删 tag 重打，换下一个版本号" }

Step "版本号 -> $Version"
$current = (Get-Content package.json -Raw | ConvertFrom-Json).version
$files = @(
    @{ Path = 'package.json'; Pattern = '^(\s*"version":\s*"){V}(")' }
    @{ Path = 'src-tauri/tauri.conf.json'; Pattern = '^(\s*"version":\s*"){V}(")' }
    # 只认 [package] 段里的那一行，依赖表里的 version = 不碰
    @{ Path = 'src-tauri/Cargo.toml'; Pattern = '(?s)(\[package\][^\[]*?^version = "){V}(")' }
    @{ Path = 'src-tauri/Cargo.lock'; Pattern = '(^name = "app"\r?\nversion = "){V}(")' }
)
if ($current -eq $Version) {
    # 重跑：上次已经提交过版本号。核对另外三处也一致，再确认它确实已在 master 上
    foreach ($f in $files) { Set-VersionIn $f.Path $f.Pattern $Version $Version }
    Write-Host "版本号已经是 $Version（上次跑过），跳过 bump"
} else {
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
    } catch {
        # 不替用户撤销：只提示。开跑前已确认工作区干净，这些文件里只有本脚本的改动
        Write-Host "`n版本号改动还留在工作区。修好问题后先撤掉再重跑：" -ForegroundColor Yellow
        Write-Host '  git restore package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml src-tauri/Cargo.lock THIRD_PARTY_NOTICES.md'
        throw
    }

    Step '提交并推送版本号'
    $allowed = @('package.json', 'src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'THIRD_PARTY_NOTICES.md')
    Invoke-Checked 'git add' { git add -- $allowed }
    $staged = Invoke-Checked 'git diff --cached' { git diff --cached --name-only }
    $extra = $staged | Where-Object { $_ -notin $allowed }
    if ($extra) { throw "暂存区里有发版提交例外之外的文件：$($extra -join ', ')" }
    Invoke-Checked 'git commit' { git commit -q -m "chore(release): 版本号升到 $Version" }
    Invoke-Checked 'git push master' { git push -q origin master }
}

if ($DryRun) { Write-Host "`n[DryRun] 到此为止"; exit 0 }

$sha = Invoke-Checked '读取 HEAD' { git rev-parse HEAD }
Invoke-Checked 'git fetch' { git fetch -q origin master }
$remoteSha = Invoke-Checked '读取 origin/master' { git rev-parse origin/master }
if ($remoteSha -ne $sha) { throw "origin/master（$remoteSha）不是本地 HEAD（$sha），版本号提交没推上去或 master 又动了" }

Step "CI 预检（workflow_dispatch @ $($sha.Substring(0, 8))）"
$existing = gh run list --workflow release.yml --event workflow_dispatch --commit $sha --limit 20 `
    --json databaseId,status,conclusion `
    --jq '([.[] | select(.conclusion == "success")] | first | .databaseId) // ([.[] | select(.status != "completed")] | first | .databaseId) // empty'
if ($LASTEXITCODE -ne 0) { throw '查询已有预检 run 失败' }
if ($existing) {
    Write-Host "同一提交已有预检 run $existing，直接复用"
    $preflight = $existing
} else {
    Invoke-Checked '触发预检' { gh workflow run release.yml --ref master }
    $preflight = Find-Run 'workflow_dispatch' $sha 'master'
}
Wait-Run $preflight '预检'

Step "打 tag $tag 并推送"
Invoke-Checked 'git tag' { git tag -a $tag -m $tag }
Invoke-Checked 'git push tag' { git push -q origin $tag }

Step '正式构建'
$release = Find-Run 'push' $sha $tag
Wait-Run $release '正式构建'

$url = "https://github.com/$script:repo/releases/tag/$tag"
if ($Publish) {
    Step '转为正式发布'
    Invoke-Checked 'gh release edit' { gh release edit $tag --draft=false --latest }
    gh release view $tag --json assets --jq '.assets[] | "  " + .name + "  " + .url'
    Write-Host "`n已发布：$url" -ForegroundColor Green
} else {
    Write-Host "`ndraft 已建好，过目后在页面上点发布：$url" -ForegroundColor Green
}
