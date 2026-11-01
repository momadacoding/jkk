# SVN 混合版本基线 C1a Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** TortoiseSVN 只提交、不更新，或者只对子目录局部更新之后，jkk 点一下「跟进」，就把基线推进到各文件的真实 revision；「本地改动」只剩还没提交到 SVN 的改动；暂存区、分支和 stash 都不受损。

**Architecture:**
- 基线 commit 的 message 从单一 `git-svn-id` 扩展为「floor + 每文件 revision + 删除墓碑」（`BaseMap`）。
- 失同步判定改为按文件比较映射（节点 revision 与最后修改 revision）。
- 新增「跟进」流程，分两段：
  - 锁外只建对象：录新基线、分组体检、对象层重放各分支。
  - 锁内按固定顺序发布：基线 → 拼好的索引写进 index.lock → CAS 分支 → rename。

**Tech Stack:** Rust（git2 读、git CLI 写，经 `run_git`）、Tauri 2 命令、React 18 + TS、Vitest、cargo 集成测试（`file://` SVN fixture）。

**Spec:** `docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md`（下称「分析」）。另外必读同目录的 `2026-10-31-svn-mixed-baseline-c1a-decisions.md`：实测 E13–E21、W17–W28 和「计划的错误 / 偏离」优先于分析原文。

## Global Constraints

- **git 版本：** git ≥ 2.43（`merge-file --object-id` 要 2.43，原写的 2.40 不够，见日志 D9）。在跟进的 F1 之前预检，错误码 `svn_follow_git_too_old`。
- **依赖：** 不新增任何 npm 包或 crate。百分号编码自己写。按 hunk 的比较用已有的 `git2::Patch`。
- **git 调用约束：**
  - 所有 git CLI 调用走 `run_git` / `run_git_code`，错误映射为 `GitError`。
  - 凡是带 pathspec 的调用，一律加 `--literal-pathspecs`（W20）。
  - `GIT_INDEX_FILE` 一律传绝对路径（W19）。
- **commit message：**
  - 用 `commit-tree -F <临时文件>` 写入 UTF-8，读回用 `git cat-file commit`（W27）。
  - trailer 只放在最后一段，自己解析，不用 `interpret-trailers`。
- **注释风格：** 中文，解释「为什么」，与现有代码一致。
- **IPC：** 改了 IPC 的结构体或命令，必须同步：
  - Rust 的 serde 属性（变体带 `rename_all_fields = "camelCase"`）；
  - `tests/svn_backend.rs` 的序列化契约测试；
  - 前端 TS 类型和对应的 vitest。
- **测试：**
  - 所有 git 写操作的测试都用 `src-tauri/tests/common/svn_fixtures.rs` 里的临时仓库，或在该文件里新增 fixture。
  - 测试名必须如实反映它覆盖的形态。
- **验证与提交：**
  - 每个 Task 完成后跑对应侧的验证：
    - Rust：在 `src-tauri/` 下跑 `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test <相关测试>`。
    - 前端：`pnpm build`、`pnpm test`。
  - 不 commit，除非用户放行（AGENTS.md §1.1；见日志【待决】）。
  - 每完成一个 Task，立即更新 decisions 日志的总账。
- **需要用户拍板的点：** 先取保守的默认，在日志里记【待决】，收尾时回修。

## Review Focus

1. **多个 wc(P)（白名单落在 external 里）：** 映射的键是仓库相对路径，status 要逐个 wc 跑并合并结果。两个单元各自有一个文件被外部提交时，两个都要判成 changed。由 Task 3 负责测试。
2. **路径带空格、`%`、中文：** 写进 trailer 再读回要逐字节一致；带 pathspec 的 git 调用不能把它当 glob。由 Task 1、Task 6 负责测试。
3. **跟进时索引里有 assume-unchanged / skip-worktree / i-t-a / 冲突条目：**
   - 有冲突条目时拒绝跟进；
   - assume-unchanged / skip-worktree 标志要保留；
   - i-t-a 条目不能丢。
   由 Task 8 负责测试。
4. **发布期间外部工具（Rider）操作 git：**
   - `git add` / `commit` 会被 index.lock 挡回；
   - `checkout -b` 能绕过 index.lock 把 HEAD 改走，第二次核对 HEAD 时必须发现并回滚。
   由 Task 10 负责测试。
5. **崩溃在发布的任意两步之间：** 重开后由 journal 补做到完成，或回滚到一致状态。不能留下「分支已移、索引是旧的」并且 index.lock 不存在的状态。由 Task 10 负责测试。

---

### Task 1: BaseMap 记账格式

**Files:**
- Create: `src-tauri/src/engine/svn_basemap.rs`（并在 `engine/mod.rs` 注册）
- Modify: `src-tauri/src/engine/svn_baseline.rs:645-692`（`create_baseline_commit_with` 改用 `-F`，message 由调用方给出的 `BaseMap` 生成）
- Test: `src-tauri/tests/svn_basemap.rs`

**Interfaces:**
- Produces:
  - `pub struct BaseMap { pub repos_root: String, pub uuid: String, pub floor: u64, pub revs: BTreeMap<String, u64>, pub gone: BTreeMap<String, u64> }`
    - `revs` 只存 revision ≠ floor 的文件，其余文件隐含 floor；`gone` 存墓碑（路径 → 删除确认时的 HEAD）。
  - `impl BaseMap { pub fn single(repos_root, uuid, rev) -> Self; pub fn is_single(&self) -> bool; pub fn rev_of(&self, path: &str) -> u64; pub fn entry_count(&self) -> usize }`
  - `pub fn format_message(map: &BaseMap, summary_extra: &[String], delivered_rev: Option<u64>) -> String`
    - `is_single()` 时与现行格式逐字节相同（`svn: rN\n\n[Delivered-Rev: ..\n]git-svn-id: ...\n`）。
    - 否则写成：首行 `svn: r<floor>（另有 N 个文件在更新的 revision，M 个文件已删除）`，空行，`summary_extra` 每行一条（例如「跟进 r1245–r1250：A、B」），空行，最后一段 trailer。trailer 依次是：`Svn-Base-Repo: <root> <uuid>`、`Svn-Base-Floor: <floor>`、按路径排序的 `Svn-Base-Rev: <rev> <编码路径>`、按路径排序的 `Svn-Base-Gone: <rev> <编码路径>`，可选 `Delivered-Rev:`。混合时**不写** `git-svn-id`。
  - `pub fn parse_message(msg: &str) -> Option<BaseMap>`：先认新 trailer，再退回 `parse_trailer`（旧格式 → `single`）。
  - `pub fn encode_path(p: &str) -> String` / `pub fn decode_path(s: &str) -> Option<String>`：把 `%`、空格、`\t`、`\r`、`\n` 以及其他 < 0x20 的字节编成 `%XX`。
  - `pub fn read_base_map(repo: &str, rev: &str) -> Result<BaseMap>`：用 `git cat-file commit <rev>` 取 message 再解析。错误码沿用 `svn_baseline_no_trailer`。
  - `pub const MAX_MAP_ENTRIES: usize = 5000;`【待决：上限值】

- [x] **Step 1: 写失败测试**
  - `single_map_formats_exactly_like_legacy_trailer`：与 `format_trailer` 拼出的旧 message 字节相等。
  - `mixed_map_round_trips_with_spaces_percent_and_cjk`：路径 `道具/装备 名称.lua`、`a%b.lua`，往返后相等。
  - `mixed_message_has_no_git_svn_id_line`。
  - `parse_message_reads_legacy_baseline`。
  - `message_with_1200_entries_round_trips_through_commit_tree`：经 fixture 仓库 `commit-tree -F` 写入，再 `read_base_map` 读回相等。
- [x] **Step 2: 跑测试确认失败**（`cargo test --test svn_basemap`）。
- [x] **Step 3: 实现。** `create_baseline_commit_with(repo, tree, parent, map: &BaseMap, summary_extra, delivered_rev)` 把 message 写进 `git_dir()` 下的临时文件，用 `-F` 传入。旧签名的调用方（init、rebuild、sync_in S7、sync_out D8a）改为传 `BaseMap::single(..)`，行为不变。
- [x] **Step 4: 测试通过**；`cargo test --test svn_baseline --test svn_init --test svn_sync_in --test svn_sync_out` 不回归。

### Task 2: 节点状态解析与映射计算

**Files:**
- Modify: `src-tauri/src/engine/svn_xml.rs`（新增解析函数；不改 `parse_node_rev_entries`）
- Create: `src-tauri/src/engine/svn_nodes.rs`
- Test: `src-tauri/tests/svn_nodes.rs`

**Interfaces:**
- Produces:
  - `pub struct NodeStatus { pub path: String, pub item: String, pub node_rev: Option<u64>, pub commit_rev: Option<u64>, pub author: Option<String>, pub wc_locked: bool, pub tree_conflicted: bool, pub changelist: Option<String> }`
    - `path` 是相对 repo_root、用正斜杠的 entry path。
    - 遍历整份文档所有 `<entry>`（E14），`changelist` 取父元素 `<changelist name>`。
  - `pub fn parse_node_status(xml: &str) -> Result<Vec<NodeStatus>>`：保留 `assert_externals_were_ignored`。
  - `pub fn read_node_status(repo: &str, whitelist: &[String], wcs: &[WcIdentity]) -> Result<Vec<NodeStatus>>`：targets 与 `svn_sync::node_revs` 相同，结果合并去重。
  - `pub fn compute_map(repos_root, uuid, files: &BTreeSet<String>, nodes: &[NodeStatus], gone: BTreeMap<String,u64>) -> Result<BaseMap>`
    - F = files 里各文件 node_rev 的最小值；c(f) 缺失时取 n(f)；c(f) ≤ F 的文件隐含 F，其余记 n(f)（日志「映射取值」条目）。
    - 文件不在 nodes 里、或 `node_rev` 缺失时，报错 `svn_map_unknown_node`。
    - 结果超过 `MAX_MAP_ENTRIES` 时报错 `svn_map_too_large`，错误信息列出 revision 最低的 10 个文件。

- [x] **Step 1: 写失败测试**
  - `parse_node_status_reads_commit_rev_author_and_changelist`：用固定的 XML 字符串（照 E13、E14 的形状）。
  - `parse_node_status_marks_wc_locked_dirs`。
  - `compute_map_keeps_untouched_files_on_floor_after_partial_update`：E12 形态；用 fixture 实跑 `svn update Bag`，只有 b1 进入 `revs`。
  - `compute_map_after_root_update_is_single`。
  - `compute_map_records_own_commit_at_node_rev`：只提交了 a。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑测试确认通过。**

### Task 3: 文件级失同步判定

**Files:**
- Modify: `src-tauri/src/engine/svn_sync.rs`（`check_sync`、`SyncVerdict`、`SyncStatus`）
- Modify: `src-tauri/tests/svn_backend.rs:764-834`（契约）、`src-tauri/tests/svn_sync.rs`
- Modify: `src/engine/svnSync.ts`（类型、`describeSync`）、`tests/vitest/svnSync.test.ts`、`tests/vitest/BaselineBanner.test.tsx`

**Interfaces:**
- Consumes: Task 1 的 `read_base_map`；Task 2 的 `read_node_status`。
- Produces:
  - `SyncVerdict` 删掉 `MixedRevision`，新增：
    - `ExternallyAdvanced { changed: Vec<ExternalChange>, added: Vec<ExternalChange>, removed: Vec<String> }`
    - `WcLocked { dirs: Vec<String> }`
  - `pub struct ExternalChange { path, rev: u64 /*c(f)，缺失时取 n(f)*/, author: Option<String> }`（camelCase）。
  - `Diverged { node_rev, baseline_rev }` 保留，表示任一文件 n(f) < R(f)，取第一个这样的文件的两个值。`PinnedReset` 的识别不变。
  - `SyncStatus { baseline_rev /*= floor*/, node_revs /*文件节点 revision 并集*/, verdict, base: BaseMapSummary { floor, newer: usize, gone: usize } }`
  - 前端：`SyncVerdict` 联合类型同步修改。`describeSync`：
    - `externallyAdvanced` 显示黄色横幅，标题「SVN 侧有 N 个文件有新内容」，列出路径、revision、作者；只给一个按钮 `{ id: 'follow', label: '跟进' }`，不给体检按钮。
    - `wcLocked` 显示灰色横幅「SVN 正在更新…」，不给按钮。
- 判定规则以分析 §6.2 第六轮版本为准（逐键：n<R 回退；c>R 前进；否则相等）。另外：
  - 任一目录 `wc_locked` 为真时，直接返回 `WcLocked`，优先级最高（E15）。
  - 外部新增：entry 有 node_rev、`item` 不是 added / unversioned、`WScope` 判为 W-file、磁盘上是文件、且不在 R 和 gone 里。
  - 消失：R 的键在 nodes 里找不到。
  - 墓碑路径重新出现时，算外部新增。

- [x] **Step 1: 写失败测试（Rust，fixture）**
  - `commit_without_update_is_externally_advanced_not_mixed`：`svn_repo_committed_without_update` 形态。
  - `partial_update_without_content_change_stays_in_sync`：E12。
  - `partial_update_with_colleague_change_lists_colleague_as_author`。
  - `committed_deletion_is_reported_as_removed`：E7，补分析 §1.4 的盲区二。
  - `rolled_back_file_is_diverged`。
  - `locked_working_copy_reports_wc_locked`：fixture 里用 `SVN_EDITOR` 卡住一个 `svn ci` 造活锁，或用 E15 的方法。
  - `changes_in_two_externals_are_both_reported`：Review Focus 1。
  - 契约：`externally_advanced_serializes_camel_case`。
- [x] **Step 2: 写失败测试（vitest）**：`describeSync` 对 `externallyAdvanced` 只给 `follow` 按钮、列出作者；对 `wcLocked` 不给按钮。
- [x] **Step 3: 实现两侧。** 同时检查所有消费 `MixedRevision` / `mixedRevision` 的代码（`svn_sync_check.rs:90-109`、`svnSync.ts:95-111`、`BaselineBanner`），一并改掉。sync_in 的 A16 分支暂时把 `ExternallyAdvanced` 当作「拒绝，请先跟进」，在 Task 4 里接管。
- [x] **Step 4: 两侧验证**（cargo 三件套 + `pnpm build` + `pnpm test`）。

### Task 4: 分组体检、墓碑与服务器侧核对

**Files:**
- Modify: `src-tauri/src/engine/svn_health.rs`（新增 `check_baseline_map`；`check_baseline` 改为调用它的单值特例）
- Modify: `src-tauri/src/engine/svn_server_set.rs:74-110`
- Create: `src-tauri/src/engine/svn_tombstone.rs`
- Test: `src-tauri/tests/svn_health.rs`、`src-tauri/tests/svn_server_set.rs`

**Interfaces:**
- Consumes: Task 1 的 `BaseMap`。
- Produces:
  - `pub fn check_baseline_map(repo, svn_url, map: &BaseMap, scope: &WScope, explained: &BTreeSet<String>, origin: ExplainedOrigin) -> Result<HealthMap>`
  - `pub enum HealthMap { Verified { health: Health, new_gone: BTreeMap<String,u64> }, Excluded { files: Vec<String> } }`
    - `new_gone` 只在 `JustRecorded` 时产生：floor 组里没被墓碑解释、也不在 S 里的 D，向服务器确认后成为墓碑。
    - `Excluded`：这些 D 在服务器 HEAD 上仍然存在（漏收，E17），调用方拒绝录制，错误码 `svn_baseline_excluded_paths`。
  - `pub fn server_head_rev(repo, repos_root) -> Result<u64>`（从 `svn_sync_out_check::remote_head_rev` 提出来共用）。
  - `pub fn confirm_absent(repo, urls_at: &[(String /*rel*/, String /*url@rev*/)]) -> Result<BTreeMap<String, bool /*存在*/>>`
    - 用 `svn info --xml --targets <临时文件>`，按 `<url>` 对回（E16）。
    - rc=1 时仍解析 stdout；出现 E160006 时报错。
  - `audit_server_set` 用 floor 的清单，并剔除墓碑路径。
- 体检命令的形状照 E18：
  - floor 组：一条 `svn diff --old=<unit_url>@<F> --new=<wc_root> --summarize --xml --ignore-properties`，从结果里剔除不在 floor 组的文件。
  - 其余每组：一条 `--old=<unit_url>@<r> --new=<wc_root> <PATH>...`，按命令行长度分批，每批不超过 24000 字符。发命令前用 status 剔除未版本化和丢失的 PATH。
  - 墓碑：校验 `svn info <unit_url>/<rel>@<d>` 必须是不存在。

- [x] **Step 1: 写失败测试**
  - `grouped_check_verifies_commit_without_update_baseline`；
  - `grouped_check_flags_lying_file_in_newer_group`：手动改基线树里一个高 revision 组的文件；
  - `committed_deletion_becomes_tombstone_when_just_recorded`；
  - `excluded_file_is_refused_not_tombstoned`：E17；
  - `tombstone_that_reappeared_on_server_is_lying`；
  - `server_audit_ignores_tombstoned_files`；
  - `single_rev_map_uses_exactly_the_legacy_command`：断言只发了一条 diff。
- [x] **Step 2: 跑测试确认失败。**（实际顺序偏离，见日志 Task 4「偏离计划」）
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑测试确认通过**，`svn_health*`、`svn_server_set` 不回归。

### Task 5: 录制泛化（重建、初始化、sync_in 接管、交付拒绝）

**Files:**
- Modify: `src-tauri/src/engine/svn_preflight.rs`（`run_preflight` 增加 `allow_mixed: bool`，混合时不做 P2，改为调用 `read_node_status`）
- Modify: `src-tauri/src/engine/svn_init.rs`（`rebuild_baseline`、`init_phases`、`check_current_baseline`）
- Modify: `src-tauri/src/engine/svn_sync_check.rs`、`svn_sync_in.rs`（`catch_up`）、`svn_sync_out_check.rs`
- Test: `src-tauri/tests/svn_init.rs`、`svn_sync_in.rs`、`svn_sync_out_check.rs`

**Interfaces:**
- Consumes: Task 1、2、4。
- Produces:
  - `pub struct Recorded { pub tree: String, pub map: BaseMap, pub nodes: Vec<NodeStatus>, pub health: Health }`
  - `pub fn record_mixed(repo: &str, cfg: &SvnConfig, idx: &TempIndex) -> Result<Recorded>`：依次执行 R1–R3c：
    - R1：拒绝 `~`、冲突与树冲突、`WcLocked`、update 中途。
    - R2a–R2c：沿用 `record_server_tree_into`。
    - 再算 `compute_map`。
    - R3b：`check_baseline_map(JustRecorded)`，把产生的 `new_gone` 合进 map。
    - R3c：重读 node status，`compute_map` 的结果必须相等，否则报 `svn_rebuild_revision_moved`。
  - `rebuild_baseline` 改用 `record_mixed`。`RebuildReport.node_rev` 改为 floor，并新增 `base: BaseMapSummary`。
  - 初始化允许混合版本（Q4）。
  - `check_current_baseline` 改用 `read_base_map` + `check_baseline_map(Unknown)`。
  - `check_sync_in`：`ExternallyAdvanced` 时 `catch_up_rev = Some(新 floor)`。`catch_up` 调用 `rebuild_baseline`，后者此时已接受混合版本。
  - `check_sync_out`：基线不是单值时拒绝，错误码 `sync_out_mixed_baseline`，文案「请先更新，或继续用 TortoiseSVN 提交后点跟进」。
  - 6 处各自读 trailer 的地方，统一换成 `read_base_map` 或 `svn_sync::read_baseline_rev`（返回 floor）。只有 `graph.rs` 和 `svn_log.rs` 保留 `parse_trailer`：混合基线不映射到 svn 节点（分析 §6.9）。

- [x] **Step 1: 写失败测试**
  - `rebuild_accepts_commit_without_update_and_writes_mixed_trailer`；
  - `rebuild_refuses_conflicted_working_copy`；
  - `init_accepts_mixed_revision_working_copy`；
  - `init_records_committed_deletion_as_tombstone`；
  - `sync_in_from_mixed_baseline_checks_out_zero_w_modifications`：分析 §11 第一条，端到端；
  - `sync_in_catches_up_externally_advanced_then_updates`；
  - `sync_out_refuses_mixed_baseline_with_own_error`；
  - `check_current_baseline_verifies_mixed_baseline`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**，并同步更新前端 `svnInit.ts` 里 `svn_preflight_mixed_revision` 的文案：初始化与重建不再触发这个错误码，但保留它，兼容旧 journal 或旧报错。
- [x] **Step 4: 两侧验证。**

### Task 6: 对象层重放引擎

**Files:**
- Create: `src-tauri/src/engine/svn_replay.rs`（从 `svn_deliver.rs:118-280` 抽出 `commits_between`、`replay`、`commit_like`、冲突解析）
- Modify: `src-tauri/src/engine/svn_deliver.rs`（改为调用 `svn_replay`，错误码与文案不变）
- Test: `src-tauri/tests/svn_replay.rs`；`svn_deliver.rs` 不回归

**Interfaces:**
- Produces:
  - `pub struct ReplayOpts<'a> { pub drop_empty: bool, pub superseded: &'a dyn Fn(&ReplayCommit) -> BTreeSet<String>, pub codes: ReplayCodes }`
    - `ReplayCodes` 给出冲突 / git 太旧 / 失败三类错误码，供交付与跟进各自使用。
  - `pub struct ReplayCommit { pub oid, pub parents: Vec<String>, pub subject, pub body, pub local_only: bool }`
  - `pub fn commits_between(repo, base, head) -> Result<Vec<ReplayCommit>>`
  - `pub enum ReplayOutcome { Done { tip: String, map: BTreeMap<String,String> /*原→新，丢弃的映射到其父的新 oid*/, dropped: Vec<String>, rewritten: Vec<(String /*oid*/, Vec<String> /*换成 SVN 版本的路径*/)> }, Conflict { at: String, files: Vec<String> } }`
  - `pub fn replay_onto(repo, onto, commits: &[ReplayCommit], opts: &ReplayOpts, memo: &mut BTreeMap<String,String>) -> Result<ReplayOutcome>`
    - `memo` 用于共享 commit 只重放一次（R20）。
    - 判空用「结果树 == 当前父的树」（W18）。
    - superseded 路径按 W19 构造 C'；P 里只能放文件路径。
  - `pub fn all_committed(repo, old_base, head, new) -> Result<bool>`：按 W20 判定，外加区间里没有 Local-Only。

- [x] **Step 1: 写失败测试**
  - `replay_drops_commit_whose_content_reached_svn`：W4；
  - `drop_empty_compares_with_current_parent_not_onto`：W18 反例；
  - `superseded_path_resolves_w10_r2_conflict`；
  - `superseded_path_fixes_w14_duplicate_foo`；
  - `superseded_keeps_other_paths_of_same_commit`：a、e；
  - `shared_commit_replayed_once_across_branches`；
  - `all_committed_true_through_no_ff_merge_with_delete_add_mode_rename`；
  - `all_committed_false_when_mode_differs`；
  - `all_committed_false_when_local_only_in_range`；
  - `replay_cjk_path_conflict_is_reported_verbatim`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑测试确认通过**，`cargo test --test svn_deliver --test svn_sync_out` 不回归。

### Task 7: Local-Only 泄漏按 hunk 判定

**Files:**
- Create: `src-tauri/src/engine/svn_leak.rs`
- Test: `src-tauri/tests/svn_leak.rs`

**Interfaces:**
- Produces:
  - `pub struct Leak { pub branch: String, pub commit: String, pub subject: String, pub path: String, pub hunks: Vec<HunkLeak> }`
  - `pub struct HunkLeak { pub old_start: u32, pub lines: u32, pub verdict: LeakVerdict }`
  - `pub enum LeakVerdict { Present, Unknown }`：Absent 不输出。
  - `pub fn local_only_leaks(repo, branch: &str, commits: &[ReplayCommit], changed_paths: &BTreeSet<String>, new_base: &str) -> Result<Vec<Leak>>`
    - 只看 `local_only` 的 commit，且只看它改过、又在 `changed_paths` 里的路径。
    - 算法用 W21 的 M4：用 `git2::Patch::from_blobs` 以 context_lines=0 求 `B→NEW` 的 hunk，做行号映射；在映射位置比较 L 的 + 行。

- [x] **Step 1: 写失败测试**：W21 的六个场景，作为参数化测试 `leak_by_hunk_matches_w21_scenarios`：只提交了功能改动、泄漏 line5、全部泄漏、只泄漏 line30、前面插入 3 行后泄漏、前面插入 3 行后不泄漏。外加 `non_local_only_commits_are_ignored`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑测试确认通过。**

### Task 8: 跟进时拼索引

**Files:**
- Create: `src-tauri/src/engine/svn_follow_index.rs`
- Modify: `src-tauri/src/engine/svn_baseline.rs`（`IndexLock` 提供 `path()`，能把 index.lock 当临时索引写）
- Test: `src-tauri/tests/svn_follow_index.rs`

**Interfaces:**
- Consumes: Task 9 的 `FollowMark`（作者与快照的判定输入）。
- Produces:
  - `pub struct PathFacts { pub author_is_me: Option<bool>, pub staged_before_mark: Option<bool> }`
    - `None` 表示不知道，一律按「拿不准」处理。
  - `pub enum StageDecision { TakeNew, KeepStaged, Merged(String /*blob*/), LostToNew }`
  - `pub struct ComposeReport { pub decisions: BTreeMap<String, StageDecision>, pub uncertain: Vec<String>, pub lost: Vec<String> }`
  - `pub struct PreparedIndex<'r> { lock: IndexLock, pinned_ref: String, pub report: ComposeReport }`
    - `fn commit(self) -> Result<()>`：rename，os error 5 / 32 退避重试 1.5 秒（W24）。
    - `fn abandon(self)`：删锁。
  - `pub fn prepare_index(repo, opid: &str, old: &str, new: &str, facts: &dyn Fn(&str) -> PathFacts) -> Result<PreparedIndex>`
    - 按日志「计划的错误」里的修正顺序：
      1. 建锁（索引有 stage 1–3 时报 `svn_follow_index_conflicted`）。
      2. 钉原始 index blob 到 `refs/mygitclient/follow/<opid>`。
      3. 复制到 index.lock。
      4. 「已暂存 ∩ OLD→NEW 改过」的路径先写成 NEW。
      5. `read-tree -i -m OLD NEW`。
      6. 写最终条目。
      7. 补回 assume-unchanged。
      8. refresh。
    - 决策规则（分析 §6.11.1 的表，T0 换成快照）：
      - 作者是我且 `staged_before_mark=true` → `TakeNew`；
      - 作者是我但其余情况 → `KeepStaged`，记入 `uncertain`；
      - 作者不是我 → `git merge-file --object-id`：rc=0 → `Merged`，否则 `LostToNew`；
      - 二进制或非内容条目（删除 / mode / 新增 / i-t-a）且作者不是我 → `LostToNew`。

- [x] **Step 1: 写失败测试**
  - `unstaged_committed_file_takes_new_and_disk_untouched`：W8 / W12；
  - `intermediate_stage_before_mark_is_discarded_for_own_commit`：U2；
  - `stage_after_mark_is_kept_and_reported_uncertain`；
  - `colleague_change_merges_with_staged_content`：W13；
  - `colleague_conflict_takes_new_and_reports_lost`；
  - `staged_delete_and_mode_change_survive_when_new_untouched`：W25；
  - `assume_unchanged_flag_restored_after_new_changes_path`；
  - `conflicted_index_is_refused`；
  - `pinned_raw_index_restores_byte_for_byte`：W28；
  - `stat_kept_for_untouched_entries`：断言未涉及条目的 mtime 非 0。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑测试确认通过。**

### Task 9: 失焦快照与「我是谁」

**Files:**
- Create: `src-tauri/src/engine/svn_follow_mark.rs`
- Modify: `src-tauri/src/engine/svn_config.rs`（`SvnConfig` 加 `#[serde(default, skip_serializing_if = "Option::is_none")] pub svn_user: Option<String>`）
- Modify: `src-tauri/src/commands/svn.rs`（命令 `svn_follow_mark(path)`、`svn_wc_db_stamps(path) -> Vec<WcDbStamp>`）
- Test: `src-tauri/tests/svn_follow_mark.rs`

**Interfaces:**
- Produces:
  - `pub struct FollowMark { pub at: i64, pub head_oid: String, pub branch: Option<String>, pub staged: BTreeMap<String, String> }`，存在 `.git/mygitclient/follow-mark.json`。
  - `pub fn write_mark(repo) -> Result<()>`：读 `diff-index --cached -z HEAD`。
  - `pub fn read_mark(repo) -> Result<Option<FollowMark>>`、`pub fn clear_mark(repo)`。
  - `pub fn my_svn_user(repo, repos_root) -> Option<String>`：先取 `cfg.svn_user`；没有就解析 `svn auth` 的输出，只读 `Authentication realm:` 与 `Username:` 两类行，realm 必须包含 repos_root 的 scheme+host；取不到返回 None。
  - `pub struct WcDbStamp { pub wc_root: String, pub mtime_ns: u128, pub size: u64 }`（camelCase）：对每个 wc(P) 的 `.svn/wc.db` 取一次 stat（E19）。

- [x] **Step 1: 写失败测试**
  - `mark_records_staged_blobs_and_head`；
  - `mark_ignores_unstaged_changes`；
  - `svn_user_prefers_config`；
  - `svn_auth_output_parsing_picks_matching_realm`：纯函数测试，喂样例文本；
  - `wc_db_stamp_changes_after_external_commit_not_after_status`：E19。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑测试确认通过。**

### Task 10: 跟进编排（F1–F6）与崩溃恢复

**Files:**
- Create: `src-tauri/src/engine/svn_follow.rs`
- Modify: `src-tauri/src/engine/svn_journal.rs`（`Flow::Follow`；`Phase::FollowPublishing`；字段 `follow_steps: Option<FollowSteps>`，用 serde default，旧 journal 仍可读）
- Modify: `src-tauri/src/commands/svn_sync.rs`（命令 `svn_follow(handle, path) -> FollowOutcome`、`svn_follow_continue`）、`src/engine/svnJournal.ts`
- Test: `src-tauri/tests/svn_follow.rs`

**Interfaces:**
- Consumes: Task 5 的 `record_mixed`；Task 6 的 `replay_onto` / `all_committed`；Task 7；Task 8 的 `prepare_index`；Task 9 的 mark 与 user。
- Produces:
  - `pub enum FollowOutcome`，变体：
    - `Done { report: FollowReport }`
    - `NeedsManual { at: String, files: Vec<String>, baseline_published: bool }`
    - `Refused { code: String, message: String }`
  - `pub struct FollowReport`，字段：
    - `new_floor`
    - `changes: Vec<ExternalChange>`
    - `dropped: Vec<String>`
    - `rewritten: Vec<(String, Vec<String>)>`
    - `stage: ComposeReport`
    - `moved_branches: Vec<String>`
    - `behind_branches: Vec<(String, String /*原因*/)>`
    - `leaks: Vec<Leak>`
  - `pub fn follow(repo: &str, opid: &str) -> Result<FollowOutcome>`
  - `pub fn resume_follow(repo) -> Result<FollowOutcome>`
- 编排要点（以分析 §6.5 为准，并带上日志里的修正）：
  - **准备阶段（不拿锁）：**
    - 预检：git 版本、HEAD 附着、git 空闲、没有 journal。
    - F1：`record_mixed` + 父提交为旧基线 + `summary_extra`（来源 revision 与作者）。
    - F2：先试 `all_committed`；否则 `assert_linear`，再 `replay_onto`。superseded 的判定 = 外部 changed 里作者是我、且该 commit 是 mark.head_oid 的祖先的路径。
    - 其他分支：包含旧基线、不在任何 worktree 里是 HEAD、没有 upstream 的分支，按同样方式处理；冲突、不线性、F3 命中的分支进入 `behind_branches`。
    - F3：`local_only_leaks`。
  - 当前分支冲突时：只发布基线（F1 + CAS），返回 `NeedsManual { baseline_published: true }`（手动跟进见 Task 13）。
  - **发布阶段：**
    - 拿 `SyncLock`，挂起 watcher，写 journal（记下全部旧 / 新 OID）。
    - 锁内复核：映射、HEAD 符号引用与 OID、svn/base、各分支 OID 都没变；有变化就释放锁，回到 F1 最多一次。
    - 按日志「偏离计划」里的顺序执行：CAS svn/base → `prepare_index` → 核对 HEAD → CAS 当前分支 → 再核对 HEAD → `commit()` → CAS 其他分支。
    - 每一步之前先把 `follow_steps` 写进 journal。
  - **收尾：** 删 journal，`clear_mark`，follow 钉住的 ref 只保留最近 20 个【待决】。
  - **「分支跟进」入口：** InSync，但 HEAD 不包含 svn/base，且 `merge-base HEAD svn/base` 是一条基线 commit。此时跳过 F1，以 svn/base 为 NEW，以 merge-base 为旧基线。

- [x] **Step 1: 写失败测试（fixture：`hybrid_with_colleague` + 外部 svn 操作）**
  - `follow_after_tortoise_commit_leaves_only_uncommitted_changes`：U2 / G1；
  - `follow_after_partial_update_puts_colleague_changes_in_baseline`：U7；
  - `follow_drops_intermediate_commits_of_own_svn_commit`：U3；
  - `follow_keeps_stash_applicable`：U4；
  - `follow_moves_feature_branch_by_object_replay`：U5；
  - `follow_marks_conflicting_branch_behind_without_blocking`；
  - `follow_after_external_no_ff_merge_uses_all_committed_shortcut`；
  - `follow_never_writes_working_tree`：前后对整个 wc 的文件做 hash 比较；
  - `follow_refuses_when_svn_conflicted`；
  - `follow_refuses_when_wc_locked`；
  - `follow_conflict_publishes_only_baseline`；
  - `follow_detects_checkout_b_during_publish_and_rolls_back`：用测试钩子在第一次核对之后执行 `git checkout -b`，Review Focus 4；
  - `resume_after_crash_between_each_step`：参数化，测试钩子在每一步之后 panic，Review Focus 5；
  - `branch_follow_entry_moves_lagging_branch`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现**（测试钩子用 `#[cfg(test)]` 或 feature 下的回调，参照仓库已有的做法）。
- [x] **Step 4: 两侧验证。**

### Task 11: 前端——检测、跟进入口与结果展示

**Files:**
- Modify: `src/store/repoStore.ts`
  - 焦点时取 `svn_wc_db_stamps`，变了才调 `refreshSvnSync`；
  - 失焦时，如果最近一次判定是 `inSync`，调 `svn_follow_mark`；
  - 新增 `svnFollow`。
- Modify: `src/components/BaselineBanner.tsx`、`src/components/RepoTab.tsx`（接 `follow` 动作）
- Create: `src/components/FollowReportDialog.tsx`、`src/engine/svnFollow.ts`（纯函数：把 `FollowReport` 转成展示模型）
- Modify: `src/components/BranchView.tsx`（显示「落后基线」标记，数据来自跟进报告和新命令 `svn_branches_behind(path) -> Vec<String>`）
- Test: `tests/vitest/svnFollow.test.ts`、`tests/vitest/FollowReportDialog.test.tsx`、`tests/vitest/BaselineBanner.test.tsx`

**Interfaces:**
- Consumes: Task 3、9、10 的命令与类型（camelCase）。
- Produces: `describeFollowReport(r: FollowReport): FollowReportModel`，分节为：跟进的文件（按作者分组）、丢弃的 commit、换成 SVN 版本的路径、丢弃的暂存、合并的暂存、拿不准的暂存、移动的分支、落后的分支、泄漏告警。

- [x] **Step 1: 写失败测试**
  - `follow report groups files by author`；
  - `uncertain stages are listed with a keep note`；
  - `leaks are shown as a warning section`；
  - `banner follow button invokes svnFollow`；
  - `focus refreshes sync only when wc.db stamp changed`：对 store 逻辑抽出的纯函数做测试。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 跑 `pnpm build` 与 `pnpm test`**；在 `pnpm tauri dev` 里实测焦点 / 失焦事件（分析 §11）。真实窗口要人来操作，做不到就在日志里记「未验证」（已记，手测清单见日志 Task 11 小节）。

### Task 12: 提交图

**Files:**
- Modify: `src-tauri/src/engine/graph.rs`（`CommitNode` 加 `baseline_floor: Option<u64>`、`baseline_newer: Option<u32>`，取自 `parse_message`；`baseline_rev` 只在单值时有值）
- Modify: `src/engine/types.ts`、`src/engine/svnGraph.ts`、`src/components/CommitRow.tsx`
- Test: `src-tauri/tests/svn_graph_contract.rs`、`tests/vitest/svnGraph.test.ts`

**Interfaces:**
- 混合基线不进 `baselineMap`，作为普通 git 节点保留，标注「svn/base：r<floor> + N 个文件在更新的 revision」。
- `spliceHistory` 删除单值基线时，混合基线的父边重写到对应的 svn 节点，不能悬空（分析 §6.9）。
- `diffBase` / `deliveredFrom` 不能把混合基线之后的提交算进 r<floor>。
- graph 读取不能显示 `refs/mygitclient/*`（W28）。

- [x] **Step 1: 写失败测试**
  - `mixed baseline stays as git node attached to its floor svn node`；
  - `svn node diff scope excludes commits after a mixed baseline`；
  - `follow pin refs are not drawn`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 两侧验证。**

### Task 13: 手动跟进（冲突兜底）

**Files:**
- Modify: `src-tauri/src/engine/svn_follow.rs`、`svn_sync_in.rs`（复用 S1 stash → `rebase --empty=drop <NEW>` → S10 apply，含 A17）
- Modify: `src/components/svnSyncFlow.ts`（冲突出口、继续 / 放弃沿用现有 UI）
- Test: `src-tauri/tests/svn_follow.rs`

**Interfaces:**
- `pub fn manual_follow(repo, opid) -> Result<SyncInOutcome>`：前提是基线已发布（`NeedsManual.baseline_published`）。journal 里 `flow = follow`，phase 沿用 sync_in 的 Stashing…ApplyingStash；apply 先带 `--index`，失败再退回不带，并在 Outcome 里标出「暂存状态没保住」。

- [x] **Step 1: 写失败测试**
  - `manual_follow_resolves_w10_r2_via_rebase_conflict_exit`；
  - `manual_follow_restores_staging_with_index_when_possible`；
  - `manual_follow_abort_returns_to_original_branch`。
- [x] **Step 2: 跑测试确认失败。**
- [x] **Step 3: 实现。**
- [x] **Step 4: 两侧验证。**

### Task 14: 收尾

- [ ] 更新分析文档：把日志里的「计划的错误 / 偏离」回写到相应章节（T0 改成快照、拼索引的步骤顺序、发布顺序、钉原始 index）。
- [ ] 全量验证：`pnpm build`、`pnpm test`、`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。
- [ ] 独立 review（opus），按 AGENTS.md §13 的 rubric，先读日志里的偏离条目；高危问题修完再停。
- [ ] 汇总【待决】给用户；按用户的决定回修。
