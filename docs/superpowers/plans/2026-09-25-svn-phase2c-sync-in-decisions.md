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
| 9 | 前端纯逻辑 | ⬜ | | |
| 10 | 前端接线 | ⬜ | | |
| 11 | E2E + spec 修订 + 收尾 | ⬜ | | |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | ⬜ |
| `cargo clippy --all-targets` | ⬜ |
| `cargo test` | ⬜（基线：master `1db8210` 上 598 passed） |
| `pnpm build` | ⬜ |
| `pnpm test` | ⬜（基线 383 passed） |
| `pnpm tauri dev` | ⬜ |

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
