# Phase 3a `sync_out` 决策日志

计划：`2026-10-07-svn-phase3a-sync-out.md`。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 0 决策日志 + 自检 | ✅ | 本文件 | 基点 `86e82c7`（worktree `.claude/worktrees/svn-phase3a-sync-out`，分支 `worktree-svn-phase3a-sync-out`） |
| 1 journal 字段 + Local-Only 解析 | ✅ | `svn_journal.rs`、`graph.rs`、`tests/svn_journal.rs` | `cargo test --test svn_journal` 14/14；fmt / clippy / 全量 cargo test 绿 |
| 2 交付树 D0.5 | ✅ | `svn_deliver.rs`、`tests/svn_deliver.rs` | `cargo test --test svn_deliver` 10/10；fmt / clippy / 全量 cargo test 绿 |
| 3 前置检查与清单 | ✅ | `svn_sync_out_check.rs`、`svn_sync_check.rs`、`svn_preflight.rs`、`tests/svn_sync_out_check.rs` | `cargo test --test svn_sync_out_check` 11/11；`svn_sync_check` 20/20、`svn_sync_in` 73/73（重构后）；全量绿 |
| 4 D4 登记与撤销 | ✅ | `svn_stage.rs`、`tests/svn_stage.rs` | `cargo test --test svn_stage` 7/7；全量绿 |
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

- **偏离计划**：无。
- **计划的错误**：无。
- **计划没写的坑**：`cargo fmt` 会把测试里单行的 `SvnNode { … }` 字面量拆成多行（超过行宽），计划里的代码块是未格式化的形态，照抄后要跑一次 `cargo fmt`。
- **为什么这么改是安全的**：新字段全部 `default + skip_serializing_if`，2c 写出的旧 journal 照常读（`a_journal_written_before_phase_3a_still_reads` 钉着）；脊梁四件没动。

### Task 2

- **偏离计划（抄自计划）**：
  - B2：D0.5 慢路径不用临时 worktree + cherry-pick，改用 `git merge-tree --write-tree --merge-base=<C^> <cur> <C>` 在对象层重放，再 `commit-tree`。journal 的 `tempWorktree` 本期不写，收尾仍兼容处理。git < 2.40 在慢路径报 `sync_out_git_too_old`。
  - B15：交付 commit 的作者沿用原 commit（`GIT_AUTHOR_*`），提交者是当前用户。
- **实施结果**：照计划代码落地，无改动。M1 复核（本机 git 2.51.2.windows.1）：`merge-tree` 冲突时退出 1，stdout 首行 tree、随后 `<mode> <oid> <stage>\t<path>` 三行，`replay_conflict` 按此解析出 `Code/A.cs` 并点名本地补丁（`an_overlapping_local_only_aborts_and_names_both_commits` 钉着）；作者 / message 保留（`replayed_commits_keep_their_author_and_message`）。
- **计划的错误**：无。
- **计划没写的坑**：无新增。`sync_out_git_too_old`（退出 129）本机无法复现，只靠代码审读；git < 2.40 的用户走快路径不受影响。
- **为什么这么改是安全的**：`plan_deliver` 只写对象（`commit-tree`）不写 ref、不碰磁盘 / 索引 / HEAD（`a_local_only_in_the_middle_is_replayed_out_without_touching_the_disk` 断言 `for-each-ref` 前后相同、`status --porcelain` 为空）；`unpin` 只删 `refs/mygitclient/deliver/<无斜杠名>`，越界报 `sync_out_bad_deliver_ref`；`pin` 以空旧值 CAS，撞上残留报错不覆盖。

### Task 3

- **偏离计划（抄自计划）**：
  - B1：D3 的确认移到流程之外：预览（纯读）→ 用户确认 → 执行时锁内复核；交付树 tree OID 或清单变了报 `sync_out_needs_reconfirm`。
  - B9：D2 发现白名单外变更时拒绝（`sync_out_outside_whitelist`），不静默过滤。
  - B12：pinned 模式在动手前拒绝（`sync_out_pinned_unsupported`），必测第 53 / 55 条推迟。
  - B13：D3.5 合并进执行时锁内复核的那次 `remoteRev` 查询，值写进 `expectedStartRev`。
- **实施结果**：照计划代码落地。`check_git_idle` 加 `code` / `verb` 参数后，`sync_in` 的报错码与文案逐字不变（`svn_sync_check`、`svn_sync_in` 全部测试照旧通过）。B9 由 `a_change_outside_the_whitelist_is_refused_not_filtered` 钉住；B13 的 `remote_head_rev` 在本 Task 只用于预览，执行时复用在 Task 5。
- **计划的错误**：无。
- **计划没写的坑**：无。
- **已知覆盖缺口**：B12 的 pinned 拒绝没有专门测试——与 `sync_in` 共用 `detect_pinned`，判定本身 2c 已覆盖；这里只是多一个出口码。
- **为什么这么改是安全的**：整个模块只读；`the_preview_changes_nothing_and_takes_no_lock` 断言预览前后 ref、git status、svn status、journal、锁全不变。`git_dirty` / `svn_targets` 空白名单直接报错，不会退化成整个工作副本。

### Task 4

- **偏离计划（抄自计划）**：
  - B5：`newParentDirs` = D4 之后 `added` 且磁盘上是目录的条目（`svn status --xml` 不给 `kind`）。
  - B6：撤销 D4 以「当前状态 − `svnStatusBefore`」重算，只取 `added` / `deleted`；`svnDelta` 照常落盘。
  - **对 spec §8.2.6 的补足**：`newParentDirs` = A 文件各级祖先里、D4 之后状态为 `added` 的目录，**含用户事先自己 `svn add` 过的**（不只是差量里的）。只按差量算，用户预先 add 过的新目录不进 target，D5 会被 `E200009` 拒。撤销仍然只撤差量，用户自己的登记不碰（§12.4）。钉在 `a_parent_dir_the_user_had_already_added_still_goes_into_the_commit_but_is_not_undone`。
- **实施结果**：照计划代码落地。M2-E1 / E2 复核：`rm` 后 `svn status --xml` 的 `item` 是 `missing`，`svn delete` 后是 `deleted`；`add --parents` 后父目录是 `added`、XML 无 `kind`（`dir` 靠查磁盘）——`staging_registers_adds_and_deletes_and_names_the_new_parent_dirs` 断言了这些值。含 `@` 的路径靠 `peg_safe` 尾 `@` 正常 add / revert。
- **计划的错误**：无。
- **计划没写的坑**：无新增。
- **已知限制（本期不处理）**：git 不跟踪目录，「删掉一整个目录」在 D1 里是一串文件删除；D4 只 `svn delete` 这些文件，服务器上会留下空目录。
- **为什么这么改是安全的**：`stage` / `svn_revert` / `snapshot` 空目标一律 `sync_out_empty_targets`（必测第 1、2 条）；`svn revert` 不递归、逐条、从深到浅；`deleted` 撤销后只对 `is_file()` 做显式 `remove_file`，不用 `git clean`；撤完与快照逐条比对，不一致报 `sync_out_rollback_incomplete` 且不删 journal。

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
