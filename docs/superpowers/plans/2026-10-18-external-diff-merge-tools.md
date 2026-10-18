# 外部 diff / merge 工具与冲突文件显示 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 冲突文件和普通文件都能一键交给外部 diff / merge 工具（默认 Beyond Compare，可按文件名规则选 UnityYAMLMerge 等）。冲突文件右侧按五种情况如实显示。DU / UD 可以在应用内二选一。缺少 merge driver 配置时发出警告，并在发起合并之前拦一下。

**Architecture:**
- Rust 侧负责：读取冲突信息并分类（libgit2，加上 `git check-attr` / `git config` / `git cat-file --filters` 三个 CLI 读取例外）；把各个版本导出成临时文件；启动外部进程并在它退出后收尾；merge driver 检查。
- 前端负责：按规则挑工具、生成菜单、把冲突信息映射成界面文案，以及在各个合并入口之前确认。
- 外部工具的配置放在 `externalTools.json`，沿用 `customCommands.json` 的机制。

**Tech Stack:** Tauri 2、Rust（git2 0.20、serde、tempfile 测试）、React 18 + TypeScript、Vitest + React Testing Library。

**Spec:** `docs/superpowers/specs/2026-10-17-external-diff-merge-tools-design.md`（下文写「spec §x」）。执行时 spec 和本计划要一起读。

## Global Constraints

- 主干 `master`。开工前用 `EnterWorktree` 切进独立 worktree（AGENTS.md §4.1），分支名 `worktree-external-tools`。
- 动第一个 Task 之前，先建 `docs/superpowers/plans/2026-10-18-external-diff-merge-tools-decisions.md`（AGENTS.md §2.1）。每完成一个 Task，就更新进度总账。
- 所有涉及 git 写操作的验证只在临时 fixture 仓库里做（AGENTS.md 最高优先级），参照 `src-tauri/tests/fixtures.rs` 的 `run_git` / `TempDir` 写法。
- 不新增依赖（npm 和 crate 都不加）。唯一 ID 用「进程号 + 纳秒时间戳 + `AtomicU64` 计数」拼出来。
- 读取走 libgit2。只有三处例外走 CLI：`git cat-file --filters`（必须用 `run_git_bytes`）、`git check-attr`、`git config`（必须用 `run_git_code`，退出码 1 表示零匹配）。写入只有 `git --literal-pathspecs add|rm -- <path>`，走 `run_git`。
- 外部进程不走 `run_git`。用 `launcher::resolve_program` 加 `proc::hidden_command`，参数作为数组传入，禁止自己拼 `cmd /c`。
- 新增的 `#[tauri::command]` 一律写成 `#[tauri::command(async)]`（`tests/commands_are_async.rs` 会检查）。错误统一用 `GitError::new(code, msg)`。JS 侧参数用 camelCase。
- 任何时候都不自动暂存冲突文件（③ 的二选一除外）。临时文件清理只能删应用临时根目录下本次创建的子目录，`$MERGED` 和工作区里的文件永远不删。
- 界面文案严格按 spec 原文，下文各 Task 中引号里的文字照抄。
- 注释用中文，解释为什么这么做。TS 用单引号、尾随逗号，跨模块导入用 `@/`。
- 每个 Task 的验证命令：
  - Rust：在 `src-tauri/` 下跑 `cargo fmt --check && cargo clippy --all-targets && cargo test`
  - 前端：`pnpm build && pnpm test`
  - 改了命令签名的 Task，最后要在 `pnpm tauri dev` 里实测 invoke 能通。
- Commit message 用 `<type>(<scope>): <summary>` 加 What / Why / Impact，scope 用 `external-tool` / `conflict` / `merge-driver`。正文里的编号写「第 N 条」，不写裸的 `#N`。

## Review Focus

1. **路径里有中文、空格、`[]`、开头是 `-`**：临时文件名、argv、`--path=`、literal pathspec 都要原样传过去，不能被截断、转义或当成模式匹配。用例在 Task 2、3、9。
2. **仓库是 linked worktree**：`check-attr`、`config`、index、`$MERGED` 都必须在那个 worktree 里解析。用例在 Task 3、5、10。
3. **外部工具还开着的时候又触发了刷新或操作**：锁要拦住重复打开；晚到的刷新结果不能覆盖新状态。用例在 Task 2、4。
4. **SVN 混合仓库的同步停在冲突上**（同步锁已经放开、journal 还在）：外部 merge 和二选一都要能用；同步锁被持有时，二选一必须被拒绝。用例在 Task 9。
5. **大仓库的 driver 检查**：只能调用一次批量的 `check-attr --stdin`，并且要有缓存，不能每次保存文件都把全量扫一遍。用例在 Task 10；耗时在 Task 12 的真实窗口里记录。

---

## 阶段 1：外部工具基础

### Task 0: 建 worktree 和决策日志

**Files:**
- Create: `docs/superpowers/plans/2026-10-18-external-diff-merge-tools-decisions.md`

- [ ] **Step 1:** 在 `master` 上 `git pull`，然后 `EnterWorktree`（name `external-tools`）。用 `pwd` 确认路径在 `.claude/worktrees/external-tools`，`git log --oneline -3` 的第一行应是 `59c2398` 或更新的提交。
- [ ] **Step 2:** 建决策日志，只放两部分：
  - 进度总账表：列为 Task、状态、产出文件、验证结果，Task 0–12 各一行。
  - 决策条目：按 Task 分节，每节下面写四个小标题：偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的。
- [ ] **Step 3:** Commit `docs(external-tool): 实现日志骨架`。

### Task 1: 外部工具配置与规则匹配

**Files:**
- Create: `src-tauri/src/engine/external_tools_config.rs`, `src-tauri/src/commands/external.rs`, `src/engine/externalTools.ts`, `src/store/externalToolsStore.ts`
- Modify: `src-tauri/src/engine/mod.rs`, `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`（注册命令）
- Test: `src-tauri/tests/external_tools_config.rs`, `tests/vitest/externalTools.test.ts`

**Interfaces:**
- Produces（Rust，全部 `#[serde(rename_all = "camelCase")]`）：
  - `ExternalTool { id: String, label: String, program: String, diff_args: Option<Vec<String>>, merge_args: Option<Vec<String>> }`
  - `ToolRule { patterns: Vec<String>, diff: Option<String>, merge: Option<String> }`
  - `ExternalToolsConfig { version: u32, default_diff: Option<String>, default_merge: Option<String>, rules: Vec<ToolRule>, tools: Vec<ExternalTool> }`
  - `pub fn default_config() -> ExternalToolsConfig`
  - `pub fn load_from(path: &Path) -> Result<ExternalToolsConfig>`：文件缺失时写入默认配置再返回；JSON 坏了或校验失败，报 `GitError` code `external_tools_config`，消息里带上路径，并且不改动文件。
  - `pub fn find_tool<'a>(cfg: &'a ExternalToolsConfig, id: &str) -> Result<&'a ExternalTool>`
- Produces（命令）：`read_external_tools(app) -> ExternalToolsConfig`、`open_external_tools_config(app) -> ()`，配置文件路径是 `app_config_dir()/externalTools.json`。
- Produces（TS）：类型与 Rust 一一对应，另外：
  - `matchesPattern(pattern: string, fileName: string): boolean`
  - `toolFor(cfg, filePath, kind: 'diff' | 'merge'): ExternalTool | null`
  - `toolMenu(cfg, filePath, kind): { primary: ExternalTool | null; others: ExternalTool[] }`
  - store：`useExternalTools(): { config, error, reload, openConfig }`

- [ ] **Step 1: Rust 测试**（`external_tools_config.rs`）
  - `missing_file_writes_defaults_with_bc_and_vscode`：ids 是 `["bc", "vscode"]`，`default_diff` 和 `default_merge` 都是 `Some("bc")`，`rules` 为空；bc 的 `program` 等于 `C:\Program Files\Beyond Compare 5\BComp.exe`；vscode 的 `merge_args` 等于 `["--wait", "--merge", "$REMOTE", "$LOCAL", "$BASE", "$MERGED"]`；文件写到了磁盘上，再读一次结果相同。
  - `broken_json_errors_and_leaves_file_untouched`：code 为 `external_tools_config`，文件字节没有变化。
  - `rule_referencing_unknown_tool_is_rejected`：规则里 `merge: "nope"` 时报错，消息里包含 `nope`。
  - `default_referencing_unknown_tool_is_rejected`
  - `duplicate_tool_ids_are_rejected`
- [ ] **Step 2:** 跑 `cargo test --test external_tools_config`，预期因为模块不存在而失败。
- [ ] **Step 3:** 实现 `external_tools_config.rs` 和两个命令。参照 `custom_config.rs` 和 `commands/custom.rs` 的结构；`open_external_tools_config` 同样不接受路径参数。bc 的 diff / merge 参数照抄 spec §4.4 的 JSON。
- [ ] **Step 4: vitest**（`externalTools.test.ts`）
  - `matchesPattern`：
    - `('*.unity', 'Main.UNITY') === true`（大小写不敏感）
    - `('*.unity', 'Main.unity.meta') === false`
    - `('a?c.txt', 'abc.txt') === true`
    - `('*.cs', 'dir/x.cs')`：调用方只传文件名，这里断言 `toolFor` 用的是 `Assets/x.cs` 的文件名部分（反斜杠路径 `Assets\\x.cs` 同样只取文件名）
  - `toolFor`：
    - 第一条命中的规则生效；
    - 规则只配了 `merge` 时，kind 为 `'diff'` 回落到 `defaultDiff`；
    - 工具没有 `mergeArgs` 时，kind 为 `'merge'` 返回 null。
  - `toolMenu`：`others` 不包含 `primary`，也不包含缺对应参数的工具。
- [ ] **Step 5:** 实现 `externalTools.ts`（只支持 `*` 和 `?`，自己写一个小匹配器）和 `externalToolsStore.ts`（参照 `customCommandsStore.ts`）。
- [ ] **Step 6:** 跑 Rust 和前端两侧的全部验证命令，预期都通过。
- [ ] **Step 7:** Commit `feat(external-tool): 外部工具配置与按文件名规则选工具`。

### Task 2: 启动外部进程、锁与临时目录

**Files:**
- Create: `src-tauri/src/engine/external_launch.rs`
- Test: `src-tauri/tests/external_launch.rs`

**Interfaces:**
- Produces：
  - `pub fn expand_template(args: &[String], vars: &[(&str, String)]) -> Vec<String>`：对每个元素单趟扫描，按最长名字优先匹配，替换进去的值不会再被展开。占位符包括 `$LOCAL $REMOTE $BASE $MERGED $LOCAL_TITLE $REMOTE_TITLE $BASE_TITLE $MERGED_TITLE`；没列出的名字原样保留。
  - `pub fn temp_root() -> PathBuf`：返回 `std::env::temp_dir().join("jkk-external")`。
  - `pub struct TempWorkspace`，方法有 `create() -> Result<Self>`（在 `temp_root()` 下用 `create_dir` 建一个唯一子目录）、`file(&self, name: &str) -> PathBuf`、`remove(self) -> Result<()>`。`remove` 先确认目录在 `temp_root()` 里面，否则拒绝删除，返回 code `external_tool_cleanup`。
  - `pub struct ToolLockGuard`，Drop 时释放锁；`pub fn try_lock(worktree: &str, file: &str) -> Result<ToolLockGuard>`：同一个 worktree 加同一个 file 已被占用时，报 code `external_tool_busy`。key 用 `engine::path` 里已有的规范化函数，大小写不敏感。
  - `pub struct ExitReport { pub code: Option<i32>, pub error: Option<String> }`
  - `pub fn spawn_watched(program: &str, args: &[String], on_exit: impl FnOnce(ExitReport) + Send + 'static) -> Result<()>`：启动失败（找不到程序等）同步返回 `GitError` code `launcher`；启动成功后在后台线程里 `wait`，进程结束后调用 `on_exit`。

- [ ] **Step 1: 测试**
  - `expand_template_replaces_inside_elements`：`["/lefttitle=$LOCAL_TITLE", "$LOCAL"]` 展开成 `["/lefttitle=main", "C:\\t\\P.LOCAL.cs"]`。
  - `expanded_values_are_not_reexpanded`：`$LOCAL` 的值是 `"$REMOTE"` 时，结果保持字面量 `$REMOTE`。
  - `unknown_placeholder_is_kept`：`$FOO` 原样保留。
  - `workspace_remove_refuses_outside_root`
  - `lock_rejects_second_holder_and_releases_on_drop`
  - `spawn_watched_delivers_argv_verbatim`（只在 Windows 上编译）：写一个 `.cmd` 假程序，用 `echo %*` 把参数写进文件；再用一个 `.exe` 假程序（测试里 `cargo` 编出来的 helper 不可用时，改用 `cmd.exe /c` 写入，并在注释里说明）。断言参数 `a b`、`中文`、`x&y`、`50%`、`!bang!`、`"q"` 都作为单个 argv 送到。
  - `spawn_watched_reports_nonzero_exit`：假程序 `exit /b 7`，`on_exit` 收到 `code == Some(7)`。
  - `spawn_missing_program_errors_synchronously`
- [ ] **Step 2:** 跑测试，预期失败。
- [ ] **Step 3:** 实现。`spawn_watched` 复用 `launcher::resolve_program` 和 `proc::hidden_command`，标准流都接到 null。
- [ ] **Step 4:** 跑 Rust 全部验证，预期通过。Windows argv 测试里某种字符送不到的话，**不要改断言**，把实测结果记进决策日志，然后停下来问用户。
- [ ] **Step 5:** Commit `feat(external-tool): 外部进程启动、锁与临时目录`。

### Task 3: 按字节导出与外部 diff 命令

**Files:**
- Create: `src-tauri/src/engine/blob_export.rs`, `src-tauri/src/engine/external_diff.rs`
- Modify: `src-tauri/src/commands/external.rs`, `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/external_diff.rs`

**Interfaces:**
- Consumes：Task 1 的 `find_tool`；Task 2 的全部接口。
- Produces：
  - `pub fn export_filtered(repo: &str, rel_path: &str, oid: &str, dest: &Path) -> Result<()>`：执行 `run_git_bytes(repo, ["cat-file", "--filters", "--path=<rel_path>", oid])`，原样写入 `dest`。`rel_path` 为空时报 code `external_tool_export`；git 失败直接返回错误，不写空文件。
  - `pub fn side_file_name(rel_path: &str, side: &str) -> String`：`Code/P.cs` 加 `LOCAL` 得到 `P.LOCAL.cs`；没有扩展名的 `Makefile` 得到 `Makefile.LOCAL`。
  - `pub fn ensure_regular_file(abs: &Path) -> Result<()>`：用 `symlink_metadata` 检查，符号链接或目录都报 code `external_tool_not_regular`。
  - `#[serde(tag = "kind", rename_all = "camelCase")] pub enum DiffTarget { Unstaged { file }, Staged { file }, Commit { oid, file, old_file: Option<String> }, Range { from, to, file, old_file: Option<String> } }`
  - `pub fn prepare_diff(repo: &str, target: &DiffTarget, ws: &TempWorkspace) -> Result<(PathBuf, PathBuf, String, String)>`：返回左路径、右路径、左标题、右标题。两侧的取法按 spec §4.3 的表；Unstaged 的右侧直接用工作区文件（先过 `ensure_regular_file`）；某一侧不存在时写空文件。
  - 命令：`open_external_diff(app, path: String, tool_id: String, target: DiffTarget) -> Result<()>`。流程是：锁 → 建 `TempWorkspace` → `prepare_diff` → 展开 `diff_args` → `spawn_watched`。进程退出时删掉 workspace、释放锁，然后 `emit("external-tool-exited", ExternalToolExited { worktree, file, tool_label, code, error })`。工具没有 `diff_args` 时报 code `external_tool_unsupported`。`spawn_watched` 返回的 `launcher` 错误要改写消息，写明 `externalTools.json` 的完整路径；`launcher::not_found` 的原文写的是「请检查 customCommands.json」，原样透传会把用户指错地方。

- [ ] **Step 1: fixture 测试**（`TempDir` 加 `run_git`）
  - `unstaged_exports_index_vs_workdir`：左边是 index 内容，右边是工作区文件的绝对路径。
  - `staged_exports_head_vs_index`
  - `commit_with_rename_uses_old_path_on_left`
  - `root_commit_left_is_empty_file`
  - `added_file_left_is_empty_file`
  - `export_applies_autocrlf`：fixture 设 `core.autocrlf=true`，导出的文件里包含 `\r\n`。
  - `export_keeps_utf16_bytes`：提交一个带 BOM 的 UTF-16LE 文件，并设 `-text`；导出结果与原文件逐字节相同。
  - `export_failure_does_not_write_file`：传一个不存在的 oid，结果是 `Err`，`dest` 不存在。
  - `symlink_workdir_side_is_refused`：没有符号链接权限时跳过本测试，并打印原因（参照 `fixtures.rs` 的 `make_dangling_link`）。
  - `linked_worktree_unstaged_uses_that_worktree`
  - `chinese_space_bracket_path_roundtrip`：文件名 `目录 a/f[1] x.txt`。
- [ ] **Step 2:** 跑测试，预期失败。
- [ ] **Step 3:** 实现两个模块和命令，并在 `lib.rs` 注册。
- [ ] **Step 4:** 跑 Rust 全部验证，预期通过。
- [ ] **Step 5:** Commit `feat(external-tool): 按字节导出与外部 diff`。

### Task 4: 前端外部 diff 入口与刷新代次

**Files:**
- Create: `src/engine/latestGate.ts`
- Modify: `src/store/externalToolsStore.ts`, `src/store/repoStore.ts`（`refreshStatus` / `refreshDiff` 加代次校验），`src/components/LocalChangesView.tsx`, `src/components/FileTree.tsx`（新增可选的 `onContextMenu?: (path: string, e: React.MouseEvent) => void`），`src/components/CommitDetailView.tsx`（新增 `diffScope: string | null` prop 和右键菜单），`src/components/RepoTab.tsx`
- Test: `tests/vitest/latestGate.test.ts`, `tests/vitest/LocalChangesView.test.tsx`, `tests/vitest/CommitDetailView.test.tsx`

**Interfaces:**
- Consumes：命令 `open_external_diff`（参数 `{ path, toolId, target }`），事件 `external-tool-exited`（payload `{ worktree, file, toolLabel, code, error }`）。
- Produces：
  - `createLatestGate(): { begin(key?: string): number; isLatest(token: number, key?: string): boolean }`：key 不同时各自独立计数。
  - store：`openExternalDiff(path, toolId, target): Promise<void>`，以及 `useExternalToolExit(path, onExit)`。`onExit` 只在 `samePath(payload.worktree, path)` 时调用。

- [ ] **Step 1: vitest**
  - `latestGate`：先后 begin 两次，第一个 token 的 `isLatest` 为 false；不同 key 之间互不影响。
  - `LocalChangesView`：
    - 右键一个 Unstaged 文件，菜单里有 `在 Beyond Compare 中对比`；点它之后，回调收到 `{ kind: 'unstaged', file }`。
    - 配了两个工具时，菜单里有 `用其他工具打开 ▸`，下面列出 `VS Code`。
    - 冲突文件的右键菜单里**没有**「对比」。
    - 菜单最后一项是 `编辑外部工具配置…`；配置读取失败时，菜单里只剩这一项。
  - `CommitDetailView`：
    - `diffScope = 'commit:abc'` 时，右键文件得到 `{ kind: 'commit', oid: 'abc', file, oldFile }`；
    - `diffScope = 'range:a..b'` 时得到 `{ kind: 'range', from: 'a', to: 'b', ... }`；
    - `diffScope` 为 null 时没有这一项。
- [ ] **Step 2:** 跑 `pnpm test`，预期失败。
- [ ] **Step 3:** 实现。
  - 菜单项文案为 `在 ${tool.label} 中对比`；子菜单标题为 `用其他工具打开 ▸`。`ContextMenu` 不支持子菜单的话，就平铺成 `用 ${label} 打开`，并记进决策日志。
  - 外部工具退出后刷新 status 和 diff。`error` 或非零 `code` 通过 `reportError` 显示，消息里带上 `toolLabel` 和退出码，不能吞掉。
  - `RepoTab` 把 `selectionDiffScope` 的结果传给 `CommitDetailView`。
  - 每个外部工具菜单的最后一项都是 `编辑外部工具配置…`，调用 `openConfig`（spec §4.5）。配置读取失败时，菜单里只保留这一项，并通过 `reportError` 显示错误。
- [ ] **Step 4:** 跑前端全部验证，预期通过。
- [ ] **Step 5:** 在 `pnpm tauri dev` 里实测：普通文件右键用 BC 打开，关掉 BC 后，`%TEMP%\jkk-external` 下没有残留目录。结果记进决策日志。
- [ ] **Step 6:** Commit `feat(external-tool): 本地改动与提交详情的外部 diff 入口`。

---

## 阶段 2：冲突显示与外部 merge

### Task 5: 冲突信息读取与五种情况分类

**Files:**
- Create: `src-tauri/src/engine/conflicts.rs`, `src-tauri/src/commands/conflict.rs`
- Modify: `src-tauri/src/engine/mod.rs`, `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/conflicts.rs`

**Interfaces:**
- Produces（serde camelCase）：
  - `StageInfo { oid: String, mode: u32 }`
  - `enum ConflictCase { Structural, DeleteModify, NotTextMerged, Markers, NoMarkers }`
  - `enum NotTextReason { BinaryAttr, CustomDriver, MergeDefault, Lfs, BinaryContent }`
  - `ConflictInfo { path, code: String /* "UU" | "AA" | "DU" | "UD" | "AU" | "UA" | "DD" */, base: Option<StageInfo>, ours: Option<StageInfo>, theirs: Option<StageInfo>, case: ConflictCase, reason: Option<NotTextReason>, driver: Option<String>, same_as_head: bool }`
  - `pub fn scan_markers(bytes: &[u8], marker_size: usize) -> bool`
  - `pub fn path_attrs(repo: &str, paths: &[&str]) -> Result<HashMap<String, PathAttrs>>`，其中 `PathAttrs { merge: String, filter: String, marker_size: usize }`，通过 `git check-attr -z merge filter conflict-marker-size -- <paths>` 获取。本函数也给 Task 10 用。
  - `pub fn merge_config(repo: &str) -> Result<MergeConfig>`，其中 `MergeConfig { default: Option<String>, drivers: HashMap<String, DriverConfig> }`，`DriverConfig { driver: Option<String>, has_other_keys: bool }`。通过 `run_git_code(["config", "-z", "--get-regexp", r"^merge\."])` 获取：退出码 1 视为空，重复的键取最后一个值。本函数也给 Task 10 用。
  - `pub fn read_conflicts(repo: &str) -> Result<Vec<ConflictInfo>>`，判定顺序按 spec §4.1 的表（⑤ → ③ → ④ → ① → ②）。
  - code 由 stage 的组合决定：
    - {1,2,3} → UU，{2,3} → AA
    - {1,2} → UD，{1,3} → DU
    - {2} → AU，{3} → UA，{1} → DD
  - mode 只要有一个不是 `0o100644` / `0o100755`，就是 Structural。
  - 命令：`read_conflicts(path) -> Vec<ConflictInfo>`

- [ ] **Step 1: 纯函数测试**
  - `scan_markers`：
    - merge 风格 → true；diff3 风格 → true；
    - 只剩 `<<<<<<< HEAD` 一行 → true；只剩 `>>>>>>> x` → true；
    - 孤立的 `=======`（Markdown 下划线）→ false；孤立的 `|||||||` → false；
    - `marker_size = 32` 时，32 个 `<` 加空格 → true，7 个 `<` → false；
    - 9 个 `<` 加空格，`marker_size = 7` → true（「至少」）；
    - CRLF 行尾也能识别。
- [ ] **Step 2: fixture 测试**（复刻 spec §4.1 表 1，每一行一个测试，fixture 里写 `.gitattributes` 和本地 `git config`）
  - `no_attr_conflict_is_markers`
  - `binary_attr_is_not_text_merged`，reason 为 BinaryAttr
  - `unset_merge_attr_is_not_text_merged`
  - `undefined_custom_driver_falls_back_to_markers`
  - `failing_custom_driver_is_not_text_merged`，reason 为 CustomDriver，driver 为 `Some("faildriver")`；driver 命令是 `sh` 下的 `echo driver-left-this > %A; exit 1`
  - `missing_driver_program_is_not_text_merged`
  - `merge_default_custom_is_not_text_merged`
  - `binary_base_only_is_not_text_merged`（BinaryContent）
  - `cleared_markers_is_no_markers`：工作区写成「两边都没有」的内容
  - `delete_modify_codes`：DU / UD 各一个，case 为 DeleteModify
  - `add_add_conflict_uses_aa`
  - `structural_codes`：AU / UA / DD 用真实 git 造出来（例如 rename/rename、rename/delete），把 `ls-files -u` 的实际形状记进决策日志；造不出来的组合，在测试注释里写明原因
  - `symlink_conflict_is_structural`（没有权限时跳过）
  - `name_only_driver_aborts_merge`：merge 退出码 128，`MERGE_HEAD` 不存在，`read_conflicts` 返回空
  - `same_as_head_flag`：`merge=binary` 时为 true
  - `conflicts_in_linked_worktree`
- [ ] **Step 3:** 跑测试，预期失败。
- [ ] **Step 4:** 实现，在 `lib.rs` 注册命令。
- [ ] **Step 5:** 跑 Rust 全部验证，预期通过。
- [ ] **Step 6:** Commit `feat(conflict): 冲突信息读取与五种情况分类`。

### Task 6: 冲突文件的 HEAD 对比工作区 diff

**Files:**
- Modify: `src-tauri/src/engine/diff.rs`（`read_diff` 默认分支）
- Test: `src-tauri/tests/fixtures.rs`（新增测试）

**Interfaces:**
- Produces：`read_diff(path, "workdir")` 对状态为 `conflicted` 的文件也返回 hunk，内容是 HEAD 的 blob 对比工作区文件的字节（`git2::Patch::from_blob_and_buffer`）。`status` 仍然是 `"conflicted"`。工作区文件不存在时 hunks 为空；任一侧是二进制时 `binary = true`。

- [ ] **Step 1: 测试** `conflicted_file_diff_is_head_vs_workdir`：造一个 UU 冲突，把工作区写成 `colleague1 + patch-local（手工合并）` 并改动第 5 行。断言 hunks 里同时有 `-colleague1`、`+colleague1 + patch-local（手工合并）`、`-line5`、`+line5 手改`。这就是 spec §3 那次实测的复刻。
- [ ] **Step 2:** 跑测试，预期失败（hunks 为空）。
- [ ] **Step 3:** 实现：在 `diff_to_data` 之后，对 conflicted 的文件补齐 hunk，复用现有的行收集逻辑，不复制一份。
- [ ] **Step 4:** 跑 Rust 全部验证，预期通过；`read_status_lists_conflicted_files` 等原有测试保持绿色。
- [ ] **Step 5:** Commit `fix(diff): 冲突文件显示 HEAD 对比工作区`。

### Task 7: 外部 merge 命令与各侧标题

**Files:**
- Modify: `src-tauri/src/engine/conflicts.rs`, `src-tauri/src/commands/external.rs`, `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/external_merge.rs`

**Interfaces:**
- Consumes：Task 2、3、5。
- Produces：
  - `pub struct MergeTitles { local: String, remote: String, base: String, merged: String }`
  - `pub fn merge_titles(repo: &str) -> Result<MergeTitles>`，按 `engine::repo::state` 区分：
    - merge：local = `<当前分支>（HEAD）`，remote = `MERGE_HEAD` 的分支名，没有分支名时用短 hash；
    - rebase：local = `当前重放结果（HEAD，基于 <onto 分支或短 hash>）`，remote = `REBASE_HEAD <短 hash> <标题>`；
    - cherry-pick / revert：remote 用 `CHERRY_PICK_HEAD` / `REVERT_HEAD`；
    - base 固定为 `共同祖先`，merged 固定为 `结果（工作区）`。
  - `pub fn prepare_merge(repo: &str, file: &str, ws: &TempWorkspace) -> Result<PreparedMerge>`，其中 `PreparedMerge { local, remote, base, merged: PathBuf, titles: MergeTitles }`。
    - case 不是 Markers / NoMarkers / NotTextMerged（且 reason 不是 BinaryContent）时，报 code `external_merge_unsupported`；
    - `merged` 必须通过 `ensure_regular_file`；
    - AA 的 base 是空文件。
  - 命令：`open_external_merge(app, path: String, tool_id: String, file: String) -> Result<()>`，流程同 `open_external_diff`，展开的是 `merge_args`。

- [ ] **Step 1: 测试**
  - `merge_exports_stage_2_3_1`：fixture 设 `core.autocrlf=true`，三份内容都是 CRLF，并分别等于 stage 2、3、1 的内容。
  - `rebase_second_commit_local_contains_first`：rebase 两个提交，第一个重放成功、第二个冲突，LOCAL 里包含第一个提交的改动。
  - `rebase_titles_mention_rebase_head`
  - `add_add_base_is_empty`
  - `delete_modify_is_unsupported`
  - `binary_content_is_unsupported`
  - `merged_symlink_is_refused`（没有权限时跳过）
  - `merged_path_survives_cleanup`：用假工具（`.cmd`，`exit /b 0`）跑完后，workspace 目录被删除，`$MERGED` 还在，内容不变。
- [ ] **Step 2:** 跑测试，预期失败。
- [ ] **Step 3:** 实现，注册命令。
- [ ] **Step 4:** 跑 Rust 全部验证，预期通过。
- [ ] **Step 5:** Commit `feat(external-tool): 冲突文件的外部 merge`。

### Task 8: 前端冲突提示与 merge 入口

**Files:**
- Create: `src/engine/conflictView.ts`, `src/components/ConflictNotice.tsx`
- Modify: `src/store/repoStore.ts`（新增状态 `conflicts: ConflictInfo[]` 和 `refreshConflicts`；只在 `status.unstaged` 里有冲突时调用后端，否则直接置空；带代次校验），`src/engine/types.ts`, `src/components/LocalChangesView.tsx`, `src/components/RepoTab.tsx`
- Test: `tests/vitest/conflictView.test.ts`, `tests/vitest/LocalChangesView.test.tsx`

**Interfaces:**
- Consumes：命令 `read_conflicts({ path })`、`open_external_merge({ path, toolId, file })`；Task 1 的 `toolFor`。
- Produces：
  - `conflictView(info: ConflictInfo, mergeTool: ExternalTool | null): ConflictViewModel`，其中 `ConflictViewModel { heading: string; body: string; showDiff: boolean; sameAsHeadNote: string | null; actions: Array<'openMerge' | 'stage' | 'keep' | 'delete'> }`。

- [ ] **Step 1: vitest**（`conflictView.test.ts`，文案逐字断言）
  - Markers：heading 为 `这个文件有冲突`，`showDiff === false`，actions 为 `['openMerge']`。
  - NoMarkers：heading 为 `未检测到冲突标记，检查后可以暂存`，`showDiff === true`，actions 为 `['stage']`。
  - NotTextMerged + BinaryAttr：heading 为 `git 没有逐行合并这个文件（按 .gitattributes 或 git 配置），需要手动合并`，actions 为 `['openMerge', 'stage']`；`sameAsHead === true` 时 `sameAsHeadNote` 为 `工作区与 HEAD（你这边）完全相同，还没合并`。
  - NotTextMerged + CustomDriver（driver `unityyamlmerge`）：heading 包含 `这个文件配置了自动合并工具 \`unityyamlmerge\`，但仍处于冲突状态`；**任何 case 的 heading 都不包含「已解决」**。
  - BinaryContent：heading 为 `二进制文件冲突`，actions 为 `['stage']`。
  - DeleteModify：actions 为 `['keep', 'delete']`；Structural：actions 为 `[]`。
  - `mergeTool` 为 null 时，actions 里没有 `openMerge`。
- [ ] **Step 2: 组件测试**（`LocalChangesView.test.tsx`）
  - 选中冲突文件时渲染 `ConflictNotice`，按钮文案为 `在 UnityYAMLMerge 中解决`（规则命中 `*.unity`）；
  - 冲突行不显示 `+0/-0`；
  - 冲突文件右键菜单里有 `在 Beyond Compare 中解决`。
- [ ] **Step 3:** 跑 `pnpm test`，预期失败。
- [ ] **Step 4:** 实现。`ConflictNotice` 放在 diff 面板顶部，`showDiff` 为 true 时下面接着显示 `DiffView`。`repo-changed` 事件和外部工具退出事件都要触发 `refreshConflicts`。
- [ ] **Step 5:** 跑前端全部验证，预期通过。
- [ ] **Step 6:** 在 `pnpm tauri dev` 里实测：在 spec §3 的 fixture 里，右键冲突文件用 BC 解决 → 保存 → 界面从 ① 变成 ② → Stage。结果记进决策日志。
- [ ] **Step 7:** Commit `feat(conflict): 冲突文件的五种显示与外部 merge 入口`。

---

## 阶段 3：DU / UD 二选一

### Task 9: 保留 / 删除

**Files:**
- Modify: `src-tauri/src/engine/conflicts.rs`, `src-tauri/src/commands/conflict.rs`, `src-tauri/src/lib.rs`, `src/components/ConflictNotice.tsx`, `src/components/RepoTab.tsx`
- Modify: `src-tauri/tests/svn_write_guard.rs`（如果新命令属于它的扫描范围，就加进写方法清单；不属于的话，在决策日志里写明为什么不属于）
- Test: `src-tauri/tests/conflicts.rs`, `tests/vitest/LocalChangesView.test.tsx`

**Interfaces:**
- Produces：
  - `pub fn resolve_delete_modify(repo: &str, file: &str, keep: bool) -> Result<()>`
    - `file` 为空时报 code `empty_path`；
    - 执行前重新读 index：仍是 DU / UD 才继续，否则报 code `conflict_state_changed`；
    - keep 时要求工作区里的文件存在，并且是普通文件；
    - 执行 `git --literal-pathspecs add -- <file>` 或 `git --literal-pathspecs rm -- <file>`；
    - 先调用 `svn_lock::assert_not_syncing(repo)`。
  - 命令：`resolve_delete_modify(path, file, keep)`

- [ ] **Step 1: 测试**
  - `keep_resolves_du`：`ls-files -u` 为空，文件已暂存为修改后的版本。
  - `delete_resolves_ud`
  - `literal_pathspec_spares_lookalike`：冲突文件 `f[1].txt`，旁边有一个已修改但未暂存的 `f1.txt`；执行 delete 之后，`f1.txt` 的内容和状态都不变。
  - `empty_path_rejected`
  - `state_changed_rejected`：先在外部 `git add` 解决掉，再调用，报 `conflict_state_changed`。
  - `keep_missing_file_rejected`
  - `refused_while_sync_lock_held`：用 `svn_lock::acquire` 持有锁，调用被拒绝。
- [ ] **Step 2: 组件测试**
  - 点「删除文件」弹出确认框（`danger`，标题和内容带上文件名），点取消就不调用回调；
  - 点「保留修改后的文件」直接调用 `keep = true`。
- [ ] **Step 3:** 跑测试，预期失败。
- [ ] **Step 4:** 实现，注册命令。
- [ ] **Step 5:** 跑 Rust 和前端两侧的全部验证，预期通过。
- [ ] **Step 6:** Commit `feat(conflict): 一边删一边改的冲突在应用内二选一`。

---

## 阶段 4：merge driver 检查

### Task 10: 后端检查与缓存

**Files:**
- Create: `src-tauri/src/engine/merge_drivers.rs`
- Modify: `src-tauri/src/commands/conflict.rs`, `src-tauri/src/lib.rs`
- Test: `src-tauri/tests/merge_drivers.rs`

**Interfaces:**
- Consumes：Task 5 的 `merge_config`。
- Produces（serde camelCase）：
  - `enum DriverState { Ok, Missing, Incomplete, ProgramMissing { program: String } }`
  - `DriverFinding { name: String, state: DriverState, sample_paths: Vec<String> /* 最多 3 条 */ }`
  - `DriverReport { findings: Vec<DriverFinding>, incomplete: Option<String> }`
  - `pub fn check_merge_drivers(repo: &str) -> DriverReport`：**不返回 Err**，所有失败都写进 `incomplete`。
    - 先 `run_git_bytes(["ls-files", "-z"])`，再 `run_git_stdin(["check-attr", "--stdin", "-z", "merge"], ...)`，这两个调用都只能出现一次。
    - 排除 `unspecified set unset text binary union`。
    - `merge.default` 是自定义名字时也要检查。
    - 程序是否存在的检查规则见 spec §4.6 第 3 点。
  - 缓存：进程内的 `Mutex<HashMap<repo, (fingerprint, DriverReport)>>`。fingerprint 由以下几项组成：
    - `merge_config` 的原始输出；
    - index 里所有 basename 为 `.gitattributes` 的条目的 oid；
    - 工作区里这些文件的 mtime；
    - `info/attributes` 的路径（用 `git rev-parse --git-path info/attributes` 取）和 mtime；
    - `core.attributesFile` 的 mtime。
  - 命令：`check_merge_drivers(path) -> DriverReport`

- [ ] **Step 1: 测试**
  - `missing_driver_found_with_samples`：`.gitattributes` 写 `*.unity merge=unityyamlmerge`，没有配置，state 为 Missing，`sample_paths` 包含 `Main.unity`。
  - `incomplete_driver`：只配了 `name`，state 为 Incomplete。
  - `program_missing`：driver 指向 `C:/nope/NotThere.exe`，state 为 `ProgramMissing { program: "C:/nope/NotThere.exe" }`。
  - `ok_driver`
  - `overridden_rule_not_reported`：根目录写 `*.unity merge=old`，子目录写 `*.unity merge=binary`，文件都在子目录里，结果没有 `old`。
  - `macro_attr_counts`：`[attr]unityscene merge=unityyamlmerge` 加上 `*.unity unityscene`，能检测到。
  - `info_attributes_counts`
  - `builtin_names_excluded`
  - `zero_config_matches_is_not_error`：完全没有 `merge.*` 配置时，`incomplete` 为 None。
  - `linked_worktree_check`
  - `cache_hits_until_gitattributes_changes`：连续调用两次，第二次不再调用 `check-attr`（为此提供 `#[doc(hidden)] pub fn check_count() -> usize` 计数）；改动 `.gitattributes` 之后重新计算。
  - `merge_tree_and_stash_apply_invoke_driver`：用一个会在临时文件里留下痕迹的假 driver，分别跑 `git merge-tree --write-tree`（参数和 `svn_deliver.rs:164` 相同）和 `git stash apply`，断言 driver 被调用了。结论写进决策日志。
- [ ] **Step 2:** 跑测试，预期失败。
- [ ] **Step 3:** 实现，注册命令。
- [ ] **Step 4:** 跑 Rust 全部验证，预期通过。
- [ ] **Step 5:** Commit `feat(merge-driver): 本机 merge driver 配置检查`。

### Task 11: 横幅与合并入口前的确认

**Files:**
- Create: `src/engine/mergeDrivers.ts`, `src/components/MergeDriverBanner.tsx`
- Modify: `src/store/repoStore.ts`（`mergeDrivers: DriverReport | null`，打开仓库和收到 `repo-changed` 时刷新），`src/components/RepoTab.tsx`, `src/components/svnSyncFlow.ts`, `src/components/svnSyncOutFlow.ts`
- Test: `tests/vitest/mergeDrivers.test.ts`, `tests/vitest/svnSyncFlow.test.ts`, `tests/vitest/svnSyncOutFlow.test.ts`

**Interfaces:**
- Produces：
  - `driverBanners(report: DriverReport | null): Array<{ tone: 'danger' | 'warning' | 'info'; text: string }>`
  - `mergeRiskConfirm(report: DriverReport): ConfirmRequest | null`：只在存在 Missing 时返回请求，`danger: true`，确认按钮文案为 `仍然继续`。
  - `RepoTab` 内部：`guardMerge(): Promise<boolean>`。它重新调用 `check_merge_drivers`，返回 false 时调用方不再继续。

- [ ] **Step 1: vitest**（文案逐字断言）
  - Missing：tone 为 `danger`，文案为 `.gitattributes 让 Main.unity 等文件用 merge driver unityyamlmerge，但本机 git 配置里没有定义它。在这台机器上合并时，git 会往这些文件里写冲突标记，Unity 场景会被写坏。`，后面附上 spec §4.6 的配置示例。
  - Incomplete：tone 为 `danger`，文案为 `merge driver unityyamlmerge 只配了一部分，缺少 driver 命令。在这台机器上，所有涉及这类文件的合并都会直接失败。`
  - ProgramMissing：tone 为 `warning`，文案为 `merge driver unityyamlmerge 指向的程序不存在：C:/nope/NotThere.exe。合并时它会失败，冲突需要手动解决。`
  - `incomplete` 非空：tone 为 `info`，文案为 `merge driver 检查未完成：<原因>`。
  - 全部 Ok 时返回 `[]`。
  - `mergeRiskConfirm`：只有 Incomplete 或 ProgramMissing 时返回 null。
  - `svnSyncFlow` / `svnSyncOutFlow`：注入的 `guardMerge` 返回 false 时，不调用 sync 的 invoke。
- [ ] **Step 2:** 跑 `pnpm test`，预期失败。
- [ ] **Step 3:** 实现，并把 `guardMerge` 接到每一个会触发合并的入口：
  - `RepoTab` 里 toolbar 的 `pull`；
  - `stash_apply` / `stash_pop`；
  - commitActions 里的 `cherry_pick` / `revert`；
  - `svnSyncFlow`（`svn_sync_in` / `svn_sync_continue`）；
  - `svnSyncOutFlow`（`svn_sync_out` / `svn_sync_out_continue`）。

  漏掉一个就是缺陷。完成后用 `grep` 再核对一遍，清单记进决策日志。
- [ ] **Step 4:** 跑前端全部验证，预期通过。
- [ ] **Step 5:** Commit `feat(merge-driver): 缺 driver 时的横幅与合并前确认`。

### Task 12: 用户文档、E2E 清单与最终验证

**Files:**
- Modify: `README.md`：新增「外部 diff / merge 工具」一节，写明配置文件路径、BC / VS Code 预设，以及 spec §4.4 的 UnityYAMLMerge 示例和 §4.6 的 driver 配置示例。
- Create: `docs/superpowers/e2e/2026-10-18-external-tools-e2e.md`
- Modify: 决策日志

- [ ] **Step 1:** 写 E2E 清单，场景照抄 spec §7「真实窗口」的 7 条。造现场的脚本放进 `scripts/e2e-external-tools-fixture.ps1`（遵守 AGENTS.md §10：`$ErrorActionPreference='Stop'`，并检查 `$LASTEXITCODE`），只在 `%TEMP%` 下建临时仓库。另外加一条：在一个 5 万文件的 fixture 里记录 `check_merge_drivers` 首次调用和命中缓存的耗时。
- [ ] **Step 2:** 两侧全部验证命令都跑一遍，结果写进决策日志的进度总账。
- [ ] **Step 3:** 在 `pnpm tauri dev` 里实测四个新命令和 `read_conflicts` 的 invoke 都能通。BC 的 `BComp.exe` 是否会等窗口关闭、VS Code `--merge` 的左右顺序，由用户在真实窗口里确认，结论写进决策日志。
- [ ] **Step 4:** 用户按 E2E 清单走一遍，把结果勾到清单里。
- [ ] **Step 5:** 发起独立的最终 review（AGENTS.md §13），review 发现的高危问题都修完。
- [ ] **Step 6:** Commit `docs(external-tool): 使用说明与 E2E 清单`。
