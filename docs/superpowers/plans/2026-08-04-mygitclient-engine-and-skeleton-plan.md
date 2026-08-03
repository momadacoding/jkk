# mygitclient 实施计划 #1：Tauri 骨架 + Rust git 引擎

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 搭建 Tauri 2 + React + TypeScript 项目骨架，实现 Rust git 引擎层，让前端能通过 IPC 打开仓库并读取 commit 拓扑与 refs 数据。

**Architecture:** Rust 侧封装 libgit2 读取能力为无状态命令式 API；Tauri 通过 `invoke` 暴露给前端；复杂解析与布局全部放在前端 TS 侧。数据以紧凑 JSON 跨 IPC 传输。

**Tech Stack:** Tauri 2, Rust, git2-rs, React, TypeScript, Vite, pnpm.

## Global Constraints

- Rust 侧代码保持无状态，每个 handler 内 `Repository::open(path)`，不维护全局句柄。
- 所有复杂算法（布局、diff 渲染）放在 TS 侧；Rust 只负责读取与 CLI 转发。
- 错误统一为 `GitError { code, message }`，前端提示条展示。
- Windows 11 为主，Tauri 配置默认 Windows，但路径与 IPC 不绑定平台。
- 测试：Rust 侧用 fixture 仓库；TS 侧用 Vitest；每个 task 必须能独立验证。
- 颜色与 UI token 通过 CSS 变量管理。
- 提交信息遵循：`<type>: <description>`，末尾追加 `Co-Authored-By: Claude <noreply@anthropic.com>`。

---

### Task 1: 初始化 Tauri 2 + React + TypeScript 项目骨架

**Files:**
- Create: `package.json`
- Create: `src-tauri/Cargo.toml`
- Create: `src-tauri/tauri.conf.json`
- Create: `src-tauri/src/main.rs`
- Create: `index.html`
- Create: `vite.config.ts`
- Create: `tsconfig.json`
- Create: `src/main.tsx`
- Create: `src/App.tsx`
- Modify: `.gitignore`

**Interfaces:**
- Produces: `pnpm install` 后能运行 `pnpm tauri dev` 并弹出空白窗口。

- [ ] **Step 1: 创建根 package.json**

```json
{
  "name": "mygitclient",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri",
    "test": "vitest run"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "@tauri-apps/plugin-shell": "^2.0.0",
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "@types/react": "^18.3.12",
    "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.3",
    "typescript": "^5.6.3",
    "vite": "^5.4.11",
    "vitest": "^2.1.5"
  }
}
```

- [ ] **Step 2: 初始化 Tauri 配置**

Run: `pnpm dlx tauri@latest init --app-name mygitclient --window-title mygitclient --frontend-dir . --dev-url http://localhost:5173 --before-dev-command "pnpm dev" --before-build-command "pnpm build"`

Expected: 生成 `src-tauri/` 目录、`tauri.conf.json`、`Cargo.toml`、`src/main.rs`。

- [ ] **Step 3: 调整 vite.config.ts 支持 @ 别名**

```ts
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
})
```

- [ ] **Step 4: 创建最简 React 入口**

`src/main.tsx`:
```tsx
import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App'

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
```

`src/App.tsx`:
```tsx
function App() {
  return <div>mygitclient v0.1</div>
}

export default App
```

- [ ] **Step 5: 运行 Tauri 开发窗口**

Run: `pnpm install && pnpm tauri dev`

Expected: 弹出桌面窗口显示 "mygitclient v0.1"，无报错。

- [ ] **Step 6: 提交骨架**

```bash
git add .
git commit -m "chore: scaffold Tauri 2 + React + TypeScript project

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: Rust 侧定义统一错误类型

**Files:**
- Create: `src-tauri/src/error.rs`
- Modify: `src-tauri/src/main.rs`

**Interfaces:**
- Produces: `GitError` struct, `Result<T>` alias, `impl Serialize` for Tauri IPC.

- [ ] **Step 1: 编写 error.rs**

```rust
use serde::Serialize;

pub type Result<T> = std::result::Result<T, GitError>;

#[derive(Debug, Serialize, Clone)]
pub struct GitError {
    pub code: String,
    pub message: String,
}

impl GitError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }
}

impl From<git2::Error> for GitError {
    fn from(err: git2::Error) -> Self {
        Self::new("git2", format!("{}: {}", err.code(), err.message()))
    }
}

impl From<std::io::Error> for GitError {
    fn from(err: std::io::Error) -> Self {
        Self::new("io", err.to_string())
    }
}
```

- [ ] **Step 2: 在 main.rs 注册模块**

Add to `src-tauri/src/main.rs`:
```rust
mod error;
```

- [ ] **Step 3: 编译通过**

Run: `cd src-tauri && cargo check`

Expected: 无错误。

- [ ] **Step 4: 提交**

```bash
git add src-tauri/src/error.rs src-tauri/src/main.rs
git commit -m "feat: add GitError type for uniform IPC error handling

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: 实现 `open_repo` 命令

**Files:**
- Create: `src-tauri/src/engine/repo.rs`
- Create: `src-tauri/src/engine/mod.rs`
- Create: `src-tauri/src/commands/repo.rs`
- Create: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/main.rs`
- Create: `src/engine/types.ts`

**Interfaces:**
- Produces Rust: `engine::repo::open(path: &str) -> Result<RepoInfo>`
- Produces TS: `invoke('open_repo', { path: string }): Promise<RepoInfo>`

- [ ] **Step 1: 编写 engine/repo.rs**

```rust
use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize)]
pub struct RepoInfo {
    pub path: String,
    pub head: String,
}

pub fn open(path: &str) -> Result<RepoInfo> {
    let repo = Repository::open(path).map_err(|_| {
        GitError::new("not_repo", format!("{} is not a git repository", path))
    })?;
    let head = repo.head()?;
    let head_name = head.shorthand().unwrap_or("HEAD").to_string();
    Ok(RepoInfo {
        path: path.to_string(),
        head: head_name,
    })
}
```

- [ ] **Step 2: 编写 engine/mod.rs**

```rust
pub mod repo;
```

- [ ] **Step 3: 编写 commands/repo.rs**

```rust
use crate::engine;

#[tauri::command]
pub fn open_repo(path: String) -> Result<crate::engine::repo::RepoInfo, crate::error::GitError> {
    engine::repo::open(&path)
}
```

- [ ] **Step 4: 编写 commands/mod.rs**

```rust
pub mod repo;
```

- [ ] **Step 5: 注册命令到 main.rs**

```rust
mod commands;
mod engine;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::repo::open_repo])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **Step 6: 前端类型文件**

`src/engine/types.ts`:
```ts
export interface RepoInfo {
  path: string
  head: string
}
```

- [ ] **Step 7: 前端按钮测试 open_repo**

Modify `src/App.tsx`:
```tsx
import { invoke } from '@tauri-apps/api/core'
import { useState } from 'react'
import type { RepoInfo } from './engine/types'

function App() {
  const [info, setInfo] = useState<RepoInfo | null>(null)

  const handleOpen = async () => {
    const result = await invoke<RepoInfo>('open_repo', { path: 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient' })
    setInfo(result)
  }

  return (
    <div>
      <button onClick={handleOpen}>Open repo</button>
      {info && <pre>{JSON.stringify(info, null, 2)}</pre>}
    </div>
  )
}

export default App
```

Run: `pnpm tauri dev` and click the button.

Expected: 显示 `{ "path": "...", "head": "master" }`（或当前分支名）。

- [ ] **Step 8: 提交**

```bash
git add .
git commit -m "feat: add open_repo command

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 实现 `load_graph` 命令

**Files:**
- Create: `src-tauri/src/engine/graph.rs`
- Create: `src-tauri/src/engine/refs.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Create: `src-tauri/src/commands/graph.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/engine/types.ts`

**Interfaces:**
- Produces Rust: `engine::graph::load(path) -> Result<GraphData>`
- Produces Rust: `engine::refs::collect(&repo) -> Result<Vec<RefPointer>>`
- Produces TS: `invoke('load_graph', { path: string }): Promise<GraphData>`
- GraphData: `{ commits: CommitNode[], refs: RefPointer[] }`

- [ ] **Step 1: 设计 CommitNode 数据结构**

`src-tauri/src/engine/graph.rs`:
```rust
use crate::error::{GitError, Result};
use git2::{Repository, Revwalk, Oid};
use serde::Serialize;

#[derive(Serialize)]
pub struct CommitNode {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    pub message: String,
}

#[derive(Serialize)]
pub struct GraphData {
    pub commits: Vec<CommitNode>,
    pub refs: Vec<crate::engine::refs::RefPointer>,
}

pub fn load(path: &str) -> Result<GraphData> {
    let repo = Repository::open(path)?;
    let mut walk = repo.revwalk()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL)?;
    walk.push_head()?;
    for branch in repo.branches(None)? {
        let (branch, _) = branch?;
        if let Some(oid) = branch.get().target() {
            walk.push(oid)?;
        }
    }

    let mut commits = Vec::new();
    for oid in walk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        commits.push(CommitNode {
            id: oid.to_string(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            message: commit.summary().unwrap_or("").to_string(),
        });
    }

    let refs = crate::engine::refs::collect(&repo)?;
    Ok(GraphData { commits, refs })
}
```

- [ ] **Step 2: 实现 refs.rs**

`src-tauri/src/engine/refs.rs`:
```rust
use crate::error::Result;
use git2::Repository;
use serde::Serialize;

#[derive(Serialize)]
pub struct RefPointer {
    pub name: String,
    pub hash: String,
    pub kind: String, // "head", "branch", "remote", "tag"
}

pub fn collect(repo: &Repository) -> Result<Vec<RefPointer>> {
    let mut refs = Vec::new();
    for reference in repo.references()? {
        let reference = reference?;
        let name = reference.name().unwrap_or("").to_string();
        let kind = if name == "HEAD" {
            "head"
        } else if name.starts_with("refs/heads/") {
            "branch"
        } else if name.starts_with("refs/remotes/") {
            "remote"
        } else if name.starts_with("refs/tags/") {
            "tag"
        } else {
            "other"
        };
        if let Some(oid) = reference.target() {
            refs.push(RefPointer {
                name: name.clone(),
                hash: oid.to_string(),
                kind: kind.to_string(),
            });
        }
    }
    Ok(refs)
}
```

- [ ] **Step 3: 注册模块与命令**

Update `src-tauri/src/engine/mod.rs`:
```rust
pub mod graph;
pub mod refs;
pub mod repo;
```

Update `src-tauri/src/commands/graph.rs`:
```rust
use crate::engine;

#[tauri::command]
pub fn load_graph(path: String) -> Result<crate::engine::graph::GraphData, crate::error::GitError> {
    engine::graph::load(&path)
}
```

Update `src-tauri/src/commands/mod.rs`:
```rust
pub mod graph;
pub mod repo;
```

Update `src-tauri/src/main.rs`:
```rust
.invoke_handler(tauri::generate_handler![
    commands::repo::open_repo,
    commands::graph::load_graph,
])
```

- [ ] **Step 4: 前端类型更新**

`src/engine/types.ts`:
```ts
export interface RefPointer {
  name: string
  hash: string
  kind: 'head' | 'branch' | 'remote' | 'tag' | 'other'
}

export interface CommitNode {
  id: string
  parents: string[]
  author: string
  time: number
  message: string
}

export interface GraphData {
  commits: CommitNode[]
  refs: RefPointer[]
}
```

- [ ] **Step 5: 前端按钮测试 load_graph**

Modify `src/App.tsx` to add a "Load graph" button that invokes `load_graph` and prints `commits.length`.

Run: `pnpm tauri dev`

Expected: 点击后显示 commit 数量（对本仓库应 > 0）。

- [ ] **Step 6: 提交**

```bash
git add .
git commit -m "feat: add load_graph command with refs

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: Rust 层 fixture 测试

**Files:**
- Create: `src-tauri/tests/fixtures.rs`
- Create: `tests/rust_fixtures/.gitkeep`

**Interfaces:**
- Produces: cargo test that creates a temp repo, makes commits, and asserts `load()` returns expected topology.

- [ ] **Step 1: 创建 fixtures 测试文件**

`src-tauri/tests/fixtures.rs`:
```rust
use mygitclient::engine;
use std::process::Command;
use std::fs;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success());
}

fn linear_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "a.txt"]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    dir
}

#[test]
fn load_linear_graph() {
    let dir = linear_repo();
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(data.commits.len(), 2);
    assert_eq!(data.commits[0].parents.len(), 1); // c2 -> c1
    assert_eq!(data.commits[1].parents.len(), 0); // c1 root
}

#[test]
fn open_invalid_repo_fails() {
    let dir = TempDir::new().unwrap();
    let result = engine::repo::open(dir.path().to_str().unwrap());
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "not_repo");
}
```

- [ ] **Step 2: 添加 dev-dependencies**

`src-tauri/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3.14"
```

Update `src-tauri/src/lib.rs` or expose engine as public:
Create `src-tauri/src/lib.rs`:
```rust
pub mod engine;
pub mod error;
```

And ensure `src-tauri/Cargo.toml` has:
```toml
[lib]
name = "mygitclient"
path = "src/lib.rs"
```

- [ ] **Step 3: 运行测试**

Run: `cd src-tauri && cargo test`

Expected: 两个测试都通过。

- [ ] **Step 4: 提交**

```bash
git add .
git commit -m "test: add Rust fixture tests for repo opening and graph loading

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 添加 `read_status` 与 `read_diff` 命令

**Files:**
- Create: `src-tauri/src/engine/diff.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Create: `src-tauri/src/commands/diff.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/engine/types.ts`

**Interfaces:**
- Produces Rust: `engine::diff::read_status(path) -> Result<StatusData>`
- Produces Rust: `engine::diff::read_diff(path, scope) -> Result<DiffData>`
- Produces TS: `invoke('read_status', { path })` / `invoke('read_diff', { path, scope })`

- [ ] **Step 1: 定义 diff 数据结构**

`src-tauri/src/engine/diff.rs`:
```rust
use crate::error::Result;
use git2::{Repository, DiffOptions};
use serde::Serialize;

#[derive(Serialize)]
pub struct FileChange {
    pub path: String,
    pub status: String, // "modified", "added", "deleted", "renamed"
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Serialize)]
pub struct StatusData {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
}

#[derive(Serialize)]
pub struct DiffLine {
    pub origin: char, // '+', '-', ' '
    pub content: String,
}

#[derive(Serialize)]
pub struct DiffHunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Serialize)]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Serialize)]
pub struct DiffData {
    pub files: Vec<FileDiff>,
}
```

- [ ] **Step 2: 实现 read_status**

```rust
pub fn read_status(path: &str) -> Result<StatusData> {
    let repo = Repository::open(path)?;
    let statuses = repo.statuses(None)?;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();
        let path_str = entry.path().unwrap_or("").to_string();
        let change = FileChange {
            path: path_str,
            status: if status.contains(git2::Status::INDEX_NEW) { "added" }
                    else if status.contains(git2::Status::INDEX_DELETED) { "deleted" }
                    else if status.contains(git2::Status::INDEX_RENAMED) { "renamed" }
                    else { "modified" }.to_string(),
            additions: 0,
            deletions: 0,
        };
        if status.intersects(git2::Status::INDEX_NEW | git2::Status::INDEX_MODIFIED | git2::Status::INDEX_DELETED | git2::Status::INDEX_RENAMED) {
            staged.push(change.clone());
        }
        if status.intersects(git2::Status::WT_NEW | git2::Status::WT_MODIFIED | git2::Status::WT_DELETED | git2::Status::WT_RENAMED) {
            unstaged.push(change);
        }
    }

    Ok(StatusData { staged, unstaged })
}
```

- [ ] **Step 3: 实现 read_diff（工作区 vs commit scope）**

```rust
pub fn read_diff(path: &str, scope: String) -> Result<DiffData> {
    let repo = Repository::open(path)?;
    if let Some(hash) = scope.strip_prefix("commit:") {
        let commit = repo.find_commit(git2::Oid::from_str(hash)?)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        return diff_to_data(diff);
    }
    // default: workdir vs HEAD
    let head = repo.head()?;
    let tree = head.peel_to_tree()?;
    let diff = repo.diff_tree_to_workdir_with_index(Some(&tree), Some(DiffOptions::new().context_lines(3)))?;
    diff_to_data(diff)
}

fn diff_to_data(diff: git2::Diff) -> Result<DiffData> {
    let mut files = Vec::new();
    diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        let path = delta.new_file().path().unwrap_or(delta.old_file().path().unwrap_or(std::path::Path::new(""))).to_string_lossy().to_string();
        let hunk_info = hunk.map(|h| (h.old_start(), h.new_start())).unwrap_or((0, 0));
        let origin = line.origin();
        let content = std::str::from_utf8(line.content()).unwrap_or("").trim_end_matches('\n').to_string();

        if files.is_empty() || files.last().unwrap().path != path {
            files.push(FileDiff { path, hunks: Vec::new() });
        }
        let file = files.last_mut().unwrap();
        if file.hunks.is_empty() || file.hunks.last().unwrap().old_start != hunk_info.0 {
            file.hunks.push(DiffHunk { old_start: hunk_info.0, new_start: hunk_info.1, lines: Vec::new() });
        }
        file.hunks.last_mut().unwrap().lines.push(DiffLine { origin, content });
        true
    })?;
    Ok(DiffData { files })
}
```

- [ ] **Step 4: 注册命令与类型**

`src-tauri/src/commands/diff.rs`:
```rust
use crate::engine;

#[tauri::command]
pub fn read_status(path: String) -> Result<crate::engine::diff::StatusData, crate::error::GitError> {
    engine::diff::read_status(&path)
}

#[tauri::command]
pub fn read_diff(path: String, scope: String) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    engine::diff::read_diff(&path, scope)
}
```

Update `src-tauri/src/main.rs` handler list.

Update `src/engine/types.ts`:
```ts
export interface FileChange {
  path: string
  status: 'modified' | 'added' | 'deleted' | 'renamed'
  additions: number
  deletions: number
}

export interface StatusData {
  staged: FileChange[]
  unstaged: FileChange[]
}

export interface DiffLine {
  origin: '+' | '-' | ' '
  content: string
}

export interface DiffHunk {
  oldStart: number
  newStart: number
  lines: DiffLine[]
}

export interface FileDiff {
  path: string
  hunks: DiffHunk[]
}

export interface DiffData {
  files: FileDiff[]
}
```

- [ ] **Step 5: fixture 测试**

Add to `src-tauri/tests/fixtures.rs`:
```rust
#[test]
fn read_status_detects_unstaged() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].status, "modified");
}
```

Run: `cd src-tauri && cargo test`
Expected: 全部通过。

- [ ] **Step 6: 提交**

```bash
git add .
git commit -m "feat: add read_status and read_diff commands

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Tauri 骨架 ✅ Task 1
- Rust 无状态命令式 API ✅ Task 2-6
- libgit2 读取 commit graph ✅ Task 4
- refs/worktree 指针读取 ✅ Task 4
- diff/status 读取 ✅ Task 6
- 错误统一 `GitError` ✅ Task 2
- Rust fixture 测试 ✅ Task 5, 6

**Placeholder scan:** 无 TBD/TODO/占位符。

**Type consistency:** Rust `CommitNode` 与 TS `CommitNode` 字段一致；Rust `RefPointer` 的 `kind` 是 `String`，TS 使用联合类型。

**未覆盖（留给 Plan 2/3）：** graph 布局算法、SVG 渲染、UI 面板、刷新机制、写操作命令。
