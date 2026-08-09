# mygitclient

一个面向个人使用的 Git GUI 客户端，目标是把 Fork 的好用 worktree 支持、Sublime Merge 级别的 commit graph，以及编辑器/终端整合放到一起。当前 MVP 聚焦在「Graph 专家版」：高质量提交图 + staging/diff/commit + 分支/worktree 管理。

## 技术栈

- **Tauri 2** + **Rust**：跨平台桌面外壳与后端命令
- **React 18** + **TypeScript**：前端 UI
- **libgit2（git2-rs）**：高性能读取（revwalk、diff、refs）
- **git CLI**：写入操作（commit、branch、checkout、stage、worktree 等），保证与原生 git 行为一致
- **SVG**：commit graph 渲染（可替换为 Canvas）
- **notify**：文件系统监听，自动刷新

## 环境要求

- [Rust](https://rustup.rs/) >= 1.77.2
- [Node.js](https://nodejs.org/) + [pnpm](https://pnpm.io/)
- Windows / macOS / Linux 均可（当前主要在 Windows 11 开发）
- 本地已安装 `git`

## 目录结构

```
.
├── docs/superpowers/
│   ├── specs/    # 设计规格文档
│   ├── plans/    # 实现计划
│   └── e2e/      # E2E 验证清单
├── src/              # React 前端
│   ├── components/   # UI 组件
│   ├── engine/       # 提交图布局/路径/SVG 渲染
│   ├── store/        # 全局状态与副作用
│   └── App.tsx
├── src-tauri/        # Rust 后端
│   ├── src/
│   │   ├── commands/ # invoke 命令
│   │   ├── engine/   # git 读取引擎
│   │   └── main.rs
│   └── tests/        # Rust fixtures 测试
└── tests/vitest/     # 前端单元测试
```

## 安装依赖

```bash
pnpm install
```

Rust 依赖在首次 `cargo build`/`pnpm tauri dev` 时自动拉取。

## 开发启动

```bash
pnpm tauri dev
```

这会同时启动：
- Vite 开发服务器（默认 `http://localhost:5174`）
- Tauri 桌面窗口
- Rust 后端热重载

> 如果端口 5174 被占用，先杀掉占用进程再启动。

## 构建

```bash
pnpm tauri build
```

产物在 `src-tauri/target/release/bundle/`。

## 测试

前端单元测试：

```bash
pnpm test
```

Rust 测试：

```bash
cd src-tauri
cargo test
```

类型检查：

```bash
pnpm exec tsc --noEmit
cd src-tauri && cargo check
```

## MVP 功能范围

- [x] 打开本地仓库，读取 HEAD
- [x] Sublime Merge 风格的 commit graph（SVG 渲染、viewport 裁剪）
- [x] 点击提交查看 diff
- [x] Staging / unstaging / commit / amend
- [x] 分支查看、checkout、创建、删除
- [x] Worktree 查看、切换、创建、删除
- [x] 文件系统监听自动刷新
- [x] merge / rebase / cherry-pick 状态横幅
- [x] 大仓库提交数上限保护（5000 commits）
- [x] 工具栏自定义命令（见下节）

## 工具栏自定义命令

工具栏右侧可以配置任意本地命令，用来把当前 worktree 交给外部程序，
比如在编辑器里打开、在该目录起一个终端、或者在文件管理器里定位。

配置文件位置：

| 平台 | 路径 |
|---|---|
| Windows | `%APPDATA%\com.tauri.dev\customCommands.json` |
| macOS | `~/Library/Application Support/com.tauri.dev/customCommands.json` |
| Linux | `~/.config/com.tauri.dev/customCommands.json` |

首次启动会写出一份当前平台的默认预设。点工具栏最右侧的 `⚙` →
**Edit commands…** 用系统默认程序打开它，改完点 **Reload** 生效（不会自动重载）。

```json
{
  "version": 1,
  "commands": [
    { "id": "vscode", "label": "VS Code", "program": "code", "args": ["$WORKTREE"] }
  ]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | 是 | 稳定标识 |
| `label` | 是 | 按钮上的文字 |
| `program` | 是 | 可执行文件名或绝对路径 |
| `args` | 否 | 参数数组，默认 `[]`。逐项做占位符替换 |
| `cwd` | 否 | 子进程工作目录，默认 `"$WORKTREE"` |

可用占位符：`$WORKTREE`（当前 worktree 绝对路径）、`$BRANCH`、`$HEAD`（完整 oid）。
未知占位符原样保留。

两点说明：

- 参数以数组形式直接传给系统，**不经过 shell 解释**。含空格或 `&` 的仓库路径是安全的，
  但也因此写不了 `cd x && y` 这类复合命令——需要的话把它包成一个脚本文件再调。
- JSON 写错时程序**不会覆盖你的文件**，只会在 `⚙` 上显示 `⚠` 并把解析错误显示出来。

## 设计文档

- 设计规格：`docs/superpowers/specs/2026-08-04-mygitclient-design.md`
- 实现计划：
  - `docs/superpowers/plans/2026-08-04-mygitclient-engine-and-skeleton-plan.md`
  - `docs/superpowers/plans/2026-08-04-mygitclient-graph-rendering-plan.md`
  - `docs/superpowers/plans/2026-08-04-mygitclient-ui-closure-plan.md`
- E2E 验证清单：`docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md`
- 工具栏自定义命令：`docs/superpowers/specs/2026-08-09-custom-toolbar-commands-design.md`、
  `docs/superpowers/plans/2026-08-09-custom-toolbar-commands.md`、
  `docs/superpowers/e2e/2026-08-09-custom-toolbar-commands.md`

## 注意事项

- 当前默认打开的路径是 `C:\Users\liuhao\workspace\dev\Dev\mygitclient`，在 `src/App.tsx` 中硬编码，后续会改成启动时选择或记忆上次路径。
- 写入操作均通过 git CLI 执行，因此要求工作目录在有效的 git 仓库内。
- 自动刷新监听整个工作区，但只响应 `.git/` 下的变更事件。
