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
| 3 Tauri 命令层 | 完成 `d20fca1` | `commands/custom.rs`、`commands/mod.rs`、`lib.rs` | `cargo test` 全套通过；fmt + clippy 干净 |
| 4 前端纯逻辑 | 完成 `c09bb08` | `src/engine/customCommands.ts`、`tests/vitest/customCommands.test.ts` | `pnpm test` 159/159（新增 12）；`pnpm build` 通过 |
| 5 前端接线与 UI | 完成 `1bad363` | `customCommandsStore.ts`、`CustomCommandBar.tsx`、`Toolbar.tsx`、`RepoTab.tsx`、`styles.css` | `pnpm build` + `pnpm test` 通过 |
| 6 真机验证与文档 | 部分完成 | `README.md`、`e2e/2026-08-09-*.md` | 9 条 E2E 中 2 条已验证（1 条真机观测 + 1 条自动化），**7 条待人工点击** |

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

### Task 3：Tauri 命令层

**偏离计划**

一处：计划给的 `lib.rs` 补丁片段用了 6 空格缩进，实际文件是 12 空格
（`1864d60` 全仓 cargo fmt 之后的形态）。按文件实际缩进插入，`cargo fmt --check` 通过。

**计划的错误**

无。

**计划没写的坑**

无。

**为什么这么改是安全的**

`open_custom_commands_config` **不接受路径参数**（这一点在写计划时就把 spec 从
`open_in_default_app(path)` 收窄成了现在的形态）。它自己解析配置目录、
确保文件存在，再交给系统默认程序打开。前端因此没有能力让系统去打开一个
它自己拼出来的任意路径——把「用默认程序打开任意路径」这个能力挡在了 API 之外。

### Task 4：前端纯逻辑

**偏离计划**

无。

**计划没写的坑**

`EnterWorktree` 建出来的新 worktree **没有 `node_modules`**，`pnpm test` 直接报
`'vitest' is not recognized`。计划里没有 `pnpm install` 这一步。
后续在新 worktree 里执行本计划的人需要先装依赖。

**为什么这么改是安全的**

占位符展开用**函数形式**的 `String.replace` 而不是字符串形式。字符串替换里
`$&`、`$1` 有特殊含义，而仓库路径是用户给的、什么字符都可能有——
`路径里的 $& $1 是字面量` 这条用例就是钉这个的。

整段只扫一遍，替换进去的值不会被二次展开（`替换结果里的占位符不会被二次展开`），
避免了「路径里恰好含 `$BRANCH` 就被再解释一次」这类注入形态。

正则用 `\b` 收尾，`$HEADER` 不会被吃成 `$HEAD` + `ER`。

### Task 5：前端接线与 UI

**偏离计划**

无。

**为什么这么改是安全的**

自定义命令是**全局**配置而不是仓库状态，所以单独开了 `customCommandsStore.ts`
而没有塞进 `repoStore`——后者是 per-tab 的仓库状态，混进去会让两种生命周期纠缠。
组件通过 store 拿数据，没有绕过 store 直接 `invoke`（AGENTS.md §7.5）。

### Task 6：真机验证与文档

**偏离计划（一）——E2E 第 9 条改为自动化**

计划要求手工新建名为 `我的 repo & test` 的 fixture 仓库，在窗口里点按钮验证。
改成了 Rust 集成测试 `spawn_detached_survives_spaces_ampersands_and_non_ascii`，
路径**同时**作为 cwd 和参数传入。

理由：手点一次只覆盖一次，回归时不会重做；而 `&` 一旦被某层拼进 shell 字符串，
失败形态是「执行了半截路径当命令」，靠肉眼看窗口很难判定。自动化覆盖面更宽且可重复。

**偏离计划（二）——E2E 第 2-8 条未执行**

计划 Task 6 Step 2 要求在真实窗口里走完 9 条清单。**实际只完成了 2 条**：

- 第 1 条：`pnpm tauri dev` 启动后，配置文件被创建且内容正确 —— 这同时证明了
  invoke 链路全通（命令注册、`app_config_dir()` 解析、serde 序列化）。
  首次观测时路径还是 `%APPDATA%\com.tauri.dev\`，之后 Task 7 改了 identifier，
  已按新路径重新验证
- 第 9 条：改为自动化测试

第 2-8 条需要在窗口里点击，实现过程中无法操作 GUI，**未验证**。
已在 E2E 文档里逐条标注为「⬜ 待人工」，没有当作通过。合并前需由人补完。

**计划没写的坑**

`read_custom_commands` 会在 `CustomCommandBar` 挂载时被调用，因此
「配置文件是否被正确创建」这件事**不需要点任何按钮**就能观测到。
这让第 1 条成了验证整条 invoke 链路最省力的探针。

**为什么这么改是安全的**

E2E 文档用三级标记（✅ 自动化 / ✅ 真机观测 / ⬜ 待人工）区分证据强度，
未验证的条目没有被记成通过。README 同步了配置文件位置、字段、占位符
和「配置坏了不覆盖」的行为（AGENTS.md §9.3）。

### Task 7（计划外）：把 bundle identifier 从模板默认值改掉

**偏离计划**

本 Task 不在原计划里，是实现完成后用户提出的追加改动：
`tauri.conf.json` 的 `identifier` 从 Tauri 模板默认值 `com.tauri.dev`
改为 `com.momadacoding.jkk`（与仓库名 `momadacoding/jkk` 对齐）。

**为什么必须改，而不只是好看**

`identifier` 决定 `app_config_dir()` 的落点（`config_dir()/${identifier}`），
用户会在 `%APPDATA%` 下看到一个叫「com.tauri.dev」的目录，和 mygitclient 对不上号。

更硬的理由：Tauri CLI 的 changelog 明确写着
**"Prevent building when the bundle identifier is the default `com.tauri.dev`"**——
保持默认值的话 `pnpm tauri build` 会被直接拒绝。这不是审美问题，是发布的前置条件。

**计划没写的坑——这是一次破坏性变更**

改 identifier 会让配置目录整体搬家：

| | 旧 | 新 |
|---|---|---|
| Windows | `%APPDATA%\com.tauri.dev\` | `%APPDATA%\com.momadacoding.jkk\` |

对已经写过配置的用户，表现是「我的自定义命令全没了」——实际上老文件还在老目录里，
只是程序不再去那里找。**本次没有做迁移，也没有删除老目录下的文件。**

选择不做迁移的理由：本功能尚未发布，唯一存在老配置的机器就是本开发机，
为一个零真实用户的场景写迁移代码不划算。老目录下的
`%APPDATA%\com.tauri.dev\customCommands.json` 属于仓库之外的用户数据，
**没有删除**，留给用户自行处理。

若将来在发布后再改 identifier，必须补迁移逻辑，不能照抄这次的做法。

**为什么这么改是安全的**

1. `identifier` 只影响配置目录位置与 bundle 身份，不影响任何命令行为、
   argv 构造或权限模型。
2. 全仓搜索 `com.tauri.dev` 已无残留（`node_modules` 下 Tauri CLI 自己的
   changelog 不算）。README 三个平台的路径表、spec、E2E 清单、
   `commands/custom.rs` 的文档注释同步更新。
3. 改完重新跑了完整门槛并用 `pnpm tauri dev` 真机确认新路径下的配置文件被正确创建。
