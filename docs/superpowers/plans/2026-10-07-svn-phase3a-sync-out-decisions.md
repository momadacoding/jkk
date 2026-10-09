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

### Task 8

- **偏离计划（抄自计划）**：
  - B7：D5 之后的「放弃」= 停止自动收尾：rebase 在进行就 `--abort`；HEAD 不在原分支时，先确认白名单内 W-file 在 svn 看来全部干净再 `checkout -f` 回原分支；删交付 ref 与 journal；已发布的基线留在新位置。
  - B8：D5 之前所有 phase 的「继续」与「放弃」走同一条清理路径；`svn-staged` 先按 B6 撤销登记。
- **偏离计划（本 Task）**：
  - `continue_restoring` 的 `rev` 改用 `delivered_rev()`（优先 `committed_rev`），与 `restore_branch` 共用——里程碑 review 第 5 条的延续。`a_d8d_conflict_is_continued_after_the_user_resolves_it` 的期望由计划的 `rev: 3` 改为 `rev: 2`（我的提交 r2，同事 r3）。
  - 新增 `a_temp_worktree_named_in_a_foreign_journal_is_left_alone`：里程碑 review 第 9 条（删掉 `worktree remove --force`）的回归测试，`abort_sync_out` 是第一个能从 journal 走到收尾的公开入口。**这条测试写在修复之后，没有看到 RED**——当时没有公开入口能构造该场景；它守的是「以后别把这一步加回来」。
- **实施结果**：照计划代码落地。B4：`committing` 时继续 / 放弃都报 `sync_out_commit_unknown`，journal 原样。B7：`post-commit` 放弃先跑 D6 拉齐（实测 M2-E6：只提交不更新是 mixed-revision，「更新」与「重建基线」都不接），之后 `check_sync` 判为 `Diverged { node_rev: 2, baseline_rev: 1 }`，「更新」的 A16 接管后分支回到新基线——`abandoning_after_the_commit…` 全程钉住。Windows 占用文件用例（share=Read）在本机按计划预期报 `sync_in_checkout_failed` 并指名 `Code/A.cs`，放开后收尾干净。
- **计划的错误**：无（`rev` 期望值随里程碑 review 修正，见上）。
- **计划没写的坑 / 覆盖缺口**：`publishing-baseline` 但还没有 `newBaselineOid`（崩在写 `captureTree` 之后、建 commit 之前）走「从 D7 重录」分支，没有对应 Step，未覆盖。
- **为什么这么改是安全的**：两个入口都先拿锁再在锁内重读 journal，opid 或 flow 变了就停；只接 `flow = sync_out`，否则 `sync_out_wrong_flow`；`sync_in` 的继续 / 放弃照旧拒绝交付 journal（码不变，文案改为指向交付横幅）。D5 之后不重复提交（`every_crash_after_the_commit…` 断言服务器始终只有 r2）；放弃只在 W-file 全部 svn 干净时才 `checkout -f` 回原分支，否则 `sync_out_unsafe_to_return` 保留现场；rebase 只 abort / continue 本次发起的那一个（`require_our_rebase` 核对 head-name / orig-head / onto）。
