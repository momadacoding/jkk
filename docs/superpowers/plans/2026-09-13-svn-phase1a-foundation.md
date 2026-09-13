# SVN Phase 1a：只读地基 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: 用 `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 逐 Task 实施。步骤用 `- [ ]` 复选框跟踪。

**Goal:** 让客户端能识别一个 SVN 工作副本、读懂并校验它的配置、按能力隐藏 git 专属入口，并能**证明**「git 实际跟踪的文件集合 == 配置声明的集合」——到此为止，不创建任何基线、不发出任何 SVN 写操作。

**Architecture:** 沿用 Phase 0 切出的 `trait Vcs` 接缝。新增一层 `engine::svn_*` 纯读模块（CLI 封装、XML 解析、配置校验、工作副本枚举、exclude 生成、纳入集合核对），由新的 `vcs::svn::SvnGitBackend` 组装；该后端把**所有 git 语义的读写原样委托给 `GitBackend`**，自己只多做两件事：按 §6.1 参与 `backend_for` 的分流，以及通过新增的 `capabilities()` 告诉前端哪些入口不该渲染。SVN 侧本期**只读**。

**Tech Stack:** Rust / Tauri 2 / `roxmltree` 0.21（新增，已获用户确认）/ svn CLI（SlikSvn 1.14.2）/ React 18 + TypeScript

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（2026-09-13 第四轮 review 修订版，commit `150ddad`）

**决策日志:** `docs/superpowers/plans/2026-09-13-svn-phase1a-foundation-decisions.md` —— 按 `AGENTS.md` §2.1，**动第一个 Task 之前必须先建好**。

---

## Global Constraints

这些是全局约束，每个 Task 的要求都隐含包含本节。值逐字抄自 spec，不要改写。

| # | 约束 | 出处 |
|---|---|---|
| G1 | **`--ignore-externals` 按子命令区分，不能「一律带」**：`status` / `update` 必须带；`info` / `diff` **拒绝**该参数（exit 1）；`log` 无此选项 | spec §4.4 |
| G2 | **`--non-interactive` 是全局选项，所有子命令都接受**，在 `run_svn` 内部统一注入，不让调用点决定 | spec §4.4；2026-09-13 实测 6 个子命令全部接受 |
| G3 | **`--xml` 输出必须用真正的 XML parser 解析，禁止行内正则**。`<wc-status` 与 `revision="…"` 不在同一行且是 CRLF，行内正则会**静默扫出空集** | spec §4.4、§6.6.4 陷阱三 |
| G4 | **取 `<wc-status revision>`，不是 `<commit revision>`**。前者是节点基线 revision，后者是最后修改 revision，实测同一仓库里后者有 2194 个不同取值 | spec §6.6.4 陷阱二 |
| G5 | **「白名单」是两个集合**：`W-path`（配置里的路径，给 SVN 命令当 target）与 `W-file`（W-path **AND** extensions，给 git 用）。构造 git 参数用 W-file，构造 SVN target 用 W-path，**不得混用** | spec §6.3.2 |
| G6 | **空列表一律 no-op，绝不退化成「全部」**。本项目已因空 pathspec 出过一次误伤整个仓库的事故 | spec §12.2；`AGENTS.md` §6.2 |
| G7 | 所有 svn CLI 调用走 `engine::svn_cli::run_svn`（内部走 `hidden_command`），错误映射为 `crate::error::GitError`，**不向前端抛裸字符串** | `AGENTS.md` §7.3 |
| G8 | 本期**不发出任何 SVN 写操作**。`svn add` / `commit` / `update` / `revert` / `delete` 一个都不许出现在 Phase 1a 的代码里 | 本计划范围 |
| G9 | 注释用中文，解释「为什么」不是「做了什么」 | `AGENTS.md` §7.3 |
| G10 | 所有涉及 git / svn 行为的测试必须自建临时 fixture 仓库并清理，禁止依赖本仓库或任何固定路径 | `AGENTS.md` §8 |

### 不在本期范围（属 Phase 1b）

初始化向导的 P1/P2/P3/P5、`svn/base` 基线分支、失同步检测（`nodeRevs` 与 `baselineRev` 比对）、基线体检（§6.8）、重建基线、混合图。**本期只做到「能识别、能校验、能证明白名单集合正确」为止。**

P4（§6.5 / §6.5.5）**在本期**，因为它是纯只读校验，且与配置校验同属「打开仓库」这条路径。

---

## File Structure

| 文件 | 新建/修改 | 职责 |
|---|---|---|
| `src-tauri/src/engine/svn_cli.rs` | 新建 | svn 子进程的唯一入口 + 可用性探测（含 §17.1 两种缺失状态） |
| `src-tauri/src/engine/svn_xml.rs` | 新建 | `svn info/status --xml` 的解析，只暴露结构体，不暴露 XML 细节 |
| `src-tauri/src/engine/svn_config.rs` | 新建 | `.git/mygitclient.json` 的读取与 §6.2.1 校验 |
| `src-tauri/src/engine/svn_wc.rs` | 新建 | 白名单递归覆盖到的全部工作副本枚举 + P4 单仓库校验 |
| `src-tauri/src/engine/svn_exclude.rs` | 新建 | 由配置生成 `.git/info/exclude` 规则文本 |
| `src-tauri/src/engine/svn_include_check.rs` | 新建 | 纳入集合核对（§6.3.1），期望集合 vs `git ls-files` |
| `src-tauri/src/engine/mod.rs` | 修改 | 声明上面 6 个模块 |
| `src-tauri/src/vcs/mod.rs` | 修改 | 加 `Capabilities` 与 `Vcs::capabilities()`；`backend_for` 按 §6.1 分流；加 `detect_repo_kind` |
| `src-tauri/src/vcs/svn.rs` | 新建 | `SvnGitBackend`：git 语义委托 `GitBackend`，只覆写 `capabilities()` |
| `src-tauri/src/vcs/git.rs` | 修改 | 补 `capabilities()` 实现（全 true） |
| `src-tauri/src/commands/repo.rs` | 修改 | 新增 `read_capabilities`、`probe_svn_environment` 两个命令 |
| `src-tauri/src/lib.rs` | 修改 | 注册上面两个命令 |
| `src-tauri/Cargo.toml` | 修改 | 加 `roxmltree = "0.21"` |
| `src/store/repoStore.ts` | 修改 | 打开仓库时读一次 capabilities 存进 state |
| `src/components/Toolbar.tsx` 等 | 修改 | 按 capabilities 隐藏入口 |
| `src-tauri/tests/svn_cli.rs` | 新建 | Task 1 的测试 |
| `src-tauri/tests/svn_xml.rs` | 新建 | Task 2 的测试 |
| `src-tauri/tests/svn_config.rs` | 新建 | Task 3 的测试 |
| `src-tauri/tests/svn_wc.rs` | 新建 | Task 4 的测试 |
| `src-tauri/tests/svn_exclude.rs` | 新建 | Task 5 + Task 6 的测试 |
| `src-tauri/tests/svn_backend.rs` | 新建 | Task 7 的测试 |
| `src-tauri/tests/svn_fixtures.rs` | 新建 | SVN fixture 的共用构造函数（被上面几个 `mod` 进来） |

**为什么按职责拆成 6 个 engine 模块而不是一个 `svn.rs`**：这 6 件事的失败模式完全不同（进程起不来 / XML 形状变了 / 用户配置写错 / 跨仓库 / 规则生成错 / git 忽略源被覆盖），单测的 fixture 也不一样。合成一个文件会让每个测试都要搭全套环境。仓库现有的 `engine/` 就是这个粒度（`branch.rs` / `refs.rs` / `stash.rs` 各自独立）。

---

## 开工前自检（`AGENTS.md` §4.2）

- [ ] **Step 0.1: 填写自检表**

```text
- 当前分支 / worktree：
- 当前分支是否已合并进 master：
- 是否允许在当前位置修改（master 或主工作区一律不允许开发）：
- 目标 worktree（必须在改动前已创建并切入）：
- Claude Code 是否已用 EnterWorktree 切入：
- `Get-Location` / `pwd` 实测值是否就在该 worktree 内：
- 是否存在未提交改动：
```

- [ ] **Step 0.2: 用 `EnterWorktree` 切入一个新 worktree**

名称建议 `svn-phase1a`（分支名会是 `worktree-svn-phase1a`）。

⚠️ **Phase 0 记过的坑，这次同样适用**：`EnterWorktree` 从 `origin/<默认分支>` 开分支，不是本地 HEAD。切之前确认 `origin/master` 已经包含本计划与修订后的 spec（`git log --oneline -1 origin/master` 应当能看到本计划的提交）。

- [ ] **Step 0.3: 确认基点**

Run: `git log --oneline -3`
Expected: 顶端是本计划的提交，且与 `origin/master` 一致。

- [ ] **Step 0.4: 建决策日志**

创建 `docs/superpowers/plans/2026-09-13-svn-phase1a-foundation-decisions.md`，含「进度总账」表（每个 Task 一行：状态 / 产出文件 / 验证结果）与「决策条目」两节。**每完成一个 Task 立即更新总账。**

- [ ] **Step 0.5: 记录重构前测试基线**

Run: `cd src-tauri && cargo test 2>&1 | tail -40`
把 passed 数记进决策日志。Task 9 拿它对账。

⚠️ **全冷 target 上第一次 `cargo test` 可能以 rlib 链接错误失败，重跑即好**（Phase 0 实测）。看到 `crate 'windows' / 'tokio' … required to be available in rlib format` 先重跑一次再怀疑代码，按 `AGENTS.md` §5.3。

---

### Task 1: svn CLI 封装与可用性探测

**Files:**
- Create: `src-tauri/src/engine/svn_cli.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_cli.rs`

**Interfaces:**
- Consumes: `crate::engine::proc::hidden_command`、`crate::error::{GitError, Result}`
- Produces:
  - `pub fn svn_command(path: &str) -> std::process::Command`
  - `pub fn run_svn(path: &str, args: &[&str]) -> Result<String>`
  - `pub fn run_svn_code(path: &str, args: &[&str]) -> Result<(i32, String)>`
  - `pub enum SvnAvailability { Available { version: String }, StalePath, NotInstalled }`
  - `pub fn probe_svn() -> SvnAvailability`

- [ ] **Step 1.1: 写失败测试**

创建 `src-tauri/tests/svn_cli.rs`：

```rust
//! svn CLI 封装的测试。
//!
//! 这些测试需要 `svn` 在 PATH 上（spec §17）。本仓库没有 CI，所有验证都在开发机
//! 本地跑，而开发机装了 SlikSvn 1.14.2。**故意不做「没装就 skip」**——静默跳过
//! 等于把「没测」显示成绿色，正是 `AGENTS.md` §13 rubric #9 说的虚假信心。

use app_lib::engine::svn_cli::{probe_svn, run_svn, run_svn_code, SvnAvailability};

#[test]
fn probe_reports_the_installed_version() {
    match probe_svn() {
        SvnAvailability::Available { version } => {
            assert!(
                version.chars().next().is_some_and(|c| c.is_ascii_digit()),
                "版本号应当以数字开头，实际是 {version:?}"
            );
        }
        other => panic!("开发机应当装了 svn，实际探测结果是 {other:?}"),
    }
}

#[test]
fn run_svn_injects_non_interactive_for_every_subcommand() {
    // --non-interactive 是**全局选项**，6 个子命令都接受（2026-09-13 实测）。
    // 这条断言挡的是「把它错当成某个子命令专属参数」——那会让另一些子命令
    // 以 exit 1 失败，而失败点会远在仓库识别那一步。
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    // 非工作副本上跑 info：应当因为「不是工作副本」失败，
    // **而不是**因为「子命令不接受某个参数」失败。
    let err = run_svn(path, &["info"]).expect_err("非工作副本上 svn info 应当失败");
    assert!(
        !err.message.contains("doesn't accept option"),
        "失败原因不该是参数不被接受，实际：{}",
        err.message
    );
}

#[test]
fn info_must_not_receive_ignore_externals() {
    // spec §4.4 / G1：svn info 拒绝这个参数。这条测试把那个实测事实钉住，
    // 免得有人「为了统一」把它加回去——加回去会让仓库识别整条路径 exit 1。
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    let (code, _) = run_svn_code(path, &["info", "--ignore-externals"]).unwrap();
    assert_ne!(code, 0, "svn info 带 --ignore-externals 必须非零退出");
}

#[test]
fn run_svn_code_hands_the_exit_code_to_the_caller() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    // 不是工作副本 → 非零退出，但 run_svn_code 本身不算失败
    let (code, _) = run_svn_code(path, &["info"]).unwrap();
    assert_ne!(code, 0);
}
```

- [ ] **Step 1.2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_cli`
Expected: 编译失败，`unresolved import app_lib::engine::svn_cli`

- [ ] **Step 1.3: 写实现**

创建 `src-tauri/src/engine/svn_cli.rs`：

```rust
//! svn 子进程的唯一入口。形状对齐 `engine::git_cli`。

use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use std::process::Command;

/// 构造一条指向 `path` 的 svn 命令。
///
/// 走 [`hidden_command`] 的理由与 git 完全相同：装机版是 GUI 子系统，
/// 启动 console 程序会闪黑框。
///
/// `--non-interactive` 由 [`run_svn`] / [`run_svn_code`] 注入，不在这里加——
/// 这个函数也被可用性探测用到，那里跑的是 `svn --version`，不需要工作目录。
pub fn svn_command(path: &str) -> Command {
    let mut cmd = hidden_command("svn");
    cmd.current_dir(path);
    cmd
}

/// 执行一条 svn 命令，成功返回 stdout，失败把 stderr 包成 GitError。
///
/// **`--non-interactive` 在这里统一注入**，调用点不需要（也不应该）自己加。
/// 理由与 `GIT_TERMINAL_PROMPT=0` 一字不差：GUI 应用没有终端，svn 一旦等待
/// stdin 输入凭据就会永久挂死并占住 Tauri 线程池。它是 svn 的**全局选项**，
/// 所有子命令都接受（2026-09-13 实测 info/status/log/diff/update/commit 六个）。
///
/// ⚠️ **`--ignore-externals` 绝不能照此办理**。它只被 `status` / `update` 接受，
/// `info` 与 `diff` 会以 `Subcommand 'x' doesn't accept option` 退出 1。哪些调用
/// 该带由调用点决定，见 spec §4.4 的子命令表。
pub fn run_svn(path: &str, args: &[&str]) -> Result<String> {
    let mut full: Vec<&str> = vec!["--non-interactive"];
    full.extend_from_slice(args);
    let output = svn_command(path).args(&full).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("svn_cli", stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 同 [`run_svn`]，但把**退出码交给调用方判断**，非零不算失败。
///
/// 与 `git_cli::run_git_code` 同一个理由：有些地方「非零」本身就是答案，
/// 塌缩成一个 GitError 就分不出来了。
pub fn run_svn_code(path: &str, args: &[&str]) -> Result<(i32, String)> {
    let mut full: Vec<&str> = vec!["--non-interactive"];
    full.extend_from_slice(args);
    let output = svn_command(path).args(&full).output()?;
    let code = output.status.code().unwrap_or(-1);
    Ok((code, String::from_utf8_lossy(&output.stdout).to_string()))
}

/// svn 的三种可用状态。
///
/// `StalePath` 必须与 `NotInstalled` 分开，这是 spec §17.1 的要求：安装器写的是
/// **持久** PATH，而已经在跑的进程持有启动时的环境快照。用户装完 SVN 之后，
/// 一个更早启动的 Tauri 应用**仍然看不见** svn，而用户会认为「我明明装了」。
/// 两种状态的正确引导完全相反——一个是「去安装」，一个是「重启应用」。
#[derive(Debug, Clone, PartialEq)]
pub enum SvnAvailability {
    /// 当前进程能跑起 svn。`version` 形如 `1.14.2-SlikSvn`。
    Available { version: String },
    /// 当前进程环境里没有，但持久 PATH 里有 → 提示**重启应用**，别让用户重装。
    StalePath,
    /// 持久 PATH 里也没有 → 引导去安装。
    NotInstalled,
}

/// 探测 svn 是否可用。
pub fn probe_svn() -> SvnAvailability {
    // `--version --quiet` 只打一行版本号，是最便宜的存活探测（实测输出
    // `1.14.2-SlikSvn`）。用 `hidden_command` 而不是 `svn_command`：这里没有
    // 工作目录可言，设 current_dir 反而可能指向一个不存在的路径。
    let probe = hidden_command("svn")
        .args(["--version", "--quiet"])
        .output();
    if let Ok(out) = probe {
        if out.status.success() {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !version.is_empty() {
                return SvnAvailability::Available { version };
            }
        }
    }
    if persistent_path_has_svn() {
        SvnAvailability::StalePath
    } else {
        SvnAvailability::NotInstalled
    }
}

/// 持久 PATH（注册表里的 Machine + User）中是否存在一个含 `svn` 可执行文件的目录。
///
/// **走 `reg.exe` 而不是引入注册表 crate**：仓库现有的 `windows` crate 只打开了
/// `Win32_Foundation` + `Win32_System_RestartManager` 两个 feature，为读一次 PATH
/// 再打开 `Win32_System_Registry` 不划算；而「shell out 到 CLI」本来就是本项目
/// 既有的做法（git、svn 都是）。`reg query` 是只读的。
#[cfg(windows)]
fn persistent_path_has_svn() -> bool {
    const MACHINE: &str = r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const USER: &str = r"HKCU\Environment";
    [MACHINE, USER].iter().any(|key| {
        let out = hidden_command("reg")
            .args(["query", key, "/v", "Path"])
            .output();
        let Ok(out) = out else { return false };
        if !out.status.success() {
            return false;
        }
        let raw = String::from_utf8_lossy(&out.stdout);
        // reg query 的输出是 `Path    REG_EXPAND_SZ    C:\a;C:\b`，
        // 取不到值时整行就没有分号，split 之后也不会命中，安全。
        raw.split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(dir_has_svn_exe)
    })
}

/// 非 Windows 平台上没有「持久 PATH 与进程 PATH 不一致」这回事——
/// shell 的 rc 文件要重开终端才生效，用户对此有直觉，不需要客户端解释。
#[cfg(not(windows))]
fn persistent_path_has_svn() -> bool {
    false
}

#[cfg(windows)]
fn dir_has_svn_exe(dir: &str) -> bool {
    // 只看文件是否存在，不执行它。展开 %SystemRoot% 这类变量不做——
    // svn 的安装目录不会藏在变量后面，而展开会引入一整套转义问题。
    std::path::Path::new(dir).join("svn.exe").is_file()
}
```

修改 `src-tauri/src/engine/mod.rs`，在 `pub mod stash;` 之后按字母序插入：

```rust
pub mod svn_cli;
```

- [ ] **Step 1.4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_cli`
Expected: 4 passed

- [ ] **Step 1.5: 跑本侧全套检查**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`
Expected: fmt 无输出、clippy 零告警、全部 passed（= 基线 + 4）

- [ ] **Step 1.6: 提交**

```bash
git add src-tauri/src/engine/svn_cli.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_cli.rs
git commit -m "feat(svn): svn CLI 封装与可用性探测

- What: 新增 engine::svn_cli，统一注入 --non-interactive，区分「没装」与「装了但当前进程看不见」
- Why: spec §4.4 要求所有 svn 调用走同一入口；§17.1 要求两种缺失状态给出相反的引导
- Impact/Test: cargo test --test svn_cli 4 passed；fmt/clippy 干净"
```

⚠️ **不要在测试里用 Git Bash 手工复现 `reg query`**：Git Bash 会把 `/v` 当成路径改写成 `C:/Program Files/Git/v`，得到 `ERROR: Invalid syntax`（2026-09-13 实测）。Rust 的 `std::process::Command` 不经过 MSYS 转换，没有这个问题。要手工核对请走 PowerShell。

---

### Task 2: `--xml` 输出解析

**Files:**
- Create: `src-tauri/src/engine/svn_xml.rs`
- Modify: `src-tauri/src/engine/mod.rs`、`src-tauri/Cargo.toml`
- Test: `src-tauri/tests/svn_xml.rs`

**Interfaces:**
- Consumes: Task 1 的 `run_svn`（仅测试用），`roxmltree`
- Produces:
  - `pub struct SvnInfoEntry { pub path: String, pub url: String, pub repos_root: String, pub uuid: String, pub revision: i64, pub wc_root: Option<String> }`
  - `pub fn parse_info(xml: &str) -> Result<Vec<SvnInfoEntry>>`
  - `pub fn parse_node_revs(xml: &str) -> Result<std::collections::BTreeSet<i64>>`
  - `pub fn parse_external_roots(xml: &str) -> Result<Vec<String>>`

- [ ] **Step 2.1: 加依赖**

修改 `src-tauri/Cargo.toml`，在 `[dependencies]` 段 `git2 = "0.20"` 之后加一行：

```toml
# svn 的 --xml 输出必须用真 parser 解析：`<wc-status` 与 `revision="…"` 不在同一行
# 且是 CRLF，行内正则**不报错、只返回空集**，于是「nodeRevs 单值」这条不变量会以
# 一种看起来通过了的方式被绕过（spec §6.6.4 陷阱三）。
# 选 roxmltree 而非 quick-xml：我们只读不写，DOM 查询能直接表达「取 <wc-status> 的
# revision 而不是 <commit> 的」，不必手写嵌套状态机——取错那一层正是 spec §6.6.4
# 陷阱二记录的永久误报。0.21 的依赖只有 memchr（已在树里）。
roxmltree = "0.21"
```

Run: `cd src-tauri && cargo fetch`
Expected: 成功拉到 roxmltree 0.21.x

- [ ] **Step 2.2: 写失败测试**

创建 `src-tauri/tests/svn_xml.rs`：

```rust
//! svn `--xml` 解析的测试。
//!
//! 这里的输入**不是手写的理想 XML**，而是从真实 svn 输出复制来的形态：属性
//! 逐行排列、CRLF 行尾、`<wc-status>` 里还嵌着一个同名属性的 `<commit>`。
//! 用理想输入测这个模块等于没测（`AGENTS.md` §13 rubric #9）。

use app_lib::engine::svn_xml::{parse_external_roots, parse_info, parse_node_revs};

/// 真实 `svn status -v --xml` 的形态：属性跨行 + CRLF + 内层 <commit>。
const STATUS_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\".\">\r\n\
<entry\r\n   path=\"A.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"29825\"\r\n   props=\"none\">\r\n\
<commit\r\n   revision=\"29293\">\r\n\
<author>someone</author>\r\n\
</commit>\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"B.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"29825\"\r\n   props=\"none\">\r\n\
<commit\r\n   revision=\"1640\">\r\n\
<author>other</author>\r\n\
</commit>\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Vendor\">\r\n\
<wc-status\r\n   item=\"external\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
</target>\r\n\
</status>\r\n";

#[test]
fn node_revs_take_wc_status_revision_not_commit_revision() {
    // 这是整个模块最重要的一条。取错会让 nodeRevs 永久多值 → 永久误报失同步
    // （spec §6.6.4 陷阱二：实测 <commit revision> 有 2194 个不同取值）。
    let revs = parse_node_revs(STATUS_XML).unwrap();
    assert_eq!(revs.len(), 1, "应当只有一个值，实际 {revs:?}");
    assert!(revs.contains(&29825));
    assert!(!revs.contains(&29293), "29293 是 <commit revision>，不该进来");
    assert!(!revs.contains(&1640), "1640 是 <commit revision>，不该进来");
}

#[test]
fn node_revs_skip_entries_without_a_revision() {
    // external 根的 <wc-status item="external"> 没有 revision 属性。
    // 跳过它而不是当成 0——0 会凭空制造一个多值集合。
    let revs = parse_node_revs(STATUS_XML).unwrap();
    assert!(!revs.contains(&0));
}

#[test]
fn external_roots_are_listed_by_item_attribute() {
    // 枚举 external 根走 item="external"，比解析 svn:externals 属性文本可靠
    // ——后者要处理 -r peg、^/ 相对 URL、引号和多行格式（spec §6.6.4）。
    let roots = parse_external_roots(STATUS_XML).unwrap();
    assert_eq!(roots, vec!["Vendor".to_string()]);
}

#[test]
fn parse_node_revs_on_garbage_errors_instead_of_returning_empty() {
    // 空集合会让「单值不变量」以看起来通过的方式被绕过。解析不了必须报错。
    let err = parse_node_revs("this is not xml").expect_err("非 XML 必须报错");
    assert_eq!(err.code, "svn_xml");
}

/// 真实 `svn info --xml` 的形态。
const INFO_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<info>\r\n\
<entry\r\n   path=\"Code\"\r\n   revision=\"9\"\r\n   kind=\"dir\">\r\n\
<url>file:///C:/repo/Code</url>\r\n\
<relative-url>^/Code</relative-url>\r\n\
<repository>\r\n\
<root>file:///C:/repo</root>\r\n\
<uuid>0f098dbb-30ec-1142-aedc-0eea138a0008</uuid>\r\n\
</repository>\r\n\
<wc-info>\r\n\
<wcroot-abspath>C:/wc1</wcroot-abspath>\r\n\
</wc-info>\r\n\
</entry>\r\n\
</info>\r\n";

#[test]
fn info_reads_url_root_uuid_and_wcroot() {
    let entries = parse_info(INFO_XML).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.path, "Code");
    assert_eq!(e.revision, 9);
    assert_eq!(e.url, "file:///C:/repo/Code");
    assert_eq!(e.repos_root, "file:///C:/repo");
    assert_eq!(e.uuid, "0f098dbb-30ec-1142-aedc-0eea138a0008");
    assert_eq!(e.wc_root.as_deref(), Some("C:/wc1"));
}

#[test]
fn info_distinguishes_repository_root_from_working_copy_url() {
    // 这两个值在本设计里用途完全不同：断网查证用 Repository Root，
    // 基线体检用工作副本 URL（spec §6.2.1 那张表）。混用会让查证查不到
    // 只碰了别的工作副本的那次提交。
    let e = &parse_info(INFO_XML).unwrap()[0];
    assert_ne!(e.url, e.repos_root);
}
```

- [ ] **Step 2.3: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_xml`
Expected: 编译失败，`unresolved import app_lib::engine::svn_xml`

- [ ] **Step 2.4: 写实现**

创建 `src-tauri/src/engine/svn_xml.rs`：

```rust
//! `svn --xml` 输出的解析。
//!
//! **这里是全项目唯一允许接触 XML 文本的地方**，对外只暴露结构体。
//! 禁止行内正则：svn 把每个属性单独放一行（且 CRLF），`<wc-status` 与
//! `revision="…"` 不在同一行，行内正则**不报错、只返回空集**——于是
//! 「nodeRevs 单值」这条不变量会以一种看起来通过了的方式被绕过
//! （spec §4.4、§6.6.4 陷阱三）。

use crate::error::{GitError, Result};
use roxmltree::Document;
use std::collections::BTreeSet;

/// `svn info --xml` 的一条 entry。
#[derive(Debug, Clone, PartialEq)]
pub struct SvnInfoEntry {
    /// entry 的 path 属性，原样保留 svn 给的写法。
    pub path: String,
    /// 该节点自己的 URL。**不是**仓库根。
    pub url: String,
    /// `<repository><root>`。断网查证与基线 trailer 用它，见 spec §6.2.1 的用途表。
    pub repos_root: String,
    /// `<repository><uuid>`。仓库身份的唯一可靠依据——URL 会因代理 / 镜像 /
    /// http 与 https 写法不同而不可靠（spec §6.2.1）。
    pub uuid: String,
    /// entry 的 revision 属性。
    pub revision: i64,
    /// `<wc-info><wcroot-abspath>`。URL 形式的 target 没有这一节，故为 Option。
    pub wc_root: Option<String>,
}

fn parse_doc(xml: &str) -> Result<Document<'_>> {
    Document::parse(xml).map_err(|e| GitError::new("svn_xml", format!("svn 输出不是合法 XML: {e}")))
}

/// 解析 `svn info --xml`。
pub fn parse_info(xml: &str) -> Result<Vec<SvnInfoEntry>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        // 缺字段一律报错而不是填默认值：这些字段是仓库身份判定的输入，
        // 一个空 uuid 会让 P4 的「所有 UUID 必须相同」在两个空串上通过。
        let path = attr(&entry, "path")?;
        let revision = attr(&entry, "revision")?
            .parse::<i64>()
            .map_err(|_| GitError::new("svn_xml", "entry 的 revision 不是整数"))?;
        let url = child_text(&entry, "url")?;
        let repository = entry
            .children()
            .find(|n| n.has_tag_name("repository"))
            .ok_or_else(|| GitError::new("svn_xml", "svn info 输出缺 <repository>"))?;
        let repos_root = child_text(&repository, "root")?;
        let uuid = child_text(&repository, "uuid")?;
        let wc_root = entry
            .children()
            .find(|n| n.has_tag_name("wc-info"))
            .and_then(|w| w.children().find(|n| n.has_tag_name("wcroot-abspath")))
            .and_then(|n| n.text())
            .map(str::to_string);
        out.push(SvnInfoEntry {
            path,
            url,
            repos_root,
            uuid,
            revision,
            wc_root,
        });
    }
    Ok(out)
}

/// 从 `svn status -v --xml` 里取每个已版本控制节点的 revision 集合。
///
/// **取 `<wc-status revision>`，不是 `<commit revision>`。** 前者是该节点在
/// 工作副本里处于哪个 revision，后者是它最后被改动是哪个 revision；实测同一个
/// 工作副本里前者 1 个取值、后者 2194 个取值，取错就是永久误报（spec §6.6.4）。
///
/// 用 `descendants()` 按 tag 名直接筛 `wc-status`，天然不会碰到内层的 `<commit>`
/// ——这正是选 DOM 而不是流式 parser 的理由。
pub fn parse_node_revs(xml: &str) -> Result<BTreeSet<i64>> {
    let doc = parse_doc(xml)?;
    let mut revs = BTreeSet::new();
    for st in doc.descendants().filter(|n| n.has_tag_name("wc-status")) {
        // external 根的 wc-status 没有 revision 属性。跳过而不是记成 0——
        // 0 会凭空造出一个多值集合，把正常仓库判成 mixed-revision。
        let Some(raw) = st.attribute("revision") else {
            continue;
        };
        let rev = raw
            .parse::<i64>()
            .map_err(|_| GitError::new("svn_xml", format!("wc-status revision 不是整数: {raw}")))?;
        revs.insert(rev);
    }
    Ok(revs)
}

/// 从 `svn status --xml`（**不带** `--ignore-externals`）里列出 external 根。
///
/// 走 `item="external"` 而不是解析 `svn:externals` 属性文本：后者要处理 `-r` peg、
/// `^/` 相对 URL、引号和多行格式，实测前者能干净列出（spec §6.6.4）。
pub fn parse_external_roots(xml: &str) -> Result<Vec<String>> {
    let doc = parse_doc(xml)?;
    let mut roots = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let is_external = entry
            .children()
            .find(|n| n.has_tag_name("wc-status"))
            .and_then(|n| n.attribute("item"))
            == Some("external");
        if is_external {
            roots.push(attr(&entry, "path")?);
        }
    }
    Ok(roots)
}

fn attr(node: &roxmltree::Node, name: &str) -> Result<String> {
    node.attribute(name)
        .map(str::to_string)
        .ok_or_else(|| GitError::new("svn_xml", format!("svn 输出的节点缺 {name} 属性")))
}

fn child_text(node: &roxmltree::Node, tag: &str) -> Result<String> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(str::to_string)
        .ok_or_else(|| GitError::new("svn_xml", format!("svn 输出缺 <{tag}>")))
}
```

在 `src-tauri/src/engine/mod.rs` 里按字母序加：

```rust
pub mod svn_xml;
```

- [ ] **Step 2.5: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_xml`
Expected: 6 passed

- [ ] **Step 2.6: 用真实 svn 输出交叉验证**

这一步**不写进测试**，是一次人工核对，结果记进决策日志。在临时 fixture 上跑：

```bash
svn status -v --xml --ignore-externals -- <白名单目录> > /tmp/real.xml
```

确认真实输出的结构与 `STATUS_XML` 常量一致（属性跨行、`<commit>` 嵌在 `<wc-status>` 里）。**结构对不上就改测试常量，不要改实现去迁就一个假输入**——`AGENTS.md` §14.1 记过：自造 fixture 不会包含设计者没想到的东西。

- [ ] **Step 2.7: 跑本侧全套检查并提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/engine/svn_xml.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_xml.rs
git commit -m "feat(svn): --xml 输出解析，取 wc-status 的 revision

- What: 新增 engine::svn_xml（parse_info / parse_node_revs / parse_external_roots），引入 roxmltree 0.21
- Why: spec §4.4 禁止行内正则——属性跨行会让正则静默扫出空集；§6.6.4 要求取 <wc-status revision> 而非 <commit revision>
- Impact/Test: cargo test --test svn_xml 6 passed，输入用的是真实 svn 输出形态（跨行 + CRLF + 内嵌 commit）"
```

---

### Task 3: 配置文件读取与校验

**Files:**
- Create: `src-tauri/src/engine/svn_config.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_config.rs`

**Interfaces:**
- Consumes: `serde`、`serde_json`（均已在依赖里）
- Produces:
  - `pub struct SvnConfig { pub version: u32, pub mode: String, pub svn_url: String, pub whitelist: Vec<String>, pub extensions: Vec<String>, pub baseline_branch: String }`
  - `pub fn config_path(repo_root: &str) -> std::path::PathBuf`
  - `pub fn read_config(repo_root: &str) -> Result<SvnConfig>`
  - `pub fn validate(cfg: &SvnConfig) -> Result<()>`
  - `pub fn normalize_whitelist_entry(raw: &str) -> Result<String>`

- [ ] **Step 3.1: 写失败测试**

创建 `src-tauri/tests/svn_config.rs`：

```rust
//! 配置校验的测试。
//!
//! 这个文件**用户手写**，所以它和输入框里敲的字符串是同一个可信级别，
//! 每个字段都会被送进 git 或 svn 的命令行（spec §6.2.1）。

use app_lib::engine::svn_config::{normalize_whitelist_entry, validate, SvnConfig};

fn ok_config() -> SvnConfig {
    SvnConfig {
        version: 1,
        mode: "svn-git".to_string(),
        svn_url: "https://svn.example.com/game/trunk".to_string(),
        whitelist: vec!["Assets/Scripts/".to_string()],
        extensions: vec![".cs".to_string()],
        baseline_branch: "svn/base".to_string(),
    }
}

#[test]
fn a_well_formed_config_passes() {
    validate(&ok_config()).unwrap();
}

#[test]
fn baseline_branch_must_be_exactly_svn_base() {
    // 全文所有流程都硬编码了这个名字，留一个可配字段只会让两者不一致（spec §6.2.1）。
    let mut cfg = ok_config();
    cfg.baseline_branch = "svn/mybase".to_string();
    let err = validate(&cfg).expect_err("非 svn/base 必须被拒");
    assert_eq!(err.code, "svn_config");
    assert!(err.message.contains("baselineBranch"));
}

#[test]
fn both_lists_empty_is_rejected() {
    // 空白名单会让空参数保护在每一步触发，等于功能不可用（spec §6.2.1）。
    let mut cfg = ok_config();
    cfg.whitelist.clear();
    assert!(validate(&cfg).is_err());

    let mut cfg = ok_config();
    cfg.extensions.clear();
    assert!(validate(&cfg).is_err());
}

#[test]
fn dangerous_whitelist_entries_are_rejected() {
    // 白名单条目同时被当 git pathspec 和 SVN target 用，两套语义各有魔法字符。
    // 本项目已因空 pathspec 误伤过整个仓库（AGENTS.md §6.2）。
    for bad in [
        "",                     // 空串
        "/",                    // 根
        ".",                    // 当前目录
        "..",                   // 上跳
        "Assets/../../etc",     // 中间段上跳
        "C:/Windows",           // 盘符
        "//server/share",       // UNC
        ":(exclude)Assets",     // git pathspec magic
        ".git/hooks",           // 以 .git 开头
        ".svn/wc.db",           // 以 .svn 开头
    ] {
        assert!(
            normalize_whitelist_entry(bad).is_err(),
            "{bad:?} 应当被拒绝"
        );
    }
}

#[test]
fn windows_backslashes_are_normalized_to_forward_slashes() {
    // 跨平台路径是本项目的高发缺陷区（AGENTS.md §8）。
    assert_eq!(
        normalize_whitelist_entry(r"Assets\Scripts\").unwrap(),
        "Assets/Scripts/"
    );
    assert_eq!(
        normalize_whitelist_entry(r"Assets\Scripts\Player.cs").unwrap(),
        "Assets/Scripts/Player.cs"
    );
}

#[test]
fn whitelist_entries_inside_externals_are_allowed() {
    // §6.5 P4 改写之后，白名单**可以**落在 external 内——那正是目标场景
    // （真实 Unity 工程的代码就在 external 里，spec §1.1.1）。
    // 跨仓库的拦截在 Task 4 的 P4，不在这里按路径文本猜。
    assert_eq!(normalize_whitelist_entry("Code/").unwrap(), "Code/");
}

#[test]
fn extensions_must_start_with_a_dot_and_carry_no_separators() {
    let mut cfg = ok_config();
    cfg.extensions = vec!["cs".to_string()];
    assert!(validate(&cfg).is_err(), "缺前导点必须被拒");

    let mut cfg = ok_config();
    cfg.extensions = vec![".a/b".to_string()];
    assert!(validate(&cfg).is_err(), "含路径分隔符必须被拒");

    // "*" 是 spec §6.3 明文给出的全纳入写法，必须放行
    let mut cfg = ok_config();
    cfg.extensions = vec!["*".to_string()];
    validate(&cfg).unwrap();
}

#[test]
fn svn_url_must_not_be_empty() {
    let mut cfg = ok_config();
    cfg.svn_url.clear();
    assert!(validate(&cfg).is_err());
}
```

- [ ] **Step 3.2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_config`
Expected: 编译失败

- [ ] **Step 3.3: 写实现**

创建 `src-tauri/src/engine/svn_config.rs`：

```rust
//! `.git/mygitclient.json` 的读取与校验。
//!
//! 形状对齐 `engine::custom_config`：用户手写，客户端只读。
//!
//! **这个文件是不可信输入。** 每个字段都会被送进 git 或 svn 的命令行，不校验
//! 就等于把命令构造交给了手滑。校验**集中在加载时一次做完**，不要散落到各个
//! 调用点（spec §6.2.1）。

use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 基线分支名是**固定值**，不是可配项。
///
/// 全文所有流程都硬编码了它；字段保留只为向前兼容，值不是它就阻断（spec §6.2.1）。
pub const BASELINE_BRANCH: &str = "svn/base";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub svn_url: String,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_baseline_branch")]
    pub baseline_branch: String,
}

fn default_version() -> u32 {
    1
}

fn default_baseline_branch() -> String {
    BASELINE_BRANCH.to_string()
}

/// 配置文件的位置：`<仓库根>/.git/mygitclient.json`。
///
/// 放在 `.git/` 内部而不是工作目录，是为了让工作目录**只多出 `.git` 一个条目**
/// （spec §3）——多一个文件就多一个被 TortoiseSVN 误提交上去的东西。
pub fn config_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join("mygitclient.json")
}

/// 读取并校验配置。文件不存在时返回 `not_configured`，调用方据此区分
/// 「未初始化的 SVN 工作副本」与「配置坏了」（spec §6.1）。
pub fn read_config(repo_root: &str) -> Result<SvnConfig> {
    let path = config_path(repo_root);
    if !path.is_file() {
        return Err(GitError::new(
            "not_configured",
            format!("{} 不存在", path.display()),
        ));
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut cfg: SvnConfig = serde_json::from_str(&raw)
        .map_err(|e| GitError::new("svn_config", format!("配置文件不是合法 JSON: {e}")))?;
    // 规范化放在校验之前：后续所有比较都基于正斜杠形式，留两种写法会让
    // 「白名单里有没有这一条」在 Windows 上时对时错。
    let mut normalized = Vec::with_capacity(cfg.whitelist.len());
    for entry in &cfg.whitelist {
        normalized.push(normalize_whitelist_entry(entry)?);
    }
    cfg.whitelist = normalized;
    validate(&cfg)?;
    Ok(cfg)
}

/// 校验一份**已规范化**的配置。
pub fn validate(cfg: &SvnConfig) -> Result<()> {
    if cfg.svn_url.trim().is_empty() {
        return Err(GitError::new("svn_config", "svnUrl 不能为空"));
    }
    if cfg.baseline_branch != BASELINE_BRANCH {
        return Err(GitError::new(
            "svn_config",
            format!(
                "baselineBranch 必须是 {BASELINE_BRANCH}，实际是 {}。\
                 全文流程都硬编码了这个名字，改它会让基线与流程不一致",
                cfg.baseline_branch
            ),
        ));
    }
    // 两者皆空要阻断，但**任一为空**同样不可用：空白名单会让每一步的空参数保护
    // 触发，空扩展名会让 W-file 恒为空集（spec §6.2.1、§6.3.2）。
    if cfg.whitelist.is_empty() {
        return Err(GitError::new("svn_config", "whitelist 不能为空"));
    }
    if cfg.extensions.is_empty() {
        return Err(GitError::new("svn_config", "extensions 不能为空"));
    }
    for e in &cfg.extensions {
        // "*" 是 spec §6.3 明文给出的全纳入写法，单独放行。
        if e == "*" {
            continue;
        }
        if !e.starts_with('.') {
            return Err(GitError::new(
                "svn_config",
                format!("扩展名必须以 . 开头：{e}"),
            ));
        }
        if e.contains('/') || e.contains('\\') || e.contains('*') || e.contains('?') {
            return Err(GitError::new(
                "svn_config",
                format!("扩展名不能含路径分隔符或通配符：{e}"),
            ));
        }
    }
    for w in &cfg.whitelist {
        // 规范化过的条目再过一遍，挡住「直接构造 SvnConfig 绕开 read_config」的调用方
        normalize_whitelist_entry(w)?;
    }
    Ok(())
}

/// 把一条白名单路径规范化成「仓库内相对路径 + 正斜杠」，并挡掉危险形态。
///
/// 白名单条目同时被当成 **git pathspec** 和 **SVN target**，两套语义各有魔法字符。
/// 一个 `:(exclude)` 或一个 `..` 就能让命令作用到预期之外的范围（spec §6.2.1）。
pub fn normalize_whitelist_entry(raw: &str) -> Result<String> {
    let reject = |why: &str| Err(GitError::new("svn_config", format!("白名单条目 {raw:?}：{why}")));

    let s = raw.replace('\\', "/");
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return reject("不能为空");
    }
    // pathspec magic 必须在最前面挡掉：`:(exclude)Assets` 会让 git 反选。
    if trimmed.starts_with(':') {
        return reject("不能以 : 开头（git pathspec magic）");
    }
    if trimmed.starts_with('/') {
        return reject("必须是仓库内相对路径，不能以 / 开头");
    }
    // 盘符：`C:/x`。上面已挡了以 : 开头，这里挡中间的。
    if trimmed.chars().nth(1) == Some(':') {
        return reject("不能含盘符");
    }
    if trimmed == "." || trimmed == ".." {
        return reject("不能是 . 或 ..");
    }
    // 逐段检查，挡住 `Assets/../../etc` 这种中间段上跳。
    for seg in trimmed.split('/') {
        if seg == ".." {
            return reject("不能含 .. 段");
        }
    }
    // `.git` / `.svn` 开头：前者会让客户端把自己的元数据交出去，后者会让
    // SVN 的工作副本数据库进 git。注意这**只挡条目本身**，深层的 `.svn`
    // 由 §6.3 生成的 exclude 末尾那条兜底规则负责（见 svn_exclude）。
    let first = trimmed.split('/').next().unwrap_or("");
    if first == ".git" || first == ".svn" {
        return reject("不能以 .git 或 .svn 开头");
    }
    Ok(trimmed.to_string())
}
```

在 `src-tauri/src/engine/mod.rs` 里加 `pub mod svn_config;`。

- [ ] **Step 3.4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_config`
Expected: 8 passed

- [ ] **Step 3.5: 全套检查并提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_config.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_config.rs
git commit -m "feat(svn): 配置文件读取与校验

- What: 新增 engine::svn_config，校验集中在加载时一次做完
- Why: spec §6.2.1——该文件用户手写，每个字段都会进命令行；baselineBranch 固定为 svn/base
- Impact/Test: cargo test --test svn_config 8 passed，覆盖 pathspec magic、盘符、UNC、.. 段、Windows 反斜杠"
```

---

### Task 4: 嵌套工作副本枚举与 P4

**Files:**
- Create: `src-tauri/src/engine/svn_wc.rs`、`src-tauri/tests/svn_fixtures.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_wc.rs`

**Interfaces:**
- Consumes: Task 1 的 `run_svn`、Task 2 的 `parse_info` / `parse_external_roots`
- Produces:
  - `pub struct WcIdentity { pub wc_root: String, pub url: String, pub repos_root: String, pub uuid: String }`
  - `pub fn enumerate_wcs(repo_root: &str, whitelist: &[String]) -> Result<Vec<WcIdentity>>`
  - `pub fn check_single_repository(wcs: &[WcIdentity]) -> Result<()>`
- `src-tauri/tests/svn_fixtures.rs` 导出 `pub fn svn_repo() -> SvnFixture`，被 Task 4 / 7 的测试 `#[path]` 引入

- [ ] **Step 4.1: 写 fixture helper**

创建 `src-tauri/tests/svn_fixtures.rs`：

```rust
//! SVN fixture 的共用构造。
//!
//! 全部在 `TempDir` 里自建 `svnadmin create` 仓库，跑完即删——禁止依赖本仓库、
//! 用户的仓库或任何固定路径（`AGENTS.md` §8）。

#![allow(dead_code)] // 不同测试文件用到的子集不同

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct SvnFixture {
    pub dir: TempDir,
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let status = Command::new(program)
        .current_dir(dir)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("{program} 起不来：{e}（spec §17：svn 必须在 PATH 上）"));
    assert!(status.success(), "{program} {args:?} 失败");
}

/// Windows 上 `file://` URL 是 `file:///C:/path`（三斜杠 + 盘符 + 正斜杠）。
/// 这是已知跨平台坑，不能靠路径拼接碰运气（spec §14.1）。
pub fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

impl SvnFixture {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
    pub fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }
    pub fn wc(&self) -> PathBuf {
        self.dir.path().join("wc1")
    }
    pub fn wc_str(&self) -> String {
        self.wc().to_string_lossy().to_string()
    }
}

/// 一个仓库 + 一个 checkout，`Code/` 下有两个 .cs。
pub fn svn_repo() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(&root, "svn", &["checkout", &url, "wc1", "--non-interactive"]);
    let wc = root.join("wc1");
    std::fs::create_dir_all(wc.join("Code")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a\n").unwrap();
    std::fs::write(wc.join("Code/B.cs"), "b\n").unwrap();
    run(&wc, "svn", &["add", "Code", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 在 `svn_repo()` 的基础上，往 `Code/` 里挂一条指向**另一个仓库**的 external。
///
/// 这是 §6.5.5 的场景：只查白名单根会拿到 repo1 的 UUID，P4 干净通过，
/// 而一个另一个仓库的子树已经进了白名单。
pub fn svn_repo_with_cross_repo_nested_external() -> SvnFixture {
    let fx = svn_repo();
    let root = fx.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo2"]);
    let url2 = file_url(&root.join("repo2"));
    run(&root, "svn", &["checkout", &url2, "wcv", "--non-interactive"]);
    let wcv = root.join("wcv");
    std::fs::write(wcv.join("V.cs"), "v\n").unwrap();
    run(&wcv, "svn", &["add", "V.cs", "--non-interactive"]);
    run(&wcv, "svn", &["commit", "-m", "vendor r1", "--non-interactive"]);

    let wc = fx.wc();
    run(
        &wc,
        "svn",
        &[
            "propset",
            "svn:externals",
            &format!("{url2} Vendor"),
            "Code",
            "--non-interactive",
        ],
    );
    run(
        &wc,
        "svn",
        &["commit", "-m", "nested external", "--non-interactive"],
    );
    run(&wc, "svn", &["update", "--non-interactive"]);
    fx
}
```

- [ ] **Step 4.2: 写失败测试**

创建 `src-tauri/tests/svn_wc.rs`：

```rust
//! 白名单覆盖到的工作副本枚举与 P4。

#[path = "svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_wc::{check_single_repository, enumerate_wcs};
use svn_fixtures::{svn_repo, svn_repo_with_cross_repo_nested_external};

#[test]
fn a_plain_working_copy_yields_exactly_one_wc() {
    let fx = svn_repo();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert_eq!(wcs.len(), 1);
    check_single_repository(&wcs).unwrap();
}

#[test]
fn a_nested_cross_repo_external_is_enumerated_and_rejected() {
    // spec §6.5.5：这正是 P4 初版漏掉的形态。
    let fx = svn_repo_with_cross_repo_nested_external();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert_eq!(wcs.len(), 2, "应当枚举到 Code 与 Code/Vendor 两个 WC");
    let err = check_single_repository(&wcs).expect_err("跨仓库必须被 P4 拒绝");
    assert_eq!(err.code, "svn_cross_repo");
    // 错误信息必须能指出是哪一条，否则用户无从下手
    assert!(err.message.contains("Vendor"), "实际：{}", err.message);
}

#[test]
fn checking_only_the_whitelist_root_would_have_let_it_through() {
    // **反向证据**：证明 §6.5.5 的递归枚举不是过度设计。
    // 只查白名单根拿到的是 repo1 的 UUID，单值，P4 会干净通过。
    let fx = svn_repo_with_cross_repo_nested_external();
    let all = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    let roots_only: Vec<_> = all.iter().take(1).cloned().collect();
    check_single_repository(&roots_only)
        .expect("只查根的旧写法会放行——这正是要修的缺陷");
}

#[test]
fn an_empty_whitelist_enumerates_nothing_and_never_runs_svn() {
    // G6：空列表是 no-op，不能退化成「整个工作副本」。
    let fx = svn_repo();
    let wcs = enumerate_wcs(&fx.wc_str(), &[]).unwrap();
    assert!(wcs.is_empty());
}
```

- [ ] **Step 4.3: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_wc`
Expected: 编译失败

- [ ] **Step 4.4: 写实现**

创建 `src-tauri/src/engine/svn_wc.rs`：

```rust
//! 白名单**递归覆盖到的全部**工作副本的枚举与 P4 校验。
//!
//! 为什么不是「对每个白名单根跑一次 svn info」：白名单根底下还可以再挂
//! external，那时只查根会拿到根所属仓库的 UUID，P4 干净通过，而一个**另一个
//! 仓库**的子树已经进了白名单——它会被 git 跟踪、进基线、进交付清单，却从不
//! 参与 nodeRevs 校验，最后在 D5 撞上 E200007（spec §6.5.5）。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::{parse_external_roots, parse_info};
use crate::error::{GitError, Result};

/// 一个工作副本的身份三元组。
#[derive(Debug, Clone, PartialEq)]
pub struct WcIdentity {
    /// 相对仓库根的路径，正斜杠。用它给用户报位置。
    pub wc_root: String,
    pub url: String,
    pub repos_root: String,
    /// 仓库身份的唯一可靠依据（spec §6.2.1）。
    pub uuid: String,
}

/// 枚举白名单递归覆盖到的全部工作副本。
///
/// 空白名单返回空列表，**一条 svn 命令都不发**（G6）。
pub fn enumerate_wcs(repo_root: &str, whitelist: &[String]) -> Result<Vec<WcIdentity>> {
    let mut out: Vec<WcIdentity> = Vec::new();
    let mut pending: Vec<String> = whitelist
        .iter()
        .map(|w| w.trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .collect();

    while let Some(target) = pending.pop() {
        // 已经收过的子树不重复展开：两条白名单可能落在同一个 WC 里。
        if out.iter().any(|w| w.wc_root == target) {
            continue;
        }
        // ⚠️ svn info **不接受** --ignore-externals（G1）。它默认也不遍历
        // externals，所以这里本来就不需要那个参数。
        let xml = run_svn(repo_root, &["info", "--xml", "--", &target])?;
        let entries = parse_info(&xml)?;
        let entry = entries
            .into_iter()
            .next()
            .ok_or_else(|| GitError::new("svn_wc", format!("svn info 对 {target} 没有输出")))?;
        out.push(WcIdentity {
            wc_root: target.clone(),
            url: entry.url,
            repos_root: entry.repos_root,
            uuid: entry.uuid,
        });

        // 找这棵子树里的嵌套 external 根。**这一次故意不带 --ignore-externals**
        // ——要的就是让它们出现（item="external"），见 spec §6.6.4 的枚举方式。
        let st = run_svn(repo_root, &["status", "--xml", "--", &target])?;
        for rel in parse_external_roots(&st)? {
            let rel = rel.replace('\\', "/");
            if !out.iter().any(|w| w.wc_root == rel) {
                pending.push(rel);
            }
        }
    }
    // 顺序稳定，便于测试与报错信息可复现
    out.sort_by(|a, b| a.wc_root.cmp(&b.wc_root));
    Ok(out)
}

/// P4：所有工作副本必须属于**同一个仓库**。
///
/// 理由不是洁癖：`svn commit` 对跨仓库的 target 列表**动手之前就拒绝**
/// （E200007，spec §6.6.5 E5），交付永远做不成。前置拦住是为了不让用户
/// 白走完整个确认流程才在 D5 撞墙。
pub fn check_single_repository(wcs: &[WcIdentity]) -> Result<()> {
    let Some(first) = wcs.first() else {
        return Ok(());
    };
    if let Some(bad) = wcs.iter().find(|w| w.uuid != first.uuid) {
        return Err(GitError::new(
            "svn_cross_repo",
            format!(
                "白名单跨了两个仓库：{} 属于 {}，而 {} 属于 {}。\
                 svn commit 对跨仓库的 target 列表会直接拒绝（E200007），交付做不成。",
                first.wc_root, first.uuid, bad.wc_root, bad.uuid
            ),
        ));
    }
    Ok(())
}
```

在 `src-tauri/src/engine/mod.rs` 里加 `pub mod svn_wc;`。

- [ ] **Step 4.5: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_wc`
Expected: 4 passed

- [ ] **Step 4.6: 全套检查并提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_wc.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_wc.rs src-tauri/tests/svn_fixtures.rs
git commit -m "feat(svn): 嵌套工作副本枚举与 P4 单仓库校验

- What: 新增 engine::svn_wc，递归枚举白名单覆盖到的全部 WC，跨仓库时以 svn_cross_repo 阻断
- Why: spec §6.5.5——只查白名单根会放行 Code/Vendor 这种跨仓库嵌套 external
- Impact/Test: cargo test --test svn_wc 4 passed，含「只查根的旧写法会放行」这条反向证据"
```

---

### Task 5: exclude 规则生成

**Files:**
- Create: `src-tauri/src/engine/svn_exclude.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_exclude.rs`

**Interfaces:**
- Consumes: Task 3 的 `SvnConfig`
- Produces:
  - `pub fn generate_exclude(whitelist: &[String], extensions: &[String]) -> String`
  - `pub fn write_exclude(repo_root: &str, content: &str) -> Result<()>`

- [ ] **Step 5.1: 写失败测试**

创建 `src-tauri/tests/svn_exclude.rs`：

```rust
//! exclude 规则生成的测试。
//!
//! **断言对象是 git 的实际行为（`git ls-files`），不是生成出来的规则文本**
//! ——测规则文本等于没测（spec §6.3）。

use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success();
    assert!(ok, "git {args:?} 失败");
}

fn tracked(dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["ls-files"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// 一个模拟 Unity 布局的目录：白名单目录里有多层嵌套、干扰文件，
/// 还有一个**真实存在的 `.svn` 目录**（白名单落在 external 根时就是这样）。
fn workdir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    run_git(p, &["init"]);
    run_git(p, &["config", "user.email", "t@t"]);
    run_git(p, &["config", "user.name", "T"]);
    for sub in [
        "Code/Sub/Deep",
        "Code/.svn/pristine/aa",
        "Art",
    ] {
        fs::create_dir_all(p.join(sub)).unwrap();
    }
    fs::write(p.join("Code/Top.cs"), "x").unwrap();
    fs::write(p.join("Code/Sub/Mid.cs"), "x").unwrap();
    fs::write(p.join("Code/Sub/Deep/Low.cs"), "x").unwrap();
    fs::write(p.join("Code/Junk.psd"), "x").unwrap();
    fs::write(p.join("Code/.svn/wc.db"), "x").unwrap();
    fs::write(p.join("Code/.svn/pristine/aa/d.svn-base"), "x").unwrap();
    fs::write(p.join("Art/boss.fbx"), "x").unwrap();
    dir
}

fn apply(dir: &Path, whitelist: &[&str], extensions: &[&str]) -> Vec<String> {
    let w: Vec<String> = whitelist.iter().map(|s| s.to_string()).collect();
    let e: Vec<String> = extensions.iter().map(|s| s.to_string()).collect();
    write_exclude(dir.to_str().unwrap(), &generate_exclude(&w, &e)).unwrap();
    run_git(dir, &["add", "-A"]);
    tracked(dir)
}

#[test]
fn nested_source_files_at_every_depth_are_tracked() {
    // spec §6.3：缺 `!/Code/**/` 那一行时，深层文件**静默丢失**——用户以为
    // 文件受 git 保护，实际没有。Unity 的 Scripts 底下必然有多层子目录。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs"]);
    assert!(files.contains(&"Code/Top.cs".to_string()));
    assert!(files.contains(&"Code/Sub/Mid.cs".to_string()));
    assert!(files.contains(&"Code/Sub/Deep/Low.cs".to_string()));
}

#[test]
fn files_outside_the_whitelist_are_pruned() {
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs"]);
    assert!(!files.iter().any(|f| f.starts_with("Art/")));
}

#[test]
fn extension_filter_excludes_other_files_inside_the_whitelist() {
    // W-file ⊊ W-path（spec §6.3.2）：.psd 在白名单目录里，但不归 git 管。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs"]);
    assert!(!files.contains(&"Code/Junk.psd".to_string()));
}

#[test]
fn svn_metadata_never_enters_git_even_with_a_matching_extension() {
    // spec §6.3：白名单落在 external 根时里面有真实的 .svn。一旦 wc.db 被 git
    // 跟踪，任何一次 git checkout 都会在一个**正在使用的**工作副本底下换掉它。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs", ".db"]);
    assert!(
        !files.iter().any(|f| f.contains(".svn/")),
        "实际跟踪了：{files:?}"
    );
}

#[test]
fn svn_metadata_never_enters_git_with_the_wildcard_extension() {
    // "*" 是 spec §6.3 明文推荐的全纳入写法，这条路径下 .svn 更容易漏进来。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &["*"]);
    assert!(
        !files.iter().any(|f| f.contains(".svn/")),
        "实际跟踪了：{files:?}"
    );
    // 但普通代码文件仍要在
    assert!(files.contains(&"Code/Sub/Deep/Low.cs".to_string()));
}

#[test]
fn a_single_file_whitelist_entry_tracks_only_that_file() {
    let dir = workdir();
    let files = apply(dir.path(), &["Code/Top.cs"], &[".cs"]);
    assert_eq!(files, vec!["Code/Top.cs".to_string()]);
}

#[test]
fn windows_backslash_input_produces_the_same_result() {
    // 跨平台路径是本项目高发缺陷区（AGENTS.md §8）。
    let dir = workdir();
    let files = apply(dir.path(), &[r"Code\Sub\"], &[".cs"]);
    assert!(files.contains(&"Code/Sub/Mid.cs".to_string()));
    assert!(files.contains(&"Code/Sub/Deep/Low.cs".to_string()));
    assert!(!files.contains(&"Code/Top.cs".to_string()));
}
```

- [ ] **Step 5.2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_exclude`
Expected: 编译失败

- [ ] **Step 5.3: 写实现**

创建 `src-tauri/src/engine/svn_exclude.rs`：

```rust
//! 由配置生成 `.git/info/exclude`。
//!
//! **由代码生成、绝不让人手写**：这套「先全排除、再逐层反选」的写法极易写错，
//! 而写错的表现是**用户以为文件受 git 保护、实际上没有**——切分支丢改动、
//! stash 存不进去、交付清单里不出现（spec §6.3）。

use crate::error::Result;
use std::io::Write;

/// 生成规则文本。
///
/// 输出形如（白名单 `Assets/Scripts/`，扩展名 `.cs` / `.meta`）：
///
/// ```gitignore
/// /*
/// !/Assets/
/// /Assets/*
/// !/Assets/Scripts/
/// /Assets/Scripts/**
/// !/Assets/Scripts/**/
/// !/Assets/Scripts/**/*.cs
/// !/Assets/Scripts/**/*.meta
/// .svn/
/// ```
pub fn generate_exclude(whitelist: &[String], extensions: &[String]) -> String {
    let mut lines: Vec<String> = vec![
        "# 由 mygitclient 自动生成，请勿手工编辑——改动会在下次打开仓库时被覆盖。".to_string(),
        "/*".to_string(),
    ];

    for raw in whitelist {
        let entry = raw.replace('\\', "/");
        let is_dir = entry.ends_with('/');
        let clean = entry.trim_end_matches('/');
        if clean.is_empty() {
            continue;
        }
        let segs: Vec<&str> = clean.split('/').collect();

        // 逐层打开父目录。每一层都要「先反选目录本身、再排除其内容」，
        // 否则下一层的反选规则够不着——git 的硬规则是**父目录被排除后，
        // 其中的文件无法再被重新纳入**（spec §6.3）。
        let mut prefix = String::new();
        for (i, seg) in segs.iter().enumerate() {
            prefix.push('/');
            prefix.push_str(seg);
            let last = i + 1 == segs.len();
            if last && !is_dir {
                // 单文件条目：直接反选它本身，不展开通配
                lines.push(format!("!{prefix}"));
                break;
            }
            lines.push(format!("!{prefix}/"));
            if last {
                lines.push(format!("{prefix}/**"));
                // ⚠️ 这一行不能省：`/x/**` 会把中间**目录**也排除掉，git 就不再
                // 往里递归，后面按扩展名的反选对深层文件永远不会生效。
                // 2026-09-12 实测：缺它时 Sub/Mid.cs 与 Sub/Deep/Low.cs 静默丢失。
                lines.push(format!("!{prefix}/**/"));
                for ext in extensions {
                    if ext == "*" {
                        lines.push(format!("!{prefix}/**/*"));
                    } else {
                        lines.push(format!("!{prefix}/**/*{ext}"));
                    }
                }
            } else {
                lines.push(format!("{prefix}/*"));
            }
        }
    }

    // ⚠️ **必须是最后一行。** `git check-ignore` 的裁决规则是「最后一条匹配的
    // 模式说了算」，而上面那些 `!/x/**/` 把白名单内部的**所有**子目录重新纳入了，
    // `.svn` 也在其中。白名单落在 external 根时那里有真实的 `.svn`；一旦 `wc.db`
    // 被 git 跟踪，任何一次 `git checkout` 都会在一个**正在使用的** SVN 工作副本
    // 底下把它换成另一个版本——那不是「恢复旧数据」，是直接损坏工作副本。
    // 不带前导 `/` 使它在任意深度生效（spec §6.3）。
    lines.push(".svn/".to_string());

    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// 把规则写进 `<仓库根>/.git/info/exclude`，整文件覆盖。
pub fn write_exclude(repo_root: &str, content: &str) -> Result<()> {
    let info = std::path::Path::new(repo_root).join(".git").join("info");
    std::fs::create_dir_all(&info)?;
    let mut f = std::fs::File::create(info.join("exclude"))?;
    f.write_all(content.as_bytes())?;
    Ok(())
}
```

在 `src-tauri/src/engine/mod.rs` 里加 `pub mod svn_exclude;`。

- [ ] **Step 5.4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_exclude`
Expected: 7 passed

若 `a_single_file_whitelist_entry_tracks_only_that_file` 失败，先手工核对：

```bash
cd <临时 fixture> && git check-ignore -v --non-matching Code/Top.cs
```

它会直接告诉你是第几行规则做的决定。**改实现，不要改断言去迁就**。

- [ ] **Step 5.5: 全套检查并提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_exclude.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_exclude.rs
git commit -m "feat(svn): 由配置生成 .git/info/exclude

- What: 新增 engine::svn_exclude，逐层反选 + 末尾 .svn/ 兜底行
- Why: spec §6.3——缺 !/x/**/ 会让深层源码静默丢失；缺末尾 .svn/ 会把 SVN 的 wc.db 收进 git
- Impact/Test: cargo test --test svn_exclude 7 passed，断言的是 git ls-files 的实际行为，含 extensions:[\"*\"] 与 Windows 反斜杠"
```

---

### Task 6: 纳入集合核对

**Files:**
- Create: `src-tauri/src/engine/svn_include_check.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: 追加到 `src-tauri/tests/svn_exclude.rs`

**Interfaces:**
- Consumes: Task 5 的 `generate_exclude` / `write_exclude`、`engine::git_cli::run_git`
- Produces:
  - `pub fn expected_set(repo_root: &str, whitelist: &[String], extensions: &[String]) -> Result<std::collections::BTreeSet<String>>`
  - `pub fn actual_set(repo_root: &str, whitelist: &[String]) -> Result<std::collections::BTreeSet<String>>`
  - `pub fn verify_include_set(repo_root: &str, whitelist: &[String], extensions: &[String]) -> Result<()>`

- [ ] **Step 6.1: 写失败测试**

追加到 `src-tauri/tests/svn_exclude.rs` 末尾：

```rust
use app_lib::engine::svn_include_check::verify_include_set;

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn include_check_passes_on_a_clean_working_directory() {
    let dir = workdir();
    let w = strs(&["Code/"]);
    let e = strs(&[".cs"]);
    write_exclude(dir.path().to_str().unwrap(), &generate_exclude(&w, &e)).unwrap();
    verify_include_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
}

#[test]
fn a_gitignore_in_the_working_tree_is_caught() {
    // spec §6.3.1：工作树里的 .gitignore 优先级**高于** .git/info/exclude。
    // 实测：Code/.gitignore 里一行 *.cs 会让 git ls-files 变成空。
    // 它不会让初始化失败，§6.8 的基线体检也查不出来——只有这道核对能抓到。
    let dir = workdir();
    fs::write(dir.path().join("Code/.gitignore"), "*.cs\n").unwrap();
    let w = strs(&["Code/"]);
    let e = strs(&[".cs"]);
    write_exclude(dir.path().to_str().unwrap(), &generate_exclude(&w, &e)).unwrap();

    let err = verify_include_set(dir.path().to_str().unwrap(), &w, &e)
        .expect_err("被 .gitignore 盖掉必须阻断");
    assert_eq!(err.code, "include_set_mismatch");
    // 错误信息必须指认裁决者，否则用户不知道是哪个文件干的
    assert!(
        err.message.contains(".gitignore"),
        "应当指出是 .gitignore 干的，实际：{}",
        err.message
    );
    assert!(
        err.message.contains("Code/Top.cs"),
        "应当列出差异文件，实际：{}",
        err.message
    );
}

#[test]
fn include_check_reports_unexpected_files_too() {
    // 双向比对：不只查「该在的没在」，也查「不该在的在了」。
    // 后者是 .svn 兜底行失效那一类缺陷的表现。
    let dir = workdir();
    let w = strs(&["Code/"]);
    let e = strs(&[".cs"]);
    // 故意写一份**缺少 .svn/ 兜底行**的规则
    let broken = generate_exclude(&w, &strs(&[".cs", ".db"]))
        .replace(".svn/\n", "");
    write_exclude(dir.path().to_str().unwrap(), &broken).unwrap();
    let err = verify_include_set(dir.path().to_str().unwrap(), &w, &e)
        .expect_err("多出 .svn 下的文件必须被报出来");
    assert_eq!(err.code, "include_set_mismatch");
}
```

- [ ] **Step 6.2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_exclude`
Expected: 编译失败，`unresolved import app_lib::engine::svn_include_check`

- [ ] **Step 6.3: 写实现**

创建 `src-tauri/src/engine/svn_include_check.rs`：

```rust
//! 纳入集合核对：证明「git 实际跟踪的集合 == 配置声明的集合」。
//!
//! 这是一条**行为断言**，不是针对某一种成因写的特判。它同时覆盖：
//! 工作树里的 `.gitignore` 盖掉了我们的再入规则（spec §6.3.1）、
//! `core.excludesFile`、用户手工改过的 `.git/info/exclude`、规则生成器自己的 bug。
//!
//! 与 §6.8 的基线体检**互不替代**：体检证明「磁盘 == 服务器」，这条证明
//! 「git 跟踪的集合 == 配置声明的集合」（spec §15 第 20 条）。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::Path;

/// 按配置遍历磁盘算出**期望**被 git 跟踪的文件集合（W-file，spec §6.3.2）。
///
/// 路径统一为仓库内相对路径 + 正斜杠，与 `git ls-files` 的输出形式对齐。
pub fn expected_set(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
) -> Result<BTreeSet<String>> {
    let root = Path::new(repo_root);
    let mut out = BTreeSet::new();
    for raw in whitelist {
        let entry = raw.replace('\\', "/");
        let clean = entry.trim_end_matches('/');
        if clean.is_empty() {
            continue;
        }
        let abs = root.join(clean);
        if abs.is_file() {
            if matches_ext(clean, extensions) {
                out.insert(clean.to_string());
            }
            continue;
        }
        if abs.is_dir() {
            walk(&abs, root, extensions, &mut out)?;
        }
    }
    Ok(out)
}

fn matches_ext(path: &str, extensions: &[String]) -> bool {
    if extensions.iter().any(|e| e == "*") {
        return true;
    }
    extensions.iter().any(|e| path.ends_with(e.as_str()))
}

fn walk(
    dir: &Path,
    root: &Path,
    extensions: &[String],
    out: &mut BTreeSet<String>,
) -> Result<()> {
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        // `.svn` 与 `.git` 在任何深度都不参与——前者是 SVN 的工作副本数据库，
        // 被 git 跟踪之后一次 checkout 就能损坏一个正在使用的工作副本（spec §6.3）。
        if name == ".svn" || name == ".git" {
            continue;
        }
        if p.is_dir() {
            walk(&p, root, extensions, out)?;
        } else {
            let rel = p
                .strip_prefix(root)
                .map_err(|_| GitError::new("include_set", "路径不在仓库内"))?
                .to_string_lossy()
                .replace('\\', "/");
            if matches_ext(&rel, extensions) {
                out.insert(rel);
            }
        }
    }
    Ok(())
}

/// git **实际**跟踪的集合，限定在白名单路径下（W-path 当 pathspec）。
pub fn actual_set(repo_root: &str, whitelist: &[String]) -> Result<BTreeSet<String>> {
    if whitelist.is_empty() {
        // G6：空列表绝不能退化成「全部」——`git ls-files` 不带 pathspec
        // 会列出整个仓库，比对就变成了拿整仓库对空集。
        return Ok(BTreeSet::new());
    }
    let mut args: Vec<&str> = vec!["ls-files", "--"];
    let owned: Vec<String> = whitelist.iter().map(|w| w.replace('\\', "/")).collect();
    args.extend(owned.iter().map(String::as_str));
    let out = run_git(repo_root, &args)?;
    Ok(out
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// 核对两个集合，不一致则阻断并指认裁决者。
pub fn verify_include_set(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
) -> Result<()> {
    let expected = expected_set(repo_root, whitelist, extensions)?;
    let actual = actual_set(repo_root, whitelist)?;
    if expected == actual {
        return Ok(());
    }
    let missing: Vec<&String> = expected.difference(&actual).collect();
    let unexpected: Vec<&String> = actual.difference(&expected).collect();

    let mut msg = String::from("git 实际跟踪的文件集合与配置声明的不一致。\n");
    if !missing.is_empty() {
        msg.push_str("\n应当被跟踪、实际没有：\n");
        for f in missing.iter().take(20) {
            // 指认是哪条规则做的决定。git 会直接给出「文件:行号:模式」，
            // 比我们自己猜准得多。
            let who = blame_rule(repo_root, f).unwrap_or_else(|_| "（查不到裁决规则）".into());
            msg.push_str(&format!("  {f}   ← {who}\n"));
        }
        if missing.len() > 20 {
            msg.push_str(&format!("  …还有 {} 条\n", missing.len() - 20));
        }
    }
    if !unexpected.is_empty() {
        msg.push_str("\n不该被跟踪、实际跟踪了：\n");
        for f in unexpected.iter().take(20) {
            msg.push_str(&format!("  {f}\n"));
        }
        if unexpected.len() > 20 {
            msg.push_str(&format!("  …还有 {} 条\n", unexpected.len() - 20));
        }
    }
    Err(GitError::new("include_set_mismatch", msg))
}

/// 问 git「这个文件是被哪条规则决定的」。
///
/// `--non-matching` 让它对**没被忽略**的文件也给出裁决规则（形如
/// `.git/info/exclude:5:!/Code/**/*`），于是两个方向都能指认。
fn blame_rule(repo_root: &str, file: &str) -> Result<String> {
    let (_, out) = run_git_code(
        repo_root,
        &["check-ignore", "-v", "--non-matching", "--", file],
    )?;
    let line = out.lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        return Ok("（无匹配规则）".to_string());
    }
    Ok(line)
}
```

在 `src-tauri/src/engine/mod.rs` 里加 `pub mod svn_include_check;`。

- [ ] **Step 6.4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_exclude`
Expected: 10 passed（7 + 3）

- [ ] **Step 6.5: 全套检查并提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_include_check.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_exclude.rs
git commit -m "feat(svn): 纳入集合核对

- What: 新增 engine::svn_include_check，期望集合 vs git ls-files，双向比对并用 check-ignore -v 指认裁决者
- Why: spec §6.3.1——工作树里的 .gitignore 优先级高于 .git/info/exclude，会静默让源码不受保护，而基线体检查不出来
- Impact/Test: cargo test --test svn_exclude 10 passed"
```

---

### Task 7: 仓库识别、`capabilities()` 与 `backend_for` 分流

**Files:**
- Modify: `src-tauri/src/vcs/mod.rs`、`src-tauri/src/vcs/git.rs`、`src-tauri/src/commands/repo.rs`、`src-tauri/src/lib.rs`
- Create: `src-tauri/src/vcs/svn.rs`
- Test: `src-tauri/tests/svn_backend.rs`

**Interfaces:**
- Consumes: Task 1–6 的全部
- Produces:
  - `pub struct Capabilities { pub worktrees: bool, pub tags: bool, pub remotes: bool, pub pull_requests: bool, pub cherry_pick: bool, pub patches: bool }`
  - `pub trait Vcs { fn capabilities(&self) -> Capabilities; /* 其余 16 个方法不变 */ }`
  - `pub enum RepoKind { Git, SvnHybrid, SvnUninitialized, SvnSuspicious }`
  - `pub fn detect_repo_kind(path: &str) -> RepoKind`
  - `pub struct SvnGitBackend`（`vcs/svn.rs`）
  - Tauri 命令 `read_capabilities(path: String) -> Result<Capabilities, GitError>`
  - Tauri 命令 `probe_svn_environment() -> Result<String, GitError>`（返回 `"available" | "stale_path" | "not_installed"`）

- [ ] **Step 7.1: 写失败测试**

创建 `src-tauri/tests/svn_backend.rs`：

```rust
//! 仓库识别与能力声明。

#[path = "svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::vcs::{backend_for, detect_repo_kind, RepoKind};
use std::fs;
use std::process::Command;
use svn_fixtures::svn_repo;
use tempfile::TempDir;

fn git_init(dir: &std::path::Path) {
    for args in [
        vec!["init"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        assert!(Command::new("git")
            .current_dir(dir)
            .args(&args)
            .status()
            .unwrap()
            .success());
    }
}

fn write_config(dir: &std::path::Path, url: &str) {
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(
        dir.join(".git/mygitclient.json"),
        format!(
            r#"{{"version":1,"mode":"svn-git","svnUrl":"{url}",
                 "whitelist":["Code/"],"extensions":[".cs"],
                 "baselineBranch":"svn/base"}}"#
        ),
    )
    .unwrap();
}

#[test]
fn a_plain_git_repo_is_detected_as_git() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());
    assert_eq!(detect_repo_kind(dir.path().to_str().unwrap()), RepoKind::Git);
}

#[test]
fn an_svn_working_copy_without_git_is_uninitialized() {
    let fx = svn_repo();
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnUninitialized);
}

#[test]
fn svn_plus_git_without_config_is_suspicious() {
    // spec §6.1：可能是别人留下的 .git，阻断并提示，不要猜。
    let fx = svn_repo();
    git_init(&fx.wc());
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnSuspicious);
}

#[test]
fn svn_plus_git_plus_config_is_hybrid() {
    let fx = svn_repo();
    git_init(&fx.wc());
    write_config(&fx.wc(), "file:///whatever");
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnHybrid);
}

#[test]
fn detection_checks_svn_before_git() {
    // 混合仓库两者都有，先查 .git 就会永远判成普通 git 仓库（spec §6.1）。
    let fx = svn_repo();
    git_init(&fx.wc());
    write_config(&fx.wc(), "file:///whatever");
    assert_ne!(detect_repo_kind(&fx.wc_str()), RepoKind::Git);
}

#[test]
fn a_suspicious_repo_refuses_to_open() {
    let fx = svn_repo();
    git_init(&fx.wc());
    let err = backend_for(&fx.wc_str()).expect_err("可疑仓库必须阻断");
    assert_eq!(err.code, "svn_unconfigured_git");
}

#[test]
fn a_git_backend_advertises_every_capability() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());
    let caps = backend_for(dir.path().to_str().unwrap())
        .unwrap()
        .capabilities();
    assert!(caps.worktrees && caps.tags && caps.remotes);
    assert!(caps.pull_requests && caps.cherry_pick && caps.patches);
}

#[test]
fn an_svn_backend_hides_the_git_only_entries() {
    // Phase 0 的交接明确警告过：这些命令**没有进接缝**，仍会直连 git。
    // 光靠 trait 拦不住它们，必须靠 capabilities + 前端不渲染入口。
    let fx = svn_repo();
    git_init(&fx.wc());
    write_config(&fx.wc(), "file:///whatever");
    let caps = backend_for(&fx.wc_str()).unwrap().capabilities();
    assert!(!caps.worktrees, "SVN 模式下 worktree 整块隐藏");
    assert!(!caps.tags);
    assert!(!caps.remotes);
    assert!(!caps.pull_requests);
    assert!(!caps.cherry_pick);
    assert!(!caps.patches);
}

#[test]
fn an_svn_backend_still_delegates_git_reads() {
    // §16 Phase 1 验收：「能打开 SVN 仓库，本地 git 的暂存/分支/stash 可用」。
    let fx = svn_repo();
    git_init(&fx.wc());
    write_config(&fx.wc(), "file:///whatever");
    fs::write(fx.wc().join("Code/A.cs"), "changed\n").unwrap();
    let backend = backend_for(&fx.wc_str()).unwrap();
    // 有改动但还没提交过 → status 读得出来，不 panic
    let st = backend.status().unwrap();
    assert!(st.unstaged.iter().any(|f| f.path.contains("A.cs")));
}
```

- [ ] **Step 7.2: 跑测试确认失败**

Run: `cd src-tauri && cargo test --test svn_backend`
Expected: 编译失败

- [ ] **Step 7.3: 加 `Capabilities` 与 `detect_repo_kind`**

在 `src-tauri/src/vcs/mod.rs` 的 `use` 之后、`pub trait Vcs` 之前插入：

```rust
pub mod svn;

use serde::Serialize;

/// 后端支持哪些功能。前端据此决定**不渲染**哪些入口。
///
/// 为什么需要它：Phase 0 只把 16 个方法切进了 `trait Vcs`，而
/// `checkout_remote_branch` / `create_tag` / `cherry_pick` / `save_patch` /
/// `fetch` / `pull` / `push` 以及整个 `commands/worktree.rs`（1359 行）
/// **仍然直连 `run_git`**。一旦 `backend_for` 能返回非 git 后端，这些命令会对着
/// 同一个工作目录做 git 操作。接缝拦不住它们，只能靠「不给入口」。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub worktrees: bool,
    pub tags: bool,
    pub remotes: bool,
    pub pull_requests: bool,
    pub cherry_pick: bool,
    pub patches: bool,
}

impl Capabilities {
    /// 普通 git 仓库：全都支持。
    pub fn all() -> Self {
        Self {
            worktrees: true,
            tags: true,
            remotes: true,
            pull_requests: true,
            cherry_pick: true,
            patches: true,
        }
    }
}

/// 打开一个目录时的四种判定（spec §6.1）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepoKind {
    /// 只有 `.git`：现有行为，零变化。
    Git,
    /// `.svn` + `.git` + 配置文件。
    SvnHybrid,
    /// 有 `.svn` 无 `.git`：引导走初始化向导（Phase 1b）。
    SvnUninitialized,
    /// `.svn` + `.git`，但没有配置文件：可能是别人留下的 `.git`，阻断。
    SvnSuspicious,
}

/// 判定目录类型。
///
/// **必须先查 `.svn`**：混合仓库两者都有，先查 `.git` 就会永远判成普通 git 仓库
/// （spec §6.1）。
pub fn detect_repo_kind(path: &str) -> RepoKind {
    let root = std::path::Path::new(path);
    let has_svn = root.join(".svn").is_dir();
    let has_git = root.join(".git").exists();
    if !has_svn {
        return RepoKind::Git;
    }
    if !has_git {
        return RepoKind::SvnUninitialized;
    }
    if crate::engine::svn_config::config_path(path).is_file() {
        RepoKind::SvnHybrid
    } else {
        RepoKind::SvnSuspicious
    }
}
```

在 `pub trait Vcs` 里、`fn open` 之前加一个方法：

```rust
    /// 本后端支持哪些功能。见 [`Capabilities`]。
    fn capabilities(&self) -> Capabilities;
```

替换 `backend_for`：

```rust
/// 为某个工作目录挑一个后端。
pub fn backend_for(path: &str) -> Result<Box<dyn Vcs>> {
    match detect_repo_kind(path) {
        RepoKind::Git => Ok(Box::new(git::GitBackend::new(path))),
        RepoKind::SvnHybrid => Ok(Box::new(svn::SvnGitBackend::open(path)?)),
        // 这两种都不是「能工作的仓库」，但原因不同，错误码必须分开——
        // 前端据此给出的引导完全相反（去初始化 vs 去查那个 .git 哪来的）。
        RepoKind::SvnUninitialized => Err(crate::error::GitError::new(
            "svn_not_initialized",
            format!("{path} 是一个 SVN 工作副本，但还没有初始化为混合仓库"),
        )),
        RepoKind::SvnSuspicious => Err(crate::error::GitError::new(
            "svn_unconfigured_git",
            format!(
                "{path} 同时有 .svn 和 .git，但没有 .git/mygitclient.json。\
                 这个 .git 可能是别人留下的——请确认之后再初始化，不要直接使用"
            ),
        )),
    }
}
```

- [ ] **Step 7.4: 给 `GitBackend` 补 `capabilities`**

在 `src-tauri/src/vcs/git.rs` 的 `impl Vcs for GitBackend` 最前面加：

```rust
    fn capabilities(&self) -> super::Capabilities {
        super::Capabilities::all()
    }
```

并把顶部的 `use super::Vcs;` 改成 `use super::{Capabilities, Vcs};`，方法体改成 `Capabilities::all()`。

- [ ] **Step 7.5: 写 `SvnGitBackend`**

创建 `src-tauri/src/vcs/svn.rs`：

```rust
//! SVN + git 混合仓库的后端。
//!
//! **Phase 1a 的它几乎只做一件事：声明 capabilities。** 全部 git 语义的读写
//! 原样委托给 `GitBackend`——§16 Phase 1 的验收标准就是「能打开 SVN 仓库，
//! 本地 git 的暂存/分支/stash 可用」。SVN 侧的状态判定、基线、混合图都在 1b。
//!
//! 之所以现在就要有这个类型：Phase 0 的交接指出接缝**不完整**，
//! `checkout_remote_branch` / `create_tag` / `cherry_pick` / `save_patch` /
//! `fetch` / `pull` / `push` 与整个 `commands/worktree.rs` 仍直连 git。
//! 光有 trait 拦不住它们，必须有一个能说「我不支持这些」的后端。

use super::git::GitBackend;
use super::{Capabilities, Vcs};
use crate::engine::diff::{DiffData, StatusData};
use crate::engine::graph::GraphData;
use crate::engine::repo::RepoInfo;
use crate::engine::stash::StashEntry;
use crate::engine::svn_config::SvnConfig;
use crate::error::Result;

pub struct SvnGitBackend {
    git: GitBackend,
    /// 已校验过的配置。持有它而不是每次读盘，正是 `trait Vcs` 绑定工作目录
    /// （而非每个方法收 path）的理由，见 `vcs/mod.rs` 开头。
    ///
    /// Phase 1a 只在 `open()` 里用它做校验，字段本身还没有读者——Phase 1b 的
    /// 状态判定、基线与体检都要用。现在就持有是为了不在 1b 改构造函数签名。
    #[allow(dead_code)]
    config: SvnConfig,
}

impl SvnGitBackend {
    /// 打开并校验。配置不合法时**这里就失败**，不要让半个坏配置流到下游。
    pub fn open(path: &str) -> Result<Self> {
        let config = crate::engine::svn_config::read_config(path)?;
        Ok(Self {
            git: GitBackend::new(path),
            config,
        })
    }
}

impl Vcs for SvnGitBackend {
    /// SVN 模式下这些功能在概念上就不存在，前端不该渲染入口。
    ///
    /// 逐条理由见 spec §1.2 与 Phase 0 决策日志的交接表：worktree / tag /
    /// 远程分支 / PR 在 SVN 侧无对应物；`cherry_pick` 依赖 git 的 DAG 语义；
    /// `save_patch` 是 `format-patch`，git 专属。
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            worktrees: false,
            tags: false,
            remotes: false,
            pull_requests: false,
            cherry_pick: false,
            patches: false,
        }
    }

    // 以下全部原样委托。Phase 1a 不改变任何 git 行为。
    fn open(&self) -> Result<RepoInfo> {
        self.git.open()
    }
    fn graph(&self) -> Result<GraphData> {
        self.git.graph()
    }
    fn status(&self) -> Result<StatusData> {
        self.git.status()
    }
    fn diff(&self, scope: &str) -> Result<DiffData> {
        self.git.diff(scope)
    }
    fn stage(&self, paths: &[String]) -> Result<()> {
        self.git.stage(paths)
    }
    fn unstage(&self, paths: &[String]) -> Result<()> {
        self.git.unstage(paths)
    }
    fn discard(&self, paths: &[String], staged: bool) -> Result<()> {
        self.git.discard(paths, staged)
    }
    fn commit(&self, message: &str, amend: bool) -> Result<()> {
        self.git.commit(message, amend)
    }
    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()> {
        self.git.create_branch(name, start_point)
    }
    fn delete_branch(&self, name: &str) -> Result<()> {
        self.git.delete_branch(name)
    }
    fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.git.rename_branch(old, new)
    }
    fn checkout(&self, rev: &str) -> Result<()> {
        self.git.checkout(rev)
    }
    fn list_stashes(&self) -> Result<Vec<StashEntry>> {
        self.git.list_stashes()
    }
    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()> {
        self.git.stash_push(message, include_untracked)
    }
    fn stash_apply(&self, index: usize) -> Result<()> {
        self.git.stash_apply(index)
    }
    fn stash_pop(&self, index: usize) -> Result<()> {
        self.git.stash_pop(index)
    }
    fn stash_drop(&self, index: usize) -> Result<()> {
        self.git.stash_drop(index)
    }
}
```

⚠️ **不要写成 `#[allow(dead_code, reason = "…")]`。** `reason` 需要 Rust 1.81，而
`src-tauri/Cargo.toml` 声明的 `rust-version` 是 **1.77.2**——本机 toolchain 是 1.95 能编过，
但那等于悄悄抬高了 MSRV。用裸 `#[allow(dead_code)]` + 上方中文注释，`AGENTS.md` §7.3
要求的「`#[allow]` 必须有注释说明理由」由那段注释满足。

- [ ] **Step 7.6: 加两个命令**

在 `src-tauri/src/commands/repo.rs` 末尾追加：

```rust
/// 读取当前仓库的能力集合，前端据此决定渲染哪些入口。
#[tauri::command(async)]
pub fn read_capabilities(
    path: String,
) -> Result<crate::vcs::Capabilities, crate::error::GitError> {
    Ok(backend_for(&path)?.capabilities())
}

/// 探测 svn 是否可用。返回 `"available"` / `"stale_path"` / `"not_installed"`。
///
/// 后两者必须分开：安装器写的是**持久** PATH，已经在跑的进程持有启动时的环境
/// 快照，于是「装完之后应用仍然看不见」是一个真实且常见的状态，正确引导是
/// **重启应用**而不是重装（spec §17.1）。
#[tauri::command(async)]
pub fn probe_svn_environment() -> Result<String, crate::error::GitError> {
    use crate::engine::svn_cli::{probe_svn, SvnAvailability};
    Ok(match probe_svn() {
        SvnAvailability::Available { .. } => "available",
        SvnAvailability::StalePath => "stale_path",
        SvnAvailability::NotInstalled => "not_installed",
    }
    .to_string())
}
```

在 `src-tauri/src/lib.rs` 的 `invoke_handler!` 里，`commands::repo::open_repo,` 之后加两行（保持字母序）：

```rust
            commands::repo::probe_svn_environment,
            commands::repo::read_capabilities,
```

- [ ] **Step 7.7: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_backend`
Expected: 9 passed

- [ ] **Step 7.8: 全套检查并提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/vcs/ src-tauri/src/commands/repo.rs src-tauri/src/lib.rs src-tauri/tests/svn_backend.rs
git commit -m "feat(svn): 仓库识别、capabilities 与 backend_for 分流

- What: trait Vcs 加 capabilities()；新增 RepoKind / detect_repo_kind / SvnGitBackend；新增 read_capabilities 与 probe_svn_environment 两个命令
- Why: spec §6.1 的四分支判定必须先查 .svn；Phase 0 交接指出接缝不完整，worktree/tag/remote/PR 等仍直连 git，只能靠不给入口
- Impact/Test: cargo test --test svn_backend 9 passed"
```

---

### Task 8: 前端按能力隐藏 git 专属入口

**Files:**
- Modify: `src/store/repoStore.ts`、`src/components/Toolbar.tsx`、以及渲染 worktree / tag / PR / remote 入口的组件
- Test: `tests/vitest/capabilities.test.ts`（新建）

**Interfaces:**
- Consumes: Task 7 的 `read_capabilities` 命令（返回 camelCase 字段）
- Produces: `src/engine/capabilities.ts` 导出 `export interface Capabilities { worktrees: boolean; tags: boolean; remotes: boolean; pullRequests: boolean; cherryPick: boolean; patches: boolean }` 与 `export const ALL_CAPABILITIES: Capabilities`

- [ ] **Step 8.1: 写失败测试**

创建 `tests/vitest/capabilities.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import { ALL_CAPABILITIES, visibleSections } from '@/engine/capabilities'

describe('capabilities', () => {
  it('普通 git 仓库渲染全部侧边栏分区', () => {
    expect(visibleSections(ALL_CAPABILITIES)).toEqual([
      'branches',
      'remotes',
      'tags',
      'stashes',
      'worktrees',
      'pullRequests',
    ])
  })

  it('SVN 模式只留下分支与 stash', () => {
    const caps = {
      worktrees: false,
      tags: false,
      remotes: false,
      pullRequests: false,
      cherryPick: false,
      patches: false,
    }
    expect(visibleSections(caps)).toEqual(['branches', 'stashes'])
  })

  it('分支与 stash 没有对应的 capability 开关——它们在两种模式下都可用', () => {
    // §16 Phase 1 验收：「本地 git 的暂存/分支/stash 可用」。
    // 如果哪天给它们加了开关，这条会提醒去改验收标准而不是顺手关掉。
    expect(Object.keys(ALL_CAPABILITIES).sort()).toEqual([
      'cherryPick',
      'patches',
      'pullRequests',
      'remotes',
      'tags',
      'worktrees',
    ])
  })
})
```

- [ ] **Step 8.2: 跑测试确认失败**

Run: `pnpm test`
Expected: FAIL，`Cannot find module '@/engine/capabilities'`

- [ ] **Step 8.3: 写 `src/engine/capabilities.ts`**

```ts
/**
 * 后端能力声明。字段与 Rust 侧 `vcs::Capabilities` 一一对应。
 *
 * Tauri 把 Rust 的 snake_case 转成 JS 的 camelCase（`pull_requests` →
 * `pullRequests`），这层转换**没有编译期检查**，写错只会在运行时静默拿到
 * `undefined`——而 `undefined` 是 falsy，表现就是「入口莫名其妙消失了」。
 * 所以这个接口的字段名必须与 Rust 侧逐字对齐（AGENTS.md §7.3）。
 */
export interface Capabilities {
  worktrees: boolean
  tags: boolean
  remotes: boolean
  pullRequests: boolean
  cherryPick: boolean
  patches: boolean
}

/** 普通 git 仓库：全支持。也是拿不到后端应答时的保守默认值。 */
export const ALL_CAPABILITIES: Capabilities = {
  worktrees: true,
  tags: true,
  remotes: true,
  pullRequests: true,
  cherryPick: true,
  patches: true,
}

export type SidebarSection =
  | 'branches'
  | 'remotes'
  | 'tags'
  | 'stashes'
  | 'worktrees'
  | 'pullRequests'

/**
 * 侧边栏该渲染哪些分区。
 *
 * `branches` 与 `stashes` **没有开关**：§16 Phase 1 的验收标准是「本地 git 的
 * 暂存/分支/stash 可用」，SVN 模式下它们照常工作。
 */
export function visibleSections(caps: Capabilities): SidebarSection[] {
  const out: SidebarSection[] = ['branches']
  if (caps.remotes) out.push('remotes')
  if (caps.tags) out.push('tags')
  out.push('stashes')
  if (caps.worktrees) out.push('worktrees')
  if (caps.pullRequests) out.push('pullRequests')
  return out
}
```

- [ ] **Step 8.4: 跑测试确认通过**

Run: `pnpm test`
Expected: 新增 3 个用例通过

- [ ] **Step 8.5: 接进 store**

在 `src/store/repoStore.ts` 的 `RepoState` 里加一个字段 `capabilities: Capabilities`，初值 `ALL_CAPABILITIES`；在打开仓库的两处 `invoke<RepoInfo>('open_repo', { path })` 之后各加一次：

```ts
const caps = await invoke<Capabilities>('read_capabilities', { path })
```

拿不到就**保持 `ALL_CAPABILITIES`**（别把入口全关掉，那样普通 git 仓库会被一个后端错误搞成半残）。

- [ ] **Step 8.6: 组件按能力渲染**

把 `Toolbar.tsx` / 侧边栏里 worktree、tag、PR、remote、cherry-pick、save patch 的入口包在对应的 `caps.x &&` 后面。**只隐藏入口，不删代码**——这些命令在普通 git 仓库上照常要用。

- [ ] **Step 8.7: 前端全套检查并提交**

Run: `pnpm build && pnpm test`
Expected: tsc 通过、vitest 全绿

```bash
git add src/engine/capabilities.ts src/store/repoStore.ts src/components tests/vitest/capabilities.test.ts
git commit -m "feat(svn): 前端按后端能力隐藏 git 专属入口

- What: 新增 src/engine/capabilities.ts，store 打开仓库时读一次 read_capabilities，组件按能力渲染
- Why: Phase 0 交接——worktree/tag/remote/PR/cherry-pick/save_patch 未进接缝，仍直连 git，只能靠不给入口
- Impact/Test: pnpm build 通过；pnpm test 新增 3 个用例"
```

---

### Task 9: 收尾验证

**Files:**
- Modify: `docs/superpowers/plans/2026-09-13-svn-phase1a-foundation-decisions.md`

- [ ] **Step 9.1: 两侧全套检查**

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test
cd .. && pnpm build && pnpm test
```

把实际数字记进决策日志。Rust 侧新增测试数 = 4（Task 1）+ 6（Task 2）+ 8（Task 3）
+ 4（Task 4）+ 7（Task 5）+ 3（Task 6）+ 9（Task 7）= **41**，前端新增 3 个 vitest 用例。
**以实测为准，不要照抄这个数字**——对不上就说明有 Task 的测试没写全或写多了，两种都要查。

- [ ] **Step 9.2: 命令注册对账**

```bash
grep -rn "#\[tauri::command" src-tauri/src/commands/ | wc -l
grep -c "commands::" src-tauri/src/lib.rs
```

Expected: 两者相等，且等于 master 上的数字 + 2。

⚠️ Phase 0 记过：`grep` 数到的数字可能比真实命令数多一个——`session.rs` 的文档注释里有一句提及。核对时按实际命令定义数。

- [ ] **Step 9.3: 确认没有 SVN 写操作混进来**

```bash
grep -rnE '"(add|commit|update|revert|delete|propset|checkout)"' src-tauri/src/engine/svn_*.rs
```

Expected: **零输出**。G8 要求本期不发出任何 SVN 写操作。（`tests/svn_fixtures.rs` 里有，那是 fixture 的构造，不在 `src/` 下。）

- [ ] **Step 9.4: 真实窗口验证**

`AGENTS.md` §5.1 要求：改了 Tauri 命令签名就必须实际跑 `pnpm tauri dev` 确认 invoke 真的通。本期新增了两个命令。

```bash
pnpm tauri dev
```

在真实窗口里逐条确认，结果记进决策日志：

| # | 操作 | 期望 |
|---|---|---|
| 1 | 打开一个**普通 git 仓库** | 与改动前完全一致：worktree / tag / PR / remote 入口**都在** |
| 2 | 在该仓库里暂存、提交、切分支、stash | 全部照常 |
| 3 | 打开一个**临时 SVN fixture 工作副本**（无 `.git`） | 报 `svn_not_initialized`，提示去初始化，**不崩** |
| 4 | 给它 `git init` 但不写配置文件 | 报 `svn_unconfigured_git`，提示可能是别人留下的 `.git` |
| 5 | 补上 `.git/mygitclient.json` 再打开 | 能打开；worktree / tag / PR / remote 入口**消失**；分支与 stash **还在** |
| 6 | 在该混合仓库里暂存 → 提交 → 切分支 → stash | 全部照常（§16 Phase 1 验收） |

⚠️ 第 3–6 条必须在**临时 fixture 工作副本**上做，禁止拿任何真实 SVN 仓库试（`AGENTS.md` 开篇的最高优先级约束）。

- [ ] **Step 9.5: 补齐决策日志的四类条目**

`AGENTS.md` §2.1 要求决策条目至少覆盖四类，缺一类视为记录不完整：**偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的**。

本期「为什么安全」至少要论证：

- 本期**没有任何 SVN 写操作**（Step 9.3 的 grep 是证据）
- 普通 git 仓库的行为**零变化**：`detect_repo_kind` 在没有 `.svn` 时立刻返回 `Git`，`backend_for` 走的还是原来那条分支；`GitBackend::capabilities()` 返回全 true
- 新增的两个命令都是**只读**的
- 空白名单在 `enumerate_wcs` 与 `actual_set` 两处都提前返回，不会退化成「整个仓库」

- [ ] **Step 9.6: 独立 review**

`AGENTS.md` §4.1：走 PR 流程的改动合并前必须由**独立于实现者的人或 agent** 完成一次 review，按 §13 rubric 执行，高危问题合并前必须修完。

- [ ] **Step 9.7: 开 Draft PR**

```bash
gh pr create --draft --title "feat(svn): Phase 1a 只读地基" --body "..."
```

⚠️ 本仓库**没有 PR 门禁 CI**，禁止宣称「CI 通过」；PR 描述里的验证结论必须来自本地实际执行（`AGENTS.md` §4.1）。

---

## Self-Review

**Spec 覆盖对照**（本期范围内的条目）

| spec 条目 | 落在哪个 Task |
|---|---|
| §4.4 SvnClient（`--non-interactive`、子命令区分 `--ignore-externals`、真 XML parser） | Task 1、2 |
| §6.1 仓库识别四分支 | Task 7 |
| §6.2 配置文件 / §6.2.1 校验 | Task 3 |
| §6.3 白名单语义与 exclude 生成（含 `.svn/` 兜底） | Task 5 |
| §6.3.1 纳入集合核对 | Task 6 |
| §6.3.2 W-path / W-file 两种粒度 | Task 5（exclude 用 W-file）、Task 4 + 6（SVN target / pathspec 用 W-path） |
| §6.5 P4 / §6.5.5 嵌套 WC 枚举 | Task 4 |
| §6.6.4 陷阱一/二/三（externals 污染、取错 revision、行内正则） | Task 2 |
| §12.1 统一过滤 / §12.2 空参数拒绝 | Task 4、6 的空列表提前返回；Task 3 的条目校验 |
| §17 / §17.1 环境前置与两种缺失状态 | Task 1、7 |
| Phase 0 交接：`capabilities()` + 前端按能力隐藏 | Task 7、8 |

**明确留到 Phase 1b 的**：§6.4 基线分支、§6.5 P1/P2/P3/P5 与初始化向导、§6.6 状态判定与失同步检测、§6.8 基线体检、§10 混合图、§16 的「重建基线」。

**类型一致性**：`Capabilities` 的 6 个字段在 Rust（`vcs/mod.rs`）、TS（`src/engine/capabilities.ts`）、测试三处逐字对齐；`SvnInfoEntry` 的字段在 Task 2 定义、Task 4 消费；`SvnConfig` 在 Task 3 定义、Task 7 消费。

**已知会在实施时暴露的不确定项**（记进决策日志，不要当成计划缺陷）：

1. `generate_exclude` 对**单文件**白名单条目的规则形态没有实测过，Step 5.4 给了核对手段（`git check-ignore -v --non-matching`）
2. `svn status --xml` 在**深层嵌套 external** 上的 `path` 属性是相对谁的，Task 4 假设相对命令的工作目录；Step 4.5 测不过时先用真实命令看输出再改
3. Task 8 的组件改动范围要等实际打开 `Toolbar.tsx` 才能确定，计划里只给了原则（只隐藏入口、不删代码）
