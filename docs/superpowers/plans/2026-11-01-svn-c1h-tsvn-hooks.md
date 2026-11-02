# SVN C1h：TortoiseSVN 钩子 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 用户在 jkk 里一键把 TortoiseSVN 客户端钩子挂到自己的工作副本上。之后跟进时，「暂存的是不是中间版本」按本机提交事件判定，不再靠作者 + 快照推断。本地补丁在 TortoiseSVN 提交框里就被拦下。钩子未注册或失效时，一切退回 C1a 的行为。

**Architecture:**
- jkk 的 exe 自带钩子子命令 `--tsvn-hook`。`main.rs` 在 Tauri 启动之前分流：普通钩子只把参数写成一条事件放进收件箱，然后立即退出 0；Check-commit 额外按 hunk 检查 Local-Only 泄漏，命中时退出 1。
- 跟进时读收件箱，把事件映射成 C1a 已有的 `PathFacts`。判定表与发布流程都不改。
- 注册直接读写 `HKCU\Software\TortoiseSVN` 的 `hooks` 值，由用户在 jkk 里确认后才写。

**Tech Stack:** Rust（`windows` crate 0.61 的注册表与代码页 API、git2、`run_git`）、Tauri 2 命令、React 18 + TS、Vitest、cargo 集成测试（`file://` SVN fixture）。

**Spec:** `docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md` §6.13（下称「分析」）。必读同目录的 `2026-11-01-svn-c1h-tsvn-hooks-decisions.md`：Task 0 的实测 H1–H5 优先于分析原文。

## Global Constraints

- **依赖：** 不新增 crate 或 npm 包。只给已显式声明的 `windows = "0.61"` 加 feature：`Win32_System_Registry`、`Win32_Globalization`。理由同 Restart Manager：这个 crate 已经在依赖树里，不会新增下载。改 `Cargo.toml` 前要先在日志里记录，并告知用户。
- **注册表形状（H1）：**
  - 位置：`HKCU\Software\TortoiseSVN`，值名 `hooks`，类型 REG_SZ。
  - 每条钩子 6 行，用 `\n` 分隔，每行都以 `\n` 结尾：`<kind>_hook`、工作副本路径（反斜杠、无尾斜杠）、命令行、`true`、`hide`、`ask`。
  - 只追加或删除 jkk 自己的条目，其余条目逐字节保留。
- **命令行：** `"<当前 exe>" --tsvn-hook <kind> --repo "<仓库根>"`，TortoiseSVN 会把自己的参数追加在后面。jkk 的条目靠命令行里含 ` --tsvn-hook ` 识别。
- **注册的钩子：**
  - `pre_commit`、`post_commit`、`post_update` 三个必选。
  - `check_commit` 只在用户勾选「提交前拦截本地补丁」时注册。
  - 不注册 `start_commit` / `start_update` / `pre_update`（偏离分析，见日志 Task 0「取舍」）。
- **钩子参数（H2、H4）：** 位置下标从 0 开始，不含 jkk 自己的前缀参数。
  - `check_commit`：`PATH MESSAGEFILE CWD`
  - `pre_commit`：`PATH DEPTH MESSAGEFILE CWD`
  - `post_commit`：`PATH DEPTH MESSAGEFILE REVISION ERROR CWD`
  - `post_update`：`PATH DEPTH REVISION ERROR CWD RESULTPATH`
  - `PATH` 与 `RESULTPATH` 是临时文件：UTF-8 无 BOM，正斜杠绝对路径，每行以 CRLF 结尾。`ERROR` 也是文件路径，成功时为空文件。
  - `CWD` 是勾选文件的公共父目录，不一定是工作副本根，不要拿它定位仓库。
- **钩子进程：**
  - 不初始化 Tauri，不打开窗口，不连接 jkk 进程。
  - 用 `catch_unwind` 兜住 panic。除了 Check-commit 命中泄漏，其余任何情况都退出 0（fail-open）。
  - stderr 按线程 ANSI 代码页编码（`WideCharToMultiByte(CP_THREAD_ACP)`，H4；原写 `CP_ACP` 是错的，见日志 Task 7）。
- **收件箱：**
  - 位置：`<repo>/.git/mygitclient/tsvn-inbox/`，文件名 `<at_ms>-<pid>-<kind>.json`。
  - 先写 `.tmp` 再 rename，保证原子。
  - 事件只记路径（仓库相对、正斜杠）、revision、时刻与错误文本，不记提交消息。
- **判定：** 钩子事件只是线索，真相仍是 BaseMap。拿不准时一律走 C1a 的保守分支：`KeepStaged`、正常重放。
- **注释、IPC、测试、验证：** 与 C1a 计划的 Global Constraints 相同。
  - 中文注释，解释为什么。
  - IPC 改动必须同步 `tests/svn_backend.rs` 与 TS 类型。
  - fixture 用 `tests/common/svn_fixtures.rs`。
  - 每个 Task 跑完对应侧的验证。
  - 不 commit，除非用户放行。
  - 每完成一个 Task 就更新日志总账。

## Review Focus

1. **工作副本路径或 `%TEMP%` 含空格、中文：** 注册写进去的命令行要能原样被 TortoiseSVN 拆回参数，`--repo` 拿到的仓库路径要逐字节一致。Task 2 用含空格和中文的路径测 `parse_hook_args` 的往返；E2E 在含空格的 fixture 路径上实测（Task 7）。
2. **用户在 TortoiseSVN 设置窗口开着的时候 jkk 去注册：** TortoiseSVN 关窗时会整值写回，覆盖 jkk 的改动。注册之后立即重读核对，没写进去就报错。下次健康检查发现条目丢了，就标「失效」。Task 1 测 CAS 失败分支。
3. **提交失败（过期）只留下 Pre 事件，几天后又有一次同路径的成功提交：** 孤儿 Pre 不能和后来的 Post 配对，否则 t_commit 会早得离谱，把提交之后的新暂存误判成中间版本。Task 3 测「配对要求两者间隔不超过 10 分钟」。
4. **同一个路径既在我的提交事件里，又在之后的更新事件里：** 取 revision 更高的那条；revision 相同时按提交处理。Task 5 测这两种顺序。
5. **钩子注册了，但 exe 换了目录（便携版）：** 健康检查判「失效」，跟进退回 C1a 的判定，并在横幅上提示重新注册。不能把「失效」当成「健康、只是没事件」，那样所有我的提交都会判成拿不准。Task 1 测健康判定，Task 5 测退回。

---

### Task 1: 注册表条目的解析、注册计划与健康判定

**Files:**
- Create: `src-tauri/src/engine/tsvn_hooks.rs`（并在 `engine/mod.rs` 注册）
- Modify: `src-tauri/Cargo.toml`（`windows` 加 `Win32_System_Registry`）
- Test: `src-tauri/tests/tsvn_hooks.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug, PartialEq, Eq, Serialize)] pub struct HookEntry { pub kind: String, pub wc_path: String, pub command: String, pub wait: String, pub show: String, pub enforce: String }`。`kind` 不带 `_hook` 后缀，序列化时补上。
  - `pub fn parse_hooks(value: &str) -> Vec<HookEntry>`。行数不是 6 的倍数时，尾部残条原样忽略。
  - `pub fn serialize_hooks(entries: &[HookEntry]) -> String`。
  - `pub fn is_ours(e: &HookEntry) -> bool`。
  - `pub fn hook_command(exe: &Path, kind: &str, repo: &str) -> String`。
  - `pub struct RegPlan { pub add: Vec<HookEntry>, pub conflicts: Vec<HookEntry> }`
  - `pub fn plan_registration(existing: &[HookEntry], wc_roots: &[String], exe: &Path, repo: &str, block_leaks: bool) -> RegPlan`
  - `pub fn without_ours(existing: &[HookEntry], repo: &str) -> Vec<HookEntry>`
  - `#[serde(tag = "state", rename_all = "camelCase")] pub enum HookHealth { NotRegistered, Healthy { block_leaks: bool }, Stale { reasons: Vec<String> } }`
  - `pub fn judge_health(existing: &[HookEntry], wc_roots: &[String], exe: &Path, repo: &str) -> HookHealth`
  - `pub fn registration_roots(repo: &str) -> Result<Vec<String>>`
  - `pub fn read_hooks_value() -> Result<Option<String>>`
  - `pub fn write_hooks_value(expected: Option<&str>, new: Option<&str>) -> Result<()>`

- [ ] **Step 1：写失败测试**

  `tests/tsvn_hooks.rs`，纯字符串，不碰注册表：
  - `parses_the_value_tortoisesvn_wrote`：输入 H1 里那条 Pre-commit 原文（6 行 + 结尾 `\n`）。解析出 1 条，`kind == "pre_commit"`、`enforce == "ask"`；`serialize_hooks` 逐字节还原。
  - `foreign_entries_survive_a_round_trip`：两条非 jkk 条目夹一条 jkk 条目。`without_ours` 之后序列化，结果等于去掉那 6 行的原文。
  - `plan_adds_four_kinds_per_root_when_blocking_leaks` 与 `plan_skips_check_commit_without_blocking`：条数分别为 4 / 3 乘以根数；`command` 等于 `hook_command(...)`。
  - `plan_reports_foreign_hook_of_the_same_kind_on_the_same_or_ancestor_path_as_conflict`：已有一条非 jkk 的 `post_commit`，路径是根的父目录，必须进 `conflicts`；路径不相干时不算冲突。路径比较不区分大小写，`\` 与 `/` 等价。
  - `plan_is_empty_when_already_registered_for_this_exe_and_repo`
  - `health_*`：
    - 三个必选 kind 齐全且 exe 相同：`Healthy`。有 `check_commit` 时 `block_leaks: true`。
    - exe 不同：`Stale`，reasons 含旧 exe 路径。
    - 缺 `post_update`：`Stale`。
    - 一条 jkk 条目都没有：`NotRegistered`。
    - 另一个仓库（`--repo` 不同）的条目不算本仓库的。
  - `hook_command_quotes_paths_with_spaces_and_cjk`：`hook_command(Path::new(r"C:\a b\jkk.exe"), "post_commit", r"D:\项目 x")` 等于 `"C:\a b\jkk.exe" --tsvn-hook post_commit --repo "D:\项目 x"`。

- [ ] **Step 2：跑测试，确认失败**

  在 `src-tauri/` 下跑 `cargo test --test tsvn_hooks`。预期：编译失败，模块不存在。

- [ ] **Step 3：实现**

  - `registration_roots`：取 `enumerate_wcs` 的 `wc_root_abspath` 去重。如果一个根的祖先目录也在集合里，就丢掉它（H3：TortoiseSVN 按路径前缀匹配，挂在祖先上已经覆盖）。
  - 注册表读写放在 `#[cfg(windows)]` 下，非 Windows 返回错误码 `tsvn_hooks_unsupported`。
  - `write_hooks_value` 是 CAS：写之前重读一次，和 `expected` 不同就报 `tsvn_hooks_registry_changed`，不写。写完再读一次，和 `new` 不同同样报这个码（Review Focus 第 2 条）。
  - `new == None` 表示删除这个值。

- [ ] **Step 4：跑测试，确认通过，再跑 fmt / clippy**

  `cargo test --test tsvn_hooks` 通过。`cargo fmt --check`、`cargo clippy --all-targets` 无告警。

- [ ] **Step 5：更新日志总账**

### Task 2：钩子子命令与收件箱写入

**Files:**
- Create: `src-tauri/src/engine/tsvn_hook_cli.rs`、`src-tauri/src/engine/tsvn_inbox.rs`（这个 Task 只写事件的结构与写入）
- Modify: `src-tauri/src/main.rs`（在 `app_lib::run()` 之前分流）、`src-tauri/src/lib.rs`（导出 `pub fn tsvn_hook_main(args: Vec<OsString>) -> Option<i32>`）、`Cargo.toml`（`Win32_Globalization`）
- Test: `src-tauri/tests/tsvn_hook_cli.rs`

**Interfaces:**
- Consumes：无。
- Produces：
  - `#[derive(Serialize, Deserialize)] #[serde(rename_all = "camelCase")] pub struct TsvnEvent { pub version: u32, pub kind: String, pub at_ms: i64, pub paths: Vec<String>, pub revision: Option<u64>, pub error: Option<String> }`
    - `version = 1`。
    - `paths`：commit 类取 `PATH`，`post_update` 取 `RESULTPATH`。
  - `pub fn inbox_dir(repo: &str) -> PathBuf`
  - `pub fn write_event(repo: &str, ev: &TsvnEvent) -> Result<PathBuf>`
  - `pub struct HookArgs { pub kind: String, pub repo: String, pub path_file: Option<PathBuf>, pub revision: Option<u64>, pub error_file: Option<PathBuf>, pub result_file: Option<PathBuf> }`
  - `pub fn parse_hook_args(args: &[OsString]) -> Option<HookArgs>`：第一个参数不是 `--tsvn-hook` 时返回 `None`，`main` 照常启动 GUI。
  - `pub fn to_repo_relative(repo: &str, abs: &str) -> Option<String>`：大小写不敏感的前缀匹配，`\` 与 `/` 等价。不在仓库内的路径返回 `None`。
  - `pub struct HookOutcome { pub code: i32, pub stderr: String }`
  - `pub fn run_hook(a: &HookArgs, now_ms: i64) -> HookOutcome`
  - `pub fn encode_thread_acp(s: &str) -> Vec<u8>`：Windows 下走 `CP_THREAD_ACP`，其他平台返回 UTF-8（原名 `encode_acp`，Task 7 更正）。

- [ ] **Step 1：写失败测试**

  用临时目录造 PATH、RESULTPATH、ERROR 文件，内容按 H2 / H4 的真实形状：CRLF、正斜杠、含中文和空格。
  - `parses_each_kind_by_position`：四个 kind 各一例，`revision` 与文件路径落在正确字段。
  - `non_hook_args_return_none`：`[]`、`["--foo"]` 都返回 `None`。
  - `post_commit_writes_one_event_with_repo_relative_paths`：`run_hook` 之后收件箱里正好 1 个文件。`paths == ["Code/A.cs", "代码/中文 文件.cs"]`，`revision == Some(3)`，`code == 0`。
  - `paths_outside_the_repo_are_dropped_and_an_empty_event_is_not_written`
  - `post_update_takes_resultpath_including_deleted_files`：RESULTPATH 列出一个磁盘上已不存在的文件，它仍然在 `paths` 里（H4）。
  - `post_commit_with_nonempty_error_records_the_error`
  - `unwritable_inbox_still_exits_zero`：`.git` 是一个文件。
  - `missing_path_file_exits_zero_without_event`
  - `encode_thread_acp_round_trips_ascii`：只在 Windows 上跑。
  - `to_repo_relative_handles_case_and_separators`：`C:/Repo/Code/A.cs` 相对于 `c:\repo` 得到 `Code/A.cs`；`C:/Repo2/x` 得到 `None`。

- [ ] **Step 2：跑测试，确认失败**

- [ ] **Step 3：实现**

  - `main.rs`：先调用 `app_lib::tsvn_hook_main(std::env::args_os().collect())`。返回 `Some(code)` 就 `std::process::exit(code)`；返回 `None` 再调用 `app_lib::run()`。
  - `tsvn_hook_main` 里不初始化日志插件。
  - 写 stderr 用 `encode_thread_acp` 之后的字节。
  - 这个 Task 里 `check_commit` 暂时只写事件、返回 0。拦截在 Task 4 接入。
  - `pre_commit` 的 `revision` 与 `error` 为 `None`。

- [ ] **Step 4：跑测试，确认通过；fmt / clippy**

- [ ] **Step 5：手测 GUI 子系统形态**

  `cargo build --release` 之后，直接跑 `target/release/app.exe --tsvn-hook post_commit --repo <临时仓库> <PATH文件> 3 <MSG> 7 <ERR> <CWD>`。确认不弹窗口、退出码 0、收件箱里有事件。结果记进日志。

- [ ] **Step 6：更新日志总账**

### Task 3：收件箱读取、配对与清理

**Files:**
- Modify: `src-tauri/src/engine/tsvn_inbox.rs`
- Test: `src-tauri/tests/tsvn_inbox.rs`

**Interfaces:**
- Consumes：Task 2 的 `TsvnEvent`、`write_event`、`inbox_dir`。
- Produces：
  - `pub struct CommitEvidence { pub t_commit_ms: Option<i64>, pub revision: u64, pub paths: BTreeSet<String> }`
  - `pub struct UpdateEvidence { pub revision: u64, pub paths: BTreeSet<String> }`
  - `#[derive(Default)] pub struct HookEvidence { pub commits: Vec<CommitEvidence>, pub updates: Vec<UpdateEvidence> }`
  - `pub fn read_evidence(repo: &str) -> HookEvidence`：读不了的文件跳过并记日志，不报错。
  - `pub fn prune_inbox(repo: &str, processed: &dyn Fn(&TsvnEvent) -> bool, now_ms: i64)`
  - 常量：`PAIR_WINDOW_MS = 600_000`、`ORPHAN_PRE_TTL_MS = 86_400_000`、`EVENT_TTL_MS = 14 * 86_400_000`、`MAX_EVENTS = 2000`。

- [ ] **Step 1：写失败测试**

  - `post_pairs_with_the_latest_preceding_pre_of_the_same_paths`：`t_commit_ms` 等于那条 Pre 的 `at_ms`。
  - `orphan_pre_from_a_failed_commit_does_not_pair_with_a_later_post`：两者间隔超过 `PAIR_WINDOW_MS`，结果 `t_commit_ms == None`（Review Focus 第 3 条）。
  - `pre_with_different_paths_does_not_pair`
  - `post_with_error_is_not_commit_evidence`
  - `post_update_becomes_update_evidence`
  - `check_commit_events_are_ignored_by_evidence`
  - `prune_removes_processed_old_orphans_and_caps_count`：已处理的删除；孤儿 Pre 超过 TTL 删除；超过 `MAX_EVENTS` 时先删最旧的；没处理的 Post 保留。
  - `garbage_file_in_inbox_is_skipped`

- [ ] **Step 2–4：先红后绿；fmt / clippy**

- [ ] **Step 5：更新日志总账**

### Task 4：Check-commit 按磁盘内容拦截 Local-Only 泄漏

**Files:**
- Modify: `src-tauri/src/engine/svn_leak.rs`（把按字节比较的核心从 `judge_path` 里抽出来）、`src-tauri/src/engine/tsvn_hook_cli.rs`
- Test: `src-tauri/tests/svn_leak.rs`（原有测试必须全绿）、`src-tauri/tests/tsvn_hook_cli.rs`

**Interfaces:**
- Consumes：Task 2 的 `run_hook`、`HookOutcome`。另外用 `svn_replay::commits_between`，以及跟进用的同一套 `FOLLOW_CODES`；如果那套码是私有的，就改成 `pub(crate)`。
- Produces：
  - `pub(crate) fn judge_bytes(b: Option<&[u8]>, l: Option<&[u8]>, n: Option<&[u8]>) -> Vec<HunkLeak>`：语义与现在的 `judge_path` 完全一致，`None` 表示文件不存在。`judge_path` 改成读 blob 之后调用它。
  - `pub fn local_only_leaks_on_disk(repo: &str, abs_paths: &[String]) -> Result<Vec<Leak>>`：对当前分支 `svn/base..HEAD` 里的 Local-Only commit，把 N 换成磁盘上这一刻的字节来判定。
  - `pub fn leak_message(leaks: &[Leak]) -> String`

- [ ] **Step 1：写失败测试**

  - `judge_bytes_matches_judge_path_on_w21_cases`：把 `leak_by_hunk_matches_w21_scenarios` 的输入改成字节再跑一遍，结论一致。
  - `restore_after_commit_with_patch_removed_is_not_a_leak`：H5 的形状。B 是 20 行；L 改第 2 行；磁盘是第 2 行恢复成 B、第 18 行改成 FEATURE。结果为空。
  - `whole_file_commit_with_patch_is_present`：磁盘等于 L 加上功能改动，结果判 `Present`。
  - `check_commit_hit_exits_one_with_message`：`run_hook` 跑 `check_commit` 时 `code == 1`。`stderr` 含相对路径和 commit 的 subject，最后一行是「取消勾选这些文件，或用「提交后恢复」删掉这些改动再提交。」
  - `check_commit_without_local_only_commits_exits_zero_fast`：分支上没有 Local-Only commit 时不读任何文件，只用 git2 列一遍 commit。
  - `check_commit_on_error_fails_open`：仓库坏了、`svn/base` 不存在，都是 `code == 0`。

- [ ] **Step 2–4：先红后绿**

  `cargo test --test svn_leak --test tsvn_hook_cli` 通过；fmt / clippy 无告警。

- [ ] **Step 5：更新日志总账**

  记一条「偏离计划」：分析 §6.13.4 写的是由 jkk 预先把 Local-Only 清单写进 `.git/mygitclient/local-only/`，这里改成钩子现场用 git2 读。理由：
  - 没有清单过期的问题；
  - 分支上没有 Local-Only commit 时只多一次 revwalk，耗时在 Task 2 Step 5 一并量。

### Task 5：跟进按钩子事件判定

**Files:**
- Modify: `src-tauri/src/engine/svn_follow.rs`（`Who`、F2 ① 的 `replaceable`、`FollowReport`、发布后清理收件箱）、`src-tauri/src/engine/svn_follow_mark.rs`（`FollowMark` 加 `at_ms`）
- Test: `src-tauri/tests/svn_follow.rs`、`src-tauri/tests/svn_follow_mark.rs`、`src-tauri/tests/svn_backend.rs`

**Interfaces:**
- Consumes：Task 1 的 `judge_health`、`registration_roots`、`read_hooks_value`、`parse_hooks`；Task 3 的 `read_evidence`、`prune_inbox`、`HookEvidence`。
- Produces：
  - `FollowMark.at_ms: Option<i64>`：`#[serde(default)]`，新快照一律写入。
  - `#[serde(rename_all = "camelCase")] pub enum Evidence { Hooks, Heuristic }`
  - `FollowReport.evidence: Evidence`：`#[serde(default)]`，默认 `Heuristic`。
  - `fault::pretend_hook_health(h: HookHealth) -> Armed`：测试用，不碰注册表。

- [ ] **Step 1：写失败测试**

  沿用 `tests/svn_follow.rs` 的 `tortoise_commit`、`svn_update`、`set_me`、`edit_mark`。新增 helper `hook_commit(fx, rel, pre_ms, post_ms) -> u64`：真提交一次，再用 `write_event` 写一对 Pre / Post。
  - `with_hooks_stage_older_than_mark_is_discarded_for_my_commit_event`：`rep.stage.discarded == [A]`，`rep.evidence == Hooks`。
  - `with_hooks_stage_differing_from_mark_is_kept_uncertain`
  - `with_hooks_mark_not_before_t_commit_is_uncertain`：`mark.at_ms >= t_commit_ms`。
  - `with_hooks_post_without_pre_is_uncertain`
  - `with_hooks_my_author_without_event_is_uncertain_not_discarded`：作者是我、快照也早，但没有事件。旧规则会丢弃，这里必须 `KeepStaged`。这条覆盖分析里 R22 说的「另一台机器的提交」。
  - `with_hooks_update_event_path_is_three_way_merged`：作者是我，但路径只在更新事件里出现，走三方合并。
  - `with_hooks_later_update_beats_earlier_commit_and_tie_goes_to_commit`（Review Focus 第 4 条）
  - `with_hooks_intermediate_commit_is_superseded_only_for_event_paths`：F2 ①。
  - `stale_hooks_fall_back_to_heuristic`：`HookHealth::Stale` 时，结论与不注册钩子完全相同，`evidence == Heuristic`（Review Focus 第 5 条）。
  - `follow_prunes_processed_events`：跟进完成后，revision 不大于映射的事件被删，更新的事件保留。
  - `svn_follow_mark.rs`：`mark_records_at_ms`；没有 `at_ms` 的旧快照仍能读出来。
  - `svn_backend.rs`：`FollowReport` 序列化出 `"evidence":"hooks"`。

- [ ] **Step 2：跑测试，确认失败**

- [ ] **Step 3：实现**

  - 在 `Who::gather` 里判健康（`fault` 优先），只有 `Healthy` 才读事件，存进 `Who.events: Option<HookEvidence>`。
  - `Who::facts` 在 `events` 是 `Some` 时按下表把事件映射成 `PathFacts`。表中的 M 指失焦快照，`t` 指 `t_commit_ms`。

    | 路径所在的事件（revision 最高的那条，相同时取提交） | `author_is_me` | `staged_before_mark` |
    |---|---|---|
    | 提交，`t` 为 `None`，或者 M 不存在 / 没有 `at_ms` / `at_ms >= t` | `Some(true)` | `Some(false)` |
    | 提交，M 的 `at_ms < t` | `Some(true)` | M 里的暂存 blob 等于当前暂存 |
    | 更新 | `Some(false)` | `None` |
    | 不在任何事件里，作者是我 | `Some(true)` | `Some(false)` |
    | 不在任何事件里，作者不是我或不知道 | 原值 | `None` |

  - `replaceable` 在有事件时取「提交事件里、M 的 `at_ms < t` 的路径」与 `after_mark` 的交集，不再用 `ext.mine`。
  - 发布成功之后调用 `prune_inbox`：事件里每条路径在新映射里的 revision 都不小于事件的 revision，就算已处理。只在 `Healthy` 时清理。

- [ ] **Step 4：跑全量测试**

  `cargo test --no-fail-fast` 全绿（`merge_drivers` 原有 1 个 ignored）。C1a 原有的跟进测试一条都不改（它们走 `NotRegistered`）。fmt / clippy 无告警。

- [ ] **Step 5：更新日志总账**

### Task 6：Tauri 命令与前端入口

**Files:**
- Modify: `src-tauri/src/commands/svn.rs`、`src-tauri/src/lib.rs`（注册命令）、`src/store/repoStore.ts`、`src/engine/svnFollow.ts`（`Evidence`）、`src/components/BaselineBanner.tsx`（入口与失效提示）、`src/components/FollowReportDialog.tsx`（显示判定依据）
- Create: `src/engine/tsvnHooks.ts`、`src/components/TsvnHooksDialog.tsx`
- Test: `src-tauri/tests/svn_backend.rs`、`tests/vitest/tsvnHooks.test.ts`、`tests/vitest/TsvnHooksDialog.test.tsx`

**Interfaces:**
- Consumes：Task 1 的全部；Task 5 的 `Evidence`。
- Produces：
  - `#[tauri::command] tsvn_hooks_status(path: String) -> Result<TsvnHooksStatus>`。`TsvnHooksStatus { supported: bool, health: HookHealth, planned: Vec<HookEntry>, conflicts: Vec<HookEntry> }`，其中 `planned` 按 `block_leaks = true` 计算。
  - `tsvn_hooks_register(path: String, block_leaks: bool) -> Result<HookHealth>`：有冲突时报 `tsvn_hooks_conflict`，不写。
  - `tsvn_hooks_unregister(path: String) -> Result<()>`
  - TS 侧：`tsvnHooksStatus`、`tsvnHooksRegister(path, blockLeaks)`、`tsvnHooksUnregister`；`healthLabel(h: HookHealth): string`。

- [ ] **Step 1：写失败测试**

  - `svn_backend.rs`：`TsvnHooksStatus` 与 `HookHealth` 三个变体的 JSON 形状，全部 camelCase，`state` 作 tag。
  - `tsvnHooks.test.ts`：`healthLabel` 三种状态的文案；`Stale` 时把每条 reason 列出来。
  - `TsvnHooksDialog.test.tsx`：
    - 打开后列出将写入的每一条（kind、工作副本路径、命令行），默认勾选「提交前拦截本地补丁」。
    - 取消勾选后确认，调用 `register(path, false)`。
    - 有冲突时确认按钮禁用，并列出冲突的条目。
    - 已注册时显示「撤销注册」。
    - `supported == false` 时只显示说明。

- [ ] **Step 2–3：先红；实现**

  - 入口放在 `BaselineBanner` 现有的「手动体检」旁边，按钮文字「TortoiseSVN 钩子…」。
  - 健康状态为 `Stale` 时，横幅加一行「TortoiseSVN 钩子已失效：<第一条原因>。TortoiseSVN 提交时可能报钩子错误；跟进暂按作者 + 快照判定」，带「重新注册」和「撤销注册」两个按钮。撤销注册在 `Stale` 下也必须能用（review 1 第 5 条：exe 不在了时，TortoiseSVN 每次提交都会报钩子失败）。
  - 注册对话框里，当前 exe 位于 `target\debug`、`target\release`，或某个 `worktrees` 目录下时，显示警告：「这是开发构建。删掉这个目录之后，TortoiseSVN 提交会报钩子错误，请先撤销注册。」不禁止注册：开发时要靠它实测。
  - `FollowReportDialog` 加一行判定依据：「TortoiseSVN 钩子事件」或「作者 + 失焦快照」。
  - 组件不直接 `invoke`，统一经过 store。

- [ ] **Step 4：验证**

  - 前端：`pnpm build`、`pnpm test` 全绿。
  - Rust：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。
  - 实际跑一次 `pnpm tauri dev`，调用三个命令各一次，确认 `blockLeaks` 参数接通（AGENTS.md §7.3）。

- [ ] **Step 5：更新日志总账**

### Task 7：E2E 清单与文档回写

**Files:**
- Create: `docs/superpowers/e2e/2026-11-02-svn-c1h-tsvn-hooks-e2e.md`
- Modify: `docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md`（§6.13、§9 状态、§11 的钩子条目）、`docs/superpowers/e2e/2026-11-01-svn-mixed-baseline-follow-e2e.md`（把场景 13 标为「已并入 C1h E2E」）

- [ ] **Step 1：写 E2E 清单**

  写法与 C1a 那份相同：PowerShell 可以直接复制粘贴，fixture 放在**含空格和中文的路径**下。至少覆盖：
  1. 在 jkk 里注册：TortoiseSVN 设置里能看到条目，已有的条目不受影响。
  2. 原场景 13：暂存中间版本，用 TortoiseSVN 提交最终版，跟进后丢弃该暂存，判定依据显示「钩子」。
  3. 同一路径用命令行 `svn ci` 提交（不经过钩子）：跟进后进入「拿不准」，暂存保留。
  4. 子目录更新：更新事件里的路径与暂存走三方合并。
  5. 「提交后恢复」删掉本地补丁再提交：不拦截；整文件提交：Check-commit 拦下，中文正常显示，Abort 后回到提交框。
  6. 过期导致提交失败，更新之后再提交：不误配对。
  7. 把 exe 换个目录启动：横幅提示失效；重新注册后恢复。另外实测**原位置的 exe 不存在时** TortoiseSVN 提交的真实表现（review 1 第 5 条），以及工作副本在盘符根目录（`CWD` 是 `D:\`）时 Post-update 的 RESULTPATH 是否还能拿到。
  8. 撤销注册：TortoiseSVN 设置里只剩用户自己的条目。

- [ ] **Step 2：在真实窗口里逐条跑，记录结果**

- [ ] **Step 3：回写分析文档**

  - §6.13 按实测更新：拦截改在 Check-commit；Post-commit 在失败时不会被调用；`CWD` 是勾选文件的公共父目录；stderr 按线程 ANSI 代码页编码（原写 ACP，Task 7 更正）。
  - §11 中钩子相关的条目打勾，或写明余下什么没测。
  - §9 表格把 C1h 标成「已实现」，同时把 C1a 的「未合并」改成「已合并」。
