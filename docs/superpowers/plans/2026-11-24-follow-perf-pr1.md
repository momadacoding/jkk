# 跟进提速 PR1：判定的空档、wc.db 子树查询、钩子状态错峰、floor 不规整原因

来源：2026-11-24 对 0.9.0 / 0.10.1 / 0.10.2 `--perf` 记录的分析（对话内，状态「已分析，未改代码」），
以及对话里对它的核对。本 PR 做分析的方案 1、2、5，方案 3 按核对结论改了做法；方案 4 单独一个 PR。

## 目标

- 判定（`check_sync`）之后约 3.7 秒没有打点的进程内工作：先打点，再去掉能确定的浪费。
- 体检的两条 wc.db 查询（excluded nodes、floor shape）慢时段合计约 6 秒：让子树条件走主键区间。
- 跟进后的刷新里两遍全量 `svn status` 并行抢磁盘：钩子状态改到刷新结束后再读。
- 体检里每次有 2 个单元要问服务器，原因只在 `log::info`：写进性能记录。

## 不变量

- 判定出的集合（changed / added / removed / locked / backward）不变。
- 体检结论不变；本地 floor 是提速，不是判据。
- 钩子状态仍是全量读（`tsvn_hooks_status`），不换成跟进前的 `quick_health`。

## Tasks

### Task 1：判定路径的进程内打点

- `perf` 加一个 `inproc(name, f)` 包装（`kind = "inproc"`，与 `git_read::or_cli` 记的同一种）。
- 打点：`parse_node_status`（`svn_nodes::read_node_status` 里，按批累计）、`check_sync` 里的
  `read_base_map`、`tree_files`、`WScope::for_repo`、`judge`（含 is_file 与 W 过滤，meta 记候选数与磁盘检查数）。
- 验收：`perf_log` 类集成测试断言一次 `check_sync` 写出这些记录。

### Task 2：方案 1，先筛 W 再 is_file；作用域复用已读的基线文件

- `judge` 的回调改成 `path_is_w` → `is_file` → `retain_not_ignored`。`WScope` 拆出只做路径判断的一步。
- `check_sync` 已经按同一个 OID 读过 `tree_files`，`WScope` 不再自己按 ref 名重走一遍基线树。
- 验收：新增单测断言 is_file 只对 W 候选调用；`svn_sync*` 集成测试全过。

### Task 3：方案 2，wc.db 子树条件走主键区间

- 子树条件改成「区间驱动 + 残余过滤」：`local_relpath >= ?2 AND local_relpath < ?2 || '0'` 加
  `(local_relpath = ?2 OR local_relpath >= ?2 || '/')`；空串（整个工作副本）单独一条，不带区间。
- 单元测试在 rusqlite 打包的 SQLite 上对 NODES / ACTUAL_NODE 的真实主键跑 `EXPLAIN QUERY PLAN`，断言走区间。
- 验收：`svn_wcdb`、`svn_health_local_floor` 等价测试全过；新增边界测试（`sub-x`、`sub.x` 这类兄弟路径不进子树）。

### Task 4：方案 3 改版，钩子状态等刷新结束再读

- `RepoTab` 的 `refreshAfterFollow` 不再在开头 `void refreshHooksHealth()`，改为刷新全部结束后再发。
- 验收：组件 / 流程测试断言钩子状态的读取发生在刷新之后；`pnpm build`、`pnpm test`。

### Task 5：方案 5，floor 不规整的原因进性能记录

- `health floor answered locally` 的 meta 加 `reasons`：每个问服务器的单元一条（截断到 200 字）。
- 验收：`svn_health_local_floor` 里断言制造不规整单元后 meta 带原因。

### Task 6：两侧门槛、独立 review、Draft PR
