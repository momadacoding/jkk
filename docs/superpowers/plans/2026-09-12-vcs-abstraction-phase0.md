# VCS 抽象层 Phase 0 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Rust 侧引入 `trait Vcs` 与 `GitBackend`，把现有 git 命令改为经由后端调用，**行为零变化**。

**Architecture:** 新增 `src-tauri/src/vcs/` 模块。`GitBackend` 绑定到一个工作目录，方法体**委托给现有 `engine::*`**，不搬动 `engine/` 下任何代码。只有两处例外——`commands/ops.rs` 的 discard 分类逻辑和 `commands/stash.rs` 的参数拼装，它们目前直接写在命令函数里，需要下沉到 `engine/` 才能被后端调用。`commands/*.rs` 的**函数签名一律不变**，只把函数体换成 `backend_for(&path)?.xxx()`。

**Tech Stack:** Rust 2021 / rustc 1.77.2、Tauri 2.11.3、git2 0.20、tempfile 3.14（dev）

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（§4 抽象层、§16 分期）

## Global Constraints

逐条来自 spec 与 `AGENTS.md`，每个 Task 都隐含包含本节。

- **行为零变化。** Phase 0 不新增任何用户可见功能，不改前端一行代码。验收标准是现有 18 个测试文件（4038 行）全绿 + 手工 E2E 与改动前无差异。
- **命令函数签名一律不变。** `src-tauri/tests/` 下有 6 个测试文件直接调用命令层：`commit_ops.rs`、`discard.rs`、`remote_branch.rs` 用 `app_lib::commands::ops`，`stash.rs` 用 `app_lib::commands::stash`，`worktree_ops.rs` 用 `app_lib::commands::worktree`，`remote.rs` 用 `app_lib::commands::remote`。改签名会连带改测试，那就不再是「零行为变化」的重构。
- **`engine/` 下现有函数不得移动、改名或改签名。** `src-tauri/tests/fixtures.rs` 等文件直接调 `app_lib::engine::graph::load` 这类路径。
- **所有 `#[tauri::command]` 必须写成 `#[tauri::command(async)]`。** `tests/commands_are_async.rs` 会扫 `src/commands/*.rs` 强制这一点。注意它**只扫 `src/commands/`**，所以 `src/vcs/` 下不得放任何 `#[tauri::command]`。
- **不新增任何依赖。** 不动 `Cargo.toml`。
- **不进抽象层**（spec §4.3）：worktree、tag、remote 分支、PR、cherry-pick、revert、`read_upstream`、`repo_state`、custom、session、watch。这些保持现状，一个字不改。
- **Phase 0 不做 `capabilities()`**：它要等 Phase 1 前端消费才有意义，现在加进来就是 dead code，会触发 clippy 告警，而 `AGENTS.md` §7.3 禁止用 `#[allow(...)]` 掩盖。
- **Phase 0 不做 `sync_in` / `sync_out` / `external_changes`**：Phase 2–4 才需要，现在加只能写成 `unimplemented!()`。
- 注释用中文，解释「为什么」不解释「做了什么」，与现有代码一致（`AGENTS.md` §7.3）。
- 每个 Task 结束必须跑：`cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`。

## 与 spec 的一处有意偏离

spec §4.1 把 `trait Vcs` 的每个方法都写成带 `path: &str` 参数。本计划**改为后端构造时绑定工作目录，方法不再带 path**。

理由：Phase 1 的 `SvnGitBackend` 需要持有每仓库状态（白名单、`svnUrl`、基线分支名，来自 `.git/mygitclient.json`）。若每次调用都传 path，配置就要在每个方法里重复读盘解析。绑定式构造让 Phase 1 只在 `backend_for()` 里读一次配置。

这条偏离必须记进 decisions 文件（见前置步骤），review 时按 `AGENTS.md` §13 第 12 条逐条核对。

---

## 前置步骤（动第一行代码之前）

- [ ] **P1: 确认当前分支状态**

```bash
git -C . rev-parse --abbrev-ref HEAD
git -C . status --short
git -C . log --oneline -3
```

期望：在 `master` 上、工作区干净、HEAD 是 `79c19f7` 或更新的文档提交。
若当前分支不是 `master` 或有未提交改动，**停下来问用户**，不要自行处理。

- [ ] **P2: 创建并切入独立 worktree**

`AGENTS.md` §4.1 要求功能开发在独立 worktree 中进行，且必须在**动第一行代码之前**切入。

Claude Code：调用 `EnterWorktree` 工具，name 用 `vcs-phase0`。
其他 agent：用自身可用的会话级 worktree/workspace 切换入口。

**只用 `git worktree add` 建目录、或用 `git -C <path>` 跨目录操作，都不算切入，属违规。**

- [ ] **P3: 验证真的切进去了**

```bash
pwd
git rev-parse --abbrev-ref HEAD
git log --oneline -3
```

期望：`pwd` 落在 worktree 目录内（不是原来的 `mygitclient`），分支是 `worktree-vcs-phase0`，
`git log` 的基点 commit 与 master 的 HEAD 一致。
任一不符**立即停止**，先切进去再继续。

- [ ] **P4: 建立 decisions 文件**

`AGENTS.md` §2.1 要求：执行 `docs/superpowers/plans/` 下的计划、或预计 ≥3 个 Task 的工作，必须在**动第一个 Task 之前**建好决策日志。

创建 `docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md`：

```markdown
# VCS 抽象层 Phase 0 决策日志

计划：`docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0.md`
规格：`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 1 只读四件套 | 未开始 | | |
| 2 discard 下沉 | 未开始 | | |
| 3 暂存与提交 | 未开始 | | |
| 4 分支操作 | 未开始 | | |
| 5 stash | 未开始 | | |
| 6 收尾验证 | 未开始 | | |

## 决策条目

### 全局

#### 偏离计划
- **trait 方法不带 `path` 参数**，改为后端构造时绑定工作目录。spec §4.1 的签名里
  每个方法都带 `path: &str`。理由见计划「与 spec 的一处有意偏离」一节：Phase 1 的
  `SvnGitBackend` 要持有每仓库配置，按 path 传参会导致每次调用重复读盘。

#### 计划的错误
（实现时发现计划里写错的代码、命令、断言，记在这里）

#### 计划没写的坑
（实现时才暴露的约束，记在这里）

#### 为什么这么改是安全的
- Phase 0 不碰任何破坏性 git 操作，不改前端，不改命令签名。安全性论证的核心是
  **现有 4038 行集成测试全绿**——它们直接覆盖命令层和 engine 层两条路径。
```

- [ ] **P5: 提交 decisions 文件**

计划文件本身已经在 `master` 上提交过了（纯文档，走 `AGENTS.md` §4.1 例外），
worktree 从 master 切出来时就带着它。这里只提交新建的 decisions：

```bash
git add docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "docs(vcs): Phase 0 决策日志"
```

- [ ] **P6: 记录重构前的基线**

```bash
cd src-tauri
cargo test 2>&1 | tail -30
```

把「N passed」的数字抄进 decisions 文件的进度总账表头。**Task 6 要拿它对账**——
测试数量只能增不能减，减了就说明有测试被意外跳过或删除。

---

## File Structure

| 文件 | 责任 | 动作 |
|---|---|---|
| `src-tauri/src/vcs/mod.rs` | `trait Vcs` 定义 + `backend_for()` 分发入口 | 新建 |
| `src-tauri/src/vcs/git.rs` | `GitBackend`，委托给 `engine::*` | 新建 |
| `src-tauri/src/engine/ops.rs` | discard 的三路分类逻辑（从 `commands/ops.rs` 下沉） | 新建 |
| `src-tauri/src/engine/stash.rs` | 新增 `push` / `apply` / `pop` / `drop`（从 `commands/stash.rs` 下沉） | 修改 |
| `src-tauri/src/engine/mod.rs` | 注册 `pub mod ops;` | 修改 |
| `src-tauri/src/lib.rs` | 注册 `pub mod vcs;` | 修改 |
| `src-tauri/src/commands/{repo,graph,diff,ops,stash}.rs` | 函数体改为经后端调用，**签名不变** | 修改 |
| `src-tauri/tests/vcs_backend.rs` | 后端与 engine 的**等价性**测试 | 新建 |

`vcs/` 拆成 `mod.rs` + `git.rs` 两个文件，是为 Phase 1 的 `vcs/svn_git.rs` 留位置——
把 `GitBackend` 和 trait 写在同一个文件里，Phase 1 加第二个后端时就得先拆一次。

---

## Task 1: vcs 模块骨架与只读四件套

**Files:**
- Create: `src-tauri/src/vcs/mod.rs`
- Create: `src-tauri/src/vcs/git.rs`
- Create: `src-tauri/tests/vcs_backend.rs`
- Modify: `src-tauri/src/lib.rs`（加 `pub mod vcs;`）
- Modify: `src-tauri/src/commands/repo.rs`（只改 `open_repo`）
- Modify: `src-tauri/src/commands/graph.rs`
- Modify: `src-tauri/src/commands/diff.rs`

**Interfaces:**
- Consumes: `engine::repo::open`、`engine::graph::load`、`engine::diff::read_status`、`engine::diff::read_diff`（签名见下方代码，已核实）
- Produces:
  - `pub trait Vcs`，Task 3/4/5 会往里加方法
  - `pub fn backend_for(path: &str) -> Result<Box<dyn Vcs>>`
  - `pub struct GitBackend { root: String }` 与 `GitBackend::new(path: &str) -> Self`

- [ ] **Step 1: 写等价性测试**

创建 `src-tauri/tests/vcs_backend.rs`：

```rust
//! 后端与 engine 的**等价性**测试。
//!
//! Phase 0 是纯重构，验收标准是「经 trait 调用与直接调 engine 结果一致」。
//! 这比「后端能跑通」强得多——后者一个返回固定值的桩也能通过。

use app_lib::engine;
use app_lib::vcs::backend_for;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

/// 一个有两条提交、一个未暂存改动、一个未跟踪文件的仓库。
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);
    fs::write(dir.path().join("a.txt"), "1\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2\n").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    // 留下工作区状态给 status/diff 用
    fs::write(dir.path().join("a.txt"), "3\n").unwrap();
    fs::write(dir.path().join("untracked.txt"), "u\n").unwrap();
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn backend_open_matches_engine() {
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().open().unwrap();
    let via_engine = engine::repo::open(&path).unwrap();
    assert_eq!(via_backend.head, via_engine.head);
    assert_eq!(via_backend.head_hash, via_engine.head_hash);
    assert_eq!(via_backend.path, via_engine.path);
}

#[test]
fn backend_graph_matches_engine() {
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().graph().unwrap();
    let via_engine = engine::graph::load(&path).unwrap();
    assert_eq!(via_backend.commits.len(), via_engine.commits.len());
    assert_eq!(via_backend.refs.len(), via_engine.refs.len());
    let ids: Vec<&String> = via_backend.commits.iter().map(|c| &c.id).collect();
    let expect: Vec<&String> = via_engine.commits.iter().map(|c| &c.id).collect();
    assert_eq!(ids, expect);
}

#[test]
fn backend_status_matches_engine() {
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().status().unwrap();
    let via_engine = engine::diff::read_status(&path).unwrap();
    assert_eq!(via_backend.staged.len(), via_engine.staged.len());
    assert_eq!(via_backend.unstaged.len(), via_engine.unstaged.len());
    // 非空断言：仓库里确实有改动，否则这个测试等于什么都没测
    assert!(!via_backend.unstaged.is_empty(), "fixture 应当有未暂存改动");
}

#[test]
fn backend_diff_matches_engine() {
    // scope 用 "workdir"：这是前端实际传的值（RepoTab.tsx 里 refreshDiff 的调用点），
    // 也是现有 tests/fixtures.rs 用的值。read_diff 只特判 "commit:" 前缀、其余一律
    // 走 workdir 分支，所以随便写个字符串也能通过——但那样测的就不是真实输入形态了。
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().diff("workdir").unwrap();
    let via_engine = engine::diff::read_diff(&path, "workdir".to_string()).unwrap();
    assert_eq!(via_backend.files.len(), via_engine.files.len());
    assert!(!via_backend.files.is_empty(), "fixture 应当有可 diff 的文件");
}

#[test]
fn backend_diff_handles_commit_scope() {
    // 另一条真实形态：详情面板点一条提交时传的是 "commit:<完整 oid>"
    let dir = repo();
    let path = p(&dir);
    let head = backend_for(&path).unwrap().open().unwrap().head_hash;
    let scope = format!("commit:{head}");
    let via_backend = backend_for(&path).unwrap().diff(&scope).unwrap();
    let via_engine = engine::diff::read_diff(&path, scope.clone()).unwrap();
    assert_eq!(via_backend.files.len(), via_engine.files.len());
    assert!(!via_backend.files.is_empty(), "c2 相对 c1 应当有变更");
}

#[test]
fn backend_for_rejects_non_repo() {
    let dir = TempDir::new().unwrap();
    // 空目录不是 git 仓库。backend_for 本身只是构造，错误在 open() 时才暴露——
    // 这与重构前 open_repo 的行为一致，不能提前到构造时报错。
    let backend = backend_for(dir.path().to_str().unwrap()).unwrap();
    let err = backend.open().unwrap_err();
    assert_eq!(err.code, "not_repo");
}
```

- [ ] **Step 2: 跑测试确认编译失败**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：编译失败，`unresolved import app_lib::vcs` / `could not find vcs in app_lib`。

- [ ] **Step 3: 写 trait 与分发入口**

创建 `src-tauri/src/vcs/mod.rs`：

```rust
//! 版本控制后端抽象。
//!
//! 一个后端**绑定到一个工作目录**，而不是每个方法都收一个 path。
//! 这是为 Phase 1 的 SVN 后端准备的：它要持有白名单、svnUrl、基线分支名
//! 这些每仓库状态（来自 `.git/mygitclient.json`），按 path 传参会让每个方法
//! 都重复读盘解析一次配置。
//!
//! Phase 0 只有 GitBackend 一个实现，方法体一律委托给 `engine::*`——
//! 这一期的目的是把接缝切出来，不是改行为。

pub mod git;

use crate::engine::diff::{DiffData, StatusData};
use crate::engine::graph::GraphData;
use crate::engine::repo::RepoInfo;
use crate::error::Result;

/// 绑定到某个工作目录的版本控制后端。
///
/// `Send + Sync` 是给 Phase 2 的仓库级操作锁准备的：那个注册表要跨线程持有
/// 后端实例。现在不加，到时候就是一次涉及所有实现的破坏性改动。
pub trait Vcs: Send + Sync {
    /// 读取仓库基本信息（HEAD 名与 oid）。
    fn open(&self) -> Result<RepoInfo>;

    /// 读取提交图。
    fn graph(&self) -> Result<GraphData>;

    /// 读取工作区状态（已暂存 / 未暂存）。
    fn status(&self) -> Result<StatusData>;

    /// 读取指定 scope 的 diff。scope 的取值形式由各后端自行解释。
    fn diff(&self, scope: &str) -> Result<DiffData>;
}

/// 为某个工作目录挑一个后端。
///
/// Phase 0 永远返回 GitBackend。**不要在这里加 `.svn` 判断**——现在加会改变
/// 「一个同时有 .svn 和 .git 的目录」的现有行为（今天它就是个普通 git 仓库），
/// 那就不是零行为变化了。SVN 分支在 Phase 1 连同配置文件识别一起加。
pub fn backend_for(path: &str) -> Result<Box<dyn Vcs>> {
    Ok(Box::new(git::GitBackend::new(path)))
}
```

- [ ] **Step 4: 写 GitBackend**

创建 `src-tauri/src/vcs/git.rs`：

```rust
//! git 后端。方法体逐条委托给 `engine::*`，不搬动 engine 里的任何代码。
//!
//! 之所以是薄委托而不是把 engine 的实现搬进来：`src-tauri/tests/` 下多个测试
//! 文件直接调 `app_lib::engine::graph::load` 这类路径，搬动会连带改测试，
//! 而 Phase 0 的验收标准就是现有测试一行不改还能全绿。

use super::Vcs;
use crate::engine;
use crate::engine::diff::{DiffData, StatusData};
use crate::engine::graph::GraphData;
use crate::engine::repo::RepoInfo;
use crate::error::Result;

pub struct GitBackend {
    /// 工作目录根。原样保存调用方给的写法，不做规范化——
    /// `engine::*` 收到的一直是这个原始字符串，改写法会动到行为。
    root: String,
}

impl GitBackend {
    pub fn new(path: &str) -> Self {
        Self {
            root: path.to_string(),
        }
    }
}

impl Vcs for GitBackend {
    fn open(&self) -> Result<RepoInfo> {
        engine::repo::open(&self.root)
    }

    fn graph(&self) -> Result<GraphData> {
        engine::graph::load(&self.root)
    }

    fn status(&self) -> Result<StatusData> {
        engine::diff::read_status(&self.root)
    }

    fn diff(&self, scope: &str) -> Result<DiffData> {
        // engine 侧收 String，这里做一次转换。trait 用 &str 是为了调用方省事，
        // 多出来的这次分配相对一次 diff 计算可以忽略。
        engine::diff::read_diff(&self.root, scope.to_string())
    }
}
```

- [ ] **Step 5: 注册模块**

修改 `src-tauri/src/lib.rs`，在 `pub mod error;` 之后加一行：

```rust
pub mod commands;
pub mod engine;
pub mod error;
pub mod vcs;
```

- [ ] **Step 6: 跑测试确认通过**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：6 个测试全部 PASS。

- [ ] **Step 7: 把三个命令改为经后端调用**

`src-tauri/src/commands/graph.rs` 全文替换为：

```rust
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn load_graph(path: String) -> Result<crate::engine::graph::GraphData, crate::error::GitError> {
    backend_for(&path)?.graph()
}
```

`src-tauri/src/commands/diff.rs` 全文替换为：

```rust
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn read_status(
    path: String,
) -> Result<crate::engine::diff::StatusData, crate::error::GitError> {
    backend_for(&path)?.status()
}

#[tauri::command(async)]
pub fn read_diff(
    path: String,
    scope: String,
) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    backend_for(&path)?.diff(&scope)
}
```

`src-tauri/src/commands/repo.rs` 中**只改 `open_repo` 一个函数**，`read_upstream` 和
`repo_state` 保持原样不动（它们是 git 专属，不进抽象层，见 Global Constraints）：

```rust
use crate::engine;
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn open_repo(path: String) -> Result<crate::engine::repo::RepoInfo, crate::error::GitError> {
    backend_for(&path)?.open()
}
```

`read_upstream` 仍然用到 `engine`，所以 `use crate::engine;` 这一行必须保留。

- [ ] **Step 8: 全量验证**

```bash
cd src-tauri
cargo fmt --check && cargo clippy --all-targets && cargo test
```

期望：三条全过，测试总数 = P6 记录的基线 + 6。

- [ ] **Step 9: 更新 decisions 并提交**

在 decisions 文件的进度总账里把 Task 1 标为「完成」，填上产出文件与验证结果。

```bash
git add src-tauri/src/vcs/ src-tauri/src/lib.rs src-tauri/src/commands/graph.rs src-tauri/src/commands/diff.rs src-tauri/src/commands/repo.rs src-tauri/tests/vcs_backend.rs docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "refactor(vcs): 引入 trait Vcs 与 GitBackend，只读命令改走后端"
```

---

## Task 2: discard 分类逻辑下沉到 engine

**Files:**
- Create: `src-tauri/src/engine/ops.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Modify: `src-tauri/src/commands/ops.rs:1-100`（删掉 `path_set` 与 `discard` 的函数体）

**Interfaces:**
- Consumes: `engine::git_cli::run_git`
- Produces: `pub fn engine::ops::discard(path: &str, paths: &[String], staged: bool) -> Result<()>`

这一步**不碰 trait**。先把逻辑挪到 engine 并证明行为没变，再在 Task 3 接进 trait——
两件事混在一个 commit 里，出问题时分不清是「挪坏了」还是「接坏了」。

- [ ] **Step 1: 确认现有 discard 测试全绿（重构前基线）**

```bash
cd src-tauri && cargo test --test discard
```

期望：全部 PASS。把用例数抄进 decisions。这批测试是本 Task 唯一的安全网。

- [ ] **Step 2: 新建 engine/ops.rs**

创建 `src-tauri/src/engine/ops.rs`，内容是从 `commands/ops.rs` **原样搬过来**的
`path_set` 与 `discard`，只改两处签名（`path: String` → `&str`，`paths: Vec<String>` → `&[String]`）：

```rust
//! 工作区文件操作。
//!
//! 这段逻辑原本直接写在 `commands/ops.rs` 的命令函数里。下沉到 engine 是为了
//! 让 `vcs::GitBackend` 能调用它——命令层往后只做分发，不放业务逻辑。

use crate::engine::git_cli::run_git;
use crate::error::Result;
use std::collections::HashSet;

/// 跑一条会输出 NUL 分隔路径列表的 git 命令，收成集合。
///
/// 命令失败当作空集：`ls-tree HEAD` 在还没有任何提交的仓库里就会失败，
/// 那正是「HEAD 里什么都没有」，不是错误。
fn path_set(path: &str, args: &[&str]) -> HashSet<String> {
    match run_git(path, args) {
        Ok(out) => out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        Err(_) => HashSet::new(),
    }
}

/// 丢弃改动。
///
/// 三类文件走三条完全不同的路，混在一次多选里是常态：
///
/// - **未跟踪**：git 里没有任何副本，只能直接删文件。这一步不可逆，界面必须先确认。
/// - **HEAD 里没有、但已进暂存区**（新加的文件）：`git rm -f` 同时从索引和磁盘去掉。
/// - **其余**：`git checkout` 恢复。
///
/// `staged` 决定恢复到哪一层：Unstaged 区的 Discard 只回退「工作区相对暂存区」的
/// 部分（`git checkout --`），把已暂存的改动一起抹掉是 Staged 区的语义
/// （`git checkout HEAD --`）。
pub fn discard(path: &str, paths: &[String], staged: bool) -> Result<()> {
    // 空 pathspec 会被 git 当成「全部」，那就成了一次误伤整个仓库的 discard
    if paths.is_empty() {
        return Ok(());
    }

    let mut probe: Vec<&str> = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    probe.extend(paths.iter().map(String::as_str));
    let untracked = path_set(path, &probe);

    let mut head_probe: Vec<&str> = vec!["ls-tree", "-r", "HEAD", "--name-only", "-z", "--"];
    head_probe.extend(paths.iter().map(String::as_str));
    let in_head = path_set(path, &head_probe);

    let mut restore: Vec<&str> = Vec::new();
    let mut remove: Vec<&str> = Vec::new();
    let mut unlink: Vec<&str> = Vec::new();
    for p in paths {
        if untracked.contains(p) {
            unlink.push(p);
        } else if staged && !in_head.contains(p) {
            remove.push(p);
        } else {
            restore.push(p);
        }
    }

    if !restore.is_empty() {
        let mut args: Vec<&str> = vec!["checkout"];
        if staged {
            args.push("HEAD");
        }
        args.push("--");
        args.extend(restore);
        run_git(path, &args)?;
    }

    if !remove.is_empty() {
        let mut args: Vec<&str> = vec!["rm", "-f", "--"];
        args.extend(remove);
        run_git(path, &args)?;
    }

    // 直接删而不是 `git clean -f`：pathspec 稍有偏差就会波及没被选中的文件，
    // 而这一步不可逆
    let root = std::path::Path::new(path);
    for rel in unlink {
        let target = root.join(rel);
        let result = if target.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        // 已经不在了就当成功——用户要的就是它消失
        if let Err(err) = result {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err.into());
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 3: 注册模块**

修改 `src-tauri/src/engine/mod.rs`，在 `pub mod launcher;` 与 `pub mod path;` 之间插入
（保持字母序，与现有风格一致）：

```rust
pub mod ops;
```

- [ ] **Step 4: 把命令函数体换成一行委托**

修改 `src-tauri/src/commands/ops.rs`：

1. 删除文件顶部的 `use std::collections::HashSet;`
2. 删除整个 `path_set` 函数（含其上方的文档注释）
3. 把 `discard` 的整个函数体与文档注释替换为：

```rust
/// 丢弃改动。分类逻辑在 `engine::ops::discard`。
#[tauri::command(async)]
pub fn discard(path: String, paths: Vec<String>, staged: bool) -> Result<()> {
    crate::engine::ops::discard(&path, &paths, staged)
}
```

文件顶部的 `use crate::engine::git_cli::run_git;` 和 `use crate::error::Result;`
**必须保留**——本文件里其余十几个命令还在用它们。

- [ ] **Step 5: 跑 discard 测试确认行为没变**

```bash
cd src-tauri && cargo test --test discard
```

期望：用例数与 Step 1 一致，全部 PASS。

**若有任何一条失败，说明搬运时改动了逻辑**——逐字比对搬过去的代码与
`git show HEAD:src-tauri/src/commands/ops.rs` 的原文，不要「顺手修一下」。

- [ ] **Step 6: 全量验证**

```bash
cd src-tauri
cargo fmt --check && cargo clippy --all-targets && cargo test
```

clippy 特别注意：`commands/ops.rs` 删掉 `path_set` 后若有未使用的 import 残留，
会报 `unused_imports`。按提示删干净，不要加 `#[allow]`。

- [ ] **Step 7: 更新 decisions 并提交**

```bash
git add src-tauri/src/engine/ops.rs src-tauri/src/engine/mod.rs src-tauri/src/commands/ops.rs docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "refactor(engine): discard 分类逻辑从命令层下沉到 engine::ops"
```

---

## Task 3: 暂存与提交进 trait

**Files:**
- Modify: `src-tauri/src/vcs/mod.rs`（trait 加 4 个方法）
- Modify: `src-tauri/src/vcs/git.rs`（实现这 4 个方法）
- Modify: `src-tauri/src/commands/ops.rs`（`stage` / `unstage` / `discard` / `commit` 改为委托）
- Modify: `src-tauri/tests/vcs_backend.rs`（追加测试）

**Interfaces:**
- Consumes: `Vcs`、`GitBackend`（Task 1）、`engine::ops::discard`（Task 2）、`engine::git_cli::run_git`
- Produces: `Vcs::stage`、`Vcs::unstage`、`Vcs::discard`、`Vcs::commit`

- [ ] **Step 1: 写失败测试**

在 `src-tauri/tests/vcs_backend.rs` 末尾追加：

```rust
#[test]
fn backend_stage_then_commit_advances_head() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();
    let before = backend.open().unwrap().head_hash;

    backend.stage(&["a.txt".to_string()]).unwrap();
    let staged = backend.status().unwrap();
    assert_eq!(staged.staged.len(), 1, "a.txt 应当已进暂存区");

    backend.commit("c3", false).unwrap();
    let after = backend.open().unwrap().head_hash;
    assert_ne!(before, after, "提交后 HEAD 必须前进");
}

#[test]
fn backend_unstage_removes_from_index() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.stage(&["a.txt".to_string()]).unwrap();
    assert_eq!(backend.status().unwrap().staged.len(), 1);

    backend.unstage(&["a.txt".to_string()]).unwrap();
    assert_eq!(backend.status().unwrap().staged.len(), 0);
    assert_eq!(backend.status().unwrap().unstaged.len(), 1, "改动应退回未暂存");
}

#[test]
fn backend_discard_restores_tracked_file() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.discard(&["a.txt".to_string()], false).unwrap();
    let content = fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(content, "2\n", "应恢复到 c2 的内容");
}

#[test]
fn backend_discard_with_empty_paths_is_a_noop() {
    // 空 pathspec 会被 git 当成「全部」。这条守住的是一次误伤整个仓库的 discard，
    // 是本项目已经踩过的坑（AGENTS.md §6.2）。
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.discard(&[], false).unwrap();

    // 工作区改动必须原封不动
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "3\n");
    assert!(dir.path().join("untracked.txt").exists(), "未跟踪文件不得被删");
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：编译失败，`no method named stage found for struct Box<dyn Vcs>`。

- [ ] **Step 3: trait 加方法**

在 `src-tauri/src/vcs/mod.rs` 的 `trait Vcs` 里，`fn diff` 之后追加：

```rust
    /// 把指定路径加入暂存区。
    fn stage(&self, paths: &[String]) -> Result<()>;

    /// 把指定路径移出暂存区。
    fn unstage(&self, paths: &[String]) -> Result<()>;

    /// 丢弃指定路径的改动。`staged` 决定回退到暂存区还是 HEAD。
    ///
    /// **空 paths 必须是 no-op**，不能退化成「全部」——那是一次误伤整个仓库的
    /// 不可逆操作。各实现都要守住这条。
    fn discard(&self, paths: &[String], staged: bool) -> Result<()>;

    /// 提交暂存区内容。
    fn commit(&self, message: &str, amend: bool) -> Result<()>;
```

- [ ] **Step 4: GitBackend 实现这 4 个方法**

在 `src-tauri/src/vcs/git.rs` 的 `impl Vcs for GitBackend` 块里，`fn diff` 之后追加：

```rust
    fn stage(&self, paths: &[String]) -> Result<()> {
        let mut args: Vec<&str> = vec!["add"];
        args.extend(paths.iter().map(String::as_str));
        engine::git_cli::run_git(&self.root, &args)?;
        Ok(())
    }

    fn unstage(&self, paths: &[String]) -> Result<()> {
        let mut args: Vec<&str> = vec!["reset", "HEAD"];
        args.extend(paths.iter().map(String::as_str));
        engine::git_cli::run_git(&self.root, &args)?;
        Ok(())
    }

    fn discard(&self, paths: &[String], staged: bool) -> Result<()> {
        engine::ops::discard(&self.root, paths, staged)
    }

    fn commit(&self, message: &str, amend: bool) -> Result<()> {
        if amend {
            engine::git_cli::run_git(&self.root, &["commit", "--amend", "-m", message])?;
        } else {
            engine::git_cli::run_git(&self.root, &["commit", "-m", message])?;
        }
        Ok(())
    }
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：10 个测试全部 PASS。

- [ ] **Step 6: 命令改为委托**

修改 `src-tauri/src/commands/ops.rs`，把这 4 个命令的函数体换成委托，**签名不变**：

```rust
#[tauri::command(async)]
pub fn stage(path: String, paths: Vec<String>) -> Result<()> {
    crate::vcs::backend_for(&path)?.stage(&paths)
}

#[tauri::command(async)]
pub fn unstage(path: String, paths: Vec<String>) -> Result<()> {
    crate::vcs::backend_for(&path)?.unstage(&paths)
}

/// 丢弃改动。分类逻辑在 `engine::ops::discard`。
#[tauri::command(async)]
pub fn discard(path: String, paths: Vec<String>, staged: bool) -> Result<()> {
    crate::vcs::backend_for(&path)?.discard(&paths, staged)
}

#[tauri::command(async)]
pub fn commit(path: String, message: String, amend: bool) -> Result<()> {
    crate::vcs::backend_for(&path)?.commit(&message, amend)
}
```

- [ ] **Step 7: 全量验证**

```bash
cd src-tauri
cargo fmt --check && cargo clippy --all-targets && cargo test
```

特别确认 `cargo test --test commit_ops` 与 `--test discard` 全绿——它们直接调命令层。

- [ ] **Step 8: 更新 decisions 并提交**

```bash
git add src-tauri/src/vcs/ src-tauri/src/commands/ops.rs src-tauri/tests/vcs_backend.rs docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "refactor(vcs): 暂存与提交命令改走后端"
```

---

## Task 4: 分支操作进 trait

**Files:**
- Modify: `src-tauri/src/vcs/mod.rs`、`src-tauri/src/vcs/git.rs`
- Modify: `src-tauri/src/commands/ops.rs`（`create_branch` / `delete_branch` / `rename_branch` / `checkout`）
- Modify: `src-tauri/tests/vcs_backend.rs`

**Interfaces:**
- Consumes: `Vcs`、`GitBackend`、`engine::git_cli::run_git`
- Produces: `Vcs::create_branch`、`Vcs::delete_branch`、`Vcs::rename_branch`、`Vcs::checkout`

**只有这 4 个进 trait。** `checkout_remote_branch`、`delete_remote_branch`、
`delete_remote_ref`、`branch_delete_info`、`create_tag`、`cherry_pick`、`revert`、
`save_patch` 全部**保持现状不动**——它们依赖远程分支、tag 这些 SVN 侧不存在的概念
（spec §4.3）。

- [ ] **Step 1: 写失败测试**

在 `src-tauri/tests/vcs_backend.rs` 末尾追加：

```rust
#[test]
fn backend_branch_lifecycle() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.create_branch("feat/x", None).unwrap();
    let refs = backend.graph().unwrap().refs;
    assert!(
        refs.iter().any(|r| r.name.ends_with("feat/x")),
        "新分支应出现在 refs 里：{:?}",
        refs.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    backend.rename_branch("feat/x", "feat/y").unwrap();
    let refs = backend.graph().unwrap().refs;
    assert!(refs.iter().any(|r| r.name.ends_with("feat/y")));
    assert!(!refs.iter().any(|r| r.name.ends_with("feat/x")));

    backend.delete_branch("feat/y").unwrap();
    let refs = backend.graph().unwrap().refs;
    assert!(!refs.iter().any(|r| r.name.ends_with("feat/y")));
}

#[test]
fn backend_create_branch_honours_start_point() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    // 从 HEAD~1（也就是 c1）开分支，它的 tip 必须不等于当前 HEAD
    backend.create_branch("from-c1", Some("HEAD~1")).unwrap();
    let head = backend.open().unwrap().head_hash;
    let refs = backend.graph().unwrap().refs;
    let branch = refs
        .iter()
        .find(|r| r.name.ends_with("from-c1"))
        .expect("分支不存在");
    assert_ne!(branch.hash, head, "start_point 没生效，分支落在了 HEAD 上");
}

#[test]
fn backend_checkout_moves_head() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.create_branch("other", None).unwrap();
    backend.checkout("other").unwrap();
    assert_eq!(backend.open().unwrap().head, "other");
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：编译失败，`no method named create_branch`。

- [ ] **Step 3: trait 加方法**

在 `src-tauri/src/vcs/mod.rs` 的 `trait Vcs` 里，`fn commit` 之后追加：

```rust
    /// 新建分支。`start_point` 为 None 时从当前 HEAD 开。
    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()>;

    /// 删除本地分支。
    fn delete_branch(&self, name: &str) -> Result<()>;

    /// 重命名本地分支。
    fn rename_branch(&self, old: &str, new: &str) -> Result<()>;

    /// 切到指定 revision。
    fn checkout(&self, rev: &str) -> Result<()>;
```

- [ ] **Step 4: GitBackend 实现**

在 `src-tauri/src/vcs/git.rs` 的 `impl Vcs for GitBackend` 块里追加：

```rust
    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()> {
        let mut args = vec!["branch", name];
        if let Some(start) = start_point {
            args.push(start);
        }
        engine::git_cli::run_git(&self.root, &args)?;
        Ok(())
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        engine::git_cli::run_git(&self.root, &["branch", "-D", name])?;
        Ok(())
    }

    fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        engine::git_cli::run_git(&self.root, &["branch", "-m", old, new])?;
        Ok(())
    }

    fn checkout(&self, rev: &str) -> Result<()> {
        engine::git_cli::run_git(&self.root, &["checkout", rev])?;
        Ok(())
    }
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：13 个测试全部 PASS。

- [ ] **Step 6: 命令改为委托**

修改 `src-tauri/src/commands/ops.rs` 的这 4 个命令，签名不变：

```rust
#[tauri::command(async)]
pub fn create_branch(path: String, name: String, start_point: Option<String>) -> Result<()> {
    crate::vcs::backend_for(&path)?.create_branch(&name, start_point.as_deref())
}

#[tauri::command(async)]
pub fn delete_branch(path: String, name: String) -> Result<()> {
    crate::vcs::backend_for(&path)?.delete_branch(&name)
}

#[tauri::command(async)]
pub fn rename_branch(path: String, old: String, new: String) -> Result<()> {
    crate::vcs::backend_for(&path)?.rename_branch(&old, &new)
}

#[tauri::command(async)]
pub fn checkout(path: String, rev: String) -> Result<()> {
    crate::vcs::backend_for(&path)?.checkout(&rev)
}
```

注意 `start_point.as_deref()`——`Option<String>` 转 `Option<&str>` 用这个，
不要写 `start_point.map(|s| s.as_str())`（借用会活不过临时值）。

- [ ] **Step 7: 全量验证**

```bash
cd src-tauri
cargo fmt --check && cargo clippy --all-targets && cargo test
```

`--test branch_delete` 与 `--test remote_branch` 必须全绿，它们覆盖分支相关的失败路径。

- [ ] **Step 8: 更新 decisions 并提交**

```bash
git add src-tauri/src/vcs/ src-tauri/src/commands/ops.rs src-tauri/tests/vcs_backend.rs docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "refactor(vcs): 本地分支操作改走后端"
```

---

## Task 5: stash 进 trait

**Files:**
- Modify: `src-tauri/src/engine/stash.rs`（新增 4 个函数）
- Modify: `src-tauri/src/vcs/mod.rs`、`src-tauri/src/vcs/git.rs`
- Modify: `src-tauri/src/commands/stash.rs`（全文替换）
- Modify: `src-tauri/tests/vcs_backend.rs`

**Interfaces:**
- Consumes: `engine::stash::list`（已有）、`engine::git_cli::run_git`
- Produces:
  - `engine::stash::push(path: &str, message: Option<&str>, include_untracked: bool) -> Result<()>`
  - `engine::stash::apply(path: &str, index: usize) -> Result<()>`
  - `engine::stash::pop(path: &str, index: usize) -> Result<()>`
  - `engine::stash::drop(path: &str, index: usize) -> Result<()>`
  - `Vcs::list_stashes`、`Vcs::stash_push`、`Vcs::stash_apply`、`Vcs::stash_pop`、`Vcs::stash_drop`

- [ ] **Step 1: 写失败测试**

在 `src-tauri/tests/vcs_backend.rs` 末尾追加：

```rust
#[test]
fn backend_stash_push_then_list_then_pop() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    assert!(backend.list_stashes().unwrap().is_empty());

    backend.stash_push(Some("wip"), false).unwrap();
    let entries = backend.list_stashes().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].message.contains("wip"));
    // stash 之后工作区应回到 HEAD 内容
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "2\n");

    backend.stash_pop(0).unwrap();
    assert!(backend.list_stashes().unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "3\n");
}

#[test]
fn backend_stash_push_untracked_flag_takes_effect() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.stash_push(Some("with-untracked"), true).unwrap();
    // -u 生效的话未跟踪文件也被收走了
    assert!(
        !dir.path().join("untracked.txt").exists(),
        "include_untracked=true 时未跟踪文件应被一并 stash"
    );

    backend.stash_apply(0).unwrap();
    assert!(dir.path().join("untracked.txt").exists());
}

#[test]
fn backend_stash_drop_removes_entry() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.stash_push(None, false).unwrap();
    assert_eq!(backend.list_stashes().unwrap().len(), 1);

    backend.stash_drop(0).unwrap();
    assert!(backend.list_stashes().unwrap().is_empty());
    // drop 不恢复改动，工作区应仍是 HEAD 内容
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "2\n");
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：编译失败，`no method named list_stashes`。

- [ ] **Step 3: engine/stash.rs 新增 4 个函数**

在 `src-tauri/src/engine/stash.rs` 末尾追加（文件顶部需要新增
`use crate::engine::git_cli::run_git;`）：

```rust
/// 把当前改动压进 stash。
///
/// 写入走 git CLI 而不是 git2：`Repository::stash_save` 的语义与 CLI 的
/// `stash push` 在未跟踪文件、pathspec 上有出入，而界面上那个「按住 Ctrl」
/// 的变体对应的正是 CLI 的 `-u`。
pub fn push(path: &str, message: Option<&str>, include_untracked: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["stash", "push"];
    // include_untracked 对应界面上按住 Ctrl 的变体
    if include_untracked {
        args.push("-u");
    }
    if let Some(msg) = message {
        args.push("-m");
        args.push(msg);
    }
    run_git(path, &args)?;
    Ok(())
}

/// 应用第 index 条 stash，保留该条目。
pub fn apply(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "apply", &format!("stash@{{{index}}}")])?;
    Ok(())
}

/// 应用第 index 条 stash 并删除它。
pub fn pop(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "pop", &format!("stash@{{{index}}}")])?;
    Ok(())
}

/// 删除第 index 条 stash，不应用。
pub fn drop(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "drop", &format!("stash@{{{index}}}")])?;
    Ok(())
}
```

- [ ] **Step 4: trait 加方法**

在 `src-tauri/src/vcs/mod.rs` 顶部的 import 里加上 `StashEntry`：

```rust
use crate::engine::stash::StashEntry;
```

在 `trait Vcs` 里，`fn checkout` 之后追加：

```rust
    /// 列出所有 stash，最新的在前。
    fn list_stashes(&self) -> Result<Vec<StashEntry>>;

    /// 把当前改动压进 stash。
    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()>;

    /// 应用第 index 条 stash，保留该条目。
    fn stash_apply(&self, index: usize) -> Result<()>;

    /// 应用第 index 条 stash 并删除它。
    fn stash_pop(&self, index: usize) -> Result<()>;

    /// 删除第 index 条 stash，不应用。
    fn stash_drop(&self, index: usize) -> Result<()>;
```

- [ ] **Step 5: GitBackend 实现**

在 `src-tauri/src/vcs/git.rs` 顶部 import 加 `use crate::engine::stash::StashEntry;`，
然后在 `impl Vcs for GitBackend` 块里追加：

```rust
    fn list_stashes(&self) -> Result<Vec<StashEntry>> {
        engine::stash::list(&self.root)
    }

    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()> {
        engine::stash::push(&self.root, message, include_untracked)
    }

    fn stash_apply(&self, index: usize) -> Result<()> {
        engine::stash::apply(&self.root, index)
    }

    fn stash_pop(&self, index: usize) -> Result<()> {
        engine::stash::pop(&self.root, index)
    }

    fn stash_drop(&self, index: usize) -> Result<()> {
        engine::stash::drop(&self.root, index)
    }
```

- [ ] **Step 6: 跑测试确认通过**

```bash
cd src-tauri && cargo test --test vcs_backend
```

期望：16 个测试全部 PASS。

- [ ] **Step 7: 命令改为委托**

`src-tauri/src/commands/stash.rs` 全文替换为：

```rust
use crate::engine::stash::StashEntry;
use crate::error::Result;
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn read_stashes(path: String) -> Result<Vec<StashEntry>> {
    backend_for(&path)?.list_stashes()
}

#[tauri::command(async)]
pub fn stash_push(path: String, message: Option<String>, include_untracked: bool) -> Result<()> {
    backend_for(&path)?.stash_push(message.as_deref(), include_untracked)
}

#[tauri::command(async)]
pub fn stash_apply(path: String, index: usize) -> Result<()> {
    backend_for(&path)?.stash_apply(index)
}

#[tauri::command(async)]
pub fn stash_pop(path: String, index: usize) -> Result<()> {
    backend_for(&path)?.stash_pop(index)
}

#[tauri::command(async)]
pub fn stash_drop(path: String, index: usize) -> Result<()> {
    backend_for(&path)?.stash_drop(index)
}
```

- [ ] **Step 8: 全量验证**

```bash
cd src-tauri
cargo fmt --check && cargo clippy --all-targets && cargo test
```

`--test stash` 必须全绿，它直接调命令层。

clippy 可能对 `engine::stash::drop` 提 `should_implement_trait`（与 `Drop::drop` 同名）。
若真报了，**不要加 `#[allow]`**，改名为 `drop_entry` 并同步改 `git.rs` 的调用点，
并把这次改名记进 decisions 的「计划的错误」。

- [ ] **Step 9: 更新 decisions 并提交**

```bash
git add src-tauri/src/engine/stash.rs src-tauri/src/vcs/ src-tauri/src/commands/stash.rs src-tauri/tests/vcs_backend.rs docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "refactor(vcs): stash 命令改走后端，参数拼装下沉 engine"
```

---

## Task 6: 收尾验证与交付

**Files:**
- Modify: `docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md`

- [ ] **Step 1: 确认没有命令被遗漏或误改**

```bash
cd src-tauri
grep -c "tauri::command" src/commands/*.rs
```

把总数与重构前对账：`git show master:src-tauri/src/lib.rs | grep -c "commands::"`
期望 44 条注册命令一条不少、一条不多。

- [ ] **Step 2: 确认 lib.rs 的 invoke_handler 没被动过**

```bash
git diff master -- src-tauri/src/lib.rs
```

期望：**只有一行新增** `pub mod vcs;`。`invoke_handler!` 里的 44 行必须一字未动——
动了就说明改了命令名或路径，那是前后端契约的破坏性变更（`AGENTS.md` §1.2）。

- [ ] **Step 3: 确认命令签名一个都没变**

```bash
git diff master -- src-tauri/src/commands/ | grep "^[-+]pub fn"
```

期望：每个 `-pub fn xxx(...)` 都有一个**完全相同**的 `+pub fn xxx(...)` 对应。
只要有一条签名文本不一致就是违规，回去改回来。

- [ ] **Step 4: 全量跑三侧检查**

```bash
cd src-tauri
cargo fmt --check
cargo clippy --all-targets
cargo test
```

```bash
cd ..
pnpm build
pnpm test
```

前端理应零改动，`pnpm build` 与 `pnpm test` 应与重构前完全一致。
**若前端有任何改动，说明越界了**，回去核对。

测试总数对账：应等于 P6 记录的基线 + 16（本计划新增的 16 个等价性测试）。

- [ ] **Step 5: 手工 E2E**

`AGENTS.md` §5.1 要求改了 Tauri 命令就必须在真实窗口里验证。

```bash
pnpm tauri dev
```

在真实窗口里逐条走一遍（这些正是本次改到的命令）：

| 操作 | 覆盖的命令 |
|---|---|
| 打开一个仓库，图正常渲染 | `open_repo`、`load_graph` |
| 点一条提交看 diff | `read_diff` |
| 改一个文件，看它出现在 Local Changes | `read_status` |
| 暂存 → 取消暂存 → 再暂存 → 提交 | `stage`、`unstage`、`commit` |
| 丢弃一个改动 | `discard` |
| 新建分支 → 重命名 → 切过去 → 删掉 | `create_branch`、`rename_branch`、`checkout`、`delete_branch` |
| stash → 列表里看到 → pop | `stash_push`、`read_stashes`、`stash_pop` |

**用临时 fixture 仓库做，不要拿本仓库或用户的真实仓库试**（`AGENTS.md` 开篇硬约束）。
建一个：

```bash
mkdir -p /tmp/e2e-fixture && cd /tmp/e2e-fixture && git init && \
  git config user.email t@t.c && git config user.name t && \
  echo 1 > a.txt && git add . && git commit -m c1 && echo 2 > a.txt && git commit -am c2
```

- [ ] **Step 6: 补完 decisions 文件**

确保四类决策条目都有内容（`AGENTS.md` §2.1 要求缺一类即视为记录不完整）：

- **偏离计划**：至少有「trait 方法不带 path」这条（P4 已写入）；实现中若还有别的偏离，补上
- **计划的错误**：本计划里写错的代码、命令、断言。**没有就明确写「无」**，不要留空
- **计划没写的坑**：实现时才暴露的约束（clippy 告警、借用问题、测试顺序依赖等）
- **为什么这么改是安全的**：填上实际的测试对账数字

同时把进度总账表 6 行全部标为完成。

- [ ] **Step 7: 提交并推送**

```bash
git add docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md
git commit -m "docs(vcs): 补完 Phase 0 决策日志"
git push -u origin HEAD
```

- [ ] **Step 8: 开 Draft PR**

`AGENTS.md` §4.1：PR 默认 Draft。

```bash
gh pr create --draft --base master \
  --title "refactor(vcs): Phase 0 抽象层——trait Vcs 与 GitBackend" \
  --body "见 docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0-decisions.md"
```

Bash 工具没有网络，`push` 和 `gh` 要走 PowerShell。

- [ ] **Step 9: 独立 review**

`AGENTS.md` §4.1 的独立 review 门槛：合并前必须由**独立于实现者**的人或 agent 完成一次
review，实现者自查不算。用 `/code-review`，或让用户跑 Codex 二次意见。

review 时先读 decisions 的「偏离计划」条目再看 diff（rubric 第 12 条）。
发现的高危问题必须修完才能合并。

---

## Self-Review

**Spec 覆盖**（对照 spec §16 Phase 0「`trait Vcs` + `GitBackend`，现有代码搬家，行为零变化」）：

| spec 要求 | 对应 Task |
|---|---|
| `trait Vcs` 定义 | Task 1 Step 3，Task 3/4/5 逐步扩充 |
| `GitBackend` 实现 | Task 1 Step 4，Task 3/4/5 逐步扩充 |
| 现有代码搬家 | Task 2（discard）、Task 5（stash 参数拼装） |
| 行为零变化 | Task 6 Step 2/3/4 三重对账 + Step 5 手工 E2E |
| 验收：现有测试全绿 | 每个 Task 的「全量验证」步骤 |

spec §4.1 trait 里的 `capabilities`、`sync_in`、`sync_out`、`external_changes`
**有意不在 Phase 0 实现**，理由已写入 Global Constraints（dead code + clippy 告警，
且 Phase 2–4 才需要）。`branches()` / `stashes()` 两个子 trait 改为平铺方法——
两个方法的子 trait 不值得一层间接，Phase 1 也没有需要它的地方。

**占位符扫描**：无 TBD / TODO / 「类似 Task N」/ 「加上适当的错误处理」。所有代码步骤都给了完整可粘贴的代码。

**类型一致性**：
- `backend_for(&str) -> Result<Box<dyn Vcs>>` 在 Task 1、3、4、5 的命令层用法一致
- `GitBackend::new(&str)` 只在 `backend_for` 里调用
- `engine::ops::discard(&str, &[String], bool)` 在 Task 2 定义、Task 3 Step 4 调用，签名一致
- `engine::stash::{push, apply, pop, drop}` 在 Task 5 Step 3 定义、Step 5 调用，签名一致
- trait 方法名在定义处与 `git.rs` 实现处、测试处三处一致：`stage`/`unstage`/`discard`/`commit`/`create_branch`/`delete_branch`/`rename_branch`/`checkout`/`list_stashes`/`stash_push`/`stash_apply`/`stash_pop`/`stash_drop`
- `StashEntry` 的 import 在 `vcs/mod.rs`（Task 5 Step 4）与 `vcs/git.rs`（Step 5）都显式加了
