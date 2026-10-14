# Phase 3a「交付」（sync_out）手工测试指引

**对应分支：`worktree-svn-phase3a-sync-out`。** 配套
`docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out.md`
与同目录的 `-decisions.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。
> 所有 PowerShell 代码块都能**原样复制粘贴**，不需要手工替换任何占位符。

## 这一期验的是什么

SVN 混合仓库「能交付」：工具栏上点「交付」，写一段提交信息，客户端把分支相对基线的
净变化提交成**一个** SVN revision，排除标了「仅本地，不交付」的本地补丁，然后把白名单
更新到最新、录新基线、把分支（连同本地补丁）接到新基线上。中途出问题时停在可收尾的
状态，顶部横幅给「清理」「继续交付」「停止收尾」。

四个新 Tauri 命令（`svn_sync_out_preview` / `svn_sync_out` / `svn_sync_out_continue` /
`svn_sync_out_abort`）的参数名 `snake_case` ↔ `camelCase` 转换**没有编译期检查**
（`AGENTS.md` §7.3）。场景 1 走通证明前两个接对了，场景 7a / 7b 证明后两个。

交付之后的**提交图**也在这里验（场景 1、8）：已交付的提交收进 SVN 节点、SVN 主线
固定在最左一列、svn 节点画成菱形、git 与 svn 行按时间交错。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | 见决策日志「全量验证」表 |
| Task 12 冒烟（交付对话框、普通交付、本地补丁、25 个文件） | ✅ 2026-10-11 真实窗口，见决策日志 Task 12 |
| 下面 9 个真实窗口场景 | ✅ 2026-10-13～14 用户在 `pnpm tauri dev` 里跑完，全部通过（途中 4 处缺陷已修复复测，见决策日志「真实窗口 E2E」）。revision 号比下文晚：场景 4、7a 各多交付过一次，以「最新」为准 |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [x] 场景 1：普通交付——改 / 增 / 删各一，新文件在两层新目录里；图上交付的提交收进 r2
- [x] 场景 2：本地补丁夹在两个待交付提交中间（慢路径）→ 服务器上没有补丁、磁盘上补丁还在
- [x] 场景 3：待交付的改动紧挨着本地补丁 → 预览就拒绝，什么都没动
- [x] 场景 4：交付框开着时同事提交了 → 被拒 → 更新 → 再交付，提交信息还在
- [x] 场景 5：白名单里的 `.psd` 本地改过 → 红色警告 + 勾选框，不勾不能交付
- [x] 场景 6：同事在 D5 与 D6 之间改了补丁那一行 → 补丁落回冲突 → 暂存 →「我已解决，继续」
- [x] 场景 7a：提交之前进程被杀 → 重开 →「清理」
- [x] 场景 7b：提交之后进程被杀 → 重开 →「继续交付」，不重复提交
- [x] 场景 8：多分支——落后于基线的分支先被拒、更新后能交付；图上 SVN 主线不被分支拆开

> **控制台里的中文提交信息是乱码不代表存错了**：`svn log` 按系统代码页输出。要核对原文用 XML 读：
> `[xml](svn log --xml -r N $URL) | % { $_.log.logentry.msg }`（实测存的是正确的 UTF-8）。

---

## 0. 准备

整个过程开**两个** PowerShell 窗口 + 应用本身，下文按用途称呼：

| 窗口 | 用途 |
|---|---|
| **应用窗口** | 跑 `pnpm tauri dev`，别的不干 |
| **操作窗口** | 造 fixture、扮演同事提交、在 `wc1` 里改文件、查结果 |

⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，这份清单里的 git / svn 写操作（包括场景 3、5
收尾里的 `reset --hard`、`svn revert`，以及 0.5 装的 hook）**只能在临时 fixture 里做**。
不要拿任何真实的 SVN 工作副本来跑这份清单。

### 0.1 起应用（应用窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase3a-sync-out
pnpm tauri dev
```

> ⚠️ **必须在这个 worktree 里跑，不是主仓库。** 主仓库的 `master` 上没有这一期的代码。

### 0.2 造 fixture（操作窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase3a-sync-out
pwsh -File scripts\e2e-svn-fixture.ps1
```

脚本在 `$env:TEMP\phase1c_xxxxxxxx\` 下建 SVN 仓库、你的工作副本 `wc1` 和同事的
工作副本 `wc2`，最后打印 `wc1` 的路径。服务器停在 r1，上面有 `Code/A.cs`、
`Code/Deep/D.cs`、`Code/Art.psd`（白名单里但扩展名不归 git 管）、`Art/boss.psd`（白名单外）。

### 0.3 初始化成混合仓库（应用里）

在应用里打开上一步打印的 `...\wc1`，走完初始化向导：**白名单填 `Code/`，扩展名用
默认**。走完之后停在 `work` 分支上，工具栏上有 **「更新」「交付」** 两个按钮。

### 0.4 定位 fixture、定义辅助函数（操作窗口，一次性粘贴）

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
if (-not $fix) { throw '没找到 fixture，先做 0.2' }
$FIX = $fix.FullName
$WC = Join-Path $FIX 'wc1'
$WC2 = Join-Path $FIX 'wc2'
if (-not (Test-Path (Join-Path $WC '.git'))) { throw "$WC 下还没有 .git，先做 0.3" }
$URL = (svn info --show-item repos-root-url $WC).Trim()
$JOURNAL = Join-Path $WC '.git\mygitclient-sync.json'
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
    $p = Join-Path $WC $rel
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    [IO.File]::WriteAllText($p, $content)
}
function Lines { ($args -join "`n") + "`n" }
function G { git -C $WC @args; if ($LASTEXITCODE -ne 0) { throw "git $args 失败" } }
function Phase { if (Test-Path $JOURNAL) { (Get-Content $JOURNAL -Raw | ConvertFrom-Json).phase } else { '（没有 journal）' } }
"FIX = $FIX"; "URL = $URL"
```

- `Colleague` = 同事在 `wc2` 里改（或新建）一个文件并提交，服务器前进一个 revision。
- `Wc` = 在你的工作副本里写一个文件（不写 BOM，父目录不存在就建）。
- `Lines` = 把几个参数拼成多行文本，每行以 `\n` 结尾。
- `G` = 在 `wc1` 里跑 git，失败就抛错。
- `Phase` = 读 journal 里记的阶段；没有 journal 时如实说没有。

> 这些定义只在当前 PowerShell 会话里有效。**操作窗口别关。**

### 0.5 装两个 hook（操作窗口，一次性粘贴）

场景 6、7 要卡一个手点不可能卡准的时机，靠 hook 变成确定性的。两个 hook 平时什么都
不做，只在对应的 flag 文件存在时触发**一次**并删掉 flag。

```powershell
$svnExe = (Get-Command svn).Source
$bat = @"
@echo off
if not exist "$FIX\colleague.flag" goto stall
del "$FIX\colleague.flag"
"$svnExe" commit --non-interactive -m "colleague during delivery" "$WC2\Code\P.cs" > "$FIX\hook.log" 2>&1
:stall
if not exist "$FIX\stall.flag" goto end
del "$FIX\stall.flag"
type nul > "$WC\.git\e2e-stall"
:end
exit /b 0
"@
[IO.File]::WriteAllText((Join-Path $FIX 'repo\hooks\post-commit.bat'), $bat)
[IO.File]::WriteAllText((Join-Path $WC '.git\hooks\post-checkout'),
    "#!/bin/sh`nif [ -f .git/e2e-stall ]; then rm .git/e2e-stall; sleep 30; fi`n")
'hooks installed'
```

| hook | 触发条件 | 做什么 |
|---|---|---|
| SVN `post-commit`（服务器侧） | `$FIX\colleague.flag` 存在 | 替同事把 `wc2` 里**预先改好的** `Code/P.cs` 提交上去——你的交付提交一落地，同事紧接着提交，正好落在 D5 与 D6 之间 |
| 同上 | `$FIX\stall.flag` 存在 | 在 `wc1\.git\` 下放 `e2e-stall`，让**提交之后**的第一次 git checkout 卡住 |
| git `post-checkout`（`wc1`） | `wc1\.git\e2e-stall` 存在 | 卡 30 秒，给你时间结束进程 |

SVN 的 post-commit 在 `file://` 下是同步跑的：客户端要等 hook 跑完才返回，所以同事那笔
一定在 D6 之前到服务器上（2026-10-11 命令行实测）。

### 0.6 界面怎么「自己变」

| 变化 | 界面靠什么发现 | 应当在多久内反映出来 |
|---|---|---|
| 交付 / 继续 / 停止 / 清理结束 | 后端结束时补发一次 `repo-changed` | 立刻 |
| 操作窗口里改文件、提交 | 文件监听（300ms 去抖） | 约 1 秒 |

---

## 场景 1：普通交付

**造现场（操作窗口）：** 改 `A.cs`、删 `D.cs`、在两层新目录里新增 `N.cs`。

```powershell
Wc 'Code/A.cs' 'class A { int e1; }'
G rm -q Code/Deep/D.cs
Wc 'Code/N1/N2/N.cs' 'class N {}'
G add Code/N1/N2/N.cs
G commit -qam 'e2e1 改增删'
```

**操作（应用）：** 点工具栏的「交付」。

**应该看到（交付框）：**

- 标题「交付到 SVN」，正文「把分支 work 相对基线（r1）的 3 个文件变化提交到 SVN。提交之后不能撤回，会直接影响全组。」；
- 三组清单：「新增 1」`Code/N1/N2/N.cs`、「修改 1」`Code/A.cs`、「删除 1」`Code/Deep/D.cs`；
- 说明「本次没有本地补丁被排除……」；
- 提交信息框是空的，**「交付」按钮置灰**；交付执行期间工具栏的「更新」「交付」也置灰。

**接着（应用）：** 提交信息写 `e2e1 普通交付`，按 **Ctrl+Enter**。框里显示「正在交付…」，
几秒后关掉，弹出 **「已交付 r2」**，正文「分支 work 的改动都已交付，可以删除这个分支了
（不会自动删）」。点「知道了」，**没有横幅**。

**核对（操作窗口）：**

```powershell
svn log -v -r 2 --non-interactive $URL
G log --oneline -1 svn/base
(G rev-parse work) -eq (G rev-parse svn/base)
G status --short
svn status -q $WC
Phase
G for-each-ref refs/mygitclient/
```

依次应当是：

- r2 的提交信息是 `e2e1 普通交付`，变更路径正好 5 条：`M /Code/A.cs`、`D /Code/Deep/D.cs`、
  `A /Code/N1`、`A /Code/N1/N2`、`A /Code/N1/N2/N.cs`（新父目录也进了这一个 revision）；
- `svn: r2`；`True`（没有本地补丁时分支直接快进到新基线）；
- `G status` 与 `svn status -q` 都没有输出；`（没有 journal）`；`for-each-ref` 没有输出。

**核对提交图（应用）：**

- 最左一列是 **菱形** 的 r2 → r1，同一种颜色；本地提交仍是圆点；
- `e2e1 改增删` **不再单独画出来**（它的内容已经是 r2）；
- 点 r2：提交信息下面有一行灰字「本地提交：xxxxxxx e2e1 改增删」（git 提交标题与 SVN 信息
  不同才有；一样时这行不出现）；下面「相对 r1 的变化」文件树里正好是 `A.cs`、`D.cs`、`N.cs`
  三个文件，`A.cs` 的 diff 是
  `class A {}` → `class A { int e1; }`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 工具栏上没有「交付」 | `showsSvnDeliver` 没接上，或 `capabilities.vcs` 不是 `svn` |
| 点「交付」没有任何反应 | `svn_sync_out_preview` 的 invoke 名或参数名写错了——打开 devtools 看 console |
| 点框里的「交付」后报参数缺失 / 反序列化失败 | `expect` 的字段名与后端 `SyncOutExpect` 对不上 |
| r2 只有 `A /Code/N1/N2/N.cs`，没有两个目录 | D4 没有带上新父目录（服务器上会是一个「没有父目录的文件」，SVN 会直接拒绝） |
| 交付后出现红色横幅 | D10 没删 journal |
| 图上 `e2e1 改增删` 悬在 r1 旁边 | `spliceHistory` 的已交付隐藏没生效 |
| 点 r2 说「要等 Phase 4」 | r1 没有本地基线，或 `diffBase` 没算出来 |

**场景 1 的已知限制：** `Code/Deep` 目录在服务器上还在、空着（交付只删文件，不删变空的
目录）。这是本期的已知限制，不是缺陷。

---

## 场景 2：本地补丁夹在中间（慢路径）

两个待交付的提交之间夹着一个「仅本地」补丁，改的是同一个文件的不同位置。交付要把补丁
摘掉、把后一个提交重放到前一个上（对象层 `merge-tree`，不碰磁盘）。

**造现场（操作窗口）：**

```powershell
Wc 'Code/P.cs' (Lines line1 line2 line3 line4 line5 line6 line7 line8)
G add Code/P.cs
G commit -qm 'e2e2 新增 P'
Wc 'Code/P.cs' (Lines patch-local line2 line3 line4 line5 line6 line7 line8)
G commit -qam 'e2e2 本地补丁' -m 'Local-Only: true'
Wc 'Code/P.cs' (Lines patch-local line2 line3 line4 line5 line6 line7 deliver8)
G commit -qam 'e2e2 改第 8 行'
```

> `-m 'Local-Only: true'` 与在「本地改动」里勾「仅本地，不交付」写出来的提交是同一个形状。

**先看图（应用）：** `e2e2 本地补丁` 那一行带「仅本地」角标。

**操作（应用）：** 点「交付」。

**应该看到：** 清单只有「新增 1」`Code/P.cs`；说明「下面 1 个本地补丁（标了「仅本地，
不交付」）不会被提交，交付之后它们仍留在你的分支上：」，下面列着 `e2e2 本地补丁`。
提交信息写 `e2e2 补丁夹在中间`，点「交付」→ 弹出 **「已交付 r3」**，正文「分支 work 上
还有 1 个本地补丁，没有随交付删除。它们只存在于这个分支上，请保留这个分支。」——
**没有**「可以删除」的字样。

**核对（操作窗口）：**

```powershell
svn cat -r 3 "$URL/Code/P.cs"
Get-Content (Join-Path $WC 'Code/P.cs')
G log --oneline svn/base..work
G log --oneline -1 svn/base
G status --short
```

- 服务器上：第一行是 `line1`，最后一行是 `deliver8`——**没有** `patch-local`；
- 磁盘上：第一行是 `patch-local`，最后一行是 `deliver8`——补丁还在；
- `svn/base..work` 正好一行，是 `e2e2 本地补丁`（新 OID，重放过）；`svn: r3`；`G status` 无输出。

| 现象 | 含义 |
|---|---|
| 服务器上的 `P.cs` 第一行是 `patch-local` | 补丁被一起交付了——`Local-Only` 判定或重放出了错，**严重** |
| 报 `sync_out_git_too_old` | 本机 git 低于 2.40，没有 `merge-tree --merge-base`；升级 git 再测 |
| 「已交付」正文说可以删除分支 | D9 没看 `localOnlyLeft` |

---

## 场景 3：待交付的改动紧挨着本地补丁

**造现场（操作窗口）：** 在补丁（第 1 行）之上改第 2 行。摘掉补丁之后这一改动放不回去。

```powershell
Wc 'Code/P.cs' (Lines patch-local deliver2 line3 line4 line5 line6 line7 deliver8)
G commit -qam 'e2e3 紧挨着补丁'
$before = (git -C $WC rev-parse HEAD).Trim()
```

**操作（应用）：** 点「交付」。

**应该看到：** **不弹交付框**，顶部错误条：「摘掉本地补丁之后，待交付的提交 `xxxxxxx`
e2e3 紧挨着补丁 放不上去——它和下面的本地补丁改到了同一处」，接着列出那个本地补丁和
冲突文件 `Code/P.cs`，结尾「什么都没有改动，也没有碰 SVN。」

**核对（操作窗口）：**

```powershell
(git -C $WC rev-parse HEAD).Trim() -eq $before
G branch --show-current
svn status -q $WC
Phase
G for-each-ref refs/mygitclient/
svn log -q -l 1 --non-interactive $URL
```

应当是 `True`、`work`、只有 `M Code\P.cs`、`（没有 journal）`、无输出、最新仍是 r3。
`P.cs` 是 `M` 是对的：磁盘上带着本地补丁和这次的提交，和 SVN 原文本来就不同。要看的是**没有**
`A` / `D` 之类的登记残留（2026-10-13 实测，清单初稿误写成「无输出」）。

**收尾（操作窗口）：** 去掉这个放不上去的提交（**只在 fixture 里**）：

```powershell
G reset -q --hard HEAD~1
```

---

## 场景 4：交付框开着时同事提交了

**造现场（操作窗口）：**

```powershell
Wc 'Code/A.cs' 'class A { int e4; }'
G commit -qam 'e2e4 改 A'
```

**操作（应用）：** 点「交付」，提交信息写 `e2e4 消息要保留`，**先别点「交付」**。

**接着（操作窗口）：** 同事抢先提交一笔。

```powershell
Colleague 'Code/C4.cs' 'class C4 {}'
```

**接着（应用）：** 点框里的「交付」。

**应该看到：** 框**不关**，框里出现红色提示「服务器上已经有 r4，你的基线还在 r3。请先
「更新」，再交付……什么都没有改动。」点「Cancel」关掉框 → 点工具栏「更新」→ 确认框里
「把 r3 更新到 r4」→ 点「更新」→ 完成后再点「交付」：**提交信息框里还是
`e2e4 消息要保留`**。点「交付」→「已交付 r5」，仍提示 1 个本地补丁。

**核对（操作窗口）：**

```powershell
svn log -q -v -l 2 --non-interactive $URL
svn log -l 1 --non-interactive $URL
```

r4 是 `colleague Code/C4.cs`；r5 只有 `M /Code/A.cs`，提交信息是 `e2e4 消息要保留`。

| 现象 | 含义 |
|---|---|
| 框里的「交付」直接成功、r4 被覆盖 | 锁内没有重算预览（计划 B13）——**严重**，不过 SVN 本身会拒绝过期文件，先看 r5 是什么 |
| 框被关掉、提示在别处 | 执行阶段的错误没有留在框里 |
| 再开框时提交信息没了 | 草稿没有跨打开保留（`deliverDraft`） |

---

## 场景 5：白名单里的 `.psd` 本地改过

`Code/Art.psd` 在白名单目录里，但扩展名不归 git 管。交付之后要把白名单更新到最新（D6），
会走到它头上，所以交付前必须让用户知情。

**造现场（操作窗口）：**

```powershell
[IO.File]::WriteAllText((Join-Path $WC 'Code/Art.psd'), 'PSD local edit')
Wc 'Code/A.cs' 'class A { int e5; }'
G commit -qam 'e2e5 改 A'
```

**操作（应用）：** 点「交付」。

**应该看到：** 框里有**红色**警告「下面这些文件在白名单目录里，但 git 管不到（被扩展名
排除），你本地改过……勾选只表示同意更新，不会还原它们」，列出 `Code/Art.psd`，下面一个
勾选框。写好提交信息 `e2e5 美术文件` 之后，**不勾选时「交付」仍置灰**；勾上才能点。
点「交付」→「已交付 r6」。

**核对（操作窗口）：**

```powershell
Get-Content (Join-Path $WC 'Code/Art.psd')
svn status -q $WC
svn cat "$URL/Code/Art.psd"
svn log -q -v -r 6 --non-interactive $URL
```

磁盘上仍是 `PSD local edit`（没被还原）；`svn status` 显示 `M` `Code\Art.psd`；服务器上
仍是 `PSD`；r6 只有 `M /Code/A.cs`。

**收尾（操作窗口）：** 撤掉 `.psd` 的本地改动（**只在 fixture 里**）：

```powershell
svn revert -q (Join-Path $WC 'Code/Art.psd')
```

---

## 场景 6：补丁落回时冲突 → 暂存 → 继续

同事改了你的本地补丁所在的那一行，而且恰好在你的提交（D5）之后、白名单更新（D6）之前
提交。D6 把同事的改动拉下来、录进新基线，补丁接回去时就冲突了。时机由 0.5 的 SVN hook 卡。

**造现场（操作窗口）：** 同事的改动先写在 `wc2` 里，不提交；放好 flag；你这边准备一笔要交付的。

```powershell
svn update -q --non-interactive $WC2
[IO.File]::WriteAllText((Join-Path $WC2 'Code/P.cs'),
    (Lines colleague1 line2 line3 line4 line5 line6 line7 deliver8))
New-Item -ItemType File -Force (Join-Path $FIX 'colleague.flag') | Out-Null
Wc 'Code/A.cs' 'class A { int e6; }'
G commit -qam 'e2e6 改 A'
```

**操作（应用）：** 点「交付」，提交信息 `e2e6 补丁那一行被同事改了`，点「交付」。

**应该看到：**

- 弹出 **「交付成功，本地补丁落回时有冲突」**，正文说提交已在服务器上（r7）、基线也已推进，
  列出 `Code/P.cs`，让你在「本地改动」里改好并暂存后点横幅上的「继续」；点「知道了」；
- 顶部**红色**横幅「交付已提交（r7），本地补丁还没放回」，按钮 **「我已解决，继续」「停止收尾」**，
  下面一行说明两个按钮各自的后果；
- 「本地改动」里 `Code/P.cs` 是冲突状态。

**核对（操作窗口）：**

```powershell
svn log -q -l 2 --non-interactive $URL
Get-Content (Join-Path $FIX 'hook.log')
G log --oneline -1 svn/base
Phase
```

r8 是 `colleague during delivery`、r7 是你的；hook.log 以 `Committed revision 8.` 结尾；
基线是 `svn: r8`（D6 更新到的 revision）；`restoring-branch`。

**接着（操作窗口）：** 手工解冲突。

```powershell
Wc 'Code/P.cs' (Lines 'colleague1 patch-local' line2 line3 line4 line5 line6 line7 deliver8)
```

**接着（应用）：** 在「本地改动」里**暂存** `Code/P.cs`（这一步必须成功——冲突期间锁是放开的），
点横幅上的「我已解决，继续」→ 确认框「确认冲突文件都已改好并暂存。会接着把本地补丁接到
新基线上，不会重复提交。」→ 点「继续」。

**应该看到：** 弹出「已交付 r7」，仍提示 1 个本地补丁；横幅消失。

```powershell
svn log -q -l 1 --non-interactive $URL
G log --oneline svn/base..work
Get-Content (Join-Path $WC 'Code/P.cs') -TotalCount 1
Phase
```

最新仍是 r8（**没有重复提交**）；`svn/base..work` 一行（本地补丁）；第一行是
`colleague1 patch-local`；`（没有 journal）`。

| 现象 | 含义 |
|---|---|
| 没有冲突、补丁直接落回 | hook 没触发：看 `$FIX\hook.log` 是否存在、`colleague.flag` 是否还在 |
| 暂存被拒，报「已经有一次同步正在进行」 | 冲突出口没有释放锁 |
| 继续之后服务器多了一个 revision | 继续时重新提交了——**严重** |

---

## 场景 7a：提交之前进程被杀 → 清理

**造现场（操作窗口）：** 放好 git 的 flag，交付时 D3.8（第一次 checkout）会卡 30 秒。

```powershell
Wc 'Code/A.cs' 'class A { int e7; }'
G commit -qam 'e2e7 改 A'
$before = (git -C $WC rev-parse HEAD).Trim()
New-Item -ItemType File -Force (Join-Path $WC '.git\e2e-stall') | Out-Null
```

**接着（操作窗口）：先挂监视，再去点交付。** 手动在 30 秒内结束进程很难卡准（2026-10-14 实测一来一回
就超时，交付照常走完了）。这段循环等 journal 进入指定 phase、再等 2 秒确认进程卡在 hook 里，然后结束它：

```powershell
while ((Phase) -ne 'checked-out') { Start-Sleep -Milliseconds 200 }
Start-Sleep 2
Get-Process app -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -like '*svn-phase3a-sync-out*' } | Stop-Process -Force
"killed at $(Phase)"
```

**操作（应用）：** 点「交付」，提交信息 `e2e7a 不该出现在服务器上`，点「交付」。框里停在
「正在交付…」，两秒左右窗口自己消失。

应用窗口里的 `pnpm tauri dev` 会跟着退出；没退出就按 Ctrl+C。**等 30 秒**（让 hook 跑完），
然后核对：

```powershell
Phase
G branch --show-current
svn log -q -l 1 --non-interactive $URL
```

应当是 `checked-out`、空（detached 在交付树上）、最新仍是 r8。

**接着（应用窗口）：** 重新 `pnpm tauri dev`。

**应该看到：** 红色横幅 **「交付被打断了，还没提交」**，正文「SVN 上什么都没发生。为了交付，
磁盘暂时换成了要提交的版本，本地补丁暂时不在磁盘上。现在暂时停在交付用的中间位置……不要在这里
改代码。」，下面一行「『清理』切回 work，本地补丁会回到磁盘上，之后可以重新交付。」，只有一个按钮
**「清理」**。点「清理」→ 确认框 →「清理」→ 弹出「已清理」。

```powershell
G branch --show-current
(git -C $WC rev-parse HEAD).Trim() -eq $before
Get-Content (Join-Path $WC 'Code/P.cs') -TotalCount 1
svn status -q $WC
Phase
G for-each-ref refs/mygitclient/
```

`work`、`True`、`colleague1 patch-local`（补丁回到磁盘上）、只有 `M Code\A.cs` 与 `M Code\P.cs`
（还没交付的提交和本地补丁；没有 `A` / `D` 登记残留）、`（没有 journal）`、无输出。

> 实际停在哪个 phase 记进决策日志。如果不是 `checked-out`，说明 hook 卡的位置与预期不同，
> 照实记下横幅文案与按钮。

---

## 场景 7b：提交之后进程被杀 → 继续交付

**造现场（操作窗口）：** 这次放 SVN 那一侧的 flag：你的提交落地后，hook 才放 git 的 flag，
于是**提交之后**第一次 checkout（D8b 切到 `svn/base`）卡 30 秒。7a 清理掉的 `e2e7 改 A` 还在分支上，
先把它换成 7b 自己的提交——内容与名字对得上，核对时不会把两个场景搅在一起（2026-10-14 实测，
沿用 7a 的提交让用户误以为 r11 交付错了内容）：

```powershell
G reset -q --hard HEAD~1
Wc 'Code/A.cs' 'class A { int e7b; }'
G commit -qam 'e2e7b 改 A'
New-Item -ItemType File -Force (Join-Path $FIX 'stall.flag') | Out-Null
```

**接着（操作窗口）：先挂监视，再去点交付。**

```powershell
while ((Phase) -ne 'publishing-baseline') { Start-Sleep -Milliseconds 200 }
Start-Sleep 2
Get-Process app -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -like '*svn-phase3a-sync-out*' } | Stop-Process -Force
"killed at $(Phase)"
```

**操作（应用）：** 点「交付」，提交信息 `e2e7b 提交之后崩溃`，点「交付」。停在「正在交付…」，
几秒后窗口自己消失。等 30 秒（让 hook 跑完），核对：

```powershell
Phase
svn log -l 1 --non-interactive $URL
G branch --show-current
```

应当是 `publishing-baseline`、r9 的提交信息是 `e2e7b 提交之后崩溃`、`svn/base`。

**接着（应用窗口）：** 重新 `pnpm tauri dev`。

**应该看到：** 红色横幅「**交付已提交（r9），还差收尾**」，正文「work 的改动已经在 SVN 上了，
不会丢，也不会重复提交。还差收尾：切回 work，并把它更新到最新版本……现在暂时停在交付途中的
中间位置（工具栏可能显示 svn/base 或 HEAD），不要在这里改代码。」，按钮 **「继续交付」
「停止收尾」**，下面一行说明两个按钮各自的后果。点「继续交付」→ 确认框 →「继续」→
「已交付 r9」，仍提示 1 个本地补丁。

```powershell
svn log -q -l 1 --non-interactive $URL
G log --oneline -1 svn/base
G log --oneline svn/base..work
Get-Content (Join-Path $WC 'Code/P.cs') -TotalCount 1
Phase
```

最新仍是 r9（**没有重复提交**）、`svn: r9`、一行本地补丁、`colleague1 patch-local`、`（没有 journal）`。

> 同 7a：实际停在哪个 phase 记进决策日志。

---

## 场景 8：多分支

`work` 不是特殊分支：交付与更新都作用于**当前 checkout 的分支**，唯一的分支门槛是它必须
包含最新的 `svn/base`（§8.1 第 0 条）。这个场景造两条落后于基线的分支，同时验多分支下的提交图。

**造现场（操作窗口）：** 从基线拉 `e2e8`，提交 `X8`；`keep8` 指着 `X8`；`grow8` 从 `X8` 长出 `G8`。

```powershell
G switch -q -c e2e8 svn/base
Wc 'Code/X8.cs' 'class X8 {}'
G add Code/X8.cs
G commit -qm 'e2e8 X8'
G branch keep8
G switch -q -c grow8
Wc 'Code/G8.cs' 'class G8 {}'
G add Code/G8.cs
G commit -qm 'e2e8 G8'
G switch -q e2e8
```

**操作（应用）：** 当前在 `e2e8`。点「交付」，提交信息 `e2e8 交付 X8` →「已交付 r10」。

**核对提交图（应用）：**

- r10 → r9 → … → r1 全是**最左一列、同一颜色的菱形**，没有被任何分支拆成两列；
- r10 那一行带 `svn/base`、`e2e8` 两个标签；
- `e2e8 X8`（带 `keep8`）**照常显示**（有分支指着它，`grow8` 也从它上面长出来），排在 r10
  与 r9 之间（按时间）；`e2e8 G8`（带 `grow8`）连到它上面；
- 所有行的时间从上到下单调不增。

**接着：落后于基线的分支先被拒（操作窗口 + 应用）：**

```powershell
G switch -q grow8
```

点「交付」→ 顶部错误条「分支 `grow8` 还停在旧基线上，没有包含最新的 `svn/base`……请先对这个
分支点一次「更新」，再交付。什么都没有改动。」

**接着：更新这个分支（应用）：** 点「更新」→ 确认框「服务器上没有比 r10 更新的提交。仍会走一遍
流程，让基线记账与工作副本保持一致。」→「更新」。

```powershell
G log --oneline svn/base..grow8
G log --oneline -1 svn/base
```

`svn/base..grow8` **只有** `e2e8 G8` 一行（`X8` 已经在基线里，不重放）；基线仍是 `svn: r10`
（多录了一条同 revision 的基线，属正常）。

**接着：交付这个分支（应用）：** 点「交付」→ 清单只有「新增 1」`Code/G8.cs` → 提交信息
`e2e8 交付 G8` →「已交付 r11」。

**接着：只指着已交付提交的分支（操作窗口 + 应用）：**

```powershell
G switch -q keep8
```

点「更新」→「更新」，然后点「交付」→ 错误条「没有可交付的改动。分支和基线的内容完全一样。」

```powershell
(G rev-parse keep8) -eq (G rev-parse svn/base)
```

应当是 `True`（`keep8` 被快进到基线上）。

**核对提交图（应用）：** 现在没有分支再指着 `X8` / `G8` 了：两者都**不再单独画出**；点 r10，
「已交付的本地提交」里有 `e2e8 X8`；点 r11，有 `e2e8 G8`。

**收尾（操作窗口 + 应用）：**

```powershell
G switch -q work
```

点「更新」→「更新」，`work` 接到 r11 上，本地补丁还在。

| 现象 | 含义 |
|---|---|
| SVN 主线一段在最左、一段在别的列 | `collectTips` 没把 SVN 链顶端当主干 |
| `e2e8 X8` 排在 r10 上面 | `spliceHistory` 没按时间交错两条序列 |
| `grow8` 更新后 `svn/base..grow8` 里有 `e2e8 X8` | rebase 重放了已交付的提交（会在下次交付时重复提交） |
| `grow8` 在「更新」之前就能交付 | §8.1 第 0 条没拦住——**严重**：会把别的分支交付过的文件当成删除提上去 |

---

## 收尾

```powershell
Remove-Item -Recurse -Force $FIX
```

把每个场景的结果（通过 / 失败 + 现象）记进决策日志「Task 13」一节，再回到「状态」表打勾。

## 本期不验（不是遗漏）

| 项 | 原因 |
|---|---|
| 进程停在 `committing`（提交发出、没拿到答复） | 3a 里这个 phase 的继续 / 放弃都拒绝，只给查证说明；断网查证是 Phase 3b |
| 「停止收尾」 | 由 `cargo test` 的 fixture 测试覆盖（`stop_after_commit` 各分支）；想在窗口里看，可在 7b 重开后点它代替「继续交付」，结果应是「已停止收尾」+ 基线需要更新的提示 |
| pinned 模式下的交付 | 与 pinned 的 `sync_in` 一起做 |
| 交付后删掉服务器上变空的目录 | 已知限制（场景 1 末） |
