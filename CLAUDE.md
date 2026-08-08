@AGENTS.md

# CLAUDE.md

## Project Overview

mygitclient 是一个面向个人使用的 Git GUI 桌面客户端（Tauri 2 + React 18 + Rust）。当前 MVP 聚焦「Graph 专家版」：高质量 commit graph + staging/diff/commit + 分支/worktree 管理。

主干分支是 `master`。

## Commands

### Development
```bash
pnpm install
pnpm tauri dev      # 完整桌面应用（Vite dev server + Tauri 窗口 + Rust 热重载）
pnpm dev            # 只起 Vite dev server（默认 http://localhost:5174）
```

涉及 Tauri API 的行为必须在 `pnpm tauri dev` 的真实窗口里验证，浏览器打开 dev server 不等价。

### Build
```bash
pnpm build          # tsc + vite build
```

### Testing
```bash
pnpm test                      # vitest run（前端纯逻辑）

cd src-tauri
cargo test                     # Rust 集成测试（fixture 仓库）
cargo clippy --all-targets
cargo fmt --check
```

本仓库**没有 lint 工具、没有 CI**。前端唯一的静态检查是 `pnpm build` 里的 `tsc`，且 `tsc` 只覆盖 `src/`，不覆盖 `tests/vitest/`。详见 `AGENTS.md` §5。

这些命令是速查，不替代 `AGENTS.md` 中的验证规则与命令发现规则。

## Architecture

### 目录结构
- `src/components/` — React UI 组件
- `src/engine/` — 纯逻辑：commit graph 布局、路径、选择、错误映射（无 React / 无 Tauri 依赖，可直接单测）
- `src/store/` — 全局状态与副作用（`invoke` 统一从这里出去）
- `src-tauri/src/commands/` — `#[tauri::command]` 入口
- `src-tauri/src/engine/` — git 读取引擎
- `src-tauri/tests/` — Rust fixture 仓库集成测试
- `tests/vitest/` — 前端单元测试
- `docs/superpowers/{specs,plans,e2e}/` — 设计规格 / 实现计划 / E2E 清单

### 分层
```text
components → store → engine(前端纯逻辑)
                ↓ invoke
        commands → engine(Rust) → libgit2(读) / git CLI(写)
```

### 新增一个 git 操作的顺序
1. 在 `src-tauri/src/engine/` 实现逻辑，走 `run_git` 或 git2
2. 在 `src-tauri/src/commands/` 暴露 `#[tauri::command]`，错误映射到 `GitError`
3. 在 `src-tauri/tests/` 用 fixture 仓库补集成测试
4. 在 `src/store/` 接 `invoke`，注意 `snake_case` → `camelCase` 参数名转换
5. 在组件里接 UI，纯逻辑下沉到 `src/engine/` 并补 vitest

## Tech Stack

- Tauri 2 + Rust（桌面外壳与后端命令）
- React 18 + TypeScript + Vite
- libgit2（`git2`）负责读取：revwalk、diff、refs
- git CLI 负责写入：commit、branch、checkout、stage、worktree
- SVG 渲染 commit graph
- notify 做文件系统监听
- 测试：Vitest（前端）、cargo test（Rust）

## Claude Code Workflow

Claude 是主要实现者，Codex 主要作为 reviewer，偶尔承担叶子任务。

- 多文件、约定密集、架构敏感的工作，自己实现，不要外包。
- 每完成一个非平凡里程碑或跨文件补丁，先跑一次 review 再继续。
- 收尾前跑一次最终 review。review 发现高危问题必须修完再停。
- 可以委派：小脚本、脚手架、独立工具函数、范围明确的测试。
- 不要委派：架构改动、约定密集的重构、跨模块行为变更。

## Output Format

完成任务时报告：

1. 改了什么
2. 实际跑了哪些命令、结果如何（未跑的说明原因）
3. 遗留风险
4. review 发现并已修复的问题
