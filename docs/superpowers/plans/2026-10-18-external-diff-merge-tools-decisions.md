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
| 4 前端外部 diff 入口与刷新代次 | ✅ 完成（真实窗口实测发现 BC 退出码误报，已修复并复测通过，见 Task 4 决策） | `src/engine/latestGate.ts`、`src/components/externalToolItems.ts`；改 `externalTools.ts`、`externalToolsStore.ts`、`repoStore.ts`、`ContextMenu.tsx`、`FileTree.tsx`、`LocalChangesView.tsx`、`CommitDetailView.tsx`、`RepoTab.tsx`、`styles.css`；测试 `latestGate.test.ts`（4）、`externalTools.test.ts`（+13）、`LocalChangesView.test.tsx`（+7）、`CommitDetailView.test.tsx`（+3） | pnpm build 通过；pnpm test 44 文件 662 例全过；cargo test 923 例全过；`pnpm tauri dev` 实测通过 |
| 5 冲突信息读取与五种情况分类 | ✅ 完成 | `engine/conflicts.rs`、`commands/conflict.rs`；测试 `tests/conflicts.rs`（35） | fmt / clippy 无告警；`cargo test -j 4 --no-fail-fast` 60 个二进制 962 例全过 |
| 6 冲突文件的 HEAD 对比工作区 diff | ✅ 完成 | 改 `engine/diff.rs`（`fill_conflicted`、`readable_only` 放行冲突条目）；测试 `tests/fixtures.rs`（+3） | fmt / clippy 无告警；`cargo test -j 4 --no-fail-fast` 60 个二进制 965 例全过 |
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
- 结果（2026-10-19，用户在 `pnpm tauri dev` 里实测）：
  - Unstaged 的 `a.txt` 用 BC 打开成功，两侧标题为「a.txt（暂存区）」「a.txt（工作区）」，只有第 3 行不同，符合 index 对比工作区。这也说明 `open_external_diff` 和 `read_external_tools` 在真实窗口里调通了。
  - BC 开着时 `%TEMP%\jkk-external` 下有子目录，关掉 BC 后子目录消失：**BComp.exe 会等比较窗口关闭才退出**，spec §4.4 要求实测的这一点成立，不需要改用 `BComp.com`。
  - 用户发现应用内的 diff 和 BC 不一致：应用内「本地改动」右侧是 HEAD 对比工作区（第 2、3 行都变了），BC 的 Unstaged 是 index 对比工作区（只有第 3 行变了）。两边都是正确的，这是 spec §4.3 里写明的有意差异；已向用户解释。
  - Staged 的 `a.txt`：左边 `line 2`、右边 `line 2 staged`，正确（HEAD 对比暂存区）。
  - `目录 a/f[1] x.txt`（中文、空格、方括号）：BC 正常打开，路径没有被截断或当成通配符。
  - 提交详情里 c2 的改名文件 `new-name.txt`：左边是改名前 `old-name.txt` 的内容，正确。
  - 子菜单「用其他工具打开 ▸」靠窗口右边缘时没有超出屏幕。
  - **发现缺陷**：关掉 BC 后顶部报「new-name.txt：Beyond Compare 退出码 13，可能没有正常完成」。见下面的修复。

#### Step 5 实测发现的缺陷与修复：BC 的退出码

- 计划的错误：Task 4 把「退出码非 0」一律当成工具故障报给用户。BC5 官方命令行文档（`command_line_reference.html`）写明 BComp.exe 用退出码报**比较结果**：0 成功、1 逐字节相同、2 按规则相同、11 逐字节有差异、12 相似、13 按规则有差异、14 检测到冲突；100 起才是故障（100 未知错误、101 有冲突且合并结果没写出、102 BComp 无法等待 BCompare、103 找不到 BCompare、104 试用期已过、105–107 脚本错误）。所以每次对比有差异的文件关掉 BC 都会误报 13。
- Ruling（用户在 A/B/C 三个方案里选了 A）：工具配置加两个可选字段 —— `successExitCodes`（缺省只认 0）和 `exitCodeMessages`（已知故障码的说明，键是十进制字符串）。BC 预设写 `[0, 1, 2, 11, 12, 13, 14]` 和 100–107 的中文说明；101 不算成功，merge 时它表示合并结果没保存，应当提示。判断放在后端：`external_launch::exit_event` 用**启动时**那份配置里的工具算出 `success` 和 `codeMessage` 放进 `external-tool-exited` 事件；前端 `describeToolExit` 只看 `success`，不再自己判断退出码。没有退出码或等待失败一律不算成功 —— 代价：用户旧的 `externalTools.json` 里没有这两个字段，BC 仍按「只认 0」误报，要删掉配置让它重新生成，或手动加上。
- 没选的方案：B 按程序名认出 BC 套用规则（把 BC 的知识写死在代码里，换个名字就失效）；C 所有工具都只在退出码 ≥ 100 时报错（对 VS Code 不成立，会漏报）。
- 用户本机的 `%APPDATA%\com.momadacoding.jkk\externalTools.json` 删除前核对过内容，和旧默认预设一字不差（用户确认没改过），经用户同意已删除，下次读取时按新预设重新生成。
- 测试：Rust `bc_preset_treats_comparison_results_as_success`、`tool_without_success_codes_only_accepts_zero`、`exit_codes_and_messages_load_from_json`、`exit_event_judges_code_by_tool_config`；vitest `describeToolExit` 五例（含「后端判为正常时即使退出码是 13 也不报」）。pnpm build 通过；pnpm test 662 例全过；fmt / clippy 无告警；`cargo test -j 4` 59 个二进制 923 例全过。
- 用户复测（2026-10-19，配置已按新预设重新生成）：对比有差异的文件、关掉 BC 后不再报错。Task 4 Step 5 全部通过。

### 阶段 1 review（Codex，2026-10-20）

独立 review 用 Codex `adversarial-review --base master --scope branch`，按 AGENTS.md §13 的 rubric，先读本文件的 Ruling 再看 diff。结论 needs-attention，三条发现逐条核实后都成立，全部修复，每条先写失败测试。

#### 计划的错误

- **[high] 父目录链接可以绕过工作区一侧的检查**（`external_diff.rs` `workdir_side`）：原来只用 `symlink_metadata` 查末级文件。已跟踪的 `sub/a.txt` 所在目录被换成指向仓库外的 junction 时，工具会打开仓库外的同名文件，用户在工具里存盘就改到了仓库外。
  - 修复：从 worktree 根往下逐段 `symlink_metadata`，中间任何一级是链接（junction 也算）都拒绝，返回 `external_tool_not_regular`；最后核对规范化后的位置仍在 worktree 内。另外在任何 git2 查找之前校验相对路径（`check_rel_path`）：空、绝对路径、盘符、`..` 一律返回 `external_tool_bad_path`。
  - 测试：`linked_parent_dir_of_workdir_side_is_refused`（本机真的建了 junction，`--nocapture` 下没有出现「跳过」）、`path_escaping_worktree_is_refused`。
  - 修复前写测试时发现：**git2 的 `Index::get_path` 拿到绝对路径时会直接 panic**（`index.rs:397`），不是返回错误；所以路径校验必须放在所有 git2 查找之前，四种 target 都要校验 `file` 和 `old_file`。
- **[medium] 后发出的旧 scope 刷新让当前提交的 diff 作废**（`repoStore.ts` `refreshDiff`）：Stage / Unstage / Discard / Commit 的回调固定刷新 `workdir`。用户操作后切到提交 B，B 的请求先发出、旧操作的 `workdir` 请求后发出；Task 4 加的代次闸不分 scope，B 的结果被判为过期，界面一直显示工作区的改动。这个竞态原来就有（谁后回谁赢），代次闸让它变成必然。
  - 修复：代次按 scope 分开计（key 为 `diff:<scope>`）；写回前再核对 scope 是不是当前视图要的那一份。当前视图的 scope 由 `useRepoEffects` 每次渲染写入 `useRepoStore` 新增的 `wantedDiffScope` ref（在渲染里写，保证按选中刷新的 effect 发请求前已是新值），`undefined` 表示还不知道（刚打开仓库），此时照常写回。`RepoRefreshers` 接口多了 `wantedDiffScope` 字段。
  - 测试：`tests/vitest/repoStoreDiffRefresh.test.ts` 用 jsdom 和 `renderHook` 驱动真实的 `useRepoStore`，`invoke` 换成可控的 promise，覆盖 workdir 先回 / 后回、同 scope 两次以旧的晚到、还不知道视图时照常写回四种情况。
- **[medium] 没配 required 的 smudge 过滤器失败被当成导出成功**（`blob_export.rs` `export_filtered`）：Codex 实测，过滤器退出 1 且没有 `required=true` 时，`git cat-file --filters` 仍退出 0，只在 stderr 警告，stdout 是跳过过滤器的内容；`run_git_bytes` 只看退出码，于是照常写临时文件、启动工具，违反 spec §4.2「过滤器失败就中止」。
  - 修复：导出前用 `git check-attr -z filter -- <path>` 查出这个路径的过滤器名，只对这一次命令加 `-c filter.<name>.required=true`，失败就变成非 0；不改仓库配置。名字里带 `=` 时没法安全拼进 `-c`，按出错处理。
  - 测试：`export_fails_when_non_required_smudge_filter_fails`（同时断言仓库配置里没有被写进 required）、`export_applies_smudge_filter`（正常过滤器照常生效，路径带空格和方括号）。

#### 计划没写的坑

- 全量 `cargo test -j 4` 一次失败在 `vcs_backend::backend_diff_matches_engine` 的 fixture 准备步骤（`git commit -m c1` 失败），这个测试和本分支的改动无关；单独重跑 3 次都通过，判断是高并行负载下的偶发失败。另外 cargo 默认在第一个失败的测试二进制处停止，只跑了 56 个；以后全量验证用 `cargo test -j 4 --no-fail-fast`，确保 59 个都跑到。

#### 验证

- pnpm build 通过；pnpm test 45 文件 666 例全过。
- cargo fmt --check、cargo clippy --all-targets 无告警；`cargo test -j 4 --no-fail-fast` 59 个二进制 927 例全过。

### Task 5

#### 偏离计划

- Ruling：`path_attrs` 用 `git check-attr --stdin -z merge filter conflict-marker-size`（`run_git_stdin`），不是计划写的 `-- <paths>` —— 冲突文件可能有几百个，Windows 命令行有长度上限；`-z` 的输入输出都不受路径字符影响 —— 代价：无。
- Ruling：多导出一个 `merge_config_raw(repo) -> Result<String>`，`merge_config` 在它之上解析 —— Task 10 的缓存 fingerprint 要用原始输出 —— 代价：无。
- Ruling：生效 driver 的判定照搬 git `ll-merge.c` 的 `find_ll_merge_driver`：`merge`（set）→ 文本；`-merge`（unset）→ binary；没写 → 看 `merge.default`；写了名字 → 先找配置里有 `merge.<名字>.*` 任意键的自定义 driver，再找内置的 `text` / `binary` / `union`，都没有就退回文本合并。所以「本机完全没配」的名字（表 1 第 3 行）走 ① / ②，不算 ④ —— 代价：无，这正是 git 的真实行为。
- Ruling：`merge.default` 指向自定义 driver 时 reason 是 `MergeDefault`、`driver` 填名字；指向 `binary` 时 `driver` 为 None —— spec 把 `merge.default` 归在 ④a —— 代价：界面上按 ④a 文案显示，不说「配置了自动合并工具」。
- Ruling：④ 里二进制内容排在最前，先于属性判定 —— ④c 不给外部 merge 入口，`merge=binary` 加二进制内容时不能被 ④a 盖掉（`binary_content_wins_over_binary_attr`）—— 代价：无。
- Ruling：UU / AA 的工作区路径被换成目录或符号链接时判为 Structural；工作区文件被删掉时判为 NoMarkers —— 前者已经不是内容合并的问题，外部 merge 也会被 `ensure_regular_file` 拒绝；后者是用户的合法选择，② 的 diff 如实显示成删除 —— 代价：删掉的文件会显示「检查后可以暂存」。
- Ruling：`same_as_head` 只对 UU / AA 计算，用 libgit2 的 `diff_tree_to_workdir`（不经过 index）加精确 pathspec —— 它套用 autocrlf 等过滤器，工作区是 CRLF、blob 是 LF 时不会误判成不同（`same_as_head_survives_autocrlf`）；HEAD 里没有的路径永远是 false —— 代价：无。
- Ruling：`symlink_conflict_is_structural` 不建真的链接，用 `update-index --cacheinfo 120000,...` 直接写 mode 120000 的条目，所以不需要「没权限就跳过」—— 判定只看 index 的 mode —— 代价：无。
- Ruling：测试进程里设 `GIT_CONFIG_GLOBAL=<空文件>`、`GIT_CONFIG_NOSYSTEM=1`（`Once` 只设一次）—— 本机全局配置里有 git-lfs 装的 `merge.lfs-text.driver`，会混进 `merge_config` 的结果，测试结论不能依赖开发机的配置 —— 代价：只影响 `tests/conflicts.rs` 这个测试二进制。
- 计划之外多加的测试：`path_attrs` 两例、`merge_config` 两例（零匹配、分组和取最后一个值、名字带点）、`merge_default_binary_is_not_text_merged`、`explicit_text_attr_overrides_merge_default`、`binary_content_wins_over_binary_attr`、`lfs_filter_is_not_text_merged`、`marker_size_attr_is_respected`、`same_as_head_survives_autocrlf`、`no_conflicts_is_empty`、`chinese_space_bracket_dash_path`。

#### 计划的错误

- 无。

#### 计划没写的坑

- 结构冲突的真实形状（git 2.51.2.windows.1）：rename/rename（一边 a→b、一边 a→c）留下 `a.txt` 只有 stage 1（`git status` 显示 DD）、`b.txt` 只有 stage 2（AU）、`c.txt` 只有 stage 3（UA）。同一次 merge 里的 modify/delete 留下 stage {1,2}，`git status` 显示 UD（deleted by them），与计划的映射一致。
- LFS 实测（本机 git-lfs 3.7.1，`git lfs track` 的标准写法 `filter=lfs diff=lfs merge=lfs -text`）：本机没有 `merge.lfs` 这个 driver（git-lfs 只装了 `merge.lfs-text`），git 退回逐行文本合并，**冲突标记写进了 pointer 文件里**（`oid` / `size` 两行被标记包住），工作区不是真实内容。所以 LFS 必须算 ④，不能按 ① 让用户去 BC 里解 pointer 文本。这条判定只看 `filter=lfs` 属性，不依赖本机有没有装 lfs。
- libgit2 `diff_tree_to_workdir_with_index` 对冲突路径只给 `Conflicted`，`diff_tree_to_workdir`（不经过 index）给 `Modified` 并带 hunk，过滤器照常生效。Task 6 用后者。
- 新仓库 HEAD 未出生时 `git rev-parse --abbrev-ref HEAD` 失败，测试取当前分支要用 `git symbolic-ref --short HEAD`。

#### 为什么这么改是安全的

- 本 Task 只读：git2 读 index、blob、HEAD tree 和工作区文件；CLI 只有 `check-attr`（stdin 喂路径）和 `config --get-regexp`，都不写仓库。
- 判不准的一律往保守方向走：属性认不出的名字按 git 的规则退回文本合并后，再看工作区有没有标记；LFS、二进制、自定义 driver 都不会显示「可以暂存」。

### Task 6

#### 偏离计划

- Ruling：冲突文件的 hunk 用 `diff_tree_to_workdir`（不经过 index）加精确 pathspec 求出，再交给现有的 `diff_to_data` 收集行；没有用计划写的 `git2::Patch::from_blob_and_buffer` —— 后者直接拿 blob 和工作区字节比，autocrlf 下工作区是 CRLF，每一行都会被当成改动；前者套用过滤器（`conflicted_file_diff_applies_autocrlf` 断言只有真正改过的一行出现）—— 代价：多一次只覆盖冲突路径的 diff。
- Ruling：工作区里的冲突文件被删掉时，显示成整份删除，不是计划写的「hunks 为空」—— 和 Task 5「工作区删掉判为 NoMarkers」配套：界面会提示「检查后可以暂存」，用户必须能看出要暂存的是一次删除（`conflicted_file_missing_in_workdir_shows_deletion`）—— 代价：无。
- Ruling：`fill_conflicted` 失败只记 warning、不让 `read_diff` 失败 —— 与本文件里坏路径不拖垮整个变更视图的既有做法一致 —— 代价：冲突文件没有行内容时界面上看不到原因，要看日志。
- 计划之外多加的测试：`conflicted_file_diff_applies_autocrlf`、`conflicted_file_missing_in_workdir_shows_deletion`。

#### 计划的错误

- `Patch::from_blob_and_buffer` 不套用过滤器，见上。

#### 计划没写的坑

- 工作区里删掉的冲突文件原来会**整个从 diff 里消失**：`readable_only` 把 `Conflicted` 条目当成「工作区读不到的坏路径」排除掉了。冲突条目 libgit2 不读内容，现在和 `Deleted` 一样放行。这一改同时作用于 `read_status` 的行数统计，冲突行本来就是 0/0，不受影响。

#### 为什么这么改是安全的

- 只读：多出来的一次 diff 只覆盖冲突路径，pathspec 按字面量匹配（`disable_pathspec_match`），`f[1].txt` 不会匹配到 `f1.txt`。
- `status` 仍是 `conflicted`，前端按冲突处理，不会把冲突文件当普通改动暂存。
