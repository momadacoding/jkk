# 工具栏自定义命令 实现决策日志

计划：`docs/superpowers/plans/2026-08-09-custom-toolbar-commands.md`
规格：`docs/superpowers/specs/2026-08-09-custom-toolbar-commands-design.md`
worktree：`.claude/worktrees/custom-toolbar-commands`，分支 `worktree-custom-toolbar-commands`，基点 `a0e1bb3`

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 准备 worktree | 完成 | 本文件 | 切入确认：`pwd` 在 worktree 内，基点 `a0e1bb3` |
| 1 配置解析 | 完成 `fc808c2` | `engine/custom_config.rs`、`tests/custom_commands.rs` | `cargo test --test custom_commands` 7/7；fmt + clippy 干净 |
| 2 launcher | 完成 `47653f7` | `engine/launcher.rs` | 同上测试文件 13/13；fmt + clippy 干净 |
| 3 Tauri 命令层 | 未开始 | | |
| 4 前端纯逻辑 | 未开始 | | |
| 5 前端接线与 UI | 未开始 | | |
| 6 真机验证与文档 | 未开始 | | |

## 决策条目

### Task 0：准备 worktree

**计划的错误**

计划 Step 4 要求检查 `.claude/worktrees/` 是否在 `.gitignore` 中，并称需要补上——
这条来自 AGENTS.md §4.1 的「该路径目前**不在** `.gitignore` 中」。**实际已经在了**，
`.gitignore:2` 就是 `.claude/worktrees/`（`.worktrees/` 在第 1 行）。
Step 4 是 no-op，没有改动 `.gitignore`。

AGENTS.md 里那句话已过时。本次不顺手改 AGENTS.md——那是规范文件，
改它超出本 PR 范围，留作后续单独处理。

**计划没写的坑**

`git check-ignore .claude/worktrees` **在 worktree 内部跑会给出误导性的答案**。

`.gitignore` 里的模式带尾部斜杠（`.claude/worktrees/`），只匹配目录。
而当前 session 的工作目录是 `.claude/worktrees/custom-toolbar-commands`，
在它内部并不存在 `.claude/worktrees` 这个路径，git 无法确认它是目录，
带斜杠的模式便匹配不上，`check-ignore` 返回「未忽略」。

回到主工作区跑同一条命令才得到正确结果：

```
.gitignore:2:.claude/worktrees/	.claude/worktrees
```

结论：**验证 `.gitignore` 规则必须在该路径真实存在的工作区里做**，
不能在一个恰好把该路径当作自身父目录的 worktree 里做。

**偏离计划**

无。

**为什么这么改是安全的**

Task 0 除新增本文件外没有任何改动，不涉及 git 写操作，无风险。

### Task 1：配置解析

**偏离计划**

无。实现与计划中的代码一致。

**计划的错误**

计划 Step 4 预留了一段应急处理：担心 `ConfigFile.version` 被 clippy 报
「字段从未读取」，要求二选一并记录选择。**该告警没有出现**——`version` 参与
`Serialize` / `Deserialize` derive，算被使用。没有加任何 `#[allow(...)]`。

**计划没写的坑**

无。

**为什么这么改是安全的**

`load_from` 的失败路径不写文件，这一点由
`broken_json_errors_and_leaves_file_untouched` 钉死：它按**字节**比较解析前后的
文件内容，而不是只断言返回了 Err。用户手写配置出错时，最坏结果是这一次读不到命令，
绝不会丢失已有配置。

所有测试都用 `tempfile::TempDir` 建临时目录，没有任何一条触碰真实的 app config dir
或本仓库。

### Task 2：launcher

**计划没写的坑（一）——探针脚本本身写错，差点误判 Rust 的行为**

计划 Task 2 Step 2 要实测「Rust 1.77.2 起 std 对 `.bat`/`.cmd` 自动经 cmd.exe
启动并转义参数」。第一次实测**失败**：

```
'c' is not recognized as an internal or external command
```

看上去像是假设不成立、需要退回自己拼 `cmd /c`。但根因是**计划里给的探针脚本写错了**，
不是 Rust 的问题。原脚本用 `echo.%~1`：

- `%~1` 会剥掉参数外层的引号
- batch 的参数展开发生在该行被**解析之前**
- 于是 `a b&c` 展开后，`&` 被当成命令分隔符，cmd 去执行 `c`

改用保留引号的 `%1` 后测试通过，`out.txt` 内容为 `"a b&c"`。

**结论：假设成立。** Rust std 确实把 `.cmd` 经 cmd.exe 启动，并让含空格与 `&` 的
参数作为单个 argv 完整送达。`launcher.rs` 因此**不需要**自己构造 `cmd /c`，
`resolve_program` 拿到带扩展名的路径后直接交给 `Command::new` 即可。

教训：一条失败的实测要先确认「测的东西错了」还是「被测的行为错了」。
差一点就为了迁就一个写错的探针，去实现更复杂也更危险的手工 cmd 转义。

**计划没写的坑（二）——实测工具链高于 MSRV**

实测在 `rustc 1.95.0` 上完成，而 `Cargo.toml` 声明的 MSRV 是 `1.77.2`。
该转义修复（CVE-2024-24576）文档记载正是落在 1.77.2，与我们的下限**恰好相同**，
但**没有在 1.77.2 工具链上直接验证过**。

风险评估：低但不为零。若将来真有人在 1.77.2 上构建，且该修复的行为在此后版本有过
调整，`rust_std_escapes_arguments_for_batch_files` 这条测试会直接失败并指出问题——
它测的是 std 的行为而不是我们的代码，正是为此而写。

**偏离计划**

一处：计划里 `resolve_program_reports_a_missing_executable` 没有加 `#[cfg(windows)]`，
但计划正文同时又说明「这条在非 Windows 上会失败，需要加 cfg」。实现时按正文说明
加了 `#[cfg(windows)]`。非 Windows 平台的同一条错误路径由
`spawn_detached_reports_a_missing_executable` 覆盖（该条无 cfg，两个平台都跑）。

**为什么这么改是安全的**

1. **不经 shell**是这个功能的核心防线。`spawn_detached` 把 `args` 以数组形式交给
   `Command::args`，直传 CreateProcess / execvp。仓库路径里的空格、`&`、中文
   不会被解析成新命令。`spawn_detached_runs_with_the_given_args_and_cwd` 用真实的
   `git init --quiet probe` 验证 argv 与 cwd 双双生效。
2. **不 wait 但起线程收尸**：不 `wait` 是必须的（等编辑器退出会卡死 UI），
   但 Unix 上不回收会留下僵尸进程。后台线程只阻塞在 `wait` 上，不占 CPU、
   不挡调用方；每次启动多一个线程，考虑到点击频率是人手速级别，可接受。
3. 本 Task 的测试**不涉及任何 git 写操作**，`git init` 建的是 `TempDir` 里的全新仓库，
   随 `TempDir` 析构一并清理，不碰本仓库也不碰用户仓库。
