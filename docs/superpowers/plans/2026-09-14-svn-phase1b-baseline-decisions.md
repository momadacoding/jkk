# Phase 1b 实现日志：建库与基线录制

> 配套 `2026-09-14-svn-phase1b-baseline.md`。按 `AGENTS.md` §2.1 建立。
> **review 请先读每个 Task 的「偏离计划」再看 diff**（§13 rubric #12）。

本文件只有两部分：**进度总账**（每完成一个 Task 立即更新）与**决策条目**
（按 Task 分节，四类缺一不可）。

---

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 开工自检与基线数字 | 🟡 进行中 | 本文件 | 待填 |
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

#### 偏离计划

（待填）

#### 计划的错误

（待填）

#### 计划没写的坑

（待填）

#### 为什么这么改是安全的

（待填）
