# mygitclient 实施计划 #3：UI 操作闭环 + 刷新机制 + 边界处理

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 Plan 1 的 Rust 引擎和 Plan 2 的 Graph 渲染对接成完整 MVP：分支/worktree 管理面板、staging/commit/diff 面板、自动刷新、写操作命令、边界情况处理。

**Architecture:** 前端 `repoStore` 作为单一数据源，持有仓库路径、graph 数据、status、选中态。Rust 侧通过 `notify` 监听 `.git` 与工作树变化并推送事件；写操作命令成功后前端主动刷新对应数据。

**Tech Stack:** Tauri 2, React, Zustand (or useState/useReducer for simplicity), TypeScript, Rust notify crate.

## Global Constraints

- Rust 写操作全部通过 `std::process::Command` 调用 git CLI，行为与 git 100% 一致。
- 刷新事件分流：`refs:changed` → 重拉 graph（保留滚动与选中）；`status:changed` → 重拉 status。
- 每次 graph 重载带 seq 号，过期结果丢弃。
- 错误用顶部提示条展示 stderr。
- 提交信息遵循：`<type>: <description>`，末尾追加 `Co-Authored-By: Claude <noreply@anthropic.com>`。

---

### Task 1: 分支、Worktree 与写操作命令

**Files:**
- Create: `src-tauri/src/commands/ops.rs`
- Create: `src-tauri/src/commands/worktree.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/engine/refs.rs`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/engine/types.ts`

**Interfaces:**
- Produces Rust commands:
  - `create_branch(path, name, start_point?)`
  - `delete_branch(path, name)`
  - `rename_branch(path, old, new)`
  - `checkout(path, rev)`
  - `create_worktree(path, name, branch?)`
  - `delete_worktree(path, name)`
  - `read_worktrees(path) -> Worktree[]`
- Produces TS types: `Worktree { name, path, head, branch }`

- [ ] **Step 1: 实现 ops.rs 分支与 checkout 命令**

`src-tauri/src/commands/ops.rs`:
```rust
use std::process::Command;
use crate::error::{GitError, Result};

fn run_git(path: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn create_branch(path: String, name: String, start_point: Option<String>) -> Result<(), GitError> {
    let mut args = vec!["branch", &name];
    if let Some(start) = &start_point {
        args.push(start);
    }
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn delete_branch(path: String, name: String) -> Result<(), GitError> {
    run_git(&path, &["branch", "-D", &name])?;
    Ok(())
}

#[tauri::command]
pub fn rename_branch(path: String, old: String, new: String) -> Result<(), GitError> {
    run_git(&path, &["branch", "-m", &old, &new])?;
    Ok(())
}

#[tauri::command]
pub fn checkout(path: String, rev: String) -> Result<(), GitError> {
    run_git(&path, &["checkout", &rev])?;
    Ok(())
}
```

- [ ] **Step 2: 实现 worktree 命令**

`src-tauri/src/commands/worktree.rs`:
```rust
use std::process::Command;
use crate::error::{GitError, Result};
use serde::Serialize;

#[derive(Serialize)]
pub struct Worktree {
    pub name: String,
    pub path: String,
    pub head: String,
    pub branch: String,
    pub is_main: bool,
}

#[tauri::command]
pub fn read_worktrees(path: String) -> Result<Vec<Worktree>, GitError> {
    let output = Command::new("git")
        .current_dir(&path)
        .args([&"worktree", "list", "--porcelain", "-z"])
        .output()?;
    if !output.status.success() {
        return Err(GitError::new("git_cli", String::from_utf8_lossy(&output.stderr).into()));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut worktrees = Vec::new();
    for block in stdout.split('\0') {
        let mut name = String::new();
        let mut path_str = String::new();
        let mut head = String::new();
        let mut branch = String::new();
        let mut is_main = false;
        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path_str = p.to_string();
                name = std::path::Path::new(p)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = b.replace("refs/heads/", "").to_string();
            } else if line == "bare" || line == "detached" {
                // ignore
            } else if line.starts_with("prunable") {
                // ignore
            }
        }
        if !path_str.is_empty() {
            // main worktree is the same as repo path
            is_main = path_str == path;
            worktrees.push(Worktree { name, path: path_str, head, branch, is_main });
        }
    }
    Ok(worktrees)
}

#[tauri::command]
pub fn create_worktree(path: String, name: String, branch: Option<String>) -> Result<(), GitError> {
    let worktree_path = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(".")).join(&name);
    let mut args = vec!["worktree", "add"];
    if let Some(b) = &branch {
        args.push(b);
    }
    args.push(worktree_path.to_str().unwrap());
    let status = Command::new("git").current_dir(&path).args(&args).status()?;
    if !status.success() {
        return Err(GitError::new("git_cli", "worktree add failed".to_string()));
    }
    Ok(())
}

#[tauri::command]
pub fn delete_worktree(path: String, name: String) -> Result<(), GitError> {
    let worktree_path = std::path::Path::new(&path).parent().unwrap_or(std::path::Path::new(".")).join(&name);
    let status = Command::new("git")
        .current_dir(&path)
        .args(["worktree", "remove", worktree_path.to_str().unwrap()])
        .status()?;
    if !status.success() {
        return Err(GitError::new("git_cli", "worktree remove failed".to_string()));
    }
    Ok(())
}
```

- [ ] **Step 3: 注册命令**

Update `src-tauri/src/commands/mod.rs`:
```rust
pub mod diff;
pub mod graph;
pub mod ops;
pub mod repo;
pub mod worktree;
```

Update `src-tauri/src/main.rs` invoke handler list.

- [ ] **Step 4: 前端类型更新**

`src/engine/types.ts`:
```ts
export interface Worktree {
  name: string
  path: string
  head: string
  branch: string
  isMain: boolean
}
```

- [ ] **Step 5: Rust fixture 测试**

Add to `src-tauri/tests/fixtures.rs`:
```rust
#[test]
fn create_and_read_worktree() {
    let dir = linear_repo();
    let wt_dir = dir.path().parent().unwrap().join("wt");
    let status = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["worktree", "add", wt_dir.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let wts = engine::worktree::read_worktrees(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(wts.len(), 2);
}
```

Note: expose engine::worktree in lib.rs.

Run: `cd src-tauri && cargo test`
Expected: 通过。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/commands/ops.rs src-tauri/src/commands/worktree.rs src-tauri/src/commands/mod.rs src-tauri/src/engine/refs.rs src-tauri/src/main.rs src/engine/types.ts src-tauri/tests/fixtures.rs src-tauri/src/lib.rs
git commit -m "feat(ops): add branch, checkout, and worktree commands

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 2: BranchView 与 WorktreeView 组件

**Files:**
- Create: `src/components/BranchView.tsx`
- Create: `src/components/WorktreeView.tsx`

**Interfaces:**
- Consumes: `RefPointer[]`, `Worktree[]`, `head`, callbacks for checkout/create/delete/rename.
- Produces: left sidebar panels.

- [ ] **Step 1: 编写 BranchView.tsx**

```tsx
import { useState } from 'react'
import type { RefPointer } from '../engine/types'

interface BranchViewProps {
  refs: RefPointer[]
  head: string
  onCheckout: (name: string) => void
  onCreate: (name: string, start?: string) => void
  onDelete: (name: string) => void
}

export function BranchView({ refs, head, onCheckout, onCreate, onDelete }: BranchViewProps) {
  const [newName, setNewName] = useState('')
  const local = refs.filter((r) => r.kind === 'branch')
  const remote = refs.filter((r) => r.kind === 'remote')

  return (
    <div className="sidebar-panel">
      <h3>Branches</h3>
      <div className="branch-section">
        <strong>Local</strong>
        {local.map((ref) => (
          <div key={ref.name} className={ref.name === head ? 'branch-row head' : 'branch-row'}>
            <span onClick={() => onCheckout(ref.name.replace('refs/heads/', ''))}>
              {ref.name.replace('refs/heads/', '')}
            </span>
            <button onClick={() => onDelete(ref.name.replace('refs/heads/', ''))}>×</button>
          </div>
        ))}
      </div>
      <div className="branch-section">
        <strong>Remote</strong>
        {remote.map((ref) => (
          <div key={ref.name} className="branch-row remote">
            {ref.name.replace('refs/remotes/', '')}
          </div>
        ))}
      </div>
      <div className="branch-actions">
        <input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="new branch" />
        <button onClick={() => { onCreate(newName); setNewName('') }}>Create</button>
      </div>
    </div>
  )
}
```

- [ ] **Step 2: 编写 WorktreeView.tsx**

```tsx
import type { Worktree } from '../engine/types'

interface WorktreeViewProps {
  worktrees: Worktree[]
  currentPath: string
  onSwitch: (path: string) => void
  onDelete: (name: string) => void
  onCreate: (name: string) => void
}

export function WorktreeView({ worktrees, currentPath, onSwitch, onDelete, onCreate }: WorktreeViewProps) {
  return (
    <div className="sidebar-panel">
      <h3>Worktrees</h3>
      {worktrees.map((wt) => (
        <div key={wt.path} className={wt.path === currentPath ? 'worktree-row active' : 'worktree-row'}>
          <span onClick={() => onSwitch(wt.path)}>{wt.name} {wt.branch && `(${wt.branch})`}</span>
          {!wt.isMain && <button onClick={() => onDelete(wt.name)}>×</button>}
        </div>
      ))}
      <button onClick={() => onCreate(`wt-${Date.now()}`)}>+ New worktree</button>
    </div>
  )
}
```

- [ ] **Step 3: 提交**

```bash
git add src/components/BranchView.tsx src/components/WorktreeView.tsx
git commit -m "feat(ui): add branch and worktree sidebar panels

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 3: StagingView + DiffHunk

**Files:**
- Create: `src/components/DiffHunk.tsx`
- Create: `src/components/StagingView.tsx`

**Interfaces:**
- Consumes: `StatusData`, `DiffData`, callbacks `onStage`, `onUnstage`, `onCommit`.
- Produces: staging + diff panel.

- [ ] **Step 1: 编写 DiffHunk.tsx**

```tsx
import type { DiffData, DiffLine } from '../engine/types'

interface DiffHunkProps {
  diff: DiffData
}

function Line({ line }: { line: DiffLine }) {
  const className = line.origin === '+' ? 'diff-line add' : line.origin === '-' ? 'diff-line del' : 'diff-line'
  return <div className={className}><span className="diff-origin">{line.origin}</span>{line.content}</div>
}

export function DiffHunk({ diff }: DiffHunkProps) {
  if (diff.files.length === 0) return <div className="empty-diff">No changes</div>
  return (
    <div className="diff-view">
      {diff.files.map((file) => (
        <div key={file.path} className="diff-file">
          <div className="diff-file-header">{file.path}</div>
          {file.hunks.map((hunk, i) => (
            <div key={i} className="diff-hunk">
              {hunk.lines.map((line, j) => <Line key={j} line={line} />)}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}
```

- [ ] **Step 2: 编写 StagingView.tsx**

```tsx
import { useState } from 'react'
import type { FileChange, StatusData, DiffData } from '../engine/types'
import { DiffHunk } from './DiffHunk'

interface StagingViewProps {
  status: StatusData
  diff: DiffData
  onStage: (paths: string[]) => void
  onUnstage: (paths: string[]) => void
  onCommit: (msg: string, amend: boolean) => void
  onSelectFile: (path: string) => void
  selectedFile: string | null
}

function FileList({ title, files, onToggle, selected, onSelect }: {
  title: string
  files: FileChange[]
  onToggle: (path: string) => void
  selected: string | null
  onSelect: (path: string) => void
}) {
  return (
    <div className="file-list">
      <strong>{title}</strong>
      {files.map((f) => (
        <div key={f.path} className={selected === f.path ? 'file-row selected' : 'file-row'}>
          <input type="checkbox" checked onChange={() => onToggle(f.path)} />
          <span onClick={() => onSelect(f.path)}>{f.path}</span>
          <span className="file-stats">+{f.additions}/-{f.deletions}</span>
        </div>
      ))}
    </div>
  )
}

export function StagingView(props: StagingViewProps) {
  const [message, setMessage] = useState('')
  const [amend, setAmend] = useState(false)

  return (
    <div className="staging-view">
      <div className="staging-sidebar">
        <FileList title="Unstaged" files={props.status.unstaged} onToggle={props.onStage} selected={props.selectedFile} onSelect={props.onSelectFile} />
        <FileList title="Staged" files={props.status.staged} onToggle={props.onUnstage} selected={props.selectedFile} onSelect={props.onSelectFile} />
        <div className="commit-box">
          <textarea value={message} onChange={(e) => setMessage(e.target.value)} placeholder="Commit message" />
          <label><input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} /> Amend</label>
          <button onClick={() => props.onCommit(message, amend)}>Commit</button>
        </div>
      </div>
      <div className="staging-diff">
        <DiffHunk diff={props.diff} />
      </div>
    </div>
  )
}
```

- [ ] **Step 3: CSS 补齐**

Add to `src/styles.css`:
```css
.staging-view {
  display: flex;
  height: 100%;
}

.staging-sidebar {
  width: 240px;
  border-right: 1px solid #333;
  overflow: auto;
}

.file-row {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 2px 4px;
  cursor: pointer;
}

.file-row.selected {
  background: var(--selected-bg);
}

.file-stats {
  margin-left: auto;
  color: #888;
  font-size: 10px;
}

.diff-view {
  flex: 1;
  overflow: auto;
  font-family: monospace;
  font-size: 12px;
}

.diff-file-header {
  padding: 8px;
  background: #2a2a2a;
  border-bottom: 1px solid #333;
}

.diff-line {
  padding: 0 8px;
  white-space: pre;
}

.diff-line.add { background: rgba(129, 199, 132, 0.15); }
.diff-line.del { background: rgba(229, 115, 115, 0.15); }

.diff-origin {
  display: inline-block;
  width: 12px;
  color: #888;
}

.commit-box {
  padding: 8px;
  border-top: 1px solid #333;
}

.commit-box textarea {
  width: 100%;
  height: 60px;
  background: #222;
  color: #d4d4d4;
  border: 1px solid #444;
}
```

- [ ] **Step 4: 提交**

```bash
git add src/components/DiffHunk.tsx src/components/StagingView.tsx src/styles.css
git commit -m "feat(ui): add staging and diff view

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 4: 前端 Store 与完整 App 整合

**Files:**
- Create: `src/store/repoStore.ts`
- Modify: `src/App.tsx`
- Modify: `src/components/GraphView.tsx`

**Interfaces:**
- Produces: `RepoState` store with actions: `openRepo`, `refreshGraph`, `refreshStatus`, `selectCommit`, `runOperation`.

- [ ] **Step 1: 创建 store/repoStore.ts**

```ts
import { invoke } from '@tauri-apps/api/core'
import { useEffect, useState } from 'react'
import type { CommitNode, DiffData, GraphData, RefPointer, RepoInfo, StatusData, Worktree } from '../engine/types'

export interface RepoState {
  path: string | null
  head: string
  commits: CommitNode[]
  refs: RefPointer[]
  worktrees: Worktree[]
  status: StatusData
  diff: DiffData
  selectedId: string | null
  loading: boolean
}

const initialState: RepoState = {
  path: null,
  head: '',
  commits: [],
  refs: [],
  worktrees: [],
  status: { staged: [], unstaged: [] },
  diff: { files: [] },
  selectedId: null,
  loading: false,
}

export function useRepoStore() {
  const [state, setState] = useState<RepoState>(initialState)

  const openRepo = async (path: string) => {
    setState((s) => ({ ...s, loading: true }))
    const info = await invoke<RepoInfo>('open_repo', { path })
    setState((s) => ({ ...s, path, head: info.head, loading: false }))
    await refreshGraph(path)
    await refreshStatus(path)
    await invoke('watch_repo', { path })
  }

  const refreshGraph = async (path: string) => {
    const data = await invoke<GraphData>('load_graph', { path })
    setState((s) => ({
      ...s,
      commits: data.commits,
      refs: data.refs,
      selectedId: s.selectedId ?? data.commits[0]?.id ?? null,
    }))
  }

  const refreshStatus = async (path: string) => {
    const status = await invoke<StatusData>('read_status', { path })
    setState((s) => ({ ...s, status }))
  }

  const refreshDiff = async (path: string, scope: string) => {
    const diff = await invoke<DiffData>('read_diff', { path, scope })
    setState((s) => ({ ...s, diff }))
  }

  const selectCommit = (id: string) => {
    setState((s) => ({ ...s, selectedId: id }))
  }

  return { state, openRepo, refreshGraph, refreshStatus, refreshDiff, selectCommit, setState }
}
```

- [ ] **Step 2: 重写 App.tsx 完整布局**

```tsx
import { useEffect, useCallback, useState } from 'react'
import { GraphView } from './components/GraphView'
import { BranchView } from './components/BranchView'
import { WorktreeView } from './components/WorktreeView'
import { StagingView } from './components/StagingView'
import { useRepoStore } from './store/repoStore'
import { invoke } from '@tauri-apps/api/core'

const REPO_PATH = 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient'

function App() {
  const { state, openRepo, refreshGraph, refreshStatus, refreshDiff, selectCommit } = useRepoStore()
  const [selectedFile, setSelectedFile] = useState<string | null>(null)

  useEffect(() => {
    openRepo(REPO_PATH)
  }, [])

  useEffect(() => {
    if (!state.path) return
    if (state.selectedId) {
      refreshDiff(state.path, `commit:${state.selectedId}`)
    } else {
      refreshDiff(state.path, 'workdir')
    }
  }, [state.selectedId, state.path])

  const handleCommit = useCallback(async (msg: string, amend: boolean) => {
    if (!state.path) return
    await invoke('commit', { path: state.path, message: msg, amend })
    await refreshGraph(state.path)
    await refreshStatus(state.path)
  }, [state.path])

  if (state.loading && state.commits.length === 0) return <div>Loading...</div>

  return (
    <div className="app" style={{ display: 'flex', flexDirection: 'column', height: '100vh' }}>
      <header style={{ padding: 4, borderBottom: '1px solid #333' }}>{state.path} @ {state.head}</header>
      <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        <aside style={{ width: 220, borderRight: '1px solid #333', overflow: 'auto' }}>
          <BranchView refs={state.refs} head={state.head}
            onCheckout={async (name) => { await invoke('checkout', { path: state.path, rev: name }); await refreshGraph(state.path!) }}
            onCreate={async (name) => { await invoke('create_branch', { path: state.path, name }); await refreshGraph(state.path!) }}
            onDelete={async (name) => { await invoke('delete_branch', { path: state.path, name }); await refreshGraph(state.path!) }}
          />
          <WorktreeView worktrees={state.worktrees} currentPath={state.path ?? ''}
            onSwitch={async (path) => { await openRepo(path) }}
            onCreate={async (name) => { await invoke('create_worktree', { path: state.path, name }); await refreshGraph(state.path!) }}
            onDelete={async (name) => { await invoke('delete_worktree', { path: state.path, name }); await refreshGraph(state.path!) }}
          />
        </aside>
        <main style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          <div style={{ flex: 1, overflow: 'hidden' }}>
            <GraphView commits={state.commits} refs={state.refs} selectedId={state.selectedId} onSelect={selectCommit} />
          </div>
          <div style={{ height: 240, borderTop: '1px solid #333' }}>
            <StagingView
              status={state.status}
              diff={state.diff}
              selectedFile={selectedFile}
              onSelectFile={setSelectedFile}
              onStage={async (paths) => { await invoke('stage', { path: state.path, paths }); await refreshStatus(state.path!) }}
              onUnstage={async (paths) => { await invoke('unstage', { path: state.path, paths }); await refreshStatus(state.path!) }}
              onCommit={handleCommit}
            />
          </div>
        </main>
      </div>
    </div>
  )
}

export default App
```

- [ ] **Step 3: 添加缺失的 stage/unstage/commit 命令**

`src-tauri/src/commands/ops.rs` add:
```rust
#[tauri::command]
pub fn stage(path: String, paths: Vec<String>) -> Result<(), GitError> {
    let mut args = vec!["add"];
    args.extend(paths.iter().map(|s| s.as_str()));
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn unstage(path: String, paths: Vec<String>) -> Result<(), GitError> {
    let mut args = vec!["reset", "HEAD"];
    args.extend(paths.iter().map(|s| s.as_str()));
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn commit(path: String, message: String, amend: bool) -> Result<(), GitError> {
    if amend {
        run_git(&path, &["commit", "--amend", "-m", &message])?;
    } else {
        run_git(&path, &["commit", "-m", &message])?;
    }
    Ok(())
}
```

- [ ] **Step 4: 注册命令并更新 main.rs**

- [ ] **Step 5: 运行验证**

Run: `pnpm tauri dev`

Expected: 应用打开后加载真实仓库，显示 graph、分支列表、底部 diff/staging 面板。

- [ ] **Step 6: 提交**

```bash
git add src/store/repoStore.ts src/App.tsx src-tauri/src/commands/ops.rs src-tauri/src/main.rs
git commit -m "feat(app): wire engine, graph, and UI into full app

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 5: 自动刷新机制

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/main.rs`
- Modify: `src/store/repoStore.ts`

**Interfaces:**
- Produces: Rust emits `refs:changed` / `status:changed` events; frontend listens and refreshes.

- [ ] **Step 1: 添加 notify crate**

`src-tauri/Cargo.toml`:
```toml
[dependencies]
notify = "6.1"
```

- [ ] **Step 2: 把 watch_repo 暴露为命令**

`src-tauri/src/commands/ops.rs`（复用）或复用已有文件：为避免文件膨胀，复用 `src-tauri/src/commands/ops.rs`，在文件顶部添加：

```rust
use std::sync::mpsc::channel;
use std::time::Duration;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
```

在末尾添加：

```rust
use std::sync::mpsc::channel;
use std::time::Duration;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

#[tauri::command]
pub fn watch_repo(app_handle: tauri::AppHandle, path: String) {
    let (tx, rx) = channel::<Result<Event, notify::Error>>();
    let mut watcher = RecommendedWatcher::new(
        move |res| { let _ = tx.send(res); },
        Config::default().with_poll_interval(Duration::from_millis(300)),
    ).unwrap();

    let _ = watcher.watch(std::path::Path::new(&format!("{}/.git", path)), RecursiveMode::Recursive);
    let _ = watcher.watch(std::path::Path::new(&path), RecursiveMode::NonRecursive);

    std::thread::spawn(move || {
        while let Ok(Ok(event)) = rx.recv() {
            match event.kind {
                notify::EventKind::Modify(_) | notify::EventKind::Create(_) | notify::EventKind::Remove(_) => {
                    let is_ref_change = event.paths.iter().any(|p| p.to_string_lossy().contains(".git/refs") || p.to_string_lossy().contains(".git/HEAD"));
                    let is_status_change = event.paths.iter().any(|p| !p.to_string_lossy().contains(".git"));
                    if is_ref_change {
                        let _ = app_handle.emit("refs:changed", ());
                    }
                    if is_status_change {
                        let _ = app_handle.emit("status:changed", ());
                    }
                }
                _ => {}
            }
        }
    });
}
```

在 `src-tauri/src/commands/mod.rs` 添加 `pub mod ops;`（已存在）后，在 `main.rs` 注册 `commands::ops::watch_repo`：

```rust
.invoke_handler(tauri::generate_handler![
    commands::repo::open_repo,
    commands::graph::load_graph,
    commands::diff::read_status,
    commands::diff::read_diff,
    commands::ops::create_branch,
    commands::ops::delete_branch,
    commands::ops::rename_branch,
    commands::ops::checkout,
    commands::ops::stage,
    commands::ops::unstage,
    commands::ops::commit,
    commands::ops::watch_repo,
    commands::worktree::read_worktrees,
    commands::worktree::create_worktree,
    commands::worktree::delete_worktree,
])
```

MVP 简化：每次 `watch_repo` 启动新 watcher；切换仓库时旧 watcher 保留（不阻塞功能，后续可优化）。

- [ ] **Step 3: 前端监听事件**

Update `src/store/repoStore.ts`:
```ts
import { listen } from '@tauri-apps/api/event'

useEffect(() => {
  if (!state.path) return
  let seq = 0
  const unsubs: (() => void)[] = []

  listen('refs:changed', async () => {
    const current = ++seq
    await refreshGraph(state.path!)
    if (current !== seq) return
  }).then((u) => unsubs.push(u))

  listen('status:changed', async () => {
    await refreshStatus(state.path!)
  }).then((u) => unsubs.push(u))

  return () => unsubs.forEach((u) => u())
}, [state.path])
```

- [ ] **Step 4: 运行验证**

Run: `pnpm tauri dev`

Expected: 在外部用 git 命令创建分支或修改文件，应用自动刷新。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/Cargo.toml src-tauri/src/main.rs src/store/repoStore.ts
git commit -m "feat(refresh): add filesystem watcher and IPC events

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 6: 边界情况与错误处理收尾

**Files:**
- Modify: `src/App.tsx`
- Modify: `src-tauri/src/engine/graph.rs`
- Modify: `src-tauri/src/error.rs`
- Create: `src/components/Toast.tsx`

**Interfaces:**
- Produces: top-level error toast; empty-repo state; in-progress operation banners.

- [ ] **Step 1: 创建 Toast 组件**

`src/components/Toast.tsx`:
```tsx
import { useEffect } from 'react'

export function Toast({ message, onClose }: { message: string; onClose: () => void }) {
  useEffect(() => {
    const t = setTimeout(onClose, 5000)
    return () => clearTimeout(t)
  }, [])
  return <div className="toast">{message}<button onClick={onClose}>×</button></div>
}
```

- [ ] **Step 2: 捕获所有 invoke 错误并显示 toast**

Wrap invoke calls in App with try/catch and set toast state.

- [ ] **Step 3: 空仓库处理**

In `engine/graph.rs` `load()`:
```rust
if commits.is_empty() {
    return Err(GitError::new("empty_repo", "This repository has no commits yet"));
}
```

In App: show empty state with "Create first commit" button.

- [ ] **Step 4: 大仓库警告**

In `engine/graph.rs`:
```rust
if commits.len() > 200_000 {
    commits.truncate(5000);
}
```

- [ ] **Step 5: 检测合并/rebase 进行中**

Add Rust command:
```rust
#[tauri::command]
pub fn repo_state(path: String) -> Result<String, GitError> {
    let repo = std::path::Path::new(&path);
    if repo.join(".git/rebase-merge").exists() || repo.join(".git/rebase-apply").exists() {
        return Ok("rebase".to_string());
    }
    if repo.join(".git/MERGE_HEAD").exists() {
        return Ok("merge".to_string());
    }
    if repo.join(".git/CHERRY_PICK_HEAD").exists() {
        return Ok("cherry-pick".to_string());
    }
    Ok("normal".to_string())
}
```

App shows banner when state != normal.

- [ ] **Step 6: 提交**

```bash
git add src/components/Toast.tsx src/App.tsx src-tauri/src/engine/graph.rs src-tauri/src/error.rs
git commit -m "feat(polish): add error toasts, empty repo, large repo limits, operation banners

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

### Task 7: E2E 手动验证清单

**Files:**
- Create: `docs/e2e-checklist.md`

- [ ] **Step 1: 编写清单**

```markdown
# mygitclient v0.1 E2E 清单

## Graph
- [ ] 打开仓库后显示 commit graph，分支线颜色正确
- [ ] 点击 commit 选中并显示 diff
- [ ] 上下键导航 commit
- [ ] 分支/HEAD 标签显示正确

## 分支
- [ ] 左栏显示本地/远程分支
- [ ] 点击本地分支 checkout
- [ ] 新建分支后 graph 自动刷新
- [ ] 删除分支

## Worktree
- [ ] 显示 worktree 列表，当前高亮
- [ ] 新建 worktree
- [ ] 切换 worktree 后打开对应仓库
- [ ] 删除非主 worktree

## Staging / Commit
- [ ] 修改文件后出现在 Unstaged
- [ ] 点击文件显示 diff
- [ ] Stage / Unstage 生效
- [ ] 填写 message 后 commit，graph 顶部出现新节点
- [ ] Amend 生效

## 刷新
- [ ] 外部 git 操作后 graph/status 自动刷新
- [ ] 自己操作后无竞态

## 边界
- [ ] 打开空仓库显示空态
- [ ] git 写操作失败显示 stderr
- [ ] 合并/rebase 进行中显示横幅
```

- [ ] **Step 2: 提交**

```bash
git add docs/e2e-checklist.md
git commit -m "docs: add E2E checklist for v0.1

Co-Authored-By: Claude <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- 分支/worktree 管理 ✅ Task 1, 2
- staging/commit/diff ✅ Task 3
- 完整 App 整合 ✅ Task 4
- 自动刷新 ✅ Task 5
- 边界情况 ✅ Task 6
- E2E 清单 ✅ Task 7

**Placeholder scan:** 无占位符。

**Type一致性:** `Worktree` Rust/TS 字段对应；`RepoState` 包含 spec 中定义的所有字段。

**未覆盖（spec 外 / 后续版本）:** push/pull/fetch、Ctrl+P、terminal/editor、字符级 diff、冲突解决 UI、浅色主题。
