# Phase 3a `sync_out` 决策日志

计划：`2026-10-07-svn-phase3a-sync-out.md`。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 0 决策日志 + 自检 | ✅ | 本文件 | 基点 `86e82c7`（worktree `.claude/worktrees/svn-phase3a-sync-out`，分支 `worktree-svn-phase3a-sync-out`） |
| 1 journal 字段 + Local-Only 解析 | ⏳ | | |
| 2 交付树 D0.5 | ⏳ | | |
| 3 前置检查与清单 | ⏳ | | |
| 4 D4 登记与撤销 | ⏳ | | |
| 5 编排主路径（无 Local-Only） | ⏳ | | |
| 6 Local-Only 全流程 | ⏳ | | |
| 7 早退与 D5 失败 | ⏳ | | |
| 8 继续 / 放弃 | ⏳ | | |
| 9 D6 / D7a 失败 | ⏳ | | |
| 10 Tauri 命令 | ⏳ | | |
| 11 前端纯逻辑与流程 | ⏳ | | |
| 12 前端接线 + 真实窗口冒烟 | ⏳ | | |
| 13 E2E 清单、spec 修订、收尾 | ⏳ | | |

## 决策条目

（按 Task 分节。每节至少覆盖：偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的。）

### Task 0

- **偏离计划**：计划写的基点是「提交本计划文档的那个 commit（`3309c70`）」。实际开工时 `master` 已前进到 `86e82c7`（多了 PR #55–#58：PR 徽章、squash 识别、0.5.0 版本号、release CI 装 SVN），按 Global Constraints「开工时最新的 `master`」取 `86e82c7`。这几个提交不碰 `svn_sync_*` / `svn_journal` / `svn_baseline`，对本期无影响。
- **计划的错误**：无。
- **计划没写的坑**：无。
- **为什么这么改是安全的**：只建文档。

### Task 1

- **偏离计划**：无（计划本身的偏离从这里起逐 Task 记）。

### Task 2

- **偏离计划（抄自计划）**：
  - B2：D0.5 慢路径不用临时 worktree + cherry-pick，改用 `git merge-tree --write-tree --merge-base=<C^> <cur> <C>` 在对象层重放，再 `commit-tree`。journal 的 `tempWorktree` 本期不写，收尾仍兼容处理。git < 2.40 在慢路径报 `sync_out_git_too_old`。
  - B15：交付 commit 的作者沿用原 commit（`GIT_AUTHOR_*`），提交者是当前用户。

### Task 3

- **偏离计划（抄自计划）**：
  - B1：D3 的确认移到流程之外：预览（纯读）→ 用户确认 → 执行时锁内复核；交付树 tree OID 或清单变了报 `sync_out_needs_reconfirm`。
  - B9：D2 发现白名单外变更时拒绝（`sync_out_outside_whitelist`），不静默过滤。
  - B12：pinned 模式在动手前拒绝（`sync_out_pinned_unsupported`），必测第 53 / 55 条推迟。
  - B13：D3.5 合并进执行时锁内复核的那次 `remoteRev` 查询，值写进 `expectedStartRev`。

### Task 4

- **偏离计划（抄自计划）**：
  - B5：`newParentDirs` = D4 之后 `added` 且磁盘上是目录的条目（`svn status --xml` 不给 `kind`）。
  - B6：撤销 D4 以「当前状态 − `svnStatusBefore`」重算，只取 `added` / `deleted`；`svnDelta` 照常落盘。

### Task 5

- **偏离计划（抄自计划）**：B1 / B13（执行时锁内复核）；B3（D5 失败只在全部错误码属「服务器确定没变」集合时回滚）；B4（`committing` 阶段继续 / 放弃都拒绝）；B10（D8d 无 Local-Only 时 `update-ref` CAS + 一次切分支）；B11（rebase 前的 merge 门槛对 `merge-base(NEWBASE, origHead)..origHead` 做）；B14（`svn commit --force-log`）。

### Task 6

- **偏离计划（抄自计划）**：B10 / B11（见 Task 5）。

### Task 7

- **偏离计划（抄自计划）**：B3 / B4 / B14（见 Task 5）。

### Task 8

- **偏离计划（抄自计划）**：
  - B7：D5 之后的「放弃」= 停止自动收尾：rebase 在进行就 `--abort`；HEAD 不在原分支时，先确认白名单内 W-file 在 svn 看来全部干净再 `checkout -f` 回原分支；删交付 ref 与 journal；已发布的基线留在新位置。
  - B8：D5 之前所有 phase 的「继续」与「放弃」走同一条清理路径；`svn-staged` 先按 B6 撤销登记。
