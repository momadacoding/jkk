# 性能记录模式实现计划

> 设计：`docs/superpowers/specs/2026-11-12-performance-monitor-design.md`（下称「设计」）。本计划只列落地顺序和每个 Task 的验收标准，做法以设计为准；实际偏离记在 `2026-11-12-performance-monitor-decisions.md`。

## Task 1：开关与记录文件（Rust）

- `src-tauri/src/engine/perf.rs`：`PerfLog`（按实例构造，每行 flush）、`Record`、全局 `install` / `enabled` / `record`、`perf_append` 的逐行校验。
- `lib.rs`：`run()` 里认 `--perf`；`setup` 里在 `app_log_dir()/perf/` 建文件、写 `meta` 头、标题加 ` [perf]`；单实例回调里看到对方带 `--perf` 而本进程没开时警告。
- `commands/perf.rs`：`perf_append`。
- 验收：Rust 单测覆盖参数识别、`Record` 字段名、坏行 / `src != "web"` 的行被丢弃。

## Task 2：子进程计时与脱敏（Rust）

- `proc.rs` 加计时包装；git / svn / gh 的 `output()` / `spawn()` 点改走它；`run_svn` 缓存命中记 `cached: true`。
- `launcher.rs` / `external_launch.rs` 只记 `mark`。
- 脱敏规则写成纯函数，按设计 §4.5 每条一个单测。
- 验收：集成测试在 fixture 仓库上跑一次成功、一次失败的 `run_git`，文件里两条 `proc`，`ok` 分别为真 / 假，不含 fixture 路径。

## Task 3：前端 invoke 包装与缓冲

- `src/engine/perfRecord.ts`：记录构造、数组长度摘要、缓冲切分（纯逻辑）。
- `src/store/perf.ts`：开关、缓冲、定时 flush、`perfSpan` / `perfMark`。
- `src/store/ipc.ts`：与 `@tauri-apps/api/core` 同签名的 `invoke`；9 个文件改导入。
- 验收：vitest 覆盖关闭时原样转发、打开时记录且结果 / 异常原样交回、摘要只取长度、缓冲切分（fake timers）。

## Task 4：第一批打点

- `handleCommit` 的 `commit` span 及子 span；`repo-changed` mark；窗口 `focus` / `blur` mark；`openRepo` span。
- 验收：`pnpm build`、`pnpm test` 通过；关闭时打点函数直接调用原函数。

## Task 5：汇总脚本、README、手测

- `scripts/perf-summary.mjs`：分组分位数表、`--around commit`、`--json`。
- README「排查」一节写用法（含 dev 下怎么传参，实测后写）。
- 验收：vitest 用手写 JSONL 测分组与时间窗；`pnpm tauri dev` 带 / 不带 `--perf` 各启动一次核对文件与标题。
