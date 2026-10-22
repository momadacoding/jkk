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
| 7 外部 merge 命令与各侧标题 | ✅ 完成 | `engine/external_merge.rs`、`commands/external.rs` 加 `open_external_merge`；`external_diff.rs` 抽出 `workdir_file` 等共用函数；测试 `tests/external_merge.rs`（20） | fmt / clippy 无告警；`cargo test -j 4 --no-fail-fast` 61 个二进制 985 例全过；`pnpm tauri dev` 实测并到 Task 8 |
| 8 前端冲突提示与 merge 入口 | ✅ 代码完成（真实窗口实测并入 Task 12） | `src/engine/conflictView.ts`、`src/components/ConflictNotice.tsx`；改 `types.ts`、`externalTools.ts`（`mergeMenuEntries`）、`externalToolItems.ts`（`externalMergeItems`、`onMerge`）、`externalToolsStore.ts`（`openExternalMerge`）、`repoStore.ts`（`conflicts`、`refreshConflicts`）、`LocalChangesView.tsx`、`RepoTab.tsx`、`styles.css`；测试 `conflictView.test.ts`（13）、`repoStoreConflicts.test.ts`（4）、`LocalChangesView.test.tsx`（+8） | pnpm build 通过；pnpm test 47 文件 691 例全过 |
| 9 DU / UD 二选一 | ✅ 代码完成（真实窗口实测并入 Task 12） | `conflicts.rs` 加 `resolve_delete_modify`、命令 `resolve_delete_modify`；`LocalChangesView`（确认框）、`ConflictNotice`、`RepoTab`；测试 `tests/conflicts.rs`（+11）、`LocalChangesView.test.tsx`（+4） | fmt / clippy 无告警；`cargo test -j 4 --no-fail-fast` 61 个二进制 996 例全过；pnpm build 通过，pnpm test 47 文件 695 例全过 |
| 10 merge driver 检查（后端） | ✅ 完成 | `engine/merge_drivers.rs`、命令 `check_merge_drivers`；`conflicts.rs` 的 `parse_merge_config` 改为 `pub(crate)`；测试 `tests/merge_drivers.rs`（22） | fmt / clippy 无告警；`cargo test -j 4 --no-fail-fast` 62 个二进制 1018 例全过 |
| 11 横幅与合并入口前的确认 | ✅ 代码完成（真实窗口实测并入 Task 12） | `src/engine/mergeDrivers.ts`、`src/components/MergeDriverBanner.tsx`；改 `repoStore.ts`（`mergeDrivers`、`refreshMergeDrivers`）、`RepoTab.tsx`（`guardMerge`）、`svnSyncFlow.ts`、`svnSyncOutFlow.ts`、`ConfirmDialog.tsx`（`focusCancel`）、`styles.css`；测试 `mergeDrivers.test.ts`（12）、`svnSyncFlow.test.ts`（+4）、`svnSyncOutFlow.test.ts`（+4）、`ConfirmDialog.test.tsx`（+2） | pnpm build 通过；pnpm test 48 文件 717 例全过 |
| 12 用户文档、E2E 清单与最终验证 | 🟡 进行中：E2E 第二版 1–12 全部真机通过（2026-10-22，第 4 条为 10-19 的结果）；E2E 中修了三处缺陷（弹框焦点样式、外部工具工作目录、「完全相同」文案）与 fixture；独立最终 review 待用户 | `README.md`「外部 diff / merge 工具」一节、`docs/superpowers/e2e/2026-10-18-external-tools-e2e.md`、`scripts/e2e-external-tools-fixture.ps1`；修 `git_cli::run_git_stdin` 死锁（`tests/git_cli.rs` +1）；`tests/merge_drivers.rs` 加 `perf_50k_files`（ignored） | fmt / clippy 无告警；`cargo test -j 4 --no-fail-fast` 62 个二进制 1019 例全过、1 例 ignored；pnpm build 通过，pnpm test 48 文件 717 例全过；fixture 脚本实际跑过一次，五个仓库状态符合预期后删除 |

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

### Task 7

#### 偏离计划

- Ruling：`merge_titles`、`prepare_merge`、`open_external_merge` 放在新模块 `engine/external_merge.rs`，不放进计划写的 `conflicts.rs` —— `conflicts.rs` 只负责只读的冲突信息，外部 merge 的结构和 `external_diff.rs` 一一对应（锁 → 临时目录 → 导出 → 展开 → 启动），放一起好对照 —— 代价：无。
- Ruling：`external_diff.rs` 的 `check_rel_path`、`path_arg`、`short` 改成 `pub(crate)`；`workdir_side` 拆出 `pub(crate) fn workdir_file(repo, rel) -> Result<Option<PathBuf>>`，`$MERGED` 复用同一套逐段检查（父目录是链接、不在 worktree 内都拒绝）—— 不复制一份安全检查 —— 代价：无，diff 侧行为不变（`external_diff` 21 例照常通过）。
- Ruling：`prepare_merge` 每次都重新调用 `read_conflicts` 并按路径找到这一条；文件已经不是冲突时报 `conflict_state_changed`（与 Task 9 同一个 code），`$MERGED` 不存在时报 `external_tool_not_regular` —— 不信任前端可能已经过期的状态 —— 代价：每次打开多读一次冲突信息（一次 `check-attr` 加一次 `config`）。
- Ruling：标题文案 —— merge：local `<分支>（HEAD）`（detached 时短 hash），remote 是指着 `MERGE_HEAD` 的分支名（先本地后远程，同类按名字排序取第一个），没有就用短 hash；rebase：local `当前重放结果（HEAD，基于 <onto 分支或短 hash>）`，onto 读 gitdir 下 `rebase-merge/onto` 或 `rebase-apply/onto`，remote `REBASE_HEAD <短 hash> <标题>`；cherry-pick / revert：`CHERRY_PICK_HEAD` / `REVERT_HEAD` 同样形状；其他状态（stash apply / pop 的冲突不留状态文件）remote 写 `对方` —— 计划没覆盖 stash —— 代价：只是文案。
- 计划之外多加的测试：`merge_titles_use_branch_names`、`merge_title_falls_back_to_short_hash`、`cherry_pick_titles_use_cherry_pick_head`、`not_text_merged_exports_original_versions`、`resolved_file_is_rejected`、`bad_paths_are_rejected_before_any_lookup`、`missing_merged_file_is_refused`、`chinese_space_bracket_dash_path`、`linked_worktree_merged_is_in_that_worktree`、`tool_without_merge_args_is_unsupported`、`missing_program_points_to_config_and_releases_lock`、`second_open_of_same_file_is_busy`。
- `merged_symlink_is_refused` 写成 `merged_under_linked_dir_is_refused`：Windows 上建文件符号链接要特权，改为把 `$MERGED` 的父目录换成指向仓库外的 junction（不需要特权，本机实际运行，没有跳过）；断言被拒绝、仓库外的文件内容不变。末级是链接的情况 Task 5 已经判成 Structural，同样被拒绝。

#### 计划的错误

- 无。

#### 计划没写的坑

- `Repository::mergehead_foreach` 要 `&mut Repository`。
- rebase 停在冲突上时 HEAD 是 detached，`repo.head()` 没有分支名，所以 rebase 的 local 标题必须靠 onto 文件取基线名，不能用 `head_label`。

#### 为什么这么改是安全的

- 不写仓库：导出只走 `cat-file --filters`，写的是应用临时目录下本次新建的子目录。
- `$MERGED` 是工作区里已有的普通文件，打开前逐段检查父目录不是链接、规范化后仍在 worktree 内；清理只调用 `TempWorkspace::remove`，它只删 `temp_root()` 的直接子目录，`merged_path_survives_cleanup` 断言 `$MERGED` 内容不变。
- 工具退出后不 `git add`；退出码只用来报故障，不参与冲突是否已解决的判断。

### Task 8

#### 偏离计划

- Ruling：`refreshConflicts` 不单独挂在 `repo-changed` 和外部工具退出事件上，而是由 `refreshStatus` 拿到新 status 之后调用 —— 这两条路径（以及 stage / discard / commit 的回调）本来就都会刷新 status，冲突分类跟着 status 走就不会漏掉哪条路径；status 里没有冲突时不调后端、直接置空 —— 代价：`refreshStatus` 的 promise 要等冲突信息也读完才 resolve。
- Ruling：`RepoState.conflicts` 是 `ConflictInfo[] | null`，`null` 表示还没读到或读取失败 —— 失败时如果写成空数组，界面会把冲突文件当成「没有冲突信息」而给出普通 diff；`null` 时冲突文件只显示 `这个文件有冲突`（`PENDING_CONFLICT_VIEW`），没有任何按钮、没有 diff、右键菜单里也没有 merge 入口 —— 代价：读取失败时用户只能看到顶部错误横幅和这句话。
- Ruling：右键菜单的 merge 入口也按 spec §4.2 限制在 ① / ② / ④a / ④b（`canExternalMerge`），不是所有冲突文件都给 —— spec 写明「③、④c、⑤ 没有入口」，计划只写了按钮 —— 代价：无。
- Ruling：几处计划没给的文案由我定：③ `HEAD 这边删除了这个文件，另一边修改了它` / `HEAD 这边修改了这个文件，另一边删除了它`，正文 `选择保留修改后的文件，或删除它。选择后立即暂存。`；⑤ 按 code 写 `结构冲突（DD）：两边都删除或改名了这个文件`、`结构冲突（AU）：只有 HEAD 这边有这个路径（通常是改名冲突）`、`结构冲突（UA）：只有另一边有这个路径（通常是改名冲突）`，mode 不是普通文件时写 `结构冲突（<code>）：这个路径涉及符号链接、子模块或文件与目录互换`，正文 `应用内暂不处理这种冲突，请在命令行里解决。`；④b 的 heading / body 按 spec 原文在「说明它没能完全合并」处断开 —— 用「HEAD 这边 / 另一边」而不是 ours / theirs，因为 rebase 时两者含义对调 —— 代价：只是文案。
- Ruling：④b 的 heading 里 driver 名用反引号包住（计划的测试断言就是这个字面量），`ConflictNotice` 把反引号段渲染成 `<code>` —— 既满足断言，界面上又不出现裸反引号 —— 代价：无。
- Ruling：`sameAsHeadNote` 非空时用这句话代替 diff，不再显示空 diff —— spec 写的是「改为提示」 —— 代价：无。
- Ruling：`ExternalToolsMenu` 加必填的 `onMerge`；diff 和 merge 两种菜单共用一个 `toolItems` 拼装函数 —— 代价：无。
- Ruling：Step 6 的真实窗口实测（BC 解决 → 保存 → ① 变 ② → Stage）推迟到 Task 12，和 E2E 清单里同一条一起由用户走一遍；Task 9–11 的界面也一并在那时验证 —— 实测需要用户操作 BC，分几次打断用户不划算 —— 代价：如果 invoke 参数名有错，要到 Task 12 才暴露（`open_external_merge` 的参数 `path` / `toolId` / `file` 与 `open_external_diff` 同一种形状）。
- 计划之外多加的测试：`canExternalMerge`、`PENDING_CONFLICT_VIEW`、④a 的 merge.default / LFS、⑤ 的 heading；组件测试里 ① 不显示 diff、② 显示 diff 并能 Stage、其他 merge 工具只列有 `mergeArgs` 的、二进制冲突没有 merge 入口、冲突信息未读到时没有任何入口；store 测试四例。

#### 计划的错误

- 无。

#### 计划没写的坑

- 选中的冲突文件顶部和右键菜单里会同时出现「在 Beyond Compare 中解决」，组件测试要用 `within(.context-menu)` 限定范围。

#### 为什么这么改是安全的

- 前端没有新的 git 写操作；Stage 按钮走现有的 `stage` 命令，必须用户点。外部 merge 只经 `open_external_merge` 走 Task 7 的后端流程，后端打开前还会重新读一遍冲突状态。
- 冲突信息的写回有代次闸（key `conflicts`），冲突在外面解决之后晚到的旧结果不会把冲突写回来（`repoStoreConflicts.test.ts` 第 3 例）。

### Task 9

#### 偏离计划

- Ruling：`svn_write_guard.rs` 不改 —— 它扫描的是 `src/vcs/svn.rs` 里 `impl Vcs for SvnGitBackend` 的方法，新命令不经过 `Vcs` trait，直接调 engine，不在扫描范围内；守卫由 `resolve_delete_modify` 自己调用 `svn_lock::assert_not_syncing` 承担，`refused_while_sync_lock_held` 断言被拒绝（code `sync_in_progress`）且 index 和工作区都没动 —— 代价：以后有人把这个函数挪进 `Vcs` trait 时要记得归类。
- Ruling：除了空路径（`empty_path`），还用 `check_rel_path` 拒绝绝对路径、盘符和 `..`（`external_tool_bad_path`）—— 防止路径逃出仓库 —— 代价：无。
- Ruling：「状态已变化」统一用 code `conflict_state_changed`：路径已经不是冲突、冲突不是 DU / UD（例如 UU）、保留时工作区文件不存在。保留时文件是链接或父目录是链接，沿用 `workdir_file` 的 `external_tool_not_regular` —— 代价：无。
- Ruling：删除确认框放在 `LocalChangesView` 里（`useConfirm`），不放 `RepoTab` —— 组件测试能直接覆盖「取消就不调用」；标题 `删除 <文件>？`，正文 `<文件> 会从工作区删除，并立即暂存这次删除；修改过的那一版不会保留。`，确认按钮 `删除文件`、`danger` —— 代价：只是文案。
- Ruling：RepoTab 沿用现有的 `runAndRefresh` 调 `resolve_delete_modify`（与 stage / unstage 同一条路径，在组件里 invoke），没有另加 store 函数 —— 与周围代码一致，不在本任务里重构 —— 代价：无。
- 计划之外多加的测试：`keep_resolves_ud_and_delete_resolves_du`、`dash_and_chinese_path_resolves`、`escaping_path_rejected`、`not_delete_modify_rejected`；`literal_pathspec_spares_lookalike` 同时覆盖 keep；组件测试「没接回调时不显示这两个按钮」、「确认之后才调用 keep = false」。

#### 计划的错误

- 无。

#### 计划没写的坑

- 实测（git 2.51.2）：`git --literal-pathspecs rm -- <path>` 对 UD 条目不需要 `-f`，退出 0，index 里变成 `D`。
- 写 fixture 时亲手踩到了 spec 要防的坑：`git rm -q 'f[1].txt'` 不加 literal 时把旁边的 `f1.txt` 一起删了。测试 fixture 里的删除也一律用 `--literal-pathspecs`。

#### 为什么这么改是安全的

- 写操作只有 `git --literal-pathspecs add|rm -- <file>`，走 `run_git`；空路径在任何 git 调用之前拒绝，`--literal-pathspecs` 保证 `f[1].txt` 不会匹配 `f1.txt`（`literal_pathspec_spares_lookalike` 断言 `f1.txt` 的内容和状态都不变）。
- 执行前重新读 index 复核，状态不符就不执行；删除必须先在确认框里确认，「保留」不弹框（spec 原文）。
- 所有测试都在 `TempDir` fixture 里跑。

### Task 10

#### 偏离计划

- Ruling：`DriverState` 序列化成带 `kind` 标签的对象（`{ kind: 'ok' }`、`{ kind: 'programMissing', program }`），不用 serde 默认的外部标签 —— 单元变体和结构变体混在一起时，默认形状一会儿是字符串一会儿是对象，前端不好判 —— 代价：无。
- Ruling：`check_count` 按仓库分开计，签名是 `check_count(repo: &str) -> usize`，不是计划写的无参全局计数 —— 同一个测试二进制里的测试并行跑，全局计数会被别的测试加上去 —— 代价：无。
- Ruling：fingerprint 在计划的五项之外多加两项：index 里**所有路径**（不含 oid），以及根目录 `.gitattributes` 的 mtime（不管是否被跟踪）—— 前者：仓库里原来没有场景文件、后来新增一个时，规则没变也必须重新算，否则缺 driver 永远报不出来（`cache_invalidated_by_new_tracked_file`）；后者：刚新建、还没 `git add` 的根 `.gitattributes` 已经生效 —— 代价：每次检查要遍历一遍 index（libgit2 内存操作，5 万文件量级在毫秒级，Task 12 实测）；暂存一个**新**文件会让缓存失效一次。子目录里**未跟踪**的 `.gitattributes` 仍然不在指纹里，要等别的变化才会重新算。
- Ruling：`core.attributesFile` 没设时，按 git 的默认位置 `$XDG_CONFIG_HOME/git/attributes`（没有 XDG 时 `~/.config/git/attributes`）取 mtime —— 计划只写了 `core.attributesFile` —— 代价：无。
- Ruling：`merge=lfs` 不算需要检查的自定义名字 —— `git lfs track` 往每个 LFS 仓库都写 `merge=lfs`，git-lfs 从来不定义 `merge.lfs` driver，git 一直是退回文本合并 pointer 文件（Task 5 实测）；不排除的话，每个 LFS 仓库都会挂一条「Unity 场景会被写坏」的红色横幅、每次合并都要确认。LFS 冲突由冲突显示的 ④（`filter=lfs`）兜住（`lfs_standard_attr_not_reported`，先删掉排除项确认它失败，再恢复）—— 代价：如果有人真的给 LFS 配了一个叫 `lfs` 的 merge driver 又配坏了，这里不会提示。
- Ruling：程序检查只认 Rust 意义上的绝对路径（Windows 上要带盘符）；`/c/Program Files/...` 这种 MSYS 路径按「不是绝对路径」跳过，算 Ok —— spec §4.6 第 3 点「不是绝对路径就跳过」 —— 代价：用 MSYS 路径写的失效 driver 报不出来。
- 计划之外多加的测试：`sample_paths_are_capped_at_three`、`quoted_program_path_with_spaces`、`relative_program_is_not_checked`、`core_attributes_file_counts`、`merge_default_custom_is_checked`、`merge_default_builtin_is_not_reported`、`check_failure_is_reported_as_incomplete`、`cache_invalidated_by_config_change`、`cache_invalidated_by_new_tracked_file`、`lfs_standard_attr_not_reported`。
- 过程说明：本 Task 的 21 个测试写好后没有单独跑一次看失败（模块不存在，只会是编译错误），实现后一次通过；之后补的 `lfs_standard_attr_not_reported` 按 RED → GREEN 验证过。

#### 计划的错误

- 无。

#### 计划没写的坑

- **实测结论（`merge_tree_and_stash_apply_invoke_driver`）**：`git merge-tree --write-tree -z --merge-base=<base> <ours> <theirs>`（与 `svn_deliver.rs` 的 replay 参数相同）和 `git stash apply` **都会调用**自定义 merge driver。所以交付慢路径和 stash apply / pop 都属于「会触发合并的入口」，Task 11 都要拦。
- 测试进程除了 `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_NOSYSTEM`，还要把 `XDG_CONFIG_HOME` 指到空目录，否则本机的全局属性文件会混进 `check-attr` 的结果。

#### 为什么这么改是安全的

- 只读：`config --get-regexp`、`rev-parse --git-path`、`ls-files -z`、`check-attr --stdin -z` 各一次，不改用户的 git 配置（spec「不自动修改用户的 git 配置」）。
- 检查失败一律写进 `incomplete`，不会显示成配置正常；只缓存成功的结果。

### Task 11

#### 偏离计划

- Ruling：`driverBanners` 的返回值多一个可选字段 `example`（配置示例），没有把示例拼进 `text` —— 计划要求 `text` 逐字等于给定文案、「后面附上配置示例」；分开放，横幅里能用 `<pre>` 显示、方便复制 —— 代价：无。示例以 UnityYAMLMerge 为例，driver 名字换成实际缺的那个，程序路径写成占位 `<UnityYAMLMerge.exe 的完整路径>`（不同机器的 Unity 版本目录不同）。
- Ruling：没有示例路径时（只来自 `merge.default`）文案写「一些文件」，不编造路径 —— 代价：无。
- Ruling：`mergeRiskConfirm` 返回 engine 自己的 `MergeRiskConfirm`（与 `ConfirmRequest` 同名字段），不 import components 的类型 —— AGENTS.md §7.5 engine 不依赖组件层，`svnSyncIn.ts` 也是这个做法。文案：标题 `合并前确认`，正文 `本机 git 配置里没有定义 merge driver <名字…>，.gitattributes 却要求用它合并。`，警告条 `git 会退回逐行文本合并，往这些文件里写冲突标记，Unity 场景会被写坏。建议先按顶部横幅的示例配好 driver 再合并。` —— 代价：只是文案。
- Ruling：`ConfirmDialog` 加可选的 `focusCancel` —— spec §4.6「默认按钮是『取消』」，原来的弹框总是把焦点放在确认按钮上，回车就确认了；不设时行为不变（`ConfirmDialog.test.tsx` 两例）—— 代价：无。
- Ruling：用纯函数 `triggersMerge(command)` 判断，在 `runToolbar` 和 `runAndRefresh` 这两个统一出口里拦，不在每个调用点各写一遍 —— 新加一个会合并的入口只要走这两个出口就自动被拦 —— 代价：直接 `invoke` 而不走这两个出口的新代码会漏拦（下面的 grep 清单是核对手段）。
- Ruling：SVN 两个流程的 `guardMerge` 是**必填**依赖；放在预览**之前**调用（预览要走网络，用户在这里取消就不用白等）；「继续」要过，「放弃 / 清理 / 停止」不合并、不过 —— 代价：无。
- Ruling：`guardMerge` 里 `check_merge_drivers` 的 invoke 本身抛错时**放行**并报错 —— 后端的检查本身从不报错（失败写在 `incomplete` 里，也不拦，spec 只拦「完全没配」）；invoke 抛错是 IPC 层面的问题，拦住会让所有合并都做不了 —— 代价：极少数情况下缺 driver 没被拦住，但顶部会有错误条。
- Ruling：`mergeDrivers` 在打开仓库时不 await 地刷新，`repo-changed` 时也刷新（后端有缓存）—— 代价：大仓库第一次打开时横幅晚一点出现。
- 已有测试的调整：`svnSyncFlow` / `svnSyncOutFlow` 各有一例断言完整的步骤日志，现在 `clearError` 和预览之间多了 `guardMerge`，期望值随之更新（这是有意的新顺序，不是放宽断言）。

#### 合并入口清单（grep 核对，2026-10-21）

| 入口 | 命令 | 经过的出口 | 是否拦 |
|---|---|---|---|
| 工具栏 Pull / Pull (rebase) | `pull` | `Toolbar.onRun` → `runToolbar` | ✅ |
| 提交右键 Cherry-pick | `cherry_pick` | `handleCommitAction` → `runToolbar` | ✅ |
| 提交右键 Revert | `revert` | `handleCommitAction` → `runToolbar` | ✅ |
| Stash 面板 Apply | `stash_apply` | `StashView.onApply` → `runAndRefresh` | ✅ |
| Stash 面板 Pop | `stash_pop` | `StashView.onPop` → `runAndRefresh` | ✅ |
| SVN「更新」 | `svn_sync_in` | `svnSyncFlow.svnUpdate` | ✅ |
| SVN 横幅「继续」（更新） | `svn_sync_continue` | `svnSyncFlow.syncAction` | ✅ |
| SVN「交付」 | `svn_sync_out` | `svnSyncOutFlow.svnDeliver` | ✅ |
| SVN 横幅「继续」（交付） | `svn_sync_out_continue` | `svnSyncOutFlow.syncOutAction` | ✅ |

- 核对方法：在 `src/` 里 grep 这些命令名和 `svnSyncIn(` 等包装函数的调用点，再对照后端 `commands/` 里会合并的 `#[tauri::command]`（`pull`、`cherry_pick`、`revert`、`stash_apply`、`stash_pop`，以及 svn_sync 四个）。应用里没有单独的 merge / rebase 命令；`svnJournal.ts` / `svnSyncIn.ts` 里的 `'rebase'` 是同步阶段名，不是命令。
- 不拦的：`fetch`、`push`、`stash_push`、`stash_drop`、`checkout`（不带 `-m`，有本地改动时 git 直接拒绝，不合并）、`svn_sync_abort`、`svn_sync_out_abort`。

#### 计划的错误

- 无。

#### 计划没写的坑

- 原来的确认框不支持「默认按钮是取消」，见上面 `focusCancel`。

#### 为什么这么改是安全的

- 不改用户的 git 配置，只提示并给出可复制的配置示例。
- 拦截只是在合并之前多一个确认，用户点取消时什么都不执行（流程测试断言预览和执行都没被调用）。

### Task 12

#### 实测发现的缺陷与修复：`run_git_stdin` 管道死锁

- 现象：5 万文件仓库的 `perf_50k_files` 跑了十几分钟不返回，进程列表里 `git check-attr --stdin -z merge` 一直挂着。
- 根因：`git_cli::run_git_stdin` 先把 stdin 全部写完、再读 stdout。`check-attr --stdin` 是边读边写的命令：输出写满管道缓冲区后 git 阻塞在写 stdout 上、不再读 stdin，我们阻塞在写 stdin 上，两边互等。几十个路径时输出小于缓冲区，所以 Task 5 / Task 10 的测试都没暴露。这正是计划 Review Focus 第 5 条（大仓库的 driver 检查）要防的场景；冲突文件很多时 `path_attrs` 也会撞上。
- 修复：stdin 改在 `std::thread::scope` 的单独线程里写（写完即 drop、关闭管道），主线程同时 `wait_with_output` 把 stdout / stderr 读空；git 先失败退出时先报 stderr，再报写入错误。原来的调用方（`svn_baseline` 的 `hash-object --stdin`）行为不变。
- 测试：`tests/git_cli.rs` 的 `run_git_stdin_does_not_deadlock_on_large_output`，2 万个路径喂给 `check-attr --stdin`，60 秒超时判失败。修复前 60 秒超时失败（RED），修复后 0.43 秒通过（GREEN）。
- 清理：卡死的测试进程与它的 `git check-attr` 子进程是本会话起的，已结束；它留在 `%TEMP%\.tmptNsY6H` 的 5 万文件 fixture 已删除。

#### 大仓库的 driver 检查耗时（引擎层实测）

- `cargo test --release --test merge_drivers -- --ignored --nocapture perf_50k_files`：5 万个被跟踪文件、500 个目录，首次 **272 ms**，命中缓存 **102 ms**（修复死锁之后）。
- 命中缓存时仍有约 100 ms：每次要起 `git config --get-regexp` 和 `git rev-parse --git-path` 两个进程，再遍历一遍 index 算指纹。`repo-changed` 每次触发都会付这个代价，界面上不阻塞（不 await）。如果真实窗口里感到卡，下一步可以把 `rev-parse` 的结果按仓库缓存。

#### 偏离计划

- Ruling：Step 3 / 4（真实窗口实测、用户走 E2E 清单）和 Task 8 / 9 / 11 推迟过来的实测合并成 E2E 清单的 12 条，由用户一次走完 —— 代价：见 Task 8 的 Ruling。
- Ruling：E2E 场景 7 原文是「在应用里点 merge → 弹出确认框」，应用里没有 merge 命令，改为在提交图上 Cherry-pick 分支 `pick` 的提交触发 —— 拦截走的是同一个 `runToolbar` 出口 —— 代价：无。
- Ruling：fixture 的 drivers 仓库用三个专门的名字（`e2e-missing` / `e2e-half` / `e2e-badexe`），不需要用户去注释掉自己的全局 driver 配置 —— 代价：无。
- Ruling：大仓库耗时除了真实窗口那条，另加一个 `#[ignore]` 的引擎层计时测试 `perf_50k_files`，数字可重复测 —— 代价：无。
- Ruling：最终独立 review 交给用户触发 Codex（`/codex:adversarial-review` 是只允许用户调用的命令），和阶段 1 的 review 同一种方式 —— 代价：review 结果回来之前不能宣称完成。

#### 计划没写的坑

- 本机 `C:\Program Files\Unity\Hub\Editor\` 下有 `2022.3.62f3`、`6000.0.32f1`，但都没有 `Editor\Data\Tools\UnityYAMLMerge.exe`；spec 里写的 `6000.6.6f1` 路径不存在。fixture 脚本找不到时 scene 仓库改用假 driver，E2E 第 6 条的「自动弹出 BC」要在装了 Unity 的机器上验证。
- fixture 脚本写文件一律用 .NET API，不用 `Set-Content -Path`（`[1]` 会被当通配符，见 Task 4）。
- Python heredoc 处理含反斜杠的文本两次出错（README 里 JSON 的 `\\` 被折叠、`\U` 被当成转义），含反斜杠的片段改用 Edit 工具写。

#### E2E 清单第一版作废（2026-10-21，用户验收不通过）

- 现象：用户走第 1 条，`Code/P.cs` 用 BC 解完、Stage 都正确，Commit 时 git 报 `Committing is not possible because you have unmerged files`。仓库没有被改动（MERGE_HEAD 还在，其余冲突原样），不是代码缺陷。
- 清单的错误：
  1. fixture 把四种冲突（UU 标记、DU、二进制、特殊路径）塞进同一个 `merge` 仓库。第 1 条要求最后 Commit，但其余三个冲突还没解决，git 一定拒绝提交——这一步按原清单**不可能成功**。写清单时没有按顺序把每一步在 fixture 上推演一遍。
  2. 第 3 条没写清楚操作哪个仓库、哪个文件；「另造一次」没给造法，同一个 DU 文件做完「保留」就没法再测「删除」。
  3. 多处「应看到」只写了大意，没写界面上的确切文案，用户没法逐字比对。
- Ruling：fixture 改成一个场景一个仓库（`01-merge-text`、`02-rebase`、`03a-keep`、`03b-delete`、`05-vscode`、`06-scene`、`07-drivers`、`08-binary`、`09-special-path`、`10-busy`、`diff`），每个冲突仓库只有一个冲突文件，三边内容统一带 `from-main` / `from-feat` / `base`；清单重写成逐步的「操作 → 应看到」，文案逐字取自 `conflictView.ts`、`mergeDrivers.ts`、`LocalChangesView.tsx`、`conflicts.ts`、`external_merge.rs` —— 代价：第一版里第 1 条已经走到 Stage 的结果作废，用户要在新仓库里重走。
- 顺带的调整：原第 3 条只测 DU，拆成 3a（DU 保留）和 3b（UD 删除），两种标题都能看到；原第 11 条不再单独操作，由 1、3a、3b、7 推出；原第 6 条拆成 6a（本机，假 driver）和 6b（需要 Unity，记为未覆盖）；第 9 条去掉「看临时文件名」这一步（BC 里设了标题后看不到路径，临时文件名由 Rust 测试 `chinese_space_bracket_dash_path` 覆盖），改为看结果是否写回了真实文件。
- 新脚本实际跑过一次：11 个仓库状态逐个核对，与脚本头注释一致；`06-scene` 的工作区是 HEAD 那边的 `x: 9`。
- 第二版清单第 5 条的错误：写成「确认 Incoming 栏是 feat、Current 栏是 HEAD」。实测 `code --wait --merge $REMOTE $LOCAL $BASE $MERGED` 打开的合并编辑器两栏上方显示的是文件名（`P.REMOTE.cs` / `P.LOCAL.cs`），「Incoming / Current」只在 VS Code 的 git 插件打开冲突时出现。实际左栏 `P.REMOTE.cs`（`from-feat`）、右栏 `P.LOCAL.cs`（`from-main`），与 git 插件「左 Incoming、右 Current」一致，预设参数顺序正确，只改清单措辞。另：命令行打开时 Result 从共同祖先开始（`No Changes Accepted`），什么都不接受就 Complete Merge 会把 `base` 写进工作区，清单已写明要先 Accept 一边。
- 第 5 条实测：VS Code 合并编辑器点「Complete Merge」后自己 `git add` 了 `$MERGED` 并关闭（VS Code 文档原文「Complete Merge stages that file and closes the merge editor」，经 Context7 `/microsoft/vscode-docs` 核对）。应用在工具退出后只刷新状态与 diff（`RepoTab.tsx` 的 `useExternalToolExit` 回调），不暂存，spec「不自动暂存」的约束对应用本身仍成立。Ruling：不做任何代码改动（拦不住、也不该拦 VS Code 自己的 git 操作），README「不会自动暂存」一句补上 VS Code 的例外，清单第 5 条按实际行为改写 —— 代价：用 VS Code 解决时少了「回应用看过再 Stage」这一步，这是用户选 VS Code 时接受的 VS Code 语义。同一份文档推荐的 `code --wait --merge $REMOTE $LOCAL $BASE $MERGED` 与预设逐字一致。
- 第 7 条实测发现缺陷：「合并前确认」框打开时焦点确实在 Cancel 上（按回车后框关闭、main 没有新提交、没有 `CHERRY_PICK_HEAD`），但**看不出来**——`styles.css` 里 `.dialog-actions button` 没有任何焦点样式，而框是鼠标点出来的、焦点由 `cancelRef.current.focus()` 挪过去，Chromium 不算 `:focus-visible`、不画默认焦点框。红色「仍然继续」看起来反倒像默认按钮，与 spec §4.6「默认按钮是取消」的意图相反。方案 A/B/C 交用户选，用户选 A。Ruling：给 `.dialog-actions button:focus` 加 2px `#4fc3f7` outline（用 `:focus` 不用 `:focus-visible`，理由同上），所有确认框都能看出回车会按哪个 —— 代价：鼠标点按钮那一瞬间按钮上也会出现焦点框（框随即关闭，看不到）；纯 CSS，jsdom 测不了，靠真实窗口复验第 7 条第 3 步。`pnpm build` 通过，`pnpm test` 48 文件 717 例全过。
- 第 7 条复验时发现、不在本分支处理：弹框打开后按 Tab，焦点会移到弹框后面的主界面（菜单栏等）。五个弹框（`ConfirmDialog`、`DeleteBranchDialog`、`DeleteWorktreeDialog`、`DeliverDialog`、`PromptDialog`）都没有 focus trap，master 上就已存在，不是本功能引入的；风险是焦点出框后按回车会触发背后的按钮。Ruling（用户 2026-10-22 选定）：本分支只记录，合并后从最新 master 开新分支按方案 A 修——共用 `useFocusTrap` hook，五个弹框都接上，Tab / Shift+Tab 在框内循环，用 React Testing Library 写回归测试 —— 代价：本分支合并到修复合并之间，这个缺口仍在。
- 计划没写的坑（更正 Task 12 前文「本机没有 UnityYAMLMerge」）：本机 Unity 装在 `E:\unity\6000.3.26f1\`，不在 Unity Hub 默认的 `C:\Program Files\Unity\Hub\Editor\` 下，前面只搜了默认位置。6b 因此可以在本机验证，fixture 脚本用 `-UnityYamlMerge` 传真实路径。
- ~~实测：这种冲突下 UnityYAMLMerge 没有「取对方的值」~~ —— **此条作废**。当时 fixture 的场景文件缺 `m_Father` / `m_Component` 等字段，UnityYAMLMerge 根本解析不了（6b 第 4 步弹窗「Could not determine the transform parent of 400000」），driver 失败与冲突无关。在 scratchpad 用与 driver 相同的参数逐一实测：旧格式「两边改同一个值」和「只有一边改」都是 exit 1、文件原样；补全成 Unity 6 真实格式（含 `SceneRoots`）后，只有一边改 → exit 0、正常合并，两边改同一个值 → **exit 2、结果取对方（`%B`）的 `x: 5`**，与 spec §4.1 和界面说明一致。fixture 脚本的 `Scene` 已改成真实格式，重造后 `06-scene` 停在 `UU`、工作区 `x: 5`。教训：fixture 里给第三方工具吃的文件，要先用该工具单独跑一遍确认它解析得了。
- 6b 实测发现缺陷：外部工具继承应用进程的工作目录（dev 版是 `src-tauri`），UnityYAMLMerge 的报错把这个目录拼在了消息前面。`external_launch::spawn_watched` 没有设 `current_dir`。Ruling：`spawn_watched` 加必填参数 `cwd`，`open_external_diff` / `open_external_merge` 传仓库根目录（`repo_path`，即打开的 worktree 根），与 git difftool / mergetool 一致 —— 代价：无（参数是数组、`$MERGED` 等都是绝对路径，不受工作目录影响）。TDD：先加 `diff_tool_runs_in_repo_root`、`merge_tool_runs_in_repo_root`（fake.cmd 用 `cd` 写出工作目录）和 `spawn_watched_runs_in_given_dir`（rustc 编的小程序写出 `current_dir`，目录名带中文和空格），红灯实测工作目录是 `...\external-tools\src-tauri`；修复后 `cargo fmt --check`、`clippy --all-targets` 无告警，`cargo test -j 4 --no-fail-fast` 62 个二进制 1022 例全过、1 例 ignored（用独立的 `CARGO_TARGET_DIR`，避开 `tauri dev` 正在运行的 `app.exe`）。
- 实测：UnityYAMLMerge 自带的 `mergespecfile.txt` 里 `.unity` / `.prefab` 的专用后备是占位程序 `YouFallbackMergeToolForScenesHere.exe`，不存在，会落到 `*` 行；`*` 行里 PlasticSCM 之后是 `%programs%\Beyond Compare 5\bcomp.exe "%r" "%l" "%b" "%d"`。
- 用户授权（2026-10-22）：改用户真实的 `%APPDATA%\com.momadacoding.jkk\externalTools.json`，加 `unityyamlmerge` 工具与 `*.unity` / `*.prefab` / `*.asset` 的 merge 规则，**验完保留**。改前原文件备份在本会话 scratchpad（`externalTools.json.bak`），改后 JSON 能解析、程序路径存在。
- 6b 实测时用户指出文案矛盾：在 BC 里有意把结果选成 HEAD 那版后，右侧显示「工作区与 HEAD（你这边）完全相同，还没合并」——既然已经完全相同，为什么说还没合并。根因是这句话把推断当事实：应用分不清「git / driver 原样留下 HEAD 那版」和「用户有意选了 HEAD 那版」。方案 A/B 交用户选，用户选 A。Ruling：改为只说事实「工作区与 HEAD（你这边）完全相同：另一边的改动都没有进来」，仍不说「可以暂存」（spec §4.1 总原则）；spec §4.1 表格同步修订 —— 代价：无。TDD：先改 `conflictView.test.ts` 断言，红灯实测收到旧文案；改 `conflictView.ts` 常量后 `pnpm build` 通过、`pnpm test` 48 文件 717 例全过。
- 第 12 条验收后用户发现：选中 5 万文件的根提交 `big`，提交详情显示 `M Assets/D0000/F001.cs +1/-0`，diff 内容是用户刚在工作区加的那一行——把工作区改动当成了这个提交的改动。
  - 根因（实测）：后端 `read_diff` 对根提交是对的（父树为空，全部是新增）。临时探针（release 构建，测完已删）：这个提交的 `read_diff` 要 **47.4 秒**，JSON **10.6 MB**；dev 窗口是 debug 构建，更慢。这期间 store 里只有一份 `diff`，仍是上一个 scope（`workdir`）的，提交详情直接拿来显示。master 上就是这样（`refreshDiff` 回来之前不清旧 diff），不是本分支引入的；平时几毫秒就换掉了看不出来。
  - Ruling（用户 2026-10-22 选定，**偏离计划**：本分支修，而不是另开分支）：store 记下 `diffScope`（这份 diff 是按哪个 scope 读的）；新增纯函数 `src/engine/currentDiff.ts`，scope 与当前视图要的不一致时返回 `null`；`RepoTab` 给提交详情传 `currentDiff(diff, diffScope, commitScope)`、给本地改动传 `currentDiff(…, 'workdir')`；两个视图拿到 `null` 时说「正在读取这个提交的改动…」/「正在读取工作区的改动…」，不列文件、Changes 不带个数。读失败时把这个 scope 记成空 diff（否则界面永远说正在读取），错误照旧抛给调用方 —— 代价：两个视图的 `diff` prop 变成 `DiffData | null`。
  - TDD：先写 `currentDiff.test.ts`（5）、`CommitDetailView.test.tsx`（+2）、`LocalChangesView.test.tsx`（+1），红灯实测模块不存在、视图读 `null.files` 崩溃；`repoStoreDiffRefresh.test.ts`（+2）是改完 store 之后补的，没有单独跑过红灯（断言的 `diffScope` 字段在旧代码里不存在）。`pnpm build` 通过，`pnpm test` 49 文件 727 例全过。
  - 遗留：`useRepoEffects` 里按选中刷新 diff 的 effect 没有 `.catch`，读失败是未处理的 rejection、顶部不报错（master 上就是这样）；现在失败时视图显示为空列表而不是旧 diff，仍可能被误读成「这个提交没有改动」。
- 后续（用户 2026-10-22 要求记录方案，不在本分支做）：超大提交的 diff 太慢、太大（5 万文件 47 秒 / 10.6 MB）。`read_diff` 对 `commit:` 一次性 `find_similar` + 打印所有文件的完整 patch，再整份序列化给前端。方案：
  1. 先定位 47 秒花在哪：`diff_tree_to_tree`、`find_similar`（5 万新增文件的改名检测）、`print(Patch)`、serde 序列化各计时。
  2. 接口分两步：`read_diff_summary` 只返回文件列表与增删行数（`git2::DiffStats` / 逐文件 `Patch::line_stats`，不拼 hunk 文本）；点到某个文件时再 `read_file_diff(scope, path)` 取那一个文件的 patch。`CommitDetailView` 的文件列表与 Changes 树只依赖前者。
  3. 文件数超过阈值（例如 1000）时跳过改名检测，并在界面上说明「文件太多，未检测改名」；或按 `diff.renameLimit` 的思路设上限。
  4. 文件列表本身上万条时，Commit 标签页的平铺列表要虚拟滚动或截断（File Tree 标签页已有「文件太多，只显示前 N 条」的先例）。
- 发现但不在本分支处理：冲突没解决完时点 Commit，界面只显示 git 的英文原文。可以改成有冲突时禁用 Commit 或给中文提示，提交流程不在本计划范围，记为后续事项。
- 6b 实测时用户提出、不在本分支处理（用户 2026-10-22 确认记为后续）：合并结果与 HEAD 相同时，Stage 后 Staged 为空，但仍要 Commit 才能结束合并，界面上只有顶部 `merge in progress` 一条线索，用户会以为没事可做。改进方向：合并进行中且冲突已全部解决时，横幅写「冲突已全部解决，提交以完成合并」，Commit 按钮改叫「提交合并」；应用里也没有放弃合并（`merge --abort`）的入口。与上一条同属提交流程，一起做。
