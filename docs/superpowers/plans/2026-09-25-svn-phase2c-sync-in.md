# Phase 2c：`sync_in` 编排 + 冲突出口 + 继续 / 放弃 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 SVN 混合仓库「能更新」：一次点击走完 spec §7.3 的 S0–S11，冲突时停在可解决的状态并释放锁，任何一步中断（冲突或崩溃）都能在横幅上继续或放弃。

**Architecture:** 后端新增三个 engine 模块——只读前置检查 `svn_sync_check`、WIP 压栈与取回 `svn_sync_stash`、编排与恢复状态机 `svn_sync_in`——全部直接调 `run_git` / `run_svn_*`，**不经过** `trait Vcs`（那一层的守卫会挡住持锁者自己）。命令层 `commands/svn_sync.rs` 负责挂起 watcher、结束后补发 `repo-changed`。前端不新建对话框组件：预览、确认、执行、继续 / 放弃全部由纯函数生成 `ConfirmRequest`，交给现有 `ConfirmDialog`。

**Tech Stack:** Rust（git CLI 写 / libgit2 读 / svn CLI）、Tauri 2 命令、React 18 + TypeScript、vitest、cargo test（fixture 仓库）。

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`——主要是 §5.1、§5.2、§7（全节）、§12.2、§12.4、§13、§14.2。执行者必须**两份一起读**：本计划只在 spec 沉默或与实测冲突的地方做决定，并在下面「对 spec 的补足与偏离」里逐条写明。

**上一期：** Phase 2b（PR #46，已合并为 `1db8210`）交付了跨进程锁 `svn_lock`、写前 journal `svn_journal`、watcher 挂起、「上次同步未完成」横幅。本期在它上面施工，**不改它们的对外语义**。

---

## Global Constraints

- 本仓库主干是 `master`；本期在 worktree `.claude/worktrees/svn-phase2c-sync-in`、分支 `worktree-svn-phase2c-sync-in` 上施工（基点 `1db8210`）。
- `AGENTS.md` 最高优先级：**所有 git / svn 写操作的验证只在临时 fixture 里做**，禁止碰本仓库或任何真实工作副本。
- 所有 git CLI 调用走 `engine::git_cli::{run_git, run_git_code, run_git_env, run_git_stdin, run_git_bytes}`；所有 svn 调用走 `engine::svn_cli::{run_svn, run_svn_code, run_svn_bytes}`（`AGENTS.md` §7.3）。错误一律 `GitError::new(code, message)`，不向前端抛裸字符串。
- **空 pathspec / 空参数列表一律拒绝**（spec §12.2，`AGENTS.md` §6.2）：`git stash push -- <白名单>` 在白名单为空时必须报错，不能退化成「全部」。
- **失败只回滚本流程自己做的修改**（spec §12.4）。
- 新 Tauri 命令全部 `#[tauri::command(async)]`（`tests/commands_are_async.rs` 守着）；返回 / 参数结构体一律 `#[serde(rename_all = "camelCase")]`，枚举再加 `rename_all_fields = "camelCase"`，并在 `tests/svn_backend.rs` 里断言序列化后的字段名。
- 不新增任何 npm 包或 crate。`Cargo.toml` 的 `rust-version = "1.77.2"`：不用 `File::try_lock`、不用 `#[allow(..., reason = ...)]`。
- 注释用中文、解释「为什么」；前端单引号、尾随逗号、`@/` 别名。
- 验证：Rust 改动跑 `cargo fmt --check` → `cargo clippy --all-targets` → `cargo test`（在 `src-tauri/`）；前端改动跑 `pnpm build` → `pnpm test`。**本仓库没有 lint、没有 PR 门禁 CI**，禁止声称「lint 通过」「CI 通过」。`tsc` 不覆盖 `tests/vitest/`。
- 新 Tauri 命令必须实跑一次 `pnpm tauri dev` 确认 invoke 通（`AGENTS.md` §7.3）。
- 决策日志：`docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md`，Task 0 建，每个 Task 完成立即更新（`AGENTS.md` §2.1）。
- 本期**不做**：`sync_out`（Phase 3）、§9.4 恢复向导（2d）、pinned 模式下的两步 update（本期拒绝，见 A1）、横幅让位时「灰化保留其余横幅」（2b N6，见下文「本期不做」）。

---

## 对 spec 的补足与偏离（review 先读这一节）

执行前在临时 fixture 上实测过的事实，一律标「实测」。每一条都要抄进决策日志对应 Task 的「偏离计划 / 计划没写的坑」。

| # | 内容 | 类型 | 依据 |
|---|---|---|---|
| A1 | **pinned 模式下 `sync_in` 在动手前拒绝**（`sync_in_pinned_unsupported`），§6.5.3 的两步 update 推迟。必测 #53 随之推迟 | 偏离（用户 2026-09-25 决定） | 用户确认公司项目的 `code` / `script` 没被钉住，§6.5.4 也说该路径「正常流程一步都不走」 |
| A2 | **新增 phase `applying-stash`** 与 journal 字段 `stashApplied`。spec §7.6 没有为 S10 定义 phase：S9 成功后若只留 `rebase`，崩溃重开时分不清 stash 有没有 apply 过 | 补足 | 实测 T2：对已 apply 过的 stash 再 apply 一次，git 拒绝且不动任何文件；所以「apply 完、`stashApplied` 还没落盘」这条缝最坏是一条报错，不会破坏数据 |
| A3 | **S10 在 apply 之前先查未跟踪文件撞名**，撞了就一个字节都不 apply，返回 `StashBlocked` | 补足 | 实测 R1：stash 里既有已跟踪改动又有撞名的未跟踪文件时，`git stash apply` **先写已跟踪部分、再在未跟踪上失败**（退出 1，无 `UU`）——半套 apply |
| A4 | **S5 的成败不看退出码**：`svn update --accept postpone` 产生文本冲突或树冲突时**退出 0**；必须事后 `svn status --xml` 查 `item="conflicted"` 或 `tree-conflicted="true"` | 补足 | 实测 Q2 / Q3；树冲突节点的 `item` 是 `added`，只看 `item` 会漏 |
| A5 | **S5 之后的「放弃」= 回原分支并恢复 WIP**：HEAD 在 `svn/base` 时，先断言白名单内 W-file 在 svn 看来**全部干净**（磁盘 == 服务器内容，可从服务器取回），再 `git checkout -f <origBranch>`；不干净就停下不覆盖 | 补足（用户 2026-09-25 决定） | 实测 R2：普通 `git checkout` 会把磁盘上的服务器内容**带到原分支上变成本地修改**，必须 `-f` |
| A6 | 前置检查对**任何** svn 冲突态（含 W-file）都拒绝，比 §7.2 第 4 条「W-file 外」更严 | 偏离（更保守） | W-file 的冲突标记在 stash 之后仍留在 `.svn` 里，S4 必然失败；早拒绝零代价 |
| A7 | `phase = stashed` 且 `stashOid` 缺省 ⇔ 白名单内没有可压的改动 | 补足 | 实测 P1：无可压内容时 `git stash push` 退出 **0** 且 `refs/stash` 不变——判定只能比对前后的 `refs/stash` |
| A8 | S10 用 `git stash apply <OID>`（**不带 `--index`**，照 spec 原文）：「已暂存」这个状态会丢，内容不丢。确认框明说 | 照 spec，写明代价 | 实测 R3：`--index` 能保留，但会新增「索引冲突时拒绝」一类失败，本期不引入 |
| A9 | S7 **不跑**基线体检，候选索引**从 HEAD 树起步** | 照 spec | §6.8.5「不在 `sync_in` 的 S7 之后跑」；§6.3.1 第 638 行「其余录制入口从当前 HEAD 树开始」 |
| A10 | `phase = start` 时「继续」与「放弃」等价：删 journal | 补足 | `start` 的定义就是「还没动任何东西」 |
| A11 | 从 `stashed` 继续前复核：HEAD 在原分支（且 tip 未变、白名单内没有新改动），或在 `svn/base`（且 tip == `baselineOidBefore`）；否则停下 | 补足 | 锁在崩溃后不存在，中断期间用户可能动过工作区 |
| A12 | S3 / S4 失败的回退用 `git checkout -f <origBranch>`，**不做** A5 的断言 | 补足 | 此时 S5 还没跑，磁盘上被跟踪的内容全部来自 git 自己（WIP 已压栈），`-f` 只会还原 git 写下的字节。只限**本进程刚跑到这里**的路径；从崩溃里恢复时一律走 A5 |
| A13 | journal 的 `flow = sync_out` 时，继续 / 放弃一律拒绝（`sync_out_not_supported`） | 补足 | Phase 3 才有 `sync_out` |
| A14 | 冲突出口释放锁后，「继续」「放弃」都要**重新拿锁**并复核 HEAD / 分支 / rebase 状态 | 照 spec §7.4 | 冲突期间用户要暂存文件，锁必须放开；这段时间里任何外部 git 操作都可能发生 |
| A15 | **每一次 `git checkout` 之后都核对它真的切干净了**：在目标分支上，且 `git diff --name-only HEAD -- <白名单>` 为空。不满足就按「文件被占用」处理并指名这些文件，**不看退出码** | 补足 | 实测 L1：文件被以 share=Read 打开（Unity 的方式）时，`git checkout svn/base` **退出 0**、HEAD 已切走，而那个文件还是原分支的内容；实测 L1′：share=None 时 checkout 在写任何东西之前整体拒绝（退出 1，HEAD 不动）。所以「checkout 失败后 HEAD 仍在原分支上 ⇒ 一个字节都没写」成立，而「退出 0 ⇒ 切干净了」不成立 |
| A16 | **外部整体 update 过（`nodeRevs` 单值且大于基线）时，「更新」不拒绝，而是在锁内、S0 之前先调 Phase 1 的 `rebuild_baseline` 把基线跟上，再走 S0–S11**。mixed-revision 与「退回基线之前」仍拒绝。横幅对「整体前进」降为黄色并给「更新…」按钮（Task 7A） | 偏离 §7.2 第 2 条（用户 2026-09-25 决定） | 用户：每天早上在 TortoiseSVN 里对项目根 update 是常态。实测（Task 7A 表）：git 这一侧服务器改动与本地改动混在一起；重建基线 + S1–S10 之后重新分开；相邻行会变成 stash 冲突，内容不丢 |
| A17 | **S10 撞名预检放宽**：撞名文件在磁盘、stash 的未跟踪部分、HEAD 三处是同一个 blob 时不算撞名，先从磁盘拿掉再 apply；任何一处不同仍按 A3 拦下，且拦下时一个字节都不动 | 补足（A16 的必要条件） | 实测 `probe-ext-added.ps1`：外部 update 带来的服务器新增文件会被 `-u` 收进 stash，接管后又作为已跟踪文件回到磁盘；不处理时 `git stash apply` 报 `already exists, no checkout` 并半套放回，处理后干净 |

---

## Review Focus

spec 没有逐条写到、但最可能咬到真实用户的六类输入。每条都在对应 Task 里有一条钉住它的测试。

1. **WIP 里有未跟踪文件，服务器恰好新增了同名文件** → 不得半套 apply；stash 原样保留，界面指名撞名的文件。钉在 Task 3 `restore_refuses_when_an_untracked_file_would_collide` 与 Task 5 `untracked_collision_after_update_stops_before_applying`。
2. **Unity 开着、占着一个 `.cs`，S3 切分支「成功」却没写进去** → 指名文件、回到原分支、WIP 回来。钉在 Task 7 `a_file_held_open_during_checkout_is_named_and_rolled_back`（Windows，A15；用 share=Read 模拟 Unity——share=None 时 S1 的 `stash push` 就读不了那个文件，走不到 S3，那是另一条失败路径）。
3. **服务器在 S5 途中不可达** → WIP 回来、HEAD 回原分支、基线不动、锁释放。钉在 Task 7 `update_failure_returns_to_the_original_branch`（用「从 `stashed` 继续」制造：先让仓库停在 S3 之后，再把 fixture 仓库改名）。
4. **冲突期间用户在外面 `git checkout` 到别的分支，再回来点「继续」** → 拒绝，journal 与 stash 原样保留。钉在 Task 6 `continue_refuses_when_the_branch_moved_during_a_conflict`。
5. **崩溃停在任意 phase** → 重开后继续 / 放弃都走得通、不丢 WIP。钉在 Task 6 的逐 phase 用例（`start` / `stashing` ×2 / `stashed` ×2 / `capturing-baseline` / `publishing-baseline` ×3 / `rebase` ×3 / `applying-stash` ×3）。
6. **早上先在 TortoiseSVN 里对项目根 update，再打开应用点「更新」**（用户明确说这是每天的第一件事，A16）→ 服务器改动与自己的改动重新分开；同事新增的文件不卡住；自己的未提交改动紧挨服务器改动时停在可解决的 stash 冲突、不丢内容。钉在 Task 7A 的 `an_external_root_update_is_taken_over_and_separated_again`、`a_file_the_server_added_does_not_block_the_catch_up`、`an_external_update_next_to_my_own_edit_stops_as_a_stash_conflict…`。

---

## 文件地图

| 文件 | 新建 / 修改 | 职责 |
|---|---|---|
| `src-tauri/src/commands/watch.rs` | 改 | `WatchSuspension`（RAII：构造即挂起，Drop 即恢复） |
| `src-tauri/src/engine/svn_lock.rs` | 改 | `Drop` 删锁文件失败时短重试；`busy_error` 显示「已持续多久」 |
| `src-tauri/src/engine/svn_journal.rs` | 改 | `Phase::ApplyingStash`、`stash_applied`；写入失败 / 崩溃留下的 `.tmp` 清理 |
| `src-tauri/src/engine/svn_xml.rs` | 改 | `parse_update_status`：`repos-status`、`tree-conflicted`、`against` |
| `src-tauri/src/engine/svn_sync_check.rs` | **新** | §7.2 前置检查 1–6（纯读）与 merge 门槛 `assert_linear`；识别「外部整体 update」（A16） |
| `src-tauri/src/engine/svn_sync_stash.rs` | **新** | S1 压栈（opid 标记、钉 ref）、S10 取回（撞名预检，A3 / A17）、按 OID drop |
| `src-tauri/src/engine/svn_baseline.rs` | 改 | 把 `publish_baseline` 拆成 `create_baseline_commit` + `move_baseline_ref`；`read_baseline` 改 `pub(crate)` |
| `src-tauri/src/engine/svn_init.rs` | 改 | `record_server_tree_into`（候选索引由调用方给）；`record_server_tree` 变成它的包装 |
| `src-tauri/src/engine/svn_sync_in.rs` | **新** | S0–S11 编排、外部整体 update 的接管（A16）、回退、冲突出口、继续 / 放弃状态机 |
| `src-tauri/src/engine/mod.rs` | 改 | 挂三个新模块 |
| `src-tauri/src/commands/svn_sync.rs` | **新** | 四个 Tauri 命令：预览、执行、继续、放弃 |
| `src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs` | 改 | 注册 |
| `src-tauri/tests/common/svn_fixtures.rs` | 改 | `hybrid_with_colleague()`、`wc2()`、`colleague_commits()` |
| `src-tauri/tests/svn_sync_check.rs` | **新** | 前置检查 |
| `src-tauri/tests/svn_sync_stash.rs` | **新** | 压栈 / 取回 |
| `src-tauri/tests/svn_sync_in.rs` | **新** | 主路径、回退、冲突、继续 / 放弃 |
| `src-tauri/tests/svn_xml.rs`、`svn_journal.rs`、`svn_lock.rs`、`svn_watch_suspend.rs`、`svn_baseline.rs`、`svn_backend.rs`、`svn_write_guard.rs` | 改 | 各自补用例 |
| `src/engine/svnSyncIn.ts` | **新** | 预览 → 确认框模型、结果文案、继续 / 放弃的确认框模型 |
| `src/engine/svnJournal.ts` | 改 | `applying-stash`、横幅动作、持锁时长 |
| `src/engine/svnSync.ts` | 改 | `BannerAction.id` 加三个值；「整体前进」横幅降为黄色、给「更新…」（A16） |
| `src/engine/toolbarActions.ts` | 改 | `showsSvnUpdate(caps)` |
| `src/store/repoStore.ts` | 改 | 四个 invoke |
| `src/components/Toolbar.tsx`、`BaselineBanner.tsx`、`RepoTab.tsx` | 改 | 「更新」按钮、横幅动作、确认框接线 |
| `tests/vitest/svnSyncIn.test.ts` | **新** | |
| `tests/vitest/svnJournal.test.ts`、`toolbarActions.test.ts`、`svnSync.test.ts` | 改 | |
| `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md` | 改 | §7.3 S5 / S10 注记、§7.6 新 phase、§18 修订记录 |
| `docs/superpowers/e2e/2026-09-25-svn-phase2c-e2e.md` | **新** | 真实窗口清单 |
| `docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md` | **新** | 决策日志 |

**行尾：** `src/vcs/svn.rs`、`src/commands/watch.rs`、`repoStore.ts`、`BaselineBanner.tsx` 是 CRLF，其余多为 LF。用 Edit 工具改即可；用脚本批量替换前先数 `CR` / `LF` 字节，改完复核（2b 决策日志 Task 3 记过这个坑）。

---

## 错误码一览（前后端共用，Task 10 的文案按它写）

| code | 何时 | 修改了什么 |
|---|---|---|
| `sync_in_progress` | 锁被别人拿着（2b 已有） | 无 |
| `sync_in_unfinished` | 已有未收尾的 journal | 无 |
| `sync_in_git_busy` | git 在 rebase / merge / cherry-pick / revert 中途，或索引有未合并条目 | 无 |
| `sync_in_detached_head` | HEAD 没附着在分支上 | 无 |
| `sync_in_on_baseline` | 当前就在 `svn/base` 上 | 无 |
| `sync_in_out_of_sync` | 完整 `nodeRevs` 校验既不是 `InSync`，也不是「整体前进」（A16：整体前进由执行时接管，不报错） | 无 |
| `sync_in_pinned_unsupported` | 白名单所属 external 被钉住（A1） | 无 |
| `sync_in_svn_conflicted` | 工作副本里有 svn 冲突态（A6） | 无 |
| `sync_in_server_unreachable` | `svn status -u` 失败 | 无 |
| `sync_in_merge_in_range` / `sync_in_merge_check_failed` | §7.3.2 | 无 |
| `include_set_mismatch` | 纳入集合预检不过（沿用既有码） | 无 |
| `sync_in_state_changed` | 锁内复核 / 继续前复核发现 HEAD、分支或基线变了 | 无（或保留 journal 原样）；A16 接管之后的复核不过时，基线已按服务器原版跟上，其余未动 |
| `sync_in_needs_reconfirm` | 执行时出现了预览里没有的「白名单外会被碰到的文件」 | 无 |
| `sync_in_checkout_failed` | S3 / S8 / 回原分支时 checkout 没切干净（多为文件被占用，A15） | S3：已回退；S8：journal 停在 `baseline-ready`；回原分支：journal 保留 |
| `sync_in_baseline_stale` | S4：W-file 在 svn 看来有修改 | 已回退 |
| `sync_in_update_failed` | S5 `svn update` 非零退出 | 已回退 |
| `sync_in_update_conflict` | S5 产生冲突（A4） | 已回退；冲突文件留给 TortoiseSVN |
| `sync_in_unsafe_to_return` | A5 的断言不过：不能 `-f` | journal 保留，未覆盖任何文件 |
| `sync_in_revision_moved` | S7 录制期间 `nodeRevs` 变了 | journal 停在 `capturing-baseline` |
| `sync_in_rebase_failed` | rebase 非零且不是冲突 | journal 停在 `rebase` |
| `sync_in_unresolved` | 点「继续」时还有未合并文件 | 无 |
| `sync_stash_lost` / `sync_stash_missing` / `sync_stash_apply_failed` | §7.3.1 判定树走到「报错停下」 / stash 对象不在 / apply 非冲突失败 | journal 保留 |
| `sync_out_not_supported` | journal 是 `sync_out` 的（A13） | 无 |
| `sync_no_unfinished` | 点继续 / 放弃时没有 journal | 无 |

---

## Task 0：决策日志 + 开工自检

**Files:**
- Create: `docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md`

**Interfaces:** 无

- [ ] **Step 1：跑 `AGENTS.md` §4.2 自检，把结果抄进决策日志**

```powershell
Get-Location
git branch --show-current
git log --oneline -3
git status --short
git merge-base --is-ancestor HEAD origin/master; "HEAD 已在 master 上（新分支正常）: exit=$LASTEXITCODE"
```

期望：路径以 `.claude\worktrees\svn-phase2c-sync-in` 结尾；分支 `worktree-svn-phase2c-sync-in`；`git log` 第一行是 `1db8210`；工作区干净。

- [ ] **Step 2：fresh worktree 没有 `node_modules`，先装依赖（走 PowerShell，Bash 工具没有网络）**

```powershell
pnpm install
```

- [ ] **Step 3：建决策日志**

写入 `docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md`：

````markdown
# Phase 2c 实现决策日志

> `AGENTS.md` §2.1 要求的落库记录。配套计划：`2026-09-25-svn-phase2c-sync-in.md`
> 分支：`worktree-svn-phase2c-sync-in`，基点 `1db8210`

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 0 | 决策日志 + 自检 | ✅ 完成 | 本文件 | （填 Step 1 的实测输出摘要） |
| 1 | 2b 遗留：watcher RAII、锁释放重试、journal tmp 清理、持锁时长 | ⬜ | | |
| 2 | 前置检查 | ⬜ | | |
| 3 | WIP 压栈与取回 | ⬜ | | |
| 4 | 基线录制零件拆分 | ⬜ | | |
| 5 | 编排主路径 S0–S11 + 冲突出口 | ⬜ | | |
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

计划「对 spec 的补足与偏离」表里 A2–A8 的依据。git 2.51.2.windows.1，svn 1.14.2-SlikSvn。

（把计划 A1–A17 逐条抄过来，并在每个 Task 下补「偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的」四节。）
````

- [ ] **Step 4：提交**

```powershell
git add docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "docs(svn): Phase 2c 决策日志与开工自检"
```

（提交规则：本 worktree 分支未合并进 master 时，commit / push 是常规动作；每次都在回复里报告 HEAD 与推送状态。）

---

## Task 1：2b 遗留——watcher RAII、锁释放重试、journal tmp 清理、持锁时长

2b 决策日志把这四条记为「2c 待办」（N3、R5、R7、R6 的后端半边）。本期第一次**真的**有代码拿锁、写 journal、挂起 watcher，它们从「理论问题」变成「每次更新都会走的路径」。

**Files:**
- Modify: `src-tauri/src/commands/watch.rs`（CRLF）
- Modify: `src-tauri/src/engine/svn_lock.rs`
- Modify: `src-tauri/src/engine/svn_journal.rs`
- Test: `src-tauri/tests/svn_watch_suspend.rs`、`src-tauri/tests/svn_lock.rs`、`src-tauri/tests/svn_journal.rs`

**Interfaces:**
- Produces: `app_lib::commands::watch::WatchSuspension::new(path: &str) -> WatchSuspension`（Drop 时 `resume_watch`）；`app_lib::engine::svn_lock::describe_age(started_at: u64, now: u64) -> String`。

- [ ] **Step 1：写失败的测试**

`src-tauri/tests/svn_watch_suspend.rs` 末尾追加：

```rust
#[test]
fn a_suspension_guard_resumes_on_drop_even_when_the_flow_panics() {
    // 2b 独立 review N3：挂起与恢复分两处手写，只要中间一个 `?` 早退或 panic，
    // watcher 就永远停着——之后外部的每一次改动界面都看不见，而且没有任何报错。
    use app_lib::commands::watch::WatchSuspension;
    let path = r"C:\fake\guard-panic";
    let r = std::panic::catch_unwind(|| {
        let _g = WatchSuspension::new(path);
        assert!(is_watch_suspended(path), "构造即挂起");
        panic!("模拟同步流程中途 panic");
    });
    assert!(r.is_err());
    assert!(!is_watch_suspended(path), "panic 展开时 Drop 必须恢复");
}

#[test]
fn a_suspension_guard_resumes_on_normal_return() {
    use app_lib::commands::watch::WatchSuspension;
    let path = r"C:\fake\guard-normal";
    {
        let _g = WatchSuspension::new(path);
        assert!(is_watch_suspended(path));
    }
    assert!(!is_watch_suspended(path));
}
```

`src-tauri/tests/svn_lock.rs` 末尾追加（文件顶部的 `use app_lib::engine::svn_lock::{...}` 里缺 `acquire` / `lock_path` 就补上）：

```rust
#[cfg(windows)]
#[test]
fn release_retries_when_a_reader_briefly_holds_the_file() {
    // 2b 第二轮 review R5：UI 读锁文件（`holder()` 的宽松共享打开，没有
    // FILE_SHARE_DELETE）恰好撞上持锁方 Drop 里的 remove_file，删除失败被吞，
    // 下一次 acquire 打出一条假的「上次同步没有正常收尾」。2c 的锁被持有期间
    // 界面每 2 秒轮询一次，撞上的概率从「理论上」变成「真的会有」。
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let lock = acquire(&root, "opid-R5").unwrap();
    let path = lock_path(&root);

    // 模拟 UI：用与 `holder()` 相同的共享模式开着文件 30ms 再关
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .unwrap();
    let t = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        drop(reader);
    });
    drop(lock);
    t.join().unwrap();
    assert!(!path.exists(), "读者 30ms 后就松手了，释放必须等到它再删掉文件");
}

#[test]
fn busy_error_says_how_long_not_a_raw_epoch() {
    // 2b 第二轮 review R6：原来打印「开始于 1757000000」。
    use app_lib::engine::svn_lock::describe_age;
    assert_eq!(describe_age(1000, 1030), "刚刚开始");
    assert_eq!(describe_age(1000, 1000 + 5 * 60), "已持续 5 分钟");
    assert_eq!(describe_age(1000, 1000 + 2 * 3600 + 60), "已持续 2 小时");
    // 时钟回拨（started_at 在未来）不能 panic，也不能报负数
    assert_eq!(describe_age(2000, 1000), "刚刚开始");
}
```

`src-tauri/tests/svn_journal.rs` 末尾追加：

```rust
#[test]
fn a_stale_tmp_left_by_a_crash_is_swept_on_the_next_write() {
    // 2b 第二轮 review R7：崩溃在 write 中途会在 `.git/` 里留下
    // `mygitclient-sync.json.<pid>.<nanos>.tmp`，没有任何机制清理它。
    // 只有持锁者写 journal（模块头「所有权由锁界定」），所以写入时看到的别的
    // tmp 必然是前人留下的残骸。
    let fx = git_dir_fixture();
    let root = fx.path().to_str().unwrap();
    let stale = fx.path().join(".git").join("mygitclient-sync.json.999.123.tmp");
    std::fs::write(&stale, b"half").unwrap();
    // 不相干的文件不能被误删
    let bystander = fx.path().join(".git").join("mygitclient-sync.json.bak");
    std::fs::write(&bystander, b"keep").unwrap();

    write(root, &minimal("sweep")).unwrap();

    assert!(!stale.exists(), "残留 tmp 必须被清掉");
    assert!(bystander.exists(), "只清 `.tmp` 结尾的同前缀文件");
}
```

- [ ] **Step 2：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_watch_suspend --test svn_lock --test svn_journal
```

期望：编译失败（`WatchSuspension`、`describe_age` 不存在）。先把 `svn_journal` 那条单独跑一次确认它是**断言**失败而不是编译失败：

```powershell
cargo test --test svn_journal a_stale_tmp_left_by_a_crash_is_swept_on_the_next_write
```

期望：`残留 tmp 必须被清掉` 断言失败。

- [ ] **Step 3：实现**

`src-tauri/src/commands/watch.rs`，在 `is_watch_suspended` 之后加：

```rust
/// 挂起监听的 RAII 守卫（spec §5.2，2b 独立 review N3）。
///
/// 构造即挂起，Drop 即恢复。**不要**在同步流程里手写成对的
/// `suspend_watch` / `resume_watch`：中间任何一个 `?` 早退或 panic 都会让
/// watcher 永远停着——之后外部的每一次改动界面都看不见，而且没有任何报错。
///
/// 恢复之后「主动发一次 `repo-changed`」不在这里做：那要 `AppHandle`，而
/// engine 层与测试都拿不到它。由命令层在守卫 drop 之后补发，见
/// `commands/svn_sync.rs`。
pub struct WatchSuspension {
    path: String,
}

impl WatchSuspension {
    pub fn new(path: &str) -> Self {
        suspend_watch(path);
        Self {
            path: path.to_string(),
        }
    }
}

impl Drop for WatchSuspension {
    fn drop(&mut self) {
        resume_watch(&self.path);
    }
}
```

`src-tauri/src/engine/svn_lock.rs`：

1. `Drop for SyncLock` 里把最后一行换成带重试的删除：

```rust
        drop(self.file.take());
        // 到这里句柄已经关了。仍可能删不掉：UI 正以宽松共享模式读它（`holder()`
        // 没带 FILE_SHARE_DELETE），或 `probe` 正独占打开它。两者都只持有几毫秒，
        // 而 2c 的界面在锁被持有期间每 2 秒轮询一次，撞上是真实会发生的事
        // （2b 第二轮 review R5）。删不掉的后果是下一次 acquire 打出一条假的
        // 「上次同步没有正常收尾」，所以值得等一小会儿；但 Drop 里不能无限等。
        for _ in 0..10 {
            match std::fs::remove_file(&self.path) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        }
        // 仍然不 unwrap：Drop 里 panic 会在展开途中直接 abort。
```

（原来那行 `let _ = std::fs::remove_file(&self.path);` 删掉；上面那段注释里「到这里句柄已经关了，删得掉」一句也删掉，由新注释替代。）

2. 在 `busy_error` 之前加：

```rust
/// 「已经持续多久」，给人看的（spec §13：显示持锁进程的 pid 与开始时间）。
///
/// 不打印 epoch 秒（2b 第二轮 review R6：原来是「开始于 1757000000」）。
/// 也不打印本地时刻：后端不知道用户的时区习惯，而「持续了多久」恰好是判断
/// 「这是不是一次卡死的同步」唯一需要的信息。
pub fn describe_age(started_at: u64, now: u64) -> String {
    let secs = now.saturating_sub(started_at);
    if secs < 60 {
        "刚刚开始".to_string()
    } else if secs < 3600 {
        format!("已持续 {} 分钟", secs / 60)
    } else {
        format!("已持续 {} 小时", secs / 3600)
    }
}
```

3. `busy_error` 里把 `"持有者：进程 {}，开始于 {}（opid {}）。"` 改成：

```rust
        Some(h) => format!(
            "持有者：进程 {}，{}（opid {}）。",
            h.pid,
            describe_age(h.started_at, now_secs()),
            h.opid
        ),
```

`src-tauri/src/engine/svn_journal.rs` 的 `write`：

1. `create_dir_all(dir)?;` 之后加一行 `sweep_stale_tmp(dir);`。
2. 把 `{ let mut f = File::create(&tmp)?; f.write_all(&body)?; f.sync_all()?; }` 这一段换成：

```rust
    // 从创建 tmp 起到 rename 为止，任何一步失败都要把 tmp 删掉（2b 第二轮
    // review R7：原来只有 rename 失败那条路径会清）。收进一个闭包，是为了让
    // 「所有 `?` 都在清理范围里」由结构保证，而不是靠每个调用点记得。
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        // rename 是原子的，但「新内容已经落盘」不是它保证的。
        // 少了这一步，断电后可能 rename 生效了而内容还是空的。
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
```

3. 文件末尾加：

```rust
/// 删掉前人留下的 `mygitclient-sync.json.*.tmp`（2b 第二轮 review R7）。
///
/// 只有持锁者写 journal（模块头「所有权由锁界定」），所以写入这一刻目录里的
/// 其它 tmp 必然是崩溃留下的残骸。删不掉就算了：它只占几百字节，不影响正确性。
fn sweep_stale_tmp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{JOURNAL_FILE}.");
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}
```

- [ ] **Step 4：跑测试，确认通过**

```powershell
cargo test --test svn_watch_suspend --test svn_lock --test svn_journal
```

期望：全部 PASS。

- [ ] **Step 5：变异验证（记进决策日志）**

逐个做、做完还原：

| 变异 | 期望只变红 |
|---|---|
| `WatchSuspension::drop` 里注释掉 `resume_watch` | 两条 guard 测试 |
| 锁 `Drop` 的重试次数改成 `0..1` | `release_retries_when_a_reader_briefly_holds_the_file` |
| `sweep_stale_tmp(dir);` 注释掉 | `a_stale_tmp_left_by_a_crash_is_swept_on_the_next_write` |

- [ ] **Step 6：全量 Rust 检查并提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri/src/commands/watch.rs src-tauri/src/engine/svn_lock.rs src-tauri/src/engine/svn_journal.rs src-tauri/tests/svn_watch_suspend.rs src-tauri/tests/svn_lock.rs src-tauri/tests/svn_journal.rs docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "fix(svn): 收掉 2b 遗留的 watcher 守卫、锁释放重试、journal 残留 tmp 与持锁时长"
```

---

## Task 2：前置检查（spec §7.2 第 1–6 条）

**纯读。** 任何一条不过就返回 `Err`，仓库一个字节都不动。

**Files:**
- Modify: `src-tauri/src/engine/svn_xml.rs`（加 `parse_update_status`）
- Create: `src-tauri/src/engine/svn_sync_check.rs`
- Modify: `src-tauri/src/engine/mod.rs`（`pub mod svn_sync_check;`，交给 `cargo fmt` 排序）
- Modify: `src-tauri/tests/common/svn_fixtures.rs`
- Test: `src-tauri/tests/svn_xml.rs`、Create `src-tauri/tests/svn_sync_check.rs`

**Interfaces:**
- Consumes: `svn_sync::check_sync`、`svn_externals::detect_pinned`、`svn_include_check::{is_w_file, normalize_roots, verify_include_set}`、`svn_baseline::{git_dir, TempIndex}`、`svn_lock::assert_not_syncing`、`svn_journal::read`。
- Produces:
  - `svn_xml::UpdateStatusEntry { path: String, item: String, props: String, tree_conflicted: bool, repos_item: Option<String> }`
  - `svn_xml::UpdateStatus { entries: Vec<UpdateStatusEntry>, against: Option<u64> }`
  - `svn_xml::parse_update_status(xml: &str) -> Result<UpdateStatus>`
  - `svn_sync_check::SyncInPreview { orig_branch: String, orig_head: String, baseline_oid: String, baseline_rev: u64, target_rev: u64, outside_changes: Vec<String> }`（`Serialize`，camelCase）
  - `svn_sync_check::preview_sync_in(repo_root: &str) -> Result<SyncInPreview>`：先查锁与 journal，再做全部检查
  - `pub(crate) svn_sync_check::check_sync_in(repo_root: &str) -> Result<SyncInPreview>`：不查锁与 journal（持锁者自己在锁内复核时用）
  - `svn_sync_check::assert_linear(repo_root: &str, target: &str, tip: &str) -> Result<()>`
  - `pub(crate) svn_sync_check::current_branch(repo_root: &str) -> Result<Option<String>>`
  - `pub(crate) svn_sync_check::rev_parse(repo_root: &str, rev: &str) -> Result<String>`
  - fixture：`svn_fixtures::hybrid_with_colleague() -> SvnFixture`、`SvnFixture::wc2()`、`SvnFixture::colleague_commits(rel, content)`

- [ ] **Step 1：fixture**

`src-tauri/tests/common/svn_fixtures.rs` 末尾追加：

```rust
/// Phase 2c 用：已初始化的混合仓库 wc1（白名单 `Code/`、扩展名 `.cs`，停在
/// `work` 分支）+ 扮演同事的 wc2。
///
/// 仓库 r1：`Code/A.cs`、`Code/B.cs`，以及白名单外的美术文件 `Art/x.psd`
/// （§7.2 第 5 条要它：git 不管、update 会碰到的文件）。
pub fn hybrid_with_colleague() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(
        &root,
        "svn",
        &["checkout", &url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");
    std::fs::create_dir_all(wc.join("Code")).unwrap();
    std::fs::create_dir_all(wc.join("Art")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a1\n").unwrap();
    std::fs::write(wc.join("Code/B.cs"), "b1\n").unwrap();
    std::fs::write(wc.join("Art/x.psd"), "psd1\n").unwrap();
    run(&wc, "svn", &["add", "Code", "Art", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    run(
        &root,
        "svn",
        &["checkout", &url, "wc2", "--non-interactive"],
    );
    app_lib::engine::svn_init::init_hybrid(&app_lib::engine::svn_init::InitRequest {
        repo_root: wc.to_string_lossy().to_string(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .expect("fixture：初始化混合仓库失败");
    SvnFixture { dir }
}

impl SvnFixture {
    /// 同事的工作副本。只有 [`hybrid_with_colleague`] 建它。
    pub fn wc2(&self) -> PathBuf {
        self.dir.path().join("wc2")
    }

    /// 同事在 wc2 里改（或新建）一个文件并提交——「服务器上有新提交」。
    pub fn colleague_commits(&self, rel: &str, content: &str) {
        let wc2 = self.wc2();
        run(&wc2, "svn", &["update", "--non-interactive"]);
        let p = wc2.join(rel);
        let is_new = !p.exists();
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
        if is_new {
            run(&wc2, "svn", &["add", "--parents", rel, "--non-interactive"]);
        }
        run(
            &wc2,
            "svn",
            &["commit", "-m", &format!("colleague {rel}"), "--non-interactive"],
        );
    }
}
```

- [ ] **Step 2：写失败的测试——XML 解析**

`src-tauri/tests/svn_xml.rs` 末尾追加（XML 原文取自 2026-09-25 实测输出）：

```rust
#[test]
fn update_status_reads_repos_status_and_against() {
    use app_lib::engine::svn_xml::parse_update_status;
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target
   path=".">
<entry
   path="Art\x.psd">
<wc-status
   item="modified"
   revision="1"
   props="none">
</wc-status>
<repos-status
   item="modified"
   props="none">
</repos-status>
</entry>
<entry
   path="Art\y.psd">
<wc-status
   item="normal"
   revision="1"
   props="none">
</wc-status>
<repos-status
   item="modified"
   props="none">
</repos-status>
</entry>
<against
   revision="2"/>
</target>
</status>"#;
    let st = parse_update_status(xml).unwrap();
    assert_eq!(st.against, Some(2));
    assert_eq!(st.entries.len(), 2);
    assert_eq!(st.entries[0].path, "Art/x.psd", "路径归一成正斜杠");
    assert_eq!(st.entries[0].item, "modified");
    assert_eq!(st.entries[0].repos_item.as_deref(), Some("modified"));
    assert!(!st.entries[0].tree_conflicted);
}

#[test]
fn update_status_sees_tree_conflicts_whose_item_is_not_conflicted() {
    // 2026-09-25 实测：树冲突节点是 `item="added" tree-conflicted="true"`。
    // 只看 item == "conflicted" 会把它当成一个普通的本地新增放过去。
    use app_lib::engine::svn_xml::parse_update_status;
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target
   path=".">
<entry
   path="Code\Gone">
<wc-status
   item="added"
   props="none"
   copied="true"
   tree-conflicted="true">
</wc-status>
</entry>
</target>
</status>"#;
    let st = parse_update_status(xml).unwrap();
    assert!(st.entries[0].tree_conflicted);
    assert_eq!(st.entries[0].item, "added");
    assert_eq!(st.entries[0].repos_item, None, "不带 -u 时没有 repos-status");
    assert_eq!(st.against, None);
}
```

- [ ] **Step 3：写失败的测试——前置检查**

新建 `src-tauri/tests/svn_sync_check.rs`：

```rust
//! `sync_in` 前置检查（spec §7.2）。全部是**纯读**：每条拒绝都要断言仓库没被动过。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_sync_check::{assert_linear, preview_sync_in};
use svn_fixtures::{hybrid_with_colleague, svn_cmd};

fn git(fx: &svn_fixtures::SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn head(fx: &svn_fixtures::SvnFixture) -> String {
    git(fx, &["rev-parse", "HEAD"])
}

fn code_of(r: Result<app_lib::engine::svn_sync_check::SyncInPreview, app_lib::error::GitError>) -> String {
    r.expect_err("应当被拒绝").code
}

#[test]
fn a_clean_hybrid_previews_ok() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.orig_branch, "work");
    assert_eq!(p.orig_head, head(&fx));
    assert_eq!(p.baseline_rev, 1);
    assert_eq!(p.target_rev, 2, "against revision 就是这次要更新到的 revision");
    assert!(p.outside_changes.is_empty());
}

#[test]
fn detached_head_is_refused() {
    // 必测 #31
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "--detach"]);
    let before = head(&fx);
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_detached_head");
    assert_eq!(head(&fx), before);
}

#[test]
fn being_on_the_baseline_branch_is_refused() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "svn/base"]);
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_on_baseline");
}

#[test]
fn a_merge_in_the_replay_range_is_refused_before_anything() {
    // 必测 #63：merge 独有的文件在普通 rebase 里会无冲突地消失（§7.3.2）
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    git(&fx, &["checkout", "-b", "side"]);
    std::fs::write(wc.join("Code/Side.cs"), "side\n").unwrap();
    git(&fx, &["add", "Code/Side.cs"]);
    git(&fx, &["commit", "-m", "side"]);
    git(&fx, &["checkout", "work"]);
    std::fs::write(wc.join("Code/A.cs"), "a-work\n").unwrap();
    git(&fx, &["commit", "-am", "work"]);
    git(&fx, &["merge", "--no-ff", "-m", "merge side", "side"]);
    let before = head(&fx);
    let err = preview_sync_in(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_merge_in_range");
    assert!(err.message.contains(&before[..7]) || err.message.contains(&before), "要列出 merge OID");
    assert_eq!(head(&fx), before);
    assert!(wc.join("Code/Side.cs").exists(), "独有文件仍在");
}

#[test]
fn a_failing_merge_query_is_not_treated_as_empty() {
    // §7.3.2：「命令失败 → 检查未完成，停止；不能把失败当作空集合」
    let fx = hybrid_with_colleague();
    let bogus = "0123456789abcdef0123456789abcdef01234567";
    let err = assert_linear(&fx.wc_str(), bogus, &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_in_merge_check_failed");
}

#[test]
fn outside_changes_that_update_will_touch_are_listed() {
    // §7.2 第 5 条的两个方向都要测：只列「本地改过且服务器也改过」的
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap();
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Art/x.psd".to_string()]);
}

#[test]
fn outside_changes_the_update_will_not_touch_are_not_listed() {
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap();
    fx.colleague_commits("Code/A.cs", "a2\n");
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert!(p.outside_changes.is_empty(), "{:?}", p.outside_changes);
}

#[test]
fn an_svn_conflict_anywhere_is_refused() {
    // A6。只更新 Art，让 Code 仍停在 r1——否则 nodeRevs 先报失同步，测不到这一条
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap();
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    svn_cmd(&fx.wc(), &["update", "--accept", "postpone", "--non-interactive", "Art"]);
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_svn_conflicted");
}

#[test]
fn an_out_of_sync_baseline_is_refused() {
    // §7.2 第 2 条：外部 TortoiseSVN update 过，基线已经失同步。
    // ⚠️ 只活到 Task 7A：A16 之后「整体前进」改由执行时接管，7A 会删掉这条、换成四条更细的

    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_out_of_sync");
}

#[test]
fn a_gitignore_that_hides_w_files_is_refused_before_anything() {
    // §7.2 第 6 条后半：当前磁盘的纳入集合预检。不在这里拦，S7 才炸——
    // 那时 svn update 已经做完，用户卡在 svn/base 上。
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/.gitignore"), "*.cs\n").unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "include_set_mismatch");
}

#[test]
fn an_unreachable_server_is_refused_before_anything() {
    let fx = hybrid_with_colleague();
    let repo = fx.repo();
    let gone = fx.path().join("repo-gone");
    std::fs::rename(&repo, &gone).unwrap();
    let r = preview_sync_in(&fx.wc_str());
    std::fs::rename(&gone, &repo).unwrap();
    assert_eq!(code_of(r), "sync_in_server_unreachable");
}

#[test]
fn a_rebase_in_progress_is_refused() {
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join(".git/rebase-merge")).unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_git_busy");
}

#[test]
fn an_unfinished_journal_is_refused() {
    use app_lib::engine::svn_journal::{write, Flow, Journal, Phase};
    let fx = hybrid_with_colleague();
    write(
        &fx.wc_str(),
        &Journal {
            opid: "left-behind".into(),
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_unfinished");
}

#[test]
fn a_held_lock_is_refused() {
    let fx = hybrid_with_colleague();
    let _held = app_lib::engine::svn_lock::acquire(&fx.wc_str(), "other").unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_progress");
}

#[test]
fn a_pinned_whitelist_external_is_refused() {
    // A1：两步 update 推迟，pinned 模式下直接拒绝
    let fx = svn_fixtures::svn_repo_pinned_external_updated_to_head();
    app_lib::engine::svn_init::init_hybrid(&app_lib::engine::svn_init::InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_pinned_unsupported");
}
```

> ⚠️ 执行者：`a_pinned_whitelist_external_is_refused` 依赖 `init_hybrid` 能在那个 fixture 上成功（它是 1b 为 `PinnedReset` 造的，初始化发生在「Code 被单独更到 HEAD」之后）。若 `init_hybrid` 在它上面失败，**不要改 fixture 去迁就**：先查失败原因，把事实记进决策日志「计划没写的坑」，再决定换哪个 fixture。

- [ ] **Step 4：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_xml update_status
cargo test --test svn_sync_check
```

期望：编译失败（`parse_update_status`、`svn_sync_check` 不存在）。

- [ ] **Step 5：实现 `parse_update_status`**

`src-tauri/src/engine/svn_xml.rs`，放在 `parse_status_entries` 之后：

```rust
/// `svn status [-u] --xml` 里 `sync_in` 关心的字段（spec §7.2 第 4、5 条，§7.3 S5）。
///
/// 与 [`StatusEntry`] 分开：那个结构被初始化、重建、体检、失同步判定共用，
/// 给它加字段会牵动所有构造点；这里要的三样东西（`repos-status`、树冲突、
/// `against`）只有更新流程用。
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStatusEntry {
    /// 相对 target 的路径，正斜杠。
    pub path: String,
    pub item: String,
    pub props: String,
    /// `<wc-status tree-conflicted="true">`。
    ///
    /// **不能只看 `item == "conflicted"`**：2026-09-25 实测，树冲突节点是
    /// `item="added" tree-conflicted="true"`，只看 item 会把它当成一个普通的
    /// 本地新增放过去。
    pub tree_conflicted: bool,
    /// `<repos-status item>`：带 `-u` 时才有。`modified` / `added` / `deleted`
    /// 表示这次 update 会碰到它，`none` 表示不会。
    pub repos_item: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStatus {
    pub entries: Vec<UpdateStatusEntry>,
    /// `<against revision>`：服务器此刻的 HEAD，也就是这次会更新到的 revision。
    /// 带 `-u` 时才有；实测**没有任何可更新内容时也会给出**。
    pub against: Option<u64>,
}

pub fn parse_update_status(xml: &str) -> Result<UpdateStatus> {
    let doc = parse_doc(xml)?;
    let mut entries = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let path = norm_sep(&attr(&entry, "path")?);
        // 与 parse_status_entries 同一条理由：缺 <wc-status> 一律报错，不跳过
        let st = entry
            .children()
            .find(|n| n.has_tag_name("wc-status"))
            .ok_or_else(|| {
                GitError::new(
                    "svn_xml",
                    format!("svn status 的 entry `{path}` 底下没有 <wc-status>"),
                )
            })?;
        let repos_item = match entry.children().find(|n| n.has_tag_name("repos-status")) {
            Some(r) => Some(attr(&r, "item")?),
            None => None,
        };
        entries.push(UpdateStatusEntry {
            path,
            item: attr(&st, "item")?,
            props: attr(&st, "props")?,
            tree_conflicted: st.attribute("tree-conflicted") == Some("true"),
            repos_item,
        });
    }
    // 项目根的 <target> 在最前；external 各有自己的 <target>/<against>，
    // 它们与根同属一个仓库时数值相同（P4 保证白名单不跨仓库），取第一个即可。
    let against = match doc.descendants().find(|n| n.has_tag_name("against")) {
        Some(a) => {
            let raw = attr(&a, "revision")?;
            Some(raw.parse::<u64>().map_err(|_| {
                GitError::new("svn_xml", format!("<against revision> 不是数字：{raw}"))
            })?)
        }
        None => None,
    };
    Ok(UpdateStatus { entries, against })
}
```

- [ ] **Step 6：实现 `svn_sync_check.rs`**

新建 `src-tauri/src/engine/svn_sync_check.rs`：

```rust
//! `sync_in` 的前置检查（spec §7.2 第 1–6 条）。**全部是纯读。**
//!
//! 第 7 条（拿锁、锁内复核）属于执行，见 `svn_sync_in::run_sync_in`：它在锁里
//! 再调一次 [`check_sync_in`]，拿结果与预览时给用户看过的比。
//!
//! # 顺序
//!
//! 本地的、便宜的检查在前，走网络的 `svn status -u` 放在最后：断网时用户先看到
//! 的应当是「你在 detached HEAD 上」这种能自己修的问题，而不是一条网络错误。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_baseline::{git_dir, TempIndex};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{read_config, BASELINE_BRANCH};
use crate::engine::svn_externals::detect_pinned;
use crate::engine::svn_include_check::{is_w_file, normalize_roots, verify_include_set};
use crate::engine::svn_sync::{check_sync, SyncVerdict};
use crate::engine::svn_xml::parse_update_status;
use crate::engine::{svn_journal, svn_lock};
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};
use std::collections::BTreeMap;

/// 预览：给确认框看的全部事实。执行时原样带回来（`SyncInExpect`）做锁内复核。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInPreview {
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    pub baseline_rev: u64,
    /// 这次会更新到的 revision（`svn status -u` 的 `<against>`）。
    pub target_rev: u64,
    /// §7.2 第 5 条：git 不管、本地改过、这次 update 又会碰到的文件。
    /// **非空时必须由用户确认**，确认只授权 update，不授权任何 revert。
    pub outside_changes: Vec<String>,
}

/// 给界面用的入口：锁被拿着、或有未收尾的 journal 时直接拒绝。
pub fn preview_sync_in(repo_root: &str) -> Result<SyncInPreview> {
    svn_lock::assert_not_syncing(repo_root)?;
    if svn_journal::read(repo_root)?.is_some() {
        return Err(unfinished_error());
    }
    check_sync_in(repo_root)
}

/// 预览与执行两处都要说这句话，收在一起免得两份文案说出不同的事。
pub(crate) fn unfinished_error() -> GitError {
    GitError::new(
        "sync_in_unfinished",
        "上一次同步还没有收尾。请先在顶部横幅上选择「继续」或「放弃」，\
         再发起新的更新——两次同步的记录不能叠在一起。",
    )
}

/// 第 1–6 条。**不查锁**：持锁者在锁内复核时调它，查锁会把自己挡住。
pub(crate) fn check_sync_in(repo_root: &str) -> Result<SyncInPreview> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库。",
        ));
    }
    let cfg = read_config(repo_root)?;

    check_git_idle(repo_root)?; // 第 1 条
    let orig_branch = attached_branch(repo_root)?; // 第 3 条
    let orig_head = rev_parse(repo_root, "HEAD")?;
    let baseline_oid = rev_parse(repo_root, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    assert_linear(repo_root, &baseline_oid, &orig_head)?; // 第 6 条前半

    if !detect_pinned(repo_root, &cfg.whitelist)?.is_empty() {
        return Err(GitError::new(
            "sync_in_pinned_unsupported",
            "白名单所在的 external 被 `-r` 钉住了（pinned 模式）。这种形态需要改用\
             「项目根 --ignore-externals + 逐个 external 显式 update」的两步更新，\
             本版本还没有实现，所以不在这里冒险。请先用 TortoiseSVN 更新，\
             再用「重建基线」把基线跟上。",
        ));
    }

    let status = check_sync(repo_root)?; // 第 2 条（完整 nodeRevs）
    if status.verdict != SyncVerdict::InSync {
        return Err(GitError::new(
            "sync_in_out_of_sync",
            format!(
                "基线已经失同步（白名单 revision {:?}，基线记的是 r{}）。\
                 在失同步的基线上更新，会把外部的改动当成服务器的新内容录进去。\
                 请先处理顶部的失同步横幅。",
                status.node_revs, status.baseline_rev
            ),
        ));
    }

    // 第 6 条后半：当前磁盘的纳入集合预检（§6.3.1）。放在 update 之前，是为了
    // 让「工作树里一个 `.gitignore` 挡住了 W-file」这种问题在动手前暴露——
    // 否则要到 S7 才炸，那时 svn update 已经做完，用户卡在 svn/base 上。
    precheck_include_set(repo_root, &cfg.whitelist, &cfg.extensions)?;

    // 第 4、5 条：走网络，放最后
    let xml = run_svn(repo_root, &["status", "-u", "--xml"]).map_err(|e| {
        GitError::new(
            "sync_in_server_unreachable",
            format!("问服务器「这次会更新哪些文件」失败了，没有做任何修改。svn 原话：{}", e.message),
        )
    })?;
    let st = parse_update_status(&xml)?;
    let roots = normalize_roots(&cfg.whitelist);
    let mut conflicted = Vec::new();
    let mut outside = Vec::new();
    for e in &st.entries {
        if e.item == "conflicted" || e.tree_conflicted {
            conflicted.push(e.path.clone());
            continue;
        }
        if is_w_file(&e.path, &roots, &cfg.extensions) {
            continue; // git 管着，S1 会压进 stash
        }
        let local = matches!(e.item.as_str(), "modified" | "missing" | "replaced")
            || e.props == "modified";
        let incoming = e.repos_item.as_deref().is_some_and(|r| r != "none");
        if local && incoming {
            outside.push(e.path.clone());
        }
    }
    if !conflicted.is_empty() {
        return Err(GitError::new(
            "sync_in_svn_conflicted",
            format!(
                "工作副本里有 SVN 冲突还没处理，在它上面再 update 只会让情况更乱：\n{}\n\
                 请先用 TortoiseSVN 解决（Resolve）后再更新。",
                list_paths(&conflicted)
            ),
        ));
    }
    let target_rev = st.against.ok_or_else(|| {
        GitError::new("svn_xml", "svn status -u 没有给出 <against revision>")
    })?;

    Ok(SyncInPreview {
        orig_branch,
        orig_head,
        baseline_oid,
        baseline_rev: status.baseline_rev,
        target_rev,
        outside_changes: outside,
    })
}

/// §7.3.2 的 merge 门槛：`target..tip` 必须是线性的。
///
/// **查询失败不能当成空集合**——那等于把「没检查」报成「检查通过」。
pub fn assert_linear(repo_root: &str, target: &str, tip: &str) -> Result<()> {
    // 空 OID 会让区间退化成 `..tip`（= `HEAD..tip`），查的就不是我们以为的那段了
    for oid in [target, tip] {
        if oid.len() != 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GitError::new(
                "sync_in_merge_check_failed",
                format!("merge 门槛拿到的不是一个完整 OID：`{oid}`"),
            ));
        }
    }
    let range = format!("{target}..{tip}");
    let (code, out) = run_git_code(repo_root, &["rev-list", "--merges", &range])?;
    if code != 0 {
        return Err(GitError::new(
            "sync_in_merge_check_failed",
            format!(
                "检查待重放区间 {range} 里有没有 merge 失败了（rev-list 退出 {code}）。\
                 查不成不等于没有 merge，所以这里停下。"
            ),
        ));
    }
    let merges: Vec<&str> = out.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if !merges.is_empty() {
        return Err(GitError::new(
            "sync_in_merge_in_range",
            format!(
                "你的分支上、基线之后有 merge commit：\n{}\n\
                 更新要把这些提交重放到新基线上，而普通 rebase 会跳过 merge——\
                 只存在于 merge 结果里的改动可能无声地消失。本版本不自动重放 merge 历史。\
                 请保留原分支，自己把历史整理成线性的（比如在新分支上逐个 cherry-pick），\
                 再回来更新。",
                merges.join("\n")
            ),
        ));
    }
    Ok(())
}

/// 第 1 条：git 不在任何未完成的操作里，索引也没有未合并条目。
fn check_git_idle(repo_root: &str) -> Result<()> {
    let dir = git_dir(repo_root)?;
    let busy = [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ];
    for (marker, what) in busy {
        if dir.join(marker).exists() {
            return Err(GitError::new(
                "sync_in_git_busy",
                format!("git 正处在一次没做完的 {what} 里。请先完成或中止它，再更新。"),
            ));
        }
    }
    if !unmerged_paths(repo_root)?.is_empty() {
        return Err(GitError::new(
            "sync_in_git_busy",
            "暂存区里有未解决的冲突。请先解决（或 `git restore --staged`）再更新。",
        ));
    }
    Ok(())
}

/// 第 3 条：HEAD 必须附着在分支上，且不是基线分支。返回分支名。
fn attached_branch(repo_root: &str) -> Result<String> {
    let Some(branch) = current_branch(repo_root)? else {
        let at = rev_parse(repo_root, "HEAD").unwrap_or_default();
        return Err(GitError::new(
            "sync_in_detached_head",
            format!(
                "HEAD 没有附着在分支上（停在 {}）。更新要把一个分支重放到新基线上，\
                 detached HEAD 没有分支可搬。请 `git switch -c <新分支名>` 或切回原来的分支。",
                &at[..at.len().min(10)]
            ),
        ));
    };
    if branch == BASELINE_BRANCH {
        return Err(GitError::new(
            "sync_in_on_baseline",
            format!(
                "当前在基线分支 `{BASELINE_BRANCH}` 上。基线由本应用维护，请先切回你的工作分支再更新。"
            ),
        ));
    }
    Ok(branch)
}

/// 当前分支名；detached 时 `None`。
pub(crate) fn current_branch(repo_root: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let name = out.trim().to_string();
    Ok(if code == 0 && !name.is_empty() {
        Some(name)
    } else {
        None
    })
}

/// `git rev-parse --verify <rev>` 的完整 OID。
pub(crate) fn rev_parse(repo_root: &str, rev: &str) -> Result<String> {
    Ok(run_git(repo_root, &["rev-parse", "--verify", rev])?
        .trim()
        .to_string())
}

/// 索引里未合并的路径（冲突态）。
pub(crate) fn unmerged_paths(repo_root: &str) -> Result<Vec<String>> {
    let out = run_git(repo_root, &["diff", "--name-only", "--diff-filter=U", "-z"])?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// 当前磁盘上的纳入集合预检：期望集合（遍历磁盘）与 `git add -A` 实际收进来的相等。
fn precheck_include_set(repo_root: &str, whitelist: &[String], extensions: &[String]) -> Result<()> {
    let idx = TempIndex::empty(repo_root)?;
    idx.add_all(repo_root, whitelist)?;
    // 空的 fixes：这里比的就是「磁盘现在的样子」，不是服务器的
    verify_include_set(repo_root, &idx, whitelist, extensions, &BTreeMap::new())
}

/// 路径清单给人看：最多 20 条，多了说还有几条。`svn_sync_in` 也用它。
pub(crate) fn list_paths(paths: &[String]) -> String {
    let mut s: Vec<String> = paths.iter().take(20).map(|p| format!("  - {p}")).collect();
    if paths.len() > 20 {
        s.push(format!("  …还有 {} 条", paths.len() - 20));
    }
    s.join("\n")
}
```

`src-tauri/src/engine/mod.rs` 加 `pub mod svn_sync_check;`。

- [ ] **Step 7：跑测试，确认通过**

```powershell
cargo test --test svn_xml update_status
cargo test --test svn_sync_check
```

期望：全部 PASS。某条失败时，先在 fixture 上用真实 `svn` / `git` 命令复现它的形状（`AGENTS.md` §6.2），不要改断言迁就实现。

- [ ] **Step 8：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `tree_conflicted` 恒为 `false` | `update_status_sees_tree_conflicts…` |
| `assert_linear` 里 `code != 0` 分支改成 `return Ok(())` | `a_failing_merge_query_is_not_treated_as_empty` |
| 去掉 `&& incoming` | `outside_changes_the_update_will_not_touch_are_not_listed` |
| 注释掉 `precheck_include_set(...)` | `a_gitignore_that_hides_w_files…` |

- [ ] **Step 9：全量检查、更新决策日志、提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): sync_in 前置检查（§7.2 第 1–6 条，纯读）"
```

---

## Task 3：WIP 压栈与取回（S1 / S10）

**Files:**
- Modify: `src-tauri/src/engine/svn_journal.rs`（`Phase::ApplyingStash`、`stash_applied`）
- Create: `src-tauri/src/engine/svn_sync_stash.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Modify: `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（§7.6 sync_in phase 表加一行；§7.3 S10 加注）
- Test: `src-tauri/tests/svn_journal.rs`、Create `src-tauri/tests/svn_sync_stash.rs`

**Interfaces:**
- Consumes: `svn_journal::{write, Journal, Phase}`、`svn_include_check::normalize_roots`、`svn_sync_check::unmerged_paths`。
- Produces:
  - `Phase::ApplyingStash`（线序名 `"applying-stash"`）；`Journal.stash_applied: bool`（线序名 `stashApplied`，`false` 时不写出）
  - `svn_sync_stash::MSG_PREFIX: &str = "mygitclient-sync:"`
  - `svn_sync_stash::pin_ref(opid: &str) -> String`
  - `svn_sync_stash::read_stash_ref(repo: &str) -> Result<Option<String>>`
  - `svn_sync_stash::find_by_opid(repo: &str, opid: &str) -> Result<Option<String>>`
  - `svn_sync_stash::stash_wip(repo: &str, j: &mut Journal, whitelist: &[String]) -> Result<()>`：结束时 `j.phase == Stashed`，`j.stash_oid` 为 `Some(oid)` 或 `None`（A7）
  - `svn_sync_stash::settle_stashing(repo: &str, j: &mut Journal) -> Result<()>`：§7.3.1 判定树，崩溃恢复也走它
  - `svn_sync_stash::RestoreOutcome { Applied, Conflicted { files: Vec<String> }, Blocked { collisions: Vec<String> } }`
  - `svn_sync_stash::restore_wip(repo: &str, oid: &str) -> Result<RestoreOutcome>`
  - `svn_sync_stash::drop_wip(repo: &str, opid: &str, oid: &str) -> Result<()>`
  - `svn_sync_stash::unpin(repo: &str, opid: &str) -> Result<()>`

- [ ] **Step 1：写失败的测试——journal 新字段**

`src-tauri/tests/svn_journal.rs`：`every_phase_uses_the_exact_string_from_the_spec` 的 `cases` 里 `(Phase::Rebase, "rebase"),` 之后加一行 `(Phase::ApplyingStash, "applying-stash"),`；文件末尾追加：

```rust
#[test]
fn stash_applied_round_trips_and_is_omitted_when_false() {
    let fx = git_dir_fixture();
    let root = fx.path().to_str().unwrap();
    let mut j = minimal("sa");
    write(root, &j).unwrap();
    let raw = std::fs::read_to_string(journal_path(root)).unwrap();
    assert!(!raw.contains("stashApplied"), "false 时不写出，sync_in 早期的 journal 保持干净");
    j.stash_applied = true;
    write(root, &j).unwrap();
    assert!(read(root).unwrap().unwrap().stash_applied);
}
```

- [ ] **Step 2：写失败的测试——压栈与取回**

新建 `src-tauri/tests/svn_sync_stash.rs`：

```rust
//! S1 / S10：WIP 按 opid 压栈、按 OID 取回（spec §7.3.1、§7.6.1）。
//! 都在临时 fixture 上跑，git 行为以 2026-09-25 实测为准（计划 A3 / A7）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal::{read, Flow, Journal, Phase};
use app_lib::engine::svn_sync_stash::{
    drop_wip, find_by_opid, pin_ref, read_stash_ref, restore_wip, settle_stashing, stash_wip,
    RestoreOutcome,
};
use svn_fixtures::hybrid_with_colleague;

fn git(root: &str, args: &[&str]) -> String {
    run_git(root, args).unwrap().trim().to_string()
}

fn journal(opid: &str) -> Journal {
    Journal {
        opid: opid.into(),
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: 1,
        ..Default::default()
    }
}

fn wl() -> Vec<String> {
    vec!["Code/".to_string()]
}

#[test]
fn nothing_to_stash_leaves_stashed_without_an_oid() {
    // A7：无可压内容时 `git stash push` 退出 0、refs/stash 不变
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap(); // 白名单外的改动不算
    let mut j = journal("none");
    stash_wip(&root, &mut j, &wl()).unwrap();
    assert_eq!(j.phase, Phase::Stashed);
    assert_eq!(j.stash_oid, None);
    assert_eq!(read_stash_ref(&root).unwrap(), None);
    assert_eq!(read(&root).unwrap().unwrap().phase, Phase::Stashed, "phase 落盘");
}

#[test]
fn wip_is_stashed_by_opid_and_pinned() {
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::write(fx.wc().join("Code/New.cs"), "untracked\n").unwrap();
    let mut j = journal("op1");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().expect("有 WIP 就必须有 OID");
    assert_eq!(read_stash_ref(&root).unwrap().as_deref(), Some(oid.as_str()));
    assert_eq!(git(&root, &["rev-parse", &pin_ref("op1")]), oid, "§7.6.1：push 之后立刻钉住");
    assert_eq!(git(&root, &["status", "--porcelain", "--", "Code"]), "", "白名单内被压干净");
    assert!(!fx.wc().join("Code/New.cs").exists(), "-u：未跟踪的 W-file 也进 stash");
}

#[test]
fn a_failed_push_that_changed_nothing_leaves_no_journal() {
    // push 失败时要先判断它落没落地：没落地就什么都没发生，journal 必须删掉，
    // 否则横幅会对一次根本没开始的同步喊「上次没收尾」。
    // 用「白名单指向不存在的路径」让 git stash push 报 pathspec 错误。
    // （「journal 写在 push 之前」这一条这里观察不到——它由 Task 8 的
    //  `continue_from_stashing_with_a_landed_push` 钉住。）
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-early");
    let r = stash_wip(&root, &mut j, &["Nope/".to_string()]);
    assert!(r.is_err());
    // push 没落地、refs/stash 没变 ⇒ 什么都没发生，journal 已被删掉
    assert!(read(&root).unwrap().is_none(), "失败且没有任何改动时不留 journal");
    assert_eq!(git(&root, &["status", "--porcelain", "--", "Code"]), "M Code/A.cs");
}

#[test]
fn an_empty_whitelist_is_refused_not_widened() {
    // spec §12.2：空 pathspec 会退化成「全部」
    let fx = hybrid_with_colleague();
    let mut j = journal("op-empty");
    let err = stash_wip(&fx.wc_str(), &mut j, &[]).unwrap_err();
    assert_eq!(err.code, "sync_stash_empty_whitelist");
}

#[test]
fn settle_finds_our_stash_even_when_another_was_pushed_on_top() {
    // 必测 #17 / #40：外部工具在窗口内又压了一个 stash
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-drift");
    j.stash_ref_before = read_stash_ref(&root).unwrap();
    j.phase = Phase::Stashing;
    git(&root, &["stash", "push", "-u", "-m", "mygitclient-sync:op-drift", "--", "Code"]);
    let ours = read_stash_ref(&root).unwrap().unwrap();
    std::fs::write(fx.wc().join("Code/B.cs"), "someone-else\n").unwrap();
    git(&root, &["stash", "push", "-m", "someone else WIP"]);
    assert_ne!(read_stash_ref(&root).unwrap().unwrap(), ours, "前提：栈顶已经不是我们的");

    settle_stashing(&root, &mut j).unwrap();
    assert_eq!(j.stash_oid.as_deref(), Some(ours.as_str()));
    assert_eq!(find_by_opid(&root, "op-drift").unwrap().as_deref(), Some(ours.as_str()));
}

#[test]
fn settle_stops_when_the_stash_vanished() {
    // §7.3.1：refs/stash 变了却找不到我们那条 ⇒ 被外部 clear / drop 过，必须停下
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let mut j = journal("op-lost");
    j.stash_ref_before = None;
    j.phase = Phase::Stashing;
    std::fs::write(fx.wc().join("Code/A.cs"), "someone\n").unwrap();
    git(&root, &["stash", "push", "-m", "not ours"]);
    let err = settle_stashing(&root, &mut j).unwrap_err();
    assert_eq!(err.code, "sync_stash_lost");
}

#[test]
fn restore_applies_and_drop_removes_only_ours() {
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/B.cs"), "someone-else\n").unwrap();
    git(&root, &["stash", "push", "-m", "someone else WIP"]); // 预先有一条别人的
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-r");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();

    assert_eq!(restore_wip(&root, &oid).unwrap(), RestoreOutcome::Applied);
    assert_eq!(std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(), "a-wip\n");
    drop_wip(&root, "op-r", &oid).unwrap();
    let list = git(&root, &["stash", "list"]);
    assert!(list.contains("someone else WIP"), "别人的 stash 不能碰：{list}");
    assert!(!list.contains("mygitclient-sync:op-r"), "我们的已经 drop：{list}");
    assert_eq!(
        run_git(&root, &["rev-parse", "-q", "--verify", &pin_ref("op-r")]).ok(),
        None,
        "钉住的 ref 已删"
    );
}

#[test]
fn restore_refuses_when_an_untracked_file_would_collide() {
    // Review Focus 1 / A3：撞名时 git 会半套 apply，所以必须在 apply 之前拒绝
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::write(fx.wc().join("Code/N.cs"), "mine\n").unwrap();
    let mut j = journal("op-c");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/N.cs"), "server\n").unwrap(); // 模拟服务器新增同名文件

    let out = restore_wip(&root, &oid).unwrap();
    assert_eq!(out, RestoreOutcome::Blocked { collisions: vec!["Code/N.cs".into()] });
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a1\n",
        "一个字节都不能 apply——已跟踪部分也不行"
    );
    assert!(git(&root, &["stash", "list"]).contains("mygitclient-sync:op-c"), "stash 保留");
}

#[test]
fn restore_passes_over_a_collision_that_is_the_same_file_everywhere() {
    // 计划 A17：外部 update 带来的服务器新增文件被 -u 收进 stash，基线跟上之后它又作为
    // 已跟踪文件回到磁盘。磁盘、stash、HEAD 三处是同一个 blob——不是撞名，是同一个文件
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let file = |rel: &str| std::fs::read_to_string(fx.wc().join(rel)).unwrap();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    std::fs::write(fx.wc().join("Code/Mine.cs"), "mine\n").unwrap();
    let mut j = journal("op-s");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    // 模拟「新基线里有 S.cs」：同样的字节进了 HEAD
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    git(&root, &["add", "Code/S.cs"]);
    git(&root, &["commit", "-m", "server adds S"]);

    assert_eq!(restore_wip(&root, &oid).unwrap(), RestoreOutcome::Applied);
    assert_eq!(file("Code/A.cs"), "a-wip\n");
    assert_eq!(file("Code/S.cs"), "server-new\n");
    assert_eq!(file("Code/Mine.cs"), "mine\n");
    assert_eq!(
        git(&root, &["status", "--porcelain", "--", "Code/S.cs"]),
        "",
        "S.cs 与 HEAD 一致，不显示成改动"
    );
}

#[test]
fn a_real_collision_blocks_even_next_to_a_same_file_one_and_removes_nothing() {
    // A17 的反方向：只要有一处字节不同就是真撞名；同一次里字节相同的那个也不能先被删
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    std::fs::write(fx.wc().join("Code/N.cs"), "mine\n").unwrap();
    let mut j = journal("op-t");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    git(&root, &["add", "Code/S.cs"]);
    git(&root, &["commit", "-m", "server adds S"]);
    std::fs::write(fx.wc().join("Code/N.cs"), "server\n").unwrap();

    assert_eq!(
        restore_wip(&root, &oid).unwrap(),
        RestoreOutcome::Blocked { collisions: vec!["Code/N.cs".into()] }
    );
    assert!(fx.wc().join("Code/S.cs").exists(), "拦下时不能已经删了 S.cs");
    assert!(git(&root, &["stash", "list"]).contains("mygitclient-sync:op-t"));
}

#[test]
fn a_same_name_file_edited_on_disk_is_a_real_collision() {
    // A17 必须三处都比：HEAD 与 stash 相同、但磁盘上的已经被改过（比如用户在冲突窗口里
    // 动了它），删掉它就是丢掉用户的字节
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    let mut j = journal("op-u");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    git(&root, &["add", "Code/S.cs"]);
    git(&root, &["commit", "-m", "server adds S"]);
    std::fs::write(fx.wc().join("Code/S.cs"), "edited\n").unwrap();

    assert_eq!(
        restore_wip(&root, &oid).unwrap(),
        RestoreOutcome::Blocked { collisions: vec!["Code/S.cs".into()] }
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/S.cs")).unwrap(),
        "edited\n"
    );
}

#[test]
fn restore_reports_conflicts_and_keeps_the_stash() {
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-x");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-committed\n").unwrap();
    git(&root, &["commit", "-am", "same line"]);

    match restore_wip(&root, &oid).unwrap() {
        RestoreOutcome::Conflicted { files } => assert_eq!(files, vec!["Code/A.cs".to_string()]),
        other => panic!("应当是冲突：{other:?}"),
    }
    assert!(git(&root, &["stash", "list"]).contains("mygitclient-sync:op-x"));
}

#[test]
fn restore_refuses_an_object_that_is_not_a_stash() {
    // §7.6.1 末：「恢复前必须先校验该 OID……是一个 stash 形状的 commit」
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let head = git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(restore_wip(&root, &head).unwrap_err().code, "sync_stash_missing");
}
```

- [ ] **Step 3：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_journal --test svn_sync_stash
```

期望：编译失败。

- [ ] **Step 4：journal 加 phase 与字段**

`src-tauri/src/engine/svn_journal.rs`：

1. `Phase` 里 `Rebase,` 之后加：

```rust
    /// S10：正在 / 已经把 stash 取回来（计划 A2，spec §7.6 本期补上）。
    ///
    /// 没有它的话，S9 成功之后只剩 `rebase`，崩溃重开时分不清 stash 有没有
    /// apply 过——再 apply 一次会撞上自己留下的改动。配合 [`Journal::stash_applied`]。
    ApplyingStash,
```

2. `Journal` 里 `stash_oid` 之后加：

```rust
    /// S10 的 `git stash apply` 已经执行过（成功或留下冲突）。
    ///
    /// 为真时「继续」只做 drop，**不再 apply**。在 apply 返回之后、drop 之前落盘；
    /// 崩在这两步之间时它仍是 false，继续会再 apply 一次——2026-09-25 实测 git
    /// 此时拒绝（「would be overwritten」）且不动任何文件，所以最坏是一条报错。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stash_applied: bool,
```

- [ ] **Step 5：实现 `svn_sync_stash.rs`**

新建 `src-tauri/src/engine/svn_sync_stash.rs`：

```rust
//! `sync_in` 的未提交改动：S1 压栈、S10 取回（spec §7.3.1、§7.6.1）。
//!
//! # 身份靠 opid，不靠栈位置
//!
//! `stash@{0}` 是栈位置，任何一次新的 stash 都会把它挤走。所以：push 时把 opid
//! 写进 message，事后按 opid 在 reflog 里找；找到之后立刻钉一个
//! `refs/mygitclient/sync/<opid>`，此后一律按不可变 OID 操作。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_include_check::normalize_roots;
use crate::engine::svn_journal::{self, Journal, Phase};
use crate::engine::svn_sync_check::unmerged_paths;
use crate::error::{GitError, Result};
use std::path::Path;

pub const MSG_PREFIX: &str = "mygitclient-sync:";

pub fn pin_ref(opid: &str) -> String {
    format!("refs/mygitclient/sync/{opid}")
}

/// `refs/stash` 当前指向谁；没有 stash 时 `None`。
pub fn read_stash_ref(repo: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(repo, &["rev-parse", "-q", "--verify", "refs/stash"])?;
    match code {
        0 => Ok(Some(out.trim().to_string())),
        // 1 = 不存在。128 是 ref 损坏，不能当成「没有」——同 `svn_baseline::read_baseline`
        1 => Ok(None),
        _ => Err(GitError::new(
            "sync_stash",
            format!("读 refs/stash 失败（rev-parse 退出 {code}）"),
        )),
    }
}

/// 按 opid 在 stash 的 reflog 里找我们那一条（§7.3.1 的确定性判定）。
pub fn find_by_opid(repo: &str, opid: &str) -> Result<Option<String>> {
    if read_stash_ref(repo)?.is_none() {
        return Ok(None); // 没有 refs/stash 时 `log -g` 会以 128 失败
    }
    let out = run_git(repo, &["log", "-g", "--format=%H %gs", "refs/stash"])?;
    // %gs 形如 `On work: mygitclient-sync:<opid>`（2026-09-25 实测）
    let tag = format!(": {MSG_PREFIX}{opid}");
    Ok(out
        .lines()
        .find(|l| l.trim_end().ends_with(&tag))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string))
}

/// S1 ①–⑤。结束时 `j.phase == Stashed`。
pub fn stash_wip(repo: &str, j: &mut Journal, whitelist: &[String]) -> Result<()> {
    let roots = normalize_roots(whitelist);
    if roots.is_empty() {
        // spec §12.2：`git stash push --` 后面什么都不跟，压的就是整个仓库
        return Err(GitError::new(
            "sync_stash_empty_whitelist",
            "白名单是空的。空 pathspec 会让 stash 压走整个仓库，一律拒绝。",
        ));
    }
    // ① 写在 push 之前（§7.3.1）
    j.stash_ref_before = read_stash_ref(repo)?;
    j.phase = Phase::Stashing;
    svn_journal::write(repo, j)?;

    // ②
    let msg = format!("{MSG_PREFIX}{}", j.opid);
    let mut args: Vec<&str> = vec!["stash", "push", "-u", "-m", &msg, "--"];
    args.extend(roots.iter().map(String::as_str));
    if let Err(e) = run_git(repo, &args) {
        // push 失败时先看它到底落没落地：没落地就什么都没发生，journal 可以删
        if find_by_opid(repo, &j.opid)?.is_none() && read_stash_ref(repo)? == j.stash_ref_before {
            svn_journal::remove(repo)?;
        }
        return Err(e);
    }
    // ③–⑤
    settle_stashing(repo, j)
}

/// §7.3.1 的判定树：phase=stashing 之后 push 落没落地、落在哪。
///
/// 正常路径与崩溃恢复共用——恢复时读到 `stashing` 也走这里。
pub fn settle_stashing(repo: &str, j: &mut Journal) -> Result<()> {
    match find_by_opid(repo, &j.opid)? {
        Some(oid) => {
            // ④ 钉住：之后即使 stash 被外部 drop，对象也不会被 gc
            run_git(repo, &["update-ref", &pin_ref(&j.opid), &oid])?;
            j.stash_oid = Some(oid);
        }
        None => {
            if read_stash_ref(repo)? != j.stash_ref_before {
                return Err(GitError::new(
                    "sync_stash_lost",
                    "stash 栈变了，却找不到本次同步压的那一条——可能被外部工具 \
                     `git stash clear` / `drop` 过。为了不把别人的 WIP 当成你的，这里停下。\
                     请用 `git fsck --unreachable | findstr commit` 找回，或联系开发者。",
                ));
            }
            // push 没落地：白名单内本来就没有可压的改动（计划 A7）
            j.stash_oid = None;
        }
    }
    j.phase = Phase::Stashed; // ⑤
    svn_journal::write(repo, j)
}

#[derive(Debug, Clone, PartialEq)]
pub enum RestoreOutcome {
    Applied,
    /// apply 了，留下冲突标记；stash 保留（spec §13 S10 那一行）。
    Conflicted { files: Vec<String> },
    /// 一个字节都没 apply：stash 里的未跟踪文件在磁盘上已经有同名的了（计划 A3）。
    Blocked { collisions: Vec<String> },
}

/// S10：按 OID 取回。
pub fn restore_wip(repo: &str, oid: &str) -> Result<RestoreOutcome> {
    let parents = stash_parents(repo, oid)?;
    // 带 `-u` 的 stash 有第三个父提交，里面是未跟踪文件。
    //
    // ⚠️ 必须在 apply **之前**查撞名：2026-09-25 实测，撞名时 git 先把已跟踪
    // 部分写上去、再在未跟踪上失败（退出 1、没有 `UU`）——半套 apply，而再
    // apply 一次又会撞上自己刚写下的改动。
    let mut same_file = Vec::new();
    if let Some(untracked) = parents.get(2) {
        let out = run_git(repo, &["ls-tree", "-r", "--name-only", "-z", untracked])?;
        let mut collisions = Vec::new();
        for p in out.split('\0').filter(|p| !p.is_empty()) {
            if Path::new(repo).join(p).symlink_metadata().is_err() {
                continue;
            }
            if is_same_file_everywhere(repo, untracked, p)? {
                same_file.push(p.to_string());
            } else {
                collisions.push(p.to_string());
            }
        }
        // 真撞名时**一个字节都不动**：字节相同的那批也还没删
        if !collisions.is_empty() {
            return Ok(RestoreOutcome::Blocked { collisions });
        }
        // 计划 A17：三处同一个 blob 的，先从磁盘拿掉，让 apply 把同样的字节写回来。
        // 删的是 HEAD 里原样存着的内容，不是用户独有的数据
        for p in &same_file {
            std::fs::remove_file(Path::new(repo).join(p))?;
        }
    }
    let result = match run_git(repo, &["stash", "apply", oid]) {
        Ok(_) => Ok(RestoreOutcome::Applied),
        Err(e) => {
            let files = unmerged_paths(repo)?;
            if files.is_empty() {
                Err(GitError::new(
                    "sync_stash_apply_failed",
                    format!(
                        "取回未提交改动失败了，而且不是冲突。stash 原样保留（{oid}）。git 原话：{}",
                        e.message
                    ),
                ))
            } else {
                Ok(RestoreOutcome::Conflicted { files })
            }
        }
    };
    // apply 没把它们写回来（中途失败）时从 HEAD 取回：HEAD 里就是同一个 blob。
    // `p` 来自 ls-tree，不会是空 pathspec（§12.2）
    for p in &same_file {
        if Path::new(repo).join(p).symlink_metadata().is_err() {
            run_git(repo, &["checkout", "HEAD", "--", p])?;
        }
    }
    result
}

/// 计划 A17：stash 里的这个未跟踪文件，与磁盘上撞名的文件、HEAD 里的同名文件是
/// **同一个 blob**——它不是两份内容在抢一个路径，而是同一个文件出现了三次。
///
/// 实际场景：外部在项目根 `svn update` 带来了服务器新增的文件，git 视它为未跟踪，
/// S1 的 `-u` 把它收进 stash；基线跟上之后它又作为已跟踪文件回到磁盘
/// （2026-09-25 实测，`probe-ext-added.ps1`）。按 A3 一律拦下的话，只要同事新增过
/// 文件，每次接管都会停在 `StashBlocked`。
///
/// 任何一处不同、HEAD 里没有它、或它不是普通文件，都按真撞名处理。
fn is_same_file_everywhere(repo: &str, untracked: &str, p: &str) -> Result<bool> {
    if !Path::new(repo).join(p).is_file() {
        return Ok(false);
    }
    let (code, head) = run_git_code(repo, &["rev-parse", "-q", "--verify", &format!("HEAD:{p}")])?;
    if code != 0 {
        return Ok(false);
    }
    let stash = run_git(repo, &["rev-parse", "--verify", &format!("{untracked}:{p}")])?;
    // `--path` 让磁盘内容走与 `git add` 相同的过滤器（svn_baseline.rs 记过这个坑）
    let disk = run_git(repo, &["hash-object", &format!("--path={p}"), "--", p])?;
    Ok(head.trim() == stash.trim() && stash.trim() == disk.trim())
}

/// stash commit 的父提交列表。不是 stash 形状（少于两个父）就报错（§7.6.1 末）。
fn stash_parents(repo: &str, oid: &str) -> Result<Vec<String>> {
    let missing = || {
        GitError::new(
            "sync_stash_missing",
            format!(
                "记录里的 stash {oid} 找不到了，或者它不是一个 stash。\
                 不猜、不退回用 stash@{{0}}（§7.6.1）——请联系开发者。"
            ),
        )
    };
    let (code, out) = run_git_code(repo, &["rev-list", "--parents", "-n", "1", oid])?;
    if code != 0 {
        return Err(missing());
    }
    // 输出：`<oid> <p1> <p2> [<p3>]`
    let parents: Vec<String> = out.split_whitespace().skip(1).map(str::to_string).collect();
    if parents.len() < 2 {
        return Err(missing());
    }
    Ok(parents)
}

/// 按 OID drop 我们那一条，再删钉住的 ref。
pub fn drop_wip(repo: &str, opid: &str, oid: &str) -> Result<()> {
    if read_stash_ref(repo)?.is_some() {
        let out = run_git(repo, &["log", "-g", "--format=%H", "refs/stash"])?;
        if let Some(n) = out.lines().position(|l| l.trim() == oid) {
            let dropped = run_git(repo, &["stash", "drop", &format!("stash@{{{n}}}")])?;
            // 输出形如 `Dropped stash@{1} (<oid>)`（2026-09-25 实测）。
            // 定位与 drop 之间若有外部工具又压了一条，位置会错开——核对一下，
            // 丢错了就把它塞回去，宁可报错也不吃掉别人的 WIP。
            if !dropped.contains(oid) {
                if let Some(other) = dropped
                    .rsplit_once('(')
                    .and_then(|(_, r)| r.split_once(')'))
                    .map(|(o, _)| o.trim().to_string())
                {
                    let _ = run_git(
                        repo,
                        &["stash", "store", "-m", "mygitclient 误删后放回", &other],
                    );
                }
                return Err(GitError::new(
                    "sync_stash_drop_raced",
                    "删除 stash 的瞬间栈被外部改动了。已把误删的那条放回，本次同步的 stash \
                     仍在，请稍后在横幅上点「继续」重试。",
                ));
            }
        }
    }
    unpin(repo, opid)
}

/// 删掉 `refs/mygitclient/sync/<opid>`；不存在也算成功。
pub fn unpin(repo: &str, opid: &str) -> Result<()> {
    let r = pin_ref(opid);
    let (code, _) = run_git_code(repo, &["rev-parse", "-q", "--verify", &r])?;
    if code == 0 {
        run_git(repo, &["update-ref", "-d", &r])?;
    }
    Ok(())
}
```

`src-tauri/src/engine/mod.rs` 加 `pub mod svn_sync_stash;`。

- [ ] **Step 6：跑测试，确认通过**

```powershell
cargo test --test svn_journal --test svn_sync_stash
```

- [ ] **Step 7：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `restore_wip` 去掉撞名预检（`if let Some(untracked)` 整段） | `restore_refuses_when_an_untracked_file_would_collide` |
| `find_by_opid` 改成「取 reflog 第一行」 | `settle_finds_our_stash_even_when_another_was_pushed_on_top` |
| `settle_stashing` 的 `None` 分支去掉 `read_stash_ref` 比对 | `settle_stops_when_the_stash_vanished` |
| `drop_wip` 固定 drop `stash@{0}` | `restore_applies_and_drop_removes_only_ours` |
| `is_same_file_everywhere` 恒为 `false`（回到 A3 一律拦下） | `restore_passes_over_a_collision_that_is_the_same_file_everywhere` |
| `is_same_file_everywhere` 不比磁盘 blob（只比 HEAD 与 stash） | `a_same_name_file_edited_on_disk_is_a_real_collision` |
| 真撞名时先删了字节相同的文件再返回 `Blocked` | `a_real_collision_blocks_even_next_to_a_same_file_one_and_removes_nothing` |

- [ ] **Step 8：spec 同步**

`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（CRLF，用 Edit）：

1. §7.6「`sync_in` 另有两个 phase 与 stash 相关」那张表末尾加一行：

```markdown
| `applying-stash` | S9 已完成，正在或已经执行 S10 的 `git stash apply <OID>`；`stashApplied` 为真表示 apply 已执行（成功或留下冲突） | `stashApplied` 为假：可再次 apply（先查未跟踪文件撞名）；为真：只 drop，**不再 apply**。放弃时只删 journal 与钉住的 ref，stash 条目原样保留给用户 |
```

并把表前那句「另有两个 phase」改成「另有三个 phase」。

2. §7.3 步骤块里 S10 那行下面加一行注记：

```text
     ⚠️ apply 之前先查 stash 里的未跟踪文件在磁盘上有没有同名的，有就一个字节
        都不 apply（2026-09-25 实测：撞名时 git 会先写已跟踪部分再失败，半套 apply）。
        例外：磁盘、stash、HEAD 三处是同一个 blob 的，是同一个文件，先从磁盘拿掉
        再 apply（外部 update 带来的服务器新增文件就是这个形状）
```

- [ ] **Step 9：全量检查、更新决策日志、提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs
git commit -m "feat(svn): sync_in 的 WIP 按 opid 压栈、按 OID 取回（S1 / S10）"
```

---

## Task 4：基线录制零件拆分

S7 要在「创建 commit 前」和「移动 ref 前」各落一次 journal（spec §7.6 `publishing-baseline` 那一行），而现有 `publish_baseline` 把两步合在一次调用里；S7 的候选索引要从 HEAD 树起步（§6.3.1 第 638 行），而 `record_server_tree` 写死了空索引。本 Task 只拆接口，**既有调用方行为逐字不变**。

**Files:**
- Modify: `src-tauri/src/engine/svn_baseline.rs`
- Modify: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_baseline.rs`、`src-tauri/tests/svn_init.rs`

**Interfaces:**
- Produces:
  - `svn_baseline::create_baseline_commit(repo_root: &str, tree: &str, parent: Option<&str>, t: &Trailer) -> Result<String>`：只建 commit，不动任何 ref
  - `svn_baseline::move_baseline_ref(repo_root: &str, oid: &str, expected_old: Option<&str>) -> Result<()>`：带旧值 CAS
  - `pub(crate) svn_baseline::read_baseline(repo_root: &str) -> Result<Option<String>>`（原私有函数改可见性）
  - `svn_init::record_server_tree_into(repo_root: &str, cfg: &SvnConfig, pre: &Preflight, idx: &TempIndex) -> Result<(String, BTreeSet<String>)>`（`pub`，测试要用）
  - `publish_baseline` 与 `record_server_tree` 签名不变，改成上面两者的包装

- [ ] **Step 1：写失败的测试**

`src-tauri/tests/svn_baseline.rs` 末尾追加（该文件已有 `fixture_git` / 临时仓库的写法，照它的风格；下面只用 `run_git` 与 `tempfile`，自足）：

```rust
mod split_publish {
    use app_lib::engine::git_cli::run_git;
    use app_lib::engine::svn_baseline::{
        create_baseline_commit, move_baseline_ref, publish_baseline, Trailer,
    };

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().to_str().unwrap();
        run_git(r, &["init", "-q", "-b", "svn/base"]).unwrap();
        std::fs::write(d.path().join("A.cs"), "a\n").unwrap();
        run_git(r, &["add", "A.cs"]).unwrap();
        run_git(r, &["commit", "-qm", "base"]).unwrap();
        d
    }

    fn t(rev: u64) -> Trailer {
        Trailer {
            url: "file:///C:/repo".into(),
            rev,
            uuid: "0f3a0000-0000-0000-0000-000000000000".into(),
        }
    }

    #[test]
    fn creating_a_commit_does_not_move_the_ref() {
        // §7.6 publishing-baseline：commit 建好、ref 还没动之间要能落一次 journal
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"]).unwrap().trim().to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"]).unwrap().trim().to_string();
        let c = create_baseline_commit(r, &tree, Some(&old), &t(2)).unwrap();
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), old);
        move_baseline_ref(r, &c, Some(&old)).unwrap();
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), c);
    }

    #[test]
    fn moving_refuses_when_the_baseline_moved_in_between() {
        // 必测 #73：注入旧基线 OID 改变时 CAS 拒绝覆盖
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"]).unwrap().trim().to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"]).unwrap().trim().to_string();
        let ours = create_baseline_commit(r, &tree, Some(&old), &t(2)).unwrap();
        let theirs = create_baseline_commit(r, &tree, Some(&old), &t(3)).unwrap();
        run_git(r, &["update-ref", "refs/heads/svn/base", &theirs]).unwrap();
        let err = move_baseline_ref(r, &ours, Some(&old)).unwrap_err();
        assert_eq!(err.code, "svn_baseline_moved");
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), theirs, "别人的不被覆盖");
    }

    #[test]
    fn publish_is_still_create_then_move() {
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"]).unwrap().trim().to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"]).unwrap().trim().to_string();
        let c = publish_baseline(r, &tree, Some(&old), &t(2), Some(&old)).unwrap();
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), c);
        let msg = run_git(r, &["log", "-1", "--format=%B", "svn/base"]).unwrap();
        assert!(msg.contains("git-svn-id: file:///C:/repo@2 "), "{msg}");
    }
}
```

`src-tauri/tests/svn_init.rs` 末尾追加：

```rust
#[test]
fn recording_from_head_agrees_with_recording_from_empty_after_an_update() {
    // §6.3.1：S7 从 HEAD 树起步，初始化 / R2 从空索引起步。两者在「磁盘就是
    // 服务器内容」时必须录出同一棵树——差别只可能出在 HEAD 里有、磁盘上没有的
    // 文件，而 `git add -A` 会把它们摘掉。
    use app_lib::engine::svn_baseline::TempIndex;
    use app_lib::engine::svn_config::read_config;
    use app_lib::engine::svn_init::record_server_tree_into;
    use app_lib::engine::svn_preflight::run_preflight;

    let fx = svn_fixtures::hybrid_with_colleague();
    let root = fx.wc_str();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_fixtures::svn_cmd(&fx.wc2(), &["delete", "Code/B.cs", "--non-interactive"]);
    svn_fixtures::svn_cmd(&fx.wc2(), &["commit", "-m", "del B", "--non-interactive"]);
    run_git(&root, &["checkout", "-q", "svn/base"]).unwrap(); // S3
    svn_fixtures::svn_cmd(&fx.wc(), &["update", "--non-interactive"]); // S5

    let cfg = read_config(&root).unwrap();
    let pre = run_preflight(&root, &cfg.whitelist).unwrap();
    let (from_head, _) =
        record_server_tree_into(&root, &cfg, &pre, &TempIndex::from_head(&root).unwrap()).unwrap();
    let (from_empty, _) =
        record_server_tree_into(&root, &cfg, &pre, &TempIndex::empty(&root).unwrap()).unwrap();
    assert_eq!(from_head, from_empty);
    let ls = run_git(&root, &["ls-tree", "-r", "--name-only", &from_head]).unwrap();
    assert!(!ls.contains("Code/B.cs"), "服务器删掉的文件不能被 HEAD 树带回来：{ls}");
}

#[test]
fn recording_from_head_still_catches_a_new_gitignore() {
    // 必测 #70 的 S7 半边：服务器带来一个挡住 .cs 的 .gitignore，候选树必须被判漏收
    use app_lib::engine::svn_baseline::TempIndex;
    use app_lib::engine::svn_config::read_config;
    use app_lib::engine::svn_init::record_server_tree_into;
    use app_lib::engine::svn_preflight::run_preflight;

    let fx = svn_fixtures::hybrid_with_colleague();
    let root = fx.wc_str();
    fx.colleague_commits("Code/.gitignore", "*.cs\n");
    fx.colleague_commits("Code/New.cs", "new\n");
    run_git(&root, &["checkout", "-q", "svn/base"]).unwrap();
    svn_fixtures::svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let cfg = read_config(&root).unwrap();
    let pre = run_preflight(&root, &cfg.whitelist).unwrap();
    let err = record_server_tree_into(&root, &cfg, &pre, &TempIndex::from_head(&root).unwrap())
        .unwrap_err();
    assert_eq!(err.code, "include_set_mismatch");
    assert!(err.message.contains("Code/New.cs"), "{}", err.message);
}
```

（`svn_init.rs` 顶部若还没有 `use app_lib::engine::git_cli::run_git;` 就补上。）

- [ ] **Step 2：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_baseline split_publish
cargo test --test svn_init recording_from_head
```

期望：编译失败（新函数不存在）。

- [ ] **Step 3：拆 `publish_baseline`**

`src-tauri/src/engine/svn_baseline.rs`：把 `publish_baseline` 的函数体拆成两个新函数，原函数改成包装。**原有注释各归其位，不要丢**（「`-m` 不能走 stdin」那段归 `create_baseline_commit`；「为什么非得带旧值」「先自己读一次再比对」两段归 `move_baseline_ref`）。

```rust
/// 用 `TREE` 建一个基线 commit，**不动任何 ref**。
///
/// 与 [`move_baseline_ref`] 分开，是因为 `sync_in` 的 S7 要在两步之间落一次
/// journal（spec §7.6 `publishing-baseline`：创建 commit 前记录 captureTree，
/// 移动 ref 前再记录 newBaselineOid）。崩在两步之间时，重开就知道「commit 已建、
/// ref 未动」，只补 update-ref，不重建另一个 commit。
pub fn create_baseline_commit(
    repo_root: &str,
    tree: &str,
    parent: Option<&str>,
    t: &Trailer,
) -> Result<String> {
    // `commit-tree` 的 message **必须走 `-m`，不能走 stdin**：……（原注释整段搬过来）
    let msg = format!("svn: r{}\n\n{}\n", t.rev, format_trailer(t));
    let mut args: Vec<&str> = vec!["commit-tree", tree];
    if let Some(p) = parent {
        args.push("-p");
        args.push(p);
    }
    args.push("-m");
    args.push(&msg);
    Ok(run_git(repo_root, &args)?.trim().to_string())
}

/// **以旧 OID 作比较条件**把基线分支移到 `oid`。
///
/// （原 `publish_baseline` 文档里「为什么非得带旧值」一节整段搬过来。）
pub fn move_baseline_ref(repo_root: &str, oid: &str, expected_old: Option<&str>) -> Result<()> {
    let current = read_baseline(repo_root)?;
    if current.as_deref() != expected_old {
        return Err(moved_error(expected_old, current.as_deref()));
    }
    let old = expected_old.unwrap_or(ZERO_OID);
    run_git(repo_root, &["update-ref", &baseline_ref(), oid, old]).map_err(|e| {
        // （原来那段「走到这里说明……」注释搬过来）
        GitError::new(
            "svn_baseline_moved",
            format!(
                "更新基线分支失败——发布前的一瞬间它被别人改了。\
                 已经建好的 commit 是 {oid}（对象还在，没有丢）。git 原话：{}",
                e.message
            ),
        )
    })?;
    Ok(())
}

/// 建 commit 并发布。`init_hybrid` 与 `rebuild_baseline` 用它，行为与拆分前逐字相同：
/// 先比对旧值，比对不过就不建 commit。
pub fn publish_baseline(
    repo_root: &str,
    tree: &str,
    parent: Option<&str>,
    t: &Trailer,
    expected_old: Option<&str>,
) -> Result<String> {
    let current = read_baseline(repo_root)?;
    if current.as_deref() != expected_old {
        return Err(moved_error(expected_old, current.as_deref()));
    }
    let oid = create_baseline_commit(repo_root, tree, parent, t)?;
    move_baseline_ref(repo_root, &oid, expected_old)?;
    Ok(oid)
}
```

`fn read_baseline` 改成 `pub(crate) fn read_baseline`。

- [ ] **Step 4：拆 `record_server_tree`**

`src-tauri/src/engine/svn_init.rs`：

1. 把现有 `fn record_server_tree(repo_root, cfg, pre)` 改名为 `pub fn record_server_tree_into(repo_root: &str, cfg: &SvnConfig, pre: &Preflight, idx: &TempIndex)`，删掉函数体里 `let idx = TempIndex::empty(repo_root)?;` 那一行，其余 `idx.xxx(...)` 调用照旧（`idx` 现在是引用，调用形式不变）。文档注释末尾加一段：

```rust
/// # 候选索引由调用方给
///
/// spec §6.3.1：「初始化 / Phase 1 R2 从空索引开始，其余录制入口从当前 HEAD 树
/// 开始」。`sync_in` 的 S7 属于后者。起点不同、后面的改写与核对完全相同，所以
/// 只把「从哪棵树起步」交出去。
```

2. 在它之前加回原名的包装：

```rust
/// 从**空索引**起步录服务器 rN 的树。初始化与 Phase 1 R2 用它（spec §6.3.1）。
fn record_server_tree(
    repo_root: &str,
    cfg: &SvnConfig,
    pre: &Preflight,
) -> Result<(String, BTreeSet<String>)> {
    let idx = TempIndex::empty(repo_root)?;
    record_server_tree_into(repo_root, cfg, pre, &idx)
}
```

- [ ] **Step 5：跑测试，确认通过；全量回归**

```powershell
cargo test --test svn_baseline --test svn_init
cargo fmt --check; cargo clippy --all-targets; cargo test
```

期望：新测试 PASS；既有 598 条一条不红（拆分不改行为）。

- [ ] **Step 6：更新决策日志、提交**

```powershell
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "refactor(svn): 基线发布拆成建 commit 与移动 ref 两步，录制可从 HEAD 树起步"
```

---

## Task 5：编排主路径 S0–S11 + 冲突出口

本 Task 写出整个 `svn_sync_in.rs` 的正向路径。S3 / S4 / S5 的四个失败出口先做成「如实报错、journal 原样保留」的最小形态（`on_*` 四个函数），Task 7 再把它们换成「回到原分支」；继续 / 放弃在 Task 6。

**Files:**
- Create: `src-tauri/src/engine/svn_sync_in.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: Create `src-tauri/tests/svn_sync_in.rs`

**Interfaces:**
- Consumes: Task 2 `check_sync_in` / `assert_linear` / `current_branch` / `rev_parse` / `unmerged_paths` / `unfinished_error` / `list_paths`；Task 3 `stash_wip` / `restore_wip` / `drop_wip`；Task 4 `create_baseline_commit` / `move_baseline_ref` / `read_baseline` / `record_server_tree_into`；既有 `swap_index_to_tree`、`run_preflight`、`read_node_rev`、`check_targets`、`enumerate_wcs`、`read_dirty`、`file_lock::{find_blockers, describe}`。
- Produces:
  - `svn_sync_in::SyncInExpect { orig_head: String, baseline_oid: String, confirmed_outside: Vec<String> }`（`Deserialize`，camelCase）
  - `svn_sync_in::SyncInOutcome`（`Serialize`，`tag = "kind"`，camelCase）：`Done { new_rev: u64 }` / `Aborted` / `RebaseConflict { files: Vec<String> }` / `StashConflict { files: Vec<String>, stash_oid: String }` / `StashBlocked { collisions: Vec<String>, stash_oid: String }`
  - `svn_sync_in::run_sync_in(repo: &str, expect: &SyncInExpect) -> Result<SyncInOutcome>`
  - 模块内部（Task 6 / 7 要用）：`from_stashed`、`capture_and_publish`、`finish_publish`、`from_baseline_ready`、`conflict_or_error`、`finish_forward`、`restore_step`、`RestoreStep`、`checkout_verified`、`dirty_w_files`、`rebase_in_progress`、`need`、`state_changed`、`unresolved`

- [ ] **Step 1a：补 fixture——白名单落在同仓库 external 里（必测 #49）**

spec §16.0：公司项目的 `code` / `script` 就是同一个仓库里的两条 external。这是用户**真实会走**的形态，不是边角。`src-tauri/tests/common/svn_fixtures.rs` 末尾追加：

```rust
/// 必测 #49 / spec §16.0 的真实形态：白名单落在**同仓库**的 external 里。
///
/// 仓库布局：`code-trunk/A.cs`、`proj/Art/x.psd`；`proj` 上挂 `^/code-trunk code`
/// （**不钉 revision**）。wc1 是 `proj` 的检出，`wc1/code` 是 external；wc2 是
/// `code-trunk` 的检出，扮演直接在代码目录里提交的同事。
pub fn hybrid_whitelist_in_external() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(
        &root,
        "svn",
        &["checkout", &url, "seed", "--non-interactive"],
    );
    let seed = root.join("seed");
    std::fs::create_dir_all(seed.join("code-trunk")).unwrap();
    std::fs::create_dir_all(seed.join("proj/Art")).unwrap();
    std::fs::write(seed.join("code-trunk/A.cs"), "a1\n").unwrap();
    std::fs::write(seed.join("proj/Art/x.psd"), "psd1\n").unwrap();
    run(
        &seed,
        "svn",
        &["add", "code-trunk", "proj", "--non-interactive"],
    );
    run(&seed, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    // 属性值走 -F 文件：命令行传值遇到混合行尾会让 propset 报 E135000 而属性
    // 静默没设上（见 svn_repo_pinned_external_updated_to_head 的注释）
    let ext = root.join("externals.txt");
    std::fs::write(&ext, "^/code-trunk code\n").unwrap();
    run(
        &seed,
        "svn",
        &[
            "propset",
            "svn:externals",
            "-F",
            ext.to_str().unwrap(),
            "proj",
            "--non-interactive",
        ],
    );
    run(
        &seed,
        "svn",
        &["commit", "-m", "r2 external", "--non-interactive"],
    );
    run(
        &root,
        "svn",
        &["checkout", &format!("{url}/proj"), "wc1", "--non-interactive"],
    );
    run(
        &root,
        "svn",
        &[
            "checkout",
            &format!("{url}/code-trunk"),
            "wc2",
            "--non-interactive",
        ],
    );
    app_lib::engine::svn_init::init_hybrid(&app_lib::engine::svn_init::InitRequest {
        repo_root: root.join("wc1").to_string_lossy().to_string(),
        whitelist: vec!["code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .expect("fixture：在 external 形态上初始化混合仓库失败");
    SvnFixture { dir }
}
```

- [ ] **Step 1：写失败的测试**

新建 `src-tauri/tests/svn_sync_in.rs`：

```rust
//! `sync_in` 编排（spec §7.3）。全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::parse_trailer;
use app_lib::engine::svn_journal::{self, Phase};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect, SyncInOutcome};
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}
fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    std::fs::write(fx.wc().join(rel), s).unwrap();
}
fn commit(fx: &SvnFixture, msg: &str) {
    git(fx, &["commit", "-qam", msg]);
}
fn baseline_rev(fx: &SvnFixture) -> u64 {
    parse_trailer(&git(fx, &["log", "-1", "--format=%B", "svn/base"])).unwrap().rev
}
fn expect(fx: &SvnFixture) -> SyncInExpect {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    SyncInExpect {
        orig_head: p.orig_head,
        baseline_oid: p.baseline_oid,
        confirmed_outside: p.outside_changes,
    }
}
fn sync(fx: &SvnFixture) -> SyncInOutcome {
    run_sync_in(&fx.wc_str(), &expect(fx)).unwrap()
}
fn phase(fx: &SvnFixture) -> Option<Phase> {
    svn_journal::read(&fx.wc_str()).unwrap().map(|j| j.phase)
}
fn our_stashes(fx: &SvnFixture) -> usize {
    git(fx, &["stash", "list"])
        .lines()
        .filter(|l| l.contains("mygitclient-sync:"))
        .count()
}

#[test]
fn a_plain_update_advances_the_baseline_and_replays_the_branch() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-local\n");
    commit(&fx, "local");
    let local = git(&fx, &["rev-parse", "HEAD"]);
    write(&fx, "Code/B.cs", "b-wip\n"); // 未提交改动
    write(&fx, "Code/New.cs", "untracked\n"); // 未跟踪的 W-file

    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });

    assert_eq!(baseline_rev(&fx), 2);
    assert_eq!(read(&fx, "Code/A.cs"), "a2\n", "服务器的改动到了磁盘上");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n", "WIP 回来了");
    assert_eq!(read(&fx, "Code/New.cs"), "untracked\n");
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "1");
    assert_ne!(git(&fx, &["rev-parse", "HEAD"]), local, "rebase 之后是新 OID");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(phase(&fx), None, "S11 删掉 journal");
    assert!(!svn_lock::is_held(&fx.wc_str()), "锁已释放");
    assert_eq!(our_stashes(&fx), 0, "stash 已 drop");
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/"]), "", "钉住的 ref 已删");
    assert_eq!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync,
        "更新之后不能误报失同步（§7.4：S7 必须跟上 revision）"
    );
}

#[test]
fn nothing_new_on_the_server_still_records_a_baseline_commit() {
    // §7.4：「S7 白名单内无变化时也要落 commit」
    let fx = hybrid_with_colleague();
    let before = git(&fx, &["rev-parse", "svn/base"]);
    let tree_before = git(&fx, &["rev-parse", "svn/base^{tree}"]);
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 1 });
    assert_ne!(git(&fx, &["rev-parse", "svn/base"]), before);
    assert_eq!(git(&fx, &["rev-parse", "svn/base^{tree}"]), tree_before);
}

#[test]
fn a_local_only_patch_survives_sync_in() {
    // 必测 #21（§6.7.1）：Local-Only 就是普通 commit，S9 把它一起搬过去
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-patch\n");
    git(&fx, &["commit", "-qam", "debug patch\n\nLocal-Only: true"]);
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/B.cs"), "b-patch\n");
    assert!(git(&fx, &["log", "-1", "--format=%B", "work"]).contains("Local-Only: true"));
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "1");
}

#[test]
fn a_linear_round_trip_is_not_reported_as_lost() {
    // 必测 #71（§7.3.2）：本地 a1→one→a1，新基线是 one。rebase 后 A 回到 a1，
    // 第一笔因为变空被 --empty=drop 丢掉，不算错误
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "one\n");
    write(&fx, "Code/A.cs", "one\n");
    commit(&fx, "to one");
    write(&fx, "Code/A.cs", "a1\n");
    commit(&fx, "back");
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/A.cs"), "a1\n");
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "1");
}

#[test]
fn a_rebase_conflict_stops_with_the_lock_released() {
    // 必测 #4 / #69 的 S9 部分：冲突是正常出口，锁必须放开（用户要暂存文件）
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(&fx, "Code/A.cs", "a-local\n");
    commit(&fx, "local A");
    write(&fx, "Code/B.cs", "b-wip\n");
    match sync(&fx) {
        SyncInOutcome::RebaseConflict { files } => assert_eq!(files, vec!["Code/A.cs".to_string()]),
        o => panic!("应当是 rebase 冲突：{o:?}"),
    }
    assert_eq!(phase(&fx), Some(Phase::Rebase));
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert!(fx.wc().join(".git/rebase-merge").exists());
    assert_eq!(baseline_rev(&fx), 2, "基线已推进，冲突发生在它之后");
    assert_eq!(our_stashes(&fx), 1, "WIP 还压着，等 S10");
}

#[test]
fn a_stash_conflict_keeps_the_stash_and_marks_it_applied() {
    // spec §13 S10：「不回滚，stash 与钉住的 ref 都保留不删」
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(&fx, "Code/A.cs", "a-wip\n"); // 未提交
    let oid = match sync(&fx) {
        SyncInOutcome::StashConflict { files, stash_oid } => {
            assert_eq!(files, vec!["Code/A.cs".to_string()]);
            stash_oid
        }
        o => panic!("应当是 stash 冲突：{o:?}"),
    };
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(j.phase, Phase::ApplyingStash);
    assert!(j.stash_applied, "已经 apply 过，继续时不能再 apply");
    assert_eq!(j.stash_oid.as_deref(), Some(oid.as_str()));
    assert_eq!(our_stashes(&fx), 1);
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

#[test]
fn untracked_collision_after_update_stops_before_applying() {
    // Review Focus 1 / A3：服务器新增了与 WIP 里未跟踪文件同名的文件
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "server\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    write(&fx, "Code/N.cs", "mine\n");
    match sync(&fx) {
        SyncInOutcome::StashBlocked { collisions, .. } => {
            assert_eq!(collisions, vec!["Code/N.cs".to_string()])
        }
        o => panic!("应当被撞名挡住：{o:?}"),
    }
    assert_eq!(read(&fx, "Code/B.cs"), "b1\n", "一个字节都没 apply");
    assert_eq!(read(&fx, "Code/N.cs"), "server\n");
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(j.phase, Phase::ApplyingStash);
    assert!(!j.stash_applied);
}

#[test]
fn a_second_sync_while_one_is_running_is_refused() {
    // 必测 #12
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let _other = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_progress");
    assert_eq!(phase(&fx), None);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
}

#[test]
fn new_outside_changes_since_the_preview_require_reconfirmation() {
    // §7.2 第 7 条：确认之后出现了用户没看过的「会被碰到的美术文件」
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    write(&fx, "Art/x.psd", "psd-local\n");
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_needs_reconfirm");
    assert!(err.message.contains("Art/x.psd"));
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn a_branch_moved_since_the_preview_is_refused() {
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    write(&fx, "Code/B.cs", "b-late\n");
    commit(&fx, "committed after preview");
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_state_changed");
    assert_eq!(phase(&fx), None);
}

#[test]
fn a_whitelist_inside_a_same_repo_external_updates() {
    // 必测 #49：公司项目就是这个形态（spec §16.0）。S5 在项目根递归 update 会
    // 连带更新 external；记账单位取 external 那个 WC 的 revision，trailer 的
    // url 取 Repository Root（§6.6.5）
    let fx = svn_fixtures::hybrid_whitelist_in_external();
    fx.colleague_commits("A.cs", "a-server\n");
    write(&fx, "code/New.cs", "wip\n");
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 3 });
    assert_eq!(read(&fx, "code/A.cs"), "a-server\n");
    assert_eq!(read(&fx, "code/New.cs"), "wip\n");
    assert_eq!(check_sync(&fx.wc_str()).unwrap().verdict, SyncVerdict::InSync);
    let t = parse_trailer(&git(&fx, &["log", "-1", "--format=%B", "svn/base"])).unwrap();
    assert!(t.url.ends_with("/repo"), "trailer 记 Repository Root，不是 external 的 URL：{}", t.url);
}
```

- [ ] **Step 2：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_sync_in
```

期望：编译失败（`svn_sync_in` 不存在）。

- [ ] **Step 3：实现 `svn_sync_in.rs`**

新建 `src-tauri/src/engine/svn_sync_in.rs`：

```rust
//! `sync_in` 编排（spec §7.3 S0–S11）、冲突出口，以及中断后的继续 / 放弃（§7.6、§13）。
//!
//! # 为什么不走 `trait Vcs`
//!
//! `SvnGitBackend` 的写方法都挂着 `assert_not_syncing`，而本流程全程持锁——走那一层
//! 会被自己的锁挡回来（见 `svn_lock::assert_not_syncing` 的文档）。这里一律直接调
//! `run_git` / `run_svn_code`。
//!
//! # 锁
//!
//! 在入口拿、函数返回时由 Drop 释放。**冲突出口也是一次返回**：用户要编辑、暂存
//! 文件来解决冲突，锁必须放开（spec §7.4）。journal 留着，继续 / 放弃时重新拿锁。
//!
//! # watcher
//!
//! 本模块不碰。挂起与恢复要跨过整次调用，由命令层用 `WatchSuspension` 包住，
//! 结束后再补发一次 `repo-changed`（`commands/svn_sync.rs`）。

use crate::engine::file_lock::{describe, find_blockers};
use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_baseline::{
    create_baseline_commit, git_dir, move_baseline_ref, parse_trailer, read_baseline,
    swap_index_to_tree, TempIndex, Trailer,
};
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_dirty::read_dirty;
use crate::engine::svn_include_check::{is_w_file, normalize_roots};
use crate::engine::svn_init::record_server_tree_into;
use crate::engine::svn_journal::{self, Flow, Journal, Phase};
use crate::engine::svn_lock;
use crate::engine::svn_preflight::{check_targets, read_node_rev, run_preflight};
use crate::engine::svn_sync_check::{
    assert_linear, check_sync_in, current_branch, list_paths, rev_parse, unfinished_error,
    unmerged_paths, SyncInPreview,
};
use crate::engine::svn_sync_stash::{self as wip, RestoreOutcome};
use crate::engine::svn_wc::enumerate_wcs;
use crate::engine::svn_xml::parse_update_status;
use crate::error::{GitError, Result};
use std::path::Path;

/// 执行时带回来的预览事实（§7.2 第 7 条：锁内复核要与用户看到的一致）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInExpect {
    pub orig_head: String,
    pub baseline_oid: String,
    /// 用户在确认框里看过并同意的「白名单外会被碰到的文件」。
    pub confirmed_outside: Vec<String>,
}

/// 一次 `sync_in` / 继续 / 放弃的结局。**冲突不是错误**，是正常出口（spec §7.4）。
///
/// `rename_all_fields` 不能省：只有 `rename_all` 时变体内部字段原样发出
/// （`new_rev`），前端读 `newRev` 拿到 undefined——`svn_sync.rs` 的
/// `SyncVerdict` 上踩过同一个坑。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SyncInOutcome {
    Done { new_rev: u64 },
    Aborted,
    RebaseConflict { files: Vec<String> },
    StashConflict { files: Vec<String>, stash_oid: String },
    StashBlocked { collisions: Vec<String>, stash_oid: String },
}

/// 一次完整的更新。
pub fn run_sync_in(repo: &str, expect: &SyncInExpect) -> Result<SyncInOutcome> {
    let opid = new_opid();
    let _lock = svn_lock::acquire(repo, &opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    // §7.2 第 7 条：锁内复核。预览与确认之间可能过了几分钟，什么都可能变
    let p = check_sync_in(repo)?;
    verify_expect(&p, expect)?;
    let cfg = read_config(repo)?;

    // S0：任何修改之前落盘；origBranch / origHead 此刻就写（§7.3.1）
    let mut j = Journal {
        opid,
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: now_secs(),
        orig_branch: Some(p.orig_branch),
        orig_head: Some(p.orig_head),
        baseline_rev_before: Some(p.baseline_rev),
        baseline_oid_before: Some(p.baseline_oid),
        ..Default::default()
    };
    svn_journal::write(repo, &j)?;

    // S1（S2 的 origBranch 已在 S0 落盘，这里只推进 phase）
    wip::stash_wip(repo, &mut j, &cfg.whitelist)?;
    from_stashed(repo, &cfg, &mut j, true)
}

fn verify_expect(p: &SyncInPreview, e: &SyncInExpect) -> Result<()> {
    if p.orig_head != e.orig_head || p.baseline_oid != e.baseline_oid {
        return Err(GitError::new(
            "sync_in_state_changed",
            "从你确认到现在，分支或基线被改动过（可能是别的 git 工具）。\
             这次没有做任何修改，请重新点一次「更新」。",
        ));
    }
    let unseen: Vec<String> = p
        .outside_changes
        .iter()
        .filter(|f| !e.confirmed_outside.contains(f))
        .cloned()
        .collect();
    if !unseen.is_empty() {
        return Err(GitError::new(
            "sync_in_needs_reconfirm",
            format!(
                "从你确认到现在，又多了几个这次更新会碰到、而 git 管不到的文件：\n{}\n\
                 这次没有做任何修改，请重新点一次「更新」看一下完整清单。",
                list_paths(&unseen)
            ),
        ));
    }
    Ok(())
}

/// S3 起。`fresh` 为真表示本进程一路从 S0 走到这里（Task 7 的回退据此选前提）。
pub(crate) fn from_stashed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    fresh: bool,
) -> Result<SyncInOutcome> {
    // S3
    if current_branch(repo)?.as_deref() != Some(BASELINE_BRANCH) {
        if let Err(e) = checkout_verified(repo, cfg, BASELINE_BRANCH, false) {
            return on_checkout_failed(repo, cfg, j, e);
        }
    }
    // S4：直接问 SVN——revision 比对抓不住「外部手工改了 base 文件」（§7.4）
    let dirty = dirty_w_files(repo, cfg)?;
    if !dirty.is_empty() {
        return on_stale_baseline(repo, cfg, j, fresh, &dirty);
    }
    // S5：`--accept postpone` 是硬要求（§7.4）。成败**不看退出码**（计划 A4）
    let up = run_svn_code(repo, &["update", "--accept", "postpone"])?;
    if up.code != 0 {
        return on_update_failed(repo, cfg, j, &up.stderr);
    }
    let conflicts = svn_conflicts(repo)?;
    if !conflicts.is_empty() {
        return on_update_conflict(repo, cfg, j, &conflicts);
    }
    // S6–S7
    capture_and_publish(repo, cfg, j)?;
    from_baseline_ready(repo, cfg, j)
}

// —— S3 / S4 / S5 的失败出口。本 Task 只如实报错、journal 原样保留；
//    Task 7 把它们换成「回到原分支并取回 WIP」。——

fn on_checkout_failed(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    e: GitError,
) -> Result<SyncInOutcome> {
    Err(e)
}

fn on_stale_baseline(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    _fresh: bool,
    dirty: &[String],
) -> Result<SyncInOutcome> {
    Err(stale_error(dirty))
}

fn on_update_failed(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    stderr: &str,
) -> Result<SyncInOutcome> {
    Err(update_failed_error(stderr))
}

fn on_update_conflict(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    conflicts: &[String],
) -> Result<SyncInOutcome> {
    Err(update_conflict_error(conflicts))
}

fn stale_error(dirty: &[String]) -> GitError {
    GitError::new(
        "sync_in_baseline_stale",
        format!(
            "切到基线之后，下面这些文件在 SVN 看来有修改——基线记的内容与工作副本对不上\
             （多半有人在外部改过）：\n{}\n基线没有被改动。请先处理失同步，再更新。",
            list_paths(dirty)
        ),
    )
}

fn update_failed_error(stderr: &str) -> GitError {
    GitError::new(
        "sync_in_update_failed",
        format!(
            "svn update 失败了（常见原因：VPN 没连、凭据过期）。svn 原话：{}",
            stderr.trim()
        ),
    )
}

fn update_conflict_error(conflicts: &[String]) -> GitError {
    GitError::new(
        "sync_in_update_conflict",
        format!(
            "svn update 产生了冲突：\n{}\n这些文件不归 git 管（或者是目录级的树冲突），\
             本应用没法替你解决，请用 TortoiseSVN 处理。工作副本已经更新到新 revision，\
             但基线没有推进——处理完冲突后，用「重建基线」把基线跟上。",
            list_paths(conflicts)
        ),
    )
}

/// S6–S7：录新基线并发布（spec §7.3 S7、§7.6 录制三阶段）。
pub(crate) fn capture_and_publish(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    j.phase = Phase::CapturingBaseline;
    j.capture_tree = None;
    j.new_baseline_oid = None;
    svn_journal::write(repo, j)?;

    // S6：P1–P4 与 nodeRevs 单值。S5 若没让 nodeRevs 收敛，在这里停（journal 留在
    // capturing-baseline，继续时从这里重来）
    let pre = run_preflight(repo, &cfg.whitelist)?;
    // §6.3.1：S7 从当前 HEAD 树（此刻就是旧基线）起步
    let idx = TempIndex::from_head(repo)?;
    let (tree, _explained) = record_server_tree_into(repo, cfg, &pre, &idx)?;
    // 与 R3c 同一道：录制期间有人在 TortoiseSVN 里 commit / update
    let now = read_node_rev(repo, &cfg.whitelist)?;
    if now != pre.node_rev {
        return Err(GitError::new(
            "sync_in_revision_moved",
            format!(
                "录新基线的过程中工作副本被外部改动了（r{} → r{now}）。基线没有推进，\
                 记录保留；外部操作结束后在顶部横幅上点「继续」。",
                pre.node_rev
            ),
        ));
    }
    // 创建 commit 前记录 captureTree（§7.6）
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree.clone());
    svn_journal::write(repo, j)?;
    let trailer = Trailer {
        url: pre.repos_root,
        rev: pre.node_rev,
        uuid: pre.uuid,
    };
    let commit = create_baseline_commit(repo, &tree, Some(&old), &trailer)?;
    // 移动 ref 前记录 newBaselineOid（§7.6）
    j.new_baseline_oid = Some(commit);
    svn_journal::write(repo, j)?;
    finish_publish(repo, j)
}

/// 发布 + 索引对齐 + `baseline-ready`。重开时也从这里续（§7.6 publishing-baseline）。
pub(crate) fn finish_publish(repo: &str, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    match read_baseline(repo)?.as_deref() {
        Some(cur) if cur == new => {} // 已发布（崩溃恢复时会走到）
        Some(cur) if cur == old => move_baseline_ref(repo, &new, Some(&old))?,
        other => {
            return Err(GitError::new(
                "svn_baseline_moved",
                format!(
                    "基线分支现在是 {other:?}，既不是更新前的 {old}，也不是本次录好的 {new}。\
                     有别的工具动过它，这里停下，不覆盖。"
                ),
            ))
        }
    }
    align_index(repo, &old, &new)?;
    j.phase = Phase::BaselineReady;
    svn_journal::write(repo, j)
}

/// S3 之后真实索引 == 旧基线树，而 HEAD 跟着 `svn/base` 移到了新 commit。
/// 不对齐的话 S8 切分支时，新旧两棵树的差异会被当成「已暂存的改动」带走。
fn align_index(repo: &str, old: &str, new: &str) -> Result<()> {
    let (code, _) = run_git_code(repo, &["diff-index", "--cached", "--quiet", new])?;
    if code == 0 {
        return Ok(()); // 已对齐
    }
    // CAS：索引必须仍是旧基线树，否则有别的 git 操作动过它，宁可停下也不覆盖
    swap_index_to_tree(repo, old, new)
}

/// S8 起：切回原分支、复核待重放区间、rebase。
pub(crate) fn from_baseline_ready(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
) -> Result<SyncInOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    // S8
    match current_branch(repo)?.as_deref() {
        Some(b) if b == BASELINE_BRANCH => checkout_verified(repo, cfg, &orig, false)?,
        Some(b) if b == orig => {}
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是基线分支也不是原分支 `{orig}`"
            )))
        }
    }
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    // S9：对新基线再复核一次（§7.3.2）
    let new_base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    assert_linear(repo, &new_base, &orig_head)?;
    j.phase = Phase::Rebase;
    svn_journal::write(repo, j)?;
    match run_git(repo, &["rebase", "--empty=drop", BASELINE_BRANCH]) {
        Ok(_) => finish_forward(repo, j),
        Err(e) => conflict_or_error(repo, e),
    }
}

/// rebase 非零退出：真冲突就停在冲突态（正常出口），否则按命令错误处理（§7.3.2 末）。
pub(crate) fn conflict_or_error(repo: &str, e: GitError) -> Result<SyncInOutcome> {
    let files = unmerged_paths(repo)?;
    if rebase_in_progress(repo)? && !files.is_empty() {
        return Ok(SyncInOutcome::RebaseConflict { files });
    }
    Err(GitError::new(
        "sync_in_rebase_failed",
        format!(
            "把分支重放到新基线上失败了，而且不是冲突。基线已经推进，记录保留。git 原话：{}",
            e.message.trim()
        ),
    ))
}

/// S10–S11，结局记为 `Done`。
pub(crate) fn finish_forward(repo: &str, j: &mut Journal) -> Result<SyncInOutcome> {
    Ok(match restore_step(repo, j)? {
        RestoreStep::Finished => SyncInOutcome::Done {
            new_rev: baseline_rev(repo)?,
        },
        RestoreStep::Conflict { files, oid } => SyncInOutcome::StashConflict {
            files,
            stash_oid: oid,
        },
        RestoreStep::Blocked { collisions, oid } => SyncInOutcome::StashBlocked {
            collisions,
            stash_oid: oid,
        },
    })
}

pub(crate) enum RestoreStep {
    Finished,
    Conflict { files: Vec<String>, oid: String },
    Blocked { collisions: Vec<String>, oid: String },
}

/// S10 + S11：取回 WIP、drop、删 journal。往前走与往回走（放弃）共用。
pub(crate) fn restore_step(repo: &str, j: &mut Journal) -> Result<RestoreStep> {
    let Some(oid) = j.stash_oid.clone() else {
        svn_journal::remove(repo)?; // 没有 WIP（计划 A7）
        return Ok(RestoreStep::Finished);
    };
    if !j.stash_applied {
        j.phase = Phase::ApplyingStash;
        svn_journal::write(repo, j)?;
        match wip::restore_wip(repo, &oid)? {
            RestoreOutcome::Applied => {}
            RestoreOutcome::Conflicted { files } => {
                j.stash_applied = true;
                svn_journal::write(repo, j)?;
                return Ok(RestoreStep::Conflict { files, oid });
            }
            RestoreOutcome::Blocked { collisions } => {
                return Ok(RestoreStep::Blocked { collisions, oid })
            }
        }
        // apply 成功之后、drop 之前落盘：崩在这里重开时只 drop，不再 apply（计划 A2）
        j.stash_applied = true;
        svn_journal::write(repo, j)?;
    } else {
        let files = unmerged_paths(repo)?;
        if !files.is_empty() {
            return Err(unresolved(&files));
        }
    }
    wip::drop_wip(repo, &j.opid, &oid)?;
    svn_journal::remove(repo)?; // S11；锁由调用方的守卫释放，watcher 由命令层恢复
    Ok(RestoreStep::Finished)
}

/// `git checkout` 之后核对它真的切干净了（计划 A15）。
///
/// 2026-09-25 实测：文件被以 share=Read 打开（Unity 的方式）时，checkout **退出 0**、
/// HEAD 已经切走，而那个文件还是原来的内容——git 只在 stderr 里打一行
/// `unable to unlink old`。所以退出码证明不了任何事，只能事后看。
pub(crate) fn checkout_verified(
    repo: &str,
    cfg: &SvnConfig,
    branch: &str,
    force: bool,
) -> Result<()> {
    let mut args: Vec<&str> = vec!["checkout"];
    if force {
        args.push("-f");
    }
    args.push(branch);
    let ran = run_git(repo, &args);
    let on_target = current_branch(repo)?.as_deref() == Some(branch);
    let stale = tracked_changes(repo, cfg)?;
    if on_target && stale.is_empty() {
        return Ok(());
    }
    let files = if stale.is_empty() {
        String::new()
    } else {
        format!("\n下面这些文件没能写进去：\n{}", list_paths(&stale))
    };
    let git_said = match ran {
        Err(e) => format!("\ngit 原话：{}", e.message.trim()),
        Ok(_) => String::new(),
    };
    let root = Path::new(repo);
    Err(GitError::new(
        "sync_in_checkout_failed",
        format!(
            "切到分支 `{branch}` 没有成功，多半是有程序占着文件（Unity / IDE）。{files}\n{}{git_said}",
            describe(root, &find_blockers(root, 5))
        ),
    ))
}

/// 白名单内被跟踪文件相对 HEAD 的改动（不含未跟踪）。
fn tracked_changes(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(Vec::new()); // 空 pathspec 会变成「全部」（§12.2）
    }
    let mut args: Vec<&str> = vec!["diff", "--name-only", "-z", "HEAD", "--"];
    args.extend(roots.iter().map(String::as_str));
    Ok(run_git(repo, &args)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// S4：白名单内的 W-file 在 SVN 看来有没有修改。`?` / ignored 不算——它们不是
/// 「base 文件被改了」，而是 SVN 不认识的文件。
pub(crate) fn dirty_w_files(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    let wcs = enumerate_wcs(repo, &cfg.whitelist)?;
    let targets = check_targets(&roots, &wcs);
    Ok(read_dirty(repo, &targets)?
        .into_iter()
        .filter(|e| is_w_file(&e.path, &roots, &cfg.extensions))
        .filter(|e| {
            !matches!(
                e.item.as_str(),
                "normal" | "unversioned" | "ignored" | "external" | "none"
            ) || e.props == "modified"
        })
        .map(|e| e.path)
        .collect())
}

/// S5 之后的冲突（计划 A4：`svn update` 有冲突时仍然退出 0）。
fn svn_conflicts(repo: &str) -> Result<Vec<String>> {
    let xml = run_svn(repo, &["status", "--xml"])?;
    Ok(parse_update_status(&xml)?
        .entries
        .into_iter()
        .filter(|e| e.item == "conflicted" || e.tree_conflicted)
        .map(|e| e.path)
        .collect())
}

pub(crate) fn rebase_in_progress(repo: &str) -> Result<bool> {
    let d = git_dir(repo)?;
    Ok(d.join("rebase-merge").exists() || d.join("rebase-apply").exists())
}

fn baseline_rev(repo: &str) -> Result<u64> {
    let msg = run_git(
        repo,
        &["log", "-1", "--format=%B", &format!("refs/heads/{BASELINE_BRANCH}")],
    )?;
    parse_trailer(&msg)
        .map(|t| t.rev)
        .ok_or_else(|| GitError::new("svn_baseline", "基线 commit 上读不到 git-svn-id"))
}

pub(crate) fn need(v: &Option<String>, field: &str) -> Result<String> {
    v.clone().ok_or_else(|| {
        GitError::new(
            "svn_journal_corrupt",
            format!("同步记录缺字段 `{field}`，不知道该往哪里走。请联系开发者，不要删记录文件。"),
        )
    })
}

pub(crate) fn state_changed(what: &str) -> GitError {
    GitError::new(
        "sync_in_state_changed",
        format!("{what}。有别的 git 操作动过仓库，这里停下，记录与 stash 原样保留。"),
    )
}

pub(crate) fn unresolved(files: &[String]) -> GitError {
    GitError::new(
        "sync_in_unresolved",
        format!(
            "还有冲突没解决：\n{}\n改好之后把它们暂存（git add），再点「继续」。",
            list_paths(files)
        ),
    )
}

fn new_opid() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 只含十六进制字符：它会进 stash message 与 ref 名（`refs/mygitclient/sync/<opid>`）
    format!("{nanos:x}{:x}", std::process::id())
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
```

`src-tauri/src/engine/mod.rs` 加 `pub mod svn_sync_in;`。

> ⚠️ `clippy` 可能对 `pub(crate)` 但本 Task 尚无调用方的函数报 `dead_code`（`rebase_in_progress` 之外，`unresolved` 等都有调用方；`need` / `state_changed` 也有）。若真报了，**不要加 `#[allow]`**：检查是不是漏写了调用；确实是给 Task 6 预留的，就把它推迟到 Task 6 再写。

- [ ] **Step 4：跑测试，确认通过**

```powershell
cargo test --test svn_sync_in
```

期望：11 条全部 PASS。任何一条失败，先在 fixture 上用 `git` / `svn` 手工复现同一形状，看是实现错了还是计划对 git 行为的假设错了（`AGENTS.md` §6.2）；后者记进决策日志「计划的错误」。

- [ ] **Step 5：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `restore_step` 里 Conflicted 分支不写 `stash_applied = true` | `a_stash_conflict_keeps_the_stash_and_marks_it_applied` |
| `from_baseline_ready` 的 rebase 去掉 `--empty=drop` | `a_linear_round_trip_is_not_reported_as_lost` |
| `capture_and_publish` 改用 `TempIndex::empty` | 无（两者在这些用例上等价，见 Task 4）——**记为「覆盖不到」**，由 Task 4 的对照测试守着 |
| `align_index` 直接 `return Ok(())` | `a_plain_update_advances_the_baseline_and_replays_the_branch`（S8 会把新旧树的差异带到原分支上，或切分支失败） |
| `verify_expect` 去掉 outside 比对 | `new_outside_changes_since_the_preview_require_reconfirmation` |

- [ ] **Step 6：全量检查、更新决策日志、提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): sync_in 编排 S0–S11，冲突停在可解决状态并释放锁"
```

---

## Task 6：继续 / 放弃（含「回到原分支」）

spec §7.6：「打开仓库时发现它存在 → 界面顶部常驻『上次同步未完成』，提供继续 / 放弃两个入口」。2b 只做了检测与展示，本 Task 做执行。每个 phase 的含义与收尾方式照 §7.6 两张表，外加计划 A2、A5、A10、A11、A13、A14。

| phase | 继续 | 放弃 |
|---|---|---|
| `start` | 删 journal（A10） | 删 journal |
| `stashing` | §7.3.1 判定树 → 按 `stashed` 继续 | 判定树 → 回原分支 + 取回 WIP |
| `stashed` | 复核（A11）→ 从 S3 接着走 | 回原分支（A5）+ 取回 WIP |
| `capturing-baseline` | 复核在旧基线上 → 重新录制 | 同上 |
| `publishing-baseline` | 有 `newBaselineOid`：补发布 / 补对齐；没有：重新录制 | 同上 |
| `baseline-ready` | 从 S8 接着走 | 同上 |
| `rebase` | 复核是本次的 rebase、无未合并 → `rebase --continue`；已在外面走完则复核落在新基线上 | `rebase --abort` → 取回 WIP（基线保持新的，分支落后于它） |
| `applying-stash` | `stashApplied` 为假：再 apply（先查撞名）；为真：确认无未合并 → drop | 只删钉住的 ref 与 journal，stash 条目留给用户 |

**Files:**
- Modify: `src-tauri/src/engine/svn_sync_in.rs`
- Test: `src-tauri/tests/svn_sync_in.rs`

**Interfaces:**
- Consumes: Task 5 的全部内部函数；Task 3 `settle_stashing` / `unpin`。
- Produces:
  - `svn_sync_in::continue_sync(repo: &str) -> Result<SyncInOutcome>`
  - `svn_sync_in::abort_sync(repo: &str) -> Result<SyncInOutcome>`
  - 模块内部（Task 7 要用）：`Disk { GitOnly, MaybeUpdated }`、`back_to_orig(repo, cfg, j, disk) -> Result<RestoreStep>`

- [ ] **Step 1：写失败的测试**

`src-tauri/tests/svn_sync_in.rs` 顶部 `use` 区补：

```rust
use app_lib::engine::svn_baseline::{create_baseline_commit, TempIndex, Trailer};
use app_lib::engine::svn_config::read_config;
use app_lib::engine::svn_init::record_server_tree_into;
use app_lib::engine::svn_journal::{Flow, Journal};
use app_lib::engine::svn_preflight::run_preflight;
use app_lib::engine::svn_sync_in::{abort_sync, continue_sync};
use app_lib::engine::svn_sync_stash::stash_wip;
```

文件末尾追加（造「停在某个 phase」的现场一律用 engine 自己的零件，形状与 `run_sync_in` 写出来的一致）：

```rust
// —— 造现场 ——

/// 同事改了 A.cs；你在 work 上提交过 B.cs，手上还有一份未提交的 B.cs 改动。
fn with_wip(fx: &SvnFixture) {
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(fx, "Code/B.cs", "b-local\n");
    commit(fx, "local");
    write(fx, "Code/B.cs", "b-wip\n");
}

/// 刚走完 S0。
fn s0(fx: &SvnFixture, opid: &str) -> Journal {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    let j = Journal {
        opid: opid.into(),
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: 1,
        orig_branch: Some(p.orig_branch),
        orig_head: Some(p.orig_head),
        baseline_rev_before: Some(p.baseline_rev),
        baseline_oid_before: Some(p.baseline_oid),
        ..Default::default()
    };
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    j
}

/// 刚走完 S1：WIP 已压栈，phase = stashed。
fn s1(fx: &SvnFixture, opid: &str) -> Journal {
    let mut j = s0(fx, opid);
    stash_wip(&fx.wc_str(), &mut j, &["Code/".to_string()]).unwrap();
    j
}

/// 刚走完 S5：HEAD 在 svn/base、工作副本已更新，phase = capturing-baseline。
fn s5(fx: &SvnFixture, opid: &str) -> Journal {
    let mut j = s1(fx, opid);
    git(fx, &["checkout", "-q", "svn/base"]);
    svn_fixtures::svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    j.phase = Phase::CapturingBaseline;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    j
}

/// 按 S7 的做法录出新基线 commit（不发布）。
fn built_commit(fx: &SvnFixture, j: &Journal, rev_override: Option<u64>) -> (String, String) {
    let root = fx.wc_str();
    let cfg = read_config(&root).unwrap();
    let pre = run_preflight(&root, &cfg.whitelist).unwrap();
    let (tree, _) =
        record_server_tree_into(&root, &cfg, &pre, &TempIndex::from_head(&root).unwrap()).unwrap();
    let t = Trailer {
        url: pre.repos_root,
        rev: rev_override.unwrap_or(pre.node_rev),
        uuid: pre.uuid,
    };
    let c = create_baseline_commit(&root, &tree, j.baseline_oid_before.as_deref(), &t).unwrap();
    (tree, c)
}

fn rebase_conflict(fx: &SvnFixture) {
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(fx, "Code/A.cs", "a-local\n");
    commit(fx, "local A");
    write(fx, "Code/B.cs", "b-wip\n");
    assert!(matches!(sync(fx), SyncInOutcome::RebaseConflict { .. }));
}

// —— start ——

#[test]
fn continue_from_start_just_clears_the_record() {
    let fx = hybrid_with_colleague();
    s0(&fx, "op-s0");
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(phase(&fx), None);
}

#[test]
fn abort_from_start_just_clears_the_record() {
    let fx = hybrid_with_colleague();
    s0(&fx, "op-s0a");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(phase(&fx), None);
}

// —— stashing（必测 #16 的 S1 部分、#32）——

#[test]
fn continue_from_stashing_with_a_landed_push() {
    // 崩在 `git stash push` 之后、写 stashOid 之前：journal 只说 stashing
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s0(&fx, "op-landed");
    j.phase = Phase::Stashing;
    j.stash_ref_before = None;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(&fx, &["stash", "push", "-u", "-m", "mygitclient-sync:op-landed", "--", "Code"]);

    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n", "不静默丢掉那条匿名 stash");
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn abort_from_stashing_that_never_landed_changes_nothing() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s0(&fx, "op-never");
    j.phase = Phase::Stashing;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

// —— stashed ——

#[test]
fn continue_from_stashed_on_the_original_branch() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s1(&fx, "op-st1");
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
}

#[test]
fn continue_from_stashed_on_the_baseline_branch() {
    // 崩在 S3 之后
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s1(&fx, "op-st2");
    git(&fx, &["checkout", "-q", "svn/base"]);
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
}

#[test]
fn continue_from_stashed_refuses_changes_made_after_the_crash() {
    // A11：锁在崩溃后不存在，用户可能已经在工作区里继续改了
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s1(&fx, "op-st3");
    write(&fx, "Code/A.cs", "edited-after-crash\n");
    assert_eq!(continue_sync(&fx.wc_str()).unwrap_err().code, "sync_in_state_changed");
    assert_eq!(phase(&fx), Some(Phase::Stashed), "记录原样保留");
    assert_eq!(read(&fx, "Code/A.cs"), "edited-after-crash\n");
}

#[test]
fn abort_from_stashed_restores_the_wip() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let j = s1(&fx, "op-st4");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
}

// —— capturing-baseline ——

#[test]
fn continue_from_capturing_baseline() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s5(&fx, "op-cap");
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(baseline_rev(&fx), 2);
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
}

#[test]
fn abort_after_the_update_returns_to_the_original_branch() {
    // A5：S5 已经做完，SVN 退不回去；回到原分支、放回 WIP，基线不动 → 失同步
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let j = s5(&fx, "op-cap-a");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(read(&fx, "Code/A.cs"), "a1\n", "工作区回到原分支的内容");
    assert_eq!(baseline_rev(&fx), 1, "基线没发布");
    assert!(
        matches!(
            check_sync(&fx.wc_str()).unwrap().verdict,
            SyncVerdict::Diverged { .. }
        ),
        "工作副本已在 r2、基线还是 r1——如实报失同步，引导重建基线"
    );
}

#[test]
fn abort_after_the_update_refuses_to_overwrite_unknown_changes() {
    // A5 的护栏：W-file 在 svn 看来不干净，说明磁盘上不全是服务器内容，不能 -f
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s5(&fx, "op-cap-g");
    write(&fx, "Code/A.cs", "someone-edited\n");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap_err().code, "sync_in_unsafe_to_return");
    assert_eq!(read(&fx, "Code/A.cs"), "someone-edited\n", "一个字节都没覆盖");
    assert_eq!(phase(&fx), Some(Phase::CapturingBaseline));
}

// —— publishing-baseline（必测 #73 的 S7 部分）——

#[test]
fn continue_from_publishing_with_the_commit_built_but_not_published() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s5(&fx, "op-pub1");
    let (tree, c) = built_commit(&fx, &j, None);
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c.clone());
    svn_journal::write(&fx.wc_str(), &j).unwrap();

    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), c, "发布的就是记下来的那个 commit，没有重录");
}

#[test]
fn continue_from_publishing_after_the_ref_already_moved() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s5(&fx, "op-pub2");
    let (tree, c) = built_commit(&fx, &j, None);
    let old = j.baseline_oid_before.clone().unwrap();
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c.clone());
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &c, &old]); // 崩在移动 ref 之后

    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), c);
}

#[test]
fn continue_from_publishing_refuses_a_baseline_moved_by_someone_else() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s5(&fx, "op-pub3");
    let (tree, c) = built_commit(&fx, &j, None);
    let (_, other) = built_commit(&fx, &j, Some(99));
    let old = j.baseline_oid_before.clone().unwrap();
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c);
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &other, &old]);

    assert_eq!(continue_sync(&fx.wc_str()).unwrap_err().code, "svn_baseline_moved");
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), other, "别人的不被覆盖");
    assert_eq!(phase(&fx), Some(Phase::PublishingBaseline));
}

// —— rebase（必测 #69 的 S9 部分）——

#[test]
fn continue_after_resolving_a_rebase_conflict() {
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    write(&fx, "Code/A.cs", "a-resolved\n");
    git(&fx, &["add", "Code/A.cs"]);
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/A.cs"), "a-resolved\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn continue_with_unresolved_files_is_refused() {
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    let err = continue_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unresolved");
    assert!(err.message.contains("Code/A.cs"));
    assert_eq!(phase(&fx), Some(Phase::Rebase));
}

#[test]
fn abort_a_rebase_conflict_goes_back_to_before() {
    let fx = hybrid_with_colleague();
    let before_head = {
        fx.colleague_commits("Code/A.cs", "a-server\n");
        write(&fx, "Code/A.cs", "a-local\n");
        commit(&fx, "local A");
        write(&fx, "Code/B.cs", "b-wip\n");
        git(&fx, &["rev-parse", "HEAD"])
    };
    assert!(matches!(sync(&fx), SyncInOutcome::RebaseConflict { .. }));
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(read(&fx, "Code/A.cs"), "a-local\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 2, "基线保持新的；分支落后于它，是 Phase 1 就有的合法状态");
    assert!(!fx.wc().join(".git/rebase-merge").exists());
}

#[test]
fn continue_refuses_when_the_branch_moved_during_a_conflict() {
    // Review Focus 4 / A14：冲突期间锁是放开的
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    git(&fx, &["rebase", "--abort"]);
    git(&fx, &["checkout", "-q", "-b", "elsewhere"]);
    assert_eq!(continue_sync(&fx.wc_str()).unwrap_err().code, "sync_in_state_changed");
    assert_eq!(phase(&fx), Some(Phase::Rebase));
    assert_eq!(our_stashes(&fx), 1, "WIP 还在");
}

// —— applying-stash ——

#[test]
fn continue_after_resolving_a_stash_conflict_drops_the_stash() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(&fx, "Code/A.cs", "a-wip\n");
    assert!(matches!(sync(&fx), SyncInOutcome::StashConflict { .. }));
    write(&fx, "Code/A.cs", "a-merged\n");
    git(&fx, &["add", "Code/A.cs"]);
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(our_stashes(&fx), 0);
    assert_eq!(read(&fx, "Code/A.cs"), "a-merged\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn continue_after_moving_a_colliding_file_applies_the_stash() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "server\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    write(&fx, "Code/N.cs", "mine\n");
    assert!(matches!(sync(&fx), SyncInOutcome::StashBlocked { .. }));
    std::fs::rename(fx.wc().join("Code/N.cs"), fx.wc().join("Code/N.server.cs")).unwrap();
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/N.cs"), "mine\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn abort_while_applying_the_stash_keeps_the_stash_for_the_user() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "server\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    write(&fx, "Code/N.cs", "mine\n");
    assert!(matches!(sync(&fx), SyncInOutcome::StashBlocked { .. }));
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(our_stashes(&fx), 1, "备份留在 stash 列表里给用户");
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/"]), "", "钉住的 ref 已删");
    assert_eq!(phase(&fx), None);
}

// —— 边界 ——

#[test]
fn a_sync_out_journal_is_not_touched() {
    // A13
    let fx = hybrid_with_colleague();
    let j = Journal {
        opid: "out".into(),
        flow: Flow::SyncOut,
        phase: Phase::Prepare,
        started_at: 1,
        ..Default::default()
    };
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    assert_eq!(continue_sync(&fx.wc_str()).unwrap_err().code, "sync_out_not_supported");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap_err().code, "sync_out_not_supported");
    assert_eq!(phase(&fx), Some(Phase::Prepare));
}

#[test]
fn continue_without_a_record_says_so() {
    let fx = hybrid_with_colleague();
    assert_eq!(continue_sync(&fx.wc_str()).unwrap_err().code, "sync_no_unfinished");
}
```

- [ ] **Step 2：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_sync_in
```

期望：编译失败（`continue_sync` / `abort_sync` 不存在）。

- [ ] **Step 3：实现**

`src-tauri/src/engine/svn_sync_in.rs`：`use` 区的 `git_cli` 改成 `use crate::engine::git_cli::{run_git, run_git_code, run_git_env};`，文件末尾追加：

```rust
// —— 继续 / 放弃（spec §7.6：「提供继续 / 放弃两个入口」）——

/// 往回走的两种前提（计划 A5 / A12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disk {
    /// S5 还没跑、且是本进程一路走到这里：磁盘上被跟踪的内容全是 git 自己写的，
    /// `checkout -f` 只会还原 git 写下的字节。
    GitOnly,
    /// S5 可能跑过，或从崩溃里恢复、不知道跑没跑：磁盘上可能是服务器内容，
    /// `-f` 之前必须先确认那确实是服务器内容（能从服务器取回）。
    MaybeUpdated,
}

/// 从中断处接着走。
pub fn continue_sync(repo: &str) -> Result<SyncInOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    match j.phase {
        // A10：`start` 的定义就是「还没动任何东西」
        Phase::Start => {
            svn_journal::remove(repo)?;
            Ok(SyncInOutcome::Aborted)
        }
        Phase::Stashing => {
            wip::settle_stashing(repo, j)?;
            resume_stashed(repo, &cfg, j)
        }
        Phase::Stashed => resume_stashed(repo, &cfg, j),
        Phase::CapturingBaseline => {
            require_on_old_baseline(repo, j)?;
            capture_and_publish(repo, &cfg, j)?;
            from_baseline_ready(repo, &cfg, j)
        }
        Phase::PublishingBaseline => {
            if j.new_baseline_oid.is_some() {
                // commit 已建：只补发布与索引对齐，不重建另一个 commit（§7.6）
                require_on_branch(repo, BASELINE_BRANCH)?;
                finish_publish(repo, j)?;
            } else {
                // 没有新 OID = 尚未发布，重新验证前提、重建候选索引（§7.6）
                require_on_old_baseline(repo, j)?;
                capture_and_publish(repo, &cfg, j)?;
            }
            from_baseline_ready(repo, &cfg, j)
        }
        Phase::BaselineReady => from_baseline_ready(repo, &cfg, j),
        Phase::Rebase => continue_rebase(repo, j),
        Phase::ApplyingStash => finish_forward(repo, j),
        _ => Err(sync_out_unsupported()),
    }
}

/// 放弃这次同步：回到更新前的分支、放回 WIP。SVN 侧已做的不退回（计划 A5）。
pub fn abort_sync(repo: &str) -> Result<SyncInOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    let step = match j.phase {
        Phase::Start => {
            svn_journal::remove(repo)?;
            return Ok(SyncInOutcome::Aborted);
        }
        Phase::Stashing => {
            wip::settle_stashing(repo, j)?;
            back_to_orig(repo, &cfg, j, Disk::MaybeUpdated)?
        }
        Phase::Stashed
        | Phase::CapturingBaseline
        | Phase::PublishingBaseline
        | Phase::BaselineReady => back_to_orig(repo, &cfg, j, Disk::MaybeUpdated)?,
        Phase::Rebase => {
            if rebase_in_progress(repo)? {
                run_git(repo, &["rebase", "--abort"])?;
            }
            back_to_orig(repo, &cfg, j, Disk::MaybeUpdated)?
        }
        Phase::ApplyingStash => {
            // 放弃接管：stash 条目原样留在列表里（message 带 opid，认得出来），
            // 只删钉住的 ref 与 journal
            wip::unpin(repo, &j.opid)?;
            svn_journal::remove(repo)?;
            return Ok(SyncInOutcome::Aborted);
        }
        _ => return Err(sync_out_unsupported()),
    };
    Ok(match step {
        RestoreStep::Finished => SyncInOutcome::Aborted,
        RestoreStep::Conflict { files, oid } => SyncInOutcome::StashConflict {
            files,
            stash_oid: oid,
        },
        RestoreStep::Blocked { collisions, oid } => SyncInOutcome::StashBlocked {
            collisions,
            stash_oid: oid,
        },
    })
}

/// 回到原分支并取回 WIP（spec §13「回滚到」一列；计划 A5 / A12 / A15）。
pub(crate) fn back_to_orig(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    disk: Disk,
) -> Result<RestoreStep> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let branch = current_branch(repo)?;
    if branch.as_deref() != Some(orig.as_str()) {
        if branch.as_deref() != Some(BASELINE_BRANCH) {
            return Err(state_changed(&format!(
                "HEAD 在 {branch:?} 上，既不是原分支 `{orig}` 也不是基线分支"
            )));
        }
        if disk == Disk::MaybeUpdated {
            // A5：能 -f 的前提是 W-file 在 svn 看来全部干净 = 磁盘上就是服务器内容
            let dirty = dirty_w_files(repo, cfg)?;
            if !dirty.is_empty() {
                return Err(GitError::new(
                    "sync_in_unsafe_to_return",
                    format!(
                        "要回到分支 `{orig}` 就得覆盖工作区里的文件，而下面这些文件的内容\
                         既不是 git 写的、也不是服务器上的——不知道是谁改的，不能覆盖：\n{}\n\
                         记录已保留。请先确认这些改动（需要的话自己备份），再点「放弃」。",
                        list_paths(&dirty)
                    ),
                ));
            }
        }
        checkout_verified(repo, cfg, &orig, true)?;
    }
    // HEAD 仍在原分支上（S3 失败时 git 整体拒绝、一个字节没写——计划 A15）
    // 或刚切回来：都要求分支本身没被动过
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    restore_step(repo, j)
}

/// 拿锁并在锁内重读 journal。
fn take_over(repo: &str) -> Result<(svn_lock::SyncLock, Journal)> {
    let first = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    if first.flow != Flow::SyncIn {
        return Err(sync_out_unsupported());
    }
    let lock = svn_lock::acquire(repo, &first.opid)?;
    // 拿锁之前那份只用来取 opid；两次读之间可能被另一个进程推进过
    let j = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    Ok((lock, j))
}

/// A11：从 `stashed` 接着走之前，确认中断期间没人动过。
fn resume_stashed(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<SyncInOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let base = need(&j.baseline_oid_before, "baselineOidBefore")?;
    match current_branch(repo)?.as_deref() {
        Some(b) if b == orig => {
            if rev_parse(repo, "HEAD")? != orig_head {
                return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
            }
            if !whitelist_clean(repo, cfg)? {
                return Err(state_changed(
                    "中断之后白名单里又有了新的改动（它们不在本次同步的 stash 里）",
                ));
            }
        }
        Some(b) if b == BASELINE_BRANCH => {
            if rev_parse(repo, "HEAD")? != base {
                return Err(state_changed("基线分支的位置变了"));
            }
        }
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是原分支 `{orig}` 也不是基线分支"
            )))
        }
    }
    from_stashed(repo, cfg, j, false)
}

fn continue_rebase(repo: &str, j: &mut Journal) -> Result<SyncInOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    if rebase_in_progress(repo)? {
        // A14：冲突期间锁是放开的，先确认进行中的正是本次同步发起的 rebase
        let want = format!("refs/heads/{orig}");
        if rebase_head_name(repo)?.as_deref() != Some(want.as_str()) {
            return Err(state_changed("进行中的 rebase 不是本次同步发起的那一个"));
        }
        let files = unmerged_paths(repo)?;
        if !files.is_empty() {
            return Err(unresolved(&files));
        }
        // GIT_EDITOR=true：非交互地接受 git 给的提交信息（2026-09-25 实测可行；
        // 冲突被解成与新基线相同时，--empty=drop 让这一笔直接丢掉，退出 0）
        if let Err(e) = run_git_env(repo, &["rebase", "--continue"], &[("GIT_EDITOR", "true")]) {
            return conflict_or_error(repo, e);
        }
    }
    // 不论是我们续完的，还是用户在外面自己走完的，都必须落在同一个状态上
    if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
        return Err(state_changed(&format!("HEAD 不在原分支 `{orig}` 上")));
    }
    let base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    if !is_ancestor(repo, &base, "HEAD")? {
        return Err(state_changed(
            "原分支还没有落到新基线上（rebase 可能在外面被中止了）；\
             要回到更新之前，请点「放弃」",
        ));
    }
    finish_forward(repo, j)
}

/// `.git/rebase-merge/head-name`（或 `rebase-apply/`）：rebase 的目标分支。
fn rebase_head_name(repo: &str) -> Result<Option<String>> {
    let d = git_dir(repo)?;
    for sub in ["rebase-merge", "rebase-apply"] {
        let p = d.join(sub).join("head-name");
        if p.exists() {
            return Ok(Some(std::fs::read_to_string(p)?.trim().to_string()));
        }
    }
    Ok(None)
}

fn is_ancestor(repo: &str, a: &str, b: &str) -> Result<bool> {
    let (code, _) = run_git_code(repo, &["merge-base", "--is-ancestor", a, b])?;
    match code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(GitError::new(
            "sync_in_state_changed",
            format!("判断 {a} 是否在 {b} 的历史里失败了（merge-base 退出 {code}）"),
        )),
    }
}

fn whitelist_clean(repo: &str, cfg: &SvnConfig) -> Result<bool> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(false); // 空 pathspec 会变成「全部」（§12.2），宁可判不干净
    }
    let mut args: Vec<&str> = vec!["status", "--porcelain", "--"];
    args.extend(roots.iter().map(String::as_str));
    Ok(run_git(repo, &args)?.trim().is_empty())
}

fn require_on_branch(repo: &str, name: &str) -> Result<()> {
    if current_branch(repo)?.as_deref() != Some(name) {
        return Err(state_changed(&format!("HEAD 不在 `{name}` 上")));
    }
    Ok(())
}

fn require_on_old_baseline(repo: &str, j: &Journal) -> Result<()> {
    require_on_branch(repo, BASELINE_BRANCH)?;
    let base = need(&j.baseline_oid_before, "baselineOidBefore")?;
    if rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))? != base {
        return Err(state_changed("基线分支已经不是更新前的位置"));
    }
    Ok(())
}

fn no_unfinished() -> GitError {
    GitError::new("sync_no_unfinished", "没有未收尾的同步记录，不需要继续或放弃。")
}

fn sync_out_unsupported() -> GitError {
    GitError::new(
        "sync_out_not_supported",
        "这条记录是「交付」流程留下的，本版本还不能替你收尾（Phase 3 提供）。\
         在那之前请不要手工去动 stash 或分支指针。",
    )
}
```

- [ ] **Step 4：跑测试，确认通过**

```powershell
cargo test --test svn_sync_in
```

- [ ] **Step 5：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `back_to_orig` 去掉 `dirty_w_files` 断言 | `abort_after_the_update_refuses_to_overwrite_unknown_changes` |
| `resume_stashed` 去掉 `whitelist_clean` | `continue_from_stashed_refuses_changes_made_after_the_crash` |
| `continue_rebase` 去掉 `is_ancestor` 复核 | `continue_refuses_when_the_branch_moved_during_a_conflict`（若它仍因 `current_branch` 那道先红，把用例改成「外面 abort 但留在 work 上」再验一次，并记进决策日志） |
| `finish_publish` 的 `cur == new` 分支改成再 `move_baseline_ref` | `continue_from_publishing_after_the_ref_already_moved` |
| `abort_sync` 的 `ApplyingStash` 分支改成 `drop_wip` | `abort_while_applying_the_stash_keeps_the_stash_for_the_user` |

- [ ] **Step 6：全量检查、更新决策日志、提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): sync_in 中断后的继续与放弃，逐 phase 收尾"
```

---

## Task 7：S3 / S4 / S5 失败回退

spec §13 前五行：S3 checkout 失败、S4 校验不过、S5 网络 / 树冲突 / 白名单外冲突，都要「切回 ORIG_BRANCH + stash apply」。本 Task 把 Task 5 留下的四个 `on_*` 换成真正的回退。

**Files:**
- Modify: `src-tauri/src/engine/svn_sync_in.rs`
- Test: `src-tauri/tests/svn_sync_in.rs`

**Interfaces:**
- Consumes: Task 6 `back_to_orig` / `Disk`。
- Produces: 无新的对外接口；失败时 `Err` 的 message 末尾追加回退结果。

- [ ] **Step 1：写失败的测试**

`src-tauri/tests/svn_sync_in.rs` 末尾追加：

```rust
// —— S3 / S4 / S5 失败回退（spec §13 前五行）——

#[test]
fn a_lying_baseline_is_caught_at_s4_and_rolled_back() {
    // S4 是双保险（§7.4）：revision 对得上，内容却对不上
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let old = git(&fx, &["rev-parse", "svn/base"]);
    let idx = TempIndex::from_head(&root).unwrap();
    idx.replace_blob(&root, "Code/A.cs", b"lie\n").unwrap();
    let tree = idx.write_tree(&root).unwrap();
    let t = parse_trailer(&git(&fx, &["log", "-1", "--format=%B", "svn/base"])).unwrap();
    let lie = create_baseline_commit(&root, &tree, Some(&old), &t).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &lie, &old]);
    write(&fx, "Code/B.cs", "b-wip\n");
    let head = git(&fx, &["rev-parse", "HEAD"]);

    let err = run_sync_in(&root, &expect(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_in_baseline_stale");
    assert!(err.message.contains("Code/A.cs"), "{}", err.message);
    assert!(err.message.contains("已回到分支 `work`"), "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(read(&fx, "Code/A.cs"), "a1\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), lie, "失败只回滚自己（§12.4），不去修基线");
}

#[test]
fn update_failure_returns_to_the_original_branch() {
    // Review Focus 3：从「S3 之后崩溃」继续，此时服务器不可达
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let j = s1(&fx, "op-net");
    git(&fx, &["checkout", "-q", "svn/base"]);
    let repo = fx.repo();
    let gone = fx.path().join("repo-gone");
    std::fs::rename(&repo, &gone).unwrap();
    let r = continue_sync(&fx.wc_str());
    std::fs::rename(&gone, &repo).unwrap();

    let err = r.unwrap_err();
    assert_eq!(err.code, "sync_in_update_failed");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 1);
    assert_eq!(phase(&fx), None);
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

#[test]
fn an_update_conflict_outside_git_rolls_back_and_points_to_tortoisesvn() {
    // §13 第 5 行 / §7.2.1：用户确认过的美术文件在 update 时冲突
    let fx = hybrid_with_colleague();
    write(&fx, "Art/x.psd", "psd-local\n");
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    let e = expect(&fx);
    assert_eq!(e.confirmed_outside, vec!["Art/x.psd".to_string()]);

    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_update_conflict");
    assert!(err.message.contains("Art/x.psd") && err.message.contains("TortoiseSVN"));
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 1, "基线不推进");
    assert_eq!(phase(&fx), None);
    let st = app_lib::engine::svn_cli::run_svn(&fx.wc_str(), &["status", "Art/x.psd"]).unwrap();
    assert!(st.starts_with('C'), "冲突现场原样留给 TortoiseSVN：{st}");
}

#[cfg(windows)]
#[test]
fn a_file_held_open_during_checkout_is_named_and_rolled_back() {
    // Review Focus 2 / A15：Unity 式的 share=Read 占用下，git checkout 退出 0
    // 却没把文件写进去。必须指名文件、回到原分支
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-local\n");
    commit(&fx, "local B"); // 切到 svn/base 必须改写 B.cs
    write(&fx, "Code/New.cs", "wip\n");
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let e = expect(&fx);

    const FILE_SHARE_READ: u32 = 0x1;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(fx.wc().join("Code/B.cs"))
        .unwrap();
    let r = run_sync_in(&fx.wc_str(), &e);
    drop(held);

    let err = r.unwrap_err();
    assert_eq!(err.code, "sync_in_checkout_failed");
    assert!(err.message.contains("Code/B.cs"), "{}", err.message);
    assert!(err.message.contains("已回到分支 `work`"), "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(read(&fx, "Code/B.cs"), "b-local\n");
    assert_eq!(read(&fx, "Code/New.cs"), "wip\n");
    assert_eq!(phase(&fx), None);
}
```

`use` 区补 `use app_lib::engine::svn_baseline::parse_trailer;`（若 Task 5 已引入则跳过）。

- [ ] **Step 2：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_sync_in a_lying_baseline update_failure an_update_conflict a_file_held_open
```

期望：四条都失败——错误码对，但 message 里没有「已回到分支」，且 journal 还在、HEAD 停在 `svn/base`。

- [ ] **Step 3：实现**

把 Task 5 的四个 `on_*` 与那句注释（「本 Task 只如实报错……」）整段换成：

```rust
// —— S3 / S4 / S5 的失败出口：回到原分支并取回 WIP（spec §13 前五行）——

fn on_checkout_failed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    e: GitError,
) -> Result<SyncInOutcome> {
    // S3 之前 S5 必然没跑：磁盘上被跟踪的内容全是 git 自己写的（A12）
    rolled_back(repo, cfg, j, Disk::GitOnly, e)
}

fn on_stale_baseline(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    fresh: bool,
    dirty: &[String],
) -> Result<SyncInOutcome> {
    // 本进程一路走到这里才敢按 GitOnly；从崩溃里恢复时不知道 S5 跑没跑（A12）
    let disk = if fresh { Disk::GitOnly } else { Disk::MaybeUpdated };
    rolled_back(repo, cfg, j, disk, stale_error(dirty))
}

fn on_update_failed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    stderr: &str,
) -> Result<SyncInOutcome> {
    rolled_back(repo, cfg, j, Disk::MaybeUpdated, update_failed_error(stderr))
}

fn on_update_conflict(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    conflicts: &[String],
) -> Result<SyncInOutcome> {
    rolled_back(repo, cfg, j, Disk::MaybeUpdated, update_conflict_error(conflicts))
}

/// 回退并把结果接在原因后面。**原因永远在前**：用户先要知道为什么没更新成。
fn rolled_back(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    disk: Disk,
    cause: GitError,
) -> Result<SyncInOutcome> {
    let orig = j.orig_branch.clone().unwrap_or_default();
    let tail = match back_to_orig(repo, cfg, j, disk) {
        Ok(RestoreStep::Finished) => {
            format!("已回到分支 `{orig}`，未提交的改动已放回；基线没有变。")
        }
        Ok(_) => "已回到原分支，但放回未提交的改动时遇到问题——详见顶部横幅。".to_string(),
        Err(re) => format!(
            "另外：回到原分支也没成功（{}）。记录已保留，处理好之后在顶部横幅上点「放弃」。",
            re.message
        ),
    };
    Err(GitError::new(&cause.code, format!("{}\n\n{tail}", cause.message)))
}
```

- [ ] **Step 4：跑测试，确认通过**

```powershell
cargo test --test svn_sync_in
```

- [ ] **Step 5：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `on_update_failed` 改回 `Err(update_failed_error(stderr))` | `update_failure_returns_to_the_original_branch` |
| `checkout_verified` 不查 `tracked_changes`（只看 `on_target`） | `a_file_held_open_during_checkout_is_named_and_rolled_back`（它会走到 S4 被报成 `sync_in_baseline_stale`） |
| `svn_conflicts` 只看 `item == "conflicted"` | 无——本 fixture 是文本冲突。**记为覆盖缺口**：树冲突由 Task 2 的解析测试守着；若执行者能在 30 分钟内造出「wc2 删目录 + wc1 目录里有非 W-file 本地修改」的 fixture，补一条 `a_tree_conflict_rolls_back`（必测 #9）并记进决策日志 |

- [ ] **Step 6：全量检查、更新决策日志、提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): sync_in 的 S3/S4/S5 失败回到原分支并放回未提交改动"
```

---

## Task 7A：外部整体 update 之后由「更新」接管（计划 A16）

**背景（用户 2026-09-25 提出，并确认按此方案做）：** 公司项目整个工程都在 SVN 里，程序每天早上第一件事就是在 TortoiseSVN 里对**项目根**点 update，美术和代码一起更新。这时本地通常有几个 git commit 和几处未提交改动。SVN 不知道 git 的存在，会把 commit 和未提交改动都当成本地修改，把服务器改动三方合并进同一批文件。

写计划前在临时 fixture 上实测（2026-09-25，`probe-ext-update.ps1`；fixture 与真实初始化一致，装了 `.git/info/attributes` 的 `* -text`，`hash-object` 带 `--path`）：

| 步骤 | 实测结果 |
|---|---|
| 外部 `svn update`（服务器改 A 第 5 行；我的 commit 改 A 第 1 行，未提交改动在 A 第 10 行） | 退出 0，`G Code\A.cs`，静默合并 |
| 此刻 `git diff HEAD -- Code/A.cs` | `+A line 5 SERVER` 与 `+A line 10 UNSTAGED` 并列：**git 这一侧分不清哪行是服务器的** |
| 重建基线（`svn cat -r BASE` 录 `svn/base`，不碰磁盘 / HEAD / 索引）之后，按 S1→S2→S4→S9→S10 走 | S4 干净；rebase 两个 commit 成功；stash apply 自动合并 |
| 结果 | `git diff HEAD` **只剩** 3 处未提交改动；`git diff svn/base HEAD` **只剩** 2 个 commit 的改动——重新分开了 |
| 同一流程，但未提交改动在 A 第 6 行（紧挨服务器的第 5 行） | svn 仍静默合并成功；**stash apply 冲突**（`UU Code/A.cs`，冲突标记里两边内容都在）。git 的合并对相邻行比 svn 严 |

结论：信息没有丢，SVN 的 BASE 就是服务器原版；Phase 1 的「重建基线」把它录成新基线，正常的 `sync_in` 就能把两者拆开。而这是**每天都会发生**的状态，不能每天挂一条红色横幅、要用户手工点两步。所以本 Task 让「更新」直接接管它：

- **只接管「整体前进」**：`nodeRevs` 单值且大于 `baselineRev`（`SyncVerdict::Diverged` 且 `node_rev > baseline_rev`）。mixed-revision、退回基线之前的状态照旧拒绝：前者要走 §9.4 的 revert / 归一，归 2d；后者多半是误操作，自动接管没有正当理由。
- **在锁内、S0 之前**调用 Phase 1 的 `rebuild_baseline`，再复核一次，然后走正常的 S0–S11。
- 相邻行冲突走现有的 `StashConflict` 出口，stash 保留、内容不丢，由用户手工解一次。这是 git 合并的固有行为，本期不绕开。
- **服务器新增的文件**：外部 update 把它放到磁盘上，git 视为未跟踪，S1 的 `-u` 把它收进 stash；基线跟上之后它又以已跟踪文件的身份回到磁盘，S10 的撞名预检（A3）会拦下。实测（`probe-ext-added.ps1`）：不处理时 `git stash apply` 报 `Code/S.cs already exists, no checkout` 并半套放回；先把「磁盘 / stash / HEAD 三处同一个 blob」的文件从磁盘拿掉再 apply，结果干净。这条放宽写在 Task 3 的 `restore_wip` 里（计划 A17），本 Task 只补端到端用例。

**Files:**
- Modify: `src-tauri/src/engine/svn_sync_check.rs`（`SyncInPreview.catch_up_rev`；第 2 条放行「整体前进」）
- Modify: `src-tauri/src/engine/svn_sync_in.rs`（`run_sync_in` 在 S0 前接管；新增 `catch_up`）
- Test: `src-tauri/tests/svn_sync_check.rs`（替换 Task 2 的 `an_out_of_sync_baseline_is_refused`）、`src-tauri/tests/svn_sync_in.rs`

**Interfaces:**
- Consumes：既有 `svn_init::{rebuild_baseline, RebuildReport}`（`RebuildReport { node_rev, old_baseline, new_baseline, health }`）、`svn_sync::SyncVerdict`；Task 2 的 `check_sync_in`；Task 3 的 `restore_wip`（含 A17 的放宽）；Task 5 的 `verify_expect`、`SyncInExpect`（`Clone`）。
- Produces：
  - `SyncInPreview` 新增字段 `pub catch_up_rev: Option<u64>`，JSON 键 `catchUpRev`，值为数字或 `null`（Task 8 的序列化测试、Task 9 的 TS 类型都按它写）
  - `fn catch_up(repo: &str, before: &SyncInPreview, expect: &SyncInExpect) -> Result<SyncInPreview>`（模块私有）

- [ ] **Step 1：写失败的测试——预览**

`src-tauri/tests/svn_sync_check.rs`：**删掉** Task 2 写的 `an_out_of_sync_baseline_is_refused`（它断言的正是本 Task 要放行的形状），换成下面四条：

```rust
#[test]
fn an_externally_advanced_working_copy_previews_a_catch_up() {
    // 计划 A16：外部在项目根 update 过——nodeRevs 单值且比基线新。
    // 预览不拒绝，而是报出要先跟上的 revision；预览本身仍然什么都不改
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.catch_up_rev, Some(2));
    assert_eq!(p.baseline_rev, 1, "baseline_rev 仍是跟上之前的");
    assert_eq!(p.target_rev, 2);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base, "预览不动基线");
}

#[test]
fn an_in_sync_working_copy_previews_no_catch_up() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    assert_eq!(preview_sync_in(&fx.wc_str()).unwrap().catch_up_rev, None);
}

#[test]
fn a_mixed_revision_working_copy_is_still_refused() {
    // A16 只接管「整体前进」。在 wc1 自己身上外部只提交一个文件造出 mixed-revision
    // （spec 必测 #13 的造法），它归 §9.4（2d）
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-external\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "external", "--non-interactive", "Code/A.cs"],
    );
    let base = git(&fx, &["rev-parse", "svn/base"]);
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_out_of_sync");
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base);
}

#[test]
fn a_working_copy_rolled_back_below_the_baseline_is_still_refused() {
    // 反方向的 Diverged：先用 Phase 1 的重建把基线推到 r2，再在外部把工作副本退回 r1
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    app_lib::engine::svn_init::rebuild_baseline(&fx.wc_str()).unwrap();
    svn_cmd(&fx.wc(), &["update", "-r", "1", "--non-interactive"]);
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_out_of_sync");
}
```

- [ ] **Step 2：写失败的测试——执行**

`src-tauri/tests/svn_sync_in.rs` 末尾追加：

```rust
/// 10 行的文件内容，`edits` 里的行（从 1 数）换成给定文本。
///
/// 外部 update 的三方合并要有「不同的行」可合：单行文件上服务器与本地一改就是 svn 冲突，
/// 测不到本 Task 要测的「静默合并」。
fn ten_lines(tag: &str, edits: &[(usize, &str)]) -> String {
    (1..=10)
        .map(|i| match edits.iter().find(|(n, _)| *n == i) {
            Some((_, s)) => format!("{s}\n"),
            None => format!("{tag}{i}\n"),
        })
        .collect()
}

/// 先让同事把 A / B 换成 10 行的文件（r2、r3），再走一次正常更新，基线停在 r3。
fn with_ten_line_files(fx: &SvnFixture) {
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[]));
    fx.colleague_commits("Code/B.cs", &ten_lines("b", &[]));
    assert_eq!(sync(fx), SyncInOutcome::Done { new_rev: 3 });
}

/// TortoiseSVN 在项目根点 update 的等价物。
fn external_root_update(fx: &SvnFixture) {
    svn_fixtures::svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive"],
    );
}

#[test]
fn an_external_root_update_is_taken_over_and_separated_again() {
    // 用户 2026-09-25 提的日常场景（计划 A16）：本地 2 个 commit + 未提交改动，
    // 早上在 TortoiseSVN 里对项目根点了 update
    let fx = hybrid_with_colleague();
    with_ten_line_files(&fx);
    write(&fx, "Code/A.cs", &ten_lines("a", &[(1, "a1-commit")]));
    commit(&fx, "c1");
    write(&fx, "Code/B.cs", &ten_lines("b", &[(1, "b1-commit")]));
    commit(&fx, "c2");
    write(&fx, "Code/A.cs", &ten_lines("a", &[(1, "a1-commit"), (9, "a9-wip")]));
    write(&fx, "Code/B.cs", &ten_lines("b", &[(1, "b1-commit"), (9, "b9-wip")]));
    write(&fx, "Code/New.cs", "new-wip\n");
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server")])); // r4
    external_root_update(&fx);

    // 前提：git 这一侧确实混在一起了——测的是真实形状，不是想象的形状
    let mixed = git(&fx, &["diff", "HEAD", "--", "Code/A.cs"]);
    assert!(mixed.contains("+a5-server") && mixed.contains("+a9-wip"), "{mixed}");

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!((p.catch_up_rev, p.baseline_rev, p.target_rev), (Some(4), 3, 4));
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 4 });

    assert_eq!(baseline_rev(&fx), 4);
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "2", "两个 commit 都在");
    // 分开了：相对 HEAD 只剩自己没提交的；相对基线只剩自己提交的
    let wip = git(&fx, &["diff", "HEAD"]);
    assert!(wip.contains("+a9-wip") && wip.contains("+b9-wip"), "{wip}");
    assert!(!wip.contains("a5-server"), "服务器的改动不该再显示成本地修改：\n{wip}");
    let committed = git(&fx, &["diff", "svn/base", "HEAD"]);
    assert!(committed.contains("+a1-commit") && committed.contains("+b1-commit"), "{committed}");
    assert!(!committed.contains("a5-server") && !committed.contains("wip"), "{committed}");
    // 磁盘上三样都在
    assert_eq!(
        read(&fx, "Code/A.cs"),
        ten_lines("a", &[(1, "a1-commit"), (5, "a5-server"), (9, "a9-wip")])
    );
    assert_eq!(read(&fx, "Code/New.cs"), "new-wip\n");
    assert_eq!(check_sync(&fx.wc_str()).unwrap().verdict, SyncVerdict::InSync);
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn a_file_the_server_added_does_not_block_the_catch_up() {
    // 计划 A17：外部 update 带来的新文件 git 视为未跟踪，S1 的 -u 把它收进 stash；
    // 基线跟上之后它又作为已跟踪文件回到磁盘。三处同一个 blob，不能停在 StashBlocked
    let fx = hybrid_with_colleague();
    write(&fx, "Code/B.cs", "b-wip\n");
    fx.colleague_commits("Code/S.cs", "server-new\n"); // r2
    external_root_update(&fx);
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code/S.cs"]), "?? Code/S.cs", "前提");

    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/S.cs"), "server-new\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(git(&fx, &["status", "--porcelain"]), "M Code/B.cs", "S.cs 已被基线跟踪、与 HEAD 一致");
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn a_catch_up_then_continues_to_a_newer_server_revision() {
    // 外部 update 到 r4 之后同事又提交了 r5：先跟上 r4，再照常更新到 r5
    let fx = hybrid_with_colleague();
    with_ten_line_files(&fx);
    write(&fx, "Code/B.cs", &ten_lines("b", &[(9, "b9-wip")]));
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server")])); // r4
    external_root_update(&fx);
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server"), (7, "a7-server")])); // r5

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!((p.catch_up_rev, p.target_rev), (Some(4), 5));
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 5 });
    assert_eq!(
        read(&fx, "Code/A.cs"),
        ten_lines("a", &[(5, "a5-server"), (7, "a7-server")])
    );
    assert_eq!(git(&fx, &["diff", "HEAD", "--name-only"]), "Code/B.cs");
    // 基线链上 r4 与 r5 各有一个 commit：接管录的 r4 是 S7 的父提交
    let revs: Vec<u64> = git(&fx, &["log", "--format=%H", "-3", "svn/base"])
        .lines()
        .map(|oid| parse_trailer(&git(&fx, &["log", "-1", "--format=%B", oid])).unwrap().rev)
        .collect();
    assert_eq!(revs, vec![5, 4, 3]);
}

#[test]
fn an_external_update_next_to_my_own_edit_stops_as_a_stash_conflict_without_losing_anything() {
    // 2026-09-25 实测：服务器改第 5 行、我没提交的改动在第 6 行时，svn 的三方合并静默
    // 成功，而 git stash apply 把相邻行当成冲突。结局是 StashConflict：内容都在、备份保留
    let fx = hybrid_with_colleague();
    with_ten_line_files(&fx);
    write(&fx, "Code/A.cs", &ten_lines("a", &[(6, "a6-wip")]));
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server")])); // r4
    external_root_update(&fx);
    assert_eq!(
        read(&fx, "Code/A.cs"),
        ten_lines("a", &[(5, "a5-server"), (6, "a6-wip")]),
        "前提：svn 静默合并成功"
    );

    match sync(&fx) {
        SyncInOutcome::StashConflict { files, .. } => {
            assert_eq!(files, vec!["Code/A.cs".to_string()])
        }
        o => panic!("相邻行应当是 stash 冲突：{o:?}"),
    }
    assert_eq!(baseline_rev(&fx), 4, "基线已经跟上，冲突发生在它之后");
    let a = read(&fx, "Code/A.cs");
    assert!(a.contains("a5-server") && a.contains("a6-wip"), "两边都在：\n{a}");
    assert_eq!(our_stashes(&fx), 1, "原始备份还在");
    assert_eq!(phase(&fx), Some(Phase::ApplyingStash));
}

#[test]
fn a_catch_up_is_refused_when_the_branch_moved_since_the_preview() {
    // 接管会移动基线指针，所以锁内复核必须在它**之前**：确认之后分支被动过就一个字节都不动
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_fixtures::svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    let e = expect(&fx);
    let base = git(&fx, &["rev-parse", "svn/base"]);
    write(&fx, "Code/B.cs", "b-late\n");
    commit(&fx, "committed after preview");
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_state_changed");
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base, "基线没被接管");
    assert_eq!(phase(&fx), None);
}
```

- [ ] **Step 3：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_sync_check
cargo test --test svn_sync_in external catch_up
```

期望：编译失败（`catch_up_rev` 不存在）。

- [ ] **Step 4：实现——预览**

`src-tauri/src/engine/svn_sync_check.rs`：

1. `SyncInPreview` 在 `target_rev` 之后加字段：

```rust
    /// 计划 A16：工作副本已被外部**整体** update 到的 revision（比基线新）。
    /// `Some` 时执行会先把基线跟上它，再走 S0–S11；确认框必须说出这件事。
    pub catch_up_rev: Option<u64>,
```

2. `check_sync_in` 里第 2 条整段换成：

```rust
    // 第 2 条（完整 nodeRevs）。「整体前进」不拒绝，交给执行时接管（计划 A16）：
    // 在 TortoiseSVN 里对项目根点 update 是每天早上的日常，不是异常
    let status = check_sync(repo_root)?;
    let catch_up_rev = match status.verdict {
        SyncVerdict::InSync => None,
        SyncVerdict::Diverged {
            node_rev,
            baseline_rev,
        } if node_rev > baseline_rev => Some(node_rev),
        _ => {
            return Err(GitError::new(
                "sync_in_out_of_sync",
                format!(
                    "基线已经失同步（白名单 revision {:?}，基线记的是 r{}）。\
                     工作副本处于多个 revision，或被退回到了基线之前——这两种状态\
                     「更新」不能自动接管，否则会把外部的改动当成服务器的内容录进基线。\
                     请先处理顶部的失同步横幅。",
                    status.node_revs, status.baseline_rev
                ),
            ))
        }
    };
```

3. 末尾构造 `SyncInPreview` 时加 `catch_up_rev,`。

- [ ] **Step 5：实现——执行**

`src-tauri/src/engine/svn_sync_in.rs`：

1. `use crate::engine::svn_init::record_server_tree_into;` 改成 `use crate::engine::svn_init::{rebuild_baseline, record_server_tree_into};`。
2. `run_sync_in` 里锁内复核那两行换成：

```rust
    // §7.2 第 7 条：锁内复核。预览与确认之间可能过了几分钟，什么都可能变。
    // **必须在接管之前**：接管会移动基线指针，复核不过就一个字节都不能动
    let mut p = check_sync_in(repo)?;
    verify_expect(&p, expect)?;
    if p.catch_up_rev.is_some() {
        p = catch_up(repo, &p, expect)?;
    }
```

3. `verify_expect` 之后加：

```rust
/// 计划 A16：工作副本被外部整体 update 到了比基线新的 rN。
///
/// 先用 Phase 1 的「重建基线」把 `svn/base` 录成服务器 rN 的原版（`svn cat -r BASE`，
/// 不碰磁盘），再走正常的 S0–S11：S9 把分支重放上去、S10 放回 WIP 时，服务器那部分
/// 改动两边一致，三方合并自动抵消，git 这一侧重新分得清「服务器的」和「我的」
/// （2026-09-25 实测，见计划 Task 7A）。
///
/// # 为什么放在 S0 之前、不写 journal
///
/// `rebuild_baseline` 不碰 HEAD、真实索引和磁盘，只以旧 OID 为条件移动一个 ref（R4）。
/// 它要么没发生、要么完整发生，两种状态都自洽：崩在它之后，重开时基线已经跟上、
/// 没有 journal，下次预览看到的就是普通的 `InSync`。没有需要恢复的中间态，
/// 为它加 phase 只会多一种要处理的崩溃现场。
///
/// # 为什么接管之后还要再复核
///
/// 后面 S0 记的 `baselineOidBefore` 必须是**新**基线；同时确认重建期间分支没被动过。
/// 预览时给用户看过的「会碰到的白名单外文件」照常再比一次。
fn catch_up(repo: &str, before: &SyncInPreview, expect: &SyncInExpect) -> Result<SyncInPreview> {
    let report = rebuild_baseline(repo)?;
    let p = check_sync_in(repo)?;
    if report.old_baseline != before.baseline_oid
        || p.catch_up_rev.is_some()
        || p.orig_branch != before.orig_branch
    {
        return Err(GitError::new(
            "sync_in_state_changed",
            format!(
                "基线已经按服务器 r{} 的原版重录好了，但复核时发现仓库在这期间被别的操作改动过。\
                 你的文件和分支都还没有被收起或改动，请重新点一次「更新」。",
                report.node_rev
            ),
        ));
    }
    verify_expect(
        &p,
        &SyncInExpect {
            baseline_oid: p.baseline_oid.clone(),
            ..expect.clone()
        },
    )?;
    Ok(p)
}
```

> ⚠️ 执行者：`rebuild_baseline` 自己带 R1 前置检查、R3b 体检、R3c 复查与 R4 的旧值守卫。它失败时 `?` 直接返回，此时还没有 journal、没有 stash、基线指针没动，和预览拒绝的结果一样。**不要**在这里为它的失败另写回退。

- [ ] **Step 6：跑测试，确认通过；全量回归**

```powershell
cargo test --test svn_sync_check
cargo test --test svn_sync_in
cargo test
```

期望：全部 PASS。`a_catch_up_then_continues_to_a_newer_server_revision` 断言基线链是 `[5, 4, 3]`：接管录一个 r4，S7 录 r5。若 S7 在「服务器没有新内容」时也会再录一个 r4（§7.4「白名单内无变化时也要落 commit」），那是主场景 `an_external_root_update_is_taken_over…` 的形状，不影响这条。

`an_external_update_next_to_my_own_edit…` 若**没有**冲突而是 `Done`，说明这台机器上 git 对相邻行的合并与写计划时不同：先用 `probe-ext-update.ps1 -UnstagedLine 6` 手工复现，把 git 版本和实际输出记进决策日志「计划的错误」，再把断言改成实测形状。**不要**为了让它变绿去改相邻的行号。

- [ ] **Step 7：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `check_sync_in` 的 `if node_rev > baseline_rev` 守卫删掉（回退也接管） | `a_working_copy_rolled_back_below_the_baseline_is_still_refused` |
| `run_sync_in` 不调 `catch_up`（`catch_up_rev` 只报不做） | `an_external_root_update_is_taken_over…`（S4 会把磁盘上的服务器改动报成 `sync_in_baseline_stale`） |
| 把接管挪到 `verify_expect` 之前 | `a_catch_up_is_refused_when_the_branch_moved_since_the_preview`（基线被移动了） |
| `catch_up` 里不重新 `check_sync_in`，沿用旧的 `p` | `an_external_root_update_is_taken_over…`（S0 记下旧的 `baselineOidBefore`，`finish_publish` 报 `svn_baseline_moved`） |
| Task 3 的 `is_same_file_everywhere` 恒为 `false` | `a_file_the_server_added_does_not_block_the_catch_up`（停在 `StashBlocked`） |

- [ ] **Step 8：全量检查、更新决策日志、提交**

决策日志本 Task 下至少记：「计划没写的坑」写相邻行冲突的实测；「为什么这么改是安全的」写接管不写 journal 的论证（Step 5 的文档注释），以及「只接管整体前进」的边界。

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): 外部整体 update 之后，「更新」先把基线跟上再整理分支"
```

---

## Task 8：Tauri 命令

**Files:**
- Create: `src-tauri/src/commands/svn_sync.rs`
- Modify: `src-tauri/src/commands/mod.rs`（`pub mod svn_sync;`）
- Modify: `src-tauri/src/lib.rs`（`generate_handler!` 里 `commands::svn::svn_sync_state,` 之后加四行）
- Test: `src-tauri/tests/svn_backend.rs`、`src-tauri/tests/svn_write_guard.rs`

**Interfaces:**
- Produces（JS 侧参数名是 camelCase，**没有编译期检查**，`AGENTS.md` §7.3）：
  - `invoke('svn_sync_in_preview', { path }) → SyncInPreview`
  - `invoke('svn_sync_in', { path, expect }) → SyncInOutcome`，`expect` 形如 `{ origHead, baselineOid, confirmedOutside }`
  - `invoke('svn_sync_continue', { path }) → SyncInOutcome`
  - `invoke('svn_sync_abort', { path }) → SyncInOutcome`
  - 三个会写仓库的命令：执行期间挂起 watcher，结束后**无论成败**补发一次 `repo-changed`（spec §5.2、§7.3 S11）

- [ ] **Step 1：写失败的测试**

`src-tauri/tests/svn_backend.rs` 末尾追加：

```rust
#[test]
fn sync_in_types_cross_ipc_in_camel_case() {
    // AGENTS.md §7.3：snake_case ↔ camelCase 没有编译期检查。字段名写错只会在
    // 运行时拿到 undefined——svn_sync.rs 的 SyncVerdict 上真出过一次。
    use app_lib::engine::svn_sync_check::SyncInPreview;
    use app_lib::engine::svn_sync_in::{SyncInExpect, SyncInOutcome};

    let p = serde_json::to_value(SyncInPreview {
        orig_branch: "work".into(),
        orig_head: "h".into(),
        baseline_oid: "b".into(),
        baseline_rev: 1,
        target_rev: 2,
        catch_up_rev: Some(2),
        outside_changes: vec!["Art/x.psd".into()],
    })
    .unwrap();
    for k in [
        "origBranch",
        "origHead",
        "baselineOid",
        "baselineRev",
        "targetRev",
        "catchUpRev",
        "outsideChanges",
    ] {
        assert!(p.get(k).is_some(), "SyncInPreview 缺 `{k}`：{p}");
    }
    // `None` 必须发成 `null` 而不是整个键消失：前端按 `catchUpRev === null` 判断（Task 9）
    let none = serde_json::to_value(SyncInPreview {
        orig_branch: "work".into(),
        orig_head: "h".into(),
        baseline_oid: "b".into(),
        baseline_rev: 1,
        target_rev: 1,
        catch_up_rev: None,
        outside_changes: vec![],
    })
    .unwrap();
    assert_eq!(none.get("catchUpRev"), Some(&serde_json::Value::Null), "{none}");

    let cases = [
        (SyncInOutcome::Done { new_rev: 2 }, r#"{"kind":"done","newRev":2}"#),
        (SyncInOutcome::Aborted, r#"{"kind":"aborted"}"#),
        (
            SyncInOutcome::RebaseConflict { files: vec!["A".into()] },
            r#"{"kind":"rebaseConflict","files":["A"]}"#,
        ),
        (
            SyncInOutcome::StashConflict { files: vec!["A".into()], stash_oid: "o".into() },
            r#"{"kind":"stashConflict","files":["A"],"stashOid":"o"}"#,
        ),
        (
            SyncInOutcome::StashBlocked { collisions: vec!["N".into()], stash_oid: "o".into() },
            r#"{"kind":"stashBlocked","collisions":["N"],"stashOid":"o"}"#,
        ),
    ];
    for (v, want) in cases {
        assert_eq!(serde_json::to_string(&v).unwrap(), want);
    }

    let e: SyncInExpect = serde_json::from_str(
        r#"{"origHead":"h","baselineOid":"b","confirmedOutside":["Art/x.psd"]}"#,
    )
    .unwrap();
    assert_eq!(e.confirmed_outside, vec!["Art/x.psd".to_string()]);
}
```

`src-tauri/tests/svn_write_guard.rs` 末尾追加：

```rust
/// 三个会写仓库的同步命令：必须挂起 watcher（spec §5.2），且它们调用的 engine 入口
/// 必须拿锁（spec §5.1）。行为上的「拿不到锁就拒绝」由 `svn_sync_in.rs` 的
/// `a_second_sync_while_one_is_running_is_refused` 守着；这里守的是接线。
#[test]
fn sync_commands_suspend_the_watcher_and_their_engine_entries_take_the_lock() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let cmd = fs::read_to_string(root.join("commands/svn_sync.rs")).unwrap();
    for (func, engine_fn) in [
        ("svn_sync_in", "run_sync_in"),
        ("svn_sync_continue", "continue_sync"),
        ("svn_sync_abort", "abort_sync"),
    ] {
        let body = fn_body(&cmd, func);
        assert!(body.contains("with_watch_suspended("), "`{func}` 没有挂起 watcher");
        assert!(body.contains(engine_fn), "`{func}` 没有调 `{engine_fn}`");
    }
    let eng = fs::read_to_string(root.join("engine/svn_sync_in.rs")).unwrap();
    assert!(fn_body(&eng, "run_sync_in").contains("svn_lock::acquire"));
    assert!(fn_body(&eng, "continue_sync").contains("take_over("));
    assert!(fn_body(&eng, "abort_sync").contains("take_over("));
    assert!(fn_body(&eng, "take_over").contains("svn_lock::acquire"));
}

/// `fn <name>(` 到下一个顶层 `fn` / `pub fn` 之间的源码。依赖 rustfmt 的格式。
fn fn_body<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("找不到 `fn {name}(`——改名了？"));
    let rest = &src[start + 1..];
    let end = [rest.find("\npub fn "), rest.find("\nfn "), rest.find("\npub(crate) fn ")]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(rest.len());
    &rest[..end]
}
```

- [ ] **Step 2：跑测试，确认失败**

```powershell
cd src-tauri
cargo test --test svn_backend sync_in_types --test svn_write_guard sync_commands
```

期望：`svn_backend` 那条通过也无妨（类型在 Task 5 就有了），`svn_write_guard` 那条因 `commands/svn_sync.rs` 不存在而失败。

- [ ] **Step 3：实现**

新建 `src-tauri/src/commands/svn_sync.rs`：

```rust
//! `sync_in` 的四个入口（spec §7、§13）。
//!
//! # 参数名的 snake_case ↔ camelCase 转换没有编译期检查
//!
//! JS 侧是 `invoke('svn_sync_in', { path, expect })`，`expect` 里是
//! `{ origHead, baselineOid, confirmedOutside }`。`tests/svn_backend.rs` 钉着
//! 序列化形状；签名改动必须实跑一次 `pnpm tauri dev`（`AGENTS.md` §7.3）。

use crate::commands::watch::WatchSuspension;
use crate::engine::svn_sync_check::{preview_sync_in, SyncInPreview};
use crate::engine::svn_sync_in::{
    abort_sync, continue_sync, run_sync_in, SyncInExpect, SyncInOutcome,
};
use crate::error::GitError;
use tauri::{AppHandle, Emitter};

/// 预览：前置检查 1–6，**什么都不改**。确认框里的全部事实都来自这里。
#[tauri::command(async)]
pub fn svn_sync_in_preview(path: String) -> Result<SyncInPreview, GitError> {
    preview_sync_in(&path)
}

/// 执行一次更新。`expect` 是用户在确认框里看到的预览，锁内复核要与它一致。
#[tauri::command(async)]
pub fn svn_sync_in(
    handle: AppHandle,
    path: String,
    expect: SyncInExpect,
) -> Result<SyncInOutcome, GitError> {
    with_watch_suspended(&handle, &path, || run_sync_in(&path, &expect))
}

/// 从中断处接着走（冲突解决之后，或崩溃重开之后）。
#[tauri::command(async)]
pub fn svn_sync_continue(handle: AppHandle, path: String) -> Result<SyncInOutcome, GitError> {
    with_watch_suspended(&handle, &path, || continue_sync(&path))
}

/// 放弃这次更新，回到更新前的分支并放回未提交的改动。
#[tauri::command(async)]
pub fn svn_sync_abort(handle: AppHandle, path: String) -> Result<SyncInOutcome, GitError> {
    with_watch_suspended(&handle, &path, || abort_sync(&path))
}

/// 挂起 watcher 跑完一段同步，然后**无论成败**补发一次 `repo-changed`。
///
/// spec §5.2：挂起期间事件被丢弃而不是攒着，所以结束时必须主动告诉前端
/// 「仓库变了」，否则 update 带来的那一大批改动界面看不见。失败也要发：回退
/// 本身就动过分支和工作区。
fn with_watch_suspended<T>(
    handle: &AppHandle,
    path: &str,
    f: impl FnOnce() -> Result<T, GitError>,
) -> Result<T, GitError> {
    let r = {
        let _w = WatchSuspension::new(path);
        f()
    };
    let _ = handle.emit("repo-changed", path);
    r
}
```

`src-tauri/src/lib.rs` 的 `generate_handler![...]` 里 `commands::svn::svn_sync_state,` 之后加：

```rust
            commands::svn_sync::svn_sync_in_preview,
            commands::svn_sync::svn_sync_in,
            commands::svn_sync::svn_sync_continue,
            commands::svn_sync::svn_sync_abort,
```

- [ ] **Step 4：跑测试，确认通过**

```powershell
cargo test --test svn_backend --test svn_write_guard --test commands_are_async
```

- [ ] **Step 5：全量检查、更新决策日志、提交**

```powershell
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
git add src-tauri docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): sync_in 的预览、执行、继续、放弃四个 Tauri 命令"
```

`pnpm tauri dev` 的实跑放在 Task 10（前端接好之后才点得到）。

---

## Task 9：前端纯逻辑

本仓库没有 jsdom，组件跑不起来；分支逻辑全部下沉到 `src/engine/`，由 vitest 覆盖（`toolbarActions.ts` 的先例）。

**Files:**
- Create: `src/engine/svnSyncIn.ts`
- Modify: `src/engine/svnJournal.ts`（LF）、`src/engine/svnSync.ts`（`BannerAction.id` 加三个值；「整体前进」的横幅，计划 A16）、`src/engine/toolbarActions.ts`
- Test: Create `tests/vitest/svnSyncIn.test.ts`；Modify `tests/vitest/svnJournal.test.ts`、`tests/vitest/toolbarActions.test.ts`、`tests/vitest/svnSync.test.ts`

**Interfaces:**
- Produces（Task 10 用）：
  - 类型 `SyncInPreview`、`SyncInExpect`、`SyncInOutcome`（与 Task 8 的 JSON 形状逐字对应）、`SyncConfirmModel`、`OutcomeNotice`、`SyncActionId = 'syncContinue' | 'syncAbort'`
  - `expectFromPreview(p: SyncInPreview): SyncInExpect`
  - `describeSyncInPreview(p: SyncInPreview): SyncConfirmModel`
  - `describeSyncOutcome(o: SyncInOutcome): OutcomeNotice | null`（`done` 返回 `null`）
  - `syncActions(phase: string): BannerAction[]`
  - `describeSyncAction(id: SyncActionId, phase: string): SyncConfirmModel`
  - `svnJournal.ts`：`SyncPhase` 加 `'applying-stash'`；`describeUnfinishedSync(s, nowSecs?)`；`sinceText(startedAt, nowSecs): string`
  - `toolbarActions.ts`：`showsSvnUpdate(caps: Capabilities): boolean`

- [ ] **Step 1：写失败的测试**

新建 `tests/vitest/svnSyncIn.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import {
  describeSyncAction,
  describeSyncInPreview,
  describeSyncOutcome,
  expectFromPreview,
  syncActions,
  type SyncInPreview,
} from '@/engine/svnSyncIn'

const preview: SyncInPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  targetRev: 3,
  catchUpRev: null,
  outsideChanges: [],
}

describe('expectFromPreview', () => {
  it('把预览里的事实原样带回去做锁内复核', () => {
    const p = { ...preview, outsideChanges: ['Art/x.psd'] }
    const e = expectFromPreview(p)
    expect(e).toEqual({
      origHead: p.origHead,
      baselineOid: p.baselineOid,
      confirmedOutside: ['Art/x.psd'],
    })
    // 拷贝而不是共享：确认框之后预览对象被改，不能悄悄改掉已确认的清单
    expect(e.confirmedOutside).not.toBe(p.outsideChanges)
  })
})

describe('describeSyncInPreview', () => {
  it('说清从哪个 revision 更新到哪个、会重放哪个分支', () => {
    const m = describeSyncInPreview(preview)
    expect(m.message).toContain('r1')
    expect(m.message).toContain('r3')
    expect(m.message).toContain('work')
    expect(m.danger).toBeFalsy()
  })

  it('服务器没有新提交时照实说，不假装有更新', () => {
    const m = describeSyncInPreview({ ...preview, targetRev: 1 })
    expect(m.message).toContain('没有比 r1 更新的提交')
  })

  it('白名单外会被碰到的文件必须列出来，并用危险色', () => {
    // spec §7.2.1：git 对它们没有任何恢复手段，确认只授权 update
    const m = describeSyncInPreview({ ...preview, outsideChanges: ['Art/boss.psd'] })
    expect(m.warning).toContain('Art/boss.psd')
    expect(m.warning).toContain('TortoiseSVN')
    expect(m.danger).toBe(true)
  })

  it('总是说明「已暂存」状态不会保留（计划 A8）', () => {
    expect(describeSyncInPreview(preview).warning).toContain('重新暂存')
  })

  it('工作副本被外部整体更新过：先说会把基线跟上、服务器改动与自己的改动会重新分开（计划 A16）', () => {
    const m = describeSyncInPreview({ ...preview, baselineRev: 1, catchUpRev: 2, targetRev: 3 })
    expect(m.message).toContain('TortoiseSVN')
    expect(m.message).toContain('r2')
    expect(m.message).toContain('重新分开')
    expect(m.message).toContain('把 r2 更新到 r3')
    expect(m.danger).toBeFalsy()
  })

  it('接管之后服务器没有更新的内容：只整理，不说「仍会走一遍流程」那句空话', () => {
    const m = describeSyncInPreview({ ...preview, baselineRev: 1, catchUpRev: 3, targetRev: 3 })
    expect(m.message).toContain('没有比 r3 更新的提交')
    expect(m.message).not.toContain('仍会走一遍流程')
  })
})

describe('describeSyncOutcome', () => {
  it('成功不打扰', () => {
    expect(describeSyncOutcome({ kind: 'done', newRev: 3 })).toBeNull()
  })

  it('rebase 冲突：列出文件，并指向横幅上的继续 / 放弃', () => {
    const n = describeSyncOutcome({ kind: 'rebaseConflict', files: ['Code/A.cs'] })!
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('继续')
    expect(n.detail).toContain('放弃')
  })

  it('撞名：列出文件并说明一个字节都没放回、备份在哪', () => {
    const n = describeSyncOutcome({
      kind: 'stashBlocked',
      collisions: ['Code/N.cs'],
      stashOid: 'f2208125aad7d35d63ee9495ad0a6cc3c1ce21be',
    })!
    expect(n.detail).toContain('Code/N.cs')
    expect(n.detail).toContain('f2208125aa')
  })

  it('stash 冲突给出备份 OID', () => {
    const n = describeSyncOutcome({
      kind: 'stashConflict',
      files: ['Code/A.cs'],
      stashOid: 'f2208125aad7d35d63ee9495ad0a6cc3c1ce21be',
    })!
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('f2208125aa')
  })
})

describe('syncActions', () => {
  it('start 只有「清除记录」：它的定义就是还没动任何东西（计划 A10）', () => {
    const a = syncActions('start')
    expect(a.map((x) => x.id)).toEqual(['syncAbort'])
    expect(a[0].label).toContain('清除')
  })

  it('中间 phase 都有继续与放弃', () => {
    for (const p of ['stashing', 'stashed', 'capturing-baseline', 'publishing-baseline', 'baseline-ready', 'rebase', 'applying-stash']) {
      expect(syncActions(p).map((x) => x.id), p).toEqual(['syncContinue', 'syncAbort'])
    }
  })

  it('认不出的 phase 不给按钮（多半是 sync_out，Phase 3 才有）', () => {
    expect(syncActions('svn-staged')).toEqual([])
  })
})

describe('describeSyncAction', () => {
  it('rebase 阶段放弃要说清：基线保持新的，分支会落后于它', () => {
    const m = describeSyncAction('syncAbort', 'rebase')
    expect(m.message).toContain('基线')
    expect(m.danger).toBe(true)
  })

  it('applying-stash 阶段放弃要说清备份留在 stash 列表里', () => {
    expect(describeSyncAction('syncAbort', 'applying-stash').message).toContain('stash')
  })

  it('S5 之后放弃要说清 SVN 侧不退回、可能要重建基线（计划 A5）', () => {
    const m = describeSyncAction('syncAbort', 'capturing-baseline')
    expect(m.message).toContain('重建基线')
  })

  it('继续不是危险操作', () => {
    expect(describeSyncAction('syncContinue', 'rebase').danger).toBeFalsy()
  })
})

describe('文案按纯文本渲染', () => {
  it('任何一段都不能带 Markdown 记号（2b 第二轮 review R4 的教训）', () => {
    const texts: string[] = []
    const push = (m: { title: string; message: string; warning?: string }) =>
      texts.push(m.title, m.message, m.warning ?? '')
    push(describeSyncInPreview({ ...preview, outsideChanges: ['Art/boss.psd'] }))
    push(describeSyncInPreview({ ...preview, targetRev: 1 }))
    push(describeSyncInPreview({ ...preview, catchUpRev: 2 }))
    for (const p of ['start', 'stashed', 'capturing-baseline', 'rebase', 'applying-stash']) {
      push(describeSyncAction('syncContinue', p))
      push(describeSyncAction('syncAbort', p))
    }
    for (const o of [
      { kind: 'aborted' as const },
      { kind: 'rebaseConflict' as const, files: ['A'] },
      { kind: 'stashConflict' as const, files: ['A'], stashOid: 'x'.repeat(40) },
      { kind: 'stashBlocked' as const, collisions: ['N'], stashOid: 'x'.repeat(40) },
    ]) {
      const n = describeSyncOutcome(o)!
      texts.push(n.title, n.detail)
    }
    for (const t of texts) expect(t).not.toContain('**')
  })
})
```

`tests/vitest/svnJournal.test.ts`：

1. 顶部 import 补 `sinceText`。
2. 把 `it('本期还没有继续/放弃按钮，但必须说清这一点', ...)` 整条替换为：

```ts
  it('sync_in 未收尾时给出继续 / 放弃按钮', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.actions.map((a) => a.id)).toEqual(['syncContinue', 'syncAbort'])
  })

  it('sync_out 留下的记录本版本收不了尾：不给按钮，并说明原因', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_out', phase: 'svn-staged' as SyncPhase, opid: 'x', startedAt: 1 },
    })!
    expect(b.actions).toEqual([])
    expect(b.note).toContain('后续版本')
  })
```

3. `每个 phase 都有自己的说明` 与 `横幅按纯文本渲染` 两条里的 phase 列表都加上 `'applying-stash'`。
4. 末尾追加：

```ts
describe('sinceText', () => {
  it('与后端 describe_age 同一套说法', () => {
    expect(sinceText(1000, 1030)).toBe('刚刚开始')
    expect(sinceText(1000, 1000 + 5 * 60)).toBe('已持续 5 分钟')
    expect(sinceText(1000, 1000 + 2 * 3600 + 60)).toBe('已持续 2 小时')
    expect(sinceText(2000, 1000)).toBe('刚刚开始')
  })

  it('正在同步的横幅带上持续时长（spec §13：pid 与开始时间）', () => {
    const b = describeUnfinishedSync(
      { locked: true, holder: { pid: 4242, startedAt: 1000, opid: 'x' }, unfinished: null },
      1000 + 7 * 60,
    )!
    expect(b.detail).toContain('4242')
    expect(b.detail).toContain('已持续 7 分钟')
  })
})
```

`tests/vitest/toolbarActions.test.ts` 末尾追加：

```ts
import { showsSvnUpdate } from '../../src/engine/toolbarActions'
import type { Capabilities } from '../../src/engine/capabilities'

describe('showsSvnUpdate', () => {
  const base: Capabilities = {
    vcs: 'git', worktrees: true, tags: true, remotes: true,
    pullRequests: true, cherryPick: true, patches: true,
  }
  it('只在 SVN 混合仓库上出现', () => {
    expect(showsSvnUpdate(base)).toBe(false)
    expect(showsSvnUpdate({ ...base, vcs: 'svn', remotes: false })).toBe(true)
  })
})
```

（两条 `import` 放进文件顶部已有的 import 区，不要留在末尾；`Capabilities` 的字段名以 `src/engine/capabilities.ts` 的实际定义为准。）

`tests/vitest/svnSync.test.ts`（计划 A16：外部整体 update 是日常，不是事故）在 `describe('describeSync', ...)` 里、`diverged 时分清是被推进还是被回退` 那条之后加：

```ts
  it('整体前进（外部在项目根 update 过）：黄色，第一个按钮是「更新」（计划 A16）', () => {
    const m = describeSync({
      baselineRev: 4,
      nodeRevs: [9],
      verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
    })!
    expect(m.tone).toBe('warn')
    expect(m.actions[0].id).toBe('svnUpdate')
    expect(m.detail).toContain('TortoiseSVN')
    // 重建与体检仍然在：「更新」要联网，离线时用户还能先把基线跟上
    expect(m.actions.map((a) => a.id)).toEqual(['svnUpdate', 'rebuild', 'check'])
  })

  it('被回退、多 revision 时不给「更新」：那两种「更新」会拒绝', () => {
    const backward = describeSync({
      baselineRev: 9,
      nodeRevs: [4],
      verdict: { kind: 'diverged', nodeRev: 4, baselineRev: 9 },
    })!
    const mixed = describeSync({
      baselineRev: 4,
      nodeRevs: [4, 7],
      verdict: { kind: 'mixedRevision', revs: [4, 7] },
    })!
    for (const m of [backward, mixed]) {
      expect(m.tone).toBe('danger')
      expect(m.actions.map((a) => a.id)).not.toContain('svnUpdate')
    }
  })
```

既有的 `diverged 时分清是被推进还是被回退`（断言前进的文案含「前进」）与 `所有 danger 文案都写明重建基线…`（断言含 `rebuild` 与 `note`）**原样保留**，新文案必须同时满足它们。

- [ ] **Step 2：跑测试，确认失败**

```powershell
pnpm vitest run tests/vitest/svnSyncIn.test.ts tests/vitest/svnJournal.test.ts tests/vitest/toolbarActions.test.ts tests/vitest/svnSync.test.ts
```

期望：`svnSyncIn` 模块不存在；`svnJournal` 新用例失败；`svnSync` 的两条新用例失败（前进仍是 `danger`、没有 `svnUpdate`）。

- [ ] **Step 3：实现 `svnSyncIn.ts`**

```ts
/**
 * 更新（sync_in）的纯展示逻辑：预览 → 确认框、结局 → 提示、继续 / 放弃 → 确认框。
 *
 * 纯函数，无 React / Tauri 依赖（`AGENTS.md` §7.2）。组件把返回值原样展开进
 * `ConfirmDialog` 的请求，所以每句文案只在这里写一份。
 *
 * ⚠️ 横幅与对话框都按**纯文本**渲染：不要写 `**`（2b 第二轮 review R4）。
 */
import type { BannerAction } from './svnSync'

/** 后端 `svn_sync_check::SyncInPreview`。 */
export interface SyncInPreview {
  origBranch: string
  origHead: string
  baselineOid: string
  baselineRev: number
  targetRev: number
  /** 计划 A16：工作副本已被外部整体 update 到的 revision；`null` 表示没有。 */
  catchUpRev: number | null
  outsideChanges: string[]
}

/** 后端 `svn_sync_in::SyncInExpect`。 */
export interface SyncInExpect {
  origHead: string
  baselineOid: string
  confirmedOutside: string[]
}

/** 后端 `svn_sync_in::SyncInOutcome`。 */
export type SyncInOutcome =
  | { kind: 'done'; newRev: number }
  | { kind: 'aborted' }
  | { kind: 'rebaseConflict'; files: string[] }
  | { kind: 'stashConflict'; files: string[]; stashOid: string }
  | { kind: 'stashBlocked'; collisions: string[]; stashOid: string }

/** 与 `ConfirmRequest` 同名的那几个字段，组件直接展开。 */
export interface SyncConfirmModel {
  title: string
  message: string
  warning?: string
  confirmLabel: string
  busyLabel: string
  danger?: boolean
}

export interface OutcomeNotice {
  title: string
  detail: string
}

export type SyncActionId = 'syncContinue' | 'syncAbort'

/** 预览里的事实原样带回去做锁内复核（spec §7.2 第 7 条）。 */
export function expectFromPreview(p: SyncInPreview): SyncInExpect {
  return {
    origHead: p.origHead,
    baselineOid: p.baselineOid,
    confirmedOutside: [...p.outsideChanges],
  }
}

const MAX_LISTED = 20

function listFiles(files: string[]): string {
  const shown = files.slice(0, MAX_LISTED).map((f) => `· ${f}`)
  if (files.length > MAX_LISTED) shown.push(`…还有 ${files.length - MAX_LISTED} 个`)
  return shown.join('\n')
}

function shortOid(oid: string): string {
  return oid.slice(0, 10)
}

/** 计划 A8：S10 用不带 `--index` 的 stash apply。 */
export const STAGING_NOTE = '「已暂存」的状态不会保留：改动内容都会放回来，只是需要重新暂存。'

export function describeSyncInPreview(p: SyncInPreview): SyncConfirmModel {
  // 计划 A16：外部整体 update 过时，先跟上的那个 revision 才是这次更新的起点
  const from = p.catchUpRev ?? p.baselineRev
  const catchUp =
    p.catchUpRev === null
      ? ''
      : `工作副本已经在外部（TortoiseSVN）更新到了 r${p.catchUpRev}，基线还停在 r${p.baselineRev}，` +
        '所以「本地改动」里服务器的改动和你自己的改动现在混在一起。' +
        `这次会先按服务器 r${p.catchUpRev} 的原版把基线跟上（不碰你的文件），` +
        '再把你的提交和未提交改动整理到新基线上，两者会重新分开。\n\n'
  const update =
    p.targetRev <= from
      ? p.catchUpRev === null
        ? `服务器上没有比 r${from} 更新的提交。仍会走一遍流程，让基线记账与工作副本保持一致。`
        : `服务器上没有比 r${from} 更新的提交，这次只做上面的整理。`
      : `把 r${from} 更新到 r${p.targetRev}：未提交的改动先收起来，更新完之后把分支 ${p.origBranch} 重放到新基线上，再把改动放回来。`
  const message = catchUp + update
  const parts: string[] = []
  if (p.outsideChanges.length > 0) {
    parts.push(
      '下面这些文件 git 管不到、你本地改过、这次更新又会碰到它们。' +
        '出现冲突时只能回 TortoiseSVN 处理，本应用没有它们的备份：\n' +
        listFiles(p.outsideChanges),
    )
  }
  parts.push(STAGING_NOTE)
  return {
    title: '更新',
    message,
    warning: parts.join('\n\n'),
    confirmLabel: '更新',
    busyLabel: '正在更新…',
    danger: p.outsideChanges.length > 0,
  }
}

export function describeSyncOutcome(o: SyncInOutcome): OutcomeNotice | null {
  switch (o.kind) {
    case 'done':
      return null // 成功不打扰：界面刷新本身就是结果
    case 'aborted':
      return { title: '已放弃这次更新', detail: '已回到更新前的分支，未提交的改动已放回。' }
    case 'rebaseConflict':
      return {
        title: '更新遇到冲突',
        detail:
          `基线已经更新好了，把你的提交重放上去时有冲突：\n${listFiles(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「继续」。想回到更新前，点「放弃」。',
      }
    case 'stashConflict':
      return {
        title: '放回未提交的改动时有冲突',
        detail:
          `更新已经完成，但你未提交的改动与新代码冲突了：\n${listFiles(o.files)}\n\n` +
          `改好并暂存之后点顶部横幅上的「继续」。改动的原始备份一直在 stash 里（${shortOid(o.stashOid)}）。`,
      }
    case 'stashBlocked':
      return {
        title: '未提交的改动还没放回来',
        detail:
          `服务器这次新增了与你的未跟踪文件同名的文件，直接放回会覆盖其中一边：\n${listFiles(o.collisions)}\n\n` +
          `把这些文件挪到别处（或改名）之后点顶部横幅上的「继续」。一个字节都还没放回，备份在 stash ${shortOid(o.stashOid)}。`,
      }
  }
}

const CONTINUE: BannerAction = { id: 'syncContinue', label: '继续' }
const ABORT: BannerAction = { id: 'syncAbort', label: '放弃，回到更新前' }

/** 横幅上的按钮。认不出的 phase（`sync_out` 的，Phase 3 才有）不给按钮。 */
export function syncActions(phase: string): BannerAction[] {
  switch (phase) {
    case 'start':
      return [{ id: 'syncAbort', label: '清除这条记录' }]
    case 'rebase':
      return [{ id: 'syncContinue', label: '我已解决，继续' }, ABORT]
    case 'applying-stash':
      return [{ id: 'syncContinue', label: '我已处理，继续' }, { id: 'syncAbort', label: '不再自动放回' }]
    case 'stashing':
    case 'stashed':
    case 'capturing-baseline':
    case 'publishing-baseline':
    case 'baseline-ready':
      return [CONTINUE, ABORT]
    default:
      return []
  }
}

const AFTER_UPDATE_ABORT =
  '回到更新前的分支，放回未提交的改动。SVN 那一侧已经更新的部分不会退回——' +
  '如果基线还没跟上，顶部会提示失同步，用「重建基线」即可。'

export function describeSyncAction(id: SyncActionId, phase: string): SyncConfirmModel {
  if (id === 'syncContinue') {
    const message =
      phase === 'rebase'
        ? '确认冲突文件都已改好并暂存。会接着把剩下的提交重放完，再放回未提交的改动。'
        : phase === 'applying-stash'
          ? '确认冲突已解决（或同名文件已挪走）。会结束这次更新并清理 stash 里的备份。'
          : '从中断的地方接着走完这次更新。'
    return { title: '继续更新', message, confirmLabel: '继续', busyLabel: '正在继续…' }
  }
  const message =
    phase === 'start'
      ? '上次更新在动手之前就中断了，清除这条记录即可，不会改动任何文件。'
      : phase === 'rebase'
        ? '中止重放，分支回到更新前的位置，放回未提交的改动。基线已经是新的，你的分支会落后于它，下次更新时再跟上。'
        : phase === 'applying-stash'
          ? '结束这次更新，不再自动放回改动。stash 里的备份（mygitclient-sync:…）会留在 stash 列表里，你可以之后自己取回。'
          : AFTER_UPDATE_ABORT
  return {
    title: '放弃这次更新',
    message,
    confirmLabel: '放弃',
    busyLabel: '正在放弃…',
    danger: phase !== 'start',
  }
}
```

- [ ] **Step 4：改 `svnSync.ts`、`svnJournal.ts`、`toolbarActions.ts`**

`src/engine/svnSync.ts`：

1. `BannerAction.id` 的联合类型加上 `| 'syncContinue' | 'syncAbort' | 'svnUpdate'`。
2. `describeSync` 的 `case 'diverged'` 整段换成（计划 A16：前进是每天早上的日常，给出口、降成黄色；回退照旧）：

```ts
    case 'diverged': {
      if (v.nodeRev > v.baselineRev) {
        return {
          tone: 'warn',
          title: '工作副本已在外部更新',
          detail:
            `工作副本已经前进到 r${v.nodeRev}，基线还记着 r${v.baselineRev}——` +
            '在 TortoiseSVN 里对项目根点过 update 就会这样。' +
            '此刻「本地改动」里服务器的改动和你自己的改动混在一起；' +
            '点「更新」会先把基线跟上，再把两者重新分开。',
          actions: [{ id: 'svnUpdate', label: '更新…' }, REBUILD, CHECK],
          note: REBUILD_NOTE,
        }
      }
      return {
        tone: 'danger',
        title: '工作副本被回退到了基线之前',
        detail:
          `工作副本被回退到了 r${v.nodeRev}，而基线记着 r${v.baselineRev}。` +
          '基线比磁盘还新，任何基于它的比较都不成立。',
        actions: [REBUILD, CHECK],
        note: REBUILD_NOTE,
      }
    }
```

> 横幅上的「更新…」走的是和工具栏同一个 `handleSvnUpdate`（Task 10），也就是同一次预览和确认。pinned 仓库（A1）点它会在预览时被 `sync_in_pinned_unsupported` 拒绝，错误文案里写着该怎么做。横幅不知道仓库是不是 pinned，为这一种情况再传一个参数进来不值得。

`src/engine/toolbarActions.ts` 末尾加：

```ts
/**
 * 「更新」（sync_in）按钮只在 SVN 混合仓库上出现。
 *
 * 它不进 `ToolbarAction`：那一组是「一个 invoke 走完」的 git 远程操作，
 * 而更新要先预览、再确认、结局可能是冲突，由 RepoTab 编排（Task 10）。
 */
export function showsSvnUpdate(caps: Capabilities): boolean {
  return caps.vcs === 'svn'
}
```

`src/engine/svnJournal.ts`：

1. `import { syncActions } from './svnSyncIn'`。
2. `SyncPhase` 联合类型在 `'baseline-ready'` 之后加 `| 'applying-stash'`。
3. `PHASE_DETAIL` 加一项：

```ts
  'applying-stash':
    '基线已经更新，正在把未提交的改动放回来。如果提示冲突或有同名文件，处理好之后点继续；改动的备份一直在 stash 里。',
```

4. 删掉 `NOTE_NEXT_PHASE` 常量，换成：

```ts
const NOTE_SYNC_IN =
  '点「继续」从中断的地方接着走；点「放弃」回到更新前。两者都不会丢掉你的提交和未提交的改动。'
const NOTE_SYNC_OUT =
  '这条记录是交付流程留下的，恢复入口在后续版本提供。在那之前请不要手工去动 stash 或分支指针。'

/** 「已经持续多久」。与后端 `svn_lock::describe_age` 同一套说法。 */
export function sinceText(startedAt: number, nowSecs: number): string {
  const secs = Math.max(0, nowSecs - startedAt)
  if (secs < 60) return '刚刚开始'
  if (secs < 3600) return `已持续 ${Math.floor(secs / 60)} 分钟`
  return `已持续 ${Math.floor(secs / 3600)} 小时`
}
```

5. `describeUnfinishedSync` 签名改成 `(s: SyncState | SyncStateUnreadable | null, nowSecs: number = Math.floor(Date.now() / 1000))`；`locked` 分支的 `who` 改成：

```ts
    const who = s.holder
      ? `持有者是进程 ${s.holder.pid}，${sinceText(s.holder.startedAt, nowSecs)}。`
      : '还读不到持有它的进程信息（它可能刚刚开始）。'
```

6. 未收尾分支的 `return` 改成：

```ts
  const isSyncIn = s.unfinished.flow === 'sync_in'
  return {
    tone: 'danger',
    title: '上次同步没有正常结束',
    detail,
    actions: isSyncIn ? syncActions(s.unfinished.phase) : [],
    note: isSyncIn ? NOTE_SYNC_IN : NOTE_SYNC_OUT,
  }
```

- [ ] **Step 5：跑测试，确认通过；再跑全量与构建**

```powershell
pnpm vitest run tests/vitest/svnSyncIn.test.ts tests/vitest/svnJournal.test.ts tests/vitest/toolbarActions.test.ts tests/vitest/svnSync.test.ts
pnpm build
pnpm test
```

`pnpm build` 此时可能因为 `BaselineBanner.tsx` 没处理新的 action id 而**仍然通过**（它只 map 渲染）——这一步只确认 `tsc` 没被本 Task 打破。

- [ ] **Step 6：变异验证**

| 变异 | 期望只变红 |
|---|---|
| `expectFromPreview` 直接用 `p.outsideChanges`（不拷贝） | `把预览里的事实原样带回去做锁内复核` |
| `syncActions` 的 `start` 返回 `[CONTINUE, ABORT]` | `start 只有「清除记录」` |
| `describeUnfinishedSync` 对 `sync_out` 也给 `syncActions` | `sync_out 留下的记录本版本收不了尾` |
| `describeSync` 回退分支也带 `svnUpdate` | `被回退、多 revision 时不给「更新」` |
| `describeSyncInPreview` 忽略 `catchUpRev`（`from` 恒为 `baselineRev`） | `工作副本被外部整体更新过：…` |

- [ ] **Step 7：更新决策日志、提交**

```powershell
git add src/engine tests/vitest docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): 更新流程的确认框、结局与继续/放弃文案（纯逻辑）"
```

---

## Task 10：前端接线 + 真实窗口冒烟

**Files:**
- Modify: `src/store/repoStore.ts`（CRLF）
- Modify: `src/components/Toolbar.tsx`
- Modify: `src/components/BaselineBanner.tsx`（CRLF）
- Modify: `src/components/RepoTab.tsx`

**Interfaces:**
- Consumes: Task 8 的四个命令；Task 9 的全部导出。
- Produces: 用户可见的「更新」按钮；横幅上的继续 / 放弃按钮；「工作副本已在外部更新」横幅上的「更新…」按钮（计划 A16）。

- [ ] **Step 1：store 加四个 invoke**

`src/store/repoStore.ts`，紧挨着既有的 `export function svnSyncState(...)`：

```ts
export function svnSyncInPreview(path: string): Promise<SyncInPreview> {
  return invoke<SyncInPreview>('svn_sync_in_preview', { path })
}

/** `expect` 的字段名是 camelCase，与后端 `SyncInExpect` 的 serde 约定对齐（AGENTS.md §7.3）。 */
export function svnSyncIn(path: string, expect: SyncInExpect): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_sync_in', { path, expect })
}

export function svnSyncContinue(path: string): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_sync_continue', { path })
}

export function svnSyncAbort(path: string): Promise<SyncInOutcome> {
  return invoke<SyncInOutcome>('svn_sync_abort', { path })
}
```

顶部 import 补 `import type { SyncInExpect, SyncInOutcome, SyncInPreview } from '../engine/svnSyncIn'`。

- [ ] **Step 2：工具栏「更新」按钮**

`src/components/Toolbar.tsx`：

1. import 补 `showsSvnUpdate`。
2. `ToolbarProps` 加：

```ts
  /** SVN 混合仓库的「更新」（sync_in）。预览、确认、执行都由 RepoTab 编排。 */
  onSvnUpdate?: () => Promise<void>
```

3. 组件参数解构加 `onSvnUpdate`；加一个 `const [updating, setUpdating] = useState(false)`；在 `<span className="toolbar-sep" />` 之后、`visibleToolbarActions(...)` 之前渲染：

```tsx
      {showsSvnUpdate(capabilities) && onSvnUpdate && (
        <button
          className="toolbar-button"
          disabled={busy !== null || updating}
          onClick={() => {
            setUpdating(true)
            void onSvnUpdate().finally(() => setUpdating(false))
          }}
          data-action="svn-update"
        >
          {updating ? '…' : '更新'}
        </button>
      )}
```

- [ ] **Step 3：横幅按钮**

`src/components/BaselineBanner.tsx`：

1. `BaselineBannerProps` 加：

```ts
  /** 未收尾同步横幅上的「继续 / 放弃」。 */
  onSyncAction: (id: SyncActionId) => void
```

   import 补 `import type { SyncActionId } from '../engine/svnSyncIn'`，参数解构加 `onSyncAction`。
2. 未收尾横幅里 `{unfinished.note && (...)}` 之后加：

```tsx
          {unfinished.actions.length > 0 && (
            <div className="banner-baseline-actions">
              {unfinished.actions.map((a) => (
                <button key={a.id} onClick={() => onSyncAction(a.id as SyncActionId)}>
                  {a.label}
                </button>
              ))}
            </div>
          )}
```

（`a.id as SyncActionId`：`syncActions` 只产出这两个值，由 Task 9 的测试钉着；`BannerAction.id` 是所有横幅共用的联合类型，这里收窄一次。）

3. 失同步横幅上的「更新…」（计划 A16）。`BaselineBannerProps` 再加：

```ts
  /** 「工作副本已在外部更新」横幅上的「更新…」，与工具栏的「更新」是同一个入口。 */
  onSvnUpdate: () => void
```

   参数解构加 `onSvnUpdate`；失同步横幅的按钮分派（现在是 `if (a.id === 'rebuild') onRebuild()` / `else if (a.id === 'check') onCheck()` / `else if (a.id === 'updateWhitelistDir' ...)` 那一串）里，在 `'check'` 之后加一支：

```tsx
                  else if (a.id === 'svnUpdate') onSvnUpdate()
```

- [ ] **Step 4：RepoTab 编排**

`src/components/RepoTab.tsx`：

1. 从 `../store/repoStore` 的 import 补 `svnSyncAbort, svnSyncContinue, svnSyncIn, svnSyncInPreview`；新增：

```ts
import {
  describeSyncAction,
  describeSyncInPreview,
  describeSyncOutcome,
  expectFromPreview,
  type SyncActionId,
  type SyncInOutcome,
} from '../engine/svnSyncIn'
```

2. 在 `handleRebuildBaseline` 附近加：

```tsx
  /** 同步收尾之后的刷新。`repo-changed` 由后端补发，这里只补它不管的 SVN 侧状态。 */
  const afterSync = async () => {
    await refreshSvnSyncState(repoPath)
    await refreshSvnSync(repoPath)
    void fetchSvnLog(repoPath)
  }

  /**
   * 冲突、撞名、放弃这几种结局要让用户看见；成功不打扰。
   *
   * 走 `confirm` 只是借它的对话框外观——取消与确认在这里等价。
   */
  const showOutcome = async (o: SyncInOutcome | null) => {
    const n = o && describeSyncOutcome(o)
    if (!n) return
    await confirm({ title: n.title, message: n.detail, confirmLabel: '知道了' })
  }

  /**
   * 「更新」（spec §7）：预览 → 确认 → 执行。
   *
   * 预览失败（前置检查不过）直接报错，一个字节都没动。执行走 `action`：
   * 它要跑完整的 svn update（网络），确认完就关框的话界面上几秒毫无动静。
   * 执行出错时对话框留在原地显示错误——后端的错误文案里已经写了回退的结果。
   */
  const handleSvnUpdate = async () => {
    let preview
    try {
      preview = await svnSyncInPreview(repoPath)
    } catch (err) {
      reportError(err)
      return
    }
    // 用盒子而不是 `let outcome`：闭包里的赋值 TS 的控制流分析看不见，
    // `if (outcome)` 会被收窄成 never
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await confirm({
      ...describeSyncInPreview(preview),
      action: async () => {
        box.outcome = await svnSyncIn(repoPath, expectFromPreview(preview))
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  /** 横幅上的继续 / 放弃（spec §7.6）。 */
  const handleSyncAction = async (id: SyncActionId) => {
    const s = state.svnSyncState
    const phase = s && !('readError' in s) && s.unfinished ? s.unfinished.phase : 'start'
    const box: { outcome: SyncInOutcome | null } = { outcome: null }
    await confirm({
      ...describeSyncAction(id, phase),
      action: async () => {
        box.outcome =
          id === 'syncContinue' ? await svnSyncContinue(repoPath) : await svnSyncAbort(repoPath)
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }
```

3. `<BaselineBanner ...>` 加 `onSyncAction={(id) => void handleSyncAction(id)}` 与 `onSvnUpdate={() => void handleSvnUpdate()}`；`<Toolbar ...>` 加 `onSvnUpdate={handleSvnUpdate}`。

> ⚠️ `confirm` 的请求类型若没有 `danger` / `busyLabel` 以外的未知字段，展开 `describeSyncInPreview(...)` 会带进 `busyLabel`、`danger`、`warning`——它们都是 `ConfirmRequest` 已有的字段（见 `ConfirmDialog.tsx`）。若 `tsc` 报多余字段，说明字段名对不上，改 Task 9 的 `SyncConfirmModel` 去对齐 `ConfirmRequest`，不要在这里删字段。

- [ ] **Step 5：构建与测试**

```powershell
pnpm build
pnpm test
```

期望：`tsc` 通过，vitest 全绿。

- [ ] **Step 6：真实窗口冒烟（`AGENTS.md` §7.3 硬性要求）**

按 Task 11 写好的 E2E 清单跑**场景 1**（普通更新）与**场景 2**（冲突 → 继续）。这两条走通就证明四个命令的 invoke 参数名全对。预览的 `catchUpRev` 只有场景 7（外部在项目根 update）才非空，那一条在 Task 11 Step 4 跑。把结果记进决策日志「全量验证」表的 `pnpm tauri dev` 一行；**没跑就写没跑**。

若 Task 11 的清单还没写，先写清单（Task 11 Step 1），再回来做这一步。

- [ ] **Step 7：更新决策日志、提交**

```powershell
git add src docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md
git commit -m "feat(svn): 工具栏「更新」按钮与横幅上的继续/放弃"
```

---

## Task 11：E2E 清单、spec 修订、收尾

**Files:**
- Create: `docs/superpowers/e2e/2026-09-25-svn-phase2c-e2e.md`
- Modify: `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（CRLF）
- Modify: `docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in-decisions.md`

- [ ] **Step 1：写 E2E 清单**

照 2b 清单（`docs/superpowers/e2e/2026-09-25-svn-phase2b-e2e.md`）的结构：三个窗口（应用 / 操作 / 同事）、第 0 节自动定位 fixture、**所有代码块原样可粘贴、不含任何占位符**。第 0 节的定位与辅助函数照下面写（操作窗口一次性粘贴）：

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
if (-not $fix) { throw '没找到 fixture，先做 0.2' }
$WC = Join-Path $fix.FullName 'wc1'
$WC2 = Join-Path $fix.FullName 'wc2'
if (-not (Test-Path (Join-Path $WC '.git'))) { throw "$WC 下还没有 .git，先做 0.3" }
function Colleague([string]$rel, [string]$content) {
    svn update -q --non-interactive $WC2
    $p = Join-Path $WC2 $rel
    $isNew = -not (Test-Path $p)
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    [IO.File]::WriteAllText($p, $content)
    if ($isNew) { svn add -q --parents --non-interactive $p }
    svn commit -q --non-interactive -m "colleague $rel" $WC2
    if ($LASTEXITCODE -ne 0) { throw 'svn commit 失败' }
}
function Wc([string]$rel, [string]$content) {
    [IO.File]::WriteAllText((Join-Path $WC $rel), $content)
}
function G { git -C $WC @args; if ($LASTEXITCODE -ne 0) { throw "git $args 失败" } }
"WC = $WC"; "WC2 = $WC2"
```

场景（顺序即执行顺序；每个场景末尾给一段「收尾」命令，把残留改动提交掉，让下一个场景从干净状态开始）：

| # | 场景 | 造现场 | 应该看到 |
|---|---|---|---|
| 1 | 普通更新 | `Colleague 'Code/A.cs' 'class A { int s1; }'`；`Wc 'Code/Deep/D.cs' 'class D { int local; }'; G commit -qam 'local D'`；`Wc 'Code/Deep/D.cs' 'class D { int local; int wip; }'`；`Wc 'Code/New.cs' 'class New {}'` | 点「更新」→ 确认框写着「把 r1 更新到 r2」→ 点更新 → 框关掉、没有横幅；`Code/A.cs` 是 `s1`，`D.cs` 带 `wip`，`New.cs` 在；`G log --oneline -2 svn/base` 顶部是 `svn: r2`；`G stash list` 里没有 `mygitclient-sync`；`.git/mygitclient-sync.json` 不存在 |
| 2 | rebase 冲突 → 继续 | `Colleague 'Code/A.cs' 'class A { int s2; }'`；`Wc 'Code/A.cs' 'class A { int mine2; }'; G commit -qam 'local A'` | 「更新遇到冲突」对话框列出 `Code/A.cs`；红色横幅（phase `rebase`）带「我已解决，继续」「放弃，回到更新前」；**此时在应用里暂存文件是允许的**（锁已释放）。`Wc 'Code/A.cs' 'class A { int s2; int mine2; }'` → 在应用里暂存 `Code/A.cs` → 点「我已解决，继续」→ 横幅消失 |
| 3 | rebase 冲突 → 放弃 | 同场景 2 的造法（换成 `s3` / `mine3`），先记下 `$before = (git -C $WC rev-parse HEAD).Trim()` | 点「放弃，回到更新前」→「已放弃这次更新」→ `git -C $WC rev-parse HEAD` 等于 `$before`；`svn/base` 仍是新的 r4（分支落后于它，是合法状态） |
| 4 | 撞名 | `Colleague 'Code/N.cs' 'class N { int server; }'`；`Wc 'Code/N.cs' 'class N { int mine; }'`；`Wc 'Code/Deep/D.cs' 'class D { int wip4; }'` | 「未提交的改动还没放回来」列出 `Code/N.cs`；`D.cs` **还不是** `wip4`（一个字节都没放回）。`Move-Item (Join-Path $WC 'Code/N.cs') (Join-Path $WC 'Code/N.server.txt')` → 点「我已处理，继续」→ `N.cs` 是 `mine`、`D.cs` 是 `wip4` |
| 5 | 崩溃后继续 | 用下面「手工造一次中断」那段命令造出 phase `stashed` 的现场 | 横幅在约 1 秒内出现，带「继续」「放弃」→ 点「继续」→ 横幅消失，`D.cs` 是 `crashed`，`A.cs` 是 `s5` |
| 6 | 前置拒绝：merge | `G checkout -q -b side; Wc 'Code/Side.cs' 'class S {}'; G add Code/Side.cs; G commit -qm side; G checkout -q work; G merge --no-ff -m 'merge side' side` | 点「更新」→ 直接报错「基线之后有 merge commit」并列出 OID；`G log --oneline -1` 仍是那个 merge；没有横幅 |
| 7 | 早上先在项目根外部 update，再点「更新」（计划 A16 / A17，用户的日常） | 先把场景 6 的 merge 撤掉：`G reset -q --hard HEAD~1`（**只在 fixture 里**）；`Wc 'Code/Deep/D.cs' 'class D { int c7; }'; G commit -qam 'local D7'`；`Wc 'Code/W7.cs' 'class W7 {}'; G add Code/W7.cs; G commit -qm 'add W7'`；`Wc 'Code/W7.cs' 'class W7 { int wip7; }'`；`Colleague 'Code/S7.cs' 'class S7 {}'`；`Colleague 'Art/boss.psd' 'art7'`；最后在操作窗口模拟 TortoiseSVN 对项目根点 update：`svn update -q --non-interactive $WC` | 几秒内顶部出现**黄色**横幅「工作副本已在外部更新」，按钮依次是「更新…」「重建基线…」「体检基线」；此刻 `G status --short` 同时列出 ` M Code/W7.cs` 与 `?? Code/S7.cs`（服务器的新文件和自己的改动混在一起）→ 点横幅上的「更新…」→ 确认框第一段写着「已经在外部（TortoiseSVN）更新到了 r…」和「重新分开」→ 点更新 → 框关掉、横幅消失；`G status --short` **只剩** ` M Code/W7.cs`；`Get-Content (Join-Path $WC 'Code/S7.cs')` 是 `class S7 {}`；`Get-Content (Join-Path $WC 'Art/boss.psd')` 是 `art7`；`G log --oneline -2 svn/base` 两条都是 `svn: r…` 且 revision 相同（接管录一条、S7 再录一条）；`G stash list` 里没有 `mygitclient-sync` |
| 8 | 美术文件确认 + update 冲突（**最后跑**：它会让仓库停在失同步） | `Wc 'Art/boss.psd' 'local'`；`Colleague 'Art/boss.psd' 'server'` | 确认框的红色警告里列出 `Art/boss.psd`，按钮是危险色 → 点更新 → 对话框里显示错误，含 `Art/boss.psd` 与「TortoiseSVN」，末尾「已回到分支 `work`」；随后顶部出现黄色的「工作副本已在外部更新」横幅（S5 已经把工作副本更新了、基线没动）；此时点横幅上的「更新…」应当被拒绝并指名 `Art/boss.psd` 的 SVN 冲突——先去 TortoiseSVN 解决，这是对的；`svn status (Join-Path $WC 'Art/boss.psd')` 以 `C` 开头 |

场景 5「手工造一次中断」的命令（操作窗口）：

```powershell
Colleague 'Code/A.cs' 'class A { int s5; }'
Wc 'Code/Deep/D.cs' 'class D { int crashed; }'
$head = (git -C $WC rev-parse HEAD).Trim()
$base = (git -C $WC rev-parse svn/base).Trim()
$rev = [int]([regex]::Match((git -C $WC log -1 --format=%B svn/base | Out-String), '@(\d+) ').Groups[1].Value)
G stash push -u -m 'mygitclient-sync:manual5' -- Code
$oid = (git -C $WC rev-parse refs/stash).Trim()
G update-ref refs/mygitclient/sync/manual5 $oid
$j = [ordered]@{
    opid = 'manual5'; flow = 'sync_in'; phase = 'stashed'
    startedAt = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
    origBranch = 'work'; origHead = $head; stashOid = $oid
    baselineRevBefore = $rev; baselineOidBefore = $base
} | ConvertTo-Json -Compress
[IO.File]::WriteAllText((Join-Path $WC '.git\mygitclient-sync.json'), $j)
```

每个场景都要有「看到别的说明」对照表（照 2b 清单的写法），至少覆盖：没有「更新」按钮（capabilities / `showsSvnUpdate` 没接上）、点了没反应（invoke 参数名错，看 devtools console）、冲突后暂存被拒（锁没释放）、继续后横幅不消失（journal 没删）；场景 7 另加三条：横幅仍是红色「基线落后于工作副本」且没有「更新…」（`describeSync` 没改）、点「更新…」没反应（`BaselineBanner` 没接 `svnUpdate`）、停在「未提交的改动还没放回来」并列出 `Code/S7.cs`（A17 没生效）。

**清单写完后，两条机械检查：**

1. 把清单里所有 `powershell` 代码块抽出来，用 `[System.Management.Automation.Language.Parser]::ParseInput` 在 PowerShell 5.1 与 7 下各解析一遍，**零语法错误**。
2. 确认 `Colleague` 函数真能提交（它不依赖应用，可以脱离真实窗口验）：新开一个 PowerShell，`pwsh -File scripts\e2e-svn-fixture.ps1` 造一个 fixture；把第 0 节代码块里 `$fix = …` 那两行、`$WC2 = …` 那一行和三个函数定义粘进去（**跳过** `.git` 检查那一行——这个 fixture 没走初始化向导）；跑 `Colleague 'Code/Probe.cs' 'x'`，`svn log -l 1 $WC2` 能看到这次提交；最后 `Remove-Item -Recurse -Force $fix.FullName` 删掉这个 fixture。

- [ ] **Step 2：spec 修订**

`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（CRLF，用 Edit）：

1. §7.3 步骤块里 S5 那行下面加：

```text
     ⚠️ 成败不看退出码：有文本冲突或树冲突时 svn update 仍然退出 0（2026-09-25
        实测），必须事后 svn status --xml 查 item="conflicted" 或 tree-conflicted="true"
```

2. §7.3 步骤块里 S3 与 S8 两行下面各加：

```text
     ⚠️ checkout 之后核对切干净了：文件被以 share=Read 打开（Unity）时 checkout 会
        退出 0 却没把它写进去（2026-09-25 实测）
```

3. §18 修订记录最前面加一节：

```markdown
### 2026-09-25（十四）· Phase 2c 施工：`sync_in` 的实测补足

计划 `docs/superpowers/plans/2026-09-25-svn-phase2c-sync-in.md` 的「对 spec 的补足与偏离」
A1–A17 全部实测或经用户决定，要点：

- **pinned 模式下 `sync_in` 本期拒绝**（用户决定），§6.5.3 两步 update 推迟；
- **新增 phase `applying-stash` 与字段 `stashApplied`**，补上 S9 与 S10 之间的崩溃窗口（§7.6 表已更新）；
- **S10 apply 前先查未跟踪文件撞名**：撞名时 git 会半套 apply；
- **S5 成败不看退出码**：冲突时 svn update 退出 0；
- **S5 之后的「放弃」**：先断言 W-file 在 svn 看来干净，再 `checkout -f` 回原分支（用户决定）；
- **每次 checkout 之后核对**：Unity 式占用下 checkout 退出 0 却没写进文件；
- **外部整体 update 由「更新」接管**（用户决定，偏离 §7.2 第 2 条）：`nodeRevs` 单值且大于基线时，
  锁内先跑 §16 的重建基线、再走 S0–S11，服务器改动与本地改动重新分开；mixed-revision 与
  「退回基线之前」仍拒绝。失同步横幅对这种状态降为黄色并给「更新…」入口；
- **撞名预检放宽**：磁盘、stash、HEAD 三处同一个 blob 的不算撞名（外部 update 带来的服务器
  新增文件就是这个形状）。
```

4. §7.2 第 2 条下面加一行注记：

```markdown
   > 2026-09-25（Phase 2c，用户决定）：`nodeRevs` 单值且**大于** `baselineRev`（外部在项目根整体 update 过）不再拒绝，由执行时先重建基线再更新，见 §18「十四」。其余失同步照旧拒绝。
```

- [ ] **Step 3：全量验证**

```powershell
cd src-tauri
cargo fmt --check; cargo clippy --all-targets; cargo test
cd ..
pnpm build
pnpm test
```

把实际数字填进决策日志「全量验证」表。**没有 lint、没有 CI**，不写「lint 通过」「CI 通过」。

- [ ] **Step 4：跑完 E2E 清单全部场景**

真实窗口里逐个场景跑，勾选清单的 checkbox。**没跑的别勾**。有场景不通过：按 `superpowers:systematic-debugging` 先复现再修，修完补测试。

- [ ] **Step 5：独立 review**

`AGENTS.md` §4.1：合并前必须由独立于实现者的人或 agent 按 §13 rubric review。执行方式若是 subagent-driven，最终的整分支 review 满足这一条；若是主会话自己施工，派一个全新上下文的 opus review agent，并**让它先读决策日志的「偏离计划」**（rubric #12）。review 的阻断项修完才算完成。

- [ ] **Step 6：决策日志收尾、提交、推送**

```powershell
git add docs
git commit -m "docs(svn): Phase 2c E2E 清单与 spec 修订记录"
git push -u origin worktree-svn-phase2c-sync-in
```

开 PR（Draft）由用户决定，不自行创建。

---

## 本期不做（review 时不要当成遗漏）

| 项 | 为什么 |
|---|---|
| pinned 模式的两步 update、必测 #53 | A1，用户决定 |
| §9.4 恢复向导、必测 #22 / #28 / #38 / #64 / #67 / #68 | 2d |
| `sync_out` 及其全部必测 | Phase 3 |
| 2b N6：横幅让位时灰化保留其余横幅 | 本期没有改动横幅的渲染与色调；未收尾横幅现在有了可执行的出口，被它压住的其余横幅在收尾后自然回来。仍记为待办 |
| 2b R8：`sync_out` + 共用 phase 的文案测试 | `sync_out` 的语义要等 Phase 3 |
| 必测 #9 树冲突的端到端用例 | 视 Task 7 Step 5 的造 fixture 结果而定；解析层由 Task 2 覆盖 |
| S10 用 `--index` 保留暂存状态 | A8：会引入「索引冲突时拒绝」一类新失败，本期不做 |
| 外部造成的 mixed-revision、「退回基线之前」由「更新」接管 | A16 只接管整体前进。mixed-revision 要 §9.4 的 revert / 归一，归 2d；退回多半是误操作，自动接管没有正当理由 |
| 相邻行的 stash 冲突自动消解 | Task 7A 实测：git 的合并对相邻行比 svn 严。走现有的 `StashConflict` 出口，内容不丢；自己写一套行级合并风险远大于收益 |

