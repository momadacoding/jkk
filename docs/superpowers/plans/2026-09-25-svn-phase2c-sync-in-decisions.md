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
| 6 | 继续 / 放弃（含回原分支） | ⬜ | | |
| 7 | S3 / S4 / S5 失败回退 | ⬜ | | |
| 7A | 外部整体 update 之后由「更新」接管（A16） | ⬜ | | |
| 8 | Tauri 命令 | ⬜ | | |
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
