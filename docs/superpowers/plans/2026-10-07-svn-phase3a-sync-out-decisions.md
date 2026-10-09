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
| 5 编排主路径（无 Local-Only） | ✅ | `svn_sync_out.rs`、`svn_sync_in.rs`（`checkout_failed` 抽取）、`svn_sync_out_check.rs`、`tests/svn_sync_out.rs` | `cargo test --test svn_sync_out` 14/14；`svn_sync_in` 73/73；全量绿 |
| 6 Local-Only 全流程 | ✅ | `tests/svn_sync_out.rs`（+5） | `cargo test --test svn_sync_out` 19/19；全量绿 |
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

- **偏离计划（本 Task 新增）——D5 的提交信息改走 `-F` 文件**：计划 B14 用 `-m <message> --force-log`。实测（本机 ACP 1252 / OEMCP 437，SlikSvn 1.14.2）svn.exe 的 C 运行库按系统 ANSI 代码页解码命令行：`-m 修复跳跃` 在服务器上**存成了 `Art`**（参数整体错位），`-m "-m 第一行\n第二行"` 存成 `-m ???\n???`。改为把 message 写进 `.git/mygitclient-commit-msg.txt`（相对仓库根，UTF-8），`svn commit -F .git/mygitclient-commit-msg.txt --encoding UTF-8`，提交后删文件；`--force-log` 保留（用户 `svn add --force .` 会把 `.git` 登记成受控，那时 svn 拒绝受控文件当 `-F`，实测 `add --force .` 确实会 `Adding .git`）。ACP 936 的中文系统上 `-m` 本可工作，但 `-F` 与代码页无关。
  - Ruling：`-F` + `--encoding UTF-8` 替代 `-m` —— 提交信息必须原样（Review Focus 1），而 `-m` 在非中文代码页的机器上会静默存错 —— 代价：多一次文件写 / 删；写失败走 `rollback_exit`（D5 之前，服务器未动）。
  - 测试改动：`a_plain_delivery…` 与 `a_message_that_looks_like…` 原用文本 `svn log` + `contains` 断言，文本输出同样按代码页转码（中文变 `?`、日期 `(??, 10 ?? 2026)`），区分不了「存坏了」与「显示坏了」。新增 `server_msg()` 读 `svn log --xml`（恒 UTF-8）并逐字 `assert_eq!`。改测试后先看到 RED（`left: "Art"` / `left: "-m ???\n???"`），再改实现转 GREEN。`assert_settled` 加一条「提交信息文件已删」。
- **计划的错误**：B14 的 `-m` 方案在 ACP ≠ 936 的 Windows 上会存错提交信息（见上）。
- **计划没写的坑**：
  - 「要留意」三处：① `svn log -v` 文本日期乱码确认存在（不断言日期，且 message 改走 XML）；② `art_outside_the_whitelist…` 通过，`record_server_tree_into` 对白名单外的 `Art/x.psd` 本地修改不报错；③ `delivery_works_when_the_whitelist_is_an_external` 通过，D6 把 external 拉齐。
  - **中文文件名在 ACP 1252 机器上不能交付**（同一个命令行转码问题，作用在 svn 的 path 参数上）。实测：`svn add -- 中文.cs@` 退出 1（`W155010 '…\??.cs' not found`）→ D4 `sync_out_stage_failed` → 撤销登记、收尾；已受控的中文文件被修改时 `svn commit -- 中文.cs@` 报 `E200009` → 在 B3 的「确定没变」集合里 → 撤销、收尾。**失败安全、无数据丢失**，但这类机器上带中文文件名的改动交付不了。`--targets FILE` 同样按本地代码页解码，不是解法。本期不处理，记为遗留风险（ACP 936 的中文系统不受影响）。
- **实施结果**：其余照计划代码落地。`checkout_failed` 抽取后 `sync_in` 文案「切到分支 `x`」变为「切到 `x`」，没有测试断言旧文案，`svn_sync_in` 73/73 照旧通过。B1 / B13：`run_sync_out_probed` 在锁内重跑 `check_sync_out`（含 `remote_head_rev`，即 D3.5）并 `verify_expect`，`expected_start_rev` 取锁内查询值。B3：`commit_failures_are_rolled_back_only_when_the_server_surely_did_nothing` 以 M2 原文钉住。B10：无 Local-Only 时 `update-ref` CAS + 从基线切回（`a_plain_delivery…` 断言 `work == svn/base`）。
- **为什么这么改是安全的**：D5 target 经 `commit_targets` 断言非空、且至少有一个文件（必测第 1 条）；`--depth=empty` 只提交 target 自身（`a_stray_add…` 钉住）；D5 状态未知时不回滚不推进；`-F` 的文件在 `.git/` 下，不会进白名单 target。

### Task 6

- **偏离计划（抄自计划）**：B10 / B11（见 Task 5）。
- **实施结果**：5 个端到端用例首跑即全部通过——代码在 Task 2 / 5 已写好，本 Task 只补测试，未改任何生产代码。慢路径重放出的交付 commit 在 D8d 被 `rebase --empty=drop` 吸收，没有出现计划担心的 patch-id 冲突。
- **计划的错误 / 没写的坑**：无。
- **为什么这么改是安全的**：只加测试。T2 冲突用例同时验了「预览拒绝」与「伪造 expect 直接调执行入口也被锁内复核拒绝」，并断言磁盘、HEAD、`refs/mygitclient/`、journal、服务器 revision 全未变。

### Task 7

- **偏离计划（抄自计划）**：B3 / B4 / B14（见 Task 5）。

### Task 8

- **偏离计划（抄自计划）**：
  - B7：D5 之后的「放弃」= 停止自动收尾：rebase 在进行就 `--abort`；HEAD 不在原分支时，先确认白名单内 W-file 在 svn 看来全部干净再 `checkout -f` 回原分支；删交付 ref 与 journal；已发布的基线留在新位置。
  - B8：D5 之前所有 phase 的「继续」与「放弃」走同一条清理路径；`svn-staged` 先按 B6 撤销登记。
