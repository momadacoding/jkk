# 体检基线按症状提示（B + C）实现计划

设计：`docs/superpowers/specs/2026-11-25-svn-baseline-mismatch-design.md`
决策日志：`docs/superpowers/plans/2026-11-25-svn-baseline-mismatch-decisions.md`

## Task 1：后端判定 `find_baseline_mismatch`

- 新文件 `src-tauri/src/engine/svn_mismatch.rs`：
  - `BaselineMismatch { files: Vec<String>, total: usize, truncated: bool }`，`#[serde(rename_all = "camelCase")]`
  - `find_baseline_mismatch(repo_root) -> Result<BaselineMismatch>`，步骤按 spec §6.1 第 2–4 条：
    1. 不是 `SvnHybrid` → `svn_not_hybrid`
    2. `git diff --name-only -z --no-renames --diff-filter=M HEAD --` → 磁盘 ≠ HEAD
    3. 只留 W-file（`WScope::plain(...).w_paths`）
    4. `git diff-tree -r -z --no-renames --name-only <svn/base> HEAD` → 去掉分支碰过的
    5. 候选为空 → 直接返回，不起 svn
    6. 候选排序后取前 `MAX_MISMATCH_CANDIDATES`，`run_svn_targets(["status","-v","--xml","--"], peg_safe(..))`
    7. `item=="normal" && props=="none" && !copied` 且在候选里 → 命中
- `svn_sync::baseline_oid` 改成 `pub(crate)` 复用。
- `src-tauri/src/commands/svn.rs` 暴露 `svn_baseline_mismatch(path)`，`lib.rs` 注册。
- `src-tauri/tests/svn_baseline_mismatch.rs`：spec §8 表里的五个用例。

## Task 2：前端纯逻辑

`src/engine/svnSync.ts`：

- `BaselineMismatch` 类型
- `needsStandaloneCheck` → `mismatchCheckApplies`（语义：B 横幅与它的查询在这些时刻才做）
- `describeBaselineMismatch(m, ackKey)`、`mismatchKey(files)`
- `manualCheckBlockedReason(sync, eol, baselineMissing, syncState)`（C 的置灰原因）
- `tests/vitest/svnSync.test.ts` 更新原有 `needsStandaloneCheck` 用例 + 新用例

## Task 3：接线

- `src/store/repoStore.ts`：`svnBaselineMismatch(path)` invoke 包装
- `src/components/RepoTab.tsx`：照「缺 Local-Only 补丁」的写法，跟着 `state.status` 查；体检 `Verified` 后记 ack
- `src/components/BaselineBanner.tsx`：删常驻横幅，渲染 B 横幅；props 加 `mismatch`
- `tests/vitest/BaselineBanner.test.tsx`

## Task 4：C 的菜单

- `src/components/Toolbar.tsx`：「SVN ▾」按钮 + `ContextMenu`，项为「体检基线…」「TortoiseSVN 钩子…」
- RepoTab 传入处理函数与置灰原因

## Task 5：注释、文档、验证

- 更新 `BaselineBanner.tsx` / `svnSync.ts` 里「必须常驻」的注释，`svn_init.rs` 过时的前提段
- e2e 文档 `2026-09-22-svn-phase2a-eol-e2e.md` 里提到灰条的两行
- 两侧门槛命令 + `pnpm tauri dev` 在 fixture 仓库上走验收
