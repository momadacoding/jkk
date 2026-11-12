# 性能记录模式（performance monitor）设计

> 来源：2026-11-11 提交慢使用报告第 5 节第 2 条（「给 jkk 加 performance monitor 模式：启动时加命令行参数开启，默认关闭；记录每个 invoke 命令与子进程的耗时，写到文件」）。
> 2026-11-12 起草。标「我的决定」的是设计者替用户拿的主意，理由写在原处；标「实现者决定」的留给实现时定。

## 1. 为什么做

报告用内核进程事件（ETW）录到了提交前后起的每个子进程，但体感 2 s 里约 1.7 s 在 jkk 进程内部：libgit2 的读取、IPC 往返、单飞队列里的等待都不起子进程，ETW 看不到。报告剩下的三条假设都卡在这里：

| 假设 | 要回答的问题 | 需要的数据 |
|---|---|---|
| H2 | 等待是不是主要花在 `refreshChanges`（libgit2 status） | 提交流程每个阶段的耗时 |
| H3 | 后台 `svn status` 是否拖慢前台刷新 | 同一时间轴上的子进程和 invoke |
| H4 | 文件越多越慢是否成立、慢在哪个阶段 | 每次读取涉及的文件数 |

另外 ETW 录制本身成本高：要管理员权限、要 `logman` 脚本、要写解析脚本，PID 复用还会让耗时配错（报告第 6 节）。有了记录模式，以后每次排查只要「带参数启动 → 操作 → 跑汇总脚本」。

## 2. 目标 / 非目标 / 反目标

**目标**

- 启动参数 `--perf` 开启，默认关闭；关闭时开销可以忽略（§4.7）。
- 在**同一条时间轴**上记三类东西：子进程（git / svn / gh）、前端发出的每个 invoke 的往返、前端打点的业务阶段（提交流程的各步）。
- 每条记录带上能回答 H4 的数量（参数里的路径数、返回值里的列表长度），**不带**文件路径、提交说明、仓库内容（§4.5）。
- 一个汇总脚本：按名称出 p50 / p95 / 最大值，并把每次提交前后几秒的记录排成时间线。

**非目标**

- 界面上的开关、实时面板、火焰图。
- libgit2 内部的细分耗时（`read_status` 内部哪一步慢）。invoke 往返已经能定位到「哪个命令慢」，再往下拆等真有需要时在那个命令里加打点。
- 长期运行的外部进程（外部 diff / merge 工具、编辑器）的耗时：它们由用户操作决定，与延迟无关，只记启动。
- 自动上传或发送记录文件。

**反目标**（做出来也算失败）

- 关闭时改变任何行为或时序：不开 `--perf` 的进程，与现在逐字节同样地起子进程、发 invoke。
- 记录文件里出现提交说明、文件路径、远程地址、文件内容（AGENTS.md §11 的仓库内容保护）。
- 记录本身拖慢被测对象：写文件不能放在 invoke 的返回路径上同步等待。

## 3. 已确认的事实

| 事实 | 出处 |
|---|---|
| 所有子进程都经 `engine::proc::hidden_command` 构造；真正 `output()` / `spawn()` 的点在 6 个文件里约 12 处：`git_cli.rs`（`git --version` 探测、`run_git_code`、`run_git_bytes` 等、`run_git_stdin_partial` 的 spawn）、`svn_cli.rs`（`run_svn`、`run_svn_bytes`、`run_svn_code`、版本与注册表探测）、`svn_follow_mark.rs`、`github.rs`、`launcher.rs`、`external_launch.rs` | 2026-11-12 grep |
| `run_svn` 有扫描结果缓存（`cached_scan`），命中时不起进程 | `svn_cli.rs:34` |
| 前端直接调 `invoke` 的有 9 个文件：`repoStore.ts`（54 处）、`RepoTab.tsx`（19）、`externalToolsStore.ts`、`appSettingsStore.ts`、`customCommandsStore.ts`、`App.tsx`、`FileContentView.tsx`、`DeleteBranchDialog.tsx`、`CommitDetailView.tsx` | 2026-11-12 grep |
| `main.rs` 在进 Tauri 之前先分流 TortoiseSVN 钩子调用（同一个 exe），参数解析要在它之后、`app_lib::run()` 里做 | `src-tauri/src/main.rs` |
| release 构建注册了单实例插件：jkk 已在运行（包括关到托盘）时再启动一个 `jkk.exe --perf`，只会唤起已有窗口然后退出，记录模式**不会**打开。dev 不注册单实例 | `src-tauri/src/lib.rs` |
| Tauri 2 的 `app_log_dir()` 在 Windows 上是 `local_data_dir/<identifier>/logs`，即 `%LOCALAPPDATA%\com.momadacoding.jkk\logs`；dev 的 identifier 是 `com.momadacoding.jkk.dev` | Context7 `/websites/rs_tauri` `PathResolver::app_log_dir`；`dev_profile.rs` |
| `tauri-plugin-log` 只在 debug 构建装，release 没有日志文件 | `src-tauri/src/lib.rs` 的 `setup` |

## 4. 设计

### 4.1 开关

- `app_lib::run()` 里读 `std::env::args()`，含 `--perf` 就打开。**我的决定**：只认命令行参数，不加环境变量、不加设置项。报告要的就是「启动时加参数」；设置项会让记录模式在用户忘记时长期开着。
- 打开后：
  - 在 `app_log_dir()/perf/` 下新建 `perf-<YYYYMMDD-HHMMSS>-<pid>.jsonl`，每次启动一个新文件。
  - 窗口标题末尾加 ` [perf]`，一眼看得出开着。
  - 第一行写一条 `meta` 记录：jkk 版本、构建类型、启动参数里除 `--perf` 外的个数、CPU 核数。
- 单实例冲突：单实例回调里如果看到第二个实例的参数含 `--perf` 而本进程没开，`log::warn!` 一条，并在窗口上弹一次错误条「记录模式要先完全退出 jkk（包括托盘）再带 `--perf` 启动」。**我的决定**：不在运行中途打开记录。中途打开的文件缺了打开仓库那一段，而且要在运行时切换全局状态，得不偿失。

### 4.2 记录格式

JSONL，一行一条，字段固定：

| 字段 | 类型 | 说明 |
|---|---|---|
| `ts` | number | 开始时刻，Unix 毫秒，带小数 |
| `dur` | number | 耗时毫秒，带小数；`mark` 为 0 |
| `src` | `"rust"` \| `"web"` | 谁记的 |
| `kind` | `"proc"` \| `"invoke"` \| `"span"` \| `"mark"` \| `"meta"` | 见下 |
| `name` | string | 见下 |
| `ok` | boolean | 失败（非零退出、invoke 抛错、阶段抛错）为 `false` |
| `meta` | object | 可选，只放数字、布尔和短的固定字符串 |

- `proc`：`name` 是「程序 + 子命令」，如 `git status`、`svn status`。`meta`：`flags`（脱敏后的选项，§4.5）、`argc`、`exit`（退出码）、`cached`（`run_svn` 缓存命中时为 `true`，不起进程，`dur` 约 0）、`repo`（工作目录最后一段）。
- `invoke`：`name` 是命令名。`meta`：参数里每个数组字段的长度（如 `{ "paths": 3 }`）；返回值若是对象，它每个顶层数组字段的长度（如 `read_status` 的 `{ "staged": 4, "unstaged": 120 }`、`load_graph` 的 `{ "commits": 2000 }`）。只取 `.length`，不序列化返回值。
- `span`：前端打点的业务阶段，名字用点分层级，如 `commit`、`commit.freshSvnSync`、`commit.invoke`、`commit.refreshChanges`、`commit.refreshGraph`、`commit.noteOwnCommit`。`meta` 放阶段自己的数量（`commit` 的 `staged` 个数、是否 `amend`）。
- `mark`：瞬时事件，用来对时间线，如 `repo-changed`（文件监听到达）、`focus`、`blur`。
- `meta`：文件头一条（§4.1）。

两边的时间基准：Rust 用 `SystemTime::now()`，前端用 `performance.timeOrigin + performance.now()`。同一台机器上两者都是墙钟，误差在毫秒以内，够排时间线。**我的决定**：不做时钟对齐握手。

### 4.3 Rust 侧

- 新模块 `src-tauri/src/engine/perf.rs`：
  - 全局 `OnceLock<Option<PerfLog>>`。`PerfLog` 持有 `Mutex<BufWriter<File>>`，每写一行就 flush，崩溃时不丢已记的。
  - `perf::enabled() -> bool`：关闭时就是一次 `OnceLock::get`。
  - `perf::record(rec: Record)`：关闭时直接返回。
- 子进程计时：在 `proc` 里加一个包装（形状实现者决定，例如 `proc::timed_output(&mut Command) -> io::Result<Output>`，或者一个 drop 时写记录的守卫）。约 12 个 `output()` 点改成走它；`run_git_stdin_partial` 那个 spawn 从 spawn 计到 `wait` 结束。程序名和参数从 `Command::get_program()` / `get_args()` 取，调用点不必传。
- `launcher.rs` / `external_launch.rs`（编辑器、外部工具）只记一条 `mark`（`launch <程序名>`），不计时。
- 新命令（都在 `commands/perf.rs`）：
  - `perf_status() -> { enabled, file }`：前端启动时问一次。
  - `perf_append(lines: Vec<String>)`：前端批量写入。后端逐行校验是合法 JSON 且 `src == "web"` 再写，坏行丢弃。
- 不给每个 `#[tauri::command]` 套计时。**我的决定**：前端的 invoke 往返已经覆盖「命令本身 + IPC + 线程池排队」。线程池被占满时，往返会明显长于子进程耗时之和，这本身就是要看的信号。

### 4.4 前端

- 新模块 `src/store/ipc.ts`，导出与 `@tauri-apps/api/core` 同签名的 `invoke`：
  - 关闭时直接转发，只多一次布尔判断。
  - 打开时记一条 `invoke` 记录，结果和异常原样交回。
  - 9 个文件把 `import { invoke } from '@tauri-apps/api/core'` 改成从 `@/store/ipc` 导入，调用点一个字都不改。现有测试 mock 的是 `@tauri-apps/api/core`，包装层底下调的就是它，mock 照样生效。
- 新模块 `src/engine/perfRecord.ts`（纯逻辑，可单测）：记录对象的构造、参数与返回值的数量摘要、批量缓冲的切分规则。
- `src/store/perf.ts`：
  - 启动时调一次 `perf_status`；没回答之前先缓冲，最多 500 条。回答「关」就清空缓冲，之后不再缓冲。
  - `perfSpan(name, meta, fn)` 包一段异步过程；`perfMark(name, meta)` 记瞬时事件。
  - 每 1 s 或攒满 100 条调一次 `perf_append`。这次调用本身不记录，也不 await 在业务路径上。窗口关闭前尽量 flush 一次，丢了也不影响业务。
- 第一批打点（实现者可以加，不要减）：
  - `handleCommit`：整体一个 `commit` span，里面 `freshSvnSync`、`invoke`、`noteOwnCommit`、`refreshChanges`、`refreshGraph` 各一个子 span。
  - `repo-changed` 监听入口一个 `mark`。
  - 窗口 `focus` / `blur` 各一个 `mark`。
  - `openRepo` 整体一个 span。

### 4.5 脱敏

子进程参数是唯一可能带用户内容的地方。规则写成纯函数，Rust 侧单测覆盖：

- 保留：程序名、子命令（第一个不以 `-` 开头的参数）、以 `-` 开头的选项。`--key=value` 只留 `--key`。
- 丢掉值、只计数：其余所有参数，包括路径、ref 名、提交说明。`-m` / `--message` / `-F` / `--file` 后面那一个参数一律丢掉，即使它看起来像选项。
- `repo` 只取工作目录最后一段，用来区分同时开的几个 worktree，不记完整路径。
- 前端 `invoke` 记录只记参数的数组长度，不记参数值；返回值同理。

### 4.6 汇总脚本

`scripts/perf-summary.mjs <file.jsonl>`，只用 Node 内置模块：

- 默认输出：按 `kind + name` 分组的 count / p50 / p95 / max / total 表，按 total 降序。
- `--around commit`：对每个 `commit` span，列出它开始前 0.5 s 到结束后 3 s 内的所有记录，时间换算成相对 span 开始的毫秒，按开始时刻排序，缩进显示 span 层级。这就是报告第 2 节那张表的进程内版本。
- `--json`：把汇总结果按 JSON 输出，方便贴进报告或喂给别的脚本。

### 4.7 开销

- 关闭时：Rust 每次起子进程多一次 `OnceLock::get`；前端每次 invoke 多一次布尔判断。相对一次子进程（几十毫秒）或一次 IPC（亚毫秒到毫秒），可以忽略。
- 打开时：每条记录一行几百字节的 JSON。按报告的节奏（空闲每 5 分钟一轮核对，几个 svn 进程；每次提交几十条 invoke），一小时在 MB 量级。Rust 侧每行 flush 一次；前端批量发，不在 invoke 的返回路径上写盘。

## 5. 用法（固化进 README 的「排查」一节）

1. 完全退出 jkk，包括托盘。
2. 带参数启动：`jkk.exe --perf`（便携版在 PowerShell 里运行 `& 'E:\jkk_0.8.0_x64-portable\jkk.exe' --perf`，或建一个目标里带 `--perf` 的快捷方式）。看到标题末尾的 `[perf]` 说明开着。
3. 正常操作：提交、切分支、等待空闲核对。
4. 退出 jkk。记录在 `%LOCALAPPDATA%\com.momadacoding.jkk\logs\perf\`，每次启动一个文件。
5. `node scripts/perf-summary.mjs <文件> --around commit`。

dev 下怎么传参：`pnpm tauri dev` 把哪一段参数交给应用进程，实现时实测后写进 README，这里不预设。

## 6. 测试

| 层 | 测什么 |
|---|---|
| Rust 单测 | 脱敏规则（§4.5 每条一个用例，含 `-m` 后跟以 `-` 开头的说明、`--format=%H`、Windows 路径）；`Record` 序列化的字段名；`perf_append` 丢坏行、丢 `src != "web"` 的行 |
| Rust 集成测试 | 打开记录到临时文件，在 fixture 仓库上跑一次 `run_git` 和一次失败的 `run_git`，文件里出现两条 `proc`，`ok` 分别为 `true` / `false`，参数里没有 fixture 的路径。`PerfLog` 要能按实例构造，测试不碰全局状态 |
| vitest | `ipc.ts`：关闭时原样转发、不记录；打开时记录，结果和异常原样交回。数量摘要只取数组长度。缓冲的切分（1 s / 100 条 / 状态未知时最多 500 条、状态为关时清空），用 fake timers。`perf-summary.mjs` 的分组与 `--around` 窗口用一份手写的小 JSONL 喂 |
| 手测 | release 构建带 `--perf` 启动：标题有 `[perf]`，提交一次后文件里有 `commit` span、`commit` / `read_status` invoke、`git commit` proc；不带参数启动：文件不生成、标题没有 `[perf]` |

## 7. 任务拆分

| Task | 内容 | 估时 |
|---|---|---|
| 1 | `perf.rs`、开关、标题后缀、`perf_status` / `perf_append`、Rust 单测 | 2 h |
| 2 | 子进程计时：约 12 个 spawn 点 + 脱敏 + 集成测试 | 2 h |
| 3 | `ipc.ts` 包装、9 个文件改导入、`perf.ts` 缓冲与 flush、vitest | 2 h |
| 4 | 第一批打点（§4.4）| 1 h |
| 5 | `perf-summary.mjs` + README「排查」一节 + 手测 | 2 h |

合计约一天。走 AGENTS.md §2.1：开工前在本目录配套 `docs/superpowers/plans/` 的计划和 `-decisions.md`。

## 8. 拿掉的方案

| 方案 | 为什么不选 |
|---|---|
| 环境变量 `JKK_PERF=1` 开启 | 报告要的是启动参数；环境变量对便携版用户不比快捷方式方便，还会被子进程继承 |
| 设置界面里的开关 | 要做 UI；容易忘了关，长期开着 |
| 给每个 Tauri 命令在 Rust 侧计时 | 前端往返已覆盖，且多出 IPC 与排队这层信息；Rust 侧要改 `generate_handler!` 的包装，改动面大 |
| 用 `tracing` crate 做 span | 新依赖（AGENTS.md §1.2 要先征得同意）；现有需求是十几个点的计时，手写足够 |
| 前端只在 store 里计时，不收口 `invoke` | `RepoTab.tsx` 里有 19 处直接调用，其中就有 `commit`，漏掉它就回答不了 H2 |
