# 跟进提速 PR1 实现日志

计划：`2026-11-24-follow-perf-pr1.md`。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 判定路径的进程内打点 | 完成 | `perf::inproc`；`svn_nodes::read_node_status`、`svn_sync::check_sync_uncached` 打点 | `tests/svn_sync_judge_perf.rs` |
| 2 | 先筛 W 再 is_file；作用域复用基线文件 | 完成 | `svn_sync::added_w_files`、`WScope::w_paths`、`WScope::for_repo_with_tracked` | 同上 + `svn_sync` 单测 |
| 3 | wc.db 子树条件走主键区间 | 完成 | `svn_wcdb::in_subtree` 与四个 SQL 构造函数 | `svn_wcdb` 单测 2 条新增；`tests/svn_wcdb.rs` 33 条、`tests/svn_health_local_floor.rs` 32 条全过 |
| 4 | 钩子状态等刷新结束再读 | 完成 | `SvnSyncFlowDeps.refreshHooksHealth`、`RepoTab` | `svnSyncFlow.test.ts` 新增 1 条，93 条全过 |
| 5 | floor 不规整原因进性能记录 | 完成 | `svn_health::LocalFloor.server_reasons`、`path_free` | `tests/svn_health_floor_reasons.rs` |
| 6 | 门槛、review、PR | 完成 | PR 198（Draft） | 两侧门槛全过；独立 review 5 条全部修复，修复后重跑全部 112 个测试二进制 |

## 决策条目

### 计划之前：与分析报告不同的地方

- **偏离（方案 3）**：报告建议跟进后不再读钩子状态，或改由跟进命令带回后端查过的状态。没照做：
  1. `refreshHooksHealth()` 是 `void`，不在用户等待的链上，代价只是与 `svn_sync_status` 并行抢磁盘；
  2. 后端跟进前用的是 `quick_health`，只看注册表里已有的条目，不跑 `enumerate_wcs`，认不出新出现、
     没挂钩子的工作副本根，带回它不等价；
  3. 跟进由外部 SVN 变化触发，那次变化可能带来白名单内新的 external 根。
  改为刷新结束后再读：抢磁盘的那 1–2 秒同样省掉，结论不变。
- **计划没写的坑（方案 1）**：判定之后的空档里不只有 is_file。`WScope::for_repo` 在开了
  `respect_gitignore` 时按 ref 名再走一遍基线树（`baseline_files`），而 `check_sync` 刚按同一个 OID 走过
  （`tree_files`）；`tree_paths` 没有打点。一并打点，并让作用域复用那份结果。
- **计划没写的坑（方案 2）**：`TOP` 是关联子查询（每行一次 `MAX(op_depth)`）。全表扫描时它对工作副本
  每一行都跑一次，改成区间后只对子树里的行跑，省下的可能比报告估的多。
- **偏离（方案 2）**：报告建议同一个 wc.db 只开一次连接。先不做：打开只读连接是毫秒级，
  每单元一个事务还能保证每个单元看到的是同一时刻；区间改完后看现场打点再定。

### Task 1：打点

- `perf::inproc(name, f)`：`kind = "inproc"`，`ok` 取结果是否 `Ok`（`InprocOk` trait，只给 `Result` 实现：
  所有调用点都是 `Result`，不为假想的调用点留口子）。关闭时只多一次 `enabled()`。
- 记录名：`sync read base map`、`sync baseline tree`、`svn status parse`（每批一条）、`sync w scope`、
  `sync judge`、`sync added w-files on disk`（meta：`candidates`、`disk_checks`、`on_disk`、`added`）。
  `sync judge` 包含后者，两条一起看就知道 judge 自身的循环花了多少。
- `svn status parse` 放在 `read_node_status` 里而不是 `check_sync`：扫描缓存命中时子进程记录不再出现，
  剩下的空档就是解析。`enumerate_wcs` 自己的解析没打点，这次不动它。

### Task 2：先筛 W 再 is_file

- **为什么安全**：`path_is_w`、`is_file`、`retain_not_ignored` 都是逐路径谓词（`.gitignore` 的裁决只取决于
  路径本身与工作树里的规则，不取决于同批还问了谁），结果是三者的交集，换顺序集合不变。
  `retain_not_ignored` 仍然最后做，「只做减法」的语义不变。集成测试断言判定出的 `added` 与改动前相同。
- `WScope::filter` 拆成 `w_paths`（只做路径判断）+ `retain_not_ignored`，`filter` 自己改成调这两步，
  其余调用点不变。`w_paths` 是 `pub(crate)`，文档写明之后必须再过 `retain_not_ignored`。
- `WScope::for_repo_with_tracked`：`check_sync` 已经按同一个 OID 读过 `tree_files`，传进去当「尊重」的参照。
  与原来按 ref 名重读的差别只在两次读之间基线被移动时，那时新写法反而与映射、文件清单是同一个基线。
  `tracked` 要 clone 一份（`WScope` 持有所有权），10 万条字符串是几十毫秒，远小于重走一遍树。
- 测试先红：`svn_sync_judge_perf` 在改动前因缺打点失败，判定集合那条断言改动前就过（证明它测的是不变量）。

### Task 3：wc.db 子树条件

- **实测（rusqlite 0.43 打包的 SQLite，表结构与 `sqlite_stat1` 取自 svn 1.14.2 新检出的真实 wc.db）**：
  原写法 `?2 = '' OR local_relpath = ?2 OR (区间)` 的计划是
  `SEARCH n USING INDEX sqlite_autoindex_NODES_1 (wc_id=?)`，即扫整个工作副本；新写法外层走
  `(wc_id=? AND local_relpath>? AND local_relpath<?)`。测试 `subtree_queries_use_the_primary_key_range` 钉住四条查询。
- **计划没写的坑**：svn 会往 wc.db 的 `sqlite_stat1` 里装固定统计（`NODES` 主键 `8000 8000 2 1` 等），
  规划器的选择受它影响。测试照抄这些统计，并且建库后关掉重开（统计在读 schema 时才装进规划器），
  否则测的不是现场的计划。
- 新写法：`local_relpath >= ?2 AND local_relpath < ?2 || '0' AND (local_relpath = ?2 OR local_relpath >= ?2 || '/')`，
  空串单独 `(?2 = '')`（仍引用 `?2`，调用方总绑两个参数）。区间 `[sub, sub0)` 里还有 `sub-x`、`sub.x` 这类兄弟
  （`-` `.` 小于 `/`），残余条件剔掉；测试 `subtree_is_the_root_itself_and_what_is_below_it` 覆盖这些兄弟、
  `sub0`、`Codex`、更深一层的同名目录、空串。
- **量级（开发机，Python 自带 SQLite 3.50.4，内存库，10 万行 NODES、23 个单元，同样的统计）**：
  excluded 查询旧条件合计 67 ms（`SCAN n`），新条件 9.3 ms（走主键区间），约 7 倍。现场两条查询合计约 6 秒，
  这个量级解释不了全部：现场 wc.db 更大、冷读磁盘，旧写法每个单元把整张表从磁盘读一遍，新写法只读子树。
  现场能省多少**未验证**，要看 PR 合并后带 `--perf` 的 `wc.db floor shape` / excluded 两条记录。
- 四条 SQL 抽成 `excluded_sql` / `working_sql` / `conflict_sql` / `base_rows_sql`，测试与运行时用同一份字符串。
  `TOP` 从函数内常量挪到模块级。

### Task 4：钩子状态错峰

- 新依赖 `SvnSyncFlowDeps.refreshHooksHealth`，在 `afterFollow` 的 `Promise.allSettled([refreshAfterFollow, afterSync])`
  之后调，不 await。放在 `RepoTab.refreshAfterFollow` 的末尾不够：判定的全量 `svn status` 在 `afterSync`
  （`refreshSvnSync`）里，与 `refreshAfterFollow` 并行，要两者都落定才错得开。
- 所有走 `afterFollow` 的入口（点「跟进」、自动跟进、手动跟进、继续）都照旧每次跟进后读一次，只是时刻后移。
  `follow.refresh` 这个 span 不再包含钩子状态那遍 `svn status`。

### Task 5：floor 不规整原因

- **偏离计划**：计划写「每个问服务器的单元一条原因」。实际按原因计数（`server_reasons: {原因: 个数}`），
  不按单元列：单元名是白名单路径，性能记录按设计不记路径（`perf.rs` 的 D5，子进程参数只留选项）。
  原因文本里反引号括起来的路径、URL 换成 `` `…` ``（`path_free`），截断到 200 字。
  探测没过（`check_probes` 的 mismatched）只取全角冒号前那段：冒号后是 svn 原始报错，可能带服务器地址。
- 原来这条记录只在至少一个单元规整时才写；现在一个单元都没走本地时也写（`local: 0` 带原因），
  要查的正是这种情况。`local`、`server` 两个字段的含义不变。
- 「本地 status 没跑成」原来是 `units.clear()`，改成逐个 `drop_unit` 记原因，`LAST_LOCAL` 结果相同（空）。
  第一版写成 `std::mem::take` 后再 `drop_unit`，原因记不上，见 Task 6 第 1 条。

### Task 6：独立 review（`/code-review high`）与修复

| 第几条 | 级别 | 发现 | 处理 |
|---|---|---|---|
| 1 | important | 本地 status 没跑成时，`std::mem::take` 先清空了 `units`，`drop_unit` 的 `remove` 返回假，原因没记上；全部单元原本规整时连 floor 记录都不写了（改动前还会写 `local: 0`） | 改为遍历 `units` 的副本逐个 `drop_unit`。新增注入点 `Point::LocalFloorStatus`，`svn_health_floor_reasons` 加场景二；临时退回旧写法确认该场景失败 |
| 2 | important | `path_free` 只遮反引号里的内容，rusqlite 打不开文件时会在报错后面再附一遍路径，落在反引号外，路径会进性能记录 | 新增 `wcdb_reason`：打开、查询失败一律换成固定的「wc.db 认不出：打不开或查询失败」；格式、未完成操作这几种自己的话照留。单测覆盖 |
| 3 | nit | 探测没过的原因按冒号切，revision 留在里面，每对 revision 各成一类；svn info 解析报错整段原文进记录 | `Probed::Ok.mismatched` 每条带一个固定原因（三种），日志照旧用完整原因。`path_free` 同时把 `r<数字>` 换成 `r…`，目录 stale 那条原因也能按类计数 |
| 4 | nit | `query_changelists` 的 changelist、external 两条查询还是旧的子树条件，注释「与 `query` 相同」已不对 | 改用 `in_subtree`，抽成 `changelist_sql` / `externals_sql`，查询计划测试加上这两条（表结构加 EXTERNALS 与它的统计） |
| 5 | nit | `.gitignore` 查询失败时 `?` 先返回，`sync added w-files on disk` 不记 | 先记（`ok = false`）再抛 |

- **为什么安全（第 1 条）**：只改记账，`LAST_LOCAL` 与 `units` 的最终状态与改动前相同（空），体检结论不受影响。
- **为什么安全（第 4 条）**：与 Task 3 同一个 `in_subtree`，`tests/svn_changelist.rs` 22 条、`tests/svn_wcdb.rs` 32 条等价测试全过。
