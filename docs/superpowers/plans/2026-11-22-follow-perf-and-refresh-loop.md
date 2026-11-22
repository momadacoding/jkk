# 冲突后刷新死循环 + 跟进提速 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 止住「手动跟进停在 rebase 冲突后」的刷新死循环，并去掉自动跟进 / 手动跟进里三处白跑的开销。

**Architecture:** 死循环先在 fixture 里用真实 watcher 复现，再按测出来的路径修（不预设根因）。提速三项都是「同一次跟进里别重复做同一件事」：暂存区判定提前到第二遍准备之前，手动跟进的只读前置检查共用一次扫描，wc.db floor shape 在一次跟进里按 `(单元, floor)` 缓存。

**Tech Stack:** Rust（`src-tauri`）、notify、git CLI / libgit2、svn CLI；测试走 `src-tauri/tests/` 的 SVN fixture（`common/svn_fixtures.rs`）。

**Spec:** 2026-11-21 使用报告（jkk 0.10.1 带 `--perf`，对话里贴的原文；没有入库，因为里面是用户仓库的现场数据）。本计划引用它的节号：§2.1 死循环、§2.2 暂存区预判、§2.3 第二遍体检、§2.5 共用扫描、§2.7 floor shape。代码位置以 master `c312ddd` 为准。

## 背景（从报告摘录，执行时不用回看报告）

- §2.1：手动跟进的 `git rebase` 停在冲突上之后，`repo-changed` 平均每秒 0.85 次，每次一整轮刷新，持续 30 分钟以上。每轮开始后 0.4–0.55 s 来一次事件；减去 300 ms 去抖，写入发生在一轮开始后 0.1–0.25 s。`read_conflicts`、`read_status` 本身只读（已核对 `conflicts.rs:337`、`diff.rs:308`）。journal `.git/mygitclient-sync.json` 不在 `is_relevant` 的排除范围里（`watch.rs:331` 只排除 `.git/mygitclient/`）。
- §2.2：自动跟进第一遍被冲突拦下 → 第二遍部分跟进（约 35 s）→ 发布前 `compose_index` 才发现暂存区保不住，返回 `svn_auto_follow_stage`。`stage_blocker` 在 `svn_follow.rs:1164`。
- §2.5：`manual_follow` 的 `check_sync_with_nodes`（`svn_follow.rs:1278`）和 `run_preflight_with`（`:1286`）各扫一遍全量 `svn status -v`，外面没有 `with_scan_cache`。
- §2.7：同一次跟进里 `wc.db floor shape` 读了两次（各 1.5 s），两次之间工作副本没被 SVN 改过。

## 不做

| 报告项 | 为什么这次不做 |
|---|---|
| §2.3 第二遍不重做服务器体检 | 关系到墓碑确认的正确性，报告要求先出方案比较 A/B；T4 做完后本次场景到不了第二遍。只修正 `prepare_auto` 上那句与行为不符的注释（T4 顺手） |
| §2.4 `git stash -u` | 报告要求先量再定，涉及本地改动能否完整放回 |
| §2.6 服务器核对与跟进互斥 | 把握低，收益不确定 |
| §3 全量 `svn status` 基础开销 | 报告本身没有方案 |

## Global Constraints

- 读走 libgit2、写走 git CLI，git CLI 一律经过 `run_git` 系列（AGENTS §7.3）。
- 不加依赖（npm / crate）。
- 所有 git / svn 写操作只在 fixture 里做（AGENTS 总规则）。
- 不改 Tauri 命令名和参数。
- Rust 注释用中文，写「为什么」。
- 全量 `cargo test` 会被内存回收：按测试二进制分批跑，每批 < 10 分钟。

## Review Focus

1. 停在冲突上时，**外部**改 `.git`（终端里 `git add` 解决冲突）仍然要触发刷新 —— T1 的正向对照。
2. 停在冲突上时，工作区文件被 IDE 改了仍然要刷新 —— T1 的修法不能动工作区分支的判定；T1 第 6 步的单测覆盖。
3. T4 预判不能误拦：暂存的文件正是被留在旧基线上的冲突文件、或是分支提交改过的文件时，不提前拦，结局与现在相同 —— T4 测试 b、c。
4. T4 预判不留残留：没有 `.git/index.lock`、没有 `refs/mygitclient/follow/*`、没有 journal —— T4 测试 a 的断言。
5. T2 的缓存范围不能盖住 `stash_wip` / rebase：之后的读取必须是新的 —— T2 只包两行只读检查，`manual_follow_*` 现有集成测试全过。

---

### Task 1：rebase 冲突后的刷新死循环

**Files:**
- Modify: `src-tauri/src/commands/watch.rs`（`is_relevant` 改为 `pub`；`Wake::Yes` 时打 perf 记录；修法视第 3 步结果）
- Test: `src-tauri/tests/svn_follow.rs`（复用 `conflict_scene` / `expect_rebase_conflict`）
- 修法可能涉及的文件：第 3 步测出来的那条写入的源头

**Interfaces:**
- Produces: `pub fn is_relevant(repo: Option<&git2::Repository>, root: &Path, target: &Path) -> bool`（行为不变，只放开可见性，给集成测试用）

- [ ] **Step 1：写复现测试 `refresh_round_while_stopped_on_a_rebase_conflict_writes_nothing_the_watcher_cares_about`**

  在 `tests/svn_follow.rs` 里写：`hybrid_with_colleague()` → `conflict_scene` → `expect_rebase_conflict`。用 `notify::RecommendedWatcher` 按 `engine::watch::watch_targets` 挂监听，先排空 1 s。然后按前端 `repo-changed` 处理函数（`repoStore.ts:1727` 起，加上 `RepoTab.tsx:389` 的 `repo_state`）调一轮命令函数：`open_repo`、`load_graph`、`read_status`、`read_conflicts`、`read_worktrees_cmd`、`read_upstream`、`read_stashes`、`check_merge_drivers`、`repo_state`、`svn_wc_db_stamps`、`svn_sync_state`、`svn_branches_behind`，冲突文件 `Code/A.cs` 的 `read_diff`（unstaged），以及第二档的 `svn_sync_status`。调两轮（第一轮可能有一次性的合理写入，比如首次刷新 index stat），只收第二轮之后 1.5 s 的事件。

  断言：

  ```rust
  let noisy: Vec<_> = seen.iter().filter(|p| is_relevant(Some(&repo), &root, p)).collect();
  assert!(noisy.is_empty(), "停在冲突上时一轮只读刷新写了监听会认的路径：{noisy:?}");
  ```

  正向对照放在同一个测试末尾：`git add Code/A.cs` 之后 1 s 内必须收到一条 `is_relevant` 为真的事件，否则 watcher 没工作，前面的「没有事件」不算数。

- [ ] **Step 2：跑，确认红**

  Run: `cargo test --test svn_follow refresh_round_while_stopped -- --nocapture`
  Expected: FAIL，失败信息里列出被写的路径。**如果是绿的**：fixture 没复现报告的现场。停下来比对现场差异（journal phase、冲突文件是否被选中、`svn_sync_status` 的第二档），把差异补进测试，直到红为止；每次尝试都写进决策日志。

- [ ] **Step 3：按失败信息里的路径定位源头，写进决策日志「计划没写的坑」**

  三种可能，修法分别是：
  | 测出来的 | 修法 |
  |---|---|
  | 某个只读命令在冲突态下写了文件（如 journal、index、`rebase-merge/*`） | 去掉那次写入，或改成内容不变时不写 |
  | 写入合理，但内容不影响任何面板 | 在 `is_relevant` 里排除这个路径，注释写清为什么排除是安全的 |
  | 某个 git CLI 读命令顺手刷新了 index | 给那条命令加 `--no-optional-locks`，或改走 libgit2 |

- [ ] **Step 4：实现修法，跑第 1 步的测试，确认绿**

- [ ] **Step 5：perf 打点**

  在 watcher 线程里，`Wake::Yes` 置 `pending` 时，如果 `perf::enabled()`，记下这一批的第一条路径。发 `repo-changed` 时写一条 `perf::Record::rust("watch", "repo-changed cause")`，`meta.path` 是这条路径。这样下次现场日志能直接看到循环源头，不用开 Process Monitor。

- [ ] **Step 6：如果第 3 步改了 `is_relevant`，在 `watch.rs` 的 `mod tests` 里补单测**

  被排除的路径返回 `false`；同目录下的 `HEAD`、`index`、`refs/heads/x` 和工作区文件仍是 `true`。

- [ ] **Step 7：验证并提交**

  Run（`src-tauri/` 下）：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test --lib commands::watch`、`cargo test --test svn_follow`
  Commit：`fix(watch): 手动跟进停在 rebase 冲突上时不再刷新死循环`

### Task 2：手动跟进的只读前置检查共用一次扫描（§2.5）

**Files:**
- Modify: `src-tauri/src/engine/svn_follow.rs:1278-1287`
- Test: Create `src-tauri/tests/svn_follow_manual_scan_budget.rs`（单测试二进制，理由同 `svn_follow_scan_budget.rs`：perf 记录器一个进程只能装一次）

**Interfaces:**
- Consumes: `svn_cli::with_scan_cache`（已有）

- [ ] **Step 1：写测试 `manual_follow_prechecks_share_one_status_scan`**

  照抄 `svn_follow_scan_budget.rs` 的 `scans()` 计数法。场景是 `svn_follow.rs` 的 `conflict_scene` 同款（`follow` 返回 `NeedsManual { baseline_published: true }`），然后 `manual_follow`。断言 `manual_follow` 期间真实执行的全量 `svn status -v` 次数比改前少 1。改前的次数在第 2 步实测，记进决策日志，再把断言写成精确值。

- [ ] **Step 2：跑，记下改前次数，确认红**

  Run: `cargo test --test svn_follow_manual_scan_budget`

- [ ] **Step 3：把 `check_sync_with_nodes` 到 `run_preflight_with` 这一段（含 `require_changes_in_whitelist`，如果它也读 status）包进 `with_scan_cache`**

  范围不能扩到 `svn_journal::write` 之后：`stash_wip` 和 rebase 会改工作副本。

- [ ] **Step 4：跑新测试和 `cargo test --test svn_follow manual_follow`，确认绿**

- [ ] **Step 5：fmt / clippy，提交** `perf(svn): 手动跟进的前置检查共用一次 svn status 扫描`

### Task 3：同一次跟进只读一次 wc.db floor shape（§2.7）

**Files:**
- Modify: `src-tauri/src/engine/svn_wcdb.rs`（`floor_shapes`）、`src-tauri/src/engine/svn_cli.rs`（暴露扫描范围的代次）
- Test: `src-tauri/tests/svn_follow_scan_budget.rs`（在现有唯一的测试里加断言）

**Interfaces:**
- Produces: `pub(crate) fn scan_scope_generation() -> Option<u64>`（`svn_cli.rs`；在 `with_scan_cache` 最外层进入时 +1，不在范围内是 `None`）

- [ ] **Step 1：先查两次读的来源，写进决策日志**

  在 `svn_follow_scan_budget.rs` 的自动跟进段，数 `"name":"wc.db floor shape"` 的记录数；临时打日志看两次调用的 `floor` 和是否在同一个 `with_scan_cache` 范围里。
  - 两次 `floor` 不同，或第二次在范围之外 → 本 Task 不做，决策日志写明原因，跳到 Task 4。
  - 相同且在同一范围 → 继续。

- [ ] **Step 2：加断言 `floor_shapes_per_follow == 1`，跑，确认红**

- [ ] **Step 3：在 `floor_shapes` 里按 `(scan_scope_generation, wc_root, floor)` 缓存 `FloorShape`（thread-local，代次变了就清空），只在 `scan_scope_generation()` 为 `Some` 时读写缓存**

  `FloorShape` 要 `Clone`。perf 记录只在真读 wc.db 时写，命中缓存不写。

- [ ] **Step 4：跑 `svn_follow_scan_budget`、`svn_health_local_floor`、`svn_follow_auto`，确认绿**

- [ ] **Step 5：fmt / clippy，提交** `perf(svn): 一次跟进里 wc.db floor shape 只读一次`

### Task 4：自动跟进的暂存区判定提前到第二遍准备之前（§2.2）

**Files:**
- Modify: `src-tauri/src/engine/svn_follow_index.rs`（新增只读判定）、`src-tauri/src/engine/svn_follow.rs`（`prepare_auto`、`stage_blocker` 的调用、`:496-497` 的注释）
- Test: `src-tauri/tests/svn_follow_auto.rs`

**Interfaces:**
- Produces: `pub fn judge_staging(repo: &str, old: &str, new: &str, facts: &dyn Fn(&str) -> PathFacts) -> Result<ComposeReport>`（`svn_follow_index.rs`）。不拿锁、不钉 ref、不写索引：读真实 `.git/index`，`diff-index --cached old` 与 `tree_changes(old, new)`，再调 `compose`，返回 `plan.report`。`compose` 里的 `merge-file -w` 只往对象库写 blob，watcher 不认。

**为什么用 `(old_base, new_base)` 判出来的结论与发布时 `(old_head, tip)` 一样：** 对一条既不在 `held` 里、也没被分支提交改过的暂存路径 P，`old_head:P == old_base:P`；第二遍只把 `held` 留在旧基线上，所以 `tip:P == new_base:P`。两组输入在 P 上相同，`compose` 对 P 的决定也相同。所以只用这类路径的 `lost` / `uncertain` 提前拦。其余路径照旧留到发布时判，不会误拦。预判是在没拿锁时读的索引：它判「拦」就整次跳过，什么都没动；判「不拦」就照原流程走，发布时持锁重判。两种结论都不会比现在更危险。

- [ ] **Step 1：写三个测试（`svn_follow_auto.rs`）**

  - a `partial_follow_that_would_lose_staging_skips_before_the_second_pass`：同时造「暂存的 `Code/B.cs` SVN 上也改了、合不到一起」和「分支提交在 `Code/A.cs` 上重放冲突」。断言 `Skipped { code: "svn_auto_follow_stage", .. }`，消息里有 `Code/B.cs`；`svn/base` 与分支 ref 都没动；没有 `.git/index.lock`、没有 `refs/mygitclient/follow/*`、`svn_journal::read` 为 `None`；暂存区里 `Code/B.cs` 还是原暂存内容。perf 记录里 `health floor answered locally` 只出现一次。如果这一条和 `svn_follow_auto.rs` 别的测试抢 perf 记录器，就单独放进一个新二进制。
  - b `staging_on_the_held_conflict_file_is_not_judged_early`：暂存的正是冲突文件 `Code/A.cs`。断言结局与改前相同（改前实测，记进决策日志）。
  - c `staging_on_a_path_the_branch_changed_is_not_judged_early`：暂存的路径被分支提交改过，又不在 `held` 里。断言结局与改前相同。

- [ ] **Step 2：跑，确认 a 红、b / c 绿**

  Run: `cargo test --test svn_follow_auto staging -- --nocapture`

- [ ] **Step 3：实现 `judge_staging`；`prepare_auto` 在 `held_candidates` 返回 `Some(held)` 之后、第二遍 `prepare` 之前调用它**

  用 `plan.old_base` / `plan.new_base`。`facts` 与 `compose_index` 相同：`who.facts(p, staged_blobs(repo))`。过滤掉 `held` 里的和 `tree_diff(old_base, old_head)` 改过的路径，剩下的 `lost` / `uncertain` 非空，就把 `stage_blocker` 的错误作为 `AutoSkip` 返回（`conflict: None`）。判定本身出错（`Err`）时只记 `log::info!`，照原流程走第二遍：预判是提速，不是判据。

- [ ] **Step 4：改 `svn_follow.rs:496-497` 的注释**

  改成与行为一致：第二遍复用第一遍录好的基线；floor 下降时会重新确认墓碑（`confirm_tombstones`），所以可能联网。

- [ ] **Step 5：跑 `svn_follow_auto`、`svn_follow_index`、`svn_follow`，确认全绿**

- [ ] **Step 6：fmt / clippy，提交** `perf(svn): 部分跟进前先判暂存区，保不住就不跑第二遍`

### Task 5：全量验证、独立 review、Draft PR

- [ ] Rust：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test --lib`，再按测试二进制分批跑全部 `tests/*.rs`
- [ ] 前端：本计划不改前端；若 T1 修法涉及前端，补跑 `pnpm build`、`pnpm test`
- [ ] 独立 review（`/code-review high` 或独立 subagent），按 AGENTS §13；高危问题修完再往下走
- [ ] 决策日志进度总账更新完，推分支，开 Draft PR（`gh pr create --draft`）
