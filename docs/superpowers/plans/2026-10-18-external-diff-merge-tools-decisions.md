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
| 2 外部进程启动、锁与临时目录 | 未开始 | | |
| 3 按字节导出与外部 diff 命令 | 未开始 | | |
| 4 前端外部 diff 入口与刷新代次 | 未开始 | | |
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
