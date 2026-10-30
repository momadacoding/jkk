# Stage 刷新慢：实现日志

没有单独的计划文件，输入是 2026-10-30 的只读排查报告（结论：一次 Stage 触发 4–6 次 libgit2 全量扫描，
前端自发的那轮刷新被 `latestGate` 作废）。用户选定方案 A + B，拆成两个分支 / 两个 PR：

- A（Rust）：`read_status` 只扫一遍工作区 —— 分支 `worktree-status-single-scan`
- B（前端）：同一种刷新在飞时不并发，只补跑一次 —— 分支 `worktree-refresh-single-flight`，
  日志单独放在 `2026-10-30-refresh-single-flight-decisions.md`（两个 PR 各带各的，避免同一个新文件 add/add 冲突）

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| A1 | `read_status` 现状快照测试（改动前必须先在旧实现上通过） | 完成 | `src-tauri/tests/status_contract.rs` | 旧实现上 7/7 通过 |
| A2 | `read_status` 去掉 `statuses()`，列表从已有的两个 diff 取 | 完成 | `src-tauri/src/engine/diff.rs` | 新实现上 `status_contract` 7/7、`fixtures` 32/32、`vcs_backend` 19/19 |
| A3 | 修复前后耗时对比 | 完成 | scratchpad 基准程序（不进仓库） | 15,000 文件 fixture，release：新旧输出逐项一致（38 项）；约 140 ms → 100 ms |
| B | 刷新单飞 + 尾随补跑 | 另一分支 | 见 `2026-10-30-refresh-single-flight-decisions.md` | |
| V | 公司机器真实工作副本上实测 Stage 耗时 | 未做 | | 这台开发机上没有该工作副本 |

## 决策条目

### A1 / A2

**为什么去掉 `statuses()` 是等价的（读源码确认，libgit2 1.9.6 `src/libgit2/status.c`）**

- `git_status_list_new` 内部就是 `git_diff_tree_to_index` + `git_diff_index_to_workdir`，再按路径配对。
  它不做任何额外的工作区扫描，所以我们自己已经算过的那两个 diff，完全可以直接拿来列文件。
- git2-rs 的 `StatusOptions::new()` 走 `git_status_init_options`，flags 初值为 0；`read_status` 只开了
  `include_untracked` 和 `recurse_untracked_dirs`。换算成 diff flags 是
  `INCLUDE_TYPECHANGE | INCLUDE_UNTRACKED | RECURSE_UNTRACKED_DIRS`。
  **`INCLUDE_TYPECHANGE` 是 status 固定加上的**，我们的 diff 选项原本没有。不加的话，类型变化会被拆成
  一删一增两条 delta，列表里会凭空多出项目，所以列文件用的 diff 必须补上这个 flag。
- 两个 diff 的 iterator 都带 `GIT_ITERATOR_INCLUDE_CONFLICTS`，冲突条目两边都会以 `Delta::Conflicted` 出现。
- 配对遍历保持各自的路径顺序（index 不区分大小写时两边都按不区分大小写的顺序排），所以从各自的 diff 列出的
  Staged / Unstaged 列表，与原来从配对结果里筛出来的顺序一致。

**计划没写的坑**

- 原实现的掩码里没有 `INDEX_TYPECHANGE` / `WT_TYPECHANGE` / `WT_UNREADABLE`，所以类型变化的文件
  原本就**不出现**在列表里。这是一个已有的缺口，不在本次范围内；A2 保持原行为。
- `git add -N`（intent-to-add）的实测（git 2.55 + libgit2 1.9.6）：libgit2 列成 Staged `added +0 -0`
  加上 Unstaged `modified +2 -0`，git 命令行只在工作区一侧显示 ` A`。这是原实现就有的差异，快照测试锁住的
  是现状，不是理想行为。
- 写快照预期时我预测错了两处，都按旧实现的实测输出修正：`git mv` 的一删一增按路径排序
  （`mv-dst` 在 `mv-src` 前）；intent-to-add 见上一条。

**偏离计划**

- 报告建议「在 jkk 里给 `read_status` / `read_diff` 加耗时日志」。没有加：Rust 日志插件只在 debug 构建启用
  （`src-tauri/src/lib.rs`），而 debug 构建的 libgit2 未开优化，记下的耗时没有参考价值；装机版里日志又不落地。
  改为 scratchpad 里一个 release 构建的基准程序，旧实现原样取自 `HEAD`，同时核对新旧输出逐项一致。
- 坏路径时 `readable_only` 绕开坏路径重建的那一遍 diff 仍然保留（只在出现坏路径时发生，用来算行数）。
  它和本次去掉的 `statuses()` 是两回事，不在范围内。

**为什么这么改是安全的**

- 快照测试先在旧实现上跑通再换实现，断言的是整份列表（路径、状态、行数、顺序），覆盖：两边都改、只暂存、
  只在工作区删、暂存的删除、AD、`git mv`、整目录删除、深层未跟踪目录、被忽略的文件、空目录、中文带空格的路径、
  无提交的仓库、干净仓库、UU 与 DU 冲突、悬空目录联接、intent-to-add、文件换成同名目录。
- 只读改动：不加 `UPDATE_INDEX`，不写 index，不碰工作区。

**review 发现并已修复（独立 `/code-review`，2026-10-30）**

- 第一版两个 diff 都传 `None` 作 index。libgit2 在 index 为 NULL 时走 `diff_load_index`：每个 diff 都会在
  磁盘上的 index 变了时重读一次。两个 diff 之间若有一次 `git add`（本应用的 Stage，或外部终端），Staged 和
  Unstaged 两个列表就来自两个版本的 index，刚暂存的文件可能两边都在或两边都没有。`statuses()` 只读一次 index，
  原实现只有行数可能不一致，列表不会。修复：`repo.index()` 读一次，三处 diff 都传 `Some(&index)`。
- 这个竞态快照测试测不出来（要在两个 diff 之间插入一次写），靠审读 libgit2 源码确认。

### A3

- 基准：50 × 20 × 15 = 15,000 个跟踪文件（三层目录），30 处工作区修改、8 个未跟踪文件，其中 2 处已暂存。
  旧实现原样取自 `HEAD`，和新实现交替各跑 5 轮。
- 结果（开发机，SSD，文件系统缓存是热的）：旧约 140 ms，新约 100 ms。省下的约 40 ms 是被去掉的那一遍
  `statuses()`。绝对值远小于报告里的约 1 秒/遍：报告是公司机器上的真实工作副本，`.git/info/exclude`
  有 494 行白名单规则，fixture 没有复刻这一点。**公司机器上的实际收益仍需在那台机器上实测确认。**
