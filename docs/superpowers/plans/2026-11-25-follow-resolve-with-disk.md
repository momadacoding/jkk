# 用磁盘上的内容跟进：实现计划

设计：`docs/superpowers/specs/2026-11-25-follow-resolve-with-disk-design.md`（第 11 节是开工前补的缺口，与第 6～8 节同等约束）。
决策日志：`2026-11-25-follow-resolve-with-disk-decisions.md`。

## Task 1：重放引擎支持「冲突时用指定 blob」

文件：`src-tauri/src/engine/svn_replay.rs`、`src-tauri/tests/svn_replay.rs`

- `ReplayOutcome::Conflict` / `Merged::Conflict` 带上每个冲突路径的 stage 条目（`ConflictStage { stage, mode, oid }`），解析 merge-tree `-z` 输出的 stage 行。
- `ReplayOpts` 加 `resolved: &dyn Fn(&ReplayCommit) -> BTreeMap<String, Resolution>`（路径 → blob 与 mode）。merge 冲突、且冲突路径全在 `resolved` 里时，在冲突树上把这些条目换成指定 blob，当作干净结果继续；有冲突路径不在里面时照旧返回冲突（`files` 只列没解决的）。
- `ReplayOutcome::Done` 加 `resolved: Vec<(String, Vec<String>)>`：真正用上替换的 (原 commit, 路径)。
- 用过替换的 commit 及它之后的 commit 都不进 `memo`（设计第 11 节第 2 条）。
- 其余调用方（交付、Local-Only 拆分）传 `no_resolved`，行为不变。
- 测试：冲突 + 替换后结果树里该文件 = 指定 blob；有一个没替换的冲突文件仍冲突；替换后的 commit 不进 memo；没有替换时与改造前逐字节相同（现有测试不改断言）。

## Task 2：磁盘内容能不能用（纯判定）

文件：新建 `src-tauri/src/engine/svn_follow_disk.rs`，测试 `src-tauri/tests/svn_follow_disk.rs`

- `outside_hunks_match(template: &[u8], disk: &[u8], open: &[u8], close: &[u8]) -> bool`：模板按标签切成稳定段，磁盘按行贪心最左匹配。
- `has_conflict_markers(bytes) -> bool`：行首 `<<<<<<<`、`>>>>>>>` 后跟空格或行尾。
- `check(repo, ctx, path, stages) -> DiskCheck`：按设计第 6 节的顺序逐条判，返回 `Usable { blob, mode }` 或 `Refused(reason)`。磁盘 blob 用 `hash-object -w --stdin-paths`；模板用 `git -c merge.conflictStyle=merge merge-file -p --object-id -L … ours base theirs`。
- 测试：匹配函数的边界（冲突块在开头 / 结尾 / 相邻、模板段重复、磁盘多一行在段里、CRLF 已归一）；fixture 上 merge-file 在用户配了 `merge.conflictStyle=diff3` 时仍输出两段式。

## Task 3：准备阶段的探测与替换

文件：`src-tauri/src/engine/svn_follow.rs`、`src-tauri/tests/svn_follow_disk.rs`

- `prepare` 加参数 `disk: DiskUse`（`Off` / `Probe` / `Apply(&BTreeSet<String>)`）。自动跟进传 `Off`。
- 冲突时：逐个文件 `check`，全部能用就带着替换重放，循环到干净或出现 ✗。
- `Probe`：结果仍是 `Target::Conflict`，但 `ManualNeeded` 记下 `disk_ready`，每个 `ConflictDetail` 带 `disk`（能用 / 原因）。
- `Apply`：全部能用、替换的文件都在确认清单里，才走 `Target::Tip`；报告写 `disk_resolved`。否则按 `Probe` 的结果回到冲突框。
- `Plan` 记下用到的 (路径, blob)，`still_valid` 重算磁盘 blob 核对。
- `FollowOutcome::NeedsManual` 加 `base`（新基线 OID）与 `disk_ready`；`ConflictDetail` 加 `disk`；`FollowReport` 加 `disk_resolved`（`serde(default)`）。
- 新入口 `follow_with_disk(repo, opid, files)`。
- 测试（fixture：同事改同一行 → wc1 `svn update` 冲突 → 写合并结果 → `svn resolve`）：能用时完成、提交里该文件 = 磁盘、工作区 mtime 不变、本地改动里没有它；五种不能用各给原因；弹框后磁盘变了重新检查；确认清单外的文件不替换；其他分支含同一提交时不被移动。

## Task 4：外部对比「提交 vs 工作区文件」

文件：`src-tauri/src/engine/external_diff.rs`、`src/engine/externalTools.ts`、对应测试

- `DiffTarget::Worktree { from, file }`：左边是提交里的 blob，右边是工作区真实文件。

## Task 5：Tauri 命令与 store

文件：`src-tauri/src/commands/svn_sync.rs`、`src-tauri/src/lib.rs`、`src/store/repoStore.ts`

- `svn_follow_with_disk(path, files)` → `invoke('svn_follow_with_disk', { path, files })`。

## Task 6：前端文案

文件：`src/engine/svnFollow.ts`、`tests/vitest/svnFollow*.test.ts`

- 类型：`ConflictDetail.disk`、`needsManual.base` / `diskReady`、`FollowReport.diskResolved`。
- `describeFollowConflictModel(o)`：标题、说明、每个文件一行（SVN 上谁改的、✓/✗ 与原因、对比的两侧）、是否显示磁盘按钮、警告。
- 跟进报告加一节「按磁盘内容解决的冲突」。

## Task 7：冲突框组件与流程

文件：新建 `src/components/FollowConflictDialog.tsx`、`src/components/svnSyncFlow.ts`、`src/components/RepoTab.tsx`、`src/styles.css`、`tests/layout/{scenarios.ts,harness.tsx}`、组件测试

- 三个按钮：Cancel / 手动跟进 / 用磁盘上的内容跟进；逐文件对比按钮；没有工具时退回文本预览。
- 能用磁盘内容时点「手动跟进」另弹现有确认框；不能用时冲突框本身就是确认（带警告）。
- 点磁盘按钮后结果仍是冲突（磁盘变了）时，按新结果重开冲突框。

## Task 8：验证与收尾

- Rust：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`（按测试二进制分批）。
- 前端：`pnpm build`、`pnpm test`、`pnpm test:layout`。
- `pnpm tauri dev` 真实窗口：fixture 仓库走一遍，确认 `svn_follow_with_disk` 参数通、对比按钮能起工具。
- 独立 review，修完高危问题。
