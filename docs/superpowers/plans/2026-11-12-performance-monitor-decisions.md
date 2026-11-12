# 性能记录模式实现日志

计划：`2026-11-12-performance-monitor.md`；设计：`../specs/2026-11-12-performance-monitor-design.md`。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 开关与记录文件 | 完成 | `engine/perf.rs`、`commands/perf.rs`、`lib.rs` | `cargo test --lib engine::perf` 17 通过 |
| 2 子进程计时与脱敏 | 完成 | `proc::output`、git_cli / svn_cli / svn_follow_mark / github / launcher / external_launch、`tests/perf_log.rs`、`tests/no_console_window.rs` 新守卫 | `perf_log` 1 通过；`no_console_window` 2 通过；clippy / fmt 干净 |
| 3 前端 invoke 包装与缓冲 | 完成 | `engine/perfRecord.ts`、`store/perf.ts`、`store/ipc.ts`、9 个文件改导入、`main.tsx` | `perfRecord.test.ts` 9、`perf.test.ts` 12 通过 |
| 4 第一批打点 | 完成 | `RepoTab.tsx` handleCommit、`repoStore.ts` openRepo / repo-changed | `pnpm build` 通过；`pnpm test` 1560 通过 |
| 5 汇总脚本、README、手测 | 完成 | `scripts/perf-summary.mjs`、`perfSummary.test.ts`、README「排查：性能记录模式」 | vitest 6 通过；`pnpm test` 92 文件 1566 通过；`pnpm tauri dev` 带 / 不带 `--perf` 各跑一次（见下） |

## 独立 review（`/code-review high`，2026-11-12）

10 条，9 条照改，1 条部分采纳。

| 条目 | 问题 | 处理 |
|---|---|---|
| 第 1 条 | 前端 `ts` 用 `performance.timeOrigin + now()`，单调钟，休眠唤醒 / 校时后与 Rust 的墙钟漂开，`--around` 窗口错位 | 改：`ts` 取 `Date.now()`，`dur` 取 `performance.now()` 差值。单测「墙钟中途跳一分钟，dur 不受影响」 |
| 第 2 条 | 带 `--perf` 但文件没建成时，前端照样计时、每秒发一次没人收的 IPC | 改：`perf_append` 返回记录器在不在，前端拿到 `false` 就 `disable()`；`ipc.ts` 按调用时的开关走。单测覆盖 |
| 第 3 条 | `open_perf_log` 在日志插件装好之前跑，release 也不装，失败的 `warn!` 没人看见 | 改：失败时弹不阻塞的警告框，写明原因 |
| 第 4 条 | 托盘「退出」走 `app.exit(0)`、关窗藏到托盘，`pagehide` 不触发，最后一批（含 `commit` span）丢失 | 改：`blur`、`visibilitychange` 也 flush。点托盘之前窗口必然先失焦 |
| 第 5 条 | `ok` 只看抛没抛：`handleCommit` 失败返回 `false`，`openRepo` 内部吞错，都记成成功 | 改：`span` 加 `okOf`，`commit` 按返回值；`openRepo` 不改返回类型（store 公共接口），在 catch 里打 `openRepo.error` mark |
| 第 6 条 | `pipe_through` 拿不到 stdin 时提前返回，计时器没结账、子进程没收 | 改：kill + wait 后照常结账再返回 |
| 第 7 条 | 守卫按文件放行 `spawn()`，同文件再加一个看不出；`.status()` 不在扫描范围 | 部分采纳：改成按次数放行（每个文件 1 次）。`.status()` 不扫：libgit2 的 `delta.status()` 同名，按行匹配会误报；目前非测试代码没有 `Command::status()`，盲区写在守卫注释里。「让 `hidden_command` 返回只暴露计时方法的包装类型」改动面太大，不在本 PR |
| 第 8 条 | `append_web` 每行一锁一 flush | 改：整批一把锁、一次 flush |
| 第 9 条 | README 新章节说单实例，上面「多开」一行还写着「没有做单实例」 | 改：那一行改成现状（安装版 / 绿色版单实例，dev 不做） |
| 第 10 条 | 汇总脚本不校验 `kind`，缺 `kind` 的行会让格式化崩掉 | 改：缺 `kind` 计入坏行，单测覆盖 |

### review 修完后的复验

- 全量 `cargo test --no-fail-fast`：94 个套件，1926 通过、1 失败。失败的是 `svn_write_guard::every_command_outside_the_vcs_trait_is_classified`：新命令 `perf_append` 没归类。**计划没写的坑**：每个新 Tauri 命令都要在 `tests/svn_write_guard.rs` 的 `NOT_GIT_ONLY` 里写明能不能在 SVN 混合仓库上跑。已加（整个 `perf.rs` 放行：只写日志目录，不碰仓库），该测试 9 个通过。
- 同一轮里 `commands::watch::tests::stage_and_unstage_only_touch_the_index` 通过了。它在上一轮（未加 `--no-fail-fast`）失败过一次，与 PR 156 记录的是同一条不稳定测试（多收到一条 `.git/hooks` 事件），本 PR 没改 `watch.rs`。
- 第二次手测（review 修改之后，同样的隔离做法）：提交成功，标题有 `[perf]`，文件里没有说明、路径、文件名；`blur` mark 在提交后 216 ms 就进了文件（失焦即 flush 生效）。
- **计划没写的坑**：`ts` 改成 `Date.now()` 之后是整毫秒，`dur` 仍是 `performance.now()` 的小数差值，子段的结束时刻可能比父段晚出不到 1 ms；开始时刻和耗时都相同的 `commit.invoke` 与 `invoke commit` 的先后也不定。汇总脚本原来把 `commit.refreshChanges` 算到了 `commit` 外面、把 `invoke commit` 排在 `commit.invoke` 前面。改：判断「包住」时结束时刻留 1 ms 余量；同时刻同耗时按 span → invoke → proc → mark 排；只有排在前面的 span 才算包住它。用这次手测的真实数值补了单测。

## Task 5 手测记录（2026-11-12，开发机）

- 隔离：`pnpm tauri dev --config <json>`，identifier 换成 `com.momadacoding.jkk.perftest`，session.json 指向 scratchpad 里的临时 fixture 仓库（`b.txt` 已暂存）。核对过 `app.exe` 在本 worktree 的 `src-tauri\target\debug\` 下。验完删掉了两个配置目录。
- 带 `-- -- --perf`：标题 `jkk 0.8.0 (...) [dev · perf-monitor] [perf]`；`logs\perf\perf-20261112-152058Z-<pid>.jsonl` 生成，首行 `meta`。经 CDP 填说明、点 Commit，fixture 里多了一个提交。`--around commit` 的时间线：`commit` 102.8 ms ⊃ `commit.invoke` 91.9 ⊃ `invoke commit` 91.8 ⊃ `proc git commit` 89.1；随后 `commit.refreshGraph` 5.7、`commit.refreshChanges` 5.2；+413 ms 处 `repo-changed` 及其后的一轮刷新与 git 子进程。文件里搜不到提交说明、fixture 路径、`b.txt`、`Users`。
- 不带参数：标题没有 `[perf]`，没有新文件，`window.__JKK_PERF__` 为 `undefined`。
- **计划没写的坑**：
  - Windows 上一个进程还开着的文件，目录列表里的大小可能是 0（元数据没刷进目录项），读内容才是准的。手测时一度以为没写进去。
  - dev 下 `openRepo` span 与 `load_session` 各出现两次：React StrictMode 在开发构建里会把 effect 跑两遍，release 不会。
  - `pnpm tauri dev --config X -- -- --perf` 实测能把 `--perf` 交到应用（设计 §5 留的待实测项）。

## 开工前定下的偏离

### D1 前端靠初始化脚本得知开关，不调 `perf_status`

- **偏离计划**：设计 §4.3 / §4.4 是前端启动时调 `perf_status`，回答之前最多缓冲 500 条。改为：`--perf` 打开时 Rust 注册一个只带 `js_init_script` 的插件，页面脚本执行前设好 `window.__JKK_PERF__ = true`；前端模块加载时读一次。`perf_status` 命令不做。
- **为什么**：
  - 没有「状态未知」的窗口，关闭时从第一条 invoke 起就是原样转发，反目标「关闭时不改变时序」不再有例外。
  - 测试环境里这个全局量不存在，等于关闭；现有测试的 invoke 次数与时序不受影响。按设计的做法，没调 `perf_status` 的测试会一直停在「未知」，每次 invoke 都被包一层 `.then`。
  - 前端用不到文件路径，路径写在 README 里，`meta` 头里也有。
- **API 形状**：Tauri 2.12.5 `plugin::Builder::js_init_script(impl Into<String>)`，`for_main_frame_only: true`（`~/.cargo/registry/.../tauri-2.12.5/src/plugin.rs:371`）。插件不带命令，不需要 capability 权限。

### D2 文件名时间用 UTC，带 `Z`

- **偏离计划**：设计写 `perf-<YYYYMMDD-HHMMSS>-<pid>.jsonl`，没说时区。实际是 `perf-20261112-031500Z-1234.jsonl`。
- **为什么**：取本地时间要么加 `chrono` / `time`（新依赖要先征得同意），要么给 `windows` crate 开新 feature 调 `GetLocalTime`。文件名只用来排序和区分，UTC 足够；带 `Z` 免得被当成本地时间。记录里的 `ts` 本来就是 Unix 毫秒。

### D3 集成测试用全局记录器

- **偏离计划**：设计 §6 说「`PerfLog` 要能按实例构造，测试不碰全局状态」。`PerfLog` 确实按实例构造、单测都按实例测；但集成测试要验证 `run_git` 真的写了记录，`run_git` 读的是全局记录器。
- **为什么是安全的**：`src-tauri/tests/` 下每个文件是独立的测试二进制、独立进程。新测试文件只有一个测试装全局记录器，影响不到别的测试二进制。把记录器一路传进 `run_git` 要改几百个调用点，不值得。

### D4 `commit.noteOwnCommit` 子 span 暂不加

- **偏离计划**：设计 §4.4 第一批打点含 `commit.noteOwnCommit`。
- **为什么**：`noteOwnCommit` 在 PR 156（`worktree-commit-latency`）里，本分支从 master 切出，还没有它。两个 PR 都改 `handleCommit`，后合并的那个负责补上这个子 span。

### D5 脱敏规则比设计多三条

- **计划没写的坑**：
  - git 的 `-c <k=v>`、`-C <dir>`、`--git-dir <dir>`、`--work-tree <dir>` 写在子命令**之前**，后面跟一个值。不跳过的话，值会被当成子命令（`git -c core.quotepath=false status` 的名字变成 `git core.quotepath=false`）。这几个只在子命令之前当作带值的选项；子命令之后 `-C` 是 `log -C`（找复制）这类不带值的开关。
  - 短选项只保留前两个字符（`-uno` → `-u`）：短选项可以把值粘在后面（`-mfoo`、`-Sfoo`），不截断就会把值带出去。
  - `--` 之后的参数全是路径，只计数。
- 子命令只接受 ASCII 字母、数字和 `-`，最长 32 字符，否则记成 `?`：防止哪个调用点把用户内容放在第一个位置参数上。

## Task 1–4 实现中的决定

### D6 单实例冲突用原生提示框，不用错误条

- **偏离计划**：设计 §4.1 是「在窗口上弹一次错误条」。改为单实例回调里用 `tauri_plugin_dialog` 的 `message(...).kind(Warning).show(..)`。
- **为什么**：回调在 Rust 侧，错误条要新开一个事件、前端再接一个监听，只为一个极少出现的提示。对话框插件本来就在依赖里，`show` 不阻塞。
- **计划没写的坑**：单实例插件交给回调的 `args` 是第二个实例的 `std::env::args()` 全量，**含 argv[0]**（`tauri-plugin-single-instance-2.4.5/src/platform_impl/windows.rs:91`），所以和 `run()` 共用 `flag_in`（跳过第一个）是对的。

### D7 invoke 的数量键带 `in.` / `out.` 前缀

- **偏离计划**：设计 §4.2 举例是 `{ "paths": 3 }`、`{ "staged": 4 }`。实际是 `in.paths`、`out.staged`；返回值本身是数组时记 `out.length`。
- **为什么**：参数和返回值可能有同名数组字段，不加前缀会互相覆盖。

### D8 打点函数关闭时不多包一层 Promise

- **计划没写的坑**：`perfSpan` 写成 `async` 函数的话，关闭时 `return fn()` 也会被包进一个新 Promise，调用方多等一拍——违反反目标「关闭时不改变时序」。改成普通函数，关闭时返回的就是 `fn()` 的那个 Promise，有单测钉住（`toBe(inner)`）。`ipc.ts` 同理：关闭时用 `...call` 原样转发，参数个数不变（现有测试断言 `toHaveBeenCalledWith('load_session')`，补 `undefined` 会让它们失败）。

### D9 `handleCommit` 多一个 `commit.confirm` 子 span

- **偏离计划**：设计没列。SVN 仓库「还没跟进就提交」的确认框会等用户，计进 `commit` 而不单独拆出来，整段耗时会被撑大还看不出原因。按设计「实现者可以加，不要减」加上。

### D10 `openRepo` / `handleCommit` 的函数体没有重新缩进

- 包进 `perfSpan(..., async () => { ... })` 时只改首尾两行，函数体保持原缩进。仓库没有格式化工具，重新缩进会让 diff 盖住整段、和 PR 156 的 `handleCommit` 改动大面积冲突。

### 为什么这么改是安全的

- 子进程：`proc::output` 关闭时就是 `cmd.output()` 前多一次 `OnceLock::get`；`pipe_through` 只把 `spawn()` 拆成 match，失败路径仍返回同一个 `GitError::from(io::Error)`。
- `run_svn` 缓存命中时只在开着的时候多构造一个 `Command` 读参数，不起进程。
- 前端：没有注入开关的环境（普通启动、vitest）走的路径与原来逐字节相同，由 `perf.test.ts` 的「原样转发」用例守住；`src/` 下再有文件直接从 `@tauri-apps/api/core` 取 `invoke` 会让 `perf.test.ts` 失败。
