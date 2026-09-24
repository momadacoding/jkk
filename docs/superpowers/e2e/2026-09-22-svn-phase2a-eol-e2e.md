# Phase 2a 行尾归一化 手工测试指引

**对应分支：`worktree-svn-phase2a-eol`。** 配套
`docs/superpowers/plans/2026-09-22-svn-phase2a-eol-normalization.md`
与同目录的 `-decisions.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。
> 每一步都写清楚：敲什么命令、点哪里、应该看到什么、看到别的说明什么。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | ✅ 已跑：567 + 369 passed，fmt / clippy 零告警 |
| 下面 4 个真实窗口场景 | 🟡 **1、4 已通过；2、3 因文案改动需再看一眼**（详见下方） |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [x] 场景 1：新建库 —— 不该出现迁移横幅（2026-09-24 通过）
- [ ] 场景 2：旧仓库打开 —— 该出现迁移横幅，且另外两条让位
      （行为部分 2026-09-24 已通过；**只剩横幅说明文字这一条要重看**）
- [ ] 场景 3：点「重建基线…」—— 横幅消失，两侧都干净，**且工作分支跟上了新基线**
      （行为部分 2026-09-25 已通过；**只剩确认框警告文字这一条要重看**）
- [x] 场景 4：分支上有本地提交时 —— 必须拒绝，且一个字节都不写（2026-09-25 通过）

> **场景 3 为什么跑了两次。** 2026-09-24 的独立 review 抓到一个阻断问题：迁移原先
> 只重录了基线，没有把**工作分支 tip 和真实索引**一起搬过去，于是真实的 Phase 1
> 仓库迁移完仍然是脏的。修掉之后迁移的行为变了，9-24 那次跑的是旧行为，因此
> 在 9-25 按新判据（第 3 步的 `rev-parse` 比对 + 第 4 步打掉 stat cache）重跑。
> 场景 4 是同一轮修复新增的拒绝路径。
>
> 场景 1、2 不受那次改动影响，保留 9-24 的结论。
>
> **第二轮 review（2026-09-25，PR #45）之后为什么 2、3 还要再看一眼。** 那轮
> 抓到三条：`read-tree` 失败时工作分支没回滚、重建窗口内新产生的暂存会被冲掉、
> **以及横幅和确认框仍在声称「不会推进你的工作分支」——那已经与修复后的后端
> 相反**。前两条是后端失败路径，由 Rust 测试确定性覆盖（手工没法稳定复现
> `.git/index.lock` 竞态）；第三条改的是用户在按下按钮前唯一能读到的那段字，
> 必须用眼睛确认一次。迁移成功后的最终状态没有变化，所以 2、3 的**行为**结论
> 仍然有效，要重看的只有文字。
>
> 两条新 Tauri 命令（`svn_eol_status` / `svn_migrate_eol`）的
> `snake_case` ↔ `camelCase` 转换由这两轮实跑共同确认。

---

## 0. 这一期在验什么

这一期关掉了 git 在混合仓库里的**行尾转换**。

不关的话：`core.autocrlf=true`（你这台机器就是）下，git 每次往磁盘写文件都会把
LF 换成 CRLF，而 SVN 那边存的可能是 LF。于是 git 动过的文件对 svn 立刻变「脏」
——内容一个字符没改，整个文件的行尾全翻了一遍。Phase 2 的 `sync_in` 每一步都要
由 git 往磁盘写文件，所以这是它的前置。

**要验的核心是两条 Tauri 新命令真的通**（`svn_eol_status` / `svn_migrate_eol`）。
Tauri 把 Rust 的 `snake_case` 参数名转成 JS 的 `camelCase`，这层转换**没有编译期
检查**，写错只会在运行时静默失败——自动化测试一条都抓不到，只能在真实窗口里点。

---

## 1. 准备

### 1.1 起应用

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2a-eol
pnpm tauri dev
```

首次在这个 worktree 里跑，Rust 那边要编译一会儿。

> ⚠️ **必须在这个 worktree 里跑，不是主仓库。** 主仓库的 `master` 上没有这一期的
> 代码，在那儿跑什么也验不到。

### 1.2 造 fixture

**另开一个** PowerShell 窗口：

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2a-eol
pwsh -File scripts\e2e-svn-fixture.ps1
```

脚本最后一行会打印 fixture 路径，形如
`C:\Users\liuhao\AppData\Local\Temp\phase1c_xxxxxxxx`。
下面统一把工作副本记作 `<FIX>\wc1`。

**整个 fixture 是一次性的**，测完整个目录删掉，对你的机器不留痕迹。所有 git / svn
写操作都只发生在它里面——`AGENTS.md` 开篇那条最高优先级约束要求如此。

### 1.3 需要「回到初始状态」时

一律**重造 fixture**（重跑 1.2），不要试图在旧的上面清理。

---

## 场景 1：新建库 —— 不该出现迁移横幅

新建的库天然带着规则，所以不该提示迁移。这条同时验 `svn_eol_status` 通不通
——它在打开仓库时就会被调用。

1. 在应用里打开 `<FIX>\wc1`。
2. 走完初始化向导（白名单填 `Code/`，扩展名默认）。

**应该看到：**

- 建库成功，界面正常；
- **没有**任何红色横幅说「基线是在旧的行尾设置下录的」；
- 底部有一条灰条，带「体检基线」按钮（那是常驻入口，本来就该在）。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 出现了迁移横幅 | 建库时没装上规则，或者装规则排在了录基线之后。Task 2 的顺序错了 |
| 界面报「参数缺失」之类的错 | `svn_eol_status` 的 `camelCase` 参数名对不上（本期要抓的就是这个） |
| 连灰条都没有 | `needsStandaloneCheck` 的新参数把常驻入口误关了 |

3. 顺手确认磁盘没被动过。在第二个 PowerShell 窗口里：

```powershell
cd <FIX>\wc1
git status --porcelain
svn status
```

`git status --porcelain` 应该**完全为空**；`svn status` 应该只有一行 `? .git`
（`.git` 对 svn 来说是未版本控制的文件夹，这是常态，不是问题）。

---

## 场景 2：旧仓库打开 —— 该出现迁移横幅

模拟「Phase 1 建的库」：把规则文件删掉，等价于建库时根本没写过。

1. **关掉应用**（不是只关标签页——要让它重新读一次仓库状态）。
2. 在 PowerShell 里删掉规则文件：

```powershell
Remove-Item <FIX>\wc1\.git\info\attributes
```

3. 重新 `pnpm tauri dev`，打开 `<FIX>\wc1`。

**应该看到：**

- 一条**红色**横幅，标题是 **「基线是在旧的行尾设置下录的」**；
- 横幅上**只有一个按钮**：「重建基线…」；
- 横幅下面那段说明里提到「工作目录里的文件一个都不会动；工作分支和暂存区会跟着
  新基线一起前移一笔」；
- **底部那条常驻的「体检基线」灰条不见了**（它让位了——迁移没做完时基线本来就
  不可信，那时体检给出的结论会误导人）。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 没有横幅 | `svn_eol_status` 没被调用，或者它错误地报了「不需要迁移」 |
| 横幅在，但「体检基线」灰条也还在 | `needsStandaloneCheck` 的让位判据没生效 |
| 同时出现两条红条 | 失同步横幅没让位（不过这个 fixture 本来就是同步的，正常不会有第二条） |

---

## 场景 3：点「重建基线…」—— 横幅消失，两侧都干净

这条验 `svn_migrate_eol` 通不通，以及它真的把基线对齐了。

1. 接着场景 2，点横幅上的「重建基线…」。

**应该看到：**

- 弹出确认框，标题 **「重建基线以对齐行尾」**；
- 框里有一段警告文字，必须同时说到这三件事（2026-09-25 review 前它说的是反话，
  见下面的「看到别的说明」表）：
  1. 「工作目录里的文件一个都不会动」；
  2. 「工作分支和暂存区会跟着新基线一起前移一笔」；
  3. 「会接管 `.git/info/attributes`，原有内容备份成 `attributes.bak`」；
- 确认按钮写着「重建」。

2. 点「重建」。

**应该看到：**

- 按钮变成「正在重建…」，随后弹框关闭；
- **红色横幅消失**；
- 底部的「体检基线」灰条**回来了**；
- 图和历史正常；工作分支上**多了一笔**——它跟着新基线一起前移了
  （下一步用 `rev-parse` 核实）。

3. 在 PowerShell 里核对（**第 3 条是这次新增的，最关键**）：

```powershell
cd <FIX>\wc1
git status --porcelain
svn status
type .git\info\attributes
git rev-parse HEAD
git rev-parse refs/heads/svn/base
```

**应该看到：**

- `git status --porcelain` **完全为空**；
- `svn status` 只有 `? .git` 一行；
- `.git\info\attributes` 的内容是两行：一行 `# 由 mygitclient 自动生成…` 的注释，
  一行 `* -text`；
- **最后两条 `rev-parse` 输出的哈希完全相同** ← 工作分支必须跟着新基线一起前移。

4. **再把 stat cache 打掉，确认不是假干净。** 这一步不能省：git 的 stat cache 会让
   刚迁移完的 `git status` 即使有问题也先报干净，要等文件 mtime 被碰一下才冒出来
   （spec §7.3.0）。所以要主动碰一遍：

```powershell
cd <FIX>\wc1
Get-ChildItem -Recurse Code -Filter *.cs | ForEach-Object { $_.LastWriteTime = Get-Date }
git update-index --refresh
git status --porcelain
```

**最后那条 `git status --porcelain` 必须仍然完全为空。**

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 点了没反应 / 报错 | `svn_migrate_eol` 的参数名或注册有问题 |
| 横幅没消失 | 迁移成功了但 `refreshSvnEol` 没被调用，状态没刷新 |
| 两条 `rev-parse` 哈希**不同** | 工作分支没跟上新基线——正是 2026-09-24 review 抓到的那个阻断问题 |
| 警告文字里出现「不会推进你的工作分支」 | 文案退回了旧版本。后端会把工作分支和索引一起搬走，这句话是反的（2026-09-25 review 抓到）|
| 警告文字里没提 `.git/info/attributes` | 少了一条披露：迁移会接管那个文件，用户原有的 filter / diff / merge 规则会失效 |
| 第 3 步干净，但第 4 步打掉 stat cache 后冒出 `M` | 同上。这就是那个「潜伏」故障的真面目 |
| `attributes` 文件不在 | 迁移把规则回滚掉了，说明重建那一步其实失败了（但界面没报错？那是更严重的问题） |

5. **再点一次**没有入口了——横幅已经消失。这是对的：规则已装时后端会拒绝重复迁移
   （`svn_eol_already_migrated`），因为重建会移动基线指针，没理由时不该动它。

---

## 场景 4：分支上有本地提交时 —— 必须拒绝，且一个字节都不写

**2026-09-24 新增。** 工作分支上有自己的 git 提交时，那些 commit 里记的内容是旧的
行尾形状，光移动分支指针修不了；要修就得逐个文件改写你已有提交的内容，而判断目标
行尾形态只能靠猜——猜错就是把你的历史改坏。所以本期**在动任何东西之前**拒绝。

1. **重造 fixture**（§1.3），在应用里打开、走完初始化向导。
2. 关掉应用，删规则文件造出「旧仓库」：

```powershell
Remove-Item <FIX>\wc1\.git\info\attributes
```

3. 在 PowerShell 里，在工作分支上造一笔本地提交：

```powershell
cd <FIX>\wc1
Add-Content Code\A.cs "// my local change"
git add Code/A.cs
git commit -m "my local change"
git rev-parse HEAD                      # 记下来，最后要比
git rev-parse refs/heads/svn/base       # 记下来，最后要比
```

4. 重新 `pnpm tauri dev`，打开 `<FIX>\wc1`，点迁移横幅上的「重建基线…」，
   在确认框里点「重建」。

**应该看到：**

- **确认框不关闭**，框里原地显示一条红色错误；
- 错误文字里说清了「分支 `work` 上有 **1 笔**本地提交还没进基线」；
- 后面解释了为什么不能迁移，以及这件事留给后续的 `sync_in`；
- 关掉框之后，迁移横幅**还在**（什么都没发生，状态没变）。

5. 在 PowerShell 里确认**一个字节都没写**：

```powershell
cd <FIX>\wc1
Test-Path .git\info\attributes          # 必须是 False
git rev-parse HEAD                      # 必须与第 3 步记下的一致
git rev-parse refs/heads/svn/base       # 必须与第 3 步记下的一致
```

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 迁移**成功**了 | 前置检查没生效——这会让分支和基线之间留下一批纯行尾差异，Phase 3 会把它们提交进 SVN |
| `.git\info\attributes` 存在了 | 拒绝路径写了规则文件，仓库停在半迁移状态 |
| 两个哈希有任何一个变了 | 拒绝路径动了不该动的东西 |
| 错误说的是「落后基线 N 笔」 | 成因判反了（那是另一种情形：基线被单独重建过而分支没跟上） |

---

## 每个场景都要：全程看 console

整个过程中打开 devtools（应用窗口里 F12 或右键检查），确认：

- 没有红色的 `invoke` 报错；
- 没有「missing required key」之类的参数错误。

这是本期真实窗口验证的**主要目的**——那层 `snake_case` ↔ `camelCase` 转换出错时，
console 往往是唯一会说话的地方。

---

## 收尾

```powershell
Remove-Item -Recurse -Force <FIX>
```

把上面清单里通过的场景勾上（`- [ ]` → `- [x]`）。不通过的写清楚：

- 你做了什么（第几步）
- 期望看到什么
- 实际看到什么（截图或 console 原文）
