# 自动跟进耗时（2026-11-14）实现日志

输入：PR 167 合并后用户反馈「跟进还是很慢，自动跟进最好做到无感」。没有对应的计划文件，本文件同时记方案取舍。

## 基线测量

fixture：`perf-fixtures/svn-20k` 的副本（20011 个文件，git 纳入 6021 个，501 个本地改动，外部提交改了 30 个白名单文件 + 30 个白名单外文件，`file://` 仓库）。工具：`followbench auto`（直接调 `svn_follow::follow_auto`，性能记录器记每个子进程），每轮先把 `.git` 恢复到跟进前、`git status` 与 `svn status` 各预热一次，blob 缓存保留（PR 167 之后的常态）。

改动前（master 90c63a2），热身后两轮：8348 ms、8408 ms，54 个子进程。按用途分：

| 用途 | 进程 | 耗时 |
|---|---|---|
| `svn status` 全量扫描 6 遍（找 external 2、节点 revision 3、脏文件 1） | 6 | 约 1900 ms |
| 服务器体检：`svn info -R` + `svn diff --summarize` | 2 | 约 1730 ms |
| 录磁盘：`git add -A` + `git write-tree` | 2 | 约 1460 ms |
| blob 缓存的键：`svn info` 4 次 + `check-attr` 2 次 | 6 | 约 480 ms |
| 其余 git 小进程（每个 35–60 ms） | 约 33 | 约 1500 ms |
| 进程之外（解析 XML、遍历磁盘） | — | 约 1100 ms |

同一条链路上前端还要先跑一次检测（`svn_sync_status`：2 遍扫描）、跟进后再强制复查一次（又 2 遍）。所以用户从切回窗口到横幅消失，一共是 10 遍 `svn status` 全量扫描。

公司机器上一遍扫描约 4 s（2026-11-07 报告 3.4 节：23 个目录一次传 4139 ms），开发机约 0.3 s。**公司机器上的耗时主要由全量扫描的遍数决定**，其次是子进程个数（每个几十到上百毫秒）。fixture 上的毫秒数只用来比较前后，判断方向看扫描遍数与进程数。

## 方案取舍

| 方案 | 做法 | 结论 |
|---|---|---|
| A 削扫描遍数与进程数 | 三种 `svn status` 合成同一条命令、同一范围内只跑一次；跟进里 R3c 的重读交给发布前的复核；体检里纯本地的 `svn info -R` 与录磁盘并行；零碎进程能省则省 | **采用**。不改任何一道检查的判据 |
| B 直接读 `.svn/wc.db` | 用 SQLite 读节点 revision，不起 svn 进程、不遍历磁盘 | 不做。要新增依赖（`rusqlite`），而且等于自己重新实现 svn 判断「文件改没改」的逻辑（大小 / 时间戳 / 关键字与行尾转换后的内容），判错一个文件就是假基线 |
| C 增量录制 | 不再 `git add -A` 整个白名单，只处理判定出来的变化文件 | 不做。现在「录磁盘 + 纳入集合核对 + 体检」三道互相兜底，增量录制把基线的正确性押在判定上，等于拆掉其中两道 |
| D 先发布、后体检 | 体检挪到发布之后异步跑 | 不做。分析 §6.12 条件 6 明确「没核对过的基线不自动发布」，而且没有撤销 |
| E 发布前的复核改看 wc.db 戳 | 戳没变就不重扫 | 不做。操作进行中（句柄没关）时 NTFS 目录项里的时间戳可能还没更新，复核会漏掉正在进行的提交 |

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T0 计时回路 | 完成 | scratch 里的 `followbench` + `run-follow.sh`（不进仓库） | 见「基线测量」 |
| T1 三种 `svn status` 合成一条 | 完成 | `svn_cli::STATUS_SCAN`；`svn_wc` / `svn_nodes` / `svn_dirty` / `svn_sync` / `svn_xml` | `svn_follow_scan_budget.rs`（改之前红：检测 2 遍）；`svn_nodes.rs` 新增 1 条（去掉过滤时红）；`svn_wc.rs` 新增两层嵌套 external 1 条 |
| T2 跟进里 R3c 交给发布前的复核 | 完成 | `svn_init::RecordMode`、`record_mixed_for_follow`；`svn_follow::still_valid` | 预算测试降到 2 遍；`svn_follow.rs` 新增 1 条（去掉复核里的 `bypass_scan_cache` 时红：发布了错误基线） |
| T3 体检与录磁盘并行 | 完成 | `svn_cli::prefetch_scan` / `ScanAhead` / `in_scan_cache`；`svn_health::prefetch_excluded_nodes`；`svn_init::record_server_index_timed`；`svn_include_check::verify_include_set_listing` | `svn_cli.rs` 新增 4 条 |
| T4 零碎进程 | 不做 | — | 见「没做的」 |
| 独立 review（`/code-review high`） | 10 条：修 6 条、记 4 条 | 见「review 处理」 | 修完后相关测试与全量测试重跑 |

## 结果

同一 fixture、同一方法（每步之后热身 1 轮、取后 2 轮）：

| 版本 | 自动跟进耗时 | 全量扫描 | 子进程 |
|---|---|---|---|
| 改动前（master 90c63a2） | 8348 / 8408 ms | 6 遍 | 54 |
| T1 + T2 | 6883 ms | 2 遍 | 50 |
| T1 + T2 + T3 | 4945 / 4984 ms | 2 遍 | 50 |
| review 修完之后 | 4848 / 4905 ms | 2 遍 | 50 |

检测（`svn_sync_status`）单独量：487 / 495 ms，1 遍扫描。

连同前端在跟进前后的检测（各从 2 遍降到 1 遍），用户从切回窗口到横幅消失：全量扫描 10 遍 → 4 遍。按公司机器一遍约 4 s 估，光扫描就少 24 s 左右；这个数没有在公司机器上实测，要用新构建带 `--perf` 录一次才算数。

没有达到「无感」：fixture 上还剩约 5 s，其中发布前后约 30 个 git 小进程 1.4 s、服务器体检 1 s、录磁盘 0.8 s、两遍扫描 0.6 s、blob 缓存的键 0.5 s。再往下要动的是「没做的」里那几项，或者方案 B / C。

## 验证（最终代码）

- `cargo fmt --check`、`cargo clippy -j 2 --all-targets`：通过，无告警。
- 集成测试：95 个测试文件、1885 条，全部通过。分批在前台跑的（`-j 2 -- --test-threads=4`，每批 4–16 个文件）：整批放后台跑时两次被系统因内存不足回收。
- lib 单元测试：145 条里 144 条通过。`commands::watch::tests::stage_and_unstage_only_touch_the_index` 失败（「stage 碰到了清单以外的相关路径：`.git/hooks`」）。判断与这次改动无关：
  - `src/commands/`、`src/vcs/` 相对 master 没有改动，这条测试只走纯 git 的暂存，不经过任何 `svn_*` 代码；
  - 只跑 `commands::watch` 时通过，跑 `commands::` 或整个 lib 时失败，`--test-threads=1` 也一样，是同进程里别的测试先跑造成的顺序依赖；
  - 同一天、带着 T1–T3 改动的一轮全量里它通过过。
  没有在 master 上对照跑（会话隔离在本 worktree 里），所以「master 上也失败」是推断，不是实测。
- 前端没有改动，没跑 `pnpm build` / `pnpm test`。
- 没有在真实窗口里跑：命令签名、返回结构都没变。公司机器上的耗时没有实测。

## 决策条目

### T1 三种 `svn status` 合成一条

做法：`svn_cli::STATUS_SCAN`（`status -v --no-ignore --ignore-externals --xml --`）是原来三条命令参数的并集，`read_node_status`（节点 revision）、`read_dirty`（脏条目）都发它；`enumerate_wcs`（找嵌套 external）在扫描缓存范围里发它，不在范围里时照旧发原来那条输出很小的 `status --xml`（review 第 3、7 条）。`check_sync_with_nodes` 与 `run_preflight_with` 整个包进 `with_scan_cache`；`still_valid` 的重读先 `bypass_scan_cache` 再开一个只包住这次重读的范围。

为什么这么改是安全的：

- **找 external**：原来不带 `--ignore-externals`，一次看到所有层的 external；现在带，只看到直接挂在这一层的，更深的由 `enumerate_wcs` 已有的按层循环到它自己的工作副本里找。external 根那条 `item="external"` 带不带这个参数都出现，是 `svn_xml::assert_externals_were_ignored` 文档里 2026-09-13 的实测，`read_dirty` 也一直依赖它。最终枚举出的集合相同，现有的嵌套 external 用例（`tests/svn_wc.rs`、`svn_externals.rs`、`svn_preflight.rs`）钉着。
- **节点 revision**：多了 `--no-ignore`，只多出 `item="ignored"` 的条目。`read_node_status` 读完就丢掉它们，节点表与原来逐条相同。不丢的话 `dir_revs` 会把「只含被忽略文件的目录」认成目录、参与 floor，映射会变。
- **脏条目**：命令一个字没变。
- `still_valid` 是专门用来读「此刻」的，所以不沿用外层范围；现在的调用方都不在范围里，`bypass` 是防着以后有人把整次跟进包进去。

计划没写的坑：

- 白名单底下有嵌套 external 时，枚举按层发命令（第一层是白名单、第二层是 external 根），另两处一次传全部 target，命令行不同、缓存命不中，多扫一遍。原来是「层数 + 2」遍，现在是「层数 + 1」遍，没有变差。白名单落在 external **里面**（本项目的常态）不属于这种情况，只有一层。按 target 拆缓存做不到：`svn status --xml` 把 changelist 成员放在 `<target>` 之外的 `<changelist>` 块里，拆不回各个 target。
- 只调 `enumerate_wcs`、不接着读节点的调用方（`svn_targets`、`wc_db_stamps` 首次枚举、`svn_log`、钩子等）都不在扫描缓存范围里，走的仍是原来那条命令，行为与改动前逐字相同。第一版让它们也发大输出的共享扫描、并在 `parse_external_roots` 里加了「没有 external 就不解析」的捷径，review 指出两个问题：白付十几 MB 的打印与拷贝；坏掉的 XML 不再报错。现在捷径只留在范围内那条路径上（那份输出随后会被读节点 / 脏条目的调用方完整解析），`parse_external_roots` 本身没改。

### T2 跟进的 R3c 交给发布前的锁内复核

做法：`record_mixed_with` 的 `reuse: bool` 换成 `RecordMode { Standalone, Follow }`。`Follow` 时不做 R3c；`record_mixed_reusing(.., true)` 改名 `record_mixed_for_follow`，文档写明调用方的义务。

为什么这么改是安全的：

- R3c 防的是「R2a 读节点状态之后外部提交或更新，某个文件的 BASE 换了 revision」。换成 BASE 的那批文件（`S`）体检不比，只有重读节点状态才拦得住。
- 跟进在发布前有同样的重读（`still_valid`，注释里原来就叫「R3c 的第三道」）：同一条规则（`compute_map`）、同一份文件清单（新基线树）、比较对象是录下的映射，时间更晚、持同步锁。BASE 换了 revision 不会自己换回去，录制里那次看得到的变化，锁内那次一定也看得到。
- `Recorded` 进了 `Plan` 之后，所有会动 ref 的路径（`follow` / `follow_auto` → `publish` / `publish_auto`）都先过 `still_valid`，而且 `plan.recorded` 恒为 `Some`（部分跟进时是完整录制的那份映射）。
- 其余入口（初始化、重建、更新的 S7）录完就发布，后面没有第二次重读，`Standalone` 的 R3c 原样保留。

行为差别：录制期间工作副本被外部改动时，原来在准备阶段以 `svn_rebuild_revision_moved` 拒绝；现在由 `still_valid` 返回 `false`，回到 F1 重来一次（最多一次），还在变才以 `svn_follow_state_changed` 结束。自动跟进把后者当暂时性原因，2 秒后再试一次。对用户是从「报错」变成「自动重试」。

测试怎么造出这个时序：跟进内部没有「录到一半」的注入点，所以用外层的 `with_scan_cache` 让准备阶段读到的全是外部提交之前的节点状态（`a_follow_that_recorded_from_stale_scans_is_stopped_by_the_recheck_in_the_lock`）。脏文件 A 在缓存之后被提交：录制取到的是新 BASE 的内容、映射照缓存记成旧 revision，体检因为 A 在 `S` 里查不出来。把 `still_valid` 里的 `bypass_scan_cache` 去掉，这条测试里跟进就以 `Done` 发布了那份错误基线，所以这个 `bypass` 是承重的，不只是防御。

### T3 体检与录磁盘并行

两处：

1. **`svn info -R` 预跑**。`svn_cli::prefetch_scan` 在后台线程里跑一条可缓存的只读扫描，`ScanAhead::adopt` 把输出记进调用线程的扫描缓存。录制在磁盘录完（`git add -A`）之后为体检的 `excluded_nodes` 预跑，体检前 `adopt`（第一版起在录磁盘之前，review 第 1 条指出那会开出一个竞态窗口，见「review 处理」）。体检代码没动：它照常发那条命令，命中缓存。
   - 只在扫描缓存范围内起作用（跟进的准备阶段）。别的录制入口不在范围里，`prefetch_excluded_nodes` 第一步就看 `in_scan_cache()`、直接返回 `None`，行为不变。**这道检查是自查时补的**：第一版先枚举工作副本再问能不能缓存，初始化、重建、更新会因此白跑一次 `svn info` 和一遍全量扫描。
   - `verify_include_set` 的签名没动（`tests/svn_exclude.rs` 在用），返回清单的是新加的 `verify_include_set_listing`。第一版直接改了返回类型，clippy 编译测试时报错才发现。
   - 故障注入（`svn_tombstone::fault`，线程局部）不受影响：`fault::hit(Point::ExcludedScan)` 在 `excluded_nodes` 里、主线程上、发命令之前。
   - 部分自动跟进第二次体检（`confirm_tombstones`）原来就靠缓存命中这条命令，现在仍然命中。做成「把结果塞进缓存」而不是「把 JoinHandle 传进体检」就是为了这个，也为了不改体检的签名。
   - 没被 `adopt` 的（中途报错返回）在 drop 时等后台那条跑完，不留一个还在读工作副本的 svn 进程（测试的临时目录这时正要删）。
2. **`git write-tree` 与体检并行**（只在 `RecordMode::Follow`）。体检要的文件清单先取候选索引里的（纳入集合核对刚读过的那份 `ls-files`，`verify_include_set` 改为把它返回），写树在 `std::thread::scope` 的另一个线程里跑；树写出来之后核对 `tree_files(tree) == 清单`，不等就按树重新体检一遍（没有已知输入能走到）。「映射的键取录出来的树」这条没变，只是先用后验。报错先后与串行时相同：写树失败先报。

计划没写的坑（实测）：

- `write-tree` 慢不是 fsync。fixture 上同一份候选索引（新建 125 个松散对象）：默认 1009 / 1340 ms，`-c core.fsync=none` 1051 ms，`-c core.fsyncMethod=batch` 1675 ms（当时后台在跑 cargo test，绝对值偏大，相对关系成立）。慢在 Windows 上新建文件本身，约 8 ms 一个，配置消不掉，所以选并行。
- `svn_follow::check_git_version` 与 `git_cli` 的全局版本检查在应用里都只问一次；计时工具每轮是新进程，时间线开头那两个 `git --version` 在真实应用里不存在。

### review 处理（`/code-review high`，2026-11-14）

| 条 | 问题 | 处理 |
|---|---|---|
| 1 | 预跑的 `svn info -R` 起在 `git add -A` 之前：两者之间发生 `--set-depth=exclude`，扫描看到目录还在、`git add` 时文件已经没了，整单元 `svn diff` 又不报被排除的节点，基线静默丢文件、没有墓碑 | **修**。预跑挪到磁盘录完之后才起（`Indexed::excluded_ahead`），与原来「写完树再扫」是同一个先后；它仍与改写、核对、写树并行，耗时没变。`svn_follow_scan_budget.rs` 按进程记录的时刻断言「起在 `git add` 结束之后、`write-tree` 之前」 |
| 2 | 跟进里先 `audited?` 再比清单：清单对不上时第一次体检的错误直接返回了，退路走不到 | **修**。先比清单，对不上就丢掉第一次的结论（不管成败）按树重来 |
| 3 | `parse_external_roots` 的子串捷径跳过了 XML 解析，截断的输出不再报错 | **修**。见 T1「计划没写的坑」 |
| 4 | 写树失败要等并行的体检返回才报 | 记为已知代价（代码注释里写了）。写树失败本身罕见，要快就得能取消在跑的 svn 进程 |
| 5 | 前置校验在范围外连发三遍相同的 `STATUS_SCAN` | **修**。`run_preflight_with` 包进 `with_scan_cache`，更新、交付、重建、手动跟进的前置校验 3 遍 → 1 遍（预算测试钉住）。初始化 / 重建 / 更新录制里的 R3c 重读没动（2 遍），不在这次的范围 |
| 6 | P2（`check_single_node_rev`）自己一条命令，共用不了 | 不做。只有交付后的 D7 走它；要给 `parse_node_rev_entries` 加同样的过滤，另开一次 |
| 7 | 只枚举的调用方白付大输出 | **修**。同第 3 条 |
| 8 | `include_set_mismatch` 退回空索引重录时，第一次预跑的结果被丢掉、第二次再跑 | 不做，而且不能按 review 建议「drop 时进缓存」：重录要重新 `git add`，扫描必须排在新的那次之后（第 1 条的同一个理由）。罕见路径，多等一次 |
| 9 | `prefetch_scan` 为了问「有没有」把十几 MB 的输出拷出来；`adopt` 又拷一份 | **修**。`scan_is_cached` 只查键，`adopt` 把拥有的字符串直接移进缓存 |
| 10 | 跟进里写树与等预跑不在阶段计时里 | 部分修。等预跑包进了 `clock.run`；写树的耗时在它自己的线程里写开发日志（`clock` 在体检线程手里）。`--perf` 记录不受影响，两条进程本来就各有一条 `proc` |

### 没做的

| 项 | 为什么不做 |
|---|---|
| `git add -A` 时排除脏文件（它们随后都会被换成 BASE 或剔除，现在每次白哈希一遍） | 要给 `add` 加成百上千条 exclude pathspec，文件 / 目录换型、大小写不敏感下与现在的等价性没有把握 |
| blob 缓存的 4 次 `svn info` 合并（改走 `--targets` 文件） | 只省 2 个进程；`--targets` 的路径编码在本机踩过坑（线程代码页 936） |
| 发布阶段的 18 个 git 小进程 | 每个都对应崩溃恢复里的一步，改动面与收益不成比例 |
| 发布前的复核改看 wc.db 戳（方案 E） | 见上 |
