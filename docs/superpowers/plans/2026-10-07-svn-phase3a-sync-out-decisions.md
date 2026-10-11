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
| 7 早退与 D5 失败 | ✅ | `svn_sync_out.rs`（`back_from_deliver`）、`svn_sync_in.rs`、`tests/svn_sync_out.rs`（+7） | `cargo test --test svn_sync_out` 26/26；全量绿 |
| 8 继续 / 放弃 | ✅ | `svn_sync_out.rs`（`continue_sync_out` / `abort_sync_out`）、`svn_sync_in.rs`、`tests/svn_sync_out.rs`（+10） | `cargo test --test svn_sync_out` 36/36（含 Windows 占用文件用例）；全量绿 |
| 9 D6 / D7a 失败 | ✅ | `tests/svn_sync_out.rs`（+3） | `cargo test --test svn_sync_out` 39/39；全量绿 |
| 10 Tauri 命令 | ✅ | `commands/svn_sync.rs`、`commands/svn.rs`、`lib.rs`、`tests/svn_backend.rs`、`tests/svn_write_guard.rs` | `svn_backend` 29/29、`svn_write_guard` 8/8、`commands_are_async` 1/1；全量绿。`pnpm tauri dev` 实跑在 Task 12 |
| 11 前端纯逻辑与流程 | ✅ | `svnSyncOut.ts`、`svnSyncOutFlow.ts`、`svnJournal.ts`、`svnSync.ts`、`svnSyncIn.ts`（注释）、`toolbarActions.ts`、`repoStore.ts`；测试 `svnSyncOut` / `svnSyncOutFlow`（新）、`svnJournal` / `toolbarActions`（改） | `pnpm build` 过（tsc）；`pnpm test` 36 files / 516 tests 全绿 |
| 12 前端接线 + 真实窗口冒烟 | ✅ | `DeliverDialog.tsx`（新）、`ConfirmDialog.tsx`（导出 `WarnGlyph`）、`Toolbar.tsx`、`BaselineBanner.tsx`、`RepoTab.tsx`、`styles.css`；测试 `DeliverDialog.test.tsx`、`BaselineBanner.test.tsx`（新） | `pnpm build` 过；`pnpm test` 38 files / 527 tests 全绿；`pnpm tauri dev` 真实窗口两次交付（快路径 r2、慢路径 r3）全部核对通过 |
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
- **偏离计划（本 Task）**：`an_out_of_date_commit_is_rolled_back_precisely` 末尾计划用文本 `svn log` + `contains("colleague Code/A.cs")`，改成 `server_msg(&fx, 2) == "colleague Code/A.cs"`（Task 5 引入的 XML 读法；ASCII 消息两种写法等价，统一用不受代码页影响的那个）。
- **实施结果**：7 个用例首跑全部通过——Task 5 的代码已覆盖这些出口，`switching_back…` 与 `an_out_of_date…` 收尾时普通 checkout 没有被挡。Step 3 的 `-f` 退路（`back_from_deliver` + `changed_between`）照计划仍然加上，它的失败测试在 Task 8 的 `a_file_held_open_at_checkout_is_named_and_nothing_is_committed`（计划 Review Focus 第 3 条写「钉在 Task 7」，实际用例在 Task 8 的正文里——计划内部的位置笔误）。
- **B3 全流程复核**：out-of-date 的实际 stderr 含 `E155011`（`commit_rejected` 原样带出）；钩子拒绝含 `rejected by hook`；仓库目录改名时 `sync_out_commit_unknown`，journal 停在 `committing`、`A Code/Deep` 登记保留、服务器仍是 r1。
- **计划的错误**：Review Focus 第 3 条的用例位置写成 Task 7，实际在 Task 8。
- **为什么这么改是安全的（`-f` 退路）**：只有普通 checkout 失败后才考虑 `-f`；`-f` 之前逐个核对「白名单内被跟踪的改动 ∪ 交付树与原分支之间有差异的路径」，每个文件的磁盘字节都必须是交付树或原分支里原样存着的（或已不在），有一个不是就原样返回第一次的错误、不覆盖。`changed_between` 空白名单直接返回空，不发空 pathspec。

### 里程碑 review（Task 1–7，`/code-review high`）

10 条发现，按「用户实际会遇到什么」重新定级：

| # | 发现 | 定级 | 处理 |
|---|---|---|---|
| 1 | A 文件用户事先在 TortoiseSVN 里 `svn add` 过 → D4 再 add 退出 1（W150002 + E200009），交付永远过不去 | 重要 | **已修**：`stage` 跳过快照里已是 `added` 的 A 文件（仍进 D5 target、仍参与父目录计算）。`a_file_the_user_already_added_in_tortoise_is_not_added_twice` RED（实测 W150002 原文）→ GREEN |
| 3 | Windows 上只改大小写的改名 = D 旧 + A 新同一物理文件，D4 的 `svn delete` 会删掉要交付的文件 | 重要 | **已修**：`changed_files` 发现大小写不同的 D/A 对时拒绝（`sync_out_unsupported_change`，指引用 TortoiseSVN Rename）。`a_rename_that_only_changes_case_is_refused` RED → GREEN。Ruling：所有平台都拒（Linux 上本可工作）——服务器仓库也要被 Windows 同事检出，代价是 Linux 用户改大小写也得走 TortoiseSVN |
| 4 | `merge-tree` 不带 `-z` 时按 `core.quotePath` 转义非 ASCII 路径，冲突信息认不出本地补丁、文件名成乱码 | 重要（中文团队常态） | **已修**：`merge-tree --write-tree -z`，按 NUL 解析。**计划没写的坑**：开发机全局 `core.quotepath=false` 会把缺陷藏起来——测试第一次跑竟然通过；测试里显式 `git config core.quotePath true` 后才 RED → GREEN |
| 5 | `Delivered.rev` / `RebaseConflict.rev` 报的是新基线（D6 后）的 revision，同事在 D5–D6 间提交时会指向别人的提交 | 重要 | **已修**：优先用 `j.committed_rev`，读不出时退回 trailer。两条原断言 `rev: 3` 的用例改为 `rev: 2`（并补断言新基线仍是 `@3`），RED → GREEN |
| 9 | `cleanup_before_svn` ① 对 journal 里的 `tempWorktree` 路径 `worktree remove --force`，而本期从不写该字段 | 重要（危险死代码） | **已修**：删掉这一步，字段出现也不碰（只可能来自手改 / 外来 journal）。偏离计划 B2「收尾仍兼容处理」。回归测试放 Task 8（`abort_sync_out` 是第一个能从 journal 走到这里的公开入口） |
| 2 | 本地化的 svn 输出没有英文 `Committed revision`，若「提交落地但退出非零且只有拒绝类错误码」会被误判 Rejected | 次要，推迟 | Ruling：svn 1.14 的 post-commit 钩子失败仍退出 0；退出非零且只带 E165001 的只有 start-commit / pre-commit（提交之前）。场景只在理论上成立；3b 的 opid revprop 查证是正解。代价：若真发生，journal 会停在 `svn-staged` 且撤销报 `sync_out_rollback_incomplete`，不会静默推进 |
| 6 | D6 多个 target 各自解析 HEAD，更新途中有人提交会得到 mixed revision，D7 停下 | 次要，推迟 | 停在 `post-commit`，「继续」重跑 D6 即自愈 |
| 7 | `capture_delivered` / `publish_delivered` 与 `sync_in` 的录制 / 发布近乎逐行重复 | 次要，推迟 | 重构，后续统一 |
| 8 | `checkout_detached` 与 `checkout_verified` 重复；`finish` 与 `commits_between` 各解析一遍 log；`short()` 未复用 | 次要，推迟 | 同上 |
| 10 | `unstage` 每个节点起一个 svn 进程；慢路径预览每次都写松散对象 | 次要，推迟 | 性能 / 垃圾对象，gc 会清 |

验证：`rust-suite.sh` → fmt ok、clippy clean、cargo test 822 passed / 0 failed。

### Task 10

- **偏离计划**：
  - `svn_write_guard` 的新用例除了「挂起 watcher」，照 2c 那条（`sync_commands_suspend_the_watcher…`）同时断言命令调到对应引擎入口、引擎入口在锁内（`run_sync_out_probed` 含 `svn_lock::acquire`，继续 / 放弃走 `take_over(`）。
  - 先看到 RED：`svn_backend` 编译失败（`UnfinishedSync` 缺字段），`svn_write_guard` 找不到 `svn_sync_out` 的函数体。
- **计划的错误**：计划没提到 `UnfinishedSync` 加字段会让 `tests/svn_backend.rs::sync_state_serializes_to_camel_case` 的结构体字面量编译失败。已补字段（`None`，键集合不变），并在同一用例里加一段 `sync_out` / `committing` 带值的断言，钉住 `expectedStartRev`、`commitMessage` 两个键名。
- **计划没写的坑**：无。
- **为什么这么改是安全的**：四个命令都是 `#[tauri::command(async)]`（`commands_are_async` 守着）；三个写命令都包在 `with_watch_suspended` 里，结束后无论成败补发 `repo-changed`；新字段 `skip_serializing_if`，旧前端读不到也不受影响。

### Task 9

- **偏离计划**：`a_new_gitignore…` 的结局断言由计划的 `Delivered { rev: 4 }` 改为 `rev: 2`（我的提交；里程碑 review 第 5 条），另加断言新基线 trailer 是 `@4`、服务器 HEAD 仍是 r4（没有第二次交付）。
- **计划的错误**：`a_d6_conflict_on_a_file_git_does_not_manage_is_handed_to_tortoisesvn` 用 `svn resolve --accept working` 模拟「用户解决冲突」。实测文本文件冲突时 svn 已把 `<<<<<<< .mine … >>>>>>> .r4` 写进工作文件，`working` 原样保留标记，断言 `psd LOCAL` 失败。改成 `--accept mine-full`（「保留我的版本」）。被测的生产代码没有问题：冲突报 `sync_out_update_conflict`、phase 停在 `post-commit`、解决后「继续」走完。
- **实施结果**：其余两条首跑通过。`a_failed_d6…`：仓库不可达时 `sync_out_update_failed` 且文案带「提交已经成功（r2）」，恢复后继续不重复提交。`a_new_gitignore…`：服务器新增 `Code/.gitignore`（`*.cs`）让 D7a 报 `include_set_mismatch` 并点名 `Code/New2.cs`，基线未动；本地把规则改空后继续，从 D7 重录成功——git 按磁盘上的 `.gitignore` 判定，计划担心的「修正后仍不过」没有出现。
- **为什么这么改是安全的**：只改测试。

### Task 8

- **偏离计划（抄自计划）**：
  - B7：D5 之后的「放弃」= 停止自动收尾：rebase 在进行就 `--abort`；HEAD 不在原分支时，先确认白名单内 W-file 在 svn 看来全部干净再 `checkout -f` 回原分支；删交付 ref 与 journal；已发布的基线留在新位置。
  - B8：D5 之前所有 phase 的「继续」与「放弃」走同一条清理路径；`svn-staged` 先按 B6 撤销登记。
- **偏离计划（本 Task）**：
  - `continue_restoring` 的 `rev` 改用 `delivered_rev()`（优先 `committed_rev`），与 `restore_branch` 共用——里程碑 review 第 5 条的延续。`a_d8d_conflict_is_continued_after_the_user_resolves_it` 的期望由计划的 `rev: 3` 改为 `rev: 2`（我的提交 r2，同事 r3）。
  - 新增 `a_temp_worktree_named_in_a_foreign_journal_is_left_alone`：里程碑 review 第 9 条（删掉 `worktree remove --force`）的回归测试，`abort_sync_out` 是第一个能从 journal 走到收尾的公开入口。**这条测试写在修复之后，没有看到 RED**——当时没有公开入口能构造该场景；它守的是「以后别把这一步加回来」。
- **实施结果**：照计划代码落地。B4：`committing` 时继续 / 放弃都报 `sync_out_commit_unknown`，journal 原样。B7：`post-commit` 放弃先跑 D6 拉齐（实测 M2-E6：只提交不更新是 mixed-revision，「更新」与「重建基线」都不接），之后 `check_sync` 判为 `Diverged { node_rev: 2, baseline_rev: 1 }`，「更新」的 A16 接管后分支回到新基线——`abandoning_after_the_commit…` 全程钉住。Windows 占用文件用例（share=Read）在本机按计划预期报 `sync_in_checkout_failed` 并指名 `Code/A.cs`，放开后收尾干净。
- **计划的错误**：无（`rev` 期望值随里程碑 review 修正，见上）。
- **计划没写的坑 / 覆盖缺口**：`publishing-baseline` 但还没有 `newBaselineOid`（崩在写 `captureTree` 之后、建 commit 之前）走「从 D7 重录」分支，没有对应 Step，未覆盖。（里程碑 review 第 10 条补上，见下。）

### 里程碑 review（Task 8–10，`/code-review high`，范围 `2bc16f9..958c527`）

10 条发现，逐条对着代码核实过：

| # | 发现 | 定级 | 处理 |
|---|---|---|---|
| 1 | 继续 / 放弃 `post-commit` 时 D6（svn update）先于「HEAD 还在交付树上」的检查——用户中断期间切回了分支，svn update 会把同事的改动合进那条分支，之后才在 `capture_delivered` 报错 | 高 | **已修**：`require_left_in_place` 在 D6 之前断言 HEAD 在交付树（detached）上，`publishing-baseline` 还接受「在新基线分支、HEAD = newBaselineOid」。报错指引 `git checkout --detach <deliverRef>`。`resuming_after_the_user_left_the_deliver_tree_never_runs_svn_update` RED → GREEN（并验证照指引切回后能续完） |
| 7 | 放弃时 `checkout -f <orig>` 之前只看 W-file 的 svn 状态，中断期间在 detached HEAD 上做的提交会被切丢 | 高 | **已修**：`-f` 之前同样走 `require_left_in_place`。`a_commit_made_on_the_detached_deliver_tree_is_not_thrown_away_by_abort` RED（原先报 `sync_out_unsafe_to_return` 是碰巧，因为 A.cs 在 svn 看来改了）→ GREEN。**范围说明**：用户用 `git add -f` 强行跟踪的白名单内非 W-file、被暂存后又在磁盘还原的改动，仍不在检查范围内——git 在本模型里只跟踪 W-file，这两种都需要用户刻意绕开排除规则 |
| 3 | `restoring-branch` 无本地补丁、HEAD 已被用户切回原分支（仍是 `origHead`）时，`restore_branch` 对**当前**分支 `update-ref`，索引和磁盘留在旧内容，结局报成功——git 看来是「撤掉了同事的改动」 | 高 | **已修**：这种情况先要求原分支上没有被跟踪的改动，再 `checkout_verified` 回基线分支（磁盘写成新基线），然后走原来的 update-ref + 切回。`continuing_after_the_user_switched_back_by_hand_does_not_move_the_branch_under_head` RED（`Code/Col.cs` 不在磁盘上）→ GREEN |
| 2 | 放弃时 HEAD 已在原分支上（D8d 那次 checkout 退出 0 却没写干净，或刚 `rebase --abort`）就跳过残留核对，报「已停止收尾」，磁盘上是撤掉了本地补丁的内容 | 重要 | **已修**：把 `sync_in` 的 `settle_checkout_residue` 核心抽成 `settle_residue(repo, cfg, bases, orig, unsafe_code)`，两边共用（`sync_in` 行为、码、文案不变）。交付侧 bases = 新基线（若有）+ 交付树；不是任何 base 里的字节就 `sync_out_unsafe_to_return`、不覆盖。`abandoning_on_the_original_branch_settles_checkout_residue` RED → GREEN（同时钉住 foreign 字节不被覆盖） |
| 5 | `post-commit` 时有别人的 rebase 进行中，`require_our_rebase` 拿还没有的 `newBaselineOid` 去认，报成 `svn_journal_corrupt` | 重要 | **已修**：`check_rebase`——D8d 只发生在 `restoring-branch`，更早的 phase 里出现的 rebase 一律「不是本次交付发起的」（`sync_out_state_changed`），且判定先于 D6。`sync_in` 抽出 `is_our_rebase(repo, j, onto)` 供交付侧用自己的基线核对调用。`someone_elses_rebase_after_the_commit_is_named_not_reported_as_corruption` RED → GREEN |
| 6 | 交付的继续路径借用了 `sync_in` 的报错码与文案（`sync_in_unresolved`、`sync_in_state_changed`「记录与 stash 原样保留」） | 重要（Task 11 会按码分流） | **已修**：交付侧自带 `unresolved`（`sync_out_unresolved`）、`is_ancestor`（`sync_out_state_changed`），rebase 认领见第 5 条。`a_d8d_conflict_is_continued…` 的断言由 `sync_in_unresolved` 改为 `sync_out_unresolved`。**未改**：`checkout_failed` 仍是 `sync_in_checkout_failed`（Task 8 起就由 `a_file_held_open_at_checkout…` 钉着，文案不提 stash，与流程无关），Task 11 前端把它当通用码处理 |
| 4 | 离线时「放弃」也失败（要先 D6），报错还叫人去点「继续」 | 次要（文案） | **语义不改，文案已改**。理由：不拉齐白名单就切回分支，工作副本停在 mixed-revision；之后无论用户在 TortoiseSVN 手工 update 还是走「更新」接管，同事的改动都会被当成用户的 WIP——比「离线时只能等」更糟。`update_whitelist` 加 `Then::{Continue, Stop}`，放弃时报错说明为什么也要联网、网络恢复后点「继续交付」或「停止收尾」都行。`abandoning_offline_says_it_needs_the_network_too` RED → GREEN |
| 10 | 交付记录里出现认不出的 phase 时报「这条记录是『更新』流程留下的」；`publishing-baseline` 无 `newBaselineOid` 的重录分支无测试 | 次要 | **已修**：新码 `sync_out_unknown_phase`，文案带 phase 线序名。`an_unknown_phase_in_a_delivery_journal_is_not_blamed_on_update` RED → GREEN。补 `a_crash_before_the_new_baseline_commit_is_recaptured_on_continue`（**直接 GREEN**：覆盖缺口，不是缺陷）。报错里提到的「清理」按钮由 Task 11 的横幅提供，不改 |
| 8 | `take_over`、A5 回原分支、rebase 的 `-c` 参数在两个流程里重复 | 次要，推迟 | 本轮只抽了与修复直接相关的两处（`settle_residue`、`is_our_rebase`）。注意 `sync_in::take_over` 没有交付侧那次 opid 复核——是 2c 的既有行为，不在本 PR 范围 |
| 9 | Task 10 注册了命令却没跑 `pnpm tauri dev` | 已知推迟 | 计划本来就把实跑放在 Task 12（那时前端才有入口），Task 12 必做 |

**给 Task 11 的输入**（前端按这些码与按钮名写）：`sync_out_unresolved`、`sync_out_unknown_phase` 是新码；后端文案引用的按钮名是「继续」「清理」「继续交付」「停止收尾」，前端标签要与之一致。

**为什么这么改是安全的**：所有新检查都只会让原来「继续走下去」的路径**提前停下**、journal 原样保留（每条回归测试都断言 phase 不变）；唯一新增的写操作是第 3 条的 `checkout_verified(svn/base)`，它不带 `-f`，前置条件是原分支上没有被跟踪的改动，事后核对切干净，否则报 `checkout_failed`、记录保留。`settle_residue` 只写回 `bases` 里原样存着的字节（逐文件 `hash-object --path` 比对），空列表不发 pathspec。

验证：`cargo fmt --check` ok、`cargo clippy --all-targets` 无告警、`cargo test` 全绿（`svn_sync_out` 47/47，含新增 8 条）。
- **为什么这么改是安全的**：两个入口都先拿锁再在锁内重读 journal，opid 或 flow 变了就停；只接 `flow = sync_out`，否则 `sync_out_wrong_flow`；`sync_in` 的继续 / 放弃照旧拒绝交付 journal（码不变，文案改为指向交付横幅）。D5 之后不重复提交（`every_crash_after_the_commit…` 断言服务器始终只有 r2）；放弃只在 W-file 全部 svn 干净时才 `checkout -f` 回原分支，否则 `sync_out_unsafe_to_return` 保留现场；rebase 只 abort / continue 本次发起的那一个（`require_our_rebase` 核对 head-name / orig-head / onto）。

### Task 11

- **偏离计划**：
  - `svnSyncOutFlow` 的横幅按钮（继续 / 停止 / 清理）执行**失败时也刷新同步状态**（抽成 `refreshing()`，交付对话框的失败路径共用）。计划只在交付对话框的失败路径刷新；横幅按钮失败时 journal 同样可能已推进（继续从 `post-commit` 走到 D7a 停下），不刷新的话横幅停在旧 phase、按钮与文案都错。新增用例「继续失败（比如 D6 又断网）：横幅状态也要刷新」先 RED 后 GREEN。
  - 「停止收尾」在 `post-commit` 的确认文案多一句「要先把白名单更新到最新……所以需要网络」，对应里程碑 review 第 4 条后端语义（放弃也要先 D6）。新增断言钉住，且 `restoring-branch` 不出现这句。
  - `rebaseConflict` 结局的文案把「点『停止』」改成「点『停止收尾』」，与横幅按钮名一致；新增用例钉住三个按钮名与后端报错里引用的一致。
  - 新增「文案里不能有 `**`」的用例覆盖交付对话框模型；`svnJournal` 的同名用例扩到交付全部 9 个 phase。
- **计划的错误**：计划说 `svnJournal.test.ts` 只需改 `NOTE_SYNC_OUT` 那一条旧断言。实际还有一条——「认不出的 phase 也要有话说」拿 `sync_out` / `svn-staged` 当「认不出」，现在它认得了。改为两个方向各测一个真正认不出的组合（`sync_in` 记录里的 `svn-staged`、`sync_out` 记录里的 `stashed`），并断言不给按钮。原「本版本收不了尾」那条改写为「挂交付那一组按钮，绝不挂 sync_in 的」。
- **计划没写的坑**：新 worktree 没有 `node_modules`，`pnpm test` 报 `vitest is not recognized`；`pnpm install --frozen-lockfile`（PowerShell，Bash 工具无网络）后正常，无新增依赖。
- **为什么这么改是安全的**：纯前端展示与编排，不发任何新的写命令；四个 invoke 包装的参数名与 Task 10 的命令签名逐字对齐（`{ path }`、`{ path, expect }`，`expect` 字段由 `svn_backend` 的序列化用例钉住）。`describeUnfinishedSync` 先按 `flow` 分流，交付记录永远拿不到 `sync_in` 的按钮（后端也会以 `sync_out_not_supported` 拒绝）。真实窗口里 invoke 是否打通在 Task 12 验证。

### Task 12

- **偏离计划**：
  - **横幅分发的位置（计划的错误，见下）**：交付按钮在**未收尾横幅**里分发，不在失同步横幅那串 `a.id === 'svnUpdate'` 里。`BaselineBanner` 加类型守卫 `isSyncOutAction`，未收尾横幅按 id 分流到 `onSyncOutAction` / `onSyncAction`。新增组件测试 `BaselineBanner.test.tsx`（3 条），前两条 RED（交付按钮被交给了 `onSyncAction`）→ GREEN。
  - 对话框外观沿用 `ConfirmDialog` 的图标头部（红色警告图标，与删除类确认一致：提交不能撤回），正文与 `.dlg-warning` / `.dlg-progress` 一样缩进 54px；为此 `ConfirmDialog` 导出 `WarnGlyph`。计划的对话框没有头部图标，`.dlg-warning` 自带的 54px 缩进会与其余正文错位。
  - 本地补丁说明用现成的 `.dlg-note`，勾选框用现成的 `.dlg-check`（`accent-color`），不新写 `.deliver-note`；输入框样式照 `.dialog input`；本地补丁列表也限高（120px）；整框 `max-height: 90vh` 可滚，矮窗口里按钮也够得到。
  - `.deliver-dialog` 加 `color-scheme: dark`：真实窗口截图里清单的滚动条是系统默认的白色，在深色框里刺眼（与 `.dlg-check input` 注释里的同一类问题）。加上后经热更新复截，滚动条变深色。**全应用的滚动条都是浅色默认样式**（没有任何 `scrollbar` / `color-scheme` 规则），不在本期范围，列为后续项。
  - 工具栏「更新」与「交付」互相禁用（`svnBusy`）；流程层共用同一个 `running` ref 已经拦住并发，这里让按钮状态看得出来。
  - 组件测试比计划多一条：交付跑起来之后 Esc 与点遮罩都关不掉。
  - `RepoTab` 里 `{deliverDialog}` 渲染在 `{confirmDialog}` 之前，交付结局的提示框（走 `confirm`）盖在上面。
- **计划的错误**：计划 Step 4 让 `BaselineBanner` 在第 159 行附近（失同步横幅的 `svnUpdate` 分发串）加交付按钮的分发。但交付那一组按钮由 `describeUnfinishedSync` 产出，挂在**未收尾横幅**上；那里原本把所有按钮一律 `as SyncActionId` 交给 `onSyncAction`。照计划接线，「清理 / 继续交付 / 停止收尾」会走「更新」的继续 / 放弃，后端以 `sync_out_not_supported` 拒绝——按钮永远点不通。
- **计划没写的坑**：
  - 截图：`Graphics.CopyFromScreen` 截到的是盖在应用上的别的窗口（几乎全白）；改用 `PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT=2)`，WebView2 内容被遮挡也能完整画出。截图脚本只放会话 scratchpad，不进仓库。
  - `pnpm tauri dev` 的进程名是 `app`，窗口会恢复上次打开的标签页（含真实仓库）。冒烟只在 fixture 的 wc1 标签页里操作。
- **真实窗口冒烟（`scripts/e2e-svn-fixture.ps1` 造的 `phase1c_696a6c45`）**：
  1. 初始化（白名单 `Code/`、扩展名 `.cs`）；改 `Code/A.cs` 提交；改 `Code/Deep/D.cs` 勾「仅本地，不交付」提交。
  2. 「交付」：框列出「修改 1 · Code/A.cs」和 1 个本地补丁，「交付」按钮置灰；工具栏「更新」与「交付」都禁用。截图：默认窗口尺寸（2904×1572 物理像素）。头部 / 缩进 / 置灰都正常。
  3. 写「修复跳跃」、Ctrl+Enter → 「已交付 r2」，正文说还有 1 个本地补丁、请保留分支，没有「可以删除」，也没有横幅。
  4. 核对：`svn log --xml -v -r 2` 只有 `M /Code/A.cs`，`<msg>修复跳跃</msg>` 原样；服务器 `D.cs` 仍是 `class D {}`；磁盘 `D.cs` 仍带补丁；`work` = 本地补丁接在 `svn/base`（r2）上；`git status` 干净；没有 journal、交付 ref、提交信息临时文件。
  5. 视觉证据：窗口缩到 800 宽，用 git 在 wc1 造一笔 25 个新文件（长文件名）的提交、叠在本地补丁之上。框内清单限高滚动，长路径没有撑出横向滚动，输入框和按钮都在视野内；滚动条问题见上，修后复截。写「批量新增」交付 → 「已交付 r3」：服务器 26 条路径（`A dir /Code/Many` + 25 个文件，新父目录进了 target），`D.cs` 未交付；`many files` 被重放成新 OID（`bc09ca5` → `b64ead0`，慢路径），本地补丁接回 r3 之上，工作区干净。
  - 观察（非缺陷）：提交图上交付的那笔提交（`class A`）从 r1 分出、并回 r2——新基线的父提交是交付树（spec §8.2.2），图上能看出 r2 交付的是哪一笔。
  - 四个新 invoke 里预览、执行两个已在真实窗口里打通；继续 / 放弃在 Task 13 的 E2E 场景里验证。
- **为什么这么改是安全的**：组件只做渲染与事件绑定，不直接 `invoke`；交付执行期间 Esc / 遮罩 / Cancel 都关不掉对话框（组件测试钉着），避免用户以为取消了而 svn commit 已经发出；按钮分流有组件测试，交付记录永远到不了「更新」的命令。

### Task 12 补充：交付之后的提交图（用户反馈，计划外）

- **起因**：真实窗口两次交付后，用户反馈图「看不懂」：交付过的 `class A`、`many files` 从 r1 分出、悬在旁边，还排在 r3 上面，像没交付；r2 / r3 点开看不到任何文件变化。
- **根因（对照 fixture 的 `git log --graph`）**：git 链是一条直线 `local → B3 → many → B2 → classA → B1`——交付录的新基线以交付树为父（spec §8.2.2）。`spliceHistory` ③ 把基线换成 svn 节点、svn 节点只连上一个 revision，基线 → 交付提交的那条边丢了，交付提交成了没有子节点的「分支」；`[...rewritten, ...svnNodes]` 又让所有 git 行排在 svn 行之前。r2 / r3 没有文件：svn 节点详情本来就不拉 diff（Phase 4），而 `commit:<基线>` 比的是基线和交付树，恰好漏掉交付的内容。
- **用户选定的方案**：A（隐去交付提交）+ 本地两版基线算文件变化。备选 B（画成 merge 边，每次交付多一个分叉）、C（后端改基线父提交，推翻 §8.2.2）未采用。
- **实现**：
  - `spliceHistory(svnRevs, gitCommits, tips)`：**有子节点、子节点全是基线或已隐藏的提交、且没有 ref / HEAD 直接指着**的提交不进图，记到对应 svn 节点的 `deliveredFrom`。还有分支指着、或别的分支从它上面长出来的照常显示。`tips` 由 store 的 `merge` 传入（ref hash + `headHash`）。
  - svn 节点带 `baselineOid`（同 revision 多条基线取最新）与 `diffBase`（**只认图上紧挨着的下一个节点**的基线；它没有基线——同事的 revision——就不给，避免把几个 revision 的改动算到一个头上）。
  - `commitDiffScope`：git 节点 `commit:<oid>`；svn 节点两者都在时 `range:<diffBase>..<baselineOid>`，否则 `null`（原先发 `commit:svn:N`，后端解析失败、错误被吞）。
  - 后端 `read_diff` 新增 `range:<a>..<b>` scope（libgit2 `diff_tree_to_tree` + 改名检测，**只读**，Tauri 命令签名不变）。格式不对报 `bad_diff_scope`——**不能**落到默认的 workdir 分支，那会把工作区改动当成节点的变化（RED 时实测正是如此）。
  - 详情面板：svn 节点显示「录这条基线之前已交付的本地提交」清单；有 `diffBase` 时复用文件树 + diff 布局，注明「相对 rP，按本地两版基线算，只含 git 管的文件」；否则说明前一版没有本地基线、要等 Phase 4。
- **偏离 / 措辞上的取舍**：基线 trailer 记的是 D6 更新到的 revision，不是 D5 的提交 revision；D5–D6 间同事又提交过时，「交付自」会挂在同事那个 revision 上。所以文案写「录这条基线之前已交付的本地提交」，不写「这个 revision 就是这些提交」。要精确对应需要把 `committed_rev` 写进基线 commit，留作后续。
- **测试**：`svnGraph.test.ts` +13（隐藏规则 6、基线字段 3、`commitDiffScope` 4），先 RED 11 条；`fixtures.rs` +2（`range` 跨多个提交、5 种畸形 scope 报错），先 RED。
- **真实窗口**：重启后图为 `D, only local [work] → 批量新增 [svn/base] r3 → 修复跳跃 r2 → r1`，交付提交不再悬挂；点 r2：显示「已交付：79c661c class A」「相对 r1 的变化」、文件树只有 `Code/A.cs`、diff `class A {}` → `class A { int x; }`。
- **为什么这么改是安全的**：前端只改「画不画」与「读哪段 diff」，不改任何 git 数据；隐藏判据保守（被 ref / HEAD 指着、或有非基线子节点的一律显示），最坏情况是该隐藏的没隐藏，不会藏掉用户的分支。后端只加一个只读分支，畸形输入报错而不是退回工作区 diff。
- **验证**：`pnpm build` 过；`pnpm test` 38 files / 540 tests；`cargo fmt --check` 过、`cargo clippy --all-targets` 无告警、`cargo test` 55 套 / 847 passed / 0 failed。

### Task 12 补充：有分支时 SVN 主线被拆成两列（用户反馈，计划外）

- **起因**：用户要求造一个带分支的 fixture 看隐藏规则（之前的冒烟都只有 `work` 一条分支）。复制 `phase1c_696a6c45` 为 `phase1c_branches_25282`（`svn relocate` 到副本 repo），加三种场景：`keep-a` 指着已交付的 `class A`；`grow` 从已交付的 `many files` 长出一个提交；`feature` 从 `svn/base` 拉出一个提交。隐藏规则本身表现正确（`class A`、`many files` 因被 ref 指着 / 有非基线子节点而照常显示），但用户指出图很乱：r1、r2 在蓝色 lane 0，r3 在绿色 lane 1，三个 revision 不在一条直线上。
- **根因**：`collectTips` 的主干级只认 `master` / `main`。SVN 仓库没有它们，lane 0 归行序最靠前的 tip `grow`，它沿第一父链 `grow → many files → r2 → r1` 认领走；`svn/base`（r3）头上骑着以它为第一父的 `feature` / `work`，按「有第一父子节点的不独立成线」被滤掉，r3 随 `feature` 进了 lane 1。只有一条分支时恰好都在 lane 0，所以之前没暴露。
- **改法**：图里有 `svn:` 节点时，主干级改为 SVN 链顶端（行序第一个 `svn:N`），压过 master/main；它沿第一父链认领整条 SVN 序列。
  - 不认 `svn/base`：比基线新、还没更新的同事 revision 在它上面，会被甩出主线（测试钉住）。
  - `svn:` 节点只有 `spliceHistory` 接合过的图才有，普通 git 仓库里叫 `svn/base` 的分支不受影响（测试钉住）。
- **偏离计划 / spec**：`svnGraph.ts` 头注释与 spec §10.4 说 layout「一行都不用改、只认 parents」。现在 `owners.ts` 认 `svn:` 前缀，是唯一的例外，头注释已同步改写。
- **测试**：`layout.test.ts` +3（fixture 的真实拓扑、SVN 链顶端在基线之上、普通仓库的 `svn/base` 不抢 lane 0），前两条先 RED（`[1,0,0]` 等），第三条是护栏。
- **真实窗口**：`phase1c_branches_25282` 重新截图，r3 → r2 → r1 同在 lane 0、同色；`grow` 从 r2 分出（`many files` 在它的线上），`feature`、`work` 从 r3 分出，`keep-a` 从 r1 分出。
- **为什么这么改是安全的**：只改车道归属的起点顺序，不改节点、边与任何 git 数据；对非 SVN 图零影响（无 `svn:` 节点时走原逻辑，原有 owners / layout 测试全绿）。
- **验证**：`pnpm build` 过；`pnpm test` 38 files / 543 tests。

### Task 12 补充：git 与 svn 两条序列没按时间交错（用户反馈，计划外）

- **起因**：同一个带分支的 fixture 上，`keep-a` 指着的 `class A`（06:26:30）排在 r3（06:35:30）、r2（06:30:47）上面。
- **根因**：`spliceHistory` 输出是 `[...git, ...svn]`——所有 git 行在前、svn 行在后，从不交错。之前交付过的提交都被隐藏，图上只剩比基线新的本地提交，所以没暴露；有分支指着已交付提交时它就显示出来、排错位置。
- **改法**：④ 之后用 `dateOrder` 合并，规则同 `git log --date-order`：子一定在父之上，在此前提下越新越靠上，时间相同保持输入顺序（git 在前）。不单纯按时间排：本地或服务器时钟一偏，子会落到父下面。
- **为什么这么改是安全的**：只改行序，不改节点与边。纯 git 仓库不经过 `spliceHistory`（`merge` 首行直接返回），零影响；SVN 模式下 git 提交之间的相对顺序不变——`CommitNode.time` 是 libgit2 的 `commit.time()`（committer 时间），与后端 revwalk 的排序键一致。
- **测试**：`svnGraph.test.ts` +2：fixture 的真实拓扑与时间（先 RED）；子的时钟比父还早时子仍在上（护栏，改前改后都应绿）。
- **真实窗口**：行序 `grow / feature (23:10) → work (06:35:32) → r3 → many files → r2 → class A → r1`，时间单调递减，SVN 主线仍在 lane 0。
- **验证**：`pnpm build` 过；`pnpm test` 38 files / 545 tests。

### Task 12 补充：svn 节点画成菱形（用户提议，计划外）

- **起因**：用户反馈 svn 节点与 git 提交都是实心圆，不好区分。选定方案 A（菱形）；备选圆角方块（小尺寸下与圆差别不明显）、空心圆（与合并提交的环冲突）、行内「SVN」角标（hash 列已显示 `rN`，冗余）未采用。
- **改法**：`svgRenderer.renderNode` 对 `vcs === 'svn'` 画 `<polygon>` 菱形，车道色填充 + 白边，`<g>` 加 `svn` class；半对角线取圆半径的 1.3 倍（面积与圆点相当），HEAD 放大、HEAD / 选中描边加粗与圆点规则一致。不用颜色区分：颜色归车道。
- **偏离 spec**：spec 没规定 svn 节点的样式，§10.4 只说渲染层认 `parents`；现在渲染层认 `vcs`，修订 spec 时补上。
- **为什么这么改是安全的**：只改节点形状，不改布局、行序、边与数据；纯 git 仓库没有 `vcs === 'svn'` 的节点，零影响。
- **测试**：新增 `svgRenderer.test.tsx`（jsdom）4 条：svn 菱形 / git 圆、菱形以车道中心对称、HEAD 放大加粗、选中加粗，先全部 RED。形状好不好认靠真实窗口截图（`phase1c_branches_25282`：r3 / r2 / r1 为蓝色菱形，本地提交仍是圆点，lane 0 的菱形左顶点没被裁）。
- **验证**：`pnpm build` 过；`pnpm test` 39 files / 549 tests。
