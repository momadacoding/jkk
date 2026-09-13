# Phase 1b 实现日志：建库与基线录制

> 配套 `2026-09-14-svn-phase1b-baseline.md`。按 `AGENTS.md` §2.1 建立。
> **review 请先读每个 Task 的「偏离计划」再看 diff**（§13 rubric #12）。

本文件只有两部分：**进度总账**（每完成一个 Task 立即更新）与**决策条目**
（按 Task 分节，四类缺一不可）。

---

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 开工自检与基线数字 | ✅ 完成 | 本文件、`2026-09-14-svn-phase1b-baseline.md` | 改动前基线（新 worktree 内实测）：Rust **316 passed / 0 failed / 28 个二进制**、前端 `pnpm test` **272 passed / 21 files**、`pnpm build` ✓。与计划预期一致 |
| 1 `git_cli` 支持环境变量 | ⬜ 未开始 | | |
| 2 `svn_xml` 三个新解析器 | ⬜ 未开始 | | |
| 3 P1/P2/P3 硬门槛 | ⬜ 未开始 | | |
| 4 P5 externals 钉死检测 | ⬜ 未开始 | | |
| 5 基线录制（临时索引 / trailer） | ⬜ 未开始 | | |
| 6 §6.8 基线体检 | ⬜ 未开始 | | |
| 7 初始化向导编排与回退 | ⬜ 未开始 | | |
| 8 重建基线 R1–R5 | ⬜ 未开始 | | |
| 9 Tauri 命令与向导 UI | ⬜ 未开始 | | |
| 10 收尾验证 | ⬜ 未开始 | | |

---

## 开工自检（`AGENTS.md` §4.2）

```text
- 当前分支 / worktree：worktree-svn-phase1b
    @ .claude/worktrees/svn-phase1b
- 当前分支是否已合并进 master：否，全新分支
- 是否允许在当前位置修改：是，独立 worktree，已用 EnterWorktree 会话级切入
- 目标 worktree 是否在改动前已创建并切入：是
- Get-Location 实测值：C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase1b ✓
- 基点 commit：ca304d6（Phase 1a 合并进 master 的那个 squash commit）
    其父是 310c17d（spec 的最新修订），所以本 worktree 里的 spec 就是最新版
- 是否存在未提交改动：无（建完即查，干净）
```

**上一期的收尾状态**：Phase 1a 的 PR #41 已 squash 合并为 `ca304d6`。
`worktree-svn-phase1a` 那个 worktree 保留在磁盘上未删（其分支因 squash 合并不是
`master` 的祖先，`git branch -d` 会拒绝，删除需要用户单独授权）。

---

## 全局

### 本期的范围是用户在对话里定的

spec §16 的 Phase 1 是一大坨（向导、基线、体检、重建、失同步检测、SVN log、混合图、
本地补丁标记）。2026-09-14 用户选定切法：**本期 = 建库与基线录制**，剩下的「看得见」
那半（失同步横幅、SVN log、混合图、补丁标记）留 Phase 1c。

理由：录制机制（临时索引 → `write-tree` → `commit-tree` → 体检 → `update-ref`）被
首个基线和重建基线共用，拆开会让这段最不能写错的序列被抄两遍；而失同步检测要有基线
才有东西可比，天然排在后面。

### Phase 1a 遗留的窗口验证并进本期

1a 的两个命令（`read_capabilities` / `probe_svn_environment`）是在 `AGENTS.md` §5.1
要求的真实窗口验证**没有执行**的情况下合并进 master 的（PR #41 里那两个复选框一直
空着，没有谎报）。2026-09-14 用户确认并进本期一起验，清单见
`2026-09-13-svn-phase1a-foundation-decisions.md` 的 9.4 一节。

### 从 1a 记账过来的一条

1a 决策日志 §9.8 记了：新版 spec §6.3.1 把纳入集合核对改成「临时索引 + `git add -A`
+ 同一份索引 `write-tree`」，而 1a 的 `actual_set` 是**纯读模拟**（当时写明的偏离）。
当时判断「1a 不录基线，TREE 那条耦合还不存在，先不改」，并记账给 1b。

本期 spec 的 R2 **字面要求** `GIT_INDEX_FILE`，所以临时索引一定会建起来。届时模拟版
就是第二个会分叉的事实源——1a 的 B1 已经证明「模拟 `git add -A`」很容易写错。
**处置写在计划的 Task 5 Step 5：删掉模拟版，核对与录制共用同一个 `TempIndex`。**

---

## Task 0：开工自检与基线数字

**改动前基线（在本 worktree 内实测，不是抄上一期的）：**

| 项 | 值 |
|---|---|
| `cargo test -j 2` | **316 passed / 0 failed / 28 个二进制** |
| `pnpm test` | **272 passed / 21 files** |
| `pnpm build`（含 `tsc`） | 通过 |

三项与计划里写的预期一致，无需修正计划。

#### 偏离计划

无。Task 0 按计划执行。

#### 计划的错误

无。

#### 计划没写的坑

- **新 worktree 里 `node_modules` 不存在，必须先 `pnpm install`。** 实测
  `Test-Path node_modules` → `False`。1a 踩过一次：不装的话 `pnpm test` 会**静默跑
  主工作区**的测试（`@` 别名解析到主 checkout 的 `src/`），拿到一个看起来正常、
  实际测的是另一份代码的数字。计划的 Task 0 Step 2 只写了跑命令，没写这一步。
  **Task 10 的收尾验证同样适用**，换 worktree 之后要重来一次。

- **PowerShell 的 `Select-String` 默认大小写不敏感。** 统计失败行时用
  `-Pattern 'FAILED'` 会把 28 行 `0 failed;` 全部匹配进来，得到「30 个失败」
  这种吓人又错误的结论。核对测试结果时必须加 `-CaseSensitive`，或者直接判
  「非 `test result: ok.` 开头的行数」。本期后面每次统计数字都走后者。

- **`cargo test` 的正常输出里混着两行看着像错误的东西**，是 fixture 故意造的形态，
  不是失败：`error: cannot delete branch 'feat/busy' used by worktree at …`
  （断言 git 会拒绝删被检出的分支）与 `Automatic merge failed; fix conflicts`
  （冲突 fixture）。判读时不要被它们带偏——以 `test result:` 行和退出码为准。

#### 为什么这么改是安全的

Task 0 只读不写：跑测试、记数字、建两个 `docs/` 下的文件。没有任何代码改动，
也没有发出任何 git / svn 写命令。
