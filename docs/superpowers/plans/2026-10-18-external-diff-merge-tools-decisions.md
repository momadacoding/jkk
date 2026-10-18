# 外部 diff / merge 工具 —— 实现日志

> 计划：`docs/superpowers/plans/2026-10-18-external-diff-merge-tools.md`
> 设计：`docs/superpowers/specs/2026-10-17-external-diff-merge-tools-design.md`
> 分支：`worktree-external-tools`（worktree `.claude/worktrees/external-tools`），基点 `dd8911c`
> 执行方式：主会话直接施工（`superpowers:executing-plans`），不用 subagent（用户 2026-10-18 指定）

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 建 worktree 与决策日志 | ✅ 完成 | 本文件 | 纯文档，无代码验证；`git status` 只有本文件 |
| 1 外部工具配置与规则匹配 | ✅ 完成 | `engine/external_tools_config.rs`、`commands/external.rs`、`src/engine/externalTools.ts`、`src/store/externalToolsStore.ts`；测试 `tests/external_tools_config.rs`（7）、`tests/vitest/externalTools.test.ts`（17） | fmt / clippy / cargo test 全绿；pnpm build 通过，pnpm test 43 文件 637 例全过；`pnpm tauri dev` 实测推迟到 Task 4（见决策） |
| 2 外部进程启动、锁与临时目录 | ✅ 完成 | `engine/external_launch.rs`；测试 `tests/external_launch.rs`（12） | fmt / clippy 无告警；cargo test 58 个测试二进制 898 例全过 |
| 3 按字节导出与外部 diff 命令 | ✅ 完成 | `engine/blob_export.rs`、`engine/external_diff.rs`、`commands/external.rs` 加 `open_external_diff`；`external_launch.rs` 加 `ExternalToolExited`、`TempWorkspace` 的 Drop 兜底；测试 `tests/external_diff.rs`（21） | fmt / clippy 无告警；`cargo test -j 4` 59 个二进制 919 例全过；`%TEMP%\jkk-external` 无残留 |
| 4 前端外部 diff 入口与刷新代次 | 🟡 代码完成，Step 5 真实窗口实测待用户执行 | `src/engine/latestGate.ts`、`src/components/externalToolItems.ts`；改 `externalTools.ts`、`externalToolsStore.ts`、`repoStore.ts`、`ContextMenu.tsx`、`FileTree.tsx`、`LocalChangesView.tsx`、`CommitDetailView.tsx`、`RepoTab.tsx`、`styles.css`；测试 `latestGate.test.ts`（4）、`externalTools.test.ts`（+13）、`LocalChangesView.test.tsx`（+7）、`CommitDetailView.test.tsx`（+3） | pnpm build 通过；pnpm test 44 文件 661 例全过；`pnpm tauri dev` 实测未做 |
| 5 冲突信息读取与五种情况分类 | 未开始 | | |
| 6 冲突文件的 HEAD 对比工作区 diff | 未开始 | | |
| 7 外部 merge 命令与各侧标题 | 未开始 | | |
| 8 前端冲突提示与 merge 入口 | 未开始 | | |
| 9 DU / UD 二选一 | 未开始 | | |
| 10 merge driver 检查（后端） | 未开始 | | |
| 11 横幅与合并入口前的确认 | 未开始 | | |
| 12 用户文档、E2E 清单与最终验证 | 未开始 | | |

## 决策条目

### Task 0

#### 偏离计划

- 无。

#### 计划的错误

- 无。

#### 计划没写的坑

- `EnterWorktree` 建出来的分支，在没有任何新提交时，`git branch --merged master` 也会把它列出来（分支指针和 master 指向同一个提交）。这不代表「已合并的旧分支」，这个分支是本任务新开的。
- `superpowers:executing-plans` 自带的 scratch 台账放在 `<仓库根>/.superpowers/sdd/`，但这个目录不在 `.gitignore` 里，建了会变成未跟踪文件。

#### 为什么这么改是安全的

- Ruling：不建 `.superpowers/sdd/` 下的 scratch 台账，进度总账和决策条目都只记在本文件里。理由是 AGENTS.md §2.1 规定，主会话直接施工时这两部分都要落在 `docs/` 下，本文件已经同时承担两种职责。代价（如果这个决定错了）：skill 的 `task-start` / `task-done` 脚本用不了，需要手工维护上面的总账表。

### Task 1

#### 偏离计划

- Ruling：`open_external_tools_config` 只在文件**不存在**时调用 `load_from` 写默认预设，文件存在时不解析就直接打开 —— 计划说「参照 `commands/custom.rs`」，而 custom 版本每次都先 `load_from`，配置一坏就打不开配置文件，用户正是要去修它 —— 代价：无（打开前不校验，校验仍由 `read_external_tools` 负责）。
- Ruling：`pnpm tauri dev` 里实测 invoke 推迟到 Task 4 —— 本 Task 只注册命令，还没有任何界面调用它们；Task 4 接上右键菜单后在真实窗口里一起验证两个命令 —— 代价：如果参数名或注册有误，要到 Task 4 才暴露（两个命令都只有 `app` 参数，没有 camelCase 转换风险）。
- 计划之外多加的测试：Rust `optional_fields_fall_back_to_none`、`find_tool_reports_unknown_id`；vitest 里正则元字符按字面量、多个 `*` 回溯、目录名命中不算、引用不存在的工具返回 null、没有 primary 时 others 仍列出。

#### 计划的错误

- 无。

#### 计划没写的坑

- 新 worktree 里没有 `node_modules`，要先 `pnpm install --frozen-lockfile`（不改 lockfile，不算新增依赖）。pnpm 会提示 esbuild 的 build script 被忽略，不影响 `pnpm build` / `pnpm test`。
- 序列化形状：`defaultDiff` / `defaultMerge` 为空时输出 `null`；工具的 `diffArgs` / `mergeArgs` 和规则的 `diff` / `merge` 为空时**不输出该键**（`skip_serializing_if`），这样默认预设落盘后和 spec §4.4 的示例一致。TS 类型照此写成 `string | null` 和可选字段。
- 非 Windows 平台没有实测过 BC 的路径，预设写 `bcomp`（BC 在 macOS / Linux 上装的命令行名），与 `custom_config.rs` 一样不做探测。

#### 为什么这么改是安全的

- 本 Task 没有任何 git 操作。唯一的写盘是配置文件缺失时写默认预设；解析或校验失败时一律不写（`broken_json_errors_and_leaves_file_untouched` 断言文件字节不变）。
- 规则或默认值引用不存在的工具时报错，不回退到默认工具，避免场景文件被悄悄交给 BC 做文本合并。

### Task 2

#### 偏离计划

- Ruling：`try_lock` 多一个参数 `tool_label`，签名变成 `try_lock(worktree, file, tool_label)` —— spec §4.2 要求重复点击时提示「已在 <工具> 中打开」，计划给的两参数签名拿不到占着锁的是哪个工具；锁表改成 key → 工具名 —— 代价：Task 3、7 调用时多传一个 label，无其他影响。
- Ruling：`TempWorkspace` 多一个 `create_in(root)`，`create()` 等于 `create_in(&temp_root())` —— `workspace_remove_refuses_outside_root` 需要造一个建在根目录之外的 workspace；`remove` 仍只认 `temp_root()` 的**直接子目录**，所以 `create_in` 建在别处的目录永远删不掉 —— 代价：无。
- Ruling：`expand_template` 的 `vars` 名字不带 `$`，并且名字后面紧跟字母、数字或下划线时不算匹配（`$LOCALX` 原样保留）—— 和 `src/engine/customCommands.ts` 用 `\b` 的做法一致；最长名字优先照计划做 —— 代价：无。
- Ruling：argv 测试不用 `echo %*`，改为测试时用 `rustc` 现场编一个把 argv 写进文件的小程序，分两条路径测：直接启动这个 exe（BComp.exe 的形态），以及经一个转发 `%*` 的 `.cmd` 再启动它（`code.cmd` 的形态）—— `echo %*` 打出来的是 cmd 转义后的原始命令行，断言不了「每个参数是单个 argv」；也不在 Cargo.toml 里加 `[[bin]]`，那会影响 Tauri 打包时选主程序 —— 代价：测试依赖 PATH 上有 `rustc`（跑得了 cargo test 的机器都有），编不出来时测试失败，不跳过。
- 计划之外多测了 `%PATH%`、`^caret` 两种参数，以及 `longest_name_wins_regardless_of_order`、`lock_key_is_normalized_on_windows`、`workspace_files_live_in_a_fresh_dir_under_root`。

#### 计划的错误

- 无。

#### 计划没写的坑

- 实测：参数 `a b`、`中文`、`x&y`、`50%`、`!bang!`、`"q"`、`%PATH%`、`^caret` 无论直接启动 exe，还是经 `.cmd` 转发 `%*`，都作为单个 argv 原样送到。所以 Step 4「某种字符送不到就停下来问」的情况没有出现。
- `launcher::resolve_program` 找不到程序时，错误消息写的是「请检查 customCommands.json」。`spawn_watched` 改为自己生成不提配置文件的消息「找不到可执行文件 `<program>`」，由 Task 3 的命令层补上 `externalTools.json` 的路径。

#### 为什么这么改是安全的

- 递归删除只发生在 `TempWorkspace::remove`。删之前用 `symlink_metadata` 确认目标是目录、不是符号链接，并且 canonicalize 之后的父目录**恰好**是 canonicalize 之后的 `temp_root()`；任何一条不满足都拒绝删除，返回 `external_tool_cleanup`。`workspace_remove_refuses_outside_root` 断言越界的文件还在。
- 子目录用 `create_dir` 而不是 `create_dir_all` 创建：撞名时直接报错，不会和别的调用共用一个目录、把对方的文件删掉。
- 外部进程不经过 shell，参数作为数组交给 std；`.cmd` 的转义由 std 负责，argv 测试覆盖了 cmd 的元字符。

### Task 3

#### 偏离计划

- Ruling：打开流程（锁 → 临时目录 → 导出 → 展开参数 → 启动 → 退出后清理）放进 engine 函数 `external_diff::open_external_diff(repo, cfg, config_path, tool_id, target, on_exit)`，Tauri 命令只负责读配置、发事件 —— 计划把流程写在命令里，那样 cleanup、锁释放、错误改写都没法在没有 Tauri 的测试里验证 —— 代价：多一层函数，Task 7 的 merge 照这个结构写。
- Ruling：`TempWorkspace` 加 Drop 兜底删除（校验规则与 `remove` 相同，`remove` 调用过就不再重试）—— 回调闭包持有 workspace，`spawn_watched` 启动失败时闭包被丢弃，没有 Drop 的话临时目录会残留；导出中途失败也是同理 —— 代价：无。
- Ruling：`ExternalToolExited` 定义在 `external_launch.rs`（diff 和 merge 共用），不在 `external_diff.rs` —— 代价：无。
- Ruling：`DiffTarget` 用 serde 的 `rename_all_fields = "camelCase"`（serde 1.0.229 支持）—— 计划只写了 `rename_all = "camelCase"`，那只改变体名（`commit`），**不改**变体里的字段，`old_file` 会要求前端传 `old_file`；嵌套参数也不经过 Tauri 的顶层 camelCase 转换。`diff_target_deserializes_camel_case` 锁住这一点 —— 代价：无。
- Ruling：各侧标题的文案 —— Unstaged「`<file>`（暂存区）/（工作区）」；Staged「（HEAD）/（暂存区）」；Commit 左「`<旧路径>`（父提交短 hash）」，根提交写「（无父提交）」，右「`<file>`（短 hash 标题）」；Range「（from 短 hash）/（to 短 hash）」。spec 把具体文案留给实现者 —— 代价：只是文案。
- Ruling：临时文件不设只读 —— spec 留给实现者；Windows 上 `remove_dir_all` 遇到只读文件会失败、导致残留，而只读带来的好处（BC 里误改临时侧时提醒）不大 —— 代价：用户在 BC 里改了临时那一侧，改动随临时目录一起被删。
- Ruling：`Staged` 不做改名检测，左右都用同一个路径 —— 「本地改动」的 Staged 列表本身没有旧路径（`FileChange` 只有 `path`），重命名后 HEAD 里没有新路径，左边是空文件 —— 代价：暂存区里的改名看起来是「新增」。
- 计划之外多加的测试：`deleted_workdir_file_right_is_empty_temp_file`、`range_uses_from_and_to`、`export_rejects_empty_path`、`side_file_name_keeps_extension`、`directory_is_not_a_regular_file`、`diff_target_deserializes_camel_case`、`tool_without_diff_args_is_unsupported`、`missing_program_error_points_to_config_and_releases_lock`、`second_open_of_same_file_is_busy`、`exit_cleans_workspace_and_releases_lock`。

#### 计划的错误

- `rename_all = "camelCase"` 不足以让前端的 `oldFile` 反序列化成功，见上面的 Ruling。

#### 计划没写的坑

- 全量 `cargo test` 在本机一次失败于 `os error 1455`（The paging file is too small）：59 个测试二进制并行链接，把内存和页面文件耗尽了。这是环境问题，不是代码问题；用 `cargo test -j 4` 重跑全部通过。以后的 Task 都用 `-j 4`。
- `symlink_workdir_side_is_refused` 在本机实际运行了，没有跳过（`--nocapture` 下没有出现「跳过」）。
- `repo.workdir()` 在 Windows 上返回正斜杠路径；工作区那一侧经 `components().collect()` 重新拼成反斜杠再交给外部工具。

#### 为什么这么改是安全的

- 工作区文件只读不写：Unstaged 的右侧直接交给外部工具（用户在工具里存盘是期望的行为），工作区里不存在的一侧写到临时目录，绝不在工作区里造文件（`deleted_workdir_file_right_is_empty_temp_file` 断言）。
- 工作区一侧先用 `symlink_metadata` 检查，符号链接和目录都拒绝；测试断言链接指向的文件内容不变。
- 导出走 `run_git_bytes` 按字节写盘，git 失败时不写文件（`export_failure_does_not_write_file`）。
- 本 Task 没有 git 写操作。所有 git 调用都只在 `TempDir` fixture 里跑。

### Task 4

#### 偏离计划

- Ruling：做了真正的子菜单「用其他工具打开 ▸」，没有按计划的退路平铺成「用 X 打开」—— spec §4.5 写明子菜单是用户确认过的设计；`ContextMenu` 的 `MenuItem` 加了可选的 `children`，只支持一层，鼠标移上去或点一下展开，子菜单渲染在主菜单的 DOM 里（「点菜单外面就关」的判断才会放行它）—— 代价：贴着窗口右边缘弹出时子菜单可能出界，要在真实窗口里看一眼。
- Ruling：菜单拼装分两层 —— `src/engine/externalTools.ts` 的 `diffMenuEntries` 给出文案和工具 id（纯逻辑，可单测），`src/components/externalToolItems.ts` 的 `externalDiffItems` 把它们变成 `MenuItem`（`MenuItem` 类型在 components 层，engine 不能依赖它）。两个视图共用同一个 `ExternalToolsMenu` prop —— 代价：无。
- Ruling：计划之外新增的 engine 函数和常量：`DiffTarget` 类型、`diffTargetFromScope`（解析 `commit:` / `range:`，与 Rust 侧 `read_diff` 一致）、`describeToolExit`（退出后的报错文案）、`EDIT_CONFIG_LABEL`、`OTHER_TOOLS_LABEL` —— 代价：无。
- Ruling：`CommitDetailView` 的 `diffScope` 写成可选 `diffScope?: string | null`，不是计划的必填 —— 现有测试和调用方不传它；不传等于 null，没有对比项 —— 代价：无。
- Ruling：配置在窗口重新获得焦点时重读 —— 计划没写何时重读；只在挂载时读的话，用户点「编辑外部工具配置…」改完，要重启应用菜单才更新 —— 代价：每次切回窗口多一次读小 JSON 的 invoke。
- Ruling：冲突文件的右键菜单里也保留「编辑外部工具配置…」（没有对比项）—— spec §4.5 规定每个外部工具菜单的最后一项都是它；Task 8 会在它前面加 merge 项 —— 代价：无。
- Ruling：外部工具只打开右键点中的那一个文件，不跟随多选 —— 一次开一堆 BC 窗口不是用户要的 —— 代价：无。
- Ruling：提交详情只在 Changes 标签页的文件树、以及 svn 节点有两版基线时的文件树上挂右键菜单；Commit 标签页的文件列表和 File Tree 标签页不挂 —— 计划只要求 FileTree 的 `onContextMenu`；File Tree 标签页看的是整棵树，不是改动 —— 代价：在 Commit 标签页的文件列表上右键没有反应，要切到 Changes。
- Ruling：退出事件报错时消息前面加上文件路径（`<file>：<工具> 退出码 N，可能没有正常完成`）—— 同时开着几个文件时要知道是哪一个 —— 代价：无。
- 计划之外多加的测试：Staged 右键得到 staged 目标、只有一个工具时没有子菜单、点「编辑外部工具配置…」调用 `onEditConfig`、`describeToolExit` 四例、`diffMenuEntries` 三例、`diffTargetFromScope` 的各种畸形输入、`latestGate` 实例之间互不影响。

#### 计划的错误

- 无。

#### 计划没写的坑

- 代次闸的取舍：较新的请求如果失败（`invoke` 抛错），较早那次的成功结果也已经被判为过期、丢掉了，界面会停在更早的状态，直到下一次刷新。文件监听很快会再触发，所以接受。
- PowerShell 的 `Set-Content -Path` 把 `f[1] x.txt` 里的 `[1]` 当通配符，报一个看起来毫不相干的「找不到参数 NoNewline」；fixture 脚本必须用 `-LiteralPath`。Task 12 的 fixture 脚本照此写。
- BComp.exe 会不会等比较窗口关掉才退出（spec §4.4 要求实测），以及 BC 做 diff 时正常关闭的退出码是不是 0，都要靠 Step 5 的真实窗口实测确认。如果 BComp 立刻返回，临时目录会在 BC 还开着时被删掉，Step 5 能直接看出来。

#### 为什么这么改是安全的

- 前端没有任何 git 写操作；外部 diff 只经 `open_external_diff` 走 Task 3 的后端流程。
- 刷新只认 `worktree` 与当前标签相同的退出事件（`samePath`），别的标签不受影响；代次闸保证晚到的旧结果不会盖掉新状态。
- 退出码非零、等待失败都通过 `reportError` 报出来，不吞掉；退出码不参与任何「是否已解决」的判断。

#### Step 5 真实窗口实测（待用户执行）

- fixture：`scratchpad\make-diff-fixture.ps1` 在会话 scratchpad 里建一次性仓库 `diff-fixture`（Staged + Unstaged 的 `a.txt`、只在工作区改的 `目录 a/f[1] x.txt`、改名提交 c2）。
- 结果：待填。
