# Git 操作工具栏 Implementation Plan（阶段二）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 顶部加一条 Fork 风格的 Git 操作工具栏（Fetch / Pull / Push / Stash，按住 Ctrl 切换到变体动作），配 ahead/behind 徽章和一个 Stash 侧边栏面板。

**Architecture:** 读操作走 libgit2（ahead/behind、stash 列表），写操作走 git CLI（fetch/pull/push/stash）——沿用项目既有分工。所有 git 子进程集中到一个加固过的 `git_cli` 模块，统一设 `GIT_TERMINAL_PROMPT=0` 防止认证提示把进程挂死。

**Tech Stack:** Rust + git2-rs 0.19、Tauri 2、TypeScript 5.6、React 18、Vitest 2.1（无 jsdom / testing-library）

规格：`docs/superpowers/specs/2026-08-06-toolbar-and-graph-lanes-design.md`

## Global Constraints

- 工作目录：`C:\Users\liuhao\workspace\dev\Dev\mygitclient\.worktrees\git-toolbar`，分支 `feat/git-toolbar-and-graph-polish`。
- **本计划独立于阶段一**（`2026-08-06-graph-lane-rework.md`），两者互不依赖，可以任意顺序执行。
- **所有 git 子进程必须设 `GIT_TERMINAL_PROMPT=0`。** 凭据管理器（Windows 上的 GCM）弹自己的 GUI 窗口是正常且要保留的；要防的是 git 往 stderr 打印 `Username:` 然后无限等 stdin——GUI 应用里没人能回答它，子进程会永久挂死并占住 Tauri 的线程池。
- 读走 libgit2、写走 git CLI，这是项目既有分工，不要混。
- 代码风格：单引号、尾随逗号、严格 TypeScript 类型、中文 JSDoc 注释。Rust 结构体跨 IPC 时加 `#[serde(rename_all = "camelCase")]`。
- 测试命令：`pnpm test`、`pnpm exec tsc --noEmit`、`cd src-tauri && cargo test`。
- **本仓库没有 jsdom / @testing-library/react。** 只给纯函数和 Rust 写单测；React 组件行为改用 CDP 运行时验证（Task 7）。不要为了测组件去装新依赖。
- 所有网络操作的测试用**本地 bare 仓库当 remote**（`file://` 路径），不碰真实网络。

---

### Task 1: git CLI 加固模块

**Files:**
- Create: `src-tauri/src/engine/git_cli.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Modify: `src-tauri/src/commands/ops.rs:1-14`（删掉本地 `run_git`，改用共享的）
- Test: `src-tauri/tests/git_cli.rs`

**Interfaces:**
- Produces:
  - `pub fn git_command(path: &str) -> std::process::Command` —— 已设好 cwd 与环境变量的裸命令
  - `pub fn run_git(path: &str, args: &[&str]) -> Result<String>` —— 执行并把非零退出转成 `GitError::new("git_cli", stderr)`

- [ ] **Step 1: 写失败的测试**

创建 `src-tauri/tests/git_cli.rs`：

```rust
use app_lib::engine::git_cli;

#[test]
fn git_command_disables_terminal_prompt() {
    // GUI 里没人能回答 git 的 "Username:" 提示，子进程会永久挂死。
    // 这个变量是唯一的防线，改动 git_cli 时不能把它弄丢。
    let cmd = git_cli::git_command(".");
    let envs: Vec<_> = cmd.get_envs().collect();
    let prompt = envs
        .iter()
        .find(|(k, _)| *k == std::ffi::OsStr::new("GIT_TERMINAL_PROMPT"))
        .expect("GIT_TERMINAL_PROMPT not set");
    assert_eq!(prompt.1, Some(std::ffi::OsStr::new("0")));
}

#[test]
fn run_git_surfaces_stderr_on_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let err = git_cli::run_git(dir.path().to_str().unwrap(), &["rev-parse", "HEAD"])
        .expect_err("expected failure outside a repo");
    assert_eq!(err.code, "git_cli");
    assert!(!err.message.is_empty());
}

#[test]
fn run_git_returns_stdout_on_success() {
    let out = git_cli::run_git(".", &["--version"]).unwrap();
    assert!(out.starts_with("git version"), "got {out}");
}
```

如果 `GitError` 的字段不是公开的 `code` / `message`，先看 `src-tauri/src/error.rs` 用实际字段名调整断言，**不要**为了测试去改 `error.rs` 的可见性以外的东西。

- [ ] **Step 2: 运行测试确认失败**

```
cd src-tauri && cargo test --test git_cli
```

Expected: 编译失败 —— `unresolved import app_lib::engine::git_cli`

- [ ] **Step 3: 创建 `src-tauri/src/engine/git_cli.rs`**

```rust
use crate::error::{GitError, Result};
use std::process::Command;

/// 构造一个指向 `path` 的 git 命令，并关掉所有交互式提示。
///
/// `GIT_TERMINAL_PROMPT=0` 是必须的：GUI 应用没有终端，git 一旦往 stderr
/// 打印 "Username:" 就会无限等待 stdin，子进程永久挂死并占住 Tauri 的线程池。
/// 关掉之后 git 会立即失败，错误能走到界面上的错误横幅。
///
/// 凭据管理器（Windows 的 GCM）弹自己的 GUI 窗口不受影响——那条路径不经过终端。
pub fn git_command(path: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(path);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// 执行一条 git 命令，成功返回 stdout，失败把 stderr 包成 GitError。
pub fn run_git(path: &str, args: &[&str]) -> Result<String> {
    let output = git_command(path).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

在 `src-tauri/src/engine/mod.rs` 加一行：

```rust
pub mod git_cli;
```

- [ ] **Step 4: `src-tauri/src/commands/ops.rs` 改用共享实现**

删除 `src-tauri/src/commands/ops.rs:1-14`（`use std::process::Command;` 与本地 `run_git` 函数），换成：

```rust
use crate::engine::git_cli::run_git;
use crate::error::Result;
```

文件其余部分不动——所有 `run_git(&path, &[...])` 调用点签名一致。注意原来的 `use crate::error::{GitError, Result};` 里 `GitError` 变成未使用，要去掉，否则 `cargo build` 会报 warning。

- [ ] **Step 5: 运行测试确认通过**

```
cd src-tauri && cargo test
```

Expected: 全部通过，含 3 个新用例。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/git_cli.rs src-tauri/src/engine/mod.rs src-tauri/src/commands/ops.rs src-tauri/tests/git_cli.rs
git commit -m "feat(git): centralize git CLI with terminal prompt disabled"
```

---

### Task 2: ahead / behind

**Files:**
- Create: `src-tauri/src/engine/upstream.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Modify: `src-tauri/src/commands/repo.rs`
- Modify: `src-tauri/src/lib.rs:19-37`
- Test: `src-tauri/tests/upstream.rs`

**Interfaces:**
- Produces:
  - Rust `pub struct UpstreamInfo { branch: String, upstream: Option<String>, ahead: usize, behind: usize }`（`#[serde(rename_all = "camelCase")]`）
  - `pub fn read(path: &str) -> Result<UpstreamInfo>`
  - Tauri 命令 `read_upstream(path: String)`

- [ ] **Step 1: 写失败的测试**

创建 `src-tauri/tests/upstream.rs`：

```rust
use app_lib::engine::upstream;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

/// 一个带本地 bare「远程」的仓库，全程不碰网络。
fn repo_with_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);

    let local = TempDir::new().unwrap();
    run_git(local.path(), &["init"]);
    run_git(local.path(), &["config", "user.email", "test@test.com"]);
    run_git(local.path(), &["config", "user.name", "Test"]);
    commit(local.path(), "c1");
    let url = remote.path().to_str().unwrap();
    run_git(local.path(), &["remote", "add", "origin", url]);
    run_git(local.path(), &["push", "-u", "origin", "HEAD"]);
    (local, remote)
}

#[test]
fn reports_zero_when_in_sync() {
    let (local, _remote) = repo_with_remote();
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert!(info.upstream.is_some());
    assert_eq!(info.ahead, 0);
    assert_eq!(info.behind, 0);
}

#[test]
fn counts_local_commits_as_ahead() {
    let (local, _remote) = repo_with_remote();
    commit(local.path(), "c2");
    commit(local.path(), "c3");
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert_eq!(info.ahead, 2);
    assert_eq!(info.behind, 0);
}

#[test]
fn reports_no_upstream_for_fresh_branch() {
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["checkout", "-b", "solo"]);
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert_eq!(info.branch, "solo");
    assert!(info.upstream.is_none());
    // 没有 upstream 时计数必须是 0，不能是脏值——界面靠它决定不显示徽章
    assert_eq!(info.ahead, 0);
    assert_eq!(info.behind, 0);
}

#[test]
fn reports_branch_name_without_refs_prefix() {
    let (local, _remote) = repo_with_remote();
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert!(!info.branch.starts_with("refs/"), "got {}", info.branch);
}
```

- [ ] **Step 2: 运行测试确认失败**

```
cd src-tauri && cargo test --test upstream
```

Expected: 编译失败 —— `unresolved import app_lib::engine::upstream`

- [ ] **Step 3: 创建 `src-tauri/src/engine/upstream.rs`**

```rust
use crate::error::Result;
use git2::{BranchType, Repository};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamInfo {
    /// 当前分支名（不带 refs/heads/ 前缀）；游离 HEAD 时是 "HEAD"。
    pub branch: String,
    /// 上游分支的简称，如 "origin/master"。没有设上游时为 None。
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

/// 读当前分支相对其上游的领先/落后数。
///
/// 走 libgit2 而不是 `git rev-list --count`：这是读操作，开子进程不划算，
/// 而且它会跟着每次 repo-changed 事件刷新。
pub fn read(path: &str) -> Result<UpstreamInfo> {
    let repo = Repository::open(path)?;
    let head = repo.head()?;
    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();

    let none = |branch: String| UpstreamInfo { branch, upstream: None, ahead: 0, behind: 0 };

    // 游离 HEAD 没有分支，也就没有上游
    if !head.is_branch() {
        return Ok(none(branch_name));
    }

    let branch = match repo.find_branch(&branch_name, BranchType::Local) {
        Ok(b) => b,
        Err(_) => return Ok(none(branch_name)),
    };
    let upstream = match branch.upstream() {
        Ok(u) => u,
        // 新建分支还没 push 过就没有上游，这是正常状态而不是错误
        Err(_) => return Ok(none(branch_name)),
    };

    let local_oid = head.target();
    let upstream_oid = upstream.get().target();
    let upstream_name = upstream.name()?.unwrap_or("").to_string();

    let (ahead, behind) = match (local_oid, upstream_oid) {
        (Some(l), Some(u)) => repo.graph_ahead_behind(l, u)?,
        _ => (0, 0),
    };

    Ok(UpstreamInfo {
        branch: branch_name,
        upstream: Some(upstream_name),
        ahead,
        behind,
    })
}
```

在 `src-tauri/src/engine/mod.rs` 加：

```rust
pub mod upstream;
```

- [ ] **Step 4: 加 Tauri 命令**

在 `src-tauri/src/commands/repo.rs` 末尾追加：

```rust
#[tauri::command]
pub fn read_upstream(path: String) -> Result<crate::engine::upstream::UpstreamInfo, crate::error::GitError> {
    engine::upstream::read(&path)
}
```

在 `src-tauri/src/lib.rs` 的 `generate_handler!` 列表里按字母序插入：

```rust
      commands::repo::read_upstream,
```

（放在 `commands::repo::open_repo,` 之后、`commands::repo::repo_state,` 之前。）

- [ ] **Step 5: 运行测试确认通过**

```
cd src-tauri && cargo test
```

Expected: 全部通过，含 4 个新用例。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/upstream.rs src-tauri/src/engine/mod.rs src-tauri/src/commands/repo.rs src-tauri/src/lib.rs src-tauri/tests/upstream.rs
git commit -m "feat(remote): read ahead/behind against upstream"
```

---

### Task 3: fetch / pull / push 命令

**Files:**
- Create: `src-tauri/src/commands/remote.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs:19-37`
- Test: `src-tauri/tests/remote.rs`

**Interfaces:**
- Consumes: Task 1 的 `git_cli::run_git`
- Produces: Tauri 命令
  - `fetch(path: String, all: bool)`
  - `pull(path: String, rebase: bool)`
  - `push(path: String, force: bool)`

  三者都返回 `Result<(), GitError>`。`all` / `rebase` / `force` 对应界面上的 Ctrl 变体。

- [ ] **Step 1: 写失败的测试**

创建 `src-tauri/tests/remote.rs`：

```rust
use app_lib::commands::remote;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

fn init(dir: &std::path::Path) {
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
}

/// bare 远程 + 两个克隆，全程走本地路径，不碰网络。
fn trio() -> (TempDir, TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let a = TempDir::new().unwrap();
    init(a.path());
    commit(a.path(), "c1");
    run_git(a.path(), &["remote", "add", "origin", &url]);
    run_git(a.path(), &["push", "-u", "origin", "HEAD"]);

    let b = TempDir::new().unwrap();
    run_git(b.path(), &["clone", &url, b.path().to_str().unwrap()]);
    run_git(b.path(), &["config", "user.email", "test@test.com"]);
    run_git(b.path(), &["config", "user.name", "Test"]);

    (remote, a, b)
}

#[test]
fn fetch_updates_remote_tracking_ref() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);

    let before = git_out(b.path(), &["rev-parse", "origin/HEAD"]);
    remote::fetch(b.path().to_str().unwrap().to_string(), false).unwrap();
    let after = git_out(b.path(), &["rev-parse", "FETCH_HEAD"]);
    assert_ne!(before, after);
}

#[test]
fn pull_brings_commits_into_working_branch() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);

    remote::pull(b.path().to_str().unwrap().to_string(), false).unwrap();
    assert!(b.path().join("c2.txt").exists());
}

#[test]
fn pull_with_rebase_keeps_history_linear() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    commit(b.path(), "local-side");

    remote::pull(b.path().to_str().unwrap().to_string(), true).unwrap();
    // rebase 不产生 merge 提交：所有提交都只有一个父
    let parents = git_out(b.path(), &["log", "--pretty=%p"]);
    for line in parents.lines() {
        assert!(line.split_whitespace().count() <= 1, "found a merge commit: {line}");
    }
}

#[test]
fn push_sets_upstream_for_a_fresh_branch() {
    // 新分支没有上游，裸 `git push` 会失败；实现必须自动补 -u origin <branch>
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["checkout", "-b", "feature"]);
    commit(b.path(), "f1");

    remote::push(b.path().to_str().unwrap().to_string(), false).unwrap();
    let upstream = git_out(b.path(), &["rev-parse", "--abbrev-ref", "feature@{upstream}"]);
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn push_propagates_rejection_as_error() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    commit(b.path(), "local-side");

    // b 落后于远程，非快进推送必须被拒绝并转成 GitError 而不是静默成功
    let err = remote::push(b.path().to_str().unwrap().to_string(), false)
        .expect_err("non-fast-forward push should fail");
    assert_eq!(err.code, "git_cli");
}

#[test]
fn force_push_overwrites_remote() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    run_git(b.path(), &["fetch"]);
    commit(b.path(), "local-side");
    run_git(b.path(), &["reset", "--hard", "HEAD"]);

    remote::push(b.path().to_str().unwrap().to_string(), true).unwrap();
    let local = git_out(b.path(), &["rev-parse", "HEAD"]);
    let pushed = git_out(a.path(), &["ls-remote", "origin", "HEAD"]);
    assert!(pushed.starts_with(&local[..8]), "remote HEAD {pushed} != local {local}");
}
```

`force_push_overwrites_remote` 用 `--force-with-lease`，它要求本地对远程状态的认知是最新的，所以测试里先 `fetch` 再造分叉。若该用例因 lease 检查而失败，说明实现漏了 `fetch` 前提——**不要**把它改成 `--force`，那会绕过整个安全机制。

- [ ] **Step 2: 运行测试确认失败**

```
cd src-tauri && cargo test --test remote
```

Expected: 编译失败 —— `unresolved import app_lib::commands::remote`

- [ ] **Step 3: 创建 `src-tauri/src/commands/remote.rs`**

```rust
use crate::engine::git_cli::run_git;
use crate::error::Result;

/// 当前分支名。push 需要它来补 -u。
fn current_branch(path: &str) -> Result<String> {
    Ok(run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])?.trim().to_string())
}

/// 该分支是否已经设过上游。
fn has_upstream(path: &str) -> bool {
    run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_ok()
}

#[tauri::command]
pub fn fetch(path: String, all: bool) -> Result<()> {
    // all 对应界面上按住 Ctrl 的变体
    if all {
        run_git(&path, &["fetch", "--all", "--prune", "--tags"])?;
    } else {
        run_git(&path, &["fetch", "--prune"])?;
    }
    Ok(())
}

#[tauri::command]
pub fn pull(path: String, rebase: bool) -> Result<()> {
    // 默认不带参数，尊重用户自己的 pull.rebase 配置——
    // 这是 git 客户端最不意外的行为
    if rebase {
        run_git(&path, &["pull", "--rebase"])?;
    } else {
        run_git(&path, &["pull"])?;
    }
    Ok(())
}

#[tauri::command]
pub fn push(path: String, force: bool) -> Result<()> {
    let branch = current_branch(&path)?;
    // 新分支没有上游，裸 push 会直接失败，自动补 -u 省掉一次无谓的报错
    let set_upstream = !has_upstream(&path);

    let mut args: Vec<&str> = vec!["push"];
    // --force-with-lease 而不是 --force：远程有别人的新提交时会拒绝，
    // 不会把同事的工作覆盖掉
    if force {
        args.push("--force-with-lease");
    }
    if set_upstream {
        args.push("-u");
        args.push("origin");
        args.push(&branch);
    }
    run_git(&path, &args)?;
    Ok(())
}
```

在 `src-tauri/src/commands/mod.rs` 加：

```rust
pub mod remote;
```

在 `src-tauri/src/lib.rs` 的 `generate_handler!` 列表里插入（放在 `commands::repo::open_repo,` 之前）：

```rust
      commands::remote::fetch,
      commands::remote::pull,
      commands::remote::push,
```

- [ ] **Step 4: 运行测试确认通过**

```
cd src-tauri && cargo test --test remote
```

Expected: 6 个用例全部通过。

- [ ] **Step 5: 跑全量后端测试**

```
cd src-tauri && cargo test
```

Expected: 全绿。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/commands/remote.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tests/remote.rs
git commit -m "feat(remote): add fetch, pull, and push commands"
```

---

### Task 4: Stash 后端

**Files:**
- Create: `src-tauri/src/engine/stash.rs`
- Create: `src-tauri/src/commands/stash.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs:19-37`
- Test: `src-tauri/tests/stash.rs`

**Interfaces:**
- Consumes: Task 1 的 `git_cli::run_git`
- Produces:
  - Rust `pub struct StashEntry { index: usize, message: String, hash: String }`（`#[serde(rename_all = "camelCase")]`）
  - `pub fn list(path: &str) -> Result<Vec<StashEntry>>`
  - Tauri 命令 `read_stashes(path)`、`stash_push(path, message, include_untracked)`、`stash_apply(path, index)`、`stash_pop(path, index)`、`stash_drop(path, index)`

- [ ] **Step 1: 写失败的测试**

创建 `src-tauri/tests/stash.rs`：

```rust
use app_lib::commands::stash as cmd;
use app_lib::engine::stash;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    dir
}

#[test]
fn list_is_empty_on_a_clean_repo() {
    let dir = repo();
    assert!(stash::list(dir.path().to_str().unwrap()).unwrap().is_empty());
}

#[test]
fn push_then_list_reports_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();

    cmd::stash_push(p.clone(), Some("my work".into()), false).unwrap();
    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 0);
    assert!(entries[0].message.contains("my work"), "got {}", entries[0].message);
    // hash 要能直接喂给现成的 commit:<hash> diff scope，所以必须是完整 oid
    assert_eq!(entries[0].hash.len(), 40);
    // 工作区已经被还原
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
}

#[test]
fn push_without_message_still_produces_an_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();

    cmd::stash_push(p.clone(), None, false).unwrap();
    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].message.is_empty());
}

#[test]
fn include_untracked_captures_new_files() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("new.txt"), "fresh").unwrap();

    cmd::stash_push(p.clone(), None, true).unwrap();
    assert!(!dir.path().join("new.txt").exists(), "untracked file was not stashed");
    assert_eq!(stash::list(&p).unwrap().len(), 1);
}

#[test]
fn newest_stash_is_index_zero() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "first").unwrap();
    cmd::stash_push(p.clone(), Some("first".into()), false).unwrap();
    fs::write(dir.path().join("a.txt"), "second").unwrap();
    cmd::stash_push(p.clone(), Some("second".into()), false).unwrap();

    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].message.contains("second"), "got {}", entries[0].message);
    assert_eq!(entries[0].index, 0);
    assert_eq!(entries[1].index, 1);
}

#[test]
fn pop_restores_changes_and_removes_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_pop(p.clone(), 0).unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "changed");
    assert!(stash::list(&p).unwrap().is_empty());
}

#[test]
fn apply_restores_changes_but_keeps_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_apply(p.clone(), 0).unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "changed");
    assert_eq!(stash::list(&p).unwrap().len(), 1);
}

#[test]
fn drop_removes_the_entry_without_touching_the_working_tree() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_drop(p.clone(), 0).unwrap();
    assert!(stash::list(&p).unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
}
```

- [ ] **Step 2: 运行测试确认失败**

```
cd src-tauri && cargo test --test stash
```

Expected: 编译失败 —— `unresolved import app_lib::engine::stash`

- [ ] **Step 3: 创建 `src-tauri/src/engine/stash.rs`**

```rust
use crate::error::Result;
use git2::Repository;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    /// stash@{N} 里的 N。0 是最新的一条。
    pub index: usize,
    pub message: String,
    /// stash 本身就是一个提交，前端拿它复用现成的 `commit:<hash>` diff scope。
    pub hash: String,
}

/// 列出所有 stash，最新的在前。
pub fn list(path: &str) -> Result<Vec<StashEntry>> {
    // stash_foreach 需要可变借用
    let mut repo = Repository::open(path)?;
    let mut entries = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        entries.push(StashEntry {
            index,
            message: message.to_string(),
            hash: oid.to_string(),
        });
        true
    })?;
    Ok(entries)
}
```

- [ ] **Step 4: 创建 `src-tauri/src/commands/stash.rs`**

```rust
use crate::engine::git_cli::run_git;
use crate::engine::stash::{self, StashEntry};
use crate::error::Result;

#[tauri::command]
pub fn read_stashes(path: String) -> Result<Vec<StashEntry>> {
    stash::list(&path)
}

#[tauri::command]
pub fn stash_push(path: String, message: Option<String>, include_untracked: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["stash", "push"];
    // include_untracked 对应界面上按住 Ctrl 的变体
    if include_untracked {
        args.push("-u");
    }
    if let Some(msg) = &message {
        args.push("-m");
        args.push(msg);
    }
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn stash_apply(path: String, index: usize) -> Result<()> {
    run_git(&path, &["stash", "apply", &format!("stash@{{{index}}}")])?;
    Ok(())
}

#[tauri::command]
pub fn stash_pop(path: String, index: usize) -> Result<()> {
    run_git(&path, &["stash", "pop", &format!("stash@{{{index}}}")])?;
    Ok(())
}

#[tauri::command]
pub fn stash_drop(path: String, index: usize) -> Result<()> {
    run_git(&path, &["stash", "drop", &format!("stash@{{{index}}}")])?;
    Ok(())
}
```

在 `src-tauri/src/engine/mod.rs` 加 `pub mod stash;`，在 `src-tauri/src/commands/mod.rs` 加 `pub mod stash;`。

在 `src-tauri/src/lib.rs` 的 `generate_handler!` 列表里插入（放在 `commands::repo::repo_state,` 之后）：

```rust
      commands::stash::read_stashes,
      commands::stash::stash_apply,
      commands::stash::stash_drop,
      commands::stash::stash_pop,
      commands::stash::stash_push,
```

- [ ] **Step 5: 运行测试确认通过**

```
cd src-tauri && cargo test
```

Expected: 全绿，含 8 个新用例。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/stash.rs src-tauri/src/commands/stash.rs src-tauri/src/engine/mod.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/tests/stash.rs
git commit -m "feat(stash): list, push, apply, pop, and drop"
```

---

### Task 5: 前端类型与 store 接入

**Files:**
- Modify: `src/engine/types.ts`（追加两个接口）
- Modify: `src/store/repoStore.ts:8-19,21-32,37-49,94-152`
- Create: `src/engine/toolbarActions.ts`
- Test: `tests/vitest/toolbarActions.test.ts`

**Interfaces:**
- Consumes: Task 2 的 `read_upstream`、Task 3 的 `fetch`/`pull`/`push`、Task 4 的 `read_stashes`
- Produces:
  - TS `UpstreamInfo { branch: string; upstream: string | null; ahead: number; behind: number }`
  - TS `StashEntry { index: number; message: string; hash: string }`
  - `RepoState` 新增 `upstream: UpstreamInfo | null`、`stashes: StashEntry[]`
  - store 新增 `refreshUpstream(path)`、`refreshStashes(path)`
  - `RepoRefreshers` 新增这两个字段
  - `export type ToolbarAction = 'fetch' | 'pull' | 'push' | 'stash'`
  - `export interface ToolbarPlan { label: string; command: string; args: Record<string, unknown>; confirm: string | null }`
  - `export function planToolbarAction(action: ToolbarAction, ctrl: boolean, branch: string): ToolbarPlan`

**为什么要 `toolbarActions.ts`：** 本仓库没有 jsdom，React 组件测不了。把「按钮 + Ctrl → 调哪个命令、传什么参数、要不要确认」这段纯逻辑抽成独立模块，就能用 vitest 完整覆盖，组件只剩渲染和事件绑定。

- [ ] **Step 1: 写失败的测试**

创建 `tests/vitest/toolbarActions.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import { planToolbarAction } from '../../src/engine/toolbarActions'

describe('planToolbarAction', () => {
  it('默认 fetch 只 prune，不碰其他远程', () => {
    const plan = planToolbarAction('fetch', false, 'master')
    expect(plan.command).toBe('fetch')
    expect(plan.args).toEqual({ all: false })
    expect(plan.confirm).toBeNull()
  })

  it('Ctrl+fetch 抓所有远程', () => {
    expect(planToolbarAction('fetch', true, 'master').args).toEqual({ all: true })
  })

  it('默认 pull 不带 rebase，尊重用户的 pull.rebase 配置', () => {
    expect(planToolbarAction('pull', false, 'master').args).toEqual({ rebase: false })
  })

  it('Ctrl+pull 走 rebase', () => {
    expect(planToolbarAction('pull', true, 'master').args).toEqual({ rebase: true })
  })

  it('默认 push 不强推，也不需要确认', () => {
    const plan = planToolbarAction('push', false, 'master')
    expect(plan.args).toEqual({ force: false })
    expect(plan.confirm).toBeNull()
  })

  it('Ctrl+push 强推，并且必须要求确认', () => {
    // 强推可能覆盖远程历史，这是本工具栏唯一不可逆的动作
    const plan = planToolbarAction('push', true, 'master')
    expect(plan.args).toEqual({ force: true })
    expect(plan.confirm).toBeTruthy()
  })

  it('强推的确认文案里带上分支名，避免推错分支', () => {
    expect(planToolbarAction('push', true, 'release/2.0').confirm).toContain('release/2.0')
  })

  it('Ctrl+stash 连未跟踪文件一起存', () => {
    expect(planToolbarAction('stash', false, 'master').args).toEqual({ message: null, includeUntracked: false })
    expect(planToolbarAction('stash', true, 'master').args).toEqual({ message: null, includeUntracked: true })
  })

  it('按住 Ctrl 时按钮文字要跟着变，否则这功能没人发现得了', () => {
    for (const action of ['fetch', 'pull', 'push', 'stash'] as const) {
      const plain = planToolbarAction(action, false, 'master').label
      const ctrl = planToolbarAction(action, true, 'master').label
      expect(ctrl).not.toBe(plain)
    }
  })

  it('命令名与后端注册的 Tauri 命令一致', () => {
    expect(planToolbarAction('stash', false, 'master').command).toBe('stash_push')
  })
})
```

- [ ] **Step 2: 运行测试确认失败**

```
pnpm exec vitest run tests/vitest/toolbarActions.test.ts
```

Expected: FAIL — `Failed to resolve import "../../src/engine/toolbarActions"`

- [ ] **Step 3: 创建 `src/engine/toolbarActions.ts`**

```ts
export type ToolbarAction = 'fetch' | 'pull' | 'push' | 'stash'

export interface ToolbarPlan {
  /** 按钮上显示的文字。按住 Ctrl 时必须变化，否则变体功能无从发现。 */
  label: string
  /** 要 invoke 的 Tauri 命令名。 */
  command: string
  /** 除 path 之外的参数。 */
  args: Record<string, unknown>
  /** 非空时先弹确认框，文案即此字符串。 */
  confirm: string | null
}

/**
 * 把「哪个按钮 + 有没有按 Ctrl」翻译成一次具体的后端调用。
 *
 * 抽成纯函数是为了可测：本仓库没有 jsdom，组件跑不起来，
 * 但这段分支逻辑恰恰是最容易写错的部分。
 */
export function planToolbarAction(
  action: ToolbarAction,
  ctrl: boolean,
  branch: string,
): ToolbarPlan {
  switch (action) {
    case 'fetch':
      return {
        label: ctrl ? 'Fetch all' : 'Fetch',
        command: 'fetch',
        args: { all: ctrl },
        confirm: null,
      }
    case 'pull':
      return {
        label: ctrl ? 'Pull (rebase)' : 'Pull',
        command: 'pull',
        args: { rebase: ctrl },
        confirm: null,
      }
    case 'push':
      return {
        label: ctrl ? 'Force push' : 'Push',
        command: 'push',
        args: { force: ctrl },
        // 工具栏上唯一可能覆盖远程历史的动作，值得拦一道
        confirm: ctrl
          ? `Force-push ${branch}? This can overwrite commits on the remote.`
          : null,
      }
    case 'stash':
      return {
        label: ctrl ? 'Stash all' : 'Stash',
        command: 'stash_push',
        args: { message: null, includeUntracked: ctrl },
        confirm: null,
      }
  }
}
```

- [ ] **Step 4: 运行测试确认通过**

```
pnpm exec vitest run tests/vitest/toolbarActions.test.ts
```

Expected: PASS，10 个用例。

- [ ] **Step 5: 前端类型**

在 `src/engine/types.ts` 末尾追加：

```ts
export interface UpstreamInfo {
  /** 当前分支名，不带 refs/heads/ 前缀。 */
  branch: string
  /** 上游简称如 "origin/master"；没设上游时为 null。 */
  upstream: string | null
  ahead: number
  behind: number
}

export interface StashEntry {
  /** stash@{N} 里的 N，0 最新。 */
  index: number
  message: string
  /** stash 本身是个提交，可直接用于 `commit:<hash>` diff scope。 */
  hash: string
}
```

- [ ] **Step 6: store 接入**

`src/store/repoStore.ts` 四处改动。

其一，`RepoState` 接口加两个字段（放在 `worktrees` 之后）：

```ts
  upstream: UpstreamInfo | null
  stashes: StashEntry[]
```

其二，`initialState` 相应加：

```ts
  upstream: null,
  stashes: [],
```

其三，在 `refreshWorktrees` 之后加两个刷新函数：

```ts
  const refreshUpstream = async (path: string) => {
    const upstream = await invoke<UpstreamInfo>('read_upstream', { path })
    setState((s) => ({ ...s, upstream }))
  }

  const refreshStashes = async (path: string) => {
    const stashes = await invoke<StashEntry[]>('read_stashes', { path })
    setState((s) => ({ ...s, stashes }))
  }
```

并加进 `useRepoStore` 的返回对象。

其四，`openRepo` 里 `src/store/repoStore.ts:44-45` 之后补上首次读取：

```ts
      const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
      setState((s) => ({ ...s, worktrees: wts }))
      await refreshUpstream(path)
      await refreshStashes(path)
```

其五，`RepoRefreshers` 接口（`src/store/repoStore.ts:109-115`）加两个字段：

```ts
export interface RepoRefreshers {
  refreshHead: (path: string) => Promise<void>
  refreshGraph: (path: string) => Promise<void>
  refreshStatus: (path: string) => Promise<void>
  refreshWorktrees: (path: string) => Promise<void>
  refreshDiff: (path: string, scope: string) => Promise<void>
  refreshUpstream: (path: string) => Promise<void>
  refreshStashes: (path: string) => Promise<void>
}
```

其六，`useRepoEffects` 的解构（`src/store/repoStore.ts:122`）与 `repo-changed` 处理跟上：

```ts
  const { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, refreshUpstream, refreshStashes } = refreshers
```

在 `src/store/repoStore.ts:147`（`void refreshWorktrees(path).catch(ignore)`）之后加两行：

```ts
      // 外部 push/fetch 会改变领先落后数，命令行 stash 也要反映到面板上
      void refreshUpstream(path).catch(ignore)
      void refreshStashes(path).catch(ignore)
```

其七，顶部 import 加上新类型：

```ts
import type { CommitNode, DiffData, GraphData, RefPointer, RepoInfo, StashEntry, StatusData, UpstreamInfo, Worktree } from '../engine/types'
```

- [ ] **Step 7: 类型检查并跑测试**

```
pnpm exec tsc --noEmit && pnpm test
```

Expected: 无类型错误，全部单测通过。

- [ ] **Step 8: 提交**

```bash
git add src/engine/toolbarActions.ts src/engine/types.ts src/store/repoStore.ts tests/vitest/toolbarActions.test.ts
git commit -m "feat(ui): toolbar action planning and remote state in store"
```

---

### Task 6: Toolbar 与 StashView 组件

**Files:**
- Create: `src/components/Toolbar.tsx`
- Create: `src/components/StashView.tsx`
- Modify: `src/components/RepoTab.tsx:145-190`
- Modify: `src/styles.css`（追加）

**Interfaces:**
- Consumes: Task 5 的 `planToolbarAction`、`state.upstream`、`state.stashes`
- Produces:
  - `export function useCtrlHeld(): boolean`（在 `Toolbar.tsx` 内导出）
  - `export function Toolbar(props: ToolbarProps)`
  - `export function StashView(props: StashViewProps)`

- [ ] **Step 1: 创建 `src/components/Toolbar.tsx`**

```tsx
import { useCallback, useEffect, useState } from 'react'
import type { UpstreamInfo } from '../engine/types'
import { planToolbarAction, type ToolbarAction } from '../engine/toolbarActions'

/**
 * 跟踪 Ctrl 是否按下。
 *
 * 必须在 blur 时强制复位：按着 Ctrl 切走窗口再切回来，keyup 事件不会送到
 * 本窗口，按钮会永久卡在变体状态上——用户会在毫不知情的情况下强推。
 */
export function useCtrlHeld(): boolean {
  const [held, setHeld] = useState(false)
  useEffect(() => {
    const down = (e: KeyboardEvent) => { if (e.key === 'Control') setHeld(true) }
    const up = (e: KeyboardEvent) => { if (e.key === 'Control') setHeld(false) }
    const reset = () => setHeld(false)
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    window.addEventListener('blur', reset)
    return () => {
      window.removeEventListener('keydown', down)
      window.removeEventListener('keyup', up)
      window.removeEventListener('blur', reset)
    }
  }, [])
  return held
}

interface ToolbarProps {
  /** 当前分支名，来自 state.head。 */
  branch: string
  upstream: UpstreamInfo | null
  /** 执行一次工具栏动作；由 RepoTab 负责 invoke 与刷新。 */
  onRun: (command: string, args: Record<string, unknown>) => Promise<void>
}

const ACTIONS: ToolbarAction[] = ['fetch', 'pull', 'push', 'stash']

export function Toolbar({ branch, upstream, onRun }: ToolbarProps) {
  const ctrl = useCtrlHeld()
  const [busy, setBusy] = useState<ToolbarAction | null>(null)

  const run = useCallback(async (action: ToolbarAction) => {
    const plan = planToolbarAction(action, ctrl, branch)
    if (plan.confirm && !window.confirm(plan.confirm)) return
    setBusy(action)
    try {
      await onRun(plan.command, plan.args)
    } finally {
      setBusy(null)
    }
  }, [ctrl, branch, onRun])

  /** Pull 显示落后数、Push 显示领先数；没有上游就不显示。 */
  const badge = (action: ToolbarAction): number => {
    if (!upstream?.upstream) return 0
    if (action === 'pull') return upstream.behind
    if (action === 'push') return upstream.ahead
    return 0
  }

  return (
    <div className="toolbar">
      <span className="toolbar-branch" title={upstream?.upstream ?? 'no upstream'}>
        ⎇ {branch}
      </span>
      <span className="toolbar-sep" />
      {ACTIONS.map((action) => {
        const plan = planToolbarAction(action, ctrl, branch)
        const count = badge(action)
        return (
          <button
            key={action}
            className={`toolbar-button${ctrl ? ' variant' : ''}`}
            disabled={busy !== null}
            onClick={() => void run(action)}
            data-action={action}
          >
            {busy === action ? '…' : plan.label}
            {count > 0 && (
              <span className="toolbar-badge">
                {action === 'pull' ? '↓' : '↑'}{count}
              </span>
            )}
          </button>
        )
      })}
    </div>
  )
}
```

- [ ] **Step 2: 创建 `src/components/StashView.tsx`**

```tsx
import type { StashEntry } from '../engine/types'
import { ContextMenu, useContextMenu } from './ContextMenu'

interface StashViewProps {
  stashes: StashEntry[]
  /** 点击条目时用它的 hash 走 `commit:<hash>` diff scope。 */
  onSelect: (hash: string) => void
  onApply: (index: number) => void
  onPop: (index: number) => void
  onDrop: (index: number) => void
}

export function StashView({ stashes, onSelect, onApply, onPop, onDrop }: StashViewProps) {
  const { menu, open, close } = useContextMenu()

  return (
    <div className="sidebar-panel">
      <h3>Stashes</h3>
      {stashes.length === 0 && <div className="changes-empty">No stashes</div>}
      {stashes.map((entry) => {
        const items = [
          { label: 'Apply', onSelect: () => onApply(entry.index) },
          { label: 'Pop', onSelect: () => onPop(entry.index) },
          { label: 'Drop', onSelect: () => onDrop(entry.index), danger: true },
        ]
        return (
          <div
            key={entry.hash}
            className="stash-row"
            title="点击查看内容 · 右键更多操作"
            onClick={() => onSelect(entry.hash)}
            onContextMenu={(e) => open(e, items)}
          >
            <span className="stash-index">{`{${entry.index}}`}</span>
            <span className="stash-message">{entry.message}</span>
          </div>
        )
      })}
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}
```

- [ ] **Step 3: 样式追加到 `src/styles.css` 末尾**

```css
/* 顶部 Git 操作工具栏 */
.toolbar {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 8px;
  background: #252525;
  border-bottom: 1px solid #333;
  flex-shrink: 0;
}

.toolbar-branch {
  font-family: monospace;
  color: #4fc3f7;
  max-width: 260px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.toolbar-sep {
  width: 1px;
  align-self: stretch;
  background: #444;
  margin: 0 4px;
}

.toolbar-button {
  display: flex;
  align-items: center;
  gap: 4px;
  padding: 3px 10px;
  background: #333;
  border: 1px solid #555;
  border-radius: 3px;
  color: var(--fg);
  cursor: pointer;
  white-space: nowrap;
}

.toolbar-button:hover:not(:disabled) {
  background: #3a6ea5;
  border-color: #4fc3f7;
}

.toolbar-button:disabled {
  color: #666;
  cursor: default;
}

/* 按住 Ctrl 时整排按钮换色，让「现在按下去是变体动作」显而易见 */
.toolbar-button.variant {
  background: #4a3a2a;
  border-color: #ffb74d;
  color: #ffcc80;
}

.toolbar-badge {
  padding: 0 4px;
  border-radius: 7px;
  background: #4fc3f7;
  color: #1e1e1e;
  font-size: 10px;
  font-weight: bold;
}

.stash-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 2px 4px;
  cursor: pointer;
}

.stash-row:hover {
  background: rgba(255, 255, 255, 0.05);
}

.stash-index {
  color: #888;
  font-family: monospace;
  flex-shrink: 0;
}

.stash-message {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
```

- [ ] **Step 4: 在 `src/components/RepoTab.tsx` 挂载**

顶部加 import：

```tsx
import { Toolbar } from './Toolbar'
import { StashView } from './StashView'
```

替换 `src/components/RepoTab.tsx:70-78` 的解构与 effects 调用：

```tsx
  const {
    state, openRepo, refreshHead, refreshGraph, refreshStatus, refreshWorktrees,
    refreshDiff, refreshUpstream, refreshStashes, selectCommit, clearError, reportError,
  } = useRepoStore()
  const [repoState, setRepoState] = useState<string>('normal')
  const [viewMode, setViewMode] = useState<ViewMode>('commits')
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null)

  useRepoEffects(
    state,
    { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, refreshUpstream, refreshStashes },
    viewMode,
  )
```

加一个统一的执行器（放在 `runAndRefresh` 之后）：

```tsx
  /** 工具栏动作：跑完把所有受影响的状态刷一遍。 */
  const runToolbar = useCallback(async (command: string, args: Record<string, unknown>) => {
    const path = state.path
    if (!path) return
    try {
      await invoke(command, { path, ...args })
      // fetch/pull/push/stash 会同时影响图、状态、上游计数和 stash 列表
      await refreshHead(path)
      await refreshGraph(path)
      await refreshStatus(path)
      await refreshUpstream(path)
      await refreshStashes(path)
    } catch (err) {
      reportError(err)
    }
  }, [state.path, refreshHead, refreshGraph, refreshStatus, refreshUpstream, refreshStashes, reportError])
```

在 `<div className="repo-tab">` 内、错误横幅之后、`repo-path-bar` 之前插入：

```tsx
      <Toolbar branch={state.head} upstream={state.upstream} onRun={runToolbar} />
```

在侧边栏 `<WorktreeView ... />` 之后插入：

```tsx
              <StashView
                stashes={state.stashes}
                onSelect={(hash) => { setViewMode('commits'); selectCommit(hash) }}
                onApply={(index) => runAndRefresh('stash_apply', { index }, async () => {
                  await refreshStatus(state.path!)
                  await refreshStashes(state.path!)
                })}
                onPop={(index) => runAndRefresh('stash_pop', { index }, async () => {
                  await refreshStatus(state.path!)
                  await refreshStashes(state.path!)
                })}
                onDrop={(index) => runAndRefresh('stash_drop', { index }, async () => {
                  await refreshStashes(state.path!)
                })}
              />
```

- [ ] **Step 5: 类型检查并跑测试**

```
pnpm exec tsc --noEmit && pnpm test
```

Expected: 无类型错误，全部单测通过。

- [ ] **Step 6: 提交**

```bash
git add src/components/Toolbar.tsx src/components/StashView.tsx src/components/RepoTab.tsx src/styles.css
git commit -m "feat(ui): git operations toolbar and stash panel"
```

---

### Task 7: 运行时验证

**Files:**
- Modify: `docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md`（追加本轮条目）

组件行为靠 CDP 对着真实 WebView2 验证，方法沿用上一轮：以
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 启动 `pnpm tauri dev`，
再用 `Runtime.evaluate` / `Page.captureScreenshot` 检查。

**先准备一个可以随便折腾的仓库**，不要拿用户的真实仓库测 push / force push：

```bash
cd "$TMPDIR" && mkdir -p e2egit-remote && git init --bare e2egit-remote
git clone e2egit-remote e2egit-local
```

- [ ] **Step 1: 启动应用并打开这个测试仓库**

```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 pnpm tauri dev
```

- [ ] **Step 2: 逐条核对**

1. 工具栏出现在顶部，显示当前分支名与四个按钮。
2. 按住 Ctrl，四个按钮的**文字同时变化**（Fetch all / Pull (rebase) / Force push / Stash all）且整排换成橙色。
3. 松开 Ctrl，按钮全部还原。
4. **按住 Ctrl → Alt+Tab 切走 → 切回来**，按钮必须已经还原成默认态。这是 `useCtrlHeld` 里 blur 复位那段的验证点；不复位的话用户会在不知情的情况下强推。
5. 在命令行给远程推一个提交，点 Fetch，`origin/master` 标签移动。
6. 点 Pull，工作区出现远程那个文件，Pull 按钮上的 ↓ 徽章消失。
7. 本地提交一次，Push 按钮出现 ↑1 徽章；点 Push 后徽章消失。
8. 新建一个分支并提交，点 Push（此时无上游），成功且 `git rev-parse --abbrev-ref @{upstream}` 能读到 `origin/<branch>`。
9. Ctrl+点 Push **弹出确认框**，文案里带分支名；点取消则什么都不发生。
10. 改一个文件，点 Stash，Stashes 面板出现一条，工作区恢复干净。
11. 点该 stash 条目，下方 diff 面板显示它的内容。
12. 右键 → Pop，改动回到工作区，面板条目消失。
13. 新建一个未跟踪文件，Ctrl+点 Stash，该文件也被存进去（工作区里消失）。
14. **认证不挂死**：把 remote 改成一个需要认证的假地址（`git remote set-url origin https://example.invalid/x.git`），点 Fetch，必须在几秒内弹出红色错误横幅，**而不是**按钮一直转圈。这是 `GIT_TERMINAL_PROMPT=0` 的验证点。

- [ ] **Step 3: 把结果记进 E2E 清单并提交**

在 `docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md` 追加一节
「2026-08-06 Git 操作工具栏」，逐条记录通过/失败与证据。

```bash
git add docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md
git commit -m "docs(e2e): record git toolbar verification"
```

---

## 完成标准

- `cd src-tauri && cargo test` 全绿，含新增的 git_cli（3）、upstream（4）、remote（6）、stash（8）用例。
- `pnpm test` 全绿，含新增的 toolbarActions（10）用例。
- `pnpm exec tsc --noEmit` 无错误。
- Task 7 第 4 条（Ctrl + Alt+Tab 后复位）通过。
- Task 7 第 9 条（强推确认框）通过。
- Task 7 第 14 条（认证失败快速报错而非挂死）通过。

## 明确不做

- git 进度输出解析（fetch/push 的百分比）。本轮只做忙碌态。
- 取消正在进行的网络操作。
- merge / rebase / cherry-pick 的工具栏入口。
- 选择 remote / 分支的完整选项对话框（用户已选「变体动作」而非「弹对话框」）。
