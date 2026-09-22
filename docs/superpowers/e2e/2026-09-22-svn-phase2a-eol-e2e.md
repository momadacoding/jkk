# Phase 2a 行尾归一化 手工测试指引

**对应分支：`worktree-svn-phase2a-eol`。** 配套
`docs/superpowers/plans/2026-09-22-svn-phase2a-eol-normalization.md`
与同目录的 `-decisions.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。
> 每一步都写清楚：敲什么命令、点哪里、应该看到什么、看到别的说明什么。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | ✅ 已跑：556 + 365 passed，fmt / clippy 零告警 |
| 下面 3 个真实窗口场景 | ⬜ **未跑** |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [ ] 场景 1：新建库 —— 不该出现迁移横幅
- [ ] 场景 2：旧仓库打开 —— 该出现迁移横幅，且另外两条让位
- [ ] 场景 3：点「重建基线…」—— 横幅消失，两侧都干净

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
- 横幅下面那段说明里提到「不会修改工作目录里的任何文件，也不会推进你的工作分支」；
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
- 框里有一段警告文字，说「不会修改工作目录里的任何文件，也不会推进你的工作分支」，
  以及「不做的话，文件会在被编辑或 update 之后一个一个地无故显示成「已修改」」；
- 确认按钮写着「重建」。

2. 点「重建」。

**应该看到：**

- 按钮变成「正在重建…」，随后弹框关闭；
- **红色横幅消失**；
- 底部的「体检基线」灰条**回来了**；
- 图和历史正常，工作分支没有被动过。

3. 在 PowerShell 里核对：

```powershell
cd <FIX>\wc1
git status --porcelain
svn status
type .git\info\attributes
```

**应该看到：**

- `git status --porcelain` **完全为空**；
- `svn status` 只有 `? .git` 一行；
- `.git\info\attributes` 的内容是两行：一行 `# 由 mygitclient 自动生成…` 的注释，
  一行 `* -text`。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 点了没反应 / 报错 | `svn_migrate_eol` 的参数名或注册有问题 |
| 横幅没消失 | 迁移成功了但 `refreshSvnEol` 没被调用，状态没刷新 |
| `git status` 里冒出一堆 `M` | 基线重录了但索引没对齐——这是本期最该抓的回归 |
| `attributes` 文件不在 | 迁移把规则回滚掉了，说明重建那一步其实失败了（但界面没报错？那是更严重的问题） |

4. **再点一次**没有入口了——横幅已经消失。这是对的：规则已装时后端会拒绝重复迁移
   （`svn_eol_already_migrated`），因为重建会移动基线指针，没理由时不该动它。

---

## 4. 全程看 console

整个过程中打开 devtools（应用窗口里 F12 或右键检查），确认：

- 没有红色的 `invoke` 报错；
- 没有「missing required key」之类的参数错误。

这是本期真实窗口验证的**主要目的**——那层 `snake_case` ↔ `camelCase` 转换出错时，
console 往往是唯一会说话的地方。

---

## 5. 收尾

```powershell
Remove-Item -Recurse -Force <FIX>
```

把上面清单里通过的场景勾上（`- [ ]` → `- [x]`）。不通过的写清楚：

- 你做了什么（第几步）
- 期望看到什么
- 实际看到什么（截图或 console 原文）
