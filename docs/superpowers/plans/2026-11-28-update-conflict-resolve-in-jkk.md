# 「更新」时 git 不管的文件冲突在 jkk 里解决：实现计划

设计：`docs/superpowers/specs/2026-11-28-update-conflict-resolve-in-jkk-design.md`　决策日志：`2026-11-28-update-conflict-resolve-in-jkk-decisions.md`

每个 Task 先写失败测试，再改实现（TDD）。Rust 测试一律用临时 fixture（`tests/common/svn_fixtures.rs` 的 `hybrid_with_colleague`）。

## Task 1：（取消）录基线前的冲突检查收窄

设计阶段决定不改 `check_recordable`，只改文案（并入 Task 4）。理由见决策日志。

## Task 2：S5 放行可放行的冲突

- `svn_sync_in::from_stashed`：冲突按设计 §1 分类，全部可放行时接着 `capture_and_publish`。
- `update_conflict_error` 只在有要回退的冲突时用，文案分两组列。
- 测试（`tests/svn_sync_in.rs`）：
  - 改写 `an_update_conflict_outside_git_rolls_back_and_points_to_tortoisesvn` → 更新完成、基线推进、`Art/x.psd` 仍是冲突。
  - 改写 `a_property_conflict_during_update_rolls_back` → 同上（属性冲突）。
  - 新增：白名单里的 `Code/ui.png` 冲突放行，基线推进，基线树里没有它。
  - 新增：目录 `Art` 上的属性冲突仍回退。
  - `a_tree_conflict_rolls_back` 不变。
  - 新增：放行之后再预览，`sync_in_svn_conflicted` 照旧拒绝，文案指向 jkk 的冲突列表。

## Task 3：冲突列表与解决（engine）

- `svn_xml`：解析 `svn info --xml` 的 `<conflict>`（`type`、`prev-base-file`、`prev-wc-file`、`cur-base-file`）。
- 新模块 `src-tauri/src/engine/svn_conflict.rs`：`list_conflicts`、`resolve_conflict`、`prepare_conflict_merge`、`open_conflict_merge`。
- 测试 `tests/svn_conflict.rs`：列表字段（文本 / 属性 / 二进制 / 目录 / W-file）；`mine`、`theirs`、`merged` 各自让状态变回非冲突、临时文件被删；`merged` 在还有冲突标记时拒绝；不是冲突、树冲突、W-file 时拒绝；路径含 `..` 时拒绝；`prepare` 给出的三个文件存在且内容对。

## Task 4：命令与文案

- `commands/svn_sync.rs`（或新文件）暴露 `svn_update_conflicts`、`svn_resolve_conflict`、`open_svn_conflict_merge`，注册到 `lib.rs`。
- 预检 `sync_in_svn_conflicted`、交付 `sync_out_svn_conflicted`、录制 `svn_preflight_conflicted` 的文案更新。
- `tests/commands_are_async.rs` 之类的契约测试按需更新。

## Task 5：前端纯逻辑与 store

- `src/store/repoStore.ts`：三个 invoke（`camelCase` 参数）。
- `src/engine/svnConflicts.ts`：行模型（分组、种类文案、可用按钮）、确认框文案、哪些错误码要弹列表。
- `tests/vitest/svnConflicts.test.ts`。

## Task 6：弹框与接线

- `src/components/SvnConflictsDialog.tsx`：设计 §3.2。
- 更新流程（`svnSyncFlow.ts`）结束后、两个错误码出现时弹框；交付流程同样接。
- `tests/layout/scenarios.ts` 与 `harness.tsx` 加超长场景；组件测试 `tests/vitest/SvnConflictsDialog.test.tsx`。

## Task 7：验证

- Rust：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`（按测试二进制分批）。
- 前端：`pnpm build`、`pnpm test`、`pnpm test:layout`。
- `pnpm tauri dev` 里用 fixture 工作副本走一遍：更新出 `.prefab` 冲突 → 列表 → 三种解决。
- 独立 review。
