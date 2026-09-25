# Phase 2c「更新」（sync_in）手工测试指引

**对应分支：`worktree-svn-phase2c-sync-in`。** 配套
`docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in.md`
与同目录的 `-decisions.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。
> 所有 PowerShell 代码块都能**原样复制粘贴**，不需要手工替换任何占位符。

## 这一期验的是什么

SVN 混合仓库「能更新」：工具栏上点一次「更新」，走完「收起未提交改动 → 切到基线 →
`svn update` → 录新基线 → 切回分支 → rebase → 放回改动」。冲突时停在可解决的状态，
顶部横幅给「继续 / 放弃」。早上在 TortoiseSVN 里对项目根点过 update 之后，点「更新」
会先把基线跟上、再把服务器的改动与你自己的改动重新分开。

四个新 Tauri 命令（`svn_sync_in_preview` / `svn_sync_in` / `svn_sync_continue` /
`svn_sync_abort`）的参数名 `snake_case` ↔ `camelCase` 转换**没有编译期检查**
（`AGENTS.md` §7.3）。场景 1、2 走通就证明四个 invoke 都接对了。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | 见决策日志「全量验证」表 |
| 下面 8 个真实窗口场景 | ⬜ **待跑** |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [ ] 场景 1：普通更新——服务器改动到了磁盘上，提交与未提交改动都在，没有横幅
- [ ] 场景 2：rebase 冲突 → 暂存 → 继续
- [ ] 场景 3：rebase 冲突 → 放弃，回到更新前
- [ ] 场景 4：服务器新增了与你的未跟踪文件同名的文件 → 一个字节都不放回 → 挪走后继续
- [ ] 场景 5：崩溃后重开 → 横幅上「继续」
- [ ] 场景 6：分支上有 merge commit → 预览就拒绝
- [ ] 场景 7：早上先在项目根外部 update，再点「更新」→ 黄色横幅 →「更新…」→ 重新分开
- [ ] 场景 8：美术文件本地改过又被服务器改 → 确认框红色警告 → update 冲突 → 回到原分支

---

## 0. 准备

整个过程开**两个** PowerShell 窗口 + 应用本身，下文按用途称呼：

| 窗口 | 用途 |
|---|---|
| **应用窗口** | 跑 `pnpm tauri dev`，别的不干 |
| **操作窗口** | 造 fixture、扮演同事提交、在 `wc1` 里改文件、查结果 |

⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，这份清单里的 git / svn 写操作（包括场景 3、
6、7 收尾里的 `reset --hard`）**只能在临时 fixture 里做**。不要拿任何真实的 SVN
工作副本来跑这份清单。

### 0.1 起应用（应用窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2c-sync-in
pnpm tauri dev
```

> ⚠️ **必须在这个 worktree 里跑，不是主仓库。** 主仓库的 `master` 上没有这一期的代码。

### 0.2 造 fixture（操作窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2c-sync-in
pwsh -File scripts\e2e-svn-fixture.ps1
```

脚本在 `$env:TEMP\phase1c_xxxxxxxx\` 下建 SVN 仓库、你的工作副本 `wc1` 和同事的
工作副本 `wc2`，最后打印 `wc1` 的路径。服务器停在 r1。

### 0.3 初始化成混合仓库（应用里）

在应用里打开上一步打印的 `...\wc1`，走完初始化向导：**白名单填 `Code/`，扩展名用
默认**。走完之后停在 `work` 分支上，工具栏上应当有一个 **「更新」** 按钮。

### 0.4 定位 fixture、定义辅助函数（操作窗口，一次性粘贴）

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
if (-not $fix) { throw '没找到 fixture，先做 0.2' }
$WC = Join-Path $fix.FullName 'wc1'
$WC2 = Join-Path $fix.FullName 'wc2'
if (-not (Test-Path (Join-Path $WC '.git'))) { throw "$WC 下还没有 .git，先做 0.3" }
function Colleague([string]$rel, [string]$content) {
    svn update -q --non-interactive $WC2
    $p = Join-Path $WC2 $rel
    $isNew = -not (Test-Path $p)
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    [IO.File]::WriteAllText($p, $content)
    if ($isNew) { svn add -q --parents --non-interactive $p }
    svn commit -q --non-interactive -m "colleague $rel" $WC2
    if ($LASTEXITCODE -ne 0) { throw 'svn commit 失败' }
}
function Wc([string]$rel, [string]$content) {
    [IO.File]::WriteAllText((Join-Path $WC $rel), $content)
}
function G { git -C $WC @args; if ($LASTEXITCODE -ne 0) { throw "git $args 失败" } }
"WC = $WC"; "WC2 = $WC2"
```

- `Colleague` = 同事在 `wc2` 里改（或新建）一个文件并提交，服务器前进一个 revision。
- `Wc` = 在你的工作副本里写一个文件。用 `[IO.File]::WriteAllText` 是为了不写 BOM。
- `G` = 在 `wc1` 里跑 git，失败就抛错。

> 这些定义只在当前 PowerShell 会话里有效。**操作窗口别关。**

### 0.5 界面怎么「自己变」

| 变化 | 界面靠什么发现 | 应当在多久内反映出来 |
|---|---|---|
| 更新 / 继续 / 放弃结束 | 后端结束时补发一次 `repo-changed` | 立刻 |
| 操作窗口里改文件、写 journal、外部 `svn update` | 文件监听（300ms 去抖） | 约 1 秒 |

---

## 场景 1：普通更新

**造现场（操作窗口）：**

```powershell
Colleague 'Code/A.cs' 'class A { int s1; }'
Wc 'Code/Deep/D.cs' 'class D { int local; }'
G commit -qam 'local D'
Wc 'Code/Deep/D.cs' 'class D { int local; int wip; }'
Wc 'Code/New.cs' 'class New {}'
```

**操作（应用）：** 点工具栏的「更新」→ 确认框 → 点「更新」。

**应该看到：**

- 确认框正文写着 **「把 r1 更新到 r2」**，并提到分支 `work`；底部说明提到「已暂存」的状态不会保留；
- 点完之后框里显示「正在更新…」，几秒内框关掉，**没有任何横幅、没有结果弹框**（成功不打扰）；

**核对（操作窗口）：**

```powershell
Get-Content (Join-Path $WC 'Code/A.cs')
Get-Content (Join-Path $WC 'Code/Deep/D.cs')
Get-Content (Join-Path $WC 'Code/New.cs')
G log --oneline -2 svn/base
G stash list
Test-Path (Join-Path $WC '.git\mygitclient-sync.json')
```

依次应当是：`class A { int s1; }`；`class D { int local; int wip; }`；`class New {}`；
第一行是 `svn: r2`；`stash list` 里没有 `mygitclient-sync`；`False`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 工具栏上没有「更新」按钮 | `showsSvnUpdate` 没接上，或 `capabilities.vcs` 不是 `svn` |
| 点了「更新」没有任何反应 | `svn_sync_in_preview` 的 invoke 名或参数名写错了——打开 devtools 看 console |
| 确认框点「更新」之后报参数缺失 / 反序列化失败 | `expect` 的字段名（`origHead` / `baselineOid` / `confirmedOutside`）与后端对不上 |
| 更新完出现红色「上次同步没有正常结束」横幅 | S11 没删 journal |
| `D.cs` 变回 `class D { int local; }` 或 `New.cs` 不见了 | S10 没把 stash 放回来 |

**收尾（操作窗口）：** 把留下的未提交改动提交掉，下一个场景从干净状态开始。

```powershell
G add Code/New.cs
G commit -qam 'e2e 1 leftovers'
```

---

## 场景 2：rebase 冲突 → 继续

**造现场（操作窗口）：**

```powershell
Colleague 'Code/A.cs' 'class A { int s2; }'
Wc 'Code/A.cs' 'class A { int mine2; }'
G commit -qam 'local A'
```

**操作（应用）：** 点「更新」→ 点「更新」。

**应该看到：**

- 弹出 **「更新遇到冲突」**，列出 `Code/A.cs`，说明提到横幅上的「继续」「放弃」；点「知道了」；
- 顶部一条**红色**横幅「上次同步没有正常结束」，说明写着停在 git 自己的 rebase 冲突态上，
  按钮是 **「我已解决，继续」「放弃，回到更新前」**；
- 「本地改动」里能看到 `Code/A.cs` 的冲突。

**接着（操作窗口）：** 手工把冲突解掉。

```powershell
Wc 'Code/A.cs' 'class A { int s2; int mine2; }'
```

**接着（应用）：** 在「本地改动」里**暂存** `Code/A.cs`（这一步必须成功——冲突期间锁是放开的），
然后点横幅上的「我已解决，继续」→ 确认框 → 点「继续」。

**应该看到：** 框关掉、横幅消失、没有结果弹框。

```powershell
Get-Content (Join-Path $WC 'Code/A.cs')
G log --oneline -1 svn/base
```

应当是 `class A { int s2; int mine2; }` 和 `svn: r3`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 冲突之后暂存被拒，报「已经有一次同步正在进行」 | 冲突出口没有释放锁 |
| 横幅上没有按钮 | `syncActions` 没接到 `describeUnfinishedSync`，或 `BaselineBanner` 没渲染 `unfinished.actions` |
| 点「我已解决，继续」没反应 | `svn_sync_continue` 的 invoke 写错了——看 devtools console |
| 继续之后横幅不消失 | journal 没删，或 `afterSync` 没刷新 `svnSyncState` |

---

## 场景 3：rebase 冲突 → 放弃

**造现场（操作窗口）：**

```powershell
Colleague 'Code/A.cs' 'class A { int s3; }'
Wc 'Code/A.cs' 'class A { int mine3; }'
G commit -qam 'local A3'
$before = (git -C $WC rev-parse HEAD).Trim()
```

**操作（应用）：** 点「更新」→ 点「更新」→「更新遇到冲突」点「知道了」→ 点横幅上的
「放弃，回到更新前」→ 确认框（红色，说明写着基线保持新的、分支会落后于它）→ 点「放弃」。

**应该看到：** 弹出 **「已放弃这次更新」**；点「知道了」后横幅消失。

```powershell
(git -C $WC rev-parse HEAD).Trim() -eq $before
Get-Content (Join-Path $WC 'Code/A.cs')
G log --oneline -1 svn/base
```

应当是 `True`、`class A { int mine3; }`、`svn: r4`（基线已是新的，分支落后于它——
Phase 1 就有的合法状态）。

**收尾（操作窗口）：** 分支上那笔 `mine3` 与服务器的 `s3` 冲突，留着的话下一个场景会先撞上
它。把分支放回基线上（**只在 fixture 里**）：

```powershell
G reset -q --hard svn/base
```

---

## 场景 4：撞名

**造现场（操作窗口）：**

```powershell
Colleague 'Code/N.cs' 'class N { int server; }'
Wc 'Code/N.cs' 'class N { int mine; }'
Wc 'Code/Deep/D.cs' 'class D { int wip4; }'
```

**操作（应用）：** 点「更新」→ 点「更新」。

**应该看到：** 弹出 **「未提交的改动还没放回来」**，列出 `Code/N.cs`，说明写着一个字节都
还没放回、备份在 stash 里（带一串 10 位的 OID）；点「知道了」。顶部红色横幅，按钮是
「我已处理，继续」「不再自动放回」。

```powershell
Get-Content (Join-Path $WC 'Code/N.cs')
Get-Content (Join-Path $WC 'Code/Deep/D.cs')
```

应当是 `class N { int server; }`（服务器的）和 `class D {}`——**`D.cs` 还不是 `wip4`**，
一个字节都没放回。

**接着（操作窗口）：** 把服务器那份挪走。

```powershell
Move-Item (Join-Path $WC 'Code/N.cs') (Join-Path $WC 'Code/N.server.txt')
```

**接着（应用）：** 点「我已处理，继续」→ 点「继续」。

```powershell
Get-Content (Join-Path $WC 'Code/N.cs')
Get-Content (Join-Path $WC 'Code/Deep/D.cs')
G stash list
```

应当是 `class N { int mine; }`、`class D { int wip4; }`，`stash list` 里没有 `mygitclient-sync`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 没弹「还没放回来」，而是「放回未提交的改动时有冲突」 | 撞名预检没生效，git 已经半套 apply 了 |
| 弹了「还没放回来」但 `D.cs` 已经是 `wip4` | 预检在 apply 之后才做——半套 apply |

**收尾（操作窗口）：**

```powershell
Remove-Item (Join-Path $WC 'Code/N.server.txt')
G commit -qam 'e2e 4 leftovers'
```

---

## 场景 5：崩溃后继续

模拟「压完 stash、还没切分支就崩了」：手工造出 phase `stashed` 的现场，形状与应用自己
写出来的一致。

**造现场（操作窗口）：**

```powershell
Colleague 'Code/A.cs' 'class A { int s5; }'
Wc 'Code/Deep/D.cs' 'class D { int crashed; }'
$head = (git -C $WC rev-parse HEAD).Trim()
$base = (git -C $WC rev-parse svn/base).Trim()
$rev = [int]([regex]::Match((git -C $WC log -1 --format=%B svn/base | Out-String), '@(\d+) ').Groups[1].Value)
G stash push -u -m 'mygitclient-sync:manual5' -- Code
$oid = (git -C $WC rev-parse refs/stash).Trim()
G update-ref refs/mygitclient/sync/manual5 $oid
$j = [ordered]@{
    opid = 'manual5'; flow = 'sync_in'; phase = 'stashed'
    startedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    origBranch = 'work'; origHead = $head; stashOid = $oid
    baselineRevBefore = $rev; baselineOidBefore = $base
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText((Join-Path $WC '.git\mygitclient-sync.json'), $j)
```

**应该看到（应用，不重开标签页）：** 约 1 秒内顶部出现红色横幅「上次同步没有正常结束」，
说明写着改动已经压进 stash、东西没丢，按钮是「继续」「放弃，回到更新前」。

**操作（应用）：** 点「继续」→ 点「继续」。

```powershell
Get-Content (Join-Path $WC 'Code/Deep/D.cs')
Get-Content (Join-Path $WC 'Code/A.cs')
Test-Path (Join-Path $WC '.git\mygitclient-sync.json')
```

应当是 `class D { int crashed; }`、`class A { int s5; }`、`False`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 横幅要重开标签页才出现 | journal 的写入没触发刷新（2b 的接线） |
| 点「继续」报「分支或基线被改动过」 | 造现场之后工作区又被改过；或 `resume_stashed` 的复核判错了 |

**收尾（操作窗口）：**

```powershell
G commit -qam 'e2e 5 leftovers'
```

---

## 场景 6：前置拒绝——分支上有 merge commit

**造现场（操作窗口）：**

```powershell
G checkout -q -b side
Wc 'Code/Side.cs' 'class S {}'
G add Code/Side.cs
G commit -qm side
G checkout -q work
G merge --no-ff -m 'merge side' side
```

**操作（应用）：** 点「更新」。

**应该看到：** 不弹确认框，顶部红色错误条直接写着「基线之后有 merge commit」并列出 OID。

```powershell
G log --oneline -1
```

仍是 `merge side`；没有「上次同步没有正常结束」横幅。

---

## 场景 7：早上先在项目根外部 update，再点「更新」

用户每天早上的第一件事（计划 A16 / A17）。

**造现场（操作窗口）：** 先撤掉场景 6 的 merge（**只在 fixture 里**），再准备本地提交、
未提交改动、同事的新文件与美术改动，最后模拟 TortoiseSVN 对项目根点 update。

```powershell
G reset -q --hard HEAD~1
Wc 'Code/Deep/D.cs' 'class D { int c7; }'
G commit -qam 'local D7'
Wc 'Code/W7.cs' 'class W7 {}'
G add Code/W7.cs
G commit -qm 'add W7'
Wc 'Code/W7.cs' 'class W7 { int wip7; }'
Colleague 'Code/S7.cs' 'class S7 {}'
Colleague 'Art/boss.psd' 'art7'
svn update -q --non-interactive $WC
```

**应该看到（应用，不重开标签页）：** 几秒内顶部出现**黄色**横幅 **「工作副本已在外部更新」**，
按钮依次是 **「更新…」「重建基线…」「体检基线」**。此刻：

```powershell
G status --short
```

同时列出 ` M Code/W7.cs` 与 `?? Code/S7.cs`——服务器的新文件和你自己的改动混在一起。

**操作（应用）：** 点横幅上的「更新…」。

**应该看到：** 确认框第一段写着 **「已经在外部（TortoiseSVN）更新到了 r…」** 和 **「重新分开」**
→ 点「更新」→ 框关掉、黄色横幅消失。

```powershell
G status --short
Get-Content (Join-Path $WC 'Code/S7.cs')
Get-Content (Join-Path $WC 'Art/boss.psd')
G log --oneline -2 svn/base
G stash list
```

`status` **只剩** ` M Code/W7.cs`；`class S7 {}`；`art7`；`svn/base` 两条都是 `svn: r…`
且 revision 相同（接管录一条、S7 再录一条）；`stash list` 里没有 `mygitclient-sync`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 横幅是红色「基线落后于工作副本」，没有「更新…」 | `describeSync` 没按 A16 改 |
| 点「更新…」没反应 | `BaselineBanner` 没接 `svnUpdate` |
| 点「更新」被拒，报「基线已经失同步」 | `check_sync_in` 没放行整体前进 |
| 停在「未提交的改动还没放回来」并列出 `Code/S7.cs` | A17（三处同一个 blob 不算撞名）没生效 |
| 更新完 `G status --short` 里还有 `Code/S7.cs` 或别的服务器改动 | 接管没重录基线，服务器改动仍被当成本地修改 |

**收尾（操作窗口）：**

```powershell
G commit -qam 'e2e 7 leftovers'
```

---

## 场景 8：美术文件确认 + update 冲突（最后跑）

⚠️ 这个场景会让仓库停在 SVN 冲突 + 失同步，**放在最后跑**。

**造现场（操作窗口）：**

```powershell
Wc 'Art/boss.psd' 'local'
Colleague 'Art/boss.psd' 'server'
```

**操作（应用）：** 点「更新」。

**应该看到：** 确认框底部的**红色**警告里列出 `Art/boss.psd`，说明它不归 git 管、冲突时只能
回 TortoiseSVN；「更新」按钮是危险色 → 点「更新」→ 框**不关**，里面显示错误：含
`Art/boss.psd` 与「TortoiseSVN」，末尾 **「已回到分支 `work`」**。关掉对话框。

随后顶部出现**黄色**「工作副本已在外部更新」横幅（S5 已经把工作副本更新了、基线没动）。
点横幅上的「更新…」应当**被拒绝**，错误指名 `Art/boss.psd` 的 SVN 冲突——先去 TortoiseSVN
解决，这是对的。

```powershell
svn status (Join-Path $WC 'Art/boss.psd')
G symbolic-ref --short HEAD
```

第一行以 `C` 开头；分支是 `work`。

---

## 收尾

整个 fixture 是一次性的。先在应用里关掉 `wc1` 标签页，再在**操作窗口**删掉整个目录：

```powershell
Remove-Item -Recurse -Force $fix.FullName
```

---

## 自动化测试覆盖了什么（不用手工重复）

| 场景 | 覆盖它的测试 |
|---|---|
| 普通更新、无新提交也落基线、Local-Only 提交、线性往返 | `src-tauri/tests/svn_sync_in.rs` 前 4 条 |
| 白名单在同仓库 external 里（公司项目的真实形态） | `a_whitelist_inside_a_same_repo_external_updates` |
| rebase / stash 冲突是正常出口，锁放开 | `a_rebase_conflict_stops_with_the_lock_released`、`a_stash_conflict_keeps_the_stash_and_marks_it_applied` |
| 每个 phase 崩溃后继续 / 放弃 | `svn_sync_in.rs` 的「造现场」一组（`start` 到 `applying-stash`） |
| 冲突期间分支被外面动过 / 外面 abort 了 rebase | `continue_refuses_when_the_branch_moved_during_a_conflict`、`continue_refuses_when_the_rebase_was_aborted_outside` |
| S3 被 Unity 式占用、S4 基线说谎、S5 断网、S5 文本 / 树冲突 | `a_file_held_open_during_checkout_is_named_and_rolled_back`、`a_lying_baseline_is_caught_at_s4_and_rolled_back`、`update_failure_returns_to_the_original_branch`、`an_update_conflict_outside_git_rolls_back_and_points_to_tortoisesvn`、`a_tree_conflict_rolls_back` |
| 外部整体 update 的接管、新增文件、相邻行冲突、继续到更新的 revision | `svn_sync_in.rs` 的 7A 一组 |
| stash 按 opid 找、按 OID drop、撞名预检与 A17 放宽 | `src-tauri/tests/svn_sync_stash.rs` |
| 前置检查（merge、detached、pinned、svn 冲突、断网、`.gitignore`） | `src-tauri/tests/svn_sync_check.rs` |
| IPC 字段名（含 `catchUpRev` 为 `null` 时仍发出） | `src-tauri/tests/svn_backend.rs::sync_in_types_cross_ipc_in_camel_case` |
| 三个写命令挂起 watcher、engine 入口拿锁 | `src-tauri/tests/svn_write_guard.rs::sync_commands_suspend_the_watcher_and_their_engine_entries_take_the_lock` |
| 确认框 / 结局 / 横幅文案 | `tests/vitest/svnSyncIn.test.ts`、`svnJournal.test.ts`、`svnSync.test.ts` |
