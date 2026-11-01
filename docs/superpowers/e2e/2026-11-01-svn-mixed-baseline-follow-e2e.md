# C1a「混合版本基线 + 跟进」手工测试指引

**对应分支：`worktree-svn-mixed-baseline-c1a`（PR：momadacoding/jkk 第 98 号）。** 配套
`docs/superpowers/plans/2026-10-31-svn-mixed-baseline-c1a.md` 与同目录的 `-decisions.md`，
设计见 `docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。
> 所有 PowerShell 代码块都能**原样复制粘贴**，不需要手工替换任何占位符。

## 这一期验的是什么

在 TortoiseSVN 里只提交、不更新，或者只对子目录局部更新之后，切回 jkk，顶部出现黄色横幅
「SVN 侧有 N 个文件有新内容」，点「跟进」，基线就推进到各文件的真实 revision。跟进之后：

- 「本地改动」里只剩还没提交到 SVN 的改动，同事的改动不算到你头上；
- 暂存区里的中间版本、分支上的中间 commit 自动让位给你已提交的最终版；
- 同事的改动与你的暂存做三方合并；
- 其他分支（feature）一起跟上，冲突的标「落后基线」，切过去时走「手动跟进」；
- Local-Only 补丁不会被吞掉，带上服务器时会告警；
- 跟进本身不写工作区里的任何文件。

新增的 Tauri 命令（`svn_follow`、`svn_follow_continue`、`svn_follow_manual`、`svn_follow_mark`、
`svn_wc_db_stamps`、`svn_branches_behind`、`svn_get_user`、`svn_set_user`）参数名的
`snake_case` ↔ `camelCase` 转换**没有编译期检查**（`AGENTS.md` §7.3），自动化测试只钉到了命令函数层。
场景 1 走通就证明 `svn_follow`、`svn_wc_db_stamps`、`svn_follow_mark`、`svn_set_user` 接对了；其余命令在各自场景里验证。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` 1474 过、`pnpm test` 998 过、`pnpm build`、`fmt`、`clippy`） | ✅ 2026-11-01，见决策日志 |
| 下面的真实窗口场景 | 未跑 |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [ ] 场景 1：只提交、不更新 → 横幅 → 跟进 → 「本地改动」只剩没提交的；填「我的 SVN 用户名」
- [ ] 场景 2：暂存了中间版本，提交的是最终版 → 中间版本的暂存被丢弃
- [ ] 场景 3：commit 了好几个中间版本，提交的是最终版 → 中间 commit 被收掉
- [ ] 场景 4：子目录局部更新 → 同事的改动进基线、与你的暂存合并；没改内容的文件不报
- [ ] 场景 5：stash → 修 bug 提交 → 跟进 → stash 照常放回
- [ ] 场景 6：feature 分支 → 在 work 上修 bug 提交 → 跟进后 feature 一起跟上
- [ ] 场景 7：feature 与提交冲突 → 标「落后基线」→「切换并手动跟进」→ 解冲突 → 继续
- [ ] 场景 8：「仅切换」到落后分支 → 横幅「当前分支落后基线」→ 跟进 → 转手动跟进
- [ ] 场景 9：Local-Only 补丁 + 「提交后恢复」只提交功能改动 → 不告警、补丁还在；整文件提交 → 告警
- [ ] 场景 10：只提交删除 → 跟进后记成墓碑
- [ ] 场景 11：SVN 操作进行中切回 jkk → 「SVN 正在更新…」
- [ ] 场景 12：周一根目录整体更新 → 跟进后基线回到单值旧格式
- [ ] 场景 13（可选）：用真的 TortoiseSVN 重走场景 2

---

## 0. 准备

整个过程开**两个** PowerShell 窗口，加上应用本身，下文按用途称呼：

| 窗口 | 用途 |
|---|---|
| **应用窗口** | 跑 `pnpm tauri dev`，别的不干 |
| **操作窗口** | 造 fixture、扮演你自己在 TortoiseSVN 里提交、扮演同事提交、改文件、查结果 |

⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，这份清单里的 git / svn 写操作**只能在临时 fixture 里做**。
**不要拿公司的 Unity 工程或任何真实的 SVN 工作副本来跑这份清单。** fixture 是 `%TEMP%` 下一个一次性的
SVN 仓库，跑完整个目录删掉。

### 0.1 起应用（应用窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-mixed-baseline-c1a
pnpm tauri dev
```

> ⚠️ **必须在这个 worktree 里跑，不是主仓库。** 主仓库的 `master` 上没有这一期的代码。

### 0.2 造 fixture（操作窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-mixed-baseline-c1a
pwsh -File scripts\e2e-svn-fixture.ps1
```

脚本在 `$env:TEMP\phase1c_xxxxxxxx\` 下建一个 SVN 仓库、**你的**工作副本 `wc1` 和**同事的**工作副本 `wc2`，
服务器停在 r1。

### 0.3 定义辅助函数（操作窗口，一次性粘贴）

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
if (-not $fix) { throw '没找到 fixture，先做 0.2' }
# 0.2 失败时，「最新的 phase1c_*」会是以前留下的旧 fixture，接着往下做会改坏它
if ($fix.CreationTime -lt (Get-Date).AddMinutes(-30)) {
    throw "最新的 fixture $($fix.Name) 是 $($fix.CreationTime) 建的，不是刚才 0.2 建的。先确认 0.2 成功了"
}
$WC = Join-Path $fix.FullName 'wc1'
$WC2 = Join-Path $fix.FullName 'wc2'
function Wc([string]$rel, [string]$content) {
    $p = Join-Path $WC $rel
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    [IO.File]::WriteAllText($p, $content)
}
function Colleague([string]$rel, [string]$content) {
    svn update -q --non-interactive $WC2
    $p = Join-Path $WC2 $rel
    $isNew = -not (Test-Path $p)
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    [IO.File]::WriteAllText($p, $content)
    if ($isNew) { svn add -q --parents --non-interactive $p }
    svn commit -q --non-interactive --username colleague -m "colleague $rel" $WC2
    if ($LASTEXITCODE -ne 0) { throw 'svn commit 失败' }
}
function SvnCommit([string[]]$rels) {
    # @(...)：只有一个文件时管道结果是字符串，@paths 会把它拆成一个个字符传给 svn
    $paths = @($rels | ForEach-Object { Join-Path $WC $_ })
    svn commit -q --non-interactive -m "me: $($rels -join ', ')" @paths
    if ($LASTEXITCODE -ne 0) { throw 'svn commit 失败' }
}
function G { git -C $WC @args; if ($LASTEXITCODE -ne 0) { throw "git $args 失败" } }
function Base { git -C $WC log -1 --format=%B svn/base }
"WC = $WC"; "WC2 = $WC2"; "我的 SVN 用户名 = $env:USERNAME"
```

| 函数 | 扮演什么 |
|---|---|
| `Wc 路径 内容` | 你在 IDE 里改了一个文件（不写 BOM） |
| `SvnCommit 路径...` | **你在 TortoiseSVN 里只勾这几个文件提交**。作者是你的 Windows 登录名（`file://` 仓库就这么记） |
| `Colleague 路径 内容` | 同事在他的机器上改了一个文件并提交，作者是 `colleague` |
| `G ...` | 在 `wc1` 里跑 git，失败就抛错 |
| `Base` | 看当前基线 commit 的说明（记账写在这里） |

> 这些定义只在当前 PowerShell 会话里有效。**操作窗口别关。**

### 0.4 再铺几个模块，模拟游戏工程的目录（操作窗口）

```powershell
Wc 'Code/Bag/B1.cs' 'class B1 {}'
Wc 'Code/Bag/B2.cs' "// B2`nint l2;`nint l3;`nint l4;`nint l5;`n"
Wc 'Code/Mail/M1.cs' 'class M1 {}'
Wc 'Code/Mail/M2.cs' 'class M2 {}'
svn add -q --parents --non-interactive (Join-Path $WC 'Code/Bag') (Join-Path $WC 'Code/Mail')
svn commit -q --non-interactive -m 'modules' $WC
svn update -q --non-interactive $WC
svn info --show-item revision $WC
```

最后一行应当是 `2`：服务器在 r2，`wc1` 整体在 r2。

### 0.5 初始化成混合仓库（应用里）

在应用里打开 0.3 打印的 `WC` 路径（`...\wc1`），走完初始化向导：**白名单填 `Code/`，扩展名用默认的 `.cs`**。
走完之后停在 `work` 分支上，没有横幅。

```powershell
if (-not (Test-Path (Join-Path $WC '.git'))) { throw '还没初始化，先做 0.5' }
Base
```

最后一行应当是 `git-svn-id: file:///.../repo@2 <uuid>`：单值的旧格式。

### 0.6 界面怎么发现变化

| 变化 | 界面靠什么发现 | 应当在多久内反映出来 |
|---|---|---|
| 在操作窗口里提交（`SvnCommit` / `svn update`） | **点回应用窗口的那一刻**：比较 `.svn/wc.db` 的时间戳，变了才去问 `svn status` | 点回去后 1～3 秒 |
| 在操作窗口里改文件 | 文件监听（300ms 去抖） | 约 1 秒 |
| 跟进 / 手动跟进结束 | 后端结束时补发一次刷新 | 立刻 |

**本期最重要的一个习惯：** 下面每个场景里「点回应用窗口」都是一个真实的步骤，不是可有可无的。
点操作窗口的那一刻（应用失焦），jkk 会给当前分支和暂存区拍一份快照；点回应用（得焦）才会去检查 SVN。
「中间版本自动让位」靠的就是这份快照。

---

## 场景 1：只提交、不更新

**造现场（操作窗口）：** 改了 A 和 D，只在 TortoiseSVN 里提交 A。

```powershell
Wc 'Code/A.cs' 'class A { int mine1; }'
Wc 'Code/Deep/D.cs' 'class D { int wip1; }'
SvnCommit 'Code/A.cs'
```

**操作（应用）：** 点一下应用窗口。

**应该看到：**

- 1～3 秒内顶部出现**黄色**横幅 **「SVN 侧有 1 个文件有新内容」**，列出 `Code/A.cs`、`r3`、作者是你的登录名；
- 横幅上**只有一个**按钮 **「跟进」**，没有「体检基线」「重建基线」；
- 此刻「本地改动」里同时有 `Code/A.cs` 和 `Code/Deep/D.cs`：A 已经在 SVN 上了，基线还没跟上。

**操作（应用）：** 点「跟进」。

**应该看到：**

- 按钮在执行中变灰，不能连点；
- 弹出 **「跟进完成」**：「跟进的文件」一节按作者列出 `Code/A.cs`；
- 弹框里有一行 **「我的 SVN 用户名」** 输入框（`file://` 仓库没法自动探测，所以会出现）。
  填入 0.3 最后一行打印的登录名（例如 `liuhao`），保存，再点「知道了」。

**核对（操作窗口）：**

```powershell
G status --short
Base
Get-Content (Join-Path $fix.FullName 'wc1\.git\mygitclient.json') | Select-String svnUser
```

依次应当是：

- 只有一行 ` M Code/Deep/D.cs`——A 不再算你的本地改动；
- 基线说明里有 `Svn-Base-Floor: 2` 和 `Svn-Base-Rev: 3 Code/A.cs`，**没有** `git-svn-id` 行（混合版本的新格式）；
- `"svnUser": "liuhao"`（你的登录名）。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 点回应用后没有横幅，要重开标签页才有 | 焦点检测没接上（`svn_wc_db_stamps` 的 invoke 写错，或焦点监听没挂）——打开 devtools 看 console |
| 横幅是红色「工作副本处于多个 revision」 | 跑的不是这个 worktree 的代码 |
| 点「跟进」没反应，或报参数缺失 | `svn_follow` 的 invoke 名或参数名写错——看 devtools console |
| 弹框里没有用户名输入行 | `svn_get_user` 没接上 |
| 保存用户名后报错 | `svn_set_user` 的参数名（`user`）不对 |
| `G status` 里还有 `Code/A.cs` | 跟进没把 A 收进基线 |

---

## 场景 2：暂存了中间版本，提交的是最终版

你的日常用法：改一版、暂存，再改、看 diff，最后在 TortoiseSVN 里提交最终版。

**造现场：**

1. **操作窗口：**

   ```powershell
   Wc 'Code/A.cs' 'class A { int v1; }'
   ```

2. **应用：** 点回应用窗口，在「本地改动」里**暂存** `Code/A.cs`。
3. **操作窗口**（点进来的这一刻，jkk 记下快照：A 的暂存版本是 v1）：

   ```powershell
   Wc 'Code/A.cs' 'class A { int v2; }'
   SvnCommit 'Code/A.cs'
   ```

**操作（应用）：** 点回应用窗口 → 横幅「SVN 侧有 1 个文件有新内容」→ 点「跟进」。

**应该看到：** 「跟进完成」弹框里有 **「丢弃的暂存」** 一节，列出 `Code/A.cs`，并说明旧暂存区备份在
`refs/mygitclient/follow/...` 里能找回。

**核对（操作窗口）：**

```powershell
G status --short
G diff --cached --name-only
Get-Content (Join-Path $WC 'Code/A.cs')
```

依次应当是：只有 ` M Code/Deep/D.cs`；空（没有任何暂存）；`class A { int v2; }`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| A 出现在「拿不准的暂存」里，暂存还在 | 失焦时没写快照（`svn_follow_mark` 没接上，或写快照的条件判错了），或场景 1 的用户名没存上 |
| A 出现在「合并的暂存」或「丢失的暂存」里 | 作者判断没认出是你 |

---

## 场景 3：commit 了好几个中间版本，提交的是最终版

**造现场：** 每一步都要在两个窗口之间切换，因为快照是在你离开应用的那一刻拍的。

1. **操作窗口：** `Wc 'Code/Bag/B1.cs' 'class B1 { int w1; }'`
2. **应用：** 点回应用，暂存 `Code/Bag/B1.cs`，提交说明写 `wip1`，点提交。
3. **操作窗口：** `Wc 'Code/Bag/B1.cs' 'class B1 { int w2; }'`
4. **应用：** 暂存 `Code/Bag/B1.cs`，提交说明写 `wip2`，点提交。
5. **操作窗口：**

   ```powershell
   Wc 'Code/Bag/B1.cs' 'class B1 { int final3; }'
   SvnCommit 'Code/Bag/B1.cs'
   ```

**操作（应用）：** 点回应用 → 横幅 → 点「跟进」。

**应该看到：** 「跟进完成」弹框里有 **「丢弃的 commit」**，列出 2 个（`wip1`、`wip2` 的 OID 前 10 位），
以及「换成 SVN 版本的路径」。

**核对（操作窗口）：**

```powershell
G log --oneline svn/base..HEAD
Get-Content (Join-Path $WC 'Code/Bag/B1.cs')
G status --short
```

依次应当是：空（分支上没有自己的 commit 了）；`class B1 { int final3; }`；只有 ` M Code/Deep/D.cs`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 弹出「跟进遇到冲突」 | 中间 commit 没被认成「已被你更晚的提交取代」：快照没写，或 commit 不是快照 HEAD 的祖先 |
| `wip1`、`wip2` 还在 `svn/base..HEAD` 里 | 同上 |

---

## 场景 4：子目录局部更新

同事改了背包模块，你只更新了 `Code/Bag`。同时你自己暂存了 B2 的另一处改动。

**造现场：**

1. **操作窗口：**

   ```powershell
   Wc 'Code/Bag/B2.cs' "// B2`nint l2;`nint l3;`nint l4;`nint mine5;`n"
   ```

2. **应用：** 点回应用，暂存 `Code/Bag/B2.cs`。
3. **操作窗口：**

   ```powershell
   Colleague 'Code/Bag/B2.cs' "// B2 by colleague`nint l2;`nint l3;`nint l4;`nint l5;`n"
   Wc 'Code/Mail/M1.cs' 'class M1 { int local4; }'
   svn update -q --non-interactive (Join-Path $WC 'Code/Bag')
   svn status -v (Join-Path $WC 'Code/Bag')
   ```

   `svn status` 里 B1、B2 的节点 revision 都被推到了最新（局部更新会推高整个目录），但只有 B2 的内容真的变了。

**操作（应用）：** 点回应用。

**应该看到：** 横幅「SVN 侧有 1 个文件有新内容」，**只列 `Code/Bag/B2.cs`**，作者 `colleague`。
`Code/Bag/B1.cs` 不在列表里。点「跟进」→「跟进完成」里有「合并的暂存：`Code/Bag/B2.cs`」，
「跟进的文件」按作者分组，`colleague` 下是 `Code/Bag/B2.cs`。

**核对（操作窗口）：**

```powershell
G diff --cached Code/Bag/B2.cs
G show svn/base:Code/Bag/B2.cs
G status --short
```

- 暂存区的 diff **只有** `l5` → `mine5` 这一处，不含同事改的第一行（同事的改动不算到你头上）；
- 基线里的 B2 第一行是 `// B2 by colleague`；
- `status` 里有 `M  Code/Bag/B2.cs`（已暂存）、` M Code/Deep/D.cs`、` M Code/Mail/M1.cs`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 横幅里也列了 `Code/Bag/B1.cs` | 判定还在看节点 revision，没改看最后修改 revision（E12） |
| 暂存的 diff 里有同事的那一行 | 暂存没做三方合并，同事的改动被算成了你的 |
| B2 出现在「丢失的暂存」里 | 三方合并判成了冲突（两处改动隔了 3 行，不该冲突） |

**收尾（操作窗口）：** 把暂存的 B2 提交到 git，下一个场景从这里开始。

```powershell
G commit -qm 'e2e 4 B2'
```

---

## 场景 5：stash 去修 bug，回来继续

jkk 没有「收起改动」的按钮，这一步在操作窗口里用命令做（只在 fixture 里）。

**造现场（操作窗口）：**

```powershell
G stash push -q -m 'e2e5 wip'
G status --short
Wc 'Code/A.cs' 'class A { int fix5; }'
SvnCommit 'Code/A.cs'
```

`stash push` 之后 `status` 应当是空的。

**操作（应用）：** 点回应用 → 横幅 → 点「跟进」→「跟进完成」。

**核对（操作窗口）：**

```powershell
G stash pop -q
G status --short
Get-Content (Join-Path $WC 'Code/A.cs')
```

`stash pop` 不报冲突；`status` 里回来 ` M Code/Deep/D.cs` 和 ` M Code/Mail/M1.cs`；A 是 `class A { int fix5; }`。

**收尾（操作窗口）：** 把剩下的改动提交到 git，后面切分支要干净的工作区。

```powershell
G commit -qam 'e2e 5 leftovers'
```

---

## 场景 6：feature 分支跟着 bugfix 走

**造现场（操作窗口）：** 开 feature 分支改 M1，回到 work 修 bug 并在 TortoiseSVN 里提交。

```powershell
G switch -q -c feature
Wc 'Code/Mail/M1.cs' 'class M1 { int feature6; }'
G commit -qam 'feature M1'
G switch -q work
Wc 'Code/A.cs' 'class A { int fix6; }'
SvnCommit 'Code/A.cs'
```

**操作（应用）：** 点回应用 → 横幅 → 点「跟进」。

**应该看到：** 「跟进完成」里有 **「移动的分支：feature」**；左侧分支列表里 `feature` **没有**「落后基线」标记。

**核对（操作窗口）：**

```powershell
git -C $WC merge-base --is-ancestor svn/base feature; $LASTEXITCODE
```

应当是 `0`：feature 已经在新基线上。

**接着（应用）：** 在分支列表里切到 `feature`。不弹任何确认框，切完之后：

```powershell
svn status (Join-Path $WC 'Code')
```

只列 `M ... Code\Mail\M1.cs`。A 没有显示成「已修改」：切分支没有把旧的 A 写回磁盘。

**收尾（应用）：** 切回 `work`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| `feature` 上有「落后基线」 | 其他分支没在对象层跟着重放 |
| 切到 feature 后 `svn status` 里有 `M Code\A.cs` | feature 没跟上，切分支把旧内容写回了磁盘——这正是本期要杜绝的 |

---

## 场景 7：feature 与提交冲突 →「切换并手动跟进」

**造现场（操作窗口）：** feature 上也改了 A，然后在 work 上修 bug、提交了不一样的 A。

```powershell
G switch -q feature
Wc 'Code/A.cs' 'class A { int feat7; }'
G commit -qam 'feature A'
G switch -q work
Wc 'Code/A.cs' 'class A { int fix7; }'
SvnCommit 'Code/A.cs'
```

**操作（应用）：** 点回应用 → 横幅 → 点「跟进」。

**应该看到：**

- 「跟进完成」里有 **「落后的分支」**，列出 `feature` 和原因；当前分支 `work` 照常跟进完了；
- 左侧分支列表里 `feature` 带 **「落后基线」** 标记。

**接着（应用）：** 在分支列表里切到 `feature`。

**应该看到：** 弹出 **「切换到落后基线的分支」**，两个按钮 **「切换并手动跟进」**（默认）和 **「仅切换」**，
警告条写着「仅切换」会把旧内容写回磁盘。点 **「切换并手动跟进」** → 再弹 **「手动跟进」** 确认框
（警告写着这一步会写工作区）→ 点「手动跟进」。

**应该看到：** 弹出 **「手动跟进遇到冲突」**，列出 `Code/A.cs`；顶部出现带「继续」「放弃」的横幅。

**接着（操作窗口）：** 手工解冲突。

```powershell
Wc 'Code/A.cs' 'class A { int fix7; int feat7; }'
```

**接着（应用）：** 在「本地改动」里暂存 `Code/A.cs` → 点横幅上的「继续」→ 确认。

**核对（操作窗口）：**

```powershell
git -C $WC merge-base --is-ancestor svn/base HEAD; $LASTEXITCODE
G symbolic-ref --short HEAD
Get-Content (Join-Path $WC 'Code/A.cs')
```

应当是 `0`、`feature`、`class A { int fix7; int feat7; }`；分支列表里 `feature` 的「落后基线」标记消失。

**收尾（应用）：** 切回 `work`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 切 feature 时没有确认框 | `svn_branches_behind` 没接上，或 `BranchView` 没读它 |
| 点「切换并手动跟进」后没有后续 | `svn_follow_manual` 的 invoke 写错——看 devtools console |
| 「继续」之后横幅不消失 | journal 没删 |

---

## 场景 8：「仅切换」到落后分支

**造现场（操作窗口）：**

```powershell
G switch -q -c side8
Wc 'Code/A.cs' 'class A { int side8; }'
G commit -qam 'side8 A'
G switch -q work
Wc 'Code/A.cs' 'class A { int fix8; }'
SvnCommit 'Code/A.cs'
```

**操作（应用）：** 点回应用 → 横幅 → 点「跟进」→「跟进完成」里 `side8` 落后。
在分支列表里切到 `side8` → 这次点 **「仅切换」**。

**应该看到：**

- 切过去之后，顶部出现**黄色**横幅 **「当前分支落后基线」**，带「跟进」按钮；
- 操作窗口里 `svn status (Join-Path $WC 'Code/A.cs')` 显示 `M`：旧内容被写回了磁盘（确认框警告过的那件事）。

**操作（应用）：** 点横幅上的「跟进」。

**应该看到：** **不会**直接「跟进完成」，而是弹出 **「跟进遇到冲突」** 或 **「这次要手动跟进」**，
说明分支和暂存区都没动，按钮是「手动跟进」。点「手动跟进」→ 遇到冲突时同场景 7 解掉
（`Wc 'Code/A.cs' 'class A { int fix8; int side8; }'`，暂存，点「继续」）。

**核对（操作窗口）：**

```powershell
git -C $WC merge-base --is-ancestor svn/base HEAD; $LASTEXITCODE
G status --short
```

应当是 `0`；`status` 里没有把 SVN 内容改回去的修改。

**收尾（应用）：** 切回 `work`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 切到 side8 后没有「当前分支落后基线」横幅 | 切分支后的检测没跑（`repo-changed` 里 HEAD 变化没触发第二档校验） |
| 点「跟进」直接「跟进完成」，之后 `G diff` 显示 A 从 `fix8` 改回 `side8` 之前的内容 | 磁盘还是旧内容时没有转手动跟进——最终 review 修过的那条（Review 3 第 2 条） |

---

## 场景 9：Local-Only 补丁 +「提交后恢复」

你的 ignore changelist 用法：同一个文件里，注释 log 的那块永远不提交，功能改动要提交。

**造现场 1（操作窗口）：** 先把 D 变成多行，提交并跟进一次，作为起点。

```powershell
Wc 'Code/Deep/D.cs' "class D {`n  int a;`n  int b;`n  int c;`n  int d;`n}`n"
SvnCommit 'Code/Deep/D.cs'
```

**应用：** 点回应用 → 跟进。

**造现场 2：**

1. **操作窗口**——加一行注释 log：

   ```powershell
   Wc 'Code/Deep/D.cs' "class D {`n  int a;`n  // log(""debug"");`n  int b;`n  int c;`n  int d;`n}`n"
   ```

2. **应用：** 暂存 D，提交说明写 `log patch`，**勾上「仅本地，不交付」**，点提交。图上这个提交带「仅本地」角标。
3. **操作窗口**——再改一处功能代码，然后模拟 TortoiseSVN 的「提交后恢复」：
   先把文件改成只含功能改动的版本提交，再恢复原样。

   ```powershell
   Wc 'Code/Deep/D.cs' "class D {`n  int a;`n  // log(""debug"");`n  int b;`n  int c;`n  int d9;`n}`n"
   $full = [IO.File]::ReadAllText((Join-Path $WC 'Code/Deep/D.cs'))
   Wc 'Code/Deep/D.cs' "class D {`n  int a;`n  int b;`n  int c;`n  int d9;`n}`n"
   SvnCommit 'Code/Deep/D.cs'
   [IO.File]::WriteAllText((Join-Path $WC 'Code/Deep/D.cs'), $full)
   ```

**操作（应用）：** 点回应用 → 跟进。

**应该看到：** 「跟进完成」里**没有**「泄漏告警」。

```powershell
G log --oneline svn/base..HEAD
G status --short
```

- `log patch` 还在 `svn/base..HEAD` 里，补丁没被吞掉；
- `status` 里没有 `Code/Deep/D.cs`：功能改动已进基线，log 那行由 Local-Only 提交管着。

**造现场 3（操作窗口）：** 这次整个文件直接提交，连 log 一起带上服务器。

```powershell
Wc 'Code/Deep/D.cs' "class D {`n  int a;`n  // log(""debug"");`n  int b;`n  int c9;`n  int d9;`n}`n"
SvnCommit 'Code/Deep/D.cs'
```

**操作（应用）：** 点回应用 → 跟进。

**应该看到：** 「跟进完成」里有 **「泄漏告警」**，列出 `log patch` 这个提交和 `Code/Deep/D.cs`。
因为补丁的内容已经整个在 SVN 上了，`log patch` 同时会出现在「丢弃的 commit」里，这是对的。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 造现场 2 之后就出现泄漏告警 | 泄漏判定按文件而不是按 hunk——只提交功能 hunk 也被当成泄漏 |
| 造现场 2 之后 `log patch` 从 `svn/base..HEAD` 里消失 | Local-Only 提交被当成中间版本换掉了（Review 2 第 2 条） |
| 造现场 3 之后没有告警 | 泄漏漏报 |

---

## 场景 10：只提交删除

**造现场（操作窗口）：**

```powershell
svn delete -q --non-interactive (Join-Path $WC 'Code/Mail/M2.cs')
SvnCommit 'Code/Mail/M2.cs'
```

**操作（应用）：** 点回应用。

**应该看到：** 横幅出现，列出 `Code/Mail/M2.cs`（说明它在工作副本里已经没有了）。点「跟进」→
「跟进的文件」里有 **「已在 SVN 上删除」：`Code/Mail/M2.cs`**。

```powershell
Base | Select-String 'Svn-Base-Gone'
G status --short
```

应当有一行 `Svn-Base-Gone: <rev> Code/Mail/M2.cs`；`status` 里没有 `D Code/Mail/M2.cs`。

**看到别的说明：** 点回应用后没有横幅——只提交删除没被发现（改造前就有的盲区，本期补上了）。

---

## 场景 11：SVN 操作进行中切回 jkk

每周根目录更新要半小时，期间你可能切回 jkk。这里用一个「等编辑器」的提交把工作副本锁住 25 秒来模拟。

**造现场（操作窗口）：**

```powershell
$bat = Join-Path $fix.FullName 'slow-editor.cmd'
Set-Content $bat "@ping -n 26 127.0.0.1 >nul`r`n@echo e2e11> %1" -Encoding ascii
Wc 'Code/A.cs' 'class A { int fix11; }'
Start-Process svn -NoNewWindow -ArgumentList @('commit', '--editor-cmd', $bat, (Join-Path $WC 'Code/A.cs'))
```

**操作（应用）：** **25 秒内**点回应用窗口。

**应该看到：** 顶部是**灰色**横幅 **「SVN 正在更新…」**，没有任何按钮。

**接着：** 等 30 秒（提交完成），点一下操作窗口，再点回应用。

**应该看到：** 横幅变成黄色「SVN 侧有 1 个文件有新内容」→ 点「跟进」→ 完成。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 25 秒内点回应用，出现黄色横幅或报错 | 没认出工作副本被锁（`wc-locked`），在提交一半的时候去跟进了 |
| 提交完成后切回，横幅还是「SVN 正在更新…」 | 锁解除后的那次校验没跑 |

---

## 场景 12：周一根目录整体更新

**造现场（操作窗口）：** 同事这周提交了几次，你对整个工作副本 update。

```powershell
Colleague 'Code/Bag/B1.cs' 'class B1 { int colleague12; }'
Colleague 'Code/New12.cs' 'class New12 {}'
svn update -q --non-interactive $WC
```

**操作（应用）：** 点回应用 → 横幅列出 `Code/Bag/B1.cs` 和 `Code/New12.cs`（作者 `colleague`）→ 点「跟进」。

**核对（操作窗口）：**

```powershell
Base
G status --short
```

- 基线说明最后一行回到 **`git-svn-id: file:///.../repo@<最新 rev> <uuid>`**：单值的旧格式，
  没有 `Svn-Base-Rev`，也没有场景 10 留下的 `Svn-Base-Gone`（整体更新后墓碑自动清掉）；
- `status` 里没有 `Code/Bag/B1.cs`、`Code/New12.cs`：同事的改动没算到你头上。

**看到别的说明：** 基线说明还是 `Svn-Base-Floor:` 格式——整体更新后映射没回到单值（映射取值规则有误）。

---

## 场景 13（可选）：用真的 TortoiseSVN 重走场景 2

前面的 `SvnCommit` 是命令行，效果与 TortoiseSVN 提交相同，但 TortoiseSVN 会弹出自己的对话框，
焦点的切换时机不一样。

1. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t1; }'`
2. **应用：** 暂存 `Code/A.cs`。
3. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t2; }'`
4. 在资源管理器里打开 `wc1` 目录（`explorer $WC`），右键 → **SVN Commit...**，只勾 `Code\A.cs`，提交。
   TortoiseSVN 的进度框显示完成后，关掉它。
5. **应用：** 点回应用 → 横幅 → 跟进。

**应该看到：** 与场景 2 相同：「丢弃的暂存：`Code/A.cs`」，A 是 `t2`。

**看到别的说明：** A 出现在「拿不准的暂存」里——TortoiseSVN 弹框期间的焦点时序与预期不同，
请记下你点了哪些窗口、按什么顺序，反馈回来。

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
| 跟进过程中崩溃，重开后从每一个断点续上 | `src-tauri/tests/svn_follow.rs::resume_after_crash_between_each_step` |
| 发布中途外部 `git checkout -b`（CAS 前 / CAS 后 / 崩溃中 / 锁被手工删） | `svn_follow.rs` 的 `checkout_b_*` 一组 |
| 跟进全程不写工作区 | `follow_never_writes_working_tree` |
| 局部更新只推高 revision、内容没变时判为已同步 | `svn_sync.rs::partial_update_without_content_change_stays_in_sync` |
| 工作副本被锁时判为「正在更新」 | `svn_sync.rs::locked_working_copy_reports_wc_locked` |
| 已提交的删除成为墓碑；漏收（depth-exclude）拒绝录制；确认失败拒绝 | `svn_health.rs::committed_deletion_becomes_tombstone_when_just_recorded` 及同文件的 exclude / unconfirmed 测试 |
| Local-Only 泄漏按 hunk 判定（含相邻行、行号偏移） | `svn_leak.rs::leak_by_hunk_matches_w21_scenarios` 等 |
| 落后分支磁盘还是旧内容时转手动跟进 | `branch_entry_sends_a_stale_disk_to_manual_follow`、`a_stale_disk_on_a_lagging_branch_also_stops_a_recording_follow` |
| 手动跟进暂存没保住时 stash 保留、能取回 | `manual_follow_stage_lost_keeps_a_stage_that_differs_from_disk` |
| 多条分支共享的 commit 只重放一次；外部 `--no-ff` merge 后内容已全部进 SVN 时直接跟上 | `svn_replay.rs::shared_commit_replayed_once_across_branches`、`all_committed_true_through_no_ff_merge_with_delete_add_mode_rename` |
| 拼索引：暂存的删除 / mode / i-t-a / skip-worktree / assume-unchanged、冲突索引拒绝、钉住的旧索引 gc 后能还原 | `src-tauri/tests/svn_follow_index.rs` |
| 失焦快照、`svn auth` 用户名解析、wc.db 戳 | `src-tauri/tests/svn_follow_mark.rs` |
| IPC 字段名（camelCase） | `src-tauri/tests/svn_backend.rs` |
| 横幅、跟进报告、焦点 / 失焦判定的前端逻辑 | `tests/vitest/svnSync.test.ts`、`svnFollow.test.ts`、`repoStoreFollowFocus.test.ts`、`FollowReportDialog.test.tsx`、`BranchView.test.tsx` |
