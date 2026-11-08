# SVN 逐目录调用合并成多目标 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把跟进与后台检查里「每个白名单目录 / 每个工作副本单元起一个 svn 进程」的调用合并成一次传多个目标，进程数从 O(目录数) 降到 O(1)（按命令行长度分批）。

**Architecture:** `svn_cli` 加一个按命令行长度分批的 `run_svn_targets`；本地读（`info`、`status`、`info -R`）直接换成批量；体检的 floor diff 把 URL 就在 `svnUrl` 底下的单元合成一条 `--old=<svnUrl>@F --new=. -- <单元...>`，失败时退回逐单元的原命令；服务器侧核对的 `ls -R` 一次列多个 URL。

**Tech Stack:** Rust（`src-tauri/src/engine`）、svn CLI、`src-tauri/tests` fixture 仓库。不改前端、不改 Tauri 命令。

**Spec:** SVN 扫描性能计划 `2026-11-08-svn-scan-perf.md` 的 P4（方案 B + F）。来源数据：2026-11-07 公司机器上一次跟进约 200 秒，其中逐目录 `info -R` 22.9 s、`diff --summarize` 33.3 s，判定与预检的逐目录 `info` / `status` / `status -v` 约 35 s；报告 3.4 节实测 23 个目录一次传：`info` 1251 → 82 ms、`status` 8126 → 4139 ms。

## 范围

| 调用点 | 现在 | 改成 | 走网络 |
|---|---|---|---|
| `svn_wc::enumerate_wcs` | 每个目标 `info` + `status` | 按层批量：每层一次 `info`、一次 `status` | 否 |
| `svn_nodes::read_node_status` | 每个 target `status -v --ignore-externals` | 一次 | 否 |
| `svn_dirty::read_dirty` | 每个 target `status -v --no-ignore --ignore-externals` | 一次 | 否 |
| `svn_preflight::check_single_node_rev` | 每个 target `status -v` | 一次 | 否 |
| `svn_health::whitelist_not_under_base` | 每个白名单根 `info` | 一次 | 否 |
| `svn_health::excluded_nodes` | 每个单元 `info -R` | 一次 | 否 |
| `svn_health` floor diff | 每个单元一条 `diff --summarize` | URL 在 `svnUrl` 底下的单元合成一条，失败退回逐单元 | 是 |
| `svn_server_set::audit_inner` | 每个单元 `ls -R` | 一次 | 是 |

不在范围：`check_no_operation_in_progress`（目标是 1 + external 数，报错要点名目录）、交付专用的 `svn_stage::snapshot` / `sync_out` / `outside_changes`、`wc_db_stamps` 的重新枚举粒度（P4 的另一半，要单独论证）。

## Global Constraints

- 不新增依赖；不改 Tauri 命令名与参数。
- **空目标列表不起进程**：svn 不带 target 时作用于当前目录（整个工作副本），必须在 `run_svn_targets` 里返回空结果。
- target 的 `peg_safe` 规矩不变：`info` / `status` / `ls` 加，`diff` 的尾随 PATH 不加。
- 结论与改动前逐字相同：同一 fixture 上 `enumerate_wcs`、`read_node_status`、`read_dirty`、体检、服务器核对的返回值不变。现有测试全过。
- 失败语义不弱化：任何一个目标失败整条报错（与逐个跑时第一个失败就 `?` 相同）；体检合并命令失败时退回逐单元原命令，保住「第一条失败 = NotChecked、之后失败 = Unconfirmed」。
- Rust 验证：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。

## Review Focus

1. `assert_externals_were_ignored` 改成按 `<target>` 块判：批量里同时有 `Code` 与 `Code/Vendor` 时不能误报，单个 target 漏了 `--ignore-externals` 仍要报。
2. `info` 多目标的结果按顺序对回目标，条数对不上要报错，不能错位。
3. 体检合并命令：只合并 `url == svnUrl/wc_root`、纯 ASCII、不含 `@` 的单元；嵌套 external 照旧用自己的 URL。
4. 体检合并命令失败后退回逐单元：注入 `FirstDiff` / `LaterDiff` 时结论与改动前相同。
5. `ls -R` 多目标的每个 `<list>` 对回单元；单文件单元的特判仍成立。

---

### Task 0: 实测多目标语义

- [x] 临时测试 `tests/zz_probe_multi_target.rs`，结论记进实现日志，测试删掉。

### Task 1: `run_svn_targets` 与按 `<target>` 判 externals

**Files:** `src-tauri/src/engine/svn_cli.rs`、`src-tauri/src/engine/svn_xml.rs`；测试 `tests/svn_cli.rs`、`tests/svn_xml.rs`

- [x] 失败测试：`chunk_targets` 空列表返回空、按长度切批、单个超长 target 自成一批；`run_svn_targets` 空列表不起进程；`parse_node_status` 对两个 `<target>` 块（外层带 external 标记、另一块是 external 自己）不报错，单块里含 external 内部节点仍报错。
- [x] 实现，`cargo test --test svn_cli --test svn_xml` 全绿。

### Task 2: `enumerate_wcs` 按层批量

**Files:** `src-tauri/src/engine/svn_wc.rs`；测试 `tests/svn_wc.rs`

- [x] 测试：多白名单 + 嵌套 external + 名字带 `@` 的目录，枚举结果与改动前相同（现有测试 + 一条多白名单的回归保护）。
- [x] 实现：每层一次 `info`（按顺序对回、条数校验）、一次 `status`。

### Task 3: 本地读批量

**Files:** `svn_nodes.rs`、`svn_dirty.rs`、`svn_preflight.rs`、`svn_health.rs`（`whitelist_not_under_base`、`excluded_nodes`）

- [x] 测试：嵌套 external 的 `read_node_status`（同时以外层与 external 根为 target）不报错、结果与改动前相同；多白名单 `read_dirty`。
- [x] 实现，跑 `svn_nodes`、`svn_dirty`、`svn_preflight`、`svn_health`、`svn_sync`、`svn_init`、`svn_follow*`。

### Task 4: 体检 floor diff 合并

**Files:** `src-tauri/src/engine/svn_health.rs`；测试 `tests/svn_health.rs`

- [x] 失败测试：两个白名单根 → 一条合并命令（`--old=<svnUrl>@F --new=. -- A B`）带两条退回命令；嵌套 external 不进合并；结论与逐单元相同（Lying / Verified / 单文件白名单）；合并命令失败退回逐单元（`FirstDiff` 注入 → NotChecked；有 revision 组时 `LaterDiff` → Unconfirmed）。
- [x] 实现 `group_floor`，执行循环改成工作队列、「第一条」按「还没有一条成功」判。

### Task 5: 服务器侧核对 `ls -R` 批量

**Files:** `svn_xml.rs`（`parse_lists`）、`svn_server_set.rs`；测试 `tests/svn_xml.rs`、`tests/svn_server_set.rs`

- [x] 失败测试：`parse_lists` 多个 `<list>`；两个白名单根的核对结论与改动前相同；单文件单元特判。
- [x] 实现。

### Task 6: 开发机实测

- [x] 临时测试：fixture 上 23 个白名单目录，`check_sync` 与重建基线改动前后各 3 次计时、比结论；结果记进实现日志，测试删掉。

### Task 7: 验证、独立 review、Draft PR

- [x] `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`
- [ ] 独立 review（`/code-review high`），修完高危后提交、推送、开 Draft PR
