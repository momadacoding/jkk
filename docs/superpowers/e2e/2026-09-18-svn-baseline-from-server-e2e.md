# 基线改从服务器录 · 手工测试指引

**对应分支：`feat/svn-baseline-from-server`（PR #44，base 是 `feat/svn-phase1c` 不是 `master`）。**
本地 worktree：`.claude/worktrees/svn-baseline-b`。

> 同目录下的 `2026-09-15-svn-phase1c-e2e.md` 是**上一期**（PR #43，分支
> `feat/svn-phase1c`）的清单。它验的不是本期的改动，两份都在这个 worktree 里，
> 别弄混。

## 状态

| | |
|---|---|
| 自动化测试 | Rust 526 passed / 0 failed；前端 341 passed（2026-09-19） |
| 真实窗口 E2E | ⬜ **全部未执行**，本文件就是那份清单（勾选表在下面） |

> 本文件里的每一条命令都在真实 fixture 上跑过，输出是实测值不是猜的。
> 场景 10 最初写错过一版（会给出**假的通过**），已按实测改正——见那条里的方框。

跑完一条就勾一条。**没跑的别勾——空着是如实，勾上是谎报。**

- [x] 场景 1：干净工作副本上初始化（回归）
- [x] 场景 2：**带着本地修改**初始化 ← 核心新行为之一
- [x] 场景 3：初始化完，`git status` 恰好是你原有的那三处修改
- [x] 场景 4：基线里存的是**服务器**那一版
- [x] 场景 5：同事提交 → 失同步 → **带着本地修改**重建，且磁盘分毫未动
- [x] 场景 6：`~`（obstructed）仍然被拦，且**不自动修复**
- [x] 场景 7：未提交的新文件**不再**被误报成 mixed-revision
- [x] 场景 8：真正的 mixed-revision **仍然**被拦
- [x] 场景 9：脏工作副本上点「体检基线」，能给出**明确结论**
- [x] 场景 10：体检抓得住**真的**假基线 ← 必须和场景 9 成对做
- [x] 场景 11：行尾一致性（`core.autocrlf`）
- [x] 场景 12：整个过程一条 SVN 写命令都没发
- [ ] 场景 13：**目录**形态的脏状态 ← 2026-09-20 新增，前 12 条没覆盖

---

## 0. 这份清单在测什么

### 一句话

Phase 1c 里，只要你手上有本地修改，**「重建基线」「初始化向导」「体检基线」三个入口
全都点不动**。这个分支把它们改成不再要求你先清干净，办法是**换掉基线内容的来源**：

```text
旧：把磁盘录成基线 → 所以必须先保证「磁盘 == 服务器」→ 要求工作区干净
新：直接照服务器录 → 脏文件去取服务器那一版，干净文件才用磁盘
```

所以整份清单反复在问两件事：

1. **脏工作副本上能不能做成**（旧版会被拒）
2. **做的过程碰没碰你的磁盘**（碰了就等于 `svn revert`，是最严重的缺陷）

### ⚠️ 先确认你测的是对的那个构建

这是最容易浪费一小时的地方。在**终端 A**：

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-baseline-b
git rev-parse --abbrev-ref HEAD
```

必须打出 **`feat/svn-baseline-from-server`**。打出别的就说明你在 1c 的 worktree 里，
下面每条都会「失败」，但那不是 bug。

### 为什么这些必须手工测

自动化测试跑的是后端函数。它证明不了：横幅长什么样、按钮点下去有没有反应、
确认框里的话说得对不对、错误信息在界面上是不是人话。这些只有真实窗口能看见。

### 这份清单不会让你敲任何破坏性命令

没有 `Remove-Item .git`、没有 `git reset --hard`、没有 `svn revert -R`（只有场景 10
用一次，且明确圈定在 fixture 里）。需要「回到初始状态」时一律**重造 fixture**——
它只要几秒钟，而且零风险。

---

## 1. 准备

### 1.1 三个终端

| | 干什么 | 之后 |
|---|---|---|
| **终端 A** | 跑 `pnpm tauri dev` 把应用起起来 | 起完别动 |
| **终端 B** | 在 `wc1` 里敲命令、看结果 | 一直用 |
| **终端 C** | 扮演「同事」，在 `wc2` 里提交 | 几个场景用 |

### 1.2 确认 svn 命令行可用

**终端 B**：

```powershell
svn --version --quiet
```

看到版本号（例如 `1.14.2`）就行。报「无法识别 svn」说明只装了 TortoiseSVN 的图形
界面——重跑安装包选 Modify，把 **command line client tools** 勾上，或者装
[SlikSVN](https://sliksvn.com/download/)。装完**重开终端**。

### 1.3 起应用

**终端 A**：

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-baseline-b
pnpm install
pnpm tauri dev
```

首次编译 Rust 要几分钟。窗口弹出来就算好了。

### 1.4 造 fixture（后面会重复做好几次）

**终端 B**，同一个 worktree 目录：

```powershell
pwsh -File scripts/e2e-svn-fixture.ps1
```

跑完打出两行路径。存成变量（换成你自己的随机后缀）：

```powershell
$fx = "C:\Users\liuhao\AppData\Local\Temp\phase1c_8e0c428e"
cd $fx\wc1
```

后文写 `$fx` 就是指这一段（不含 `\wc1`）。**终端 B 之后一直待在 `$fx\wc1`。**

fixture 里的文件：

| 路径 | 为什么在那儿 |
|---|---|
| `Code\A.cs`、`Code\Deep\D.cs` | 白名单内的代码，会进 git |
| `Code\Art.psd` | 白名单内但扩展名不符——**不该**进 git，也不该进任何差异清单 |
| `Art\boss.psd` | 白名单外 |

`$fx\wc2` 是「同事的副本」，在那里提交 = 外部变更。

### 1.5 每次「重造 fixture」的完整动作

后面好几处会说「重造 fixture」，指的就是这三步：

```powershell
cd ~
Remove-Item -Recurse -Force $fx          # 删的是 %TEMP% 下那个随机目录
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-baseline-b
pwsh -File scripts/e2e-svn-fixture.ps1   # 重新打出一个新路径，更新 $fx
```

> 删之前**先在应用里关掉这个仓库**（或直接关应用），否则文件被占用删不掉。

---

## 2. 测试场景

每条的格式是「**做什么** → **应该看到** → **看到别的说明什么**」。
不符就把场景号 + 实际现象记下来，不用自己排查。

场景 2–8 是**一条连续的线**，按顺序做，中间不要重造 fixture。

### ⚠️ 先读这条：「重建基线」按钮什么时候才在

好几条场景要你点 **「重建基线」**。这颗按钮**只挂在失同步横幅上**——工作副本和
基线对得上的时候，界面上只有一条灰条加「体检基线」，没有它。（体检报
`Lying` 时弹框里另给一个入口，见场景 10；那是另一条路。）

**要让它出现，两件事都得做：**

| 要做的 | 为什么 |
|---|---|
| ① 让**你自己的工作副本**动起来 | 同事提交只改服务器，不动你的工作副本，判定仍是 `inSync`（判据见场景 5 开头）。所以同事提交之后**你还得跑一次 `svn update`** |
| ② 应用里**重新打开仓库** | 见下 |

> **「重新打开仓库」= 在应用里把当前这个仓库关掉，再 `Open Repo` 选回
> `$fx\wc1`。** 不是按 F5，也不是在终端里存个文件等它自己刷新。
>
> 各场景里仓库是什么时候**第一次**打开的：场景 1–8 是场景 1 / 场景 2 的
> 向导那一步（`Open Repo` → 走完向导 → 建库）；场景 9–12 里凡是先「重造
> fixture」的，都要在那之后重新 `Open Repo` 一次，各条的第 1 步写了。

**为什么「存个文件让它刷新一下」不够。** 失同步检测是分两档的（spec §6.6.3）：
文件监听那条高频通道上只跑第一档——读各工作副本**根**的 revision，O(1)；变了才
跑第二档的完整 `nodeRevs` 校验。而**打开仓库**走的是无条件的完整校验。

2026-09-19 在一次性 fixture 上实测，`svn update -r 1 Code/Deep` 之后：

```text
退回之前：根 revision = 2
退回之后：根 revision = 2      <<< 第一档看到的就是这个

svn status -v（第二档看到的）：
                   2        2 .
                   2        2 Code
                   2        1 Code\A.cs
                   1        1 Code\Deep          ← 只有这里退回了
                   1        1 Code\Deep\D.cs
```

根 revision 纹丝不动 ⇒ 第一档看不见 ⇒ 第二档不会被触发 ⇒ 横幅不出现。
**把子目录退回旧 revision（场景 8）这种形态，必须重新打开仓库才看得到。**

> 这是设计上的取舍不是缺陷：第二档要在整个白名单子树上跑 `svn status -v`，
> 在几万文件的工作副本上是可感知的开销，不能挂在「每存一次盘」的通道上。
> 漏的只是**提示的及时性**——真去点重建时 P2 照样会拦住（场景 8 验的就是这个）。

---

### 场景 1：干净工作副本上初始化（回归，不能省）

> 测的是「旧路径没被弄坏」。新功能再好，把原来能用的弄坏了也不行。

**做什么**

1. 造一个全新的 fixture（§1.4）。
2. 应用里 `Open Repo` → 选 `$fx\wc1`。
3. 自动弹出向导，标题 **「把这个 SVN 工作副本交给 git 管」**。
4. 第一栏「**交给 git 管的目录（一行一条）**」清空，填一行 `Code/`。
5. 第二栏「**收哪些扩展名**」填 `.cs`。
6. 点 **「检查工作副本」** → **「下一步」** → **「建库」** → **「关闭」**。

**应该看到**

- 「检查工作副本」后显示**检查通过**，`记账 revision：r1`；
- 「建库」后显示**建好了**、`首个基线：r1(...)`、`当前分支已切到 work`；
- 一条 **「基线已通过体检（r1）」**；
- 图里出现一行 SVN 提交，短 hash 的位置显示 **`r1`**；
- 顶部没有警告横幅，但有一条灰色细条 + **「体检基线」** 按钮。

终端 B 核对：

```powershell
git status --short          # 应当**没有任何输出**
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 向导任何一步报错 | 抄下错误原文 |
| `git status` 有输出 | 干净工作副本上初始化完不该有任何改动 |

---

### 场景 2：**带着本地修改**初始化 ← 核心新行为之一

> 旧版在这里会被 `[svn_preflight_dirty]` 直接拒绝。

**做什么**

1. **重造 fixture**（§1.5）——要回到「还没交给 git 管」的状态。
2. 终端 B，造出三种本地修改：

   ```powershell
   cd $fx\wc1
   Set-Content Code\A.cs -Value 'MY LOCAL WORK' -NoNewline
   Set-Content Code\New.cs -Value 'brand new file' -NoNewline
   svn delete Code\Deep\D.cs
   svn status
   ```

   `svn status` 的实测输出（顺序就是这样）：

   ```text
   M       Code\A.cs
   D       Code\Deep\D.cs
   ?       Code\New.cs
   ```

3. 应用里 `Open Repo` → 选 `$fx\wc1`，按场景 1 同样的填法走完向导。

**应该看到**

- **向导正常走完，不再被拒绝**；
- `记账 revision：r1`、`首个基线：r1(...)`、**「基线已通过体检（r1）」**；
- 关闭向导后，顶部**只有**那条灰色细条（「revision 对得上不代表基线记的内容是对的……」
  + 「体检基线」按钮），**没有**任何黄色或红色横幅。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报 `svn_preflight_dirty` | 你测的是 1c 的构建（回 §0 确认分支） |
| 报「工作副本是 mixed-revision，请先跑一次完整 svn update」 | `Code\New.cs` 那个未提交的新文件把 P2 带歪了——这正是本期修掉的一条 |
| 体检报「基线与服务器不符」 | **严重**：新判据把你自己的本地修改误判成了假基线 |
| **黄色横幅「服务器上有代码文件不在本地」，列着 `Code/Deep/D.cs`** | **假警报**，2026-09-18 已修。那个文件是你自己 `svn delete` 掉的，而且它**就在基线里**（录制时从 BASE 取回来了）；这条提示指控的本来是 `--set-depth=exclude` 那种你不知情的漏收，方向正好相反。又看到就说明修复没生效 |

---

### 场景 3：初始化完，`git status` 恰好是你原有的那三处修改

> 接着场景 2，不要重开。验的是「基线树是服务器内容、磁盘是你的内容，两者故意
> 不同，而 git 把这个差异如实显示出来」。

**做什么**

终端 B：

```powershell
git status --short
Get-Content Code\A.cs           # 必须还是 MY LOCAL WORK
Test-Path Code\Deep\D.cs        # 必须是 False（你删掉的，不许被复活）
Test-Path Code\New.cs           # 必须是 True
```

**应该看到**

`git status --short` 的实测输出**恰好**这三行：

```text
 M Code/A.cs
 D Code/Deep/D.cs
?? Code/New.cs
```

内容检查：`MY LOCAL WORK` / `False` / `True`。

> 从这里开始，`svn status` 会多出一行 `?       .git`——那是应用刚建的 git 仓库，
> SVN 不认识它。**属正常**，后面所有 `svn status` 的输出里都会有它。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| `git status` 是空的 | 基线把你的修改一起录进去了——**这正是会让 Phase 3 静默丢改动的形态** |
| `Code\A.cs` 变回了 `class A {}` | **最严重**：录制过程写了磁盘，等于 `svn revert`。立刻停下并报告 |
| `Code\Deep\D.cs` 又出现了 | 同上 |
| 多出别的行（比如 `Code/Art.psd`） | 白名单扩展名过滤失效 |

---

### 场景 4：基线里存的是**服务器**那一版

> 场景 3 证明「磁盘没被动」，这条证明「基线录对了」。两条缺一不可。

**做什么**

终端 B：

```powershell
"磁盘   : $(Get-Content Code\A.cs -Raw)"
"基线里 : $(git cat-file blob 'refs/heads/svn/base:Code/A.cs')"
git ls-tree -r --name-only refs/heads/svn/base
```

**应该看到**

```text
磁盘   : MY LOCAL WORK
基线里 : class A {}          ← 服务器 r1 的原文
```

`ls-tree` **恰好**是：

```text
Code/A.cs
Code/Deep/D.cs
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 基线里是 `MY LOCAL WORK` | 录的还是磁盘内容，核心改动没生效 |
| 树里有 `Code/New.cs` | 服务器 r1 上根本没有它，不该进基线 |
| 树里**没有** `Code/Deep/D.cs` | 你本地删了它，但服务器上还有——基线记的是服务器，必须有 |
| 树里有 `Code/Art.psd` | 扩展名不在白名单里 |

---

### 场景 5：同事提交 → 失同步 → **带着本地修改**重建，且磁盘分毫未动

> 这就是你最初卡住的那个场景，也是整个分支最关键的一条。

**先回答一个必然会冒出来的问题：我是不是每隔几分钟就得点一次「重建基线」？**

**不是。同事提交多少次，你界面上都不会变。**

失同步判据（`src-tauri/src/engine/svn_sync.rs` 的 `check_sync`）比的是
**你自己工作副本的 revision** 和**基线记的 revision**：

```rust
let baseline_rev = read_baseline_rev(repo_root)?;
let revs = node_revs(repo_root, &cfg.whitelist, &wcs)?;
```

全程 `svn info` / `svn status`，**离线，一次都不问服务器 HEAD**。同事提交只改服务器，
不动你的工作副本——你的 revision 还是 r1，判定 `inSync`，横幅不出现。

**招来横幅的是下面第 2 步你自己跑的 `svn update`**（在 TortoiseSVN 里点更新是同一回事），
不是第 1 步同事那次提交。所以频率 = **你更新的频率**，一天几次，不是几分钟一次。

> 顺带一提：你自己用 TortoiseSVN **提交**之后，已提交的那些文件 revision 会往前跳、
> 其余的不动，这时会看到的是 **mixed-revision** 那条横幅（spec §6.6.3），也不是
> 同事造成的。

**为什么这时候非重建不可？** 基线的含义是「上次和服务器对齐时，服务器上是什么内容」。
你 update 到 r2 之后，基线还写着 r1 —— 那句话现在是**假的**。重建把它改回真的。
不点也不会卡住任何功能（它是提示不是闸门），代价是「相对上次更新我改了什么」的
参照点停在旧位置。

**这一期确实笨，而且是已知的。** Phase 1 里一次 update 之后的完整手工动作是**两步**：
重建基线，**再**把 `work` 跟进到新基线（否则就是下面会看到的、`git status` 里一直挂着
同事的文件）。spec §18 明写了这是 Phase 1 的代价。

**Phase 2 会把这整套合成一个「更新」按钮**，跑 spec §7.3 的 S1–S11：

```text
S1  stash 你的改动
S3  切到 svn/base
S5  svn update
S7  录制新基线          ← 就是本场景手动点的这颗按钮
S8  切回你的分支
S9  rebase 到新基线上    ← 就是本场景之后还得手动做的第二步
S10 stash apply
```

到那时「重建基线」基本只在出异常、要走 §9.4 恢复流程时才用得上。

---

**做什么**

1. **终端 C** 扮演同事。**让他新增一个文件**，不要去改你正在改的那些：

   ```powershell
   cd $fx\wc2
   svn update
   Set-Content Code\Colleague.cs -Value 'class Colleague {}' -NoNewline
   svn add Code\Colleague.cs
   svn commit -m "colleague adds a file"
   ```

   > **为什么必须是「新增」**：你在场景 2 里改了 `Code\A.cs`、删了
   > `Code\Deep\D.cs`。同事若去动这两个文件，下一步的 `svn update` 会产生
   > 冲突（实测会得到 `Tree conflicts: 1`），那是另一条路径，会把这条场景搅浑。

2. **终端 B** 拉下来：

   ```powershell
   cd $fx\wc1
   svn update --accept postpone
   ```

   实测输出应当是干干净净的一条 `A Code\Colleague.cs` + `Updated to revision 2.`，
   **没有 conflict**。

3. 应用里**重新打开仓库**（最稳，见 §2 开头那条）。**先别点按钮**，
   在终端 B 抓一份快照：

   ```powershell
   "=== BEFORE ==="
   svn status
   git status --short
   "hash A.cs = $((Get-FileHash Code\A.cs).Hash)"
   ```

4. 点 **「重建基线」**。确认框里应当提到
   **「你未提交的 SVN 改动不影响重建」**。点确认。

5. 再抓一份：

   ```powershell
   "=== AFTER ==="
   svn status
   git status --short
   "hash A.cs = $((Get-FileHash Code\A.cs).Hash)"
   ```

**应该看到**

第 3 步之前，顶部有红色横幅，标题 **「基线落后于工作副本」**，正文含
`工作副本已经前进到 r2，而基线还记着 r1`，上面有 **「重建基线」** 和
**「体检基线」** 两个按钮。

重建之后：横幅消失，基线走到 r2。

**BEFORE 与 AFTER 三组输出逐字相同。** 实测值：

```text
svn status:              git status --short:
?       .git              M Code/A.cs
M       Code\A.cs         D Code/Deep/D.cs
D       Code\Deep\D.cs   ?? Code/Colleague.cs
?       Code\New.cs      ?? Code/New.cs
```

**为什么重建完了 `Code/Colleague.cs` 还挂在 `git status` 里**

这一条最容易看不懂，先把三个地方分清楚：

```text
                    磁盘（git 和 svn 共用这一份）
                     ↑                    ↑
   git status 比这一对 │                    │ svn status 比这一对
                     ↓                    ↓
              work 分支的 HEAD      .svn\pristine（= 服务器 r2）

              svn/base（基线）  ← 「重建基线」**只动这一个**
```

`git status` 比的是**磁盘 vs `work` 分支的 HEAD**，它**根本不看 `svn/base`**。
而「重建基线」做的事只有一件：把 `svn/base` 指到一个内容等于服务器 r2 的新
commit。它**不写磁盘**（这正是本场景要验的），也**不推进 `work` 分支**
（spec §18 明写，R1–R5 全程不动 HEAD）。

所以 `work` 的 HEAD 仍然停在 r1 的内容——没有 `Colleague.cs`、`A.cs` 是原版、
`Deep\D.cs` 还在。磁盘上则是：`Colleague.cs` 有了（第 2 步 `svn update` 带来的）、
`A.cs` 你改过、`Deep\D.cs` 你删了。两边的差正好就是上面那四行。

**注意 `?? Code/Colleague.cs` 在 BEFORE 里就已经有了。** 把它带上磁盘的是
第 2 步的 `svn update`，不是重建。重建前后这份输出逐字不变，恰恰是本场景的
通过判据——重建什么都没碰。

**为什么不顺手把 `work` 推进过去？** 那是往你自己的历史上写东西：merge 还是
rebase、有未提交改动怎么办、历史里有 merge 提交怎么整理，都得你决定。顺带还
避开了两个麻烦：`work` 上有未提交改动时 `git checkout` 会直接失败，以及
「流程走完忘了切回原分支」。

**什么时候它会消失？** 你自己把 `work` 跟进到新基线之后。Phase 2 的 `sync_in`
会做完整编排（spec §9.4），这一期不做——这一期只负责让基线重新变诚实。

再确认基线录对了：

```powershell
"基线里 : $(git cat-file blob 'refs/heads/svn/base:Code/A.cs')"   # class A {}
git ls-tree -r --name-only refs/heads/svn/base
```

树里应当有 `Code/A.cs`、`Code/Colleague.cs`、`Code/Deep/D.cs`，**没有** `Code/New.cs`。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 确认框里还写着「白名单内必须干净」 | 前端文案没跟上 |
| 点确认后报 `svn_preflight_dirty` | 你测的是 1c 的构建 |
| **哈希变了 / `svn status` 少了几行** | **最严重**：重建写了磁盘。立刻停下，把 BEFORE / AFTER 两段原样贴出来 |
| 报「重建过程中工作副本被外部改动了」 | R3c 触发了。你若确实在这期间动过 `wc1`，重点一次即可；没动过就记下来 |
| 重建之后 `git status` 里还有 `?? Code/Colleague.cs` | **正常**，见上面那段。`git status` 比的是 `work` 分支不是基线，而重建按设计不推进 `work` |
| 重建之后 `git status` **变干净了** | 反而是问题：说明重建推进了工作分支或写了磁盘，两者都超出了这一期的授权 |
| 第 2 步出现 `Tree conflicts` | 同事动的不是新增文件，回第 1 步 |

---

### 场景 6：`~`（obstructed）仍然被拦，且**不自动修复**

> 这是唯一还会阻断的脏状态。必须拦，而且拦住之后不许替用户动手。

**做什么**

> ⚠️ **顺序很重要，别照着上一版做。** 「重建基线」这颗按钮**只挂在失同步横幅上**
> ——同步状态下界面上只有一条灰条加「体检基线」，根本没有它。所以要**先**把失同步
> 造出来，**再**制造 `~`。
>
> 场景 5 做完之后你已经重建过一次、回到同步状态了，所以这里得重新造一次。

1. **终端 C** 让同事再新增一个文件：

   ```powershell
   cd $fx\wc2
   svn update
   Set-Content Code\Second.cs -Value 'class Second {}' -NoNewline
   svn add Code\Second.cs
   svn commit -m "colleague adds another file"
   ```

2. **终端 B** 拉下来。**这一步才是产生失同步的那一步**——同事那次提交本身不会，
   理由见场景 5 开头：

   ```powershell
   cd $fx\wc1
   svn update --accept postpone
   ```

3. 应用里刷新或重新打开仓库，确认红色横幅出现，上面有 **「重建基线」** 按钮。
   **看到按钮了再往下走。**

4. **现在**才制造 obstructed（`svn update` 已经跑完了，不要反过来）：

   ```powershell
   Remove-Item Code\A.cs
   New-Item -ItemType Directory Code\A.cs | Out-Null
   svn status                      # 应当出现一行带 ~ 的
   ```

5. 应用里点 **「重建基线」**。

**应该看到**

- 报错，错误码 **`svn_preflight_obstructed`**；
- 错误信息里**指名 `Code/A.cs`**；
- 引导提到「本应用不会替你动这些路径」，并说明
  **除了 `~`，其余本地修改都不再阻断**。

磁盘没被动：

```powershell
Test-Path Code\A.cs -PathType Container      # 必须仍然是 True
```

**收拾现场**（进入下一场景前）：

```powershell
Remove-Item -Recurse -Force Code\A.cs
svn revert Code\A.cs
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 重建居然成功了 | `~` 的拦截没生效——那种位置上取不到可信的服务器内容 |
| 那个目录被应用删掉了 | **严重**：本应用不做破坏性修复 |
| 错误信息只说「工作副本不干净」 | 文案没更新，会把用户引回一个已经不存在的死胡同 |
| **界面上根本找不到「重建基线」按钮** | 你还在同步状态。这颗按钮只挂在失同步横幅上，先把第 1–3 步做完。这是设计如此，不是 bug——但见下面那条已知缺口 |

> 📌 **2026-09-19 由这条场景牵出并已修的一个缺口**：「重建基线」在界面上**只有
> 失同步横幅一个入口**，而「体检基线」报 **基线与服务器不符**（`Lying`）恰恰发生在
> **revision 完全对得上**的时候——也就是横幅不在的时候。于是体检告诉你基线坏了，
> 界面上却没有任何地方能修它。
>
> 这和 2026-09-16 独立 review 修掉的那条是同一个毛病（体检入口当初也被绑在失同步
> 横幅上，「等于只在不需要它的时候提供它」），当时只解开了体检，没解开重建。
> 本期让 `Lying` 在脏工作副本上第一次真正可达（§16.2），缺口就从理论变成了现实。
>
> **修法**：体检报 `Lying` 时，弹框的确认按钮从「知道了」换成 **「重建基线」**，
> 点了接着弹重建自己那个确认框。`notChecked` 和 `verified` **不给**这个入口
> ——「没验成」不是「坏了」，拿一次网络抖动换一个新基线是另一种伤害。
> 判据在 `src/engine/svnInit.ts` 的 `healthNeedsRebuild`，有 vitest。
> **场景 10 会验到它。**

---

### 场景 7：未提交的新文件**不再**被误报成 mixed-revision

> 实现途中发现的一条连锁问题：放松「必须干净」之后，`svn add` 未提交的节点
> （`revision="-1"`）会让 P2 以为工作副本是多 revision 的。

**做什么**

终端 B：

```powershell
Set-Content Code\Fresh.cs -Value 'class Fresh {}' -NoNewline
svn add Code\Fresh.cs
svn status                      # 应当有一行 A  Code\Fresh.cs
```

应用里点 **「重建基线」**。

> 没有这颗按钮就照 §2 开头那条做：终端 C 新增一个文件并提交 → **终端 B 跑
> `svn update`** → 应用里重新打开仓库。只让同事提交是不够的。

**应该看到**

- 重建**成功**；
- 基线树里**没有** `Code/Fresh.cs`：

  ```powershell
  git ls-tree -r --name-only refs/heads/svn/base | Select-String Fresh
  # 应当**没有输出**
  ```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报「工作副本里有多个 revision，先跑一次完整的 svn update」 | 本期修的就是这条。而且那句引导做了也没用——真实原因是有个没提交的新文件 |
| 基线树里有 `Code/Fresh.cs` | 服务器上不存在的文件进了基线 |

---

### 场景 8：真正的 mixed-revision **仍然**被拦

> 和场景 7 成对。只做场景 7 的话，一个「把 P2 整个删掉」的实现也能通过。

**做什么**

终端 B，把一个子目录退回旧 revision：

```powershell
svn update -r 1 Code\Deep
svn status -v Code | Select-Object -First 8     # 应当看到两个不同的 revision 号
```

**应用里必须「重新打开仓库」，然后再点 「重建基线」。**

> ⚠️ 这一条最容易卡住。子目录退回旧 revision **不会改变工作副本根的 revision**
> （§2 开头有实测输出），所以第一档看不见它，横幅不会自己冒出来——不重新打开
> 仓库的话，你会发现界面上根本没有「重建基线」这颗按钮。
>
> 重新打开仓库之后应当看到 **mixed-revision** 那条横幅，上面带着「重建基线」。

**应该看到**

- 被拒绝，错误码 **`svn_preflight_mixed_revision`**；
- 错误信息里**列出具体是哪几个 revision**；
- 引导说的是「先跑一次完整的 svn update」。

**收拾现场**：

```powershell
svn update
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 重建成功了 | P2 被放松了。它不该放松——取服务器内容用的是 `-r BASE`，而「BASE 就是那个 N」完全依赖工作副本停在单一 revision |
| 错误信息里没有 revision 号 | 用户不知道该更新什么 |
| 重新打开仓库后仍然没有「重建基线」按钮 | 第二档没把 mixed-revision 判出来。抄下 `svn status -v Code` 的前几行 |

---

### 场景 9：脏工作副本上点「体检基线」，能给出**明确结论**

> 1c 里这颗按钮在工作副本脏时永远回答「没法下结论」——和重建那条是同一个死胡同，
> 只是换了颗按钮。

**做什么**

1. 确保工作副本是脏的（接着前面的场景就已经是了，否则）：

   ```powershell
   Set-Content Code\A.cs -Value 'WIP WIP WIP' -NoNewline
   svn status
   ```

2. 应用里点 **「体检基线」**（灰色细条上，或失同步横幅上）。

**应该看到**

- **「基线已通过体检（rN）」**之类的明确结论；
- **不是**「没法下结论 / 未经体检」。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报「白名单内的磁盘内容与基线树不一致……所以这里不下结论」 | 1c 的旧行为，没改到 |
| 报「基线与服务器不符」并列出你改过的那些文件 | **误报**：本地修改被当成了基线撒谎 |

---

### 场景 10：体检抓得住**真的**假基线 ← 必须和场景 9 成对做

> 只做场景 9 的话，一个「永远说通过」的实现也能骗过你。

> ⚠️ **这一条必须在干净的工作副本上做，而且同事要改的是一个干净文件。**
>
> 判据只能对**没被你改过**的文件下结论——你改过的那些，磁盘本来就和服务器不同，
> 体检合法地把它们排除在外（这是设计边界，不是缺陷，见 spec §6.8.7）。
>
> 这份文档最初写错过：让同事去改 `Code\Deep\D.cs`，而那个文件在场景 2 里被你
> 本地删掉了。实测结果是体检报 **通过** —— 一个**假的通过**，照着做会报出一个
> 不存在的 bug。

**做什么**

1. **终端 B**，先把工作副本清干净：

   ```powershell
   cd $fx\wc1
   svn revert -R .
   Remove-Item Code\New.cs, Code\Fresh.cs -ErrorAction SilentlyContinue
   svn status                 # 应当只剩一行 ?  .git
   ```

   > `svn revert -R .` 的作用域被 `cd` 限死在 fixture 的 `wc1` 里。
   > 敲之前确认 `pwd` 真的是 `$fx\wc1`。

2. **终端 C**，让同事改一个**干净**文件：

   ```powershell
   cd $fx\wc2
   svn update
   Set-Content Code\A.cs -Value 'class A { int fromColleague; }' -NoNewline
   svn commit -m "colleague edits A.cs"
   ```

3. **终端 B**：**不要跑 `svn update`**。手工把基线的 revision 号改成服务器那个新
   revision（模拟「基线记错了」或「恢复流程没走完」）：

   ```powershell
   cd $fx\wc1
   $srv  = (svn info --show-item revision (svn info --show-item repos-root-url)).Trim()
   "服务器在 r$srv，而我停在 r$((svn info --show-item revision).Trim())"
   $tree = git log -1 --format=%T refs/heads/svn/base
   $uuid = (svn info --show-item repos-uuid).Trim()
   $url  = (svn info --show-item repos-root-url).Trim()
   $old  = git rev-parse refs/heads/svn/base
   $msg  = "svn: r$srv`n`ngit-svn-id: $url@$srv $uuid"
   $new  = $msg | git commit-tree $tree -p $old
   git update-ref refs/heads/svn/base $new $old
   git log -1 --format=%B refs/heads/svn/base      # 确认 trailer 写的是 @$srv
   ```

   > 这几条只动 fixture 里的 git 分支指针，不碰任何真实仓库。

4. 应用里**重新打开仓库**，点 **「体检基线」**。

**应该看到**

- 报 **基线与服务器不符**，并**指名 `Code/A.cs`**；
- 弹框右下角的确认按钮写的是 **「重建基线」**，不是「知道了」
  （2026-09-19 补的出路，见场景 6 后面那段）；
- 点它之后接着弹**重建基线**自己的确认框，里面仍然写着「不会修改工作目录里的
  任何文件，也不会推进你的工作分支」。这一步做不做都行，但**按钮文字必须是
  「重建基线」**——是「知道了」就说明修复没生效，体检又变回只报丧不给出路。

实测的后端结论是 `Lying { files: ["Code/A.cs"] }`，界面上应当是对应的红色结论 + 文件名。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报「通过」 | 先回头确认第 1 步真的清干净了、第 2 步同事改的确实是干净文件。两条都对还报通过，才是**判据没有牙齿**——而那正是它存在的全部意义：`svn status` 在这种形态下什么都看不见 |
| 报「没法下结论」 | 判据被前置门槛挡掉了 |
| 报「没法下结论」时按钮也变成了「重建基线」 | **反了**。`notChecked` 是「没验成」不是「坏了」，那时不该推着用户去重建 |

---

### 场景 11：行尾一致性（`core.autocrlf`）

> 实现途中查出的一个真 bug：干净文件走 `git add`（按 autocrlf 归一），脏文件走
> `hash-object`（默认不归一），同一个文件在基线里长什么样竟然取决于它当时脏不脏。

**做什么**

1. **重造 fixture**（§1.5），然后在应用里 `Open Repo` → 选 `$fx\wc1`，照场景 1
   走完向导，在**干净**状态下建库。

   > 这就是本场景**第一次**打开仓库。第 2、3 步都只在终端里做，仓库一直开着。

2. **终端 C** 给一个文件设上 `svn:eol-style=native`：

   ```powershell
   cd $fx\wc2
   svn update
   svn propset svn:eol-style native Code\A.cs
   svn commit -m "set eol-style"
   ```

3. **终端 B**：

   ```powershell
   cd $fx\wc1
   svn update
   ```

4. 回应用里**把仓库关掉、再 `Open Repo` 选回 `$fx\wc1`**（第 1 步打开的那个一直
   开着，但第 3 步的 `svn update` 不一定会让横幅自己冒出来，理由见 §2 开头那条）。
   确认红色横幅出现之后，点 **「重建基线」**，然后记下 blob 的 SHA：

   ```powershell
   git rev-parse "refs/heads/svn/base:Code/A.cs"
   ```

   记成 **H1**。

5. 改脏这个文件，再制造一次失同步，然后再点一次 **「重建基线」**：

   ```powershell
   # 终端 B：改脏
   Set-Content Code\A.cs -Value 'dirty now' -NoNewline
   ```

   制造失同步要三步（照 §2 开头那条）：终端 C 新增一个文件并提交 → **终端 B 跑
   `svn update`** → 应用里重新打开仓库。只让同事提交，横幅不会出现。

   ```powershell
   git rev-parse "refs/heads/svn/base:Code/A.cs"
   ```

   记成 **H2**。

**应该看到**

**H1 == H2**（形如 `37bdd2221a71b58576da9d3c2dc0ef0998263652` 的 40 位串）。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| H1 ≠ H2 | 行尾归一没对齐。后果：在脏工作副本上重建一次，这个文件在工作分支上会凭空出现「每一行都改了」的假 diff |
| 第 4 步或第 5 步找不到「重建基线」按钮 | 没重新打开仓库，或者失同步没造出来。见 §2 开头那条 |

---

### 场景 12：整个过程一条 SVN 写命令都没发

> 本期的硬约束：产品代码只读 SVN。

**做什么**

**终端 B**，在应用里做任何操作**之前**和全部做完**之后**各跑一次：

```powershell
svn info --show-item revision (svn info --show-item repos-root-url)
```

**应该看到**

服务器的 revision **只在你自己用终端 C 提交时才变**。应用里的任何操作
（初始化、重建、体检）都不该让它变。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 你没在终端 C 提交过，revision 却涨了 | **最严重**：应用往服务器写了东西。立刻停下并报告 |

---

### 场景 13：**目录**形态的脏状态（2026-09-20 新增）

> 前 12 条里每一种脏都是**文件**形态。2026-09-20 的独立 review 发现：
> `svn status` 对目录级的脏只报目录本身，而 git 的 exclude 规则按扩展名**递归**收
> ——中间整棵子树会掉进缝里。这一条就是补那个缺口的。

**做什么**

1. **重造 fixture**（§1.5），在应用里 `Open Repo` → `$fx\wc1`，照场景 1 在干净状态下建库。

2. **终端 B** 造三种目录形态：

   ```powershell
   cd $fx\wc1

   # ① 未纳入版本控制的目录，里面两层 .cs
   New-Item -ItemType Directory Code\NewDir\Deeper -Force | Out-Null
   Set-Content Code\NewDir\New.cs         -Value 'class N {}' -NoNewline
   Set-Content Code\NewDir\Deeper\Deep.cs -Value 'class P {}' -NoNewline

   # ② svn:ignore 掉的目录，里面有 .cs
   New-Item -ItemType Directory Code\Ign | Out-Null
   Set-Content Code\Ign\G.cs -Value 'class G {}' -NoNewline
   svn propset svn:ignore Ign Code --non-interactive

   # ③ svn delete 掉一个目录（fixture 里的 Code\Deep）
   svn delete --non-interactive -- "Code/Deep@"

   svn status --no-ignore
   ```

   **`--no-ignore` 不能省。** 裸 `svn status` **不显示 ignored 条目**，正好把这条
   场景要测的那一项藏掉。应用自己用的就是带 `--no-ignore` 的那条命令
   （`svn_dirty.rs` 的 `read_dirty`）——被 `svn:ignore` 挡住**不代表 git 不收它**。

   2026-09-20 实测，两条命令逐字如下：

   ```text
   svn status                    svn status --no-ignore
   ?       .git                  ?       .git
    M      Code                   M      Code
   D       Code\Deep             D       Code\Deep
   ?       Code\NewDir           I       Code\Ign        ← 只有右边看得见
                                 ?       Code\NewDir
   ```

   逐行说明：

   | 行 | 什么意思 | 正常吗 |
   |---|---|---|
   | `? .git` | git 自己的目录在 SVN 工作副本里，SVN 不认识它 | 正常，而且它在白名单 `Code/` 之外，代码里没有任何地方会看到它 |
   | ` M Code` | 第二列的 `M` 是**属性**改了——就是你刚才那条 `svn propset svn:ignore` | 正常 |
   | `D Code\Deep` | 被 `svn delete` 的目录。**注意里面的 `D.cs` 一行都没有** | 正常，而且正是缺口的来源 |
   | `I Code\Ign` | 被 `svn:ignore` 挡住的目录。**里面的 `G.cs` 同样一行都没有** | 正常 |
   | `? Code\NewDir` | 未纳入版本控制的目录。**里面那两个 `.cs` 同样一行都没有** | 正常 |

   > **三条目录级的行，里面一个文件都不报**——而 git 的 exclude 规则按扩展名
   > 递归收。这就是 2026-09-20 那两条阻断项的全部根因。

3. 应用里**关掉仓库再 `Open Repo` 选回 `$fx\wc1`**，确认横幅出现后点 **「重建基线」**。

**应该看到**

- 重建**成功**（不是报「基线与服务器不符」）；
- 基线树里**没有**任何来自 ① ② 的文件，**有** ③ 里那个：

  ```powershell
  git ls-tree -r --name-only refs/heads/svn/base
  ```

  | 路径 | 该不该在树里 | 为什么 |
  |---|---|---|
  | `Code/NewDir/New.cs` | ❌ 不该 | 服务器 r1 上不存在 |
  | `Code/NewDir/Deeper/Deep.cs` | ❌ 不该 | 同上，深一层更容易漏 |
  | `Code/Ign/G.cs` | ❌ 不该 | 同上（`svn:ignore` 不代表 git 不收） |
  | `Code/Deep/D.cs` | ✅ 该在 | 你本地删了，但**服务器上还在** |
  | `Code/A.cs` | ✅ 该在 | 本来就有 |

- 磁盘分毫未动：

  ```powershell
  Test-Path Code\NewDir\New.cs     # True，你的新文件还在
  Test-Path Code\Deep               # False，删掉的目录不许被恢复
  ```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 树里有 `Code/NewDir/New.cs` 或 `Code/Ign/G.cs` | **最严重**：服务器上不存在的文件进了一个自称「等于服务器 rN」的树，而且四道检查全绿。这批文件第一次交付时会因为「和基线一样」而不在净变更里——静默漏交付 |
| 树里**没有** `Code/Deep/D.cs`，或重建报「基线与服务器不符」 | 被删目录里的文件没被取回。这会让一个完全合法的 SVN 状态变成死胡同 |
| `Code\Deep` 又回到磁盘上了 | 重建写磁盘了，整条流程的只读前提被破坏 |
| `Code\NewDir\New.cs` 不见了 | 同上，而且丢的是用户还没提交的东西 |

---

## 3. 收尾

### 3.1 删掉 fixture

**先关掉应用里打开的仓库**（或直接关应用），否则文件被占用删不掉。

```powershell
cd ~
Remove-Item -Recurse -Force $fx
```

删的是 `%TEMP%` 下那个随机后缀目录，**跟你任何真实仓库无关**。

顺手看看有没有以前留下的：

```powershell
Get-ChildItem $env:TEMP -Directory -Filter "phase1c_*" | Select-Object FullName
```

### 3.2 停掉应用

终端 A 里 `Ctrl+C`。

### 3.3 记结果

在本文件开头那份清单里把通过的场景勾上（`- [ ]` → `- [x]`）。不通过的写清楚：

```text
场景 5：❌
  期望：BEFORE / AFTER 三组输出逐字相同
  实际：Get-FileHash 的值变了
        BEFORE: A1B2C3...
        AFTER:  D4E5F6...
```

不用自己排查原因。

---

## 附：和 1c 那份清单的关系

| | 测什么 | 文件 |
|---|---|---|
| 1c 清单 | 失同步横幅、SVN log 翻页、混合图、本地补丁标记 | `2026-09-15-svn-phase1c-e2e.md` |
| **本清单** | 脏工作副本上的建库 / 重建 / 体检 | 本文件 |

1c 清单里那节「已知问题：重建基线在真实开发节奏下点不动」**原样保留着**——
它是这个分支存在的全部理由，里面的现象、根因和「为什么不能简单地放行」三段，
正是新做法必须绕开的陷阱。

## 附：这期**不**在范围内的东西

别拿这些来判定失败：

- **`sync_in` / `sync_out`**（在应用里点一下就更新 / 交付）——那是 Phase 2、3。
  这一期更新和交付仍然要回 TortoiseSVN 手工做。
- **本地补丁的摘除与回落**——Phase 3。这一期「仅本地」只是一条标记。
- **白名单外改动的视图与 revert**——Phase 4。
- **Phase 2 的行尾问题**——`sync_in` 的 stash 往返会把没设 `svn:eol-style` 的文件
  还原成 CRLF。已记在 spec §7.3.0，**本期不修**，因为这一期根本没有 `sync_in`。
- **`svn status` / `svn diff` 的「大小 + mtime」捷径盲区**——拿十六进制编辑器改一个
  同长度的字节再把时间戳改回去，体检看不见。这是**既有**盲区，不是本期引入的。
