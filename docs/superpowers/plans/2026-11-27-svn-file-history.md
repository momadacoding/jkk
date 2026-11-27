# 混合仓库文件历史与 Blame（git 打底、SVN 补全）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 混合仓库里，History 把基线提交展开成真实 SVN 提交，Blame 把归到基线的行映射到 `svn blame` 的作者和版本；git 那一层保持不变。

**Architecture:** 现有 `file_history` / `blame_file` 命令照旧先返回 git 结果，只多带基线标注。两个新命令 `svn_file_history`、`svn_blame_overlay` 异步补全 SVN 信息，第三个 `svn_rev_file_diff` 给 SVN 条目出 diff。分组、去重、换 peg 的规则是纯函数，放在 `svn_file_history.rs`，单测覆盖；IO 编排单独测 fixture。

**Tech Stack:** Rust（git2 0.20、svn CLI 经 `svn_cli`）、Tauri 2 命令、React 18 + TS、Vitest、RTL。

**Spec:** `docs/superpowers/specs/2026-11-27-svn-file-history-design.md`（下称 spec）

## Global Constraints

- 全程只读：不往用户 `.git` 写对象、ref、配置（spec §2 反目标）。
- 不改现有命令名与已有字段；只新增可选字段与新命令（AGENTS.md §1.2）。
- 所有 svn 调用经 `svn_cli`（`--non-interactive` 由它注入）；新命令的 svn 调用都带超时，默认 15 秒。
- 每页 50 条 SVN 条目；Blame 最多映射 8 个 (基线, 路径)；进程内缓存上限 200 项 LRU。
- 区间用 `BaseMap::rev_of(path)`，不用 floor。
- 纯 git 仓库行为不变：不调新命令，现有测试全过。
- 不新增依赖（npm / crate），也不给已有 crate 加 feature。
- 测试只用临时 fixture 仓库（`tests/common/svn_fixtures.rs`），禁止碰真实仓库。
- 用户可见文案用 spec 的原句：「未交付」「已交付 rN」「正在读取 SVN 日志…」「SVN 明细没取到：<原因>」「连不上 SVN，基线条目显示的是快照」「有 N 行没对上 SVN 版本，保持基线显示」「（说明未加载）」。

## Review Focus

1. **文件在 git 里有提交、但这个文件从来没进过任何基线**（git 里新建、还没交付）：不该调 SVN，也不该出现「连不上 SVN」提示。→ Task 5 的 `a_file_never_in_a_baseline_makes_no_svn_call`。
2. **从别的分支的提交打开 History**（该分支停在较旧的基线上）：不能出现那条基线之后的 SVN 版本。→ Task 5 的 `opening_an_old_commit_hides_later_revisions`。
3. **快速切换文件 / 关掉面板时，上一个文件的 SVN 结果晚到**：不能填进新文件的列表。→ Task 8 的 `stale svn response is ignored`。
4. **服务器上该路径已被删除**（墓碑，或同事删了还没更新）：peg 在旧版本上仍可查，不该报「路径不存在」。→ Task 5 的 `a_file_deleted_on_the_server_later_still_expands`。
5. **Blame 工作区时编辑器还在改文件**（`workdirVersion` 变了）：重跑 git blame 后，overlay 必须按新结果重算，不能套用旧行号。→ Task 9 的 `overlay is recomputed after workdir change`。

---

## 文件结构

| 文件 | 职责 |
|---|---|
| `src-tauri/src/engine/svn_cli.rs`（改） | 新增带超时的 `run_svn_deadline` 与错误分类 |
| `src-tauri/src/engine/svn_wc.rs`（改） | 收进 `unit_of` / `server_url_at`（从 `svn_health.rs` 挪过来），加 `units_cached` |
| `src-tauri/src/engine/svn_file_history.rs`（新） | 纯函数：分组、去重、下一页 / 换 peg；IO：`svn_file_history`、`svn_rev_file_diff`、`svn_blame_overlay`、进程内缓存 |
| `src-tauri/src/engine/file_history.rs`（改） | `FileHistoryEntry.svn`、`BlameCommit.baseline` 标注 |
| `src-tauri/src/commands/history.rs`（改）、`src-tauri/src/lib.rs`（改） | 三个新命令 |
| `src-tauri/tests/file_history_svn.rs`（新） | fixture 集成测试 |
| `src/engine/svnHistory.ts`（新） | 前端纯函数：合并分组、合并 overlay |
| `src/store/repoStore.ts`（改） | 三个 invoke |
| `src/components/FileHistoryView.tsx`、`src/components/fileHistoryItems.ts`（改） | 显示 |
| `tests/vitest/svnHistory.test.ts`（新）、`tests/vitest/FileHistoryView.test.tsx`（改） | 前端测试 |

---

### Task 0: 实现日志

- [ ] **Step 1:** 在 worktree 里建 `docs/superpowers/plans/2026-11-27-svn-file-history-decisions.md`，含「进度总账表」（Task 1–10 各一行）和按 Task 分节的四类决策条目（AGENTS.md §2.1）。先写进一条已知偏离：**spec §7 要求超时杀「含子进程」，本计划只杀 svn 主进程**——杀进程树要给 `windows` crate 加 `Win32_System_JobObjects` feature，违反「不加 feature」；svn+ssh 的 ssh 子进程在 stdin/stdout 关闭后自行退出（Task 1 验证 `svn://` 场景，svn+ssh 未测，记为风险）。
- [ ] **Step 2:** 提交 `docs(history): 实现日志`。

---

### Task 1: 带超时的 svn 调用与错误分类

**Files:**
- Modify: `src-tauri/src/engine/svn_cli.rs`
- Test: 同文件 `#[cfg(test)]`

**Interfaces:**
- Produces:
  - `pub const SVN_QUERY_TIMEOUT: Duration = Duration::from_secs(15);`
  - `pub fn run_svn_deadline(path: &str, args: &[&str], timeout: Duration) -> Result<String>`：用 `svn_command(path).args(with_global(args))` 起进程，stdout / stderr 各一个读线程，`try_wait` 每 50 ms 轮询；到点 `kill()` 后返回 `GitError::new("svn_unreachable", "等了 N 秒 SVN 没有响应")`。不读写扫描缓存、不取预跑结果。
  - `pub fn classify_svn_error(stderr: &str) -> &'static str`：返回 `"svn_unreachable" | "svn_auth" | "svn_path_missing" | "svn_cli"`。非 0 退出时 `run_svn_deadline` 用它定错误码，message 只留 stderr 第一行 `svnXXXXX: E…` 那句。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn a_server_that_never_answers_times_out() {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let _keep = std::thread::spawn(move || { let _c = l.accept(); std::thread::sleep(Duration::from_secs(30)); });
    let dir = tempfile::tempdir().unwrap();
    let t = std::time::Instant::now();
    let url = format!("svn://127.0.0.1:{port}/x");
    let e = run_svn_deadline(dir.path().to_str().unwrap(), &["log", "--xml", "-l", "1", &url], Duration::from_secs(1)).unwrap_err();
    assert_eq!(e.code, "svn_unreachable");
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_refused_port_is_unreachable_not_generic() { /* bind 后立刻 drop，拿到一个没人听的端口 */ }

#[test]
fn a_missing_path_is_classified() { /* svn_fixtures::svn_repo() 的 file:// URL 加 /nope */ assert_eq!(code, "svn_path_missing"); }
```

- [ ] **Step 2:** `cargo test --lib svn_cli` → 编译失败（函数不存在）。
- [ ] **Step 3:** 实现。错误码映射**以这三个测试实测的 stderr 为准**写进 `classify_svn_error`，把实测的 `E` 码写进决策日志「计划没写的坑」。认证失败（`svn_auth`）fixture 造不出来，按 svn 文档的 `E170001` / `E215004` 写，并在注释里标「未实测」。
- [ ] **Step 4:** `cargo test --lib svn_cli` → PASS；`cargo fmt --check`、`cargo clippy --all-targets` 无告警。
- [ ] **Step 5:** 提交 `feat(svn): 带超时的 svn 查询入口`。

---

### Task 2: 路径 → 服务器 URL 共用

**Files:**
- Modify: `src-tauri/src/engine/svn_wc.rs`、`src-tauri/src/engine/svn_health.rs`（删掉本地两个函数，改调 `svn_wc`）

**Interfaces:**
- Produces:
  - `pub fn unit_of(units: &[WcIdentity], path: &str) -> Option<(usize, String)>`（原样搬过来）
  - `pub fn server_url(units: &[WcIdentity], path: &str) -> Option<String>`：不带 `@rev` 的 URL（`url` + `/` + `encode_url_path(rel)`）；`server_url_at` 改成 `format!("{}@{rev}", server_url(..)?)`，签名不变，也搬到 `svn_wc`
  - `pub fn units_cached(repo_root: &str) -> Result<Vec<WcIdentity>>`：按 (repo_root, `read_config` 的 whitelist) 进程内缓存 `enumerate_wcs` 的结果；whitelist 变了就重算

- [ ] **Step 1:** 在 `svn_wc.rs` 测试里加 `server_url_encodes_space_chinese_and_at`（`Code/sp ace/中文@x.cs` → 不含空格和 `@`）与 `nested_external_wins_over_outer_unit`（两个单元 `Code` 与 `Code/Ext`，`Code/Ext/a.cs` 归后者）。
- [ ] **Step 2:** 跑 → 失败。
- [ ] **Step 3:** 搬代码。纯移动 + 一个新函数，`svn_health` 的行为不变。
- [ ] **Step 4:** `cargo test --lib svn_wc`、`cargo test --test svn_health*`（按文件分批，见记忆 cargo-test-full-suite-oom）→ PASS。
- [ ] **Step 5:** 提交 `refactor(svn): 路径到服务器 URL 的换算收进 svn_wc`。

---

### Task 3: History / Blame 的基线标注

**Files:**
- Modify: `src-tauri/src/engine/file_history.rs`
- Create: `src-tauri/tests/file_history_svn.rs`

**Interfaces:**
- Produces（全部 `#[serde(rename_all = "camelCase")]`）：
  - `FileHistoryEntry` 新字段 `pub svn: Option<EntrySvn>`
  - `pub struct EntrySvn { pub baseline: Option<BaselineInfo>, pub delivered: Option<DeliveredState> }`
  - `pub struct BaselineInfo { pub file_rev: u64, pub floor: u64, pub delivered_rev: Option<u64> }`
  - `#[serde(tag = "kind", rename_all = "camelCase")] pub enum DeliveredState { Undelivered, Delivered { rev: u64 } }`
  - `BlameCommit` 新字段 `pub baseline: Option<BaselineInfo>`
  - `fn annotate_svn(repo: &str, entries: &mut [FileHistoryEntry]) -> Result<()>`：`read_config(repo)` 失败（不是混合仓库）直接返回、全部保持 `None`。基线用 `git_read::commit_message` + `svn_basemap::parse_message`，`file_rev = map.rev_of(&entry.path)`，`delivered_rev` 用 `svn_baseline` 里读 `Delivered-Rev` 的函数。非基线提交：不是 `svn/base` tip 的祖先 → `Undelivered`；是某个带 `Delivered-Rev: N` 的基线 B 的祖先、且不是 B 在 `svn/base` first-parent 链上前一条基线的祖先 → `Delivered { rev: N }`；其余（更早的、找不到对应交付的）→ `None`。
  - `blame_file_limited` 结束前对 `commits` 里每个键做同样的基线判定，填 `baseline`。

- [ ] **Step 1: 写失败测试**（`tests/file_history_svn.rs`，fixture 用 `svn_fixtures::hybrid_with_colleague()`，更新 / 交付参照 `tests/svn_sync_in.rs` 的 `sync` 与 `tests/svn_sync_out.rs` 的 `deliver` 写本文件自己的 helper）

```rust
#[test] fn the_init_commit_is_marked_as_a_baseline_with_the_file_rev()
// file_history(Code/A.cs) 最老一条：svn.baseline.file_rev == 1, delivered == None
#[test] fn a_local_commit_is_undelivered()
// work 上改 A.cs 提交：最新一条 delivered == Undelivered, baseline == None
#[test] fn a_delivered_commit_carries_its_svn_rev()
// 交付后：那条本地提交 delivered == Delivered{rev: 交付的 rN}；新基线 baseline.delivered_rev == Some(rN)
#[test] fn mixed_revision_baseline_uses_rev_of_not_floor()
// 用 create_baseline_commit_with + BaseMap{floor: 3, revs: {"Code/A.cs": 5}} 建一条基线：file_rev == 5, floor == 3
#[test] fn plain_git_repo_has_no_svn_annotation()
// 普通 git fixture：所有 entry.svn == None，BlameCommit.baseline == None
#[test] fn blame_commits_carry_baseline_info()
```

- [ ] **Step 2:** `cargo test --test file_history_svn` → 编译失败。
- [ ] **Step 3:** 实现 `annotate_svn`，在 `file_history_limited` 返回前调用。
- [ ] **Step 4:** `cargo test --test file_history_svn --test file_history` → PASS（老的 `file_history` 测试不受影响）。
- [ ] **Step 5:** 提交 `feat(history): 混合仓库的历史与 Blame 标注基线和交付状态`。

---

### Task 4: 分组与翻页的纯函数

**Files:**
- Create: `src-tauri/src/engine/svn_file_history.rs`（本 Task 只写纯函数和单测；在 `engine/mod.rs` 注册）

**Interfaces:**
- Produces:
  - `pub struct GroupSpec { pub id: String, pub path: String, pub file_rev: u64 }`（从新到旧传入，`file_rev` 严格递减；不递减时返回 `bad_history_args`）
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub struct Cursor { pub peg_path: String, pub peg_rev: u64, pub next_top: u64 }`
  - `pub struct SvnRevEntry { pub rev: u64, pub author: String, pub date: String, pub message: String }`（`date` 原样 ISO8601，与 `SvnLogEntry` 一致，前端转时间）
  - `pub fn bucket(groups: &[GroupSpec], fetched: &[SvnLogEntry], hidden: &BTreeSet<u64>) -> BTreeMap<String, Vec<SvnRevEntry>>`：spec §5.3 的区间规则；`hidden` 是可达基线的 `delivered_rev` 集合；超过 `groups[0].file_rev` 的丢弃；同号去重。
  - `pub fn next_cursor(groups: &[GroupSpec], cur: &Cursor, fetched: &[SvnLogEntry], page: usize) -> Option<Cursor>`：
    - `fetched.len() == page` → `Some(Cursor { next_top: 最老的 rev − 1, ..cur })`（`next_top == 0` 时按「查完了」处理）
    - 否则这一段查完了：O = 这一段查到的最老 rev（没查到任何条目时 O = `cur.peg_rev + 1`）。找区间上界 `file_rev < O` 的组中最新的一条 → `Some(Cursor { peg_path: 它的 path, peg_rev: 它的 file_rev, next_top: 它的 file_rev })`；没有 → `None`
  - `pub const PAGE: usize = 50;`

- [ ] **Step 1: 写失败测试**（同文件 `mod tests`）

```rust
// groups: B1(r30) B2(r20) B3(r10, 初始化)
#[test] fn revisions_fall_into_half_open_intervals()      // r25→B1, r20→B2, r11→B2, r10→B3, r1→B3
#[test] fn revisions_above_the_newest_baseline_are_dropped() // r31 不出现
#[test] fn delivered_revisions_are_hidden()                // hidden={25}：B1 里没有 r25
#[test] fn duplicate_revisions_are_kept_once()
#[test] fn a_full_page_continues_from_the_oldest_minus_one()
#[test] fn an_exhausted_segment_switches_peg_to_the_newest_unreachable_group()
// 段从 B1 起，只查到 r15 就结束（O=15）：B3(r10) 上界 < 15 → 新 peg = (B3.path, 10)；B2(r20) 上界 ≥ 15 → 视为完整
#[test] fn an_exhausted_segment_with_nothing_left_returns_none()
#[test] fn non_decreasing_file_revs_are_rejected()
```

- [ ] **Step 2:** `cargo test --lib svn_file_history` → 失败。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 同命令 → PASS；fmt / clippy 干净。
- [ ] **Step 5:** 提交 `feat(history): SVN 提交按基线分组的规则`。

---

### Task 5: `svn_file_history` 命令

**Files:**
- Modify: `src-tauri/src/engine/svn_file_history.rs`、`src-tauri/src/commands/history.rs`、`src-tauri/src/lib.rs`
- Test: `src-tauri/tests/file_history_svn.rs`

**Interfaces:**
- Consumes: Task 1 `run_svn_deadline`、`SVN_QUERY_TIMEOUT`；Task 2 `units_cached`、`server_url`；Task 4 全部；`svn_xml::parse_log`
- Produces:
  - `pub struct SvnHistoryPage { pub groups: BTreeMap<String, Vec<SvnRevEntry>>, pub cursor: Option<Cursor>, pub error: Option<SvnFetchError> }`
  - `pub struct SvnFetchError { pub code: String, pub message: String }`（code 取 Task 1 的四种）
  - `pub fn svn_file_history(repo: &str, groups: &[GroupSpec], hidden: &[u64], cursor: Option<Cursor>) -> Result<SvnHistoryPage>`：SVN 出错不返回 `Err`，放进 `error`，已拿到的分组照常返回；只有参数错误返回 `Err`。`groups` 为空 → 空页、不调 svn。一页内若换了 peg，继续查直到攒满 `PAGE` 条或 `cursor` 为 `None`。每次查询：`svn log --xml <server_url(peg_path)>@<peg_rev> -r <next_top>:1 -l <PAGE>`。
  - 进程内缓存：键 `(uuid, url, peg_rev, next_top)` → `Vec<SvnLogEntry>`，200 项 LRU（自己写，`BTreeMap` + 访问序号即可，不加依赖）。
  - 命令：`#[tauri::command(async)] pub fn svn_file_history(path: String, groups: Vec<GroupSpec>, hidden: Vec<u64>, cursor: Option<Cursor>) -> Result<SvnHistoryPage, GitError>`。JS 侧：`invoke('svn_file_history', { path, groups, hidden, cursor })`，`GroupSpec` 的 JS 字段是 `{ id, path, fileRev }`。

- [ ] **Step 1: 写失败测试**（全部在 `hybrid_with_colleague()` 上；同事提交用 `fx.colleague_commits(rel, content)`）

```rust
#[test] fn three_colleague_commits_between_baselines_expand_into_the_later_one()
// 同事改 A.cs 三次 → 更新 → 最新基线组 == 这三个 rev，author/message 与 `svn log --xml` 一致
#[test] fn pre_init_history_pages_by_fifty()
// 初始化前在 wc2 上对 A.cs 提交 60 次（fixture 内循环）→ 第一页 50、cursor 再取得 10 + r1、之后 cursor == None
#[test] fn delivered_rev_is_not_listed_and_colleague_rev_right_after_is()
#[test] fn opening_an_old_commit_hides_later_revisions()
// 第一次更新后建分支 old；第二次更新带进同事的 rX；用 old 上的基线作 groups → 结果里没有 rX
#[test] fn svn_mv_history_is_followed_in_one_segment()
#[test] fn git_rename_delivered_switches_peg()
// work 上 git mv A.cs→A2.cs 交付（SVN 是删 + 增）→ 改名前的基线组也被填上
#[test] fn delete_and_recreate_switches_peg()
#[test] fn a_file_deleted_on_the_server_later_still_expands()
// 同事删了 A.cs 但本地还没更新：peg 在本地基线 rev 上，展开照常
#[test] fn a_file_never_in_a_baseline_makes_no_svn_call()
// groups 为空 → 空页、error == None；用 perf 记录或计数器断言没有 svn 调用
#[test] fn a_path_outside_every_unit_is_skipped_silently()
// groups 的 path 不在任何白名单单元下（unit_of == None）→ 该组为空、error == None、不调 svn
#[test] fn unreachable_server_is_reported_in_error_not_as_err()
// 不能 `svn relocate`（写操作）。svn_file_history 内部拆出
// `pub fn page_with_units(units: &[WcIdentity], groups, hidden, cursor) -> Result<SvnHistoryPage>`，
// 测试传一个 url 为 svn://127.0.0.1:<拒绝端口> 的假单元：返回 Ok，error.code == "svn_unreachable"
```

- [ ] **Step 2:** `cargo test --test file_history_svn` → 失败。
- [ ] **Step 3:** 实现 + 注册命令。
- [ ] **Step 4:** `cargo test --test file_history_svn` → PASS；fmt / clippy 干净。
- [ ] **Step 5:** 提交 `feat(history): 混合仓库按基线展开真实 SVN 提交`。

---

### Task 6: `svn_rev_file_diff` 命令

**Files:**
- Modify: `src-tauri/src/engine/svn_file_history.rs`、`src-tauri/src/commands/history.rs`、`src-tauri/src/lib.rs`
- Test: `src-tauri/tests/file_history_svn.rs`

**Interfaces:**
- Consumes: Task 1、Task 2；`diff::diff_to_data_only`、`DiffViewOptions`
- Produces:
  - `pub fn svn_rev_file_diff(repo: &str, peg_path: &str, peg_rev: u64, rev: u64, view: &DiffViewOptions) -> Result<DiffData>`：两次 `svn cat -r <rev-1 / rev> <url>@<peg_rev>`（`rev-1` 上不存在 → 旧侧为空）；SVN 错误返回 `Err`（这里没有 git 退路，前端照常显示错误）。
  - 内存 diff：在 `Repository::open(repo)?.odb()?` 上 `add_new_mempack_backend` 后写两个 blob、两棵只含这个文件的 tree，`diff_tree_to_tree` + `diff_to_data_only(diff, Some(peg_path))`。**这条路的前提（写入只进 mempack、不落 `.git/objects`）未核实**：Step 1 的测试先验证它；不成立就改用一个临时裸仓库（`Repository::init_bare` 到 `std::env::temp_dir()` 下的唯一目录，用完即删，失败路径也删），并写进决策日志。
  - 命令：`svn_rev_file_diff(path: String, pegPath, pegRev, rev, options: Option<DiffViewOptions>)`，JS 侧 `{ path, pegPath, pegRev, rev, options }`。

- [ ] **Step 1: 写失败测试**

```rust
#[test] fn a_colleague_revision_diff_matches_svn_diff()       // 改动行与 `svn diff -c N` 的 +/- 行一致
#[test] fn the_add_revision_shows_the_whole_file_as_added()
#[test] fn ignore_whitespace_option_applies()
#[test] fn no_object_is_written_to_the_users_git()            // 前后 `.git/objects` 下文件数与 `git count-objects -v` 不变
```

- [ ] **Step 2:** 跑 → 失败。
- [ ] **Step 3:** 实现 + 注册。
- [ ] **Step 4:** 跑 → PASS；fmt / clippy。
- [ ] **Step 5:** 提交 `feat(history): SVN 条目的单文件 diff`。

---

### Task 7: `svn_blame_overlay` 命令

**Files:**
- Modify: `src-tauri/src/engine/svn_file_history.rs`、`src-tauri/src/engine/svn_xml.rs`（加 blame 解析）、`commands/history.rs`、`lib.rs`
- Test: `src-tauri/tests/file_history_svn.rs`、`svn_xml.rs` 单测

**Interfaces:**
- Consumes: Task 1、Task 2；`git_read` 读 blob
- Produces:
  - `pub fn parse_blame(xml: &str) -> Result<Vec<Option<BlameRevAuthor>>>`（`svn_xml.rs`），`pub struct BlameRevAuthor { pub rev: u64, pub author: String, pub date: String }`；某行没有 `<commit>`（本地修改，peg 是 URL 时不会出现）为 `None`
  - `pub struct OverlayTarget { pub baseline_id: String, pub orig_path: String, pub file_rev: u64 }`
  - `pub enum OverlayResult { Mapped { lines: Vec<Option<BlameRevAuthor>> }, Mismatch { svn_lines: usize, git_lines: usize }, Failed { error: SvnFetchError }, Skipped }`（serde `tag = "kind"`）
  - `pub fn svn_blame_overlay(repo: &str, targets: &[OverlayTarget]) -> Result<Vec<OverlayResult>>`：与 `targets` 同序；前端负责只传行数最多的 8 个，后端超过 8 个的部分返回 `Skipped`（双保险）。行数核对：`svn blame --xml <url>@<file_rev>` 的行数 vs 基线提交里 `orig_path` 那个 blob 的行数（末尾无换行的最后一行也算一行，与 `git blame` 口径一致）。缓存键 `(uuid, url, file_rev)`，与 Task 5 共用 LRU。
  - 命令 `svn_blame_overlay(path: String, targets: Vec<OverlayTarget>)`，JS 侧 `{ path, targets }`，`targets` 元素 `{ baselineId, origPath, fileRev }`。

- [ ] **Step 1: 写失败测试**

```rust
#[test] fn baseline_lines_map_to_svn_blame()
// 同事在 wc2 改 A.cs 第 2 行 → 更新；git blame 里第 2 行归新基线，overlay 第 2 行 == 同事的 rev 与作者
#[test] fn line_count_mismatch_is_reported()   // 直接调内部函数，给一个行数不等的 blob
#[test] fn keywords_and_crlf_files_still_map() // svn:keywords=Id + svn:eol-style=CRLF
#[test] fn more_than_eight_targets_are_skipped()
// svn_xml 单测：parse_blame 解析一段真实 `svn blame --xml` 输出（从 fixture 采一份贴进测试）
```

- [ ] **Step 2:** 跑 → 失败。
- [ ] **Step 3:** 实现 + 注册。
- [ ] **Step 4:** 跑 → PASS；fmt / clippy。
- [ ] **Step 5:** 提交 `feat(history): Blame 归到基线的行映射到 svn blame`。

---

### Task 8: 前端 store 与纯函数

**Files:**
- Modify: `src/store/repoStore.ts`、`src/engine/fileHistory.ts`（加类型字段）
- Create: `src/engine/svnHistory.ts`、`tests/vitest/svnHistory.test.ts`

**Interfaces:**
- Consumes: Task 3/5/6/7 的 JSON 形状（camelCase）
- Produces:
  - `fileHistory.ts`：`FileHistoryEntry.svn?: EntrySvn | null`、`BlameCommit.baseline?: BaselineInfo | null` 及对应 interface
  - `repoStore.ts`：`readSvnFileHistory(path, groups: GroupSpec[], hidden: number[], cursor: Cursor | null): Promise<SvnHistoryPage>`、`readSvnRevFileDiff(path, pegPath, pegRev, rev, options?)`、`readSvnBlameOverlay(path, targets: OverlayTarget[])`
  - `svnHistory.ts`：
    - `type SvnGroups = Record<string, SvnRevEntry[]>`（键是基线提交 id）
    - `groupSpecs(entries: FileHistoryEntry[]): GroupSpec[]`（只取 `svn.baseline` 非空的，保持从新到旧）
    - `hiddenRevs(entries): number[]`（各基线的 `deliveredRev`）
    - `mergePage(prev: SvnGroups, page: SvnHistoryPage): SvnGroups`（按组追加、按 rev 去重）
    - `overlayTargets(blame: BlameData): OverlayTarget[]`（按 (commit, origPath) 聚合，基线提交才算，按行数降序取前 8）
    - `applyOverlay(blame: BlameData, targets, results): OverlayView`（每行给出 `{ kind: 'git' } | { kind: 'svn', rev, author, date } | { kind: 'baseline' }`，以及 `unmapped` 行数供提示）
    - 请求序号：`createRequestGate()` 返回 `{ next(): number, isCurrent(n): boolean }`，供组件丢弃过期响应

- [ ] **Step 1: 写失败测试**（`tests/vitest/svnHistory.test.ts`）

```ts
it('groupSpecs keeps only baselines, newest first')
it('mergePage appends and dedupes by rev')
it('overlayTargets picks the 8 largest baseline groups')
it('overlayTargets ignores non-baseline and uncommitted lines')
it('applyOverlay marks mismatch groups as baseline and counts them')
it('stale svn response is ignored') // gate.next() 两次后，第一次的 isCurrent 为 false
```

- [ ] **Step 2:** `pnpm test svnHistory` → 失败。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** `pnpm test`、`pnpm build` → PASS。
- [ ] **Step 5:** 提交 `feat(history): 前端 SVN 历史与 Blame 合并逻辑`。

---

### Task 9: FileHistoryView 显示

**Files:**
- Modify: `src/components/FileHistoryView.tsx`、`src/components/fileHistoryItems.ts`
- Test: `tests/vitest/FileHistoryView.test.tsx`

**Interfaces:**
- Consumes: Task 8 全部

行为（文案用 Global Constraints 里的原句；布局按 spec §5.5 的示意）：
- 拿到 git 历史后，若 `groupSpecs` 非空，调 `readSvnFileHistory`；基线行下面缩进列出 SVN 条目，基线行尾显示「N 次 SVN 提交」，最老一组可「加载更多」（用返回的 `cursor`）。
- 非基线行尾标「未交付」或「已交付 rN」。
- 点 SVN 条目：右侧 diff 用 `readSvnRevFileDiff`（peg 取该条所在组的 path / fileRev）。SVN 条目的右键菜单里「Blame 此版本」等置灰，提示「SVN 条目暂不支持」。
- `page.error` 非空：受影响的基线行显示「SVN 明细没取到：<message>」，列表顶部一行「连不上 SVN，基线条目显示的是快照」（code 为 `svn_unreachable` 时用这句，其它 code 用「SVN 明细没取到：<message>」）。
- Blame：拿到 `BlameData` 后算 `overlayTargets`，非空就调 `readSvnBlameOverlay`，按 `applyOverlay` 显示 `rN · 作者`；`unmapped > 0` 时顶部「有 N 行没对上 SVN 版本，保持基线显示」。
- 每次重读（换文件、`headHash` 变、`workdirVersion` 变）都换新的请求序号，过期响应丢弃。

- [ ] **Step 1: 写失败测试**（RTL，mock `repoStore` 的三个新函数）

```tsx
it('expands a baseline row into svn entries')
it('shows load more for the oldest group and appends the next page')
it('shows the offline banner when the page has svn_unreachable')
it('labels undelivered and delivered commits')
it('overlay is recomputed after workdir change') // 改 workdirVersion → readBlame 与 readSvnBlameOverlay 都再调一次，旧响应不生效
it('plain git repo never calls the svn functions')
```

- [ ] **Step 2:** `pnpm test FileHistoryView` → 失败。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** `pnpm test`、`pnpm build` → PASS。
- [ ] **Step 5:** 提交 `feat(history): 历史与 Blame 显示真实 SVN 提交`。

---

### Task 10: 真实窗口验证与收尾

- [ ] **Step 1:** 在 scratch 目录建一个 fixture 混合仓库（file:// SVN，同事提交几次、更新、交付一次、再本地提交一次），`pnpm tauri dev` 打开它（核对跑的是本 worktree 的二进制，见记忆 tauri-dev-worktree-binary-check）。逐条走 spec §10 的验收 1–5（用 fixture 文件代替 `BuildPlayerConfig.cs`）；断网用把 fixture 仓库目录临时改名来模拟 `file://` 不可达，并另测一次 `svn://` 拒绝端口。结果记进决策日志。
- [ ] **Step 2:** 更新 `docs/superpowers/plans/2026-11-08-file-history-blame.md` 的非目标那一条，注明「混合仓库已由 2026-11-27 设计改为 SVN 补全」；更新 `src-tauri/src/commands/history.rs` 顶部注释里「SVN 混合仓库看的同样是 git 这边的历史」那句。
- [ ] **Step 3:** 全量验证：前端 `pnpm build`、`pnpm test`；Rust `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`（按测试二进制分批）。
- [ ] **Step 4:** 独立 review（`/code-review` 或 review subagent，按 AGENTS.md §13），高危问题修完再复跑 Step 3。
- [ ] **Step 5:** 推分支、开 Draft PR（正文带决策日志链接与「Closes」关联，如有 issue）。
