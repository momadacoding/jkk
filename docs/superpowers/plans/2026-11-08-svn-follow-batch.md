# 跟进 R2b 批量化 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 R2b（`svn_init.rs` 的 `rewrite_to_server`，脏文件改写成服务器内容）从「每个文件 4 个进程、串行」降到「每个文件 2 个进程、有界并行 + 一次写索引」。

**Architecture:** `TempIndex` 新增 `apply_edits`：一次 `ls-files -s -z` 读出现有条目（沿用 mode、检查 D/F 冲突），一次 `update-index -z --index-info` 写入全部替换与剔除。`rewrite_to_server` 先用有界线程池并行跑 `svn cat -r BASE` + `hash-object -w --path`，再按路径顺序汇总成一批编辑。

**Tech Stack:** Rust（`std::thread::scope`，不加 crate）、git CLI、svn CLI；测试用 `src-tauri/tests/` 的 fixture 仓库。

**Spec:** SVN 扫描性能计划（PR 131 里的 `docs/superpowers/plans/2026-11-08-svn-scan-perf.md`）的 P2。来源数据：2026-11-07 公司机器上一次跟进约 200 秒，其中 R2b 约 65 秒：104 个文件各跑 `svn cat` + `ls-files -s` + `hash-object` + `update-index --cacheinfo`，88 个文件各跑 `update-index --force-remove`。

## Global Constraints

- 不新增依赖。并行用 `std::thread::scope`。
- `rewrite_to_server` 被初始化、重建、同步 S7、交付 D7a、跟进共用：**录出的 tree 必须与改动前逐字节相同**。
- `hash-object` 必须带 `--path <路径>`（`replace_blob` 注释里 2026-09-17 的 autocrlf 实测），不能为批量改成 `--stdin-paths` 或别的不跑过滤器的写法。
- 只动候选索引，不写磁盘、不碰真实索引（`svn_baseline.rs` 模块头四条硬要求）。
- 空路径一律拒绝（`svn_baseline_empty_path`），空编辑列表是 no-op，不起进程。
- `update-index` 必须串行（index.lock），只在汇总之后调一次。
- 不改 Tauri 命令；Rust 验证：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。

## Review Focus

1. D/F 冲突：`--index-info` 遇到「放 `Code/F/Y.cs`、索引里有文件 `Code/F`」会**静默替换**（实测，见实现日志），旧写法 `--cacheinfo` 报错。批量版必须保留报错，否则会从基线里挤掉一个不在 `S` 里的条目。
2. 并行里某个文件取不到（`NotOnServer`、svn 失败）：整批报错，不写任何索引条目；报错的是路径顺序里最靠前的那一个已知失败。
3. 非 ASCII、带空格的路径：`-z` 两头都要用。
4. 剔除一个仍在磁盘上的文件：磁盘不动，只摘索引条目。
5. 同一批里先剔除 `Code/F`、再放 `Code/F/Y.cs`（文件变目录）：与旧的逐条顺序执行结果相同，不算冲突。

---

### Task 1: `TempIndex::apply_edits`

**Files:**
- Modify: `src-tauri/src/engine/svn_baseline.rs`
- Test: `src-tauri/tests/svn_baseline.rs`

**Interfaces:**
- Produces:
  - `pub enum IndexEdit { Put { path: String, oid: String }, Remove { path: String } }`：`Put` 沿用索引里已有的 mode，没有则 `100644`（与 `replace_blob` 相同）。
  - `pub fn hash_blob(repo_root: &str, path: &str, bytes: &[u8]) -> Result<String>`：`hash-object -w --path <p> --stdin`，空 OID 报 `svn_baseline_hash_object`。
  - `TempIndex::apply_edits(&self, repo_root: &str, edits: &[IndexEdit]) -> Result<()>`：空列表 no-op；任何路径为空报 `svn_baseline_empty_path`；D/F 冲突报 `svn_baseline_index_conflict`，索引不动。
  - `replace_blob` = `hash_blob` + `apply_edits([Put])`；`remove_path` = `apply_edits([Remove])`。签名不变。

- [x] **Step 1: 失败测试**（`tests/svn_baseline.rs`）
  - `apply_edits_puts_and_removes_in_one_batch`：同一批替换 `Code/A.cs`、新建 `Code/New.cs`、剔除 `Code/B.cs`；树里三者分别是新内容 / 新内容 / 不存在，磁盘 `Code/B.cs` 仍在。
  - `apply_edits_keeps_existing_modes`：替换前后 `stage_mode` 相同，新建的是 `100644`。
  - `apply_edits_with_an_empty_list_is_a_no_op`：tree OID 不变。
  - `apply_edits_refuses_an_empty_path`：`Put` / `Remove` 各一条空路径，报 `svn_baseline_empty_path`，tree 不变。
  - `apply_edits_refuses_a_file_directory_conflict`：索引有文件 `Code/A.cs`，`Put Code/A.cs/X.cs` → `svn_baseline_index_conflict`；索引有 `Code/Deep/D.cs`，`Put Code/Deep` → 同样报错；tree 不变。
  - `apply_edits_allows_a_file_that_becomes_a_directory_when_the_file_is_removed_in_the_same_batch`：`Remove Code/A.cs` + `Put Code/A.cs/X.cs` 成功。
  - `apply_edits_handles_non_ascii_and_spaces`：`Code/角色 x.cs` 放进去、读得回来。
- [x] **Step 2: `cargo test --test svn_baseline`，确认因 `apply_edits` 不存在编译失败**
- [x] **Step 3: 实现**。D/F 检查：对每条 `Put p`，`p` 的任一祖先是现有条目且不在本批 `Remove` 里 → 冲突；现有条目里有以 `p/` 开头的且不在本批 `Remove` 里 → 冲突。
- [x] **Step 4: `cargo test --test svn_baseline` 全绿（含原有 `replace_blob_*` / `remove_path_*`）**

### Task 2: `rewrite_to_server` 并行取数 + 一次写索引

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_init.rs`

**Interfaces:**
- Consumes: Task 1 的 `hash_blob`、`IndexEdit`、`TempIndex::apply_edits`；`svn_pristine::cat_base`。
- Produces: `const BASE_FETCH_WORKERS: usize = 6`（`svn_init.rs` 私有）。

- [x] **Step 1: 失败测试 / 回归保护**（`tests/svn_init.rs`）
  - `rebuild_records_server_content_for_many_dirty_files`：r1 提交 40 个 `.cs`；改脏其中 30 个、删掉 5 个（`svn delete`）、新建 10 个未版本控制的；重建后 35 个是 r1 内容，10 个新文件不在树里，磁盘不动。这条在旧代码上也成立，是并行化的回归保护。
- [x] **Step 2: 实现**：`refuse_replaced` 不变；筛出 `touched` 里的条目，`TakeBase` 用线程池并行 `cat_base` + `hash_blob`（一个失败就停止领取新任务），按 `fixes` 的顺序汇总；有失败就返回顺序最靠前的那一个（`NotOnServer` 仍报 `svn_baseline_classification_conflict`，文案不变）；全部成功后一次 `apply_edits`。
- [x] **Step 3: 验证**：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。
- [x] **Step 4: 开发机实测**：临时测试在 fixture 上造 150 个脏文件跑 `rebuild_baseline`，改动前后各计时 3 次、对比录出的 tree OID，结果记进实现日志，测试删掉。
- [ ] **Step 5: 独立 review，修完高危后提交、推送、开 Draft PR**
