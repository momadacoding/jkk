# 被排除节点改查 wc.db：实现日志

目标：跟进体检里找 `--set-depth=exclude` 节点的 `svn info -R`（fixture 上 0.75–1.9 s，且与体检的 `svn diff` 抢 `wc.db` 的锁）改为只读查询 `wc.db`。用户 2026-11-15 选了方案 B（加 `rusqlite`，只认格式号 31，其他一律退回 `svn info -R`），并要求「不要去猜」表结构。

没有单独的计划文件；本日志的 Task 划分即计划。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T0 依赖与等价性测试（先红） | 完成 | `Cargo.toml`（`rusqlite 0.43`，`bundled`，不开默认 feature）；`tests/svn_wcdb.rs` 18 条 | 模块不存在时编译不过（红）；T1 后 18/18 |
| T1 `svn_wcdb` 模块 | 完成 | `src/engine/svn_wcdb.rs`；`svn_tombstone::fault::Point::WcdbRefused` | 单测 2 条、`svn_wcdb.rs` 18/18 |
| T2 接进体检与预跑 | 完成 | `svn_health::excluded_nodes` 先查 wc.db、认不出退回 svn；`prefetch_excluded_nodes` 能查 wc.db 时不再预跑 `svn info -R`；`svn_follow_scan_budget.rs` 改断言 | `svn_health.rs` 51/51（新增退回路径 1 条）、预算测试通过 |
| T3 实测与 review | 完成 | 实测表、review 9 条（修 8 条、记 1 条） | 见下 |

## 验证

- `cargo fmt --check`、`cargo clippy -j 4 --all-targets`：无告警。
- `cargo test`：集成测试 96 个文件全过。整批放后台跑时被系统以内存不足停掉一次，停之前跑完的 47 个都过了；其余 50 个改成前台分批跑（`-j 1`、`--test-threads=2`），也都过了。
- lib 147 条：146 过。`commands::watch::tests::stage_and_unstage_only_touch_the_index` 失败，原因是 fixture 的 `.git\hooks` 事件晚到。与本改动无关：在 master 4510fa2 的源码上跑同一条，交替 3 轮，master 失败 2 次，本分支失败 2 次、通过 1 次。
- `node scripts/gen-notices.mjs --check`：通过（257 个组件）。
- 没跑 `pnpm tauri dev`：没改命令签名、返回结构和前端。review 第 9 条的日志核对也还没做。

## 表结构的依据

不靠猜，只认两份源码（Subversion 没有对外的表结构文档）：

- `subversion/libsvn_wc/wc-metadata.sql`：`NODES.presence` 的取值与注释。`excluded` 是 "administratively excluded by the user (sparse WC)"；在 WORKING 层里也可能出现。
- `subversion/libsvn_wc/wc.h`：1.8.x–1.14.x 都是格式 31；1.15.x 引入 32（可选），默认仍是 31。

再用 fixture 测试把查询结果与 `svn info -R --xml` 的输出逐条比对，比不上就算失败。

另外核对过 `libsvn_subr/sqlite.c`（1.14.x）：`#define BUSY_TIMEOUT 10000`、`PRAGMA journal_mode = TRUNCATE;`、`PRAGMA locking_mode = NORMAL;`。只有配置了独占锁时（`svn_wc__db_util_open_db` 的 `exclusive`）svn 才换锁模式。

## 决策条目

### T0 依赖与等价性测试

- **为什么是这些场景**：被排除的文件、目录；白名单外的不报；单元根在工作副本根底下（路径要换算）；两个单元；`Code` 与 `Code2` 前缀；中文目录；嵌套 external（另一个 wc.db）；`depth=empty` 不算排除；父目录待删；工作副本内 `svn copy`（WORKING 层的 excluded）；排除后再恢复。每条都与 `svn info -R --xml` 比整张表，并另外钉住 svn 那一侧的结果，避免两边都空、比对空过。
- **实测（计划没写的坑）**：父目录 `svn delete` 之后，svn 仍报其下被排除的节点（schedule `normal`）；`svn copy` 出来的副本里，被排除的节点在 `op_depth 2`，svn 两处都报。所以查询取每个路径**最高一层**，不是只看 BASE（op_depth 0）。
- **计划的错误（我自己的测试假设）**：原来断言「读完只剩 `wc.db`、没有 journal」，第一次跑就红了。svn 自己每次写完都会留一个 0 字节的 `wc.db-journal`（`journal_mode = TRUNCATE`）。改成读前、读后比较全部 `wc.db*` 文件的名字与字节。
- **认不出来就拒绝**的四条也各有测试：格式号 32；`WORK_QUEUE` 非空；单元没有 `wcroot-abspath`；单元的根在 `NODES` 里查不到。这两条测试会改临时 fixture 的 `wc.db`，只动 `TempDir` 里的副本。

### T1 `svn_wcdb` 模块

- **只读打开**：`SQLITE_OPEN_READ_ONLY`；busy timeout 与 svn 相同（10 s）。几条查询放在同一个读事务里。
- **子树用区间，不用 LIKE**：LIKE 在 SQLite 里不区分 ASCII 大小写，还要转义 `_` 和 `%`。`local_relpath` 按字节比较，`sub/` 到 `sub0` 之间正好是 `sub/` 开头的全部路径（`/` 是 0x2F，`0` 是 0x30）。
- **路径换算**：单元相对工作副本根的那一段，在 Windows 上不区分大小写匹配（盘符、打开仓库时的写法可能与 svn 记的不同）。再往下的部分照抄 wc.db。输出里单元那一段用调用方的写法，与 `svn info` 的 entry path 照抄 target 相同。
- **单元的根必须在 wc.db 里查得到、且是 `normal`**：查不到时（例如白名单的大小写与 svn 记的不同）不能当成「没有被排除的节点」，退回 svn，由 svn 照原来的方式报错或给结果。
- **kind 只认 `file` / `dir`**。`symlink` 这类 svn info 会报成什么没实测，遇到就退回 svn。
- 为什么是安全的：读不改写（有测试比字节）；认不出来的一律退回原来的 `svn info -R`；退回路径有端到端测试（`excluded_nodes_are_still_found_when_wc_db_cannot_be_read`，注入 `Point::WcdbRefused`）。

### T2 接进体检与预跑

- **偏离原计划**：原以为只要把 `svn info -R` 换掉。实际 `prefetch_excluded_nodes` 在 `git add` 之后会把它放到后台预跑，只换体检那一处，预跑照样会起，照样抢锁。所以录完时先查一次 wc.db（约 19 ms，`svn_wcdb::probe`），能查就不起 svn。查到的结果（或「认不出来」的结论）由 `ExcludedAhead::adopt` 交给体检，体检不再查第二次。初版是两处各查一次，review 后改成现在这样，见下面 review 第 3 条。
- **先后约束不变**：查询仍在 `git add` 之后，与原来预跑 `svn info -R` 是同一个时刻。预算测试按记录时刻断言这一点，并断言常态下整次跟进不起 `svn info -R`、wc.db 只查一次。改之前这条断言是红的：master 上恰好有一条 `svn info -R`。

### T3 实测与 review

**实测**（2 万文件 fixture，与应用相同的启动环境，master 4510fa2 与本分支交替各 6 轮）。机器很忙：CPU 40%，47 个 node 进程，TSVNCache 在跑。墙钟噪声比差值大，所以看每一轮的记录：

| | master | 本分支 |
|---|---|---|
| 一次跟进的 svn 总工作量 | 3.0–4.0 s | 2.2–2.5 s |
| `svn info -R` | 0.79–1.07 s | 无 |
| wc.db 查询 | 无 | 约 19 ms（首轮冷缓存 100 ms） |
| 墙钟 | 5.8–6.9 s | 5.8–7.1 s |

单独合进 master 时墙钟看不出差别。原因：master 上 `svn info -R` 本来就与录制的本地步骤并行，关键路径在后面才起跑的 `svn diff` 上；去掉它只减少了抢锁。省下的 0.6–0.9 s（上一轮在 PR 173 分支上实测）要等 PR 173 让 `svn diff` 也在录完时预跑之后才兑现：那时两条大扫描同时跑、互相拖慢，去掉一条才缩短关键路径。

**独立 review**（subagent，按 AGENTS.md §13）：没有阻断项。

| 第几条 | 问题 | 处理 |
|---|---|---|
| 1 | 新增 4 个 crate，`THIRD_PARTY_NOTICES.md` 没重新生成，发版预检会失败 | 已修：worktree 里 `pnpm install --frozen-lockfile` 后跑 `node scripts/gen-notices.mjs`，253 → 257 个组件，只多了这 4 个；`--check` 通过 |
| 2 | `svn_init.rs` 与 `prefetch_excluded_nodes` 的注释仍写着「`excluded_ahead` 就是后台的 `svn info -R`」；「必须在 `git add` 之后」的理由没落到 wc.db 查询上 | 已修，注释改为两条路都写明这个先后 |
| 3 | 常态下查两次 wc.db，每次单元 busy timeout 10 s；锁被长时间占住时最坏要等 (2N+1)×10 s | 已修：录完时查一次，结果交给体检（`svn_wcdb::Probe` / `Adopted`，只拿一次，守卫 drop 即作废，单元列表不同不拿）。查不了时体检直接走 svn，不再等第二轮。一个单元失败即整体返回，所以 wc.db 这一侧最多等一轮 10 s |
| 4 | `WORK_QUEUE` 测试只断言 `is_err()` | 已修，断言错误里有 `WORK_QUEUE`；「没有工作副本根」那条同样补了断言 |
| 5 | 「根在 wc.db 里查不到」的测试输入是 `Nope`，注释却说大小写不同 | 已修：改用 `code`；另加 `a_repo_root_spelled_in_another_case_still_reads`（整条仓库路径大小写翻转，Windows 上仍查得到） |
| 6 | 缺 `svn move`、重叠白名单两个场景 | 已补；replace（删后复制到同名）没补，是已知缺口 |
| 7 | 注释「svn 也要求先 cleanup」不对：只读的 `svn info` 不因 `WORK_QUEUE` 非空而拒绝 | 已修，改为「这一条是我们自己保守」 |
| 8 | Cargo.toml「Windows 上没有系统 SQLite」不准确（有 `winsqlite3.dll`） | 已修，改为「没有可以链接、版本可控的系统 SQLite」 |
| 9 | 真实应用传进来的 `repo_root` 若是 `\\?\C:\...` 形式，加速会静默不生效（安全，只是退回 svn） | 查了代码：跟进的 `repo_root` 是用户打开的仓库路径，`canonicalize` 只出现在局部比较里，不往下传，所以可能性低。仍记为待验证：在 `pnpm tauri dev` 里确认日志里没有「被排除的节点改用 svn info -R 找」 |
