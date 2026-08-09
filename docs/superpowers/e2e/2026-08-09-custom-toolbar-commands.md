# 工具栏自定义命令 E2E 验证清单

- 日期：2026-08-09
- 分支：`worktree-custom-toolbar-commands`
- 验证环境：Windows 11 Pro 10.0.26200，`rustc 1.95.0`，`pnpm 10.28.0`

## 验证方式说明

本清单的条目分三类，**不要把后两类当成已通过**：

| 标记 | 含义 |
|---|---|
| ✅ 自动化 | 有对应的 `cargo test` / `pnpm test` 用例，可重复执行 |
| ✅ 真机观测 | 在 `pnpm tauri dev` 真实窗口启动后，通过可观测的副作用确认 |
| ⬜ 待人工 | 需要在窗口里点击才能确认，**尚未验证** |

## 清单

| # | 场景 | 结果 | 证据 |
|---|---|---|---|
| 1 | 首次启动创建 `%APPDATA%\com.tauri.dev\customCommands.json`，内容为三条默认预设 | ✅ 真机观测 | `pnpm tauri dev` 启动后该文件存在，内容为 vscode / terminal / explorer 三条，字段与序列化格式正确 |
| 2 | 未打开仓库时三个按钮置灰，`⚙` 仍可点 | ⬜ 待人工 | 逻辑侧由 `planCustomCommand` 在 `worktree === null` 时返回 `null` 保证（vitest 覆盖），但**置灰的视觉效果未在窗口中确认** |
| 3 | Explorer 按钮 → 文件管理器定位到当前 worktree | ⬜ 待人工 | — |
| 4 | Terminal 按钮 → 终端在该 worktree 目录打开 | ⬜ 待人工 | — |
| 5 | VS Code 按钮 → 编辑器打开该 worktree；未安装时报「找不到可执行文件 `code`，请检查 customCommands.json」 | ⬜ 待人工 | 报错文案由 `spawn_detached_reports_a_missing_executable` 覆盖，但**未在界面横幅上确认呈现** |
| 6 | `⚙` → Edit commands… → 配置文件被默认程序打开 | ⬜ 待人工 | — |
| 7 | 改配置加一条命令 → `⚙` → Reload → 新按钮出现 | ⬜ 待人工 | — |
| 8 | 配置改成非法 JSON → Reload → `⚙` 变 `⚠`、按钮组清空、**文件未被改写** | ⬜ 待人工 | 「文件未被改写」由 `broken_json_errors_and_leaves_file_untouched` 按字节断言覆盖；UI 降级表现待人工确认 |
| 9 | 路径含空格与非 ASCII 字符的仓库，命令仍能正确定位 | ✅ 自动化 | `spawn_detached_survives_spaces_ampersands_and_non_ascii`：目录名 `我的 repo & test`，路径同时作为 cwd 和参数传入，`git init` 成功建出 `.git` |

## 关于第 9 条

原计划要求手工新建一个名为 `我的 repo & test` 的 fixture 仓库并在窗口里点按钮。
改成了自动化测试，理由：

- 手点一次只覆盖一次，回归时没人会重做
- `&` 是 shell 的命令分隔符，一旦哪层把 argv 拼成字符串，失败形态是
  「执行了半截路径当命令」——这种缺陷靠肉眼看窗口很难判定
- 自动化用例把路径**同时**作为 cwd 和参数传入，比手点覆盖面更宽

## 已知未覆盖

- **第 2-8 条需要在真实窗口里点击确认。** 本轮未执行，原因：实现过程无法操作 GUI。
  合并前应由人在 `pnpm tauri dev` 窗口中走完。
- **Linux 的 `x-terminal-emulator` 预设无验证条件**，本项目主要在 Windows 上开发。
- **`.cmd` 转义行为实测在 `rustc 1.95.0` 上完成**，`Cargo.toml` 声明的 MSRV 是
  `1.77.2`，未在下限工具链上直接验证。详见决策日志 Task 2。

## 自动化用例清单

```bash
cd src-tauri && cargo test --test custom_commands   # 14 例
pnpm test                                           # 159 例（其中 12 例为本功能新增）
```
