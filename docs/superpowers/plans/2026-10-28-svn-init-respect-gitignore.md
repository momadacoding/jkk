# 建库时可选「尊重已有的 .gitignore」实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建库向导检测起作用的 `.gitignore`，让用户选择「尊重」（持久口径、只做减法）或「不尊重」（改名停用）。所有提到 `.gitignore` 的报告都写完整路径加「第 N 行」。

**Architecture:**
- 新模块 `svn_gitignore.rs`：负责问 git「这条路径是谁裁决的」（真仓库或临时探针 git 目录）、检测冲突、改名和恢复、路径显示。
- 新模块 `svn_scope.rs`：`WScope` 是全项目唯一的 W-file 口径。`is_w_file` 收进这个模块，改成私有，由编译器保证没有入口绕过它。
- 建库编排（`svn_init.rs`）在 `git init` 之后检测，过闸门后改名或切到「尊重」口径，失败时连改过的名一起回退。

**Tech Stack:** Rust（Tauri 2 命令、git CLI 经 `run_git` 一族），React 18 + TS，cargo test（SVN + git fixture），vitest（含 jsdom 组件测试）。

**Spec:** `docs/superpowers/specs/2026-10-28-svn-init-respect-gitignore-design.md`（实施者必须同时读 spec）

## Global Constraints

- 新配置字段**缺省为 false**，旧仓库行为逐字不变；选了「尊重」时 `version` 写 2，否则写 1；`validate` 只接受 1 和 2。
- 「尊重」**只做减法**：`!` 反选造成的「多收」在两种模式下都拦下。
- 停用后的新名字：`.gitignore.jkk-disabled`；撞名时**不能覆盖**。
- 「起作用」的判定：挡住了服务器期望集合（`server_expected_set` 口径）里的 W-file，或用 `!` 把非 W-file 纳入。只挡未版本控制文件的不算。
- 「已在当前基线树里的文件」不会被后来的 `.gitignore` 移出 W-file。所有入口统一以 `refs/heads/svn/base` 的树为参照。
- 检测与预检不写用户的工作副本。唯一的写操作是用户确认后的改名。
- 显示用完整 Windows 路径（仓库根 + 相对路径，`/` 换成 `\`），规则写「第 N 行 `rule`」；`.git/info/exclude` 的规则写「本应用生成的规则」。
- 所有 git 调用走 `run_git` 一族，错误映射为 `GitError`；空白名单、空路径列表绝不能退化成「全部」（`AGENTS.md` §6.2）。
- 改 Tauri 命令签名必须前后端同步，并在 `pnpm tauri dev` 里实跑（`AGENTS.md` §7.3）。
- **提交**：`AGENTS.md` §1.1 比全局规则更具体——每个 Task 结束只更新决策日志。`git commit` 等用户在对话里放行再做。
- 决策日志 `docs/superpowers/plans/2026-10-28-svn-init-respect-gitignore-decisions.md` 在 Task 1 之前建好，每个 Task 完成立即更新（`AGENTS.md` §2.1）。

## Review Focus

1. **目录规则挡住的、磁盘上不存在的服务器文件**：`Plugins/` 挡住整个目录，而其中 `Plugins/x.dll` 在本地被删（svn `!`）。期望行为：`check-ignore` 仍判它被挡，「尊重」模式下它不算 W-file。→ Task 3 的 `scope_excludes_a_missing_file_under_an_ignored_directory`。
2. **大小写不敏感的规则**：`.gitignore` 写 `*.EXE`，磁盘上是 `a.exe`。探针 git 目录与建库后的真仓库对 `core.ignoreCase` 必须给同样的裁决，否则第 2 步显示的和建库时拦的不一致。→ Task 5 的 `probe_and_repo_agree_on_case_insensitive_rules`。
3. **非 ASCII 路径**：`资源/.gitignore`，以及中文样例路径。期望行为：完整路径和样例原样显示，不出现八进制转义。→ Task 5 的 `conflicts_keep_non_ascii_paths_verbatim`。
4. **连续撞名**：`.gitignore.jkk-disabled` 与 `-2` 都已存在。期望行为：改成 `-3`，前两个文件的内容不变。→ Task 7 的 `disabling_never_overwrites_an_existing_disabled_file`。
5. **回退时原位置又出现了新的 `.gitignore`**：期望行为：恢复时不覆盖它，错误里点名两个文件。→ Task 7 的 `restore_never_overwrites_a_gitignore_that_reappeared`。

---

### Task 1: 报告写完整路径和「第 N 行」，并抽出裁决查询

**Files:**
- Create: `src-tauri/src/engine/svn_gitignore.rs`
- Modify: `src-tauri/src/engine/mod.rs`（注册模块）
- Modify: `src-tauri/src/engine/svn_include_check.rs`：把 `Verdict` / `blame_rules` / `is_worktree_gitignore` 挪走，`mismatch_report` 改用 `describe`
- Modify: `src/engine/svnInit.ts`：`include_set_mismatch` 引导语里如有 `文件:行号:规则` 字样，相应改写
- Create: `docs/superpowers/plans/2026-10-28-svn-init-respect-gitignore-decisions.md`
- Test: `src-tauri/tests/svn_exclude.rs`、`src-tauri/tests/svn_init.rs`（改断言）

**Interfaces:**
- Produces（`svn_gitignore.rs`）：
  - `pub(crate) enum GitCtx<'a> { Repo(&'a str), Probe { work_tree: &'a str, git_dir: &'a Path } }`
    - `fn work_tree(&self) -> &str`
    - `fn run_code(&self, args: &[&str]) -> Result<(i32, String)>`
    - `fn run_stdin_code(&self, args: &[&str], stdin: &[u8]) -> Result<(i32, String)>`
    - `Probe` 在 args 前补 `--git-dir=<git_dir>` 与 `--work-tree=<work_tree>`，cwd 取 `work_tree`。本 Task 只用 `Repo`。
  - `pub(crate) struct Verdict { pub source: String, pub line: String, pub pattern: String }`
    - `fn ignores(&self) -> bool`
    - `fn from_worktree_gitignore(&self) -> bool`：原 `is_worktree_gitignore` 的判据
    - `fn describe(&self, repo_root: &str) -> String`
  - `pub(crate) fn query_verdicts(ctx: &GitCtx, files: &[&str]) -> Result<BTreeMap<String, Verdict>>`：原 `blame_rules`，参数与解析不变
  - `pub fn display_path(repo_root: &str, rel: &str) -> String`

- [ ] **Step 1: 建决策日志**（进度总账表 + 四类决策条目的空节），总账列出 Task 1–9。

- [ ] **Step 2: 写失败测试**
  - `tests/svn_exclude.rs` 里四处 `N 个被 Code/.gitignore:1:*.exe 挡掉` 改成 ``format!("N 个被 {} 第 1 行 `*.exe` 挡掉", full)``，其中 `full = format!("{}\\Code\\.gitignore", wc.display().to_string().replace('/', "\\"))`。
  - `tests/svn_init.rs:217` 同样改写。
  - 新增 `report_names_generated_rules_without_exposing_the_exclude_path`：一个多收的文件（`git add -f` 进候选索引，被 `/Code/**` 排除）。报告含 ``本应用生成的规则 `/Code/**` ``，不含 `info/exclude`。
  - 新增 `display_path_is_windows_shaped_at_any_depth`：
    - `display_path("C:/w/wc1", "Assets/Plugins/.gitignore") == r"C:\w\wc1\Assets\Plugins\.gitignore"`
    - 根带尾斜杠（`"C:\\w\\wc1\\"`）时不出现双反斜杠

- [ ] **Step 3: 跑测试确认失败**：`cd src-tauri; cargo test --test svn_exclude --test svn_init`，预期上面几条 FAIL。

- [ ] **Step 4: 实现 `svn_gitignore.rs` 的上述接口，并改 `mismatch_report`**
  - `describe` 三种形态：
    - 工作树 `.gitignore`：``{display_path} 第 {line} 行 `{pattern}` ``
    - 来源以 `info/exclude` 结尾：``本应用生成的规则 `{pattern}` ``
    - 其余（全局文件，绝对路径）：``{source} 第 {line} 行 `{pattern}` ``
  - 分组标签里原来的 `v.rule()` 全部换成 `v.describe(repo_root)`。

- [ ] **Step 5: 跑测试确认通过**：`cargo test --test svn_exclude --test svn_init`，然后全量 `cargo fmt --check; cargo clippy --all-targets; cargo test`。改了 `svnInit.ts` 就再跑 `pnpm test`。

- [ ] **Step 6: 更新决策日志总账**

---

### Task 2: 配置新增「尊重」与改名记录，版本 2

**Files:**
- Modify: `src-tauri/src/engine/svn_config.rs`
- Modify: `src-tauri/src/engine/svn_init.rs`（`build_config` 的结构体字面量）
- Test: `src-tauri/tests/svn_config.rs`（`ok_config()` 字面量补新字段）

**Interfaces:**
- Produces：
  - `SvnConfig` 新增两个字段：
    - `pub respect_gitignore: bool`：`#[serde(default, skip_serializing_if = "std::ops::Not::not")]`
    - `pub disabled_gitignores: Vec<DisabledGitignore>`：`#[serde(default, skip_serializing_if = "Vec::is_empty")]`
  - `#[serde(rename_all = "camelCase")] pub struct DisabledGitignore { pub from: String, pub to: String }`：仓库相对路径，正斜杠
  - 常量：
    - `pub const RESPECT_GITIGNORE_VERSION: u32 = 2`
    - `SUPPORTED_VERSION` 换成 `pub const SUPPORTED_VERSIONS: [u32; 2] = [1, 2]`；全仓 grep 旧名，一并改掉

- [ ] **Step 1: 写失败测试**（`tests/svn_config.rs`）
  - `version_2_is_accepted_only_with_respect`：
    - `{version:2, respect:true}` 通过；
    - `{version:1, respect:true}` 报 `svn_config`，消息含 `respectGitignore`；
    - `{version:3}` 报 `svn_config`。
  - `a_v1_config_round_trips_byte_identical`：不含新字段的旧 JSON 经 `read_config` → `write_config` 后，不出现 `respectGitignore` / `disabledGitignores` 键。
  - `respect_config_is_written_as_version_2`：`write_config` 一份 respect 配置，读回原始 JSON，`"version": 2`。旧客户端的 `validate` 是 `version != 1` 就拒，这条断言就是「旧版明确拒绝」的保证。

- [ ] **Step 2: 跑测试确认失败**：`cargo test --test svn_config`

- [ ] **Step 3: 实现**
  - `validate` 的规则：
    - `version` 必须在 `SUPPORTED_VERSIONS` 里；
    - `respect_gitignore` 为 true 时 `version` 必须是 2；
    - `version` 为 2 而不尊重时放行。
  - 错误文案沿用现有的「请确认这个配置文件是不是给更新版本的客户端写的」。
  - `disabled_gitignores` 只记录，不进任何命令行，不校验。

- [ ] **Step 4: 跑测试确认通过**：先 `cargo test --test svn_config`，再全量 Rust 三件套。

- [ ] **Step 5: 更新决策日志**

---

### Task 3: `WScope`——全项目唯一的 W-file 口径

**Files:**
- Create: `src-tauri/src/engine/svn_scope.rs`
- Modify（签名改为收 `&WScope`，调用点用 `WScope::for_repo(repo, &cfg)?` 构造）：
  - `svn_include_check.rs`：`server_expected_set`、`verify_include_set`、`dry_run_include_set`；删掉 `is_w_file`
  - `svn_dirty.rs`：`explained_paths`
  - `svn_health.rs`：`check_baseline`，即第 211 行的过滤
  - `svn_server_set.rs`：第 159、180 行
  - `svn_sync_check.rs`：第 112、145、160 行，以及 `precheck_include_set`
  - `svn_sync_in.rs`：`dirty_w_files`
  - `svn_sync_out_check.rs`：第 115、274、322 行
  - `svn_init.rs`：所有调用点；本 Task 里建库仍按 `cfg` 构造，`cfg.respect_gitignore` 恒为 false
- Test: 新建 `src-tauri/tests/svn_scope.rs`；改 `tests/svn_health.rs`、`svn_health_dirty.rs`、`svn_exclude.rs`、`svn_server_set.rs` 的调用（约 27 处，都包成 `&WScope::plain(&wl, &ext)`）

**Interfaces:**
- Consumes：Task 1 的 `query_verdicts`、`GitCtx::Repo`、`Verdict::{ignores, from_worktree_gitignore}`；Task 2 的 `cfg.respect_gitignore`
- Produces（`svn_scope.rs`）：
  - `pub struct WScope`：私有字段 `whitelist`、`roots`、`extensions`，以及 `respect: Option<Respect { repo_root: String, tracked: BTreeSet<String> }>`
  - `pub fn plain(whitelist: &[String], extensions: &[String]) -> Self`
  - `pub fn for_repo(repo_root: &str, cfg: &SvnConfig) -> Result<Self>`：不尊重时等于 `plain`，**不起任何进程**
  - 访问器：`whitelist(&self) -> &[String]`、`extensions(&self) -> &[String]`、`roots(&self) -> &[String]`、`respects_gitignore(&self) -> bool`
  - `pub fn filter<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> Result<BTreeSet<String>>`：W-file 判定
  - `pub(crate) fn retain_not_ignored(&self, set: &mut BTreeSet<String>) -> Result<()>`：只做「尊重」那一半
  - `pub fn hidden_tracked_on_disk(&self) -> Result<Vec<String>>`：Task 4 使用；本 Task 先实现并测
- 新签名：
  - `server_expected_set(repo_root, scope, fixes)`
  - `verify_include_set(repo_root, idx, scope, fixes)`
  - `dry_run_include_set(repo_root, scope, fixes, touched)`
  - `explained_paths(fixes, scope) -> Result<BTreeSet<String>>`
  - `check_baseline(repo_root, svn_url, rev, scope, explained, origin)`
  - `precheck_include_set(repo_root, scope)`

判定算法（`filter` / `retain_not_ignored` / `hidden_tracked_on_disk` 共用）：

```text
W(p) = path_is_w(p)                       // 原 is_w_file，原样搬进来、改成私有
       ∧ ( 不尊重
           ∨ p ∈ tracked                  // 参照：refs/heads/svn/base 的树
           ∨ ¬(v = verdict(p); v.ignores() ∧ v.from_worktree_gitignore()) )
verdict：对「path_is_w 为真且不在 tracked 里」的路径一次批量 query_verdicts(GitCtx::Repo)
tracked：rev-parse --verify -q refs/heads/svn/base 退出 1 时为空（建库途中）；
         否则 ls-tree -r -z --name-only refs/heads/svn/base
hidden_tracked_on_disk = { p ∈ tracked | path_is_w(p) ∧ 磁盘上是文件 ∧ verdict 被工作树 .gitignore 挡 }
```

- [ ] **Step 1: 写失败测试**（`tests/svn_scope.rs`）

  fixture 用 `svn_repo()` 加正常 `init_hybrid`，再用测试内 helper `enable_respect(wc)` 改配置：读配置 → `respect_gitignore = true`、`version = 2` → `write_config`。
  - `plain_scope_ignores_gitignore_entirely`：`.gitignore` 写 `*.cs`。`plain` 的 `filter(["Code/A.cs"])` 含它，且不起 git 进程（用一个不是仓库的目录当 `repo_root` 也能跑通即可证明）。
  - `respect_scope_excludes_files_hidden_by_a_worktree_gitignore`：`Code/.gitignore` 写 `Gen*.cs`，新增未进基线的 `Code/Gen1.cs`，`filter` 不含它，含 `Code/A.cs`。
  - `respect_scope_keeps_files_already_in_the_baseline`：建库后再写 `*.cs` 的 `.gitignore`。`filter(["Code/A.cs"])` 仍含它；`hidden_tracked_on_disk()` 为 `["Code/A.cs","Code/B.cs"]`。
  - `scope_excludes_a_missing_file_under_an_ignored_directory`（Review Focus 第 1 条）：`Code/.gitignore` 写 `Gen/`，`Code/Gen/X.cs` 不存在于磁盘，`filter(["Code/Gen/X.cs"])` 为空。**先用这条实测 `check-ignore` 对「不存在路径 + 目录规则」的真实行为**，结论记进决策日志。
  - `respect_scope_never_admits_non_w_files`：`!*.txt` 不会让 `Code/n.txt` 进 `filter`。
  - `empty_input_spawns_nothing`：`filter([])` 返回空集。

- [ ] **Step 2: 跑测试确认失败**：`cargo test --test svn_scope`（编译失败即算 FAIL）

- [ ] **Step 3: 实现 `svn_scope.rs`，再逐个入口换签名**
  - `server_expected_set` 的顺序：
    1. `expected_set` 原样遍历磁盘；
    2. `retain_not_ignored`；
    3. `Drop` 照旧删除；
    4. `TakeBase` 的路径先过 `scope.filter` 再插入。
  - 各入口里的循环判定改成「先 `filter` 收集，再查集合」，每处一次批量。

- [ ] **Step 4: 跑测试确认通过**：`cargo test --test svn_scope`，再全量 Rust 三件套。不尊重口径下，既有测试必须**一条不改断言**全部通过。

- [ ] **Step 5: 更新决策日志**（含 Step 1 的 `check-ignore` 实测结论）

---

### Task 4: 从空索引起步的入口补回已在基线里的文件，并逐入口验证「尊重」口径

**Files:**
- Modify: `src-tauri/src/engine/git_cli.rs`：`pipe_through` 加 `envs` 参数；新增 `run_git_stdin_env`
- Modify: `src-tauri/src/engine/svn_baseline.rs`：新增 `TempIndex::add_paths`
- Modify: `src-tauri/src/engine/svn_init.rs`：`record_server_tree_timed` 在 `add_all` 后调 `add_paths(scope.hidden_tracked_on_disk()?)`；`check_current_baseline` 同样处理
- Modify: `src-tauri/src/engine/svn_include_check.rs`：`dry_run_include_set` 的 actual 并入 `hidden_tracked_on_disk`
- Modify: `src-tauri/src/engine/svn_sync_check.rs`：`precheck_include_set` 同样处理
- Test: 新建 `src-tauri/tests/svn_respect.rs`

**Interfaces:**
- Consumes：Task 3 的 `WScope::{for_repo, hidden_tracked_on_disk}`
- Produces：
  - `pub fn run_git_stdin_env(path: &str, args: &[&str], stdin: &[u8], envs: &[(&str, &str)]) -> Result<String>`
  - `TempIndex::add_paths(&self, repo_root: &str, paths: &[String]) -> Result<()>`
    - 实现：`update-index --add -z --stdin`，带 `GIT_INDEX_FILE`；
    - 空列表是 no-op；
    - `update-index` 不读忽略规则，所以能把被挡的文件放进去。

- [ ] **Step 1: 写失败测试**（`tests/svn_respect.rs`，fixture 与 Task 3 相同：建库 → `enable_respect` → 写一个挡住基线文件的 `.gitignore`）
  - `add_paths_bypasses_ignore_rules_and_matches_hash_object`：被 `*.cs` 挡住的 `Code/A.cs` 经 `add_paths` 进候选索引，blob 等于 `git hash-object --path Code/A.cs Code/A.cs`（行尾过滤照常生效）。
  - `respect_rebuild_keeps_baseline_files_a_later_gitignore_hides`：`rebuild_baseline` 成功，新基线树仍含 `Code/A.cs`。
  - `respect_manual_health_check_is_not_lying_when_a_later_gitignore_hides_baseline_files`：`check_current_baseline` 不是 `Lying`。
  - `respect_update_precheck_passes_with_gitignored_versioned_files`：`Code/Gen1.cs` 受 SVN 版本控制、未进基线、被 `.gitignore` 挡。`preview_sync_in` 不报 `include_set_mismatch`。对照：同一形态在不尊重口径下报 `include_set_mismatch`。
  - `respect_delivery_treats_gitignored_files_as_outside_git`：本地改 `Code/Gen1.cs`。`preview_sync_out` 把它列进「白名单内 git 管不到的改动」，而不是要求 git 干净。
  - `respect_server_audit_does_not_flag_gitignored_server_files`：`audit_server_set_at_baseline` 为 `Matched`。
  - `respect_health_filter_drops_gitignored_diffs`：本地改 `Code/Gen1.cs`（svn `M`），`check_current_baseline` 的 `unverified` 不含它。

- [ ] **Step 2: 跑测试确认失败**：`cargo test --test svn_respect`

- [ ] **Step 3: 实现上述接口与调用点**

- [ ] **Step 4: 跑测试确认通过**：`cargo test --test svn_respect`，再全量 Rust 三件套。

- [ ] **Step 5: 更新决策日志**（「为什么 `update-index --add` 安全」写进安全论证）

---

### Task 5: 冲突检测（第 2 步用探针，建库时用真仓库）

**Files:**
- Modify: `src-tauri/src/engine/svn_gitignore.rs`
- Modify: `src-tauri/src/engine/svn_baseline.rs`：`w_pathspecs` 改成 `pub(crate)`
- Modify: `src-tauri/src/engine/svn_init.rs`：新增 `InitPreflight` 与 `init_preflight`；`build_config` 改为收 `(repo_root, whitelist, extensions, svn_url)`，供两处共用
- Test: 新建 `src-tauri/tests/svn_gitignore.rs`

**Interfaces:**
- Consumes：Task 1 的 `GitCtx`、`query_verdicts`、`display_path`；`svn_exclude::generate_exclude`；`svn_include_check::server_expected_set` 配 `WScope::plain`；`svn_dirty::classify`
- Produces（`svn_gitignore.rs`）：
  - `pub(crate) struct ProbeGitDir`
    - `fn create(work_tree: &str, whitelist: &[String], extensions: &[String]) -> Result<Self>`
    - `fn ctx(&self) -> GitCtx<'_>`
    - `Drop` 时删除整个目录
  - `#[serde(rename_all = "camelCase")] pub struct GitignoreConflict { pub path: String, pub full_path: String, pub versioned: bool, pub rules: Vec<GitignoreRule> }`：`path` 是仓库相对路径、正斜杠
  - `pub struct GitignoreRule { pub line: u32, pub pattern: String, pub effect: RuleEffect, pub count: usize, pub samples: Vec<String> }`：`samples` 最多 3 个，按字母序
  - `#[serde(rename_all = "camelCase")] pub enum RuleEffect { Excludes, Includes }`
  - `pub(crate) fn detect_conflicts(ctx: &GitCtx, whitelist: &[String], extensions: &[String], fixes: &BTreeMap<String, Fix>) -> Result<Vec<GitignoreConflict>>`：按 `path` 排序，规则按行号排序
- Produces（`svn_init.rs`）：
  - `#[serde(rename_all = "camelCase")] pub struct InitPreflight { #[serde(flatten)] pub preflight: Preflight, pub gitignore_conflicts: Vec<GitignoreConflict> }`
  - `pub fn init_preflight(repo_root: &str, whitelist: &[String], extensions: &[String]) -> Result<InitPreflight>`：顺序为 svn info → `build_config`（校验与建库同一套）→ `run_preflight` → 探针检测

检测算法：

```text
少收：cands = server_expected_set(root, plain, fixes)
      对 cands 跑 query_verdicts；v.ignores() ∧ v.from_worktree_gitignore() 的，按 (v.source, v.line) 归组
多收：others = ctx 下 ls-files -o --exclude-standard -z -- <w_pathspecs>（尾斜杠去掉；
               磁盘上是含 .git 的目录的跳过——嵌套仓库不是 .gitignore 的问题，最终核对会报）
      over = others − expected_set(root, wl, ext)
      对 over 跑 query_verdicts；pattern 以 '!' 开头 ∧ from_worktree_gitignore 的，按 (source, line) 归组
versioned：每个命中的 .gitignore 跑 svn status --xml -v --depth empty -- <peg_safe>，
           item ∉ {unversioned, ignored, none} 即受版本控制（added 也算）
空白名单或空扩展名：直接返回空，不起 git
探针：在 std::env::temp_dir() 下建唯一目录（pid + 纳秒 + 进程内序号），里面 git init -q；
      写 info/exclude = generate_exclude(...)；
      core.ignoreCase 显式设成工作副本的实际情况（work_tree 下 `.SVN` 能解析到 `.svn` 即 true）
```

- [ ] **Step 1: 写失败测试**（`tests/svn_gitignore.rs`，SVN fixture。白名单 `Code/`，扩展名 `*`，未建库时调用 `init_preflight`）
  - `an_unversioned_gitignore_hiding_versioned_files_is_reported_with_full_path_line_and_samples`：
    - 已提交 `Code/a.exe`、`Code/b.exe`；未版本控制的 `Code/.gitignore` 写 `*.exe`。
    - 得到 1 个冲突：`full_path` 以 `\Code\.gitignore` 结尾，`versioned == false`，`rules == [{line:1, pattern:"*.exe", effect:Excludes, count:2, samples:["Code/a.exe","Code/b.exe"]}]`。
  - `a_gitignore_that_only_hides_unversioned_junk_is_not_a_conflict`：只挡未版本控制的 `Code/x.obj`，结果为空。
  - `a_versioned_gitignore_is_flagged_versioned`
  - `a_negation_that_admits_unselected_files_is_reported_as_includes`：
    - 扩展名 `.cs`，`Code/.gitignore` 写 `!*.txt`，磁盘上有 `Code/n.txt`。
    - 得到 `effect == Includes`，`count == 1`。
  - `a_nested_gitignore_reports_its_own_location`：`Code/Sub/.gitignore` 的 `path == "Code/Sub/.gitignore"`。
  - `probe_and_repo_agree_on_case_insensitive_rules`（Review Focus 第 2 条）：
    - `.gitignore` 写 `*.EXE`。
    - 探针检测与在真仓库（手工 `git init` 到另一份副本）上用 `GitCtx::Repo` 检测，结果相同。
  - `conflicts_keep_non_ascii_paths_verbatim`（Review Focus 第 3 条）：`资源/.gitignore` 与 `资源/图.exe`，原样出现。
  - `preflight_detection_leaves_no_trace`：调用后工作副本里没有 `.git`；`temp_dir()` 下没有残留的探针目录（按前缀统计，调用前后相等）。
  - `invalid_extensions_are_rejected_at_preflight`：扩展名 `exe`（缺点号），报 `svn_config`。

- [ ] **Step 2: 跑测试确认失败**：`cargo test --test svn_gitignore`

- [ ] **Step 3: 实现上述接口**

- [ ] **Step 4: 跑测试确认通过**：先 `cargo test --test svn_gitignore`，再全量 Rust 三件套。

- [ ] **Step 5: 更新决策日志**

---

### Task 6: 建库的检测阶段与「尊重」路径

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_init.rs`（`req()` 补新字段；改写两条旧用例；新增用例）

**Interfaces:**
- Consumes：
  - Task 2 的配置字段与版本常量；
  - Task 3 的 `WScope::for_repo`；
  - Task 5 的 `detect_conflicts`、`GitCtx::Repo`、`GitignoreConflict`
- Produces：
  - `InitRequest` 新增 `pub respect_gitignore: bool` 与 `pub acknowledged: Vec<String>`。后者是第 3 步展示过的 `.gitignore` 的 `path`。
  - `InitReport` 新增 `pub gitignore: GitignoreOutcome`
  - `#[serde(rename_all = "camelCase")] pub struct GitignoreOutcome { pub respected: bool, pub conflicts: Vec<GitignoreConflict>, pub disabled: Vec<RenamedGitignore> }`
  - `pub struct RenamedGitignore { pub from: String, pub to: String }`：完整 Windows 路径，Task 7 填写
  - 错误码 `svn_gitignore_unconfirmed`、`svn_gitignore_includes`

`build_repo` 的新顺序：

```text
建仓库、写规则（git init）                   ← 只做 init + exclude + attributes，不再写配置
检查起作用的 .gitignore（git check-ignore）   ← 新阶段：detect_conflicts(GitCtx::Repo)
  闸门 1：有冲突不在 acknowledged 里 → svn_gitignore_unconfirmed
  闸门 2：respect 且有 Includes → svn_gitignore_includes
  （不尊重的分支在 Task 7）
write_config(cfg)                          ← respect 时 version=2、respect_gitignore=true
纳入集合预检 … 后续不变，scope 都从 WScope::for_repo(root, &cfg) 来
```

错误文案（后端，原样使用）：
- `svn_gitignore_unconfirmed`：「发现了你在上一步没看到的 .gitignore：\n{每行一个完整路径}\n什么都没改，刚建的 .git 已删除。请看一下新的清单，重新确认后再建库。」
- `svn_gitignore_includes`：「{full_path} 第 {line} 行 `{pattern}` 会把你没选的文件拉进 git（{count} 个，例如 {samples}）。「尊重」只允许 .gitignore 少收文件、不允许多收。请取消勾选，或先修改这一行。」

- [ ] **Step 1: 写失败测试**（`tests/svn_init.rs`）
  - 改写 `a_gitignore_is_caught_by_the_precheck_before_anything_is_recorded`：根 `.gitignore` 写 `*.cs`。默认请求（`acknowledged` 为空）报 `svn_gitignore_unconfirmed`，消息含完整路径，`.git` 不存在。原来「预检拦下 `.gitignore`」的覆盖面，转由新增的嵌套仓库用例承担：`a_nested_repo_is_still_caught_by_the_precheck`，断言 `include_set_mismatch` 且消息含「预检」。
  - 改写 `the_precheck_does_not_block_what_the_final_check_accepts_on_a_dirty_working_copy`：请求带 `respect_gitignore: true` 与 `acknowledged: [".gitignore"]`，建库成功。
  - `respect_init_succeeds_and_leaves_hidden_files_out_of_git`：
    - 已提交 `Code/Gen.cs`，`.gitignore` 写 `Gen.cs`，respect + acknowledged。
    - 成功；基线树不含 `Code/Gen.cs`；配置 `version == 2`、`respectGitignore == true`；`report.gitignore.respected`；`conflicts` 含该文件。
  - `respect_init_blocks_a_negation_that_admits_unselected_files`：`!*.txt` 形态，报 `svn_gitignore_includes`，`.git` 已删。
  - `respect_init_then_open_health_and_rebuild_stay_green`：建库后 `check_current_baseline` 为 `Verified`，`svn_rebuild_baseline` 成功。
  - `the_new_phase_shows_up_in_timings`：成功报告的 `timings` 含「检查起作用的 .gitignore（git check-ignore）」。

- [ ] **Step 2: 跑测试确认失败**：`cargo test --test svn_init`

- [ ] **Step 3: 实现**
  - 闸门失败走 `build_repo` 的 `Err`，由现有回退删掉 `.git`。
  - 建库成功时 `report.gitignore = GitignoreOutcome { respected, conflicts, disabled: [] }`。

- [ ] **Step 4: 跑测试确认通过**：先 `cargo test --test svn_init`，再全量 Rust 三件套。

- [ ] **Step 5: 更新决策日志**

---

### Task 7: 「不尊重」路径——改名停用、改名后重查、回退复原

**Files:**
- Modify: `src-tauri/src/engine/svn_gitignore.rs`：改名与恢复
- Modify: `src-tauri/src/engine/svn_init.rs`：不尊重分支、回退时恢复改名、配置记录
- Test: `src-tauri/tests/svn_gitignore.rs`（改名单元），`src-tauri/tests/svn_init.rs`（编排）

**Interfaces:**
- Consumes：Task 6 的 `build_repo` 阶段结构与 `GitignoreOutcome`
- Produces：
  - `pub(crate) fn disable_gitignore(repo_root: &str, rel: &str) -> Result<String>`：返回新的相对路径
  - `pub(crate) fn restore_gitignores(repo_root: &str, renamed: &[DisabledGitignore]) -> Vec<String>`：返回恢复失败的说明，空表示全部恢复
  - 错误码 `svn_gitignore_versioned`

改名规则：

```text
候选名：同目录下 .gitignore.jkk-disabled，其后 .gitignore.jkk-disabled-2、-3 … 到 -99
占名：OpenOptions::new().write(true).create_new(true) 原子占住候选名（已存在则试下一个）
改名：fs::rename(原文件, 占位名)。Windows 上 rename 会替换已存在的目标，替换的只是自己的占位
      ——这正是先占名的理由；rename 失败时删掉占位再返回 Err
恢复：原位置已有文件时**不覆盖**，记一条失败「{原路径} 又出现了一个新文件，{新路径} 没有改回去」
```

`build_repo` 里的不尊重分支（紧跟在 Task 6 闸门 1 之后，放进新阶段「停用 .gitignore（改名）」）：

```text
有受版本控制的冲突 → svn_gitignore_versioned（不改任何文件）
逐个 disable_gitignore，结果记进 init_phases 持有的 renamed 列表（回退要用）
重查：detect_conflicts(GitCtx::Repo) 非空 → svn_gitignore_unconfirmed（列出新冒出的），走回退
cfg.disabled_gitignores = renamed；之后 write_config
```

回退（`init_phases` 的 `Err` 分支，「回退」阶段内）：先 `restore_gitignores`，再 `rollback`。恢复失败时，在原错误正文后追加「\n\n下面这些 .gitignore 没能改回原名，请手工处理：\n{每条}」。

`svn_gitignore_versioned` 文案：「{full_path} 受 SVN 版本控制，是团队提交的规则，本应用不会改它。请勾选「尊重已有的 .gitignore 规则」，或先和团队确认。什么都没改。」

- [ ] **Step 1: 写失败测试**
  - `tests/svn_gitignore.rs`：
    - `disabling_renames_to_the_jkk_disabled_name`
    - `disabling_never_overwrites_an_existing_disabled_file`（Review Focus 第 4 条）：`-disabled` 与 `-2` 已存在，结果是 `-3`，两份旧文件内容不变。
    - `restore_never_overwrites_a_gitignore_that_reappeared`（Review Focus 第 5 条）：恢复前在原位置写一个新 `.gitignore`。返回一条失败，原位置内容仍是新写的，停用文件仍在。
  - `tests/svn_init.rs`：
    - `default_init_disables_an_acknowledged_unversioned_gitignore`：根 `.gitignore` 写 `*.cs`，acknowledged。
      - 成功，基线含 `Code/A.cs`；
      - 磁盘上 `.gitignore.jkk-disabled` 存在、`.gitignore` 不存在；
      - 配置 `disabledGitignores == [{from:".gitignore", to:".gitignore.jkk-disabled"}]`，`version == 1`；
      - `report.gitignore.disabled[0].from` 是完整路径。
    - `default_init_disables_a_gitignore_whose_negation_admits_unselected_files`：
    - 扩展名 `.cs`，未版本控制的 `Code/.gitignore` 写 `!*.txt`，磁盘上有 `Code/n.txt`，acknowledged。
    - 成功，改名后基线里没有 `Code/n.txt`。这是 spec §7「`!` 多收在两种模式下」的不尊重那一半。
  - `default_init_refuses_to_rename_a_versioned_gitignore`：`.gitignore` 已 `svn add` 并提交。报 `svn_gitignore_versioned`，`.gitignore` 原样存在，`.git` 已删。
    - `a_failed_init_puts_the_renamed_gitignore_back`：先改名，再在后续阶段失败（复用 `a_failure_after_git_init_rolls_the_whole_thing_back` 的失败触发手法）。`.gitignore` 回到原名，`.git` 已删。
    - `a_gitignore_that_becomes_active_after_renaming_sends_the_user_back`：
      - `Code/.gitignore` 写 `Sub/`；`Code/Sub/.gitignore` 写 `*.cs`；`Code/Sub/X.cs` 已提交；acknowledged 只含 `Code/.gitignore`。
      - 报 `svn_gitignore_unconfirmed`，消息含 `Code\Sub\.gitignore` 的完整路径；
      - `Code/.gitignore` 已改回原名，`.git` 已删。

- [ ] **Step 2: 跑测试确认失败**：`cargo test --test svn_gitignore --test svn_init`

- [ ] **Step 3: 实现**

- [ ] **Step 4: 跑测试确认通过**：全量 Rust 三件套。

- [ ] **Step 5: 更新决策日志**（改名、恢复、占名的安全论证；Windows 上 `fs::rename` 会替换目标的实测结论）

---

### Task 8: IPC 与向导界面

**Files:**
- Modify: `src-tauri/src/commands/svn.rs`
  - `svn_preflight_check(path: String, whitelist: Vec<String>, extensions: Vec<String>) -> Result<InitPreflight, GitError>`
  - `svn_init_hybrid(path, whitelist, extensions, respect_gitignore: bool, acknowledged: Vec<String>)`
- Modify: `src-tauri/tests/svn_backend.rs`：序列化契约
- Modify: `src/store/repoStore.ts`：`svnPreflightCheck(path, whitelist, extensions)`；`svnInitHybrid(path, whitelist, extensions, respectGitignore, acknowledged)`，invoke 键名为 `respectGitignore`、`acknowledged`
- Modify: `src/engine/svnInit.ts`、`src/components/SvnInitWizard.tsx`、`src/components/RepoTab.tsx:634-635`、`src/styles.css`（如需要）
- Test: `tests/vitest/svnInit.test.ts`；新建 `tests/vitest/SvnInitWizard.test.tsx`（文件头 `// @vitest-environment jsdom`）

**Interfaces:**
- Consumes：Task 5 的 `InitPreflight` / `GitignoreConflict`，Task 6 的 `GitignoreOutcome`，Task 6/7 的三个错误码
- Produces（`svnInit.ts`）：
  - 类型：`GitignoreRule`、`GitignoreConflict`、`RenamedGitignore`、`GitignoreOutcome`、`InitPreflight extends Preflight { gitignoreConflicts: GitignoreConflict[] }`；`InitReport` 加 `gitignore`
  - `describeGitignoreConflicts(conflicts: GitignoreConflict[]): Notice | null`
  - `gitignoreBlockReason(conflicts: GitignoreConflict[], respect: boolean): string | null`
  - `describeGitignoreOutcome(o: GitignoreOutcome): Notice | null`
  - `needsReconfirm(code: string): boolean`：仅 `svn_gitignore_unconfirmed` 为 true

界面文案（原样使用）：
- 冲突提示块：
  - 标题「这些 .gitignore 会影响交给 git 管的文件」，tone `warn`。
  - 每个文件一段：首行完整路径，受版本控制时追加「（受 SVN 版本控制）」。
  - 每条规则一行：
    - 「第 {line} 行 `{pattern}`：挡掉 {count} 个 SVN 里有的文件，例如 {samples 用、连接}」
    - 「第 {line} 行 `{pattern}`：多收 {count} 个你没选的文件，例如 …」
- 勾选框：
  - label「尊重已有的 .gitignore 规则」，默认不勾；
  - 说明「不勾：上面这些 .gitignore 会改名为 .gitignore.jkk-disabled 停用，文件照样交给 git 管。勾上：被它们挡掉的文件不交给 git 管，以后的更新、交付也按这个口径。」
- `gitignoreBlockReason`：不勾且有受版本控制的、或勾上且有 Includes 时，返回与后端同义的一句话（见 Task 6/7 文案）。此时「建库」按钮禁用，理由显示在按钮上方。
- 完成页：
  - 改过名：标题「停用了 {n} 个 .gitignore」，每行「{from} → {to}」，末行「想恢复就把它改回原名。」
  - 尊重：标题「按 .gitignore 的规则，这些文件没有交给 git 管」，按文件与规则列数量和样例。
- `wizardErrorHint` 新增三条：
  - `svn_gitignore_unconfirmed`：「清单已经刷新，请看过之后重新点「建库」。」
  - `svn_gitignore_versioned`：「受版本控制的 .gitignore 是团队规则。要按它来就勾选「尊重」；要改它请先和团队商量，用 TortoiseSVN 提交。」
  - `svn_gitignore_includes`：「「尊重」只接受 .gitignore 少收文件。取消勾选会把这个 .gitignore 改名停用。」

向导行为：
- `confirm` 步在按钮上方渲染冲突块和勾选框。**没有冲突时两者都不渲染**，`respect` 传 false。
- `onInit` 传 `respect` 与 `preflight.gitignoreConflicts.map(c => c.path)`。
- 错误码满足 `needsReconfirm` 时：重新调 `onPreflight`，留在 `confirm` 步显示新清单。错误块照常显示。勾选状态保持不变。

- [ ] **Step 1: 写失败测试**
  - `tests/vitest/svnInit.test.ts`：
    - `describeGitignoreConflicts`：空数组返回 null；一条 Excludes 规则的 detail 含完整路径与「第 1 行 `*.exe`：挡掉 2 个」；受版本控制的带「（受 SVN 版本控制）」。
    - `gitignoreBlockReason` 的四格真值表：受版本控制 × 勾选，Includes × 勾选。
    - `describeGitignoreOutcome`：既没尊重也没改名返回 null；改名一条时含「→」与「想恢复」。
    - `needsReconfirm`。
    - 三条新 hint 存在。
  - `tests/vitest/SvnInitWizard.test.tsx`：
    - `hides_the_checkbox_when_nothing_conflicts`
    - `shows_the_checkbox_unchecked_and_passes_acknowledged_paths`
    - `disables_build_for_a_versioned_gitignore_until_respect_is_checked`
    - `refreshes_the_list_after_an_unconfirmed_error`：`onInit` 第一次抛 `svn_gitignore_unconfirmed`，`onPreflight` 被再调一次，新清单出现。
  - `src-tauri/tests/svn_backend.rs`：
    - `InitPreflight` 键为 `["gitignoreConflicts","nodeRev","reposRoot","uuid"]`；
    - `InitReport` 键为 `["baseline","gitignore","health","nodeRev","pinned","timings"]`；
    - `GitignoreConflict` 键为 `["fullPath","path","rules","versioned"]`；
    - `GitignoreRule` 键为 `["count","effect","line","pattern","samples"]`；
    - `effect` 序列化为 `"excludes"` / `"includes"`；
    - `GitignoreOutcome` 键为 `["conflicts","disabled","respected"]`。

- [ ] **Step 2: 跑测试确认失败**：`pnpm test`；`cd src-tauri; cargo test --test svn_backend`

- [ ] **Step 3: 实现**

- [ ] **Step 4: 跑测试确认通过**：`pnpm build`、`pnpm test`，以及 Rust 三件套。

- [ ] **Step 5: 更新决策日志**

---

### Task 9: 真实窗口验证与文档收尾

**Files:**
- Modify: `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`：§6.3（W-file 定义加「尊重」口径与参照基线）、§6.5（序列加检测阶段与改名阶段）
- Modify: 决策日志（「真实窗口验证」一节）

- [ ] **Step 1: 建一次性 SVN fixture**：放在 scratchpad，不碰任何真实仓库。要有三种形态：
  1. 未版本控制的 `.gitignore` 挡住已提交的 `.exe`；
  2. 受版本控制的 `.gitignore`；
  3. 嵌套的 `Sub/` 加内层 `.gitignore`。
- [ ] **Step 2: 备份 `%APPDATA%\com.momadacoding.jkk\session.json`**，然后启动 `pnpm tauri dev`，带 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 和独立的 `WEBVIEW2_USER_DATA_FOLDER`（PR #90 的 CDP 手法）。
- [ ] **Step 3: 逐项走一遍并截图**，预期结果：
  - 第 3 步显示冲突块（完整路径、第 N 行、样例）与勾选框；
  - 不勾时建库成功，完成页列出改名；
  - 受版本控制的那份：按钮禁用、勾上后可建；
  - 嵌套形态：退回第 3 步并出现新清单；
  - 没有冲突的工程：不出现勾选框。
  - 每项记录 DOM 证据：元素文本，以及 `disabled` 状态。
- [ ] **Step 4: 收尾**：恢复 `session.json` 并比对哈希，删掉 fixture，更新 spec 与决策日志。
- [ ] **Step 5: 最终验证**：`pnpm build`、`pnpm test`、`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test` 全部重跑，结果写进决策日志。
