# Phase 2c 实现决策日志

> `AGENTS.md` §2.1 要求的落库记录。配套计划：`2026-09-25-svn-phase2c-sync-in.md`
> 分支：`worktree-svn-phase2c-sync-in`，基点 `1db8210`（计划本身提交为 `e334a8a`）

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 0 | 决策日志 + 自检 | ✅ 完成 | 本文件 | 自检见下「Task 0」；`pnpm install` 成功 |
| 1 | 2b 遗留：watcher RAII、锁释放重试、journal tmp 清理、持锁时长 | ✅ 完成 | `commands/watch.rs`、`svn_lock.rs`、`svn_journal.rs` + 5 条测试 | RED 已见（2 编译失败 + 1 断言失败）；3 条变异各自只红对应测试；fmt / clippy 0 告警 / `cargo test` 608 passed 0 failed |
| 2 | 前置检查 | ✅ 完成 | `svn_xml::parse_update_status`、新模块 `svn_sync_check.rs`、fixture `hybrid_with_colleague` + 17 条测试 | RED 已见（两个测试文件编译失败）；4 条变异各自只红对应测试；fmt / clippy 0 告警 / `cargo test` 625 passed 0 failed |
| 3 | WIP 压栈与取回 | ✅ 完成 | `svn_journal`（`ApplyingStash`、`stash_applied`）、新模块 `svn_sync_stash.rs` + 14 条测试、spec §7.3 S10 注记 / §7.6 phase 表 | RED 已见（两个测试文件编译失败）；8 条变异各自红对应测试；fmt / clippy 0 告警 / `cargo test` 639 passed 0 failed |
| 4 | 基线录制零件拆分 | ✅ 完成 | `svn_baseline::{create_baseline_commit, move_baseline_ref}`、`read_baseline` 改 `pub(crate)`；`svn_init::record_server_tree_into` + 5 条测试 | RED 已见（两个测试文件编译失败）；fmt / clippy 0 告警 / `cargo test` 644 passed 0 failed |
| 5 | 编排主路径 S0–S11 + 冲突出口 | ✅ 完成 | 新模块 `svn_sync_in.rs`、fixture `hybrid_whitelist_in_external` + 11 条测试 | RED 已见（模块不存在，编译失败）；5 条变异中 3 条红对应测试、2 条「覆盖不到」（见下）；fmt / clippy 0 告警 / `cargo test` 655 passed 0 failed |
| 6 | 继续 / 放弃（含回原分支） | ✅ 完成 | `svn_sync_in::{continue_sync, abort_sync, back_to_orig, Disk}` 等 + 24 条测试（计划 23 + 补 1） | RED 已见（编译失败）；5 条变异各自红对应测试（第 3 条靠补的用例）；fmt / clippy 0 告警 / `cargo test` 679 passed 0 failed |
| 7 | S3 / S4 / S5 失败回退 | ✅ 完成 | 四个 `on_*` 改为 `rolled_back` 回原分支；`Disk::GitOnly` 加回 + 5 条测试（计划 4 + 补 1 条树冲突） | RED 已见（4 条错误码对但未回退）；3 条变异各自红对应测试（第 3 条靠补的树冲突用例）；fmt / clippy 0 告警 / `cargo test` 684 passed 0 failed |
| 7A | 外部整体 update 之后由「更新」接管（A16） | ✅ 完成 | `SyncInPreview.catch_up_rev`、`check_sync_in` 放行整体前进、`svn_sync_in::catch_up` + 9 条测试（替换 Task 2 的 1 条） | RED 已见（`catch_up_rev` 不存在）；5 条变异各自红对应测试；fmt / clippy 0 告警 / `cargo test` 692 passed 0 failed |
| 8 | Tauri 命令 | ✅ 完成 | 新文件 `commands/svn_sync.rs`（4 个命令）+ 注册 + 2 条测试 | RED 已见（guard 测试因文件不存在失败）；2 条变异各自红对应测试；fmt / clippy 0 告警 / `cargo test` 695 passed 0 failed；`pnpm tauri dev` 实跑留到 Task 10 |
| 9 | 前端纯逻辑 | ✅ 完成 | 新模块 `src/engine/svnSyncIn.ts`；`svnJournal.ts`（`applying-stash`、横幅动作、`sinceText`）、`svnSync.ts`（整体前进降黄、`svnUpdate`）、`toolbarActions.ts`（`showsSvnUpdate`）+ 测试 | RED 已见（6 条失败 + 1 个模块缺失）；5 条变异各自红对应测试（第 3 条靠加强的用例）；`pnpm build` 通过 / `pnpm test` 430 passed |
| 10 | 前端接线 | ✅ 完成（真实窗口冒烟随 E2E 8 个场景于 2026-09-30 通过） | `repoStore.ts` 四个 invoke、`Toolbar.tsx`「更新」、`BaselineBanner.tsx` 继续 / 放弃与「更新…」、`RepoTab.tsx` 编排、`styles.css` 的 `.dialog-message` 换行 | `pnpm build` 通过 / `pnpm test` 430 passed；`pnpm tauri dev` 场景 1 / 2 **未跑**（需人工操作真实窗口） |
| 11 | E2E + spec 修订 + 收尾 | 🟡 清单、spec、真实窗口 E2E（8 个场景，2026-09-30 用户跑完）完成；最新几轮修复的独立 review 未做 | `docs/superpowers/e2e/2026-09-25-svn-phase2c-e2e.md`（8 个场景）、spec §7.2 注记 / §7.3 S3·S5·S8 注记 / §18「十四」 | 全量验证见下表；E2E 机械检查两条通过；独立 review 见「Task 11」 |
| 12 | PR #49 review（第二轮）修复 | ✅ 完成 | `svn_sync_check::current_branch`、`svn_sync_in`（`untracked_leftovers`、`require_our_rebase`、`back_outcome`、`continue_rebase` 重走 S9）、journal `leftOrig` / `rollingBack`、`Released.rolledBack`、前端文案 + 12 条 Rust / 2 条 vitest | 见下「PR #49 review（第二轮）」；fmt / clippy 0 告警 / `cargo test` 711 passed 0 failed；`pnpm build` 通过 / `pnpm test` 433 passed |
| 13 | PR #49 review（第三轮）修复 | ✅ 完成 | `require_restore_target`、`require_our_rebase` 加 `onto` / `orig-head`、`svn_sync_stash::blocking_ancestor`、`UpdateStatusEntry.repos_props` + `is_conflicted` / `is_incoming`、`SyncInExpect.orig_branch`（前后端）+ 10 条 Rust / 1 条 vitest 改写 | 见下「PR #49 review（第三轮）」；fmt / clippy 0 告警 / `cargo test` 721 passed 0 failed；`pnpm build` 通过 / `pnpm test` 433 passed |
| 14 | PR #49 review（第四轮）修复 | ✅ 完成 | `published_baseline`（重放目标 / 认 rebase / 放回复核都以 `newBaselineOid` 为准）、`commands/svn.rs::refuse_while_unfinished`、`RestoreStep::Kept` + `SyncInOutcome::Settled`（冲突后继续不 drop）、前端 `settled` 文案与「继续」确认框 + 5 条 Rust 新增 / 1 条改写、2 条 vitest；修正第三轮 heredoc 写坏的一处测试字面量 | 见下「PR #49 review（第四轮）」；fmt / clippy 0 告警 / `cargo test` 726 passed 0 failed；`pnpm build` 通过 / `pnpm test` 435 passed |
| 15 | PR #49 Codex 独立 review（`29a3fab..44ef36e`）修复 | ✅ 完成 | `restore_step` 的 Kept 先查 stash 栈、`svn_sync_stash::stash_position`、`vcs/svn.rs::assert_not_sync_backup`、`from_baseline_ready` 先核对基线再 S8、`useConfirm` 关框只关自己、`svnSyncFlow` 同一时间只走一个流程 + 3 条 Rust / 3 条 vitest | 见下「PR #49 Codex 独立 review」；fmt / clippy 0 告警 / `cargo test` 729 passed 0 failed；`pnpm build` 通过 / `pnpm test` 446 passed |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ 0（2026-09-26，Task 11 Step 3） |
| `cargo clippy --all-targets` | ✅ 0 告警 |
| `cargo test` | ✅ 729 passed / 0 failed（PR #49 Codex review 修复之后；第四轮后 726；第三轮后 721；第二轮后 711；第一轮修复后 699，修复前 695。基线：master `1db8210` 上 598 passed） |
| `pnpm build` | ✅ tsc + vite 通过 |
| `pnpm test` | ✅ 29 个文件 446 passed / 0 failed（PR #49 Codex review 修复之后；E2E 发现修复后 443；第四轮后 27 个文件 435；第三轮后 433；第一轮后 431，修复前 430。基线 383 passed） |
| `pnpm tauri dev` | ✅ 2026-09-30 用户在真实窗口跑完 E2E 清单 8 个场景，四个新命令的 invoke 都走通（见「真实窗口 E2E 发现」）。之后的 Codex review 修复（`a990c28` 之后）没有改 Tauri 命令签名，未重跑真实窗口 |
| E2E 清单机械检查 | ✅ 29 个 `powershell` 代码块在 PS 5.1.26100 与 PS 7.6.1 下 `Parser::ParseInput` 均 0 语法错误；`Colleague` 在新造的 fixture 上连提两次（新增文件 r2、修改已有文件 r3）均落地，fixture 已删 |

---

## 开工前实测（写计划时在临时 fixture 上跑过，执行者不必重跑）

计划「对 spec 的补足与偏离」表的依据。git 2.51.2.windows.1，svn 1.14.2-SlikSvn。探针脚本在会话 scratchpad（`probe-*.ps1`），不进仓库。

| # | 内容 | 实测依据 |
|---|---|---|
| A1 | pinned 模式下 `sync_in` 动手前拒绝（用户决定） | —— |
| A2 | 新增 phase `applying-stash` 与字段 `stashApplied` | T2：对已 apply 过的 stash 再 apply，git 拒绝且不动文件 |
| A3 | S10 apply 前查未跟踪文件撞名 | R1：撞名时 git 先写已跟踪部分再失败（退出 1，无 `UU`），半套 apply |
| A4 | S5 成败不看退出码 | Q2 / Q3：文本冲突、树冲突时 `svn update --accept postpone` 退出 0；树冲突节点 `item="added" tree-conflicted="true"` |
| A5 | S5 之后放弃：断言 W-file svn 干净，再 `checkout -f` 回原分支（用户决定） | R2：普通 checkout 会把服务器内容带到原分支变成本地修改 |
| A6 | 任何 svn 冲突态都在前置检查拒绝 | W-file 的冲突标记在 stash 后仍留在 `.svn` 里，S4 必败 |
| A7 | `stashed` 且无 `stashOid` ⇔ 没有可压的改动 | P1：无可压内容时 `stash push` 退出 0、`refs/stash` 不变 |
| A8 | S10 不带 `--index`，暂存状态丢失、内容不丢 | R3 |
| A9 | S7 不跑体检，候选索引从 HEAD 树起步 | spec §6.8.5、§6.3.1 |
| A10 | `start` 阶段继续 = 放弃 = 删 journal | 定义 |
| A11 | 从 `stashed` 继续前复核 HEAD / 分支 / 基线 | 崩溃后没有锁 |
| A12 | S3 / S4 失败回退用 `checkout -f`，不做 A5 断言（仅本进程刚走到这里时） | S5 未跑，磁盘被跟踪内容全来自 git |
| A13 | `flow = sync_out` 的 journal 继续 / 放弃一律拒绝 | Phase 3 才有 |
| A14 | 冲突出口放锁，继续 / 放弃重新拿锁并复核 | spec §7.4 |
| A15 | 每次 checkout 后核对真的切干净了 | L1：share=Read 占用时 checkout 退出 0 而文件没写进去；L1′：share=None 时整体拒绝 |
| A16 | 外部整体 update（`nodeRevs` 单值且大于基线）由「更新」接管（用户决定） | `probe-ext-update.ps1`：混在一起 → 重建基线 + S1–S10 后重新分开；相邻行变成 stash 冲突 |
| A17 | 撞名预检放宽：磁盘 / stash / HEAD 三处同一个 blob 不算撞名 | `probe-ext-added.ps1`：服务器新增文件被 `-u` 收进 stash，不处理时 `already exists, no checkout` 半套放回 |

---

## Task 0：开工自检

`AGENTS.md` §4.2（2026-09-26 实测）：

```text
- 当前分支 / worktree：worktree-svn-phase2c-sync-in / .claude/worktrees/svn-phase2c-sync-in
- 是否已合并进 master：否（`merge-base --is-ancestor HEAD origin/master` exit=1）
- 是否基于最新 master：是（`merge-base --is-ancestor origin/master HEAD` exit=0）
- 是否允许在当前位置修改：是（独立 worktree，非 master）
- 目标 worktree 是否已切入：是，Get-Location = C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2c-sync-in
- 是否存在未提交改动：否
- git log --oneline -3：e334a8a（计划）/ 1db8210（#46）/ 4ae3e34
```

### 偏离计划

- 计划 Step 1 期望「`git log` 第一行是 `1db8210`」。实际是 `e334a8a`：用户要求先单独提交并推送计划文档，所以基点之上多了这一个纯文档提交。不影响任何代码 Task。

### 计划的错误

- 无。

### 计划没写的坑

- PowerShell 下 `git commit -F - @'...'@` 不会把 here-string 送进 stdin，而是当成 pathspec，报 `did not match any file(s)`；提交信息要先写进文件再 `-F <文件>`。
- 本 worktree 的 Bash 工具会拒绝「形式复杂」的 git 命令（worktree 隔离检查），git 操作一律走 PowerShell 的简单命令。

### 为什么这么改是安全的

- 本 Task 只新增文档，不碰代码与仓库状态。

---

## Task 1：2b 遗留——watcher RAII、锁释放重试、journal tmp 清理、持锁时长

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | `svn_journal` 新用例断言失败「残留 tmp 必须被清掉」；`svn_watch_suspend` / `svn_lock` 编译失败（`WatchSuspension`、`describe_age` 不存在），与计划期望一致 |
| GREEN | `cargo test --test svn_watch_suspend --test svn_lock --test svn_journal`：6 / 10 / 11 passed |
| 变异 1：`WatchSuspension::drop` 不调 `resume_watch` | 只红两条 guard 测试 |
| 变异 2：锁 `Drop` 重试 `0..1` | 只红 `release_retries_when_a_reader_briefly_holds_the_file` |
| 变异 3：不调 `sweep_stale_tmp` | 只红 `a_stale_tmp_left_by_a_crash_is_swept_on_the_next_write` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 608 passed / 0 failed（46 个测试二进制） |

### 偏离计划

- 无代码偏离。计划给的两段测试代码不满足 rustfmt（长行），`cargo fmt` 自动折行，语义不变。

### 计划的错误

- 计划写的 `cargo test` 基线是 598 passed；本 Task 加 5 条后实测 608。差出的 5 条没有逐一核对来源（基线数字抄自 2b 决策日志，统计口径可能不同，例如是否计入 doc-test）。**不影响结论**：0 failed。以后以本表的 608 为新基线。

### 计划没写的坑

- `cargo fmt` 会把 CRLF 的测试文件整体写回成 LF。`git diff --stat` 只显示真实增删（autocrlf 在提交时归一），所以没有造成整文件改动；但用脚本处理这些文件前要重新数行尾。
- `release_retries_when_a_reader_briefly_holds_the_file` 在实现前处于编译失败状态，没法先看到它的「断言失败」。用变异 2 补上：重试降到 1 次时它确实红，证明测的是重试本身。

### 为什么这么改是安全的

- `WatchSuspension` 只把既有的 `suspend_watch` / `resume_watch` 包进 RAII，挂起集合的语义不变；Drop 在 panic 展开时也会跑（测试钉住）。
- 锁 `Drop` 的重试最多等 10 × 20ms = 200ms，且只在删除失败、文件仍在时才等；`NotFound` 立刻返回。Drop 里依旧不 panic。
- `sweep_stale_tmp` 只删「`mygitclient-sync.json.` 开头且 `.tmp` 结尾」的文件，同目录的 `.bak` 等不相干文件由测试证明不被误删；只有持锁者写 journal，写入时的其它 tmp 必然是残骸。
- journal 写入失败时现在会删掉自己的 tmp（原来只有 rename 失败那条路径会删），目标文件本身从未被触碰，原子性不变。

---

## Task 2：前置检查（spec §7.2 第 1–6 条）

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| 接口核对 | 计划 Consumes 的 `check_sync`、`detect_pinned`、`verify_include_set`、`git_dir`、`TempIndex::{empty, add_all}`、`assert_not_syncing`、`svn_journal::read` 签名均与实际一致；`is_w_file` / `normalize_roots` 是 `pub(crate)`，新模块在同一 crate 内，可用 |
| RED | `svn_xml` 缺 `parse_update_status`、`svn_sync_check` 模块不存在，两个测试文件编译失败，与计划期望一致 |
| GREEN | `svn_xml update_status` 2 passed；`svn_sync_check` 15 passed（含计划标了风险的 `a_pinned_whitelist_external_is_refused`：`init_hybrid` 在该 fixture 上成功，无需换 fixture） |
| 变异 1：`tree_conflicted` 恒 `false` | 只红 `update_status_sees_tree_conflicts_whose_item_is_not_conflicted` |
| 变异 2：`rev-list` 非零退出当成通过 | 只红 `a_failing_merge_query_is_not_treated_as_empty` |
| 变异 3：去掉 `&& incoming` | 只红 `outside_changes_the_update_will_not_touch_are_not_listed` |
| 变异 4：不跑纳入集合预检 | 只红 `a_gitignore_that_hides_w_files_is_refused_before_anything` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 625 passed / 0 failed（47 个测试二进制，= Task 1 的 608 + 本 Task 17） |

### 偏离计划

- 无代码偏离。计划给的代码与 rustfmt 结果有几处折行不同，`cargo fmt` 自动处理，语义不变。

### 计划的错误

- 无。

### 计划没写的坑

- `an_out_of_sync_baseline_is_refused` 断言的「整体前进即拒绝」只是本 Task 的中间状态：A16 之后这种状态由执行时接管，Task 7A 会删掉这条测试、换成四条更细的（测试里已写明）。在 7A 之前，`sync_in_out_of_sync` 的文案仍让用户「先处理失同步横幅」。

### 为什么这么改是安全的

- 整个模块是**纯读**：git 只跑 `symbolic-ref` / `rev-parse` / `rev-list` / `diff --name-only`，svn 只跑 `status -u --xml`；纳入集合预检用 `TempIndex`（临时索引文件，Drop 时删），不碰真实索引、HEAD 与磁盘。
- `assert_linear` 在拼 `target..tip` 之前校验两端都是 40 位十六进制 OID：空 OID 会让区间退化成 `..tip`（= `HEAD..tip`），查的不是我们以为的那段；`rev-list` 非零退出一律报错，不当成「没有 merge」（变异 2 钉住）。
- svn 冲突判定同时看 `item == "conflicted"` 与 `tree-conflicted="true"`：树冲突节点的 `item` 是 `added`（变异 1 钉住）。

---

## Task 3：WIP 压栈与取回（S1 / S10）

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | `svn_journal` 缺 `ApplyingStash` / `stash_applied`，`svn_sync_stash` 模块不存在，两个测试文件编译失败，与计划期望一致 |
| GREEN（首轮） | 12 / 13：`a_failed_push_that_changed_nothing_leaves_no_journal` 红在 `r.is_err()`——见「计划的错误」 |
| GREEN | `svn_journal` 12 passed；`svn_sync_stash` 13 passed |
| 变异 1：去掉撞名预检整段 | 红 `restore_refuses_when_an_untracked_file_would_collide`，另红三条 A17 用例（同名同 blob 的放行逻辑也在这段里，属预期） |
| 变异 2：`find_by_opid` 取 reflog 第一行 | 红 `settle_finds_our_stash_even_when_another_was_pushed_on_top`，另红 `settle_stops_when_the_stash_vanished`（第一行就是「不是我们的」那条，被当成我们的，属预期） |
| 变异 3：`settle_stashing` 的 `None` 分支不比对 `refs/stash` | 只红 `settle_stops_when_the_stash_vanished` |
| 变异 4：`drop_wip` 固定 drop `stash@{0}` | 计划原测试**没抓住**（见「偏离计划」）；加强后只红 `restore_applies_and_drop_removes_only_ours` |
| 变异 5：`is_same_file_everywhere` 恒 `false` | 红 `restore_passes_over_…` 与 `a_real_collision_blocks_…`（后者撞名列表多出 `S.cs`，属预期） |
| 变异 6：不比磁盘 blob | 只红 `a_same_name_file_edited_on_disk_is_a_real_collision` |
| 变异 7：真撞名时先删字节相同的文件 | 红 `a_real_collision_blocks_…`（目标）；`restore_passes_over_…` 也红，是变异写法把删除做了两遍的副作用 |
| 变异 8（补充）：push 失败时不删 journal | 只红 `a_failed_push_that_changed_nothing_leaves_no_journal`——证明换了失败手段之后这条测试真的走到了删除分支 |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 639 passed / 0 failed（48 个测试二进制，= Task 2 的 625 + 本 Task 14） |

### 偏离计划

- `restore_applies_and_drop_removes_only_ours`：在我们的 stash 之后**再压一条别人的**，并断言它还在。计划原样的测试里「别人的」那条压在我们之前，我们那条恰好在栈顶，`drop stash@{0}` 与按 OID 定位结果一样，变异 4 抓不到。
- `a_failed_push_that_changed_nothing_leaves_no_journal`：制造 push 失败的手段从「白名单指向不存在的路径」换成「预先放一个 `.git/index.lock`」，白名单用正常的 `Code/`。

### 计划的错误

- 计划假设 `git stash push -u -m … -- Nope/`（pathspec 不匹配任何文件）会报错。**2026-09-26 实测（git 2.51.2.windows.1）**：带 `-u` 时输出 `No local changes to save`、**退出 0**；不带 `-u` 才报 `pathspec … did not match` 退出 1。所以那条测试在实现正确时也必然红。

### 计划没写的坑

- `.git/index.lock` 存在时 `git stash push -u` 报 `Unable to create … index.lock` / `could not write index`，**退出 1，工作区、未跟踪文件、stash 栈都不动**（同日实测）。这是「push 失败且没落地」的真实形态，也是用户侧最可能遇到的一种（另一个 git 进程崩溃后留下的锁）。
- 同一现象的推论：白名单里某个根目录在磁盘上不存在时，`stash_wip` 不报错，按 A7 走到 `stashed` 且无 `stashOid`。这与「那里本来就没有可压的改动」语义一致，不需要特殊处理。
- `svn_journal.rs`、`engine/mod.rs` 是 CRLF；本 Task 用脚本改它们时先判断行尾再替换。

### 为什么这么改是安全的

- `stash_wip` 空白名单直接拒绝（`sync_stash_empty_whitelist`），不会退化成整仓 stash（§12.2，测试钉住）。
- push 失败后只有在「按 opid 找不到、且 `refs/stash` 与写 journal 前一致」时才删 journal；任何一条不满足都保留 journal，交给 `settle_stashing` 的判定树。
- `restore_wip` 在 apply 前完成撞名判定；真撞名时返回 `Blocked`，此时**没有删任何文件、没有 apply**（变异 7 钉住）。A17 放行只删「磁盘 == stash == HEAD 同一 blob」的文件，删掉的字节在 HEAD 里原样存在；apply 中途失败时从 HEAD 取回。
- `drop_wip` 按 OID 在 reflog 里定位；drop 输出里的 OID 与预期不符时把误删的那条 `stash store` 放回并报错（`sync_stash_drop_raced`），不吞别人的 WIP。

---

## Task 4：基线录制零件拆分

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | `create_baseline_commit` / `move_baseline_ref` / `record_server_tree_into` 不存在，两个测试文件编译失败，与计划期望一致 |
| GREEN | `svn_baseline` 22 passed；`svn_init` 57 passed（既有用例一条不红，拆分不改行为） |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 首轮 1 条 `needless_borrow`（`idx` 变成引用后 `&idx` 多余），改掉后 0 告警；`cargo test` 644 passed / 0 failed（48 个测试二进制，= Task 3 的 639 + 本 Task 5）。clippy 修正在全量之后，只删了一个 `&`，修正后重跑了 `svn_init` 57 passed |

### 偏离计划

- 无行为偏离。`create_baseline_commit` 的文档里补了一句 `parent` 的含义（原来在 `publish_baseline` 文档里，拆开后归到用它的函数）；`move_baseline_ref` 同理带上了 `expected_old` 的含义。

### 计划的错误

- 计划 Step 5 期望「既有 598 条一条不红」——598 是 2b 的基线，本分支此时已是 639。以实测 644（= 639 + 5）为准。

### 计划没写的坑

- `record_server_tree_into` 的 `idx` 改成引用后，函数体内 `verify_include_set(repo_root, &idx, …)` 会触发 clippy `needless_borrow`，计划的「其余 `idx.xxx(...)` 调用照旧」漏了这一处。
- `svn_init.rs` 里「`let idx = TempIndex::empty(repo_root)?;` + `idx.add_all(...)`」这两行出现两次（另一处在体检相关函数里）；只删 `record_server_tree` 函数体里那一处，按上下文定位。
- Git Bash 的 `sed -i` 会把 CRLF 文件写回 LF（本仓库 index 是 LF、提交时归一，`git diff --stat` 不受影响）。

### 事后修正（Task 7A 之后发现，单独提交）

- 本 Task 的拆分是用 Bash 工具里的 Python heredoc 写进去的，而 Bash 工具会把 heredoc 里的 `\\` 折成一个 `\`。结果 `d9aac2d` 里有两处字符串被改坏：
  1. `create_baseline_commit` 的 `format!("svn: r{}\n\n{}\n", …)` 变成了跨行的字面换行。rustc 把源码换行归一为 LF，生成的 message 与原来逐字相同（trailer 解析的既有用例一直是绿的），但写法偏离了拆分前的原文。
  2. `move_baseline_ref` 里 `svn_baseline_moved` 报错的续行 `\` 丢了，**用户可见的中文句子中间夹进 17 个空格**。这条只在「预检通过、`update-ref` 自己失败」的窄竞态路径上出现，原有测试走不到。
- 修法：先补 `split_publish::a_failed_update_ref_reads_as_one_sentence`（用残留的 `refs/heads/svn/base.lock` 让 `update-ref` 失败，读 ref 不受影响），RED 已见（报错里带着那串空格）；再用 Edit 工具把两处字符串逐字恢复成拆分前的原文。`git diff 9e2703d -- svn_baseline.rs` 复核：除计划要的结构拆分外没有别的差异。全仓 Rust 文件扫描「中文后跟 8 个以上空格」无其它命中。修正后 `cargo test` 693 passed / 0 failed。

### 为什么这么改是安全的

- `publish_baseline` 仍然先比对旧值、比对不过**不建 commit**，然后才调 `create_baseline_commit` + `move_baseline_ref`；`move_baseline_ref` 自己再比对一次、`update-ref` 带旧值兜住窄竞态——比拆分前多一次读取，语义不变（`publish_is_still_create_then_move` 与既有 `svn_baseline` / `svn_init` 用例钉住）。
- `record_server_tree` 仍从空索引起步（包装里建 `TempIndex::empty`），初始化与 R2 的录制结果逐字不变；`recording_from_head_agrees_with_recording_from_empty_after_an_update` 证明「磁盘 = 服务器内容」时两种起点录出同一棵树，服务器删掉的文件不会被 HEAD 树带回来。

---

## Task 5：编排主路径 S0–S11 + 冲突出口

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| 接口核对 | Consumes 里 Task 2–4 与既有函数（`swap_index_to_tree`、`run_preflight` / `Preflight.{node_rev, repos_root, uuid}`、`read_node_rev`、`check_targets`、`enumerate_wcs`、`read_dirty` → `StatusEntry.{path, item, props}`、`run_svn_code` → `.code / .stderr`、`file_lock::{find_blockers, describe}`、`svn_lock::{acquire, is_held}`）签名均与实际一致 |
| RED | `svn_sync_in` 模块不存在，编译失败，与计划期望一致 |
| GREEN | `svn_sync_in` 11 passed，首轮全过（含必测 #49 external 形态） |
| 变异 1：Conflicted 分支不写 `stash_applied = true` | 只红 `a_stash_conflict_keeps_the_stash_and_marks_it_applied` |
| 变异 2：rebase 去掉 `--empty=drop` | **不红**——见「计划的错误」 |
| 变异 3：`capture_and_publish` 改用 `TempIndex::empty` | 不红，计划已预期「覆盖不到」，由 Task 4 的对照测试守着 |
| 变异 4：`align_index` 直接返回 | 红 `a_plain_update_advances_the_baseline_and_replays_the_branch`（目标），另红 6 条走到 S8 的用例——不对齐索引时 S8 切分支失败，所有走到 S8 的路径都会红，属预期 |
| 变异 5：`verify_expect` 不比 outside | 只红 `new_outside_changes_since_the_preview_require_reconfirmation` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警（计划担心的 `dead_code` 没出现）；`cargo test` 655 passed / 0 failed（49 个测试二进制，= Task 4 的 644 + 本 Task 11） |

### 偏离计划

- 无代码偏离。

### 计划的错误

- 计划期望变异 2（去掉 `--empty=drop`）只红 `a_linear_round_trip_is_not_reported_as_lost`。**2026-09-26 实测（git 2.51.2.windows.1，scratchpad `p2`）**：非交互 `git rebase` 的默认行为本来就是丢弃「变空的提交」；那条用例里的 `to one` 还与新基线是同一个 patch，git 直接以 `patch contents already upstream` 跳过。即使构造「变空但 patch-id 不同」（上游一次改 A+B、本地只改 A），不带参数也照样 drop。所以这条变异在本机**不可观察**，记为「覆盖不到」。保留 `--empty=drop` 是为了不依赖默认值（`-i` 时默认是 `stop`，spec §7.3 S9 写明要显式带）。

### 计划没写的坑

- `tests/common/svn_fixtures.rs` 是纯 LF 文件，追加 fixture 时保持 LF。

### 为什么这么改是安全的

- 本 Task 的四个失败出口（S3 checkout / S4 基线不干净 / S5 update 失败 / S5 冲突）只如实报错、**journal 原样保留**，不做任何回退动作；回退在 Task 7 做。此时 WIP 在按 opid 钉住的 stash 里，不会丢。
- 锁内先复核（`check_sync_in` + `verify_expect`），不过就在**写 journal 之前**返回：`a_branch_moved_since_the_preview_is_refused`、`new_outside_changes_…` 断言 journal 不存在、没有 stash。
- S7 的 ref 移动走 `finish_publish` 的三态判断：已是新值（崩溃恢复）/ 仍是旧值（CAS 移动）/ 都不是（停下不覆盖）。
- `align_index` 用 `swap_index_to_tree` 的 CAS：索引不是旧基线树就停下，不覆盖别的 git 操作留下的索引。
- 每次 checkout 之后按 A15 核对「在目标分支上且白名单内无被跟踪改动」，不看退出码。

---

## Task 6：继续 / 放弃（含「回到原分支」）

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | `continue_sync` / `abort_sync` 不存在，编译失败，与计划期望一致 |
| GREEN | `svn_sync_in` 34 passed（Task 5 的 11 + 本 Task 23），首轮全过 |
| 变异 1：`back_to_orig` 去掉 `dirty_w_files` 断言 | 只红 `abort_after_the_update_refuses_to_overwrite_unknown_changes` |
| 变异 2：`resume_stashed` 去掉 `whitelist_clean` | 只红 `continue_from_stashed_refuses_changes_made_after_the_crash` |
| 变异 3：`continue_rebase` 去掉 `is_ancestor` 复核 | 计划用例**不红**（被前面的 `current_branch` 复核先挡住，计划已预判）；按计划补 `continue_refuses_when_the_rebase_was_aborted_outside` 后只红它 |
| 变异 4：`finish_publish` 的 `cur == new` 再 `move_baseline_ref` | 只红 `continue_from_publishing_after_the_ref_already_moved` |
| 变异 5：`abort_sync` 的 `ApplyingStash` 改成 `drop_wip` | 只红 `abort_while_applying_the_stash_keeps_the_stash_for_the_user` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 679 passed / 0 failed（49 个测试二进制，= Task 5 的 655 + 本 Task 24） |

### 偏离计划

- **`Disk::GitOnly` 推迟到 Task 7 再定义**。计划本 Task 就定义两个变体，但 `GitOnly` 只有 Task 7 的 S3 / S4 失败回退会构造，本 Task 编译出 `variant GitOnly is never constructed` 告警。照计划 Task 5 对 `dead_code` 的规矩（不加 `#[allow]`，推迟到有调用方的 Task 再写），先只留 `MaybeUpdated`，文档里写明 A12 那一种由 Task 7 引入。
- 新增用例 `continue_refuses_when_the_rebase_was_aborted_outside`（计划 Step 5 第 3 行明写的补救）：外面 `rebase --abort` 后人还在 `work` 上，分支名对得上，只有 `is_ancestor` 能拦住——否则 S10 会把 WIP 放到没重放过的旧分支上、drop 掉 stash、宣布完成。

### 计划的错误

- 无。

### 计划没写的坑

- 无新坑。`rebase --continue` 走 `run_git_env` 带 `GIT_EDITOR=true`，与计划实测一致。

### 为什么这么改是安全的

- 继续 / 放弃都**先拿锁、在锁内重读 journal**（`take_over`），`flow = sync_out` 的一律不碰（A13，测试钉住 journal 原样）。
- 从 `stashed` 继续前复核 HEAD / 分支 / 白名单干净 / 基线位置（A11）；不过就停下，journal 与 stash 原样。
- 放弃时 HEAD 在 `svn/base` 上：先断言白名单内 W-file 在 svn 看来全部干净（= 磁盘上是可从服务器取回的内容），才 `checkout -f` 回原分支（A5）；不干净返回 `sync_in_unsafe_to_return`，一个字节不覆盖（测试钉住）。
- `applying-stash` 放弃只删钉住的 ref 与 journal，stash 条目留给用户（message 带 opid 认得出来）。
- `rebase` 继续前复核进行中的 rebase 的 `head-name` 是 `refs/heads/<origBranch>`，且结束后原分支必须以新基线为祖先。

---

## Task 7：S3 / S4 / S5 失败回退

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | 四条都红：错误码对，但仍停在 `svn/base`、message 里没有「已回到分支」，与计划期望一致 |
| GREEN | `svn_sync_in` 39 passed（含 Windows 专属的 share=Read 占用用例） |
| 变异 1：`on_update_failed` 不回退 | 只红 `update_failure_returns_to_the_original_branch` |
| 变异 2：`checkout_verified` 只看 `on_target` | 只红 `a_file_held_open_during_checkout_is_named_and_rolled_back` |
| 变异 3：`svn_conflicts` 只看 `item == "conflicted"` | 计划用例不红（计划已预判为覆盖缺口）；补 `a_tree_conflict_rolls_back`（必测 #9）后只红它 |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 684 passed / 0 failed（= Task 6 的 679 + 本 Task 5） |

### 偏离计划

- `Disk::GitOnly` 在本 Task 加回（Task 6 推迟的那个变体），文档与计划原文一致。
- 新增 `a_tree_conflict_rolls_back`：计划 Step 5 允许「30 分钟内造得出就补」。形状是「wc2 `svn delete Art` 并提交 + wc1 本地改了 `Art/x.psd`」，预览把 `Art/x.psd` 列进白名单外会被碰到的文件、用户确认后，S5 产生树冲突（`item` 不是 `conflicted`），回退到 `work`、基线不动。

### 计划的错误

- Step 2 的命令 `cargo test --test svn_sync_in a_lying_baseline update_failure …` 跑不起来：cargo 的 `TESTNAME` 只接受**一个**位置参数，多个过滤要写在 `--` 之后（`cargo test --test svn_sync_in -- a b c d`）。

### 计划没写的坑

- 无。

### 为什么这么改是安全的

- 回退只走 Task 6 的 `back_to_orig`：S3 失败（HEAD 还在原分支，git 整体拒绝、一个字节没写——A15）与「本进程一路走到 S4」时按 `GitOnly` 直接 `-f`；S5 之后、以及从崩溃里恢复时一律 `MaybeUpdated`，先断言 W-file 在 svn 看来干净再 `-f`（A5 / A12）。
- 回退失败不吞原因：原因永远在 message 前面，回退结果接在后面；回退没成功时 journal 保留，横幅上还能「放弃」。
- 失败只回滚本流程自己的修改（§12.4）：`a_lying_baseline_is_caught_at_s4_and_rolled_back` 断言基线分支仍指向那个「说谎」的 commit，不去修它；S5 冲突现场（`C Art/x.psd`）原样留给 TortoiseSVN。

---

## Task 7A：外部整体 update 之后由「更新」接管（A16）

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| 接口核对 | `svn_init::rebuild_baseline(&str) -> Result<RebuildReport>`，`RebuildReport.{node_rev, old_baseline, new_baseline, health}`；`SyncVerdict::Diverged { node_rev, baseline_rev }`；`SyncInExpect` 已 derive `Clone`——均与计划一致 |
| RED | `catch_up_rev` 字段不存在，两个测试文件编译失败，与计划期望一致 |
| GREEN（首轮） | `svn_sync_check` 18 passed；`svn_sync_in` 44 / 45：`an_external_root_update_is_taken_over_and_separated_again` 红在「服务器改动不该显示成本地修改」——见「计划的错误」 |
| GREEN | `svn_sync_check` 18 passed；`svn_sync_in` 45 passed |
| 变异 1：去掉 `node_rev > baseline_rev` 守卫 | 只红 `a_working_copy_rolled_back_below_the_baseline_is_still_refused` |
| 变异 2：`run_sync_in` 不调 `catch_up` | 红主场景 `an_external_root_update_is_taken_over…`，另红其余三条接管用例（它们都依赖接管，属预期） |
| 变异 3：接管挪到锁内复核之前（有 catch-up 时跳过第一道 `verify_expect`） | 只红 `a_catch_up_is_refused_when_the_branch_moved_since_the_preview` |
| 变异 4：`catch_up` 不重新 `check_sync_in`、沿用旧预览 | 红主场景，另红其余三条接管用例（S0 记下旧的 `baselineOidBefore`，发布时 CAS 不过，属预期） |
| 变异 5：`is_same_file_everywhere` 恒 `false` | 只红 `a_file_the_server_added_does_not_block_the_catch_up` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 692 passed / 0 failed（= Task 7 的 684 − 1 条被替换 + 4 条预览 + 5 条执行） |

### 偏离计划

- 主场景用例里「相对 HEAD / 相对基线只剩自己的改动」两组断言，改成只看 diff 的**改动行**（新增测试辅助函数 `changed_lines`：去掉 `+++` / `---` 文件头与 `@@` hunk 头）。断言的语义不变，只是不再被 hunk 头误伤。

### 计划的错误

- 计划的断言 `!git diff HEAD .contains("a5-server")` 在实现正确时也会红。**2026-09-26 实测**：接管之后 `git diff HEAD -- Code/A.cs` 的改动行只有 `-a9` / `+a9-wip`，但 hunk 头是 `@@ -6,5 +6,5 @@ a5-server`——git 默认把 hunk 之前最近一个「看起来像函数名」的行（这里就是第 5 行）印在 `@@` 之后作为上下文。服务器改动已经完全从改动行里分离出去，断言只是没有区分 hunk 头与改动行。

### 计划没写的坑

- **相邻行冲突照计划实测复现**：服务器改第 5 行、我没提交的改动在第 6 行时，外部 `svn update` 静默三方合并成功（磁盘上两处都在）；接管后 S10 的 `git stash apply` 把相邻行当成冲突，结局 `StashConflict { files: ["Code/A.cs"] }`，基线已跟上到 r4、冲突文件里两边内容都在、原始 stash 保留。git 的合并对相邻行比 svn 严，本期不绕开，由用户手工解一次（`an_external_update_next_to_my_own_edit_stops_as_a_stash_conflict_without_losing_anything` 钉住）。
- Bash 工具会把 heredoc 里的 `\\` 折成一个 `\`（即使 heredoc 带引号），所以用 Python heredoc 改 Rust 源码时，含反斜杠（字符串续行、`\n`）的替换片段会匹配失败或被改坏；这类替换改用 Edit 工具。

### 为什么这么改是安全的

- **接管不写 journal**：`rebuild_baseline` 不碰 HEAD、真实索引和磁盘，只以旧 OID 为条件移动 `svn/base` 一个 ref（R4）。它要么没发生、要么完整发生，两种状态都自洽：崩在它之后，重开时基线已跟上、没有 journal，下次预览看到的是普通 `InSync`。没有需要恢复的中间态。
- **锁内复核在接管之前**：确认之后分支被动过时一个字节都不动，基线不被接管（变异 3 钉住）；接管之后再复核一次，S0 记下的 `baselineOidBefore` 是新基线，且白名单外会碰到的文件照常与用户确认过的比。
- `rebuild_baseline` 失败时 `?` 直接返回：此时没有 journal、没有 stash、基线指针没动，与预览拒绝等价，不另写回退（计划 Step 5 的要求）。
- **只接管「整体前进」**：`nodeRevs` 单值且大于基线。mixed-revision 归 §9.4（2d），退回基线之前多半是误操作——两者照旧 `sync_in_out_of_sync` 拒绝（`a_mixed_revision_…`、`a_working_copy_rolled_back_…` 钉住，变异 1 证明守卫起作用）。pinned 形态在更早的第 3 条之后就被 A1 拒绝，走不到这里。

---

## Task 8：Tauri 命令

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| 接口核对 | 既有 `repo-changed` 事件的 payload 是仓库路径字符串（`commands/watch.rs`），前端 `RepoTab.tsx` 用 `samePath(e.payload, path)` 过滤——本 Task 补发的事件形状一致；`WatchSuspension::new(&str)` 与 Task 1 一致 |
| RED | `sync_in_types_cross_ipc_in_camel_case` 直接通过（类型在 Task 5 / 7A 已有，计划已说明无妨）；`sync_commands_suspend_the_watcher_…` 因 `commands/svn_sync.rs` 不存在失败 |
| GREEN | `svn_backend` 28、`svn_write_guard` 6、`commands_are_async` 1 全过 |
| 变异 1：`svn_sync_abort` 不挂起 watcher | 只红 `sync_commands_suspend_the_watcher_and_their_engine_entries_take_the_lock` |
| 变异 2：`svn_sync_continue` 去掉 `(async)` | 只红 `every_command_is_declared_async` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 695 passed / 0 failed（= 693 + 本 Task 2） |

### 偏离计划

- 无。

### 计划的错误

- Step 2 的 `cargo test --test svn_backend sync_in_types --test svn_write_guard sync_commands` 同 Task 7：多个位置过滤参数 cargo 不接受，实际用 `-- sync_in_types sync_commands`。

### 计划没写的坑

- 无。

### 为什么这么改是安全的

- 三个写命令都经 `with_watch_suspended`：`WatchSuspension` 是 RAII，`f()` 返回或 panic 展开时都会恢复 watcher；结束后无论成败补发一次 `repo-changed`，回退动过的分支 / 工作区也能被界面看到。
- 预览命令不挂起 watcher、不拿锁，只调纯读的 `preview_sync_in`。
- 命令层不含任何业务判断，拿锁、复核、回退全部在 engine 里，已由 `svn_sync_in.rs` 的用例覆盖；接线由 `svn_write_guard.rs` 的源码断言守着。

---

## Task 9：前端纯逻辑

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | `svnSyncIn.test.ts` 模块不存在；`svnJournal` 4 条、`svnSync` 1 条、`toolbarActions` 1 条失败，与计划期望一致。`svnSync` 的「被回退、多 revision 时不给『更新』」实现前就通过——它是护栏，旧实现本来就不给 |
| GREEN | 四个测试文件 92 passed |
| 变异 1：`expectFromPreview` 不拷贝 | 只红 `把预览里的事实原样带回去做锁内复核` |
| 变异 2：`start` 给继续 + 放弃 | 只红 `start 只有「清除记录」` |
| 变异 3：`sync_out` 也给 `syncActions` | 计划用例**不红**（见「偏离计划」）；加强后只红 `sync_out 留下的记录本版本收不了尾` |
| 变异 4：回退分支也带 `svnUpdate` | 只红 `被回退、多 revision 时不给「更新」` |
| 变异 5：`describeSyncInPreview` 忽略 `catchUpRev` | 红 `工作副本被外部整体更新过…` 与 `接管之后服务器没有更新的内容…`（两条都依赖 `from`，属预期） |
| 全量 | `pnpm build`（tsc + vite）通过；`pnpm test` 27 个文件 430 passed / 0 failed |

### 偏离计划

- `sync_out 留下的记录本版本收不了尾` 加了一个 phase：除 `svn-staged` 外再测 `publishing-baseline`。计划原样只用 `svn-staged`，而 `syncActions('svn-staged')` 本来就返回 `[]`，「`sync_out` 也给按钮」的变异抓不到。`publishing-baseline` 是 spec §7.6 明写的两条流程共用阶段名，正是只看 phase 会出错的那种输入。

### 计划的错误

- 计划写 `svnJournal.ts` 是 LF；实际本分支上它与本 Task 碰到的其余前端文件都是 CRLF。用 Edit 工具改，行尾不受影响。

### 计划没写的坑

- 无。

### 为什么这么改是安全的

- 全部是纯函数，无 React / Tauri 依赖；`svnJournal.ts` → `svnSyncIn.ts` → `svnSync.ts`（仅 type import），没有运行时循环依赖。
- 「整体前进」横幅降为黄色，但重建基线与体检两个出口仍在（`所有 danger 文案都写明重建基线…` 与新用例一起钉住），离线时用户仍可先把基线跟上。回退与 mixed-revision 照旧红色、不给「更新」。
- 新的横幅 action id（`svnUpdate`、`syncContinue`、`syncAbort`）在 Task 10 接线之前会被 `BaselineBanner` 渲染成按钮但没有处理——Task 10 紧接着接上，本 Task 不单独发布。

---

## Task 10：前端接线 + 真实窗口冒烟

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| 接口核对 | `ConfirmRequest` 已有 `title / message / warning / confirmLabel / danger / action / busyLabel`，`SyncConfirmModel` 展开进去 `tsc` 不报多余字段；`refreshSvnSyncState / refreshSvnSync / fetchSvnLog` 都收 `path: string` |
| 构建 | `pnpm build`（tsc + vite）通过 |
| 测试 | `pnpm test` 27 个文件 430 passed / 0 failed（本 Task 没有新增纯逻辑，组件在本仓库跑不了 vitest——没有 jsdom） |
| 真实窗口 | **未跑。** 场景 1（普通更新）与场景 2（冲突 → 继续）要在 `pnpm tauri dev` 的窗口里点按钮、在另一个 PowerShell 里造现场，需要人工。清单见 `docs/superpowers/e2e/2026-09-25-svn-phase2c-e2e.md` |

### 偏离计划

- **`.dialog-message` 加 `white-space: pre-wrap`**（`src/styles.css`）。计划没写：`describeSyncOutcome` 与接管说明都用换行符分行列文件清单，而 `ConfirmDialog` 的 message 原来按 HTML 默认折叠空白，清单会挤成一行。其它调用方的 message 都是单行，加上后渲染不变（`.dlg-warning` 本来就是 `pre-wrap`）。

### 计划的错误

- 无。

### 计划没写的坑

- 同上：对话框 message 不保留换行。

### 为什么这么改是安全的

- 组件只做接线，分支逻辑全部在 Task 9 的纯函数里；invoke 参数名 `path` / `expect` 与 Rust 签名一致，`expect` 内部字段由 `tests/svn_backend.rs` 的序列化断言钉住——但这层转换没有编译期检查，最终仍要靠真实窗口里点一次确认（未跑，见上）。
- 执行与继续 / 放弃都走 `confirm` 的 `action`：出错时对话框留在原地显示后端错误（错误文案里已写明回退结果），不会静默关掉。

---

## Task 11：独立 review 与修复

### review

- 由全新上下文的 opus agent 按 `AGENTS.md` §13 rubric 做整分支 review（`1db8210..56d27ee`），先读了本文件全部「偏离计划 / 计划的错误」与执行 ledger 的 9 条 ruling。结论 **With fixes**，**未发现未记录的偏离**。
- reviewer 实跑：`svn_sync_in` / `svn_sync_stash` / `svn_sync_check` / `svn_backend` / `svn_write_guard` 共 110 passed；`pnpm test` 430 passed；在 `%TEMP%` 自建临时仓库做了两次 git 探针，用完删除。

### 分级与处置

| # | 发现 | 定级 | 处置 |
|---|---|---|---|
| C1 | S8 或「回原分支」那次 checkout 被 share=Read 占用、退出 0 却留下基线字节。之后点「继续」：rebase 在 `rebase.autoStash=true`（本机全局配置就是这样，reviewer 实测）时把残留收起、重放、再放回，报 Done，磁盘上是撤掉了你那笔提交的内容；点「放弃」直接放回 WIP，残留原样留着 | Critical | 已修 |
| I1 | `applying-stash` 阶段放弃返回 `Aborted`，界面说「已回到更新前的分支，未提交的改动已放回」——两句都是假的，用户可能把 stash 里的备份当垃圾清掉 | Important | 已修 |
| I2 | watcher 挂起用集合不计数：同仓库两个标签页，B 的命令因拿不到锁很快返回，它的恢复把 A 的挂起一起撤掉 | Important | 已修 |
| M1–M6 | `drop_wip` 竞态分支吞掉 `stash store` 失败且依赖未固定 locale 的输出；A17 删除非原子；空白名单时 A5 断言空转；`catch_up` 之后的报错说「没有做任何修改」；S8 失败文案没说下一步；Review Focus 3 的测试不是「S5 途中」断网 | Minor | 记为 deferred，不在本轮修 |

### 修复与验证

| 发现 | 修法 | 测试（先红后绿） |
|---|---|---|
| C1 | ① rebase 显式 `-c rebase.autoStash=false -c rebase.updateRefs=false`；② 新增 `settle_checkout_residue`：HEAD 已在原分支上时（`from_baseline_ready` 写 Rebase 之前、`back_to_orig` 放回 WIP 之前），白名单内与 HEAD 不一致的被跟踪文件逐个比对，全是新旧基线里的 blob（或文件已不在）才按 HEAD 写回，否则 `sync_in_unsafe_to_return` 指名停下、记录保留；写回后仍不干净按占用报 `sync_in_checkout_failed` | `continue_after_s8_left_baseline_bytes_does_not_revert_my_commit`（测试里显式开 `rebase.autoStash`）、`abort_after_s8_left_baseline_bytes_does_not_revert_my_commit`、`bytes_that_are_neither_the_branch_nor_the_baseline_are_not_overwritten`：三条 RED 与 reviewer 描述一致（继续报 Done 而 `B.cs` 是基线字节；放弃后残留原样；来路不明的字节也被当成功） |
| I1 | 后端新结局 `SyncInOutcome::Released { stashOid }`；前端 `describeSyncOutcome` 的 `released` 文案说清分支已在新基线上、改动没有（完整）放回、备份在 stash 里（带短 OID） | Rust：`abort_while_applying_the_stash_keeps_the_stash_for_the_user` 改为断言 `Released` 且 `B.cs` 确实没放回；`sync_in_types_cross_ipc_in_camel_case` 加 `{"kind":"released","stashOid":"o"}`（两者先编译失败）。vitest：新增 `在放回改动那一步放弃…` 并把 `released` 加进纯文本检查（先红） |
| I2 | `SUSPENDED` 改为按归一化路径计数，计数归零才算恢复；没挂起过的恢复仍是空操作 | `a_second_guard_on_the_same_repo_does_not_resume_the_first`（先红：B 结束后 A 已不在挂起） |

变异复核：去掉 `from_baseline_ready` 的残留处理 → 继续用例红；去掉 `back_to_orig` 的残留处理 → 放弃用例红；把任何字节都当基线 → 「来路不明」用例红；去掉下面那条 S1 前提守卫 → `abort_from_stashing_that_never_landed_changes_nothing` 红。**`rebase.autoStash` 钉死这一条不可观察**：残留处理已保证 rebase 前白名单内干净，钉配置是纵深防御（autoStash 只收已跟踪改动），记为 ruling。

### 偏离计划

- 新增 `SyncInOutcome::Released`（计划没有这个结局）与 `sync_in_unsafe_to_return` 的第二个触发点（原来只在 A5 断言）。
- 计划文案 `aborted →「已放回」` 被用于 `applying-stash` 放弃，是计划本身的错误（I1）。

### 计划没写的坑

- **首轮修复带出一条回归**：`abort_from_stashing_that_never_landed_changes_nothing` 红了。push 从未落地时 WIP 还在磁盘上，残留检查把它当成「来路不明」。「WIP 全在 stash 里」只在 S1 确实完成过时成立，所以 `stashOid` 缺省且 phase 仍是 `start` / `stashing` / `stashed` 时跳过残留处理——那时 HEAD 从没离开过原分支，没有 checkout 残留可言。
- 本机全局 git 配置开着 `rebase.autoStash=true`，fixture 仓库会继承它——测试结果会随开发机配置变化，C1 的继续用例因此显式设置它。

### 为什么这么改是安全的

- 残留写回只覆盖「字节在新旧基线 commit 里原样存着」或「磁盘上已经没有」的文件，被覆盖的字节随时能从基线取回；任何一个文件不满足就整体不动。写回的 pathspec 来自 `git diff --name-only`，非空才走到这里（§12.2）。
- `Released` 只改变结局的报法，后端动作（删钉住的 ref 与 journal、stash 条目留给用户）不变。
- watcher 计数只影响「何时恢复」，挂起与去抖线程的判断方式不变；恢复没挂起过的路径仍是空操作。

### 仍未做

- 真实窗口 E2E（`pnpm tauri dev`）没有跑，需要人工。

## PR #49 review（第二轮）

### review 与核实

外部对 PR #49 的 review 提了 10 条（#1 实测，其余读代码）。逐条核实：#1 在临时 fixture 上复现；#6 / #7 依赖的 git 行为在临时 fixture 上实测；其余读代码确认。核实后的分级与处置：

| # | 发现 | 核实 | 定级 | 处置 |
|---|---|---|---|---|
| 1 | 分支与 tag 同名时 `symbolic-ref --short` 给 `heads/<b>`，`git checkout heads/<b>` 退出 0 却 detached；S8 报错后继续 / 放弃都认不出分支 | 成立（实测） | High | 已修 |
| 2 | 没有 WIP 时 phase 一路是 `stashed`，C1 的残留守卫把「回退留下的残留」整个跳过，放弃报「已放回」 | 成立 | High | 已修 |
| 3 | `applying-stash` 两个方向共用；回退途中被撞名挡住后，继续报 `Done`、放弃说「分支已在新基线上」、横幅说「基线已经更新」 | 成立，且比 review 说的更容易触发（S5 冲突 + 服务器新增同名文件） | High | 已修 |
| 4 | 放弃在 `rebase` 阶段不查 head-name 就 `rebase --abort` | 成立；但 git 不允许同时开两个 rebase，须先在外面结束本次的，降为 Medium | Medium | 已修 |
| 5 | 用户在外面走完 rebase 后放弃总是失败 | 基本不成立：此时「继续」能正常收尾，放弃要回去只能 `reset --hard`，拒绝是对的；只是文案没指向「继续」 | Low | deferred |
| 6 | rebase 启动失败后继续永远不重试 | 成立（实测：退出 1、不留 rebase 目录、HEAD 不动；清掉挡路的东西后同一条 rebase 成功） | Medium | 已修 |
| 7 | checkout 核对只看被跟踪文件，该删却没删掉的文件成了未跟踪残留 | 成立（实测：同名未跟踪文件**内容相同**也会挡住 checkout），与 #6 连成死路 | Medium | 已修 |
| 8 | 失败时前端不刷新 SVN 侧 | 部分成立：后端照发 `repo-changed`，图与同步状态会刷；只有 A16 接管后又回退时整体前进横幅与 SVN 日志是旧的 | Low | deferred |
| 9 | 接管路径重复走网络 | 部分成立：锁内 2 次 `svn status -u`（预览那次不在锁内） | Low | deferred |
| 10 | `now_secs` 重复；分支名查询三处重复 | 部分成立：`--short` 只有两处，另一处是 master 上的 `svn_init::check_can_align`，同名 tag 时 `update-ref` 带旧值守卫安全失败，不在本 PR 范围 | Low | deferred |

### 修复与验证

| # | 修法 | 测试（先红后绿） |
|---|---|---|
| 1 | `current_branch` 改用 `symbolic-ref --quiet HEAD` 并剥 `refs/heads/`（被多处调用，一处改全） | `a_tag_named_like_the_branch_does_not_detach_head`、`continue_after_a_rebase_conflict_with_a_tag_named_like_the_branch`（RED：`sync_in_checkout_failed`，「切到分支 `heads/work`」） |
| 2 | journal 新字段 `leftOrig`：S3 在 checkout **之前**置真并落盘；残留守卫改为「无 stashOid 且未离开过原分支且 phase 在 S3 之前」才跳过 | `abort_without_wip_does_not_skip_what_a_rollback_left_behind`（守卫先随 #7 一起落地，故用变异确认：去掉 `!j.left_orig` 条件 → 红，`B.cs` 为 `b1`）、`the_record_says_head_may_have_left_the_branch_once_s3_starts`（RED：字段为假） |
| 3 | journal 新字段 `rollingBack`：`back_to_orig` 在放回之前置真；`applying-stash` 继续时按它走 `back_outcome`（结局 `Aborted`）；`Released` 带 `rolledBack`；`UnfinishedSync` 带 `rollingBack`；前端 `released` 与横幅按方向换文案 | `continue_after_a_blocked_rollback_is_not_reported_as_an_update`（RED：`Done { new_rev: 1 }`）、`abort_after_a_blocked_rollback_says_the_branch_went_back`（先编译失败）、`sync_in_types_cross_ipc_in_camel_case` / `sync_state_serializes_to_camel_case` 钉住新字段名；vitest 两条（先红） |
| 4 | 抽出 `require_our_rebase`（A14），继续与放弃共用 | `abort_does_not_abort_a_rebase_that_is_not_ours`（RED：别人的 rebase 目录被删） |
| 6 | `continue_rebase`：没有进行中的 rebase、HEAD 在原分支且仍是 `origHead` 时交回 `from_baseline_ready` 重走 S8 后半与 S9；`sync_in_rebase_failed` 文案指向「继续」 | `continue_retries_a_rebase_that_never_started`（预置 `.git/index.lock` 让 rebase 确定地启动失败；RED：第二次继续 `sync_in_state_changed`） |
| 7 | 新增 `untracked_leftovers`（`diff --diff-filter=A HEAD <from>` 且磁盘上还在、不在索引里）；`checkout_verified` 以切换前的 HEAD 为 `from` 一起核对；`settle_checkout_residue` 以新旧基线为 `from`，字节是基线 blob 才删，否则指名停下 | `continue_after_s8_left_a_server_added_file_behind`（RED：`sync_in_rebase_failed`「untracked … would be overwritten」）、`abort_after_s8_left_a_server_added_file_behind`、`a_leftover_whose_bytes_are_not_the_baseline_is_not_deleted`、Windows 专属 `a_file_that_checkout_could_not_delete_is_named_and_rolled_back`（真的以 share=Read 占住文件；RED：停在 S8 而不是回退） |

### 偏离计划

- **A14 语义变化**：原测试 `continue_refuses_when_the_rebase_was_aborted_outside` 要求「在外面 `rebase --abort` 之后继续」拒绝。现在改为重新走 S9（测试改名 `continue_replays_again_when_the_rebase_was_aborted_outside`，断言再次得到 `RebaseConflict`）。原测试防的是「直接进 S10、把 WIP 放到没重放过的旧分支上」，新行为同样不会发生；而「rebase 启动就失败」与「在外面被中止」的现场完全相同，分不出来，拒绝会让 #6 永远走不通。
- journal 新增两个字段、`Released` 新增一个字段（spec §7.6 已同步）。

### 计划没写的坑

- 同名未跟踪文件即使内容与要写进来的 blob 完全相同，git checkout / rebase 也拒绝（「would be overwritten」），2026-09-26 在 git 2.51.2.windows.1 实测。
- rebase 在最初 checkout onto 时失败：退出 1、打印 `Aborting` / `could not detach HEAD`，**不**留 `rebase-merge`，HEAD 与分支都不动。
- 有同名 tag 时 `git checkout <b>` 仍然切到分支（附着，stderr 打 `refname is ambiguous`），只有 `heads/<b>` 这种消歧义写法会 detached。

### 为什么这么改是安全的

- 删除未跟踪残留只发生在「路径在某个基线 commit 里、不在 HEAD、不在索引、磁盘字节就是那个 blob」时，删掉的字节随时能从基线取回；任何一个文件不满足就整体不动，记录保留。候选路径来自 `git diff --name-only`，pathspec 为空时直接返回（§12.2）；`ls-files` 只在候选非空时调用。
- `leftOrig` 先落盘再 checkout：崩在两步之间时它为真而 HEAD 没动。此时 WIP 已在 stash 里（S3 只在 S1 成功之后执行），比对通常是空的；若中断后用户又改了白名单内的文件，只有字节恰好等于某个基线 blob 的才会被写回 HEAD（那份字节能从基线取回），其余指名停下、不覆盖。
- 「继续」重走 S9 前的所有复核（分支、`origHead`、残留、线性）都在 `from_baseline_ready` 里，与首次执行相同。
- `require_our_rebase` 只让放弃多一道拒绝，不增加任何写操作。

### 仍未做

- #5 / #8 / #9 / #10 记为 deferred（见上表）。
- 真实窗口 E2E 仍未跑。

## PR #49 review（第三轮）

### review 与核实

外部 review 又提了 6 条（3 × P1、3 × P2）。逐条核实全部成立：#3 用临时 git 仓库、#4 / #5 用临时 svn 仓库实测复现，#2 实测 `rebase-merge/` 里记的身份，#1 / #6 读代码确认。

| # | 发现 | 核实 | 定级（review → 核实后） | 处置 |
|---|---|---|---|---|
| 1 | `applying-stash` 继续时不复核当前分支：撞名挡住后切到别的分支再继续，WIP 被合进那条分支、备份被 drop、报 Done | 成立。冲突留下 `UU` 时 git 拒绝切分支，主要是撞名这条路 | P1 → High | 已修 |
| 2 | `require_our_rebase` 只比 head-name：同一条分支上的另一次 rebase 会被认成本次的，放弃时被中止 | 成立；触发要先在外面结束本次 rebase | P1 → Medium | 已修 |
| 3 | stash 撞名预检不看祖先：`Code/Sub` 成了普通文件时 `Code/Sub/u.cs` 取元数据报错，被当成不存在 | 成立（实测：`stash apply` 退出 1、`cannot create directory`，已跟踪部分已写入）。不丢数据，stash 仍在 | P1 → Medium | 已修 |
| 4 | S5 之后与预览都不认属性冲突 | 成立（实测：update 退出 0，`item="normal" props="conflicted"`，汇总 `Property conflicts: 1`） | P2 → Medium | 已修 |
| 5 | 只改属性的远端改动不进白名单外确认 | 成立（实测：`repos-status item="none" props="modified"`；解析器原先不读远端 props） | P2 → Medium | 已修 |
| 6 | 执行时不核对预览的分支名 | 成立。不丢数据，但重放的是用户没确认过的分支 | P2 → Low–Medium | 已修 |

### 修复与验证

| # | 修法 | 测试（先红后绿） |
|---|---|---|
| 1 | 新增 `require_restore_target`，`continue_sync` 的 `applying-stash` 两个方向都先过它：必须在原分支上；往前走要求 `svn/base` 是 HEAD 的祖先（中间多了用户自己的提交不要紧），往回走要求 HEAD == `origHead` | `continue_does_not_put_the_wip_on_a_branch_switched_to_meanwhile`、`continue_after_a_blocked_rollback_refuses_a_moved_branch`（RED：两条的「继续」都直接成功） |
| 2 | `require_our_rebase` 另比 `rebase-merge/orig-head == origHead` 与 `onto == svn/base`（`rebase_head_name` 泛化为 `rebase_state`） | `a_new_rebase_of_the_same_branch_is_not_taken_for_ours`（RED：放弃把用户的 rebase 中止了）。本次 rebase 自己的继续 / 放弃用例全绿，证明真实的 `onto` / `orig-head` 形状对得上 |
| 3 | `blocking_ancestor`：逐级看祖先，存在却不是目录（含符号链接）即按撞名报**那一级**（用户要挪走的是它），一个字节都不 apply | `restore_refuses_when_a_parent_of_an_untracked_file_is_now_a_file`（RED：`sync_stash_apply_failed` / `cannot create directory`） |
| 4 + 5 | `UpdateStatusEntry` 新增 `repos_props`，并收拢两个判据：`is_conflicted()`（内容 / 树 / 属性冲突）、`is_incoming()`（远端 item 或 props 非 `none`）；预览与 S5 之后的冲突检查都改用它们 | 解析：`update_status_counts_a_props_only_incoming_change`、`update_status_sees_property_conflicts`（XML 照实测抄；先编译失败）。集成：`a_props_only_change_on_both_sides_is_listed`、`an_existing_property_conflict_is_refused`、`a_property_conflict_during_update_rolls_back`（RED：清单为空 / 预览放行） |
| 6 | `SyncInExpect` 新增 `orig_branch`，`verify_expect` 一并比较；前端 `expectFromPreview` 带上 `origBranch` | `a_branch_switched_since_the_preview_is_refused`（先只加字段不加比较：RED，同步在 other 上跑完了）；`sync_in_types_cross_ipc_in_camel_case` 钉住 `origBranch`；vitest `expectFromPreview` 用例（RED） |

### 偏离计划

- `SyncInExpect` 多了一个必填字段，`svn_sync_in` 命令的 JS 参数形状随之改变（`commands/svn_sync.rs` 模块注释已更新）。**必须在 `pnpm tauri dev` 里实跑一次「更新」**才算验证过（`AGENTS.md` §7.3）。
- 撞名结果里可以出现「祖先那一级」的路径（`Code/Sub`），不只是 stash 里的文件本身。前端 `stashBlocked` 文案（「服务器新增了与你的未跟踪文件同名的文件」）对这种形状仍然成立。

### 计划没写的坑

- svn 属性冲突：`svn update --accept postpone` 退出 0，汇总行 `Property conflicts: 1`，节点 `item="normal" props="conflicted"`，旁边多一个 `.prej` 未版本化文件（2026-09-26，svn 1.14 实测）。
- 只改属性的远端改动：`<repos-status props="modified" item="none">`。
- `.git/rebase-merge/` 里 `onto`、`orig-head` 都是完整 OID；本机 `rebase.autoStash=true` 时还会多一个 `autostash` 文件（与本修复无关）。
- 祖先被普通文件占着时，Rust 对子路径的 `symlink_metadata` 报错，看起来和「不存在」一样。

### 为什么这么改是安全的

- #1、#2、#6 都只是多一道拒绝，不增加任何写操作；拒绝时记录与 stash 原样保留。
- #3 只让更多情形走进已有的 `Blocked` 出口（一个字节都不动）。
- #4 让属性冲突走已有的「S5 冲突 → 回退」出口，与内容冲突一致；#5 只让确认清单多列文件。`repos-status` 的 `props` 属性按 svn 的 schema 是必有的，缺了按 XML 解析错误处理（与现有 `item` 一致），不静默放行。

## PR #49 review（第四轮）

### review 与核实

复查确认第三轮六条的原始复现场景都已覆盖，另提两条 P1。核实（读代码；review 已在临时仓库实测）：

| # | 发现 | 核实 | 定级（review → 核实后） | 处置 |
|---|---|---|---|---|
| P1-1 | `require_our_rebase` 的 `onto` 与**当前** `svn/base` 比：本次 rebase 被外部中止、基线被挪走、用户在同一原分支 rebase 到新基线时三项全对上，放弃会中止用户的 rebase | 成立。同一根因另有两处 review 没提：S9 的重放目标、往前走的放回复核（`require_restore_target`）都用当前 `svn/base`。另发现后端的重建基线 / 行尾迁移只看锁、不看 journal——冲突期间锁是放开的，只有界面藏起入口挡着 | P1 → Medium（界面内挪不动基线，要外部工具） | 已修 |
| P1-2 | stash 冲突后外部 `reset --hard`（或 `reset --hard svn/base`），WIP 与冲突都没了，复核全过，继续会 drop 唯一的备份、报 Done | 成立 | P1 → High | 已修（用户选方案 A） |

### 方案选择（P1-2）

| 方案 | 做法 | 取舍 |
|---|---|---|
| **A（采用）** | 放回时停下过的（`stashApplied` 为真），继续收尾时不 drop：只删 journal 与钉住的 ref，stash 条目留给用户，结局 `settled` 说明备份还在 | 从根上消除「删掉唯一备份」；代价是每次 stash 冲突在 stash 列表里留一条，与 spec §13 S10「解决后 drop」不一致（spec 已改） |
| B | 进入 applying-stash 时记 HEAD；继续时要求 HEAD 未变、stash 涉及的每个路径仍与 HEAD 不同 | 「WIP 还在」只能推断；解决冲突时全取 HEAD 版本会被误拒 |

用户决定按推荐走 A。

### 修复与验证

| # | 修法 | 测试（先红后绿） |
|---|---|---|
| P1-1 | 新增 `published_baseline`：取 journal 的 `newBaselineOid`，`svn/base` 不等于它就 `sync_in_state_changed`。S9 直接以该 OID 为 upstream；`require_our_rebase` 的 `onto`、`continue_rebase` 的「已落在新基线上」、`require_restore_target` 往前走的祖先判断都改用它。命令层 `refuse_while_unfinished`：有 journal 时 `svn_rebuild_baseline` / `svn_migrate_eol` 拒绝（`sync_in_unfinished`） | `abort_does_not_abort_a_new_rebase_onto_a_moved_baseline`（review 的场景；RED：用户的 rebase 被中止）、`a_baseline_moved_before_the_replay_is_refused`（RED：已经重放到被挪走的基线上）、`continue_restoring_refuses_a_baseline_moved_back`（RED：继续直接成功）、`rebuilding_the_baseline_is_refused_while_a_sync_is_unfinished`（RED：重建执行了） |
| P1-2 | `restore_step` 在 `stashApplied` 为真时只查未解决文件，然后 `unpin` + 删 journal，返回 `RestoreStep::Kept`；往前走映射为 `Settled { rolledBack: false }`，往回走为 `true`。同一进程里刚刚干净 apply 完的照旧 drop。前端 `settled` 文案、「继续」确认框不再说「清理备份」 | `continue_after_the_wip_was_reset_away_does_not_delete_the_only_backup`（review 的场景）、`continue_after_resolving_a_stash_conflict_keeps_the_backup`（原 `…drops_the_stash` 改写）：先只加 `Settled` 变体不改行为，两条都 RED 于 `Done { new_rev: 2 }`；`sync_in_types_cross_ipc_in_camel_case` 钉住 `settled`；vitest 两条（RED） |

### 偏离计划

- **偏离 spec §13 S10 / §7.6「解决后 drop」**：见上方案 A，spec 已改。E2E 清单不受影响——场景 4（撞名 → 继续）走的是同一进程里干净 apply，仍然 drop。
- 新结局 `SyncInOutcome::Settled`，前端联合类型随之新增 `settled`。

### 计划没写的坑

- **Bash 工具的 heredoc 反斜杠折叠又犯了两次**（记忆里早有这一条）：第三轮 `a_branch_switched_since_the_preview_is_refused` 里的 `"a2\n"` 被写成真换行（LF 工作区下行为恰好相同，所以测试是绿的），本轮 `refuse_while_unfinished` 的续行变成一串空格。两处都已改正；扫描「以 `"` 开头的行」确认没有别的。

### 为什么这么改是安全的

- `published_baseline` 只增加拒绝，不增加写操作；拒绝时记录、stash、rebase 状态原样保留。
- `Kept` 比原来**少做**一件事（不 drop），stash 条目与对象都还在；钉住的 ref 删掉后，stash reflog 仍让对象可达。
- `refuse_while_unfinished` 只读 journal；journal 坏了按现有语义往上抛 `svn_journal_corrupt`，不吞成「没有记录」。

## 真实窗口 E2E 发现（2026-09-28）

### 场景 2：结局框卡在「处理中…」

- **现象**（用户截图）：rebase 冲突后弹出的「更新遇到冲突」框里有转圈的「处理中…」，确认按钮是禁用的 `…`，没有清单写的「知道了」；Esc、点遮罩都关不掉。
- **根因**：`useConfirm` 渲染 `<Dialog>` 不带 key。「更新」确认框的 `action` 跑完调 `finish(true)`（`setPending(null)`），`handleSvnUpdate` 紧接着 `showOutcome` 弹结局框（`setPending(下一个)`）；两次更新被 React 18 合并成一次渲染，`<Dialog>` 从未卸载，上一个框的 `busy=true` 被带进新框（`finish` 不复位 `busy`，它原本假设关框即卸载）。同一形状出现在所有「带 action 的框之后紧跟另一个 `confirm`」的地方：继续 / 放弃之后的结局框也中招。
- **修法**：`Pending` 带自增 `id`，`<Dialog key={pending.id}>`，每个请求都是全新组件。
- **测试**：修复时仓库没有组件测试设施，先以真实窗口复测为准；随后经用户同意新增 devDependencies `@testing-library/react` 16.3.3、`@testing-library/dom` 10.4.2（前者的 peer）、`jsdom` 30.1.1，补了 `tests/vitest/ConfirmDialog.test.tsx` 4 条。只有这个文件用 `// @vitest-environment jsdom`，全局环境仍是 node，既有测试不受影响。
  - 回归两条（照 `RepoTab` 的真实形状：带 action 的框跑完后立刻 `await confirm` 弹结局框）：新框的「知道了」可点、没有「处理中」；新框能用 Esc 关掉。**变异确认**：去掉 `key` 后这两条都红（`findByRole('知道了')` 超时——按钮文字是 `…`），恢复后全绿。
  - 另两条钉住既有行为：action 进行中进度与禁用；action 抛错时留在原地、按钮恢复。
  - `pnpm build` 通过 / `pnpm test` 28 个文件 439 passed。`AGENTS.md` §8、`CLAUDE.md` 补了组件测试的写法与界限（jsdom 不渲染布局，不替代 §6.1 的视觉验证）。
- **E2E 清单不用改**：清单写的「点『知道了』」是对的行为，错的是实现。

### 场景 6 → 7：旧的红色错误条与新横幅叠在一起

- **现象**（用户截图）：场景 6 预览被拒留下的 `sync_in_merge_in_range` 红条，在场景 7 撤掉 merge、外部 update 之后仍挂着，顶部同时有黄色「工作副本已在外部更新」横幅与这条过时的红条。
- **原因**：顶部错误条（`reportError`）全应用常驻、点 × 才关；各操作开始时都不清它（仅「双击 tag 检出」例外）。不是本 PR 新引入的机制，但在「更新」这里表现为过时的前置检查结论。
- **方案**（用户选 A + C）：A 重试即清——「更新」与横幅上的继续 / 放弃开始时先 `clearError()`，这次再失败照常报新的；C 清单场景 6 末尾加「点 × 关掉」，场景 7 的「看到别的」表加一行。**否决 B**（`repo-changed` 时清 `sync_in_*` 错误）：每个同步命令结束时后端都补发一次 `repo-changed`，错误会一闪即逝。
- **重构**：为了能测，把 `afterSync` / `showOutcome` / `handleSvnUpdate` / `handleSyncAction` 从 `RepoTab.tsx` 原样移到新文件 `src/components/svnSyncFlow.ts`（`createSvnSyncFlow(deps)`，不依赖 React），`RepoTab` 只注入依赖。
- **测试**：`tests/vitest/svnSyncFlow.test.ts` 4 条（node 环境，`vi.mock` 掉四个 invoke 包装）。先做纯搬移：全流程那条绿（证明搬移没改行为），三条清错的 RED；加上 `clearError()` 后全绿。其中一条钉住「这次预览又被拒时，新错误在清除之后报出、不会被清掉」。`pnpm build` 通过 / `pnpm test` 29 个文件 443 passed。

### 场景 7：`svn/base` 的两个 OID 在图里找不到——设计行为，清单补说明

- **现象**：`G log --oneline -2 svn/base` 给出两条 `svn: r8`（与清单一致），但图里搜不到这两个 hash。
- **结论**：不是缺陷。`spliceHistory`（spec §10.3 / §10.4）把基线 commit 换成 `svn:N` 节点，hash 列显示 `rN`、标题取 SVN log 的提交说明、`svn/base` 标签重映射到该行；同 revision 的两条基线落在同一行 `r8`。清单场景 7 补了一段说明（`a990c28`）。

### 结果

- 2026-09-30 用户在 `pnpm tauri dev` 真实窗口跑完清单 8 个场景，全部符合预期。途中发现的两个缺陷（场景 2 结局框卡「处理中」、场景 6 → 7 旧红条）已修复并复测通过。

## PR #49 Codex 独立 review（2026-09-30）

范围 `29a3fab..44ef36e`（第四轮修复、确认框 key、测试设施、`svnSyncFlow`、两个文档提交）。Codex 只读审查，`cargo test` 在它的环境里没能起进程，Rust 测试由实现者本地跑。6 条逐条对照代码核实，用户决定「全修」（第 2 条的 `svn/base` 部分与第 3 条按下述理由记录、不在本 PR 修）。

| # | 定级（Codex → 核实） | 结论 | 处理 |
|---|---|---|---|
| 1 | P1 → P1 | 成立：干净 apply 后先落 `stashApplied=true`、再 drop、再删 journal。崩在 drop 与删 journal 之间，重开「继续」走 Kept，报「备份还在 stash 里」——实际已删 | 修 |
| 2 | P1 → P1（stash）/ 既有（`svn/base`） | stash 部分成立：冲突出口放开锁，应用内 drop / pop 只看锁，删掉本次那条后「继续」的 Kept 拆掉最后的钉子。删除 / 重命名 `svn/base` 在 Phase 1 起任何状态下都没拦，非本 PR 引入 | stash 修；`svn/base` 记录，另开 |
| 3 | P2 → 既有 | 成立但源自 Phase 2b（#46）：重建 / 迁移「先查锁再做、做时不持锁」。第四轮只在旁边加了 journal 检查。同步进行时有模态框挡着，界面上难以并发触发 | 记录，另开（修法：两命令持同一把同步锁） |
| 4 | P2 → P2 | 成立：`from_baseline_ready` 先 S8 切分支、清残留，才调 `published_baseline` | 修 |
| 5 | P2 → P3 | 理论成立：`useConfirm` 被新请求顶替时，旧框的 `onDone` 无条件清空，把新框也关掉、新 Promise 永不结束。机制早于 key 修复；实际要连点、且第二次预览恰在第一次确认执行中返回 | 修（用户要求一起修） |
| 6 | P3 → P3 | 成立：验证表 `pnpm tauri dev` 一行仍写「未跑」，测试数停在第四轮 | 修 |

### 修法与测试

- **#1**：`restore_step` 的 Kept 分支先用新的 `svn_sync_stash::stash_position` 查 OID 是否还在栈里；在 → `Kept`（`Settled`），不在 → `Finished`（往前 `Done`、往回 `Aborted`）。`drop_wip` 改用同一个 helper 定位。测试 `continue_after_a_crash_between_drop_and_clearing_the_record_claims_no_backup`：没有注入崩溃的手段，用「stash 冲突 → 解决 → 按我们自己的 `drop_wip` 删掉」构造与那一刻相同的 journal / stash 栈 / 钉子状态。RED：返回 `Settled`。
- **#2**：`vcs/svn.rs::assert_not_sync_backup`，`stash_drop` / `stash_pop` 在 journal 的 `stashOid` 恰是 `stash@{index}` 时报 `sync_in_unfinished`；别的 stash 照常，`stash_apply` 不删东西、不挡。测试 `the_app_does_not_drop_the_sync_backup_while_the_sync_is_unfinished` 走 `commands::stash` 的真实入口（`backend_for` → `SvnGitBackend`），并断言另一条 stash 仍可删。RED：drop 成功。已知窄窗：查位置与按 index drop 之间外部又压一条会错位——与 `GitBackend` 按 index 操作的既有语义相同，不在本条范围。
- **#4**：`published_baseline` 挪到 `from_baseline_ready` 开头。它在所有能走到这里的路径上本来就要求成立（S8 不动 `svn/base`），前移只改变失败时机。测试 `a_baseline_moved_while_still_on_it_is_refused_before_switching_back`：baseline-ready、HEAD 仍在 `svn/base`、基线被外部挪走 → 报 `sync_in_state_changed`，HEAD 仍在 `svn/base`、磁盘上 `Code/B.cs` 未变。RED：HEAD 已被切到 `work`。
- **#5**：两处都改。
  - `useConfirm` 的 `onDone` 只在 `pending.id` 仍是自己时清空。测试见 `ConfirmDialog.test.tsx` 新增一条（RED：第二个框被关掉）。
  - `svnSyncFlow` 加 `running` 标记，同一时间只走一个流程。**计划没写的坑**：`RepoTab` 每次渲染都重新 `createSvnSyncFlow`，标记放闭包里会随渲染重置，所以由调用方 `useRef` 注入；测试按这个形状每次调用都重新 create、共享同一个 `running`（RED：第二次调用等在同一个预览上超时）。`useRef` 必须排在 `RepoTab` 里 `loading` 的提前 `return` 之前，否则 hooks 数量随渲染变化。
  - 遗留：顶替时旧请求的 Promise 仍不结束（机制未变），`running` 挡住了同步流程之间的重叠；其他调用方的确认框都是用户点击触发，模态框挡着。
- **#6**：验证表按实际更新。

验证：`cargo fmt --check` 干净 / `cargo clippy --all-targets` 0 告警 / `cargo test --no-fail-fast` 729 passed 0 failed；`pnpm build` 通过 / `pnpm test` 29 个文件 446 passed。**没有改 Tauri 命令签名**，未重跑真实窗口。

### 后续：#2 的 `svn/base` 部分与 #3（PR #49 合并后另开的小 PR，分支 `worktree-svn-guard-followups`）

- **#2 `svn/base` 删除 / 改名**：`vcs/svn.rs::refuse_baseline_branch`。`delete_branch` 与 `rename_branch` 的两端（`old`、`new`）都查，精确匹配 `svn/base`（容忍 `refs/heads/` 前缀），报 `svn_baseline_branch_protected`。`svn/base-backup` 之类照常。**计划没写的坑**：比较不区分大小写。2026-09-30 在临时仓库实测（Windows / NTFS，ref 为散装文件），`git branch -D SVN/BASE` 删掉的就是 `svn/base`，`git branch -m SVN/BASE x` 把它改了名；测试 `a_differently_cased_name_does_not_get_around_it`（RED：删掉了）。界面 `BranchView` 的「Rename…」对基线置灰，与既有的「Delete…」一致。与锁无关：任何状态下都拒绝。
- **#3 重建 / 迁移全程持锁**：`commands/svn.rs::hold_for_baseline_move`。它先查 journal，再 `svn_lock::acquire`，拿到锁后再查一次 journal；两条命令把锁绑在 `_lock` 上，持到命令结束。
  - 先查一次，是为了不给注定被拒的操作去接管崩溃留下的残锁。
  - 锁内复核，是因为查与拿之间可能刚有一次同步停在冲突上：冲突出口放锁、留 journal。
  - opid 带 `baseline-` 前缀，「正忙」时看得出持有者不是一次更新。
  - 持锁期间界面会显示通用的「这个工作副本正在进行一次同步」横幅，文案没有区分重建。可以接受：此时重建的确认框正显示进度。
- **测试**（新文件 `tests/svn_baseline_guard.rs`，fixture 仓库）：
  - 基线删不掉。RED：删掉了。
  - 基线改名两个方向都被拒。RED：改名成功。
  - 别的分支照常能删、能改名。
  - 持锁期间 `run_sync_in` 报 `sync_in_progress`、不写 journal、HEAD 不动，放锁后 `is_held` 为假。
  - 被占锁 / 有 journal 时拒绝，且被拒时不留锁。
  - `svn_write_guard.rs` 新增接线测试：两条命令在干活之前调用持锁入口，且不是 `let _ =`；入口在拿锁之后复核 journal。RED：找不到持锁入口。
  - 原 `GUARDED_COMMANDS` 表里的这两条挪到新测试去守，因为拿锁本身就包含了「有人持有就拒绝」。
  - **竞态本身**（检查与移动之间插进一次同步）没有确定性的复现手段。持锁入口从结构上消除了这个窗口；行为测试守「持锁期间同步被拒」，接线测试守「命令确实全程持锁」。
- 验证：`cargo fmt --check` 干净 / `cargo clippy --all-targets` 0 告警 / `cargo test --no-fail-fast` 742 passed 0 failed；`pnpm build` 通过 / `pnpm test` 30 个文件 456 passed。没有改 Tauri 命令签名。

