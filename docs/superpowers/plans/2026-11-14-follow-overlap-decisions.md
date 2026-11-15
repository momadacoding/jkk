# 自动跟进：把体检的 `svn diff` 与本地步骤重叠（2026-11-14）实现日志

输入：PR 168 合并后用户反馈「开发机上的结果已经不满意」，不用再去公司机器上量。上一轮的取舍与没做的项见
`2026-11-14-follow-latency-decisions.md`。没有对应的计划文件，本文件同时记方案取舍。

## 基线测量

fixture：`perf-fixtures/svn-20k` 的副本（与上一轮同一脚本造：20011 个文件，git 纳入 6021 个，外部提交改了 30 个白名单文件 + 30 个白名单外文件，`file://` 仓库）。工具：scratch 里的 `followbench auto`（直接调 `svn_follow::follow_auto`，性能记录器记每个子进程），每轮先把 `.git` 恢复到跟进前、`git status` 与 `svn status` 各预热一次，blob 缓存保留。

改动前（master 4510fa2）：热身 1 轮后 8021 / 8676 ms。比上一轮记的 4.9 s 慢，原因是这次测量时机器上有别的会话在跑 cargo：`git write-tree` 从 461 ms 涨到 2771 ms、`git add` 从 344 ms 涨到 1116 ms，其余进程基本不变。所以本轮比较前后都取多轮里的最小值，并看关键路径上的先后，不只看总数。

关键路径（PR 168 的 4.9 s 那份记录）：

| 段 | 内容 | 耗时 |
|---|---|---|
| 准备前段 | `svn info`、全量扫描 1 遍、`read-tree`、`ls-files` 等 | 约 0.85 s |
| 录磁盘 | `git add -A` | 约 0.35 s |
| blob 缓存 | 读键（`svn info` ×2 批 + `check-attr`）+ 写前复核（同样一套） | 约 0.55 s |
| 改写 + 纳入集合核对 | `update-index`、遍历磁盘、`ls-files` | 约 0.35 s |
| 体检 | `svn diff --summarize`（与 `write-tree` 并行） | 约 1.0 s |
| 准备后段 | 重放用的 git 小进程 7 个 | 约 0.3 s |
| 锁内复核 | `svn info` + 全量扫描 | 约 0.3 s |
| 发布 | git 小进程 23 个 | 约 0.95 s |

`svn diff` 是单项最大的一块，而它在 `git add` 之后要等 blob 缓存、改写、核对全部做完才起跑。

## 方案取舍

| 方案 | 做法 | 结论 |
|---|---|---|
| A 体检的 floor `svn diff` 在磁盘录完后预跑 | 与 `svn info -R` 的预跑同一个时刻、同一个约束（必须在 `git add` 之后），和 blob 缓存、改写、核对、写树并行；体检照常发命令，参数逐字相同时拿预跑的结果 | **采用** |
| B blob 缓存的写前复核移出关键路径；读键与读属性并行 | 复核放到后台线程，录制返回前等它；`svn info` 与 `check-attr` 同时跑 | **采用** |
| C 用 C 重写 / 进程内链接 libsvn | 慢的是 svn 自己遍历磁盘、问服务器，进程内调用同一套代码不会变快；还有工作副本格式与崩溃面的风险 | 不做（对话里已向用户说明） |
| D `git add` 排除脏文件 | 少哈希几百个文件、少废掉几百个目录的 cache-tree（写树时少刷新约 280 个已有对象的时间戳） | 这轮不做：上一轮记的等价性顾虑（换型、大小写）仍在。实测收益约 0.3–0.4 s，排在 A、B 之后再评估 |
| E 绕过 `cmd\git.exe` 启动器直接起 `mingw64\bin\git.exe` | 每个 git 进程省约 10 ms（实测 40 → 30 ms） | 起初定为这轮不做（启动器会改 PATH、设 `MSYSTEM`，钩子、凭据、ssh 依赖它们）；T1 + T2 之后 git 小进程成了关键路径上最大的一块，补了等价性的实测之后**改做**，见 T3 |

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T0 计时回路 | 完成 | scratch 里的 `followbench` + `run-follow.sh`（不进仓库） | 见「基线测量」 |
| T1 floor `svn diff` 预跑 | 完成 | `svn_cli::run_ahead` / `RunAhead` / `AdoptedAhead`；`svn_health::prefetch_floor_diffs`；`svn_init::predicted_listing`、`Indexed::diff_ahead` | `svn_follow_scan_budget.rs` 新断言（改之前红：diff 排在写树之后）；`svn_cli.rs` 新增 3 条 |
| T2 blob 缓存复核移出关键路径 | 完成 | `BaseBlobs::save_in_background` / `Saving`；`read_inputs` 两路并行 | `svn_follow_reuse_base.rs`、`svn_follow_reuse_safety.rs` 通过 |
| T3 绕过 `cmd\git.exe` 启动器（方案 E） | 完成 | `engine::git_program`；`git_cli::new_git_process` / `resolve_direct_git` | `git_program` 单测 6 条；`tests/git_cli.rs` 子进程环境逐行对比（PowerShell 下跑，见 T3「计划没写的坑」） |

## T1 + T2 的结果

同一 fixture，master 版与新版计时工具交替跑（`followbench-base` 由 `git archive 4510fa2` 编出），第一轮两边都是冷的、不算：

| 轮 | master 4510fa2 | T1 + T2 |
|---|---|---|
| 2 | 5580 ms | 5054 ms |
| 3 | 5426 ms | 5003 ms |
| 4 | 5819 ms | 5149 ms |

省约 0.5 s，比估的 1.3 s 少。原因在时间线里：预跑的 `svn diff` 与 `svn info -R` 同时遍历工作副本，互相拖慢（diff 1.0 → 1.8 s、`info -R` 1.0 → 1.5 s）。体检的结束时刻从 3.72 s 提前到 3.13 s。

## T1 + T2 + T3 的结果（与应用相同的启动环境）

计时工具从 PowerShell 起（PATH 上第一个是 `cmd\git.exe`、没有 `MSYSTEM`，与从资源管理器启动的应用相同；现场准备仍走 bash），master 版与新版交替，脚本 scratch 里的 `ab.ps1`：

| 轮 | master 4510fa2 | T1 + T2 + T3 |
|---|---|---|
| 1 | 6710 ms | 4962 ms |
| 2 | 5661 ms | 5134 ms |
| 3 | 5552 ms | 5198 ms |
| 4 | 6982 ms | 5003 ms |
| 5 | 5658 ms | 4918 ms |

master 的第 1、4 轮是离群值（机器上有别的会话在编译）。去掉它们：5.55–5.66 s → 4.92–5.20 s，省约 0.5 s（约 10%）。git 小进程平均 52–55 ms → 43–46 ms（多出的两个进程：预测清单的 `ls-files`、只跑一次的 `--exec-path`）。

**之前从 bash 跑的计时量不到 T3**：Git Bash 的 PATH 第一个 `git.exe` 是 `mingw64\bin\git.exe`，master 版也从不经过启动器，两边 git 进程一样快（这正是 T3 要在应用里得到的效果）。

### 为什么到此为止：svn 的只读命令在同一个工作副本上基本串行

T1 之后预跑的 `svn diff` 与 `svn info -R` 互相拖慢，单独量了一次（fixture，16 核）：

| 命令 | 单独跑 | 并行跑 |
|---|---|---|
| floor `svn diff` + `svn info -R` | 1021 + 735 ms | 墙钟 1753 ms |
| `svn info -R` ×2 | 868 ms | 墙钟 1443 ms |
| `svn status -v` + `svn info -R` | 380 + 868 ms | 墙钟 994 ms |

不是 CPU（16 核），最可能是 `wc.db`（13 MB 的 SQLite）上的锁在排队。所以 svn 这一侧的耗时取决于 svn 的**总工作量**，不取决于怎么排：一次跟进里全量扫描、`info -R`、体检 diff、锁内复核、blob 缓存的 `svn info`，合计约 2.7 s。T1 能做的只是让 svn 从 `git add` 结束起一直有活干；T2 的后台复核里的 `svn info` 也在抢同一把锁，所以它的实际收益比预想小。再往下要减的是 svn 调用本身（见「没做的」）。

## 验证（最终代码）

- `cargo fmt --check`、`cargo clippy -j 2 --all-targets`：通过，无告警。
- 全量 `cargo test`，在 PowerShell 里跑（没有 `MSYSTEM`，直连 git 生效）：95 个集成测试文件全部通过。整批放后台跑时被系统因内存不足回收了一次，之后改为前台分批跑（`-j 1 -- --test-threads=2`，每批 8–20 个文件）。被回收前已跑完的 24 个文件与 lib 用那次的结果。
- lib 单测：全量里 151 条有 1 条失败，`commands::watch::tests::stage_and_unstage_only_touch_the_index`（「stage 碰到了清单以外的相关路径：`<fixture>\.git\hooks`」）。判断与本 PR 无关：
  - 单独跑 lib（`cargo test -j 2 --lib`）两次都是 151/151 通过，含这条；
  - 只在整批跑、机器负载高时失败，T3 没生效的那轮（从 Bash 跑，Git Bash 设了 `MSYSTEM`）也一样；
  - 报的是 fixture 刚 `git init` 完的 `.git\hooks`：像是写示例钩子的事件晚于测试里 500 ms 的排空窗口才到；stage 本身不碰 hooks。
  - master（`git archive 4510fa2` 导出的副本）单独跑 lib：145/145 通过。
- 前端没有改动，没跑 `pnpm build` / `pnpm test`。
- 没有在 `pnpm tauri dev` 真实窗口里跑：命令签名、返回结构都没变。T3 影响所有 git 子进程，但子进程环境的等价性由 `tests/git_cli.rs` 在与应用相同的启动环境下逐行比过。公司机器上的耗时没有实测。

## 决策条目

### T1 floor `svn diff` 预跑

做法：`git add` 一结束（与 `svn info -R` 的预跑同一时刻），按候选索引此刻的清单加上改写计划预测出体检要用的文件清单（`predicted_listing`），用体检里同样的 `compute_map` / `enumerate_wcs` / `plan_diffs_for` 排出 floor 组的命令，放到后台线程跑。体检前 `adopt`，`run_svn` 遇到参数逐字相同的调用直接拿结果，只拿一次；守卫在体检结束时 drop，没被取走的作废。

偏离 / 取舍：

- 没有复用扫描缓存（`prefetch_scan`）。扫描缓存在范围内可以反复命中，`svn diff` 不该这样：部分自动跟进的第二遍体检（`confirm_tombstones`）要的是那一刻的结果。所以另起一个「只给一次」的机制（`run_ahead`），失败也原样交出去，不重跑一遍、再等一轮超时。
- 只预跑 floor 组。floor 组命令只取决于单元、floor、映射是否单值；revision 组的尾随 PATH 要剔除未版本化 / 丢失的（`skip`），被排除节点的那几条要等 `svn info -R`，预测不了，照旧在体检里跑。
- 预测的清单要多跑一次 `git ls-files`（约 40 ms，在关键路径上）。没有放进后台线程：那时主线程正在改写同一份候选索引，Windows 上 git 改名覆盖一个正被别的进程读着的索引文件会重试、甚至停在交互提示上。

为什么这么改是安全的（论证也写在 `prefetch_floor_diffs` 的文档里）：

- 体检能拦住「状态扫描之后、`git add` 读到之前被改过的文件」，靠的是它读磁盘排在 `git add` 之后。起跑点就是 `git add` 结束，这个先后不变（预算测试按进程时刻断言）。
- 起跑到原来的位置之间，本进程只做 blob 缓存（读）、改写（只写候选索引）、核对（读）、写树（写对象库），不碰工作副本、不发 SVN 写命令。期间外部的改动，原来起跑之后发生的同样看不到，由锁内复核与下一次检测兜住。
- 预测错了只是参数对不上：体检自己再跑一遍，结论与原来相同（多跑一条命令）。fixture 上预测命中，预算测试断言整次跟进只有一条 `svn diff`。
- 测试注入（`fault::arm(FirstDiff)` 等）是线程局部的，预跑线程看不到；注入点在体检里、发命令之前，命中时根本不取预跑结果，行为与原来相同。

计划没写的坑：

- 两条大扫描并行会互相拖慢（见上面的结果）。fixture 在 `%TEMP%` 下，安全软件对这个目录扫得更狠，公司机器上是否同样明显没有实测。
- 自建的 master 版计时工具要从 `git archive` 导出 `src-tauri`、`THIRD_PARTY_NOTICES.md`、`LICENSE`，还要一个空的 `dist/`：`tauri-build` 在编译期检查资源与前端目录存在。

### T2 blob 缓存复核移出关键路径

做法：`rewrite_to_server` 取完数之后不再当场 `save`，改为 `save_in_background`，返回的 `Saving` 存进 `Indexed`、由 `record_mixed_with` 持有到录制结束，drop 时等它写完。`read_inputs` 里 `svn info`（留在调用线程）与 `git check-attr`（另一个线程）同时跑。

为什么这么改是安全的：

- 缓存只决定「要不要再 `svn cat`」，录制后续步骤不读它。写前复核仍排在全部 `svn cat` 之后，挪晚只会让「前后输入不同、这条不存」更严。
- 复核原来要 `bypass_scan_cache`；新线程本来就不在扫描缓存范围里，读到的就是最新的。
- `svn info` 留在调用线程，是为了在扫描缓存范围里照旧走缓存（`load` 那一次），行为与原来逐字相同。报错的先后不变：键先报。

### T3 绕过 `cmd\git.exe` 启动器

T1 + T2 之后，关键路径上最大的一块成了 git 小进程（约 30 个，每个 35–60 ms）。原来记为「这轮不做」（方案 E），实测之后改做：每个进程固定省约 10 ms，而且对全应用的 git 调用都生效，不只是跟进。

实测（PowerShell，`rev-parse HEAD` 各 40 次、三轮）：

| 起的程序 | 每次 |
|---|---|
| `C:\Program Files\Git\cmd\git.exe`（PATH 上的那个） | 40.0–44.4 ms |
| `mingw64\bin\git.exe` | 28.2–31.9 ms |
| `mingw64\libexec\git-core\git.exe` | 30.6–34.0 ms |

等价性怎么确认的（子进程里跑 `env | sort`，逐行对比）：

| 起法 | 与启动器的差别 |
|---|---|
| 直接起 `mingw64\bin\git.exe`，什么都不设 | `MSYSTEM` / `PLINK_PROTOCOL` / `HOME` 相同（真正的 git 自己补）；PATH 里 `~/bin` 排在 `mingw64/bin`、`usr/bin` 前面，顺序不同 |
| 只设 `MSYSTEM=MINGW64` | PATH 里不再有 `mingw64/bin`、`usr/bin`、`~/bin`：**git 只在 `MSYSTEM` 没设时才自己往 PATH 前面加** |
| 设 `MSYSTEM=MINGW64`，PATH 前面加 `<top>\mingw64\bin;<top>\usr\bin;<HOME>\bin;` | **逐行相同**。父进程有 `HOME` 与没有 `HOME` 两种都对比过 |

所以做法是：照启动器的样子设好 PATH 与 `MSYSTEM`，再起真正的 `git.exe`。`tests/git_cli.rs` 的 `the_git_we_start_sees_the_same_environment_as_the_one_on_path` 把这个对比钉在测试里，并断言开发机上真的绕过了（否则就是同一个程序跟自己比）。

只在认得出来时才绕（`git_program::plan_direct`），其余照旧起 `git`：PATH 上第一个 `git.exe` 在 `cmd` 目录里；`git --exec-path` 落在同一个安装的 `<prefix>\libexec\git-core`、前缀是 `mingw64` / `mingw32` / `clangarm64`；`<prefix>\bin\git.exe` 与 `usr\bin` 都在；起进程那一刻父进程没有 `MSYSTEM`。

为什么这么改是安全的：

- 起的是同一个安装里的同一个 `git.exe`，经过启动器时最终跑的也是它；子进程（钩子、凭据助手、ssh、过滤器）看到的环境与原来逐行相同。
- 认不出来的安装（scoop、Cygwin、PATH 上本来就是真正的 git、找不到 git）一律照旧，行为与改动前相同。`git --exec-path` 失败也照旧。
- 版本检查（`ensure_supported_git`）和 `--exec-path` 探测仍经过启动器，只在进程内各跑一次。
- PATH 与 HOME 每次起进程时现读，不缓存：进程环境变了，结果与启动器现算的相同。

计划没写的坑：

- **Git Bash 会设 `MSYSTEM`**。从 Bash 工具跑的测试、计时工具都不会走直连（这是有意的退路）；要验证直连必须在没有 `MSYSTEM` 的环境里跑（PowerShell，或计时脚本里 `env -u MSYSTEM`）。应用从资源管理器、开始菜单、安装器启动时没有它。
- 启动器在父进程**已有** `MSYSTEM` 时怎么处理没有实测，所以那种情况不绕（见上）。

### review 处理（`/code-review high`，2026-11-15）

| 条 | 问题 | 处理 |
|---|---|---|
| 1 | 预跑起在改写之前：之后任何一步本地出错，`RunAhead` 的 drop 要等预跑的 diff 跑完才报；离线时就是等一轮网络超时。改动前这些路径根本不发 diff | 记为已知代价（`prefetch_floor_diffs` 文档里写了）。只在「离线 + 录制中途本地出错」时多等；在线时多等约 1 s。drop 时不等的话，返回之后还有一个读着工作副本的 svn 进程（测试的临时目录这时正要删），与 `ScanAhead` 的取舍相同 |
| 2 | 后台的缓存复核与体检的 diff、`info -R` 抢 `wc.db` | 记为后续。svn 在同一工作副本上基本串行，复核挪到哪里都要花这些 svn 时间；只有挪到发布阶段的 git 工作期间（不碰 svn）才真的省，要把 `Saving` 带出录制、进 `Plan`，改动面大，另开一次 |
| 3 | `a_failed_run_ahead_hands_its_error_to_the_call` 只比错误码，真的重跑也是同一个错误码，是假绿 | **修**。预跑时文件未纳入版本控制、预跑后 `svn add`：真的重跑会成功，拿到失败只能是预跑的结果；再跑一次确认第二次是真的重跑。顺手把 `a_run_ahead_result_does_not_outlive_its_guard` 第二段只看文件名（`status -v` 无论改没改都会列）的断言改成看 `item="modified"` |
| 4 | `bypass_scan_cache` 不挡预跑结果：守卫活着时「专门读最新的」重读会拿到起跑时那份 | **修**。`bypass_scan_cache` 期间把预跑结果挪开、结束时放回。新增 `a_bypassed_read_does_not_take_a_run_ahead_result`（修之前红） |
| 5 | 两个守卫放同一条命令时互相覆盖、互相删 | **修**。`adopt` 只放进格子空着的、守卫只删自己放进去的。新增 `a_nested_run_ahead_leaves_the_outer_result_alone`（修之前红） |
| 6 | 预跑不过体检开头那三道检查（白名单为空、URL 对不上、白名单不在 svnUrl 底下），体检会直接 NotChecked 的时候也白问一次服务器 | **修**。预跑前照体检的顺序过这三道；两次 `svn info` 在扫描缓存范围里，体检随后命中缓存。`prefetch_floor_diffs` 改收 `WScope`，与体检用同一个白名单口径 |
| 7 | 直连时只拿 HOME 拼 PATH、没交给子进程；启动器会交 | **修**。照启动器把 HOME 设给子进程（git 不用再算一遍）。子进程环境逐行对比的测试仍通过 |
| 8 | 起不了写缓存的线程时整次不写，改动前总会写 | **修**。活放在共享的格子里，起不了线程就当场写 |
| 9 | 预跑键与扫描缓存键两份同样的拼法 | **修**。合成 `svn_cli::command_key`。三处「`JoinHandle` + drop 时 join」没有合并：形状各不相同（返回值、失败时的日志），合成泛型只省几行 |
| 10 | 预跑结果经 `run_svn` 里的线程局部通道交给体检，靠参数逐字相同对上；`predicted_listing` 又抄了一遍改写的筛法 | 不改。显式传递要改 `check_baseline_map_with_nodes` 的签名与 `Grouped`，牵动初始化、重建、更新、手动体检几个入口；现在预测错了的后果只是多跑一条 diff，预算测试按「整次跟进只有一条 `svn diff`」钉着，筛法一旦分叉会变红 |

### 没做的

| 项 | 为什么不做 |
|---|---|
| `git add` 排除脏文件（方案 D） | 见「方案取舍」。T3 之后再看收益 |
| 发布阶段 `blob_of`（`hash-object --no-filters --stdin`，不写对象库）改在进程内算 | 只省 2 个进程。它算的是写进 journal、供崩溃恢复比对的指纹：git2 的 `Oid::hash_object` 只算 SHA-1，SHA-256 仓库上与 CLI 不同；老版本写下的 journal 交给新版本恢复时也会对不上 |
| 两条大扫描（预跑的 `svn diff` 与 `svn info -R`）错开 | 并行时互相拖慢，但错开并不更快（串行 1.76 s，并行墙钟 1.75–1.81 s） |
| 省掉 `svn info -R`（约 0.75 s，只为找 `--set-depth=exclude` 的节点） | 没有更便宜的 svn 命令能列出被排除的节点；直接查 `wc.db` 的 `presence='excluded'` 要新增 SQLite 依赖并绑定 wc.db 的表结构，需要先征得用户同意 |
| blob 缓存写前复核改由锁内复核兜底、不再重读 `svn info` | 锁内复核只比节点 revision；`svn switch` 到同 revision 号的分支时内容变了、revision 没变，兜不住（`svn_base_blobs` 文档里的反例） |
