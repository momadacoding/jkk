# 工具栏自定义命令 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在工具栏右侧提供一组用户可自定义的按钮，把当前 worktree 交给 VS Code / 终端 / 文件管理器等外部程序。

**Architecture:** 配置是 app config dir 下一份用户手写的 `customCommands.json`；Rust 侧只负责读配置和 spawn 进程，占位符展开放在前端纯函数里以便单测；全程 argv 数组直传，不经 shell 解释。

**Tech Stack:** Rust + Tauri 2（`std::process::Command`、`serde_json`）、React 18 + TypeScript、vitest、cargo test + `tempfile`。

规格文档：`docs/superpowers/specs/2026-08-09-custom-toolbar-commands-design.md`

## Global Constraints

- **不引入任何新依赖**（npm 包或 crate）。`tauri-plugin-shell` 与 `tauri-plugin-opener` 已在评审中排除。
- MSRV 是 `1.77.2`（`src-tauri/Cargo.toml`）。不要使用更高版本才稳定的 API（例如 `Option::is_none_or`——本仓库已经因为它破坏过 MSRV）。
- 主干分支是 `master`。实现必须在**独立 worktree** 中进行，动第一行代码前用 `EnterWorktree` 切入。
- 全程 argv 数组直传，**任何地方都不得把用户路径拼进 shell 字符串**。
- 前端严格模式：`strict`、`noUnusedLocals`、`noUnusedParameters` 已开启。不得使用 `any` 或 `@ts-ignore`。
- Rust 注释用中文，解释「为什么」。前端单引号、尾随逗号。
- 所有涉及文件系统的测试必须用 `tempfile` 建临时目录，**绝不触碰真实的 app config dir**，也不碰本仓库。
- `cargo clippy --all-targets` 的告警必须处理掉，不得用 `#[allow(...)]` 掩盖。
- 本仓库**没有 CI、没有 lint 工具**。禁止宣称「CI 通过」或「lint 通过」。
- 因为跨多个 Task，必须建 `docs/superpowers/plans/2026-08-09-custom-toolbar-commands-decisions.md`，**在动 Task 1 之前建好**。

## 占位符契约（全局共享）

三个占位符，前后端都以此为准：

| 占位符 | 值来源 |
|---|---|
| `$WORKTREE` | 当前打开的 worktree 绝对路径（`RepoState.path`） |
| `$BRANCH` | 当前分支名（`RepoState.head`） |
| `$HEAD` | HEAD 完整 oid（`RepoState.headHash`） |

替换只在前端 `src/engine/customCommands.ts` 里做。Rust 侧**完全不认识占位符**。

## File Structure

| 文件 | 责任 |
|---|---|
| `src-tauri/src/engine/custom_config.rs` | 配置结构体、各平台默认预设、`load_from` / `write_defaults` |
| `src-tauri/src/engine/launcher.rs` | `resolve_program`、`spawn_detached`、`open_in_default_app` |
| `src-tauri/src/commands/custom.rs` | 三个 `#[tauri::command]` 入口，解析 app config dir |
| `src-tauri/tests/custom_commands.rs` | 配置解析 + launcher 的集成测试 |
| `src/engine/customCommands.ts` | 类型定义、占位符展开、命令计划（纯函数） |
| `src/store/customCommandsStore.ts` | `useCustomCommands` hook，唯一的 `invoke` 出口 |
| `src/components/CustomCommandBar.tsx` | 按钮组 + `⚙` 菜单 |
| `tests/vitest/customCommands.test.ts` | 占位符展开的单测 |

修改的既有文件：`src-tauri/src/engine/mod.rs`、`src-tauri/src/commands/mod.rs`、`src-tauri/src/lib.rs`、`src/components/Toolbar.tsx`、`src/styles.css`、`README.md`。

---

### Task 0: 准备 worktree 与决策日志

**Files:**
- Create: `docs/superpowers/plans/2026-08-09-custom-toolbar-commands-decisions.md`

- [ ] **Step 1: 确认当前位置与分支状态**

```bash
git -C C:/Users/liuhao/workspace/dev/Dev/mygitclient status --short
git -C C:/Users/liuhao/workspace/dev/Dev/mygitclient log --oneline -3
```

预期：工作区干净，`master` 上最新提交是本计划与 spec 的文档提交。

- [ ] **Step 2: 用 EnterWorktree 切入独立 worktree**

用 `EnterWorktree` 工具创建并切入，名称 `custom-toolbar-commands`。
**不要**用 `git worktree add` + `cd` 代替——那不改变 session 的真实工作目录，
后续相对路径命令会静默打到主工作区。

- [ ] **Step 3: 自检确实切进去了**

```bash
pwd
git log --oneline -3
```

预期：`pwd` 在 `.claude/worktrees/custom-toolbar-commands` 或 `.worktrees/` 下，
且基点 commit 与 Step 1 看到的一致。若 `pwd` 仍是启动目录，**立即停止**，先切入再继续。

- [ ] **Step 4: 确认 `.claude/worktrees/` 已被忽略**

```bash
git check-ignore -v .claude/worktrees || echo "未忽略，需要加进 .gitignore"
```

若未忽略，把 `.claude/worktrees/` 加入 `.gitignore`（AGENTS.md §4.1 已记录这是已知待办）。

- [ ] **Step 5: 建决策日志骨架**

```markdown
# 工具栏自定义命令 实现决策日志

计划：`docs/superpowers/plans/2026-08-09-custom-toolbar-commands.md`

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 准备 worktree | | | |
| 1 配置解析 | | | |
| 2 launcher | | | |
| 3 Tauri 命令层 | | | |
| 4 前端纯逻辑 | | | |
| 5 前端接线与 UI | | | |
| 6 真机验证与文档 | | | |

## 决策条目

（按 Task 分节，四类必须齐全：偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的）
```

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/plans/2026-08-09-custom-toolbar-commands-decisions.md
git commit -m "docs(custom-commands): 建实现决策日志"
```

---

### Task 1: 配置结构与解析

**Files:**
- Create: `src-tauri/src/engine/custom_config.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/custom_commands.rs`

**Interfaces:**
- Consumes: `crate::error::{GitError, Result}`（已存在）
- Produces:
  - `pub struct CustomCommand { pub id: String, pub label: String, pub program: String, pub args: Vec<String>, pub cwd: String }`
  - `pub fn default_commands() -> Vec<CustomCommand>`
  - `pub fn load_from(path: &Path) -> Result<Vec<CustomCommand>>`

- [ ] **Step 1: 写失败的测试**

创建 `src-tauri/tests/custom_commands.rs`：

```rust
use app_lib::engine::custom_config::{self, CustomCommand};
use std::fs;
use tempfile::TempDir;

/// 配置文件缺失时要写出当前平台的默认预设，而不是返回空列表。
#[test]
fn missing_file_writes_platform_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("customCommands.json");

    let commands = custom_config::load_from(&path).unwrap();

    assert!(!commands.is_empty(), "默认预设不该是空的");
    assert!(path.exists(), "缺失时应当把默认预设落盘");
    // 落盘的内容必须能被自己再读回来，否则用户第一次改配置就会踩到格式问题
    let reread = custom_config::load_from(&path).unwrap();
    assert_eq!(commands, reread);
}

/// 用户手写配置写错一个逗号，不能把整份配置冲掉。
#[test]
fn broken_json_errors_and_leaves_file_untouched() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    let broken = b"{ \"commands\": [ ";
    fs::write(&path, broken).unwrap();

    let err = custom_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "custom_config");

    let after = fs::read(&path).unwrap();
    assert_eq!(after, broken, "解析失败时绝不能改写用户的文件");
}

#[test]
fn unknown_fields_are_ignored() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"future":"whatever","commands":[
            {"id":"a","label":"A","program":"echo","args":["x"],"extra":true}
        ]}"#,
    )
    .unwrap();

    let commands = custom_config::load_from(&path).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, "echo");
}

#[test]
fn args_and_cwd_fall_back_to_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"commands":[{"id":"a","label":"A","program":"code"}]}"#,
    )
    .unwrap();

    let commands = custom_config::load_from(&path).unwrap();
    assert_eq!(commands[0].args, Vec::<String>::new());
    assert_eq!(commands[0].cwd, "$WORKTREE");
}

#[test]
fn empty_command_list_is_valid() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(&path, r#"{"version":1,"commands":[]}"#).unwrap();

    assert_eq!(custom_config::load_from(&path).unwrap(), Vec::new());
}

/// id 重复只是小瑕疵，不值得让用户丢掉整份配置。
#[test]
fn duplicate_ids_are_kept() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"commands":[
            {"id":"a","label":"One","program":"echo"},
            {"id":"a","label":"Two","program":"echo"}
        ]}"#,
    )
    .unwrap();

    assert_eq!(custom_config::load_from(&path).unwrap().len(), 2);
}

/// 默认预设里每一条都必须字段完整，否则第一次启动就是坏的。
#[test]
fn platform_defaults_are_well_formed() {
    for cmd in custom_config::default_commands() {
        let CustomCommand { id, label, program, .. } = &cmd;
        assert!(!id.is_empty(), "{cmd:?} 缺 id");
        assert!(!label.is_empty(), "{cmd:?} 缺 label");
        assert!(!program.is_empty(), "{cmd:?} 缺 program");
    }
}
```

- [ ] **Step 2: 跑测试确认它失败**

```bash
cd src-tauri && cargo test --test custom_commands
```

预期：编译失败，`could not find custom_config in engine`。

- [ ] **Step 3: 实现 `custom_config.rs`**

创建 `src-tauri/src/engine/custom_config.rs`：

```rust
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 一条用户自定义命令。
///
/// 字段名全是单词，Tauri 的 snake_case → camelCase 转换在这里没有差异，
/// 但仍显式标注 `rename_all`，与 `RepoInfo` 等既有结构保持一致；
/// 将来加多词字段时，用户配置文件里的键名才不会突然改变风格。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomCommand {
    pub id: String,
    pub label: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_cwd")]
    pub cwd: String,
}

fn default_cwd() -> String {
    "$WORKTREE".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    /// 目前恒为 1，只为将来迁移留位；解析时不校验其值。
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    commands: Vec<CustomCommand>,
}

fn default_version() -> u32 {
    1
}

fn cmd(id: &str, label: &str, program: &str, args: &[&str]) -> CustomCommand {
    CustomCommand {
        id: id.to_string(),
        label: label.to_string(),
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: default_cwd(),
    }
}

/// 当前平台的默认预设。
///
/// 这些程序在本机不一定装了——那是预期情况，点下去会报「找不到可执行文件」
/// 并指向配置文件。这里不做可用性探测。
pub fn default_commands() -> Vec<CustomCommand> {
    #[cfg(target_os = "windows")]
    {
        vec![
            cmd("vscode", "VS Code", "code", &["$WORKTREE"]),
            cmd("terminal", "Terminal", "wt", &["-d", "$WORKTREE"]),
            cmd("explorer", "Explorer", "explorer", &["$WORKTREE"]),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            cmd("vscode", "VS Code", "code", &["$WORKTREE"]),
            cmd("terminal", "Terminal", "open", &["-a", "Terminal", "$WORKTREE"]),
            cmd("finder", "Finder", "open", &["$WORKTREE"]),
        ]
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        vec![
            cmd("vscode", "VS Code", "code", &["$WORKTREE"]),
            // 大多数发行版把它指向用户选定的终端；它自己会在 cwd 里启动
            cmd("terminal", "Terminal", "x-terminal-emulator", &[]),
            cmd("files", "Files", "xdg-open", &["$WORKTREE"]),
        ]
    }
}

/// 读取配置；文件不存在时先写出默认预设再返回。
///
/// 解析失败**不写文件**：用户手写 JSON 漏个逗号是常事，
/// 拿默认预设去覆盖等于把他攒的命令全删了。
pub fn load_from(path: &Path) -> Result<Vec<CustomCommand>> {
    if !path.exists() {
        let defaults = default_commands();
        write_defaults(path, &defaults)?;
        return Ok(defaults);
    }
    let text = std::fs::read_to_string(path)?;
    let parsed: ConfigFile = serde_json::from_str(&text).map_err(|e| {
        GitError::new(
            "custom_config",
            format!("customCommands.json 解析失败：{e}"),
        )
    })?;
    Ok(parsed.commands)
}

fn write_defaults(path: &Path, commands: &[CustomCommand]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = ConfigFile {
        version: 1,
        commands: commands.to_vec(),
    };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|e| GitError::new("custom_config", e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}
```

在 `src-tauri/src/engine/mod.rs` 里按字母序加一行：

```rust
pub mod custom_config;
```

（放在 `pub mod branch;` 之后、`pub mod diff;` 之前。）

- [ ] **Step 4: 跑测试确认通过**

```bash
cd src-tauri && cargo test --test custom_commands
```

预期：7 个测试全部 PASS。

若 `ConfigFile.version` 被 clippy 报「字段从未读取」，**不要**加 `#[allow(dead_code)]`：
改成在 `load_from` 里用它做一次显式的向前兼容判断，或直接给字段加 `#[allow(dead_code)]`
并在注释里写明「保留给将来的迁移」。二选一，把选择记进决策日志。

- [ ] **Step 5: 格式与 clippy**

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets
```

预期：无输出、无告警。`cargo fmt --check` 失败就先跑 `cargo fmt`。

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/engine/custom_config.rs src-tauri/src/engine/mod.rs src-tauri/tests/custom_commands.rs
git commit -m "feat(custom-commands): 自定义命令配置的解析与平台默认预设

- What: 新增 engine::custom_config，读写 customCommands.json
- Why: 工具栏自定义命令需要一份用户可手写的配置
- Impact/Test: cargo test --test custom_commands，7 用例通过；解析失败不改写用户文件"
```

- [ ] **Step 7: 更新决策日志**

把 Task 1 一行填进进度总账，并记录 `version` 字段那个 clippy 取舍。

---

### Task 2: launcher —— 程序解析与 spawn

**Files:**
- Create: `src-tauri/src/engine/launcher.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/custom_commands.rs`（追加）

**Interfaces:**
- Consumes: `crate::error::{GitError, Result}`
- Produces:
  - `pub fn resolve_program(program: &str) -> Result<PathBuf>`
  - `pub fn spawn_detached(program: &str, args: &[String], cwd: Option<&str>) -> Result<()>`
  - `pub fn open_in_default_app(path: &str) -> Result<()>`

- [ ] **Step 1: 先验证那个关于 Rust std 的假设**

这是本计划里唯一依赖「记忆中的行为」的地方，必须实测（AGENTS.md §6.2）。
往 `src-tauri/tests/custom_commands.rs` 追加：

```rust
/// 验证一个**关于 Rust std 的**假设，而不是验证我们自己的代码：
/// Rust 1.77.2 起（CVE-2024-24576 / BatBadBut 的修复），`Command` 对 `.bat`/`.cmd`
/// 目标会自动经 cmd.exe 启动并正确转义参数。VS Code 的 `code` 就是 `code.cmd`，
/// 而仓库路径里出现空格和 `&` 是常态——这条不成立的话，整个 launcher 要换写法。
#[test]
#[cfg(windows)]
fn rust_std_escapes_arguments_for_batch_files() {
    use std::process::Command;

    let dir = TempDir::new().unwrap();
    let probe = dir.path().join("probe.cmd");
    // %~1 去掉外层引号后再写出；展开发生在解析之后，所以 & 不会被当成命令分隔符
    fs::write(&probe, "@echo off\r\n>\"%~dp0out.txt\" echo.%~1\r\n").unwrap();

    let status = Command::new(&probe).arg("a b&c").status().unwrap();
    assert!(status.success(), "batch 文件没能启动");

    let out = fs::read_to_string(dir.path().join("out.txt")).unwrap();
    assert_eq!(out.trim(), "a b&c", "参数被 cmd 二次解析了");
}
```

- [ ] **Step 2: 跑这条测试，记录真实结果**

```bash
cd src-tauri && cargo test --test custom_commands rust_std_escapes -- --nocapture
```

- **PASS** → 假设成立，Step 3 按原样实现。
- **FAIL** → 假设不成立。改为在 `resolve_program` 返回 `.bat` / `.cmd` 时，
  由 `spawn_detached` 显式构造 `cmd.exe /c <resolved> <args...>`，
  并把上面这条测试改成针对 `spawn_detached` 的断言。

无论哪种结果，**都要把实测结论写进决策日志的「计划没写的坑」**。

- [ ] **Step 3: 写 launcher 的失败测试**

继续追加到 `src-tauri/tests/custom_commands.rs`：

```rust
use app_lib::engine::launcher;

/// Windows 上 `Command::new("git")` 靠 CreateProcess 查 PATH，只补 .exe。
/// 裸名字必须自己按 PATHEXT 解析，否则 `code`（实为 code.cmd）永远找不到。
#[test]
#[cfg(windows)]
fn resolve_program_finds_a_bare_name_on_path() {
    let resolved = launcher::resolve_program("git").unwrap();
    assert!(resolved.is_file(), "解析结果不是个文件：{resolved:?}");
    let name = resolved.file_name().unwrap().to_string_lossy().to_lowercase();
    assert_eq!(name, "git.exe");
}

/// 已经带扩展名或带路径分隔符的，原样交给系统，不要多此一举去搜 PATH。
#[test]
#[cfg(windows)]
fn resolve_program_passes_through_explicit_paths() {
    assert_eq!(
        launcher::resolve_program(r"C:\tools\my app\thing.exe").unwrap(),
        std::path::PathBuf::from(r"C:\tools\my app\thing.exe"),
    );
}

#[test]
fn resolve_program_reports_a_missing_executable() {
    let err = launcher::resolve_program("definitely-not-a-real-program-xyz").unwrap_err();
    assert_eq!(err.code, "launcher");
    // 报错必须直接告诉用户去哪改
    assert!(
        err.message.contains("customCommands.json"),
        "报错没指向配置文件：{}",
        err.message
    );
}

/// argv 与 cwd 都要真的生效：在临时目录里跑 `git init --quiet probe`，
/// 然后等它把 probe/.git 建出来。既证明参数没被 shell 吃掉，也证明 cwd 用对了。
#[test]
fn spawn_detached_runs_with_the_given_args_and_cwd() {
    let dir = TempDir::new().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();

    launcher::spawn_detached(
        "git",
        &["init".to_string(), "--quiet".to_string(), "probe".to_string()],
        Some(&cwd),
    )
    .unwrap();

    // 不等待子进程退出是这个函数的设计要点，所以这里只能轮询
    let marker = dir.path().join("probe").join(".git");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(marker.exists(), "子进程没有在 cwd 里跑起来");
}

#[test]
fn spawn_detached_reports_a_missing_executable() {
    let err = launcher::spawn_detached("definitely-not-a-real-program-xyz", &[], None).unwrap_err();
    assert_eq!(err.code, "launcher");
    assert!(err.message.contains("customCommands.json"));
}
```

`resolve_program_reports_a_missing_executable` 在非 Windows 上会失败——那边
`resolve_program` 不做查找。改成给它加 `#[cfg(windows)]`，并在非 Windows 上
由 `spawn_detached_reports_a_missing_executable` 覆盖同一条错误路径。

- [ ] **Step 4: 跑测试确认失败**

```bash
cd src-tauri && cargo test --test custom_commands
```

预期：编译失败，`could not find launcher in engine`。

- [ ] **Step 5: 实现 `launcher.rs`**

创建 `src-tauri/src/engine/launcher.rs`：

```rust
use crate::error::{GitError, Result};
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn not_found(program: &str) -> GitError {
    GitError::new(
        "launcher",
        format!("找不到可执行文件 `{program}`，请检查 customCommands.json"),
    )
}

/// 把裸程序名解析成实际存在的可执行文件路径。
///
/// Windows 专属：CreateProcess 查 PATH 时只补 `.exe`，而 VS Code 装的是
/// `code.cmd`——直接 `Command::new("code")` 必然 NotFound。这里按 PATHEXT
/// 自己搜一遍，拿到带扩展名的路径再交给 std。
#[cfg(windows)]
pub fn resolve_program(program: &str) -> Result<PathBuf> {
    let path = std::path::Path::new(program);
    if path.extension().is_some() || program.contains('\\') || program.contains('/') {
        return Ok(PathBuf::from(program));
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let dirs = std::env::var_os("PATH").ok_or_else(|| not_found(program))?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(not_found(program))
}

/// 非 Windows 平台交给系统自己查 PATH，找不到时由 spawn 报 NotFound。
#[cfg(not(windows))]
pub fn resolve_program(program: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(program))
}

/// 启动一个外部程序并立即返回。
///
/// 三个要点：
/// 1. 只 `spawn` 不 `wait`——等编辑器退出会把整个 UI 卡死。
/// 2. 标准流全部接到 null，不让子进程继承 GUI 进程的句柄。
/// 3. args 是数组，直传给 CreateProcess/execvp，**不经过 shell**。
///    仓库路径里的空格、`&`、中文因此不会被解析成新命令。
pub fn spawn_detached(program: &str, args: &[String], cwd: Option<&str>) -> Result<()> {
    let resolved = resolve_program(program)?;
    let mut command = Command::new(&resolved);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => not_found(program),
        _ => GitError::new("launcher", format!("启动 `{program}` 失败：{e}")),
    })?;

    // Unix 上不 wait 会留下僵尸进程。丢给一个后台线程去收尸：
    // 它只是阻塞在 wait 上，不占 CPU，也不挡住调用方。
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// 用系统默认程序打开一个路径。
///
/// Windows 上 `explorer.exe <file>` 会走默认关联程序打开文件；它的退出码
/// 常年非 0，但我们本来就不看退出码。这样就不必为「打开文件」引入新插件。
pub fn open_in_default_app(path: &str) -> Result<()> {
    let args = [path.to_string()];
    #[cfg(windows)]
    {
        spawn_detached("explorer.exe", &args, None)
    }
    #[cfg(target_os = "macos")]
    {
        spawn_detached("open", &args, None)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        spawn_detached("xdg-open", &args, None)
    }
}
```

在 `src-tauri/src/engine/mod.rs` 里按字母序加：

```rust
pub mod launcher;
```

（放在 `pub mod graph;` 之后、`pub mod path;` 之前。）

- [ ] **Step 6: 跑测试确认通过**

```bash
cd src-tauri && cargo test --test custom_commands
```

预期：全部 PASS。

- [ ] **Step 7: 格式与 clippy**

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets
```

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/engine/launcher.rs src-tauri/src/engine/mod.rs src-tauri/tests/custom_commands.rs
git commit -m "feat(custom-commands): 外部程序启动器

- What: 新增 engine::launcher，按 PATHEXT 解析程序名并 spawn，不经 shell
- Why: Windows 上 code 实为 code.cmd，直接 Command::new 必然 NotFound
- Impact/Test: cargo test --test custom_commands 通过；实测确认 std 对 .cmd 的转义行为"
```

- [ ] **Step 9: 更新决策日志**

必须写清 Step 2 的实测结论（PASS 还是 FAIL、最终采用哪种写法），
以及「不 wait 但起线程收尸」这个选择为什么是安全的。

---

### Task 3: Tauri 命令层

**Files:**
- Create: `src-tauri/src/commands/custom.rs`
- Modify: `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `engine::custom_config::{load_from, CustomCommand}`、`engine::launcher::{spawn_detached, open_in_default_app}`
- Produces（前端据此 invoke，参数名转成 camelCase 后无变化，均为单词）：
  - `read_custom_commands() -> CustomCommand[]`
  - `run_custom_command({ program: string, args: string[], cwd: string | null }) -> void`
  - `open_custom_commands_config() -> void`

- [ ] **Step 1: 实现 `commands/custom.rs`**

这一层是薄封装，逻辑都在 Task 1/2 里测过了，不再单独写测试；
真正的验证是 Step 5 的 `pnpm tauri dev` 实跑。

```rust
use crate::engine::{custom_config, launcher};
use crate::error::{GitError, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 配置文件路径：`app_config_dir()/customCommands.json`。
///
/// `app_config_dir()` 解析成 `config_dir()/${identifier}`，
/// Windows 上即 `%APPDATA%\com.momadacoding.jkk\`。
fn config_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| GitError::new("custom_config", format!("无法定位配置目录：{e}")))?;
    Ok(dir.join("customCommands.json"))
}

#[tauri::command]
pub fn read_custom_commands(app: AppHandle) -> Result<Vec<custom_config::CustomCommand>> {
    custom_config::load_from(&config_path(&app)?)
}

/// 按前端算好的 argv 启动外部程序。
///
/// 后端不认识占位符：展开在 `src/engine/customCommands.ts` 里做完了。
#[tauri::command]
pub fn run_custom_command(program: String, args: Vec<String>, cwd: Option<String>) -> Result<()> {
    launcher::spawn_detached(&program, &args, cwd.as_deref())
}

/// 用系统默认程序打开配置文件。
///
/// 故意不接受路径参数：先自己解析位置、确保文件存在（缺失时写默认预设），
/// 前端因此没有能力让系统去打开一个它自己拼出来的路径。
#[tauri::command]
pub fn open_custom_commands_config(app: AppHandle) -> Result<()> {
    let path = config_path(&app)?;
    custom_config::load_from(&path)?;
    let text = path
        .to_str()
        .ok_or_else(|| GitError::new("custom_config", "配置路径不是合法的 UTF-8"))?;
    launcher::open_in_default_app(text)
}
```

- [ ] **Step 2: 注册模块与命令**

`src-tauri/src/commands/mod.rs` 里按字母序加（`pub mod graph;` 之后）：

```rust
pub mod custom;
```

`src-tauri/src/lib.rs` 的 `generate_handler!` 列表里按字母序加三行
（`commands::diff::read_status,` 之后、`commands::graph::load_graph,` 之前）：

```rust
        commands::custom::open_custom_commands_config,
        commands::custom::read_custom_commands,
        commands::custom::run_custom_command,
```

- [ ] **Step 3: 编译并跑全量 Rust 检查**

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test
```

预期：编译通过，全部测试 PASS，无 clippy 告警。

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/commands/custom.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(custom-commands): 暴露三个 Tauri 命令

- What: read_custom_commands / run_custom_command / open_custom_commands_config
- Why: 前端需要读配置、启动程序、打开配置文件三个入口
- Impact/Test: cargo test 全绿；invoke 连通性留到 Task 6 在真实窗口验证"
```

- [ ] **Step 5: 更新决策日志**

---

### Task 4: 前端纯逻辑 —— 占位符展开

**Files:**
- Create: `src/engine/customCommands.ts`
- Test: `tests/vitest/customCommands.test.ts`

**Interfaces:**
- Consumes: 无（纯函数，不依赖 React / Tauri）
- Produces:
  - `export interface CustomCommand { id: string; label: string; program: string; args: string[]; cwd: string }`
  - `export interface CommandContext { worktree: string | null; branch: string; headHash: string }`
  - `export interface CustomCommandPlan { program: string; args: string[]; cwd: string | null }`
  - `export function expandPlaceholders(text: string, ctx: CommandContext): string`
  - `export function planCustomCommand(cmd: CustomCommand, ctx: CommandContext): CustomCommandPlan | null`

- [ ] **Step 1: 写失败的测试**

创建 `tests/vitest/customCommands.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import {
  expandPlaceholders,
  planCustomCommand,
  type CommandContext,
  type CustomCommand,
} from '../../src/engine/customCommands'

const ctx: CommandContext = {
  worktree: 'C:\\Users\\me\\dev\\my repo',
  branch: 'feat/toolbar',
  headHash: '0a6c1863f2b1c4d5e6f708192a3b4c5d6e7f8091',
}

const vscode: CustomCommand = {
  id: 'vscode',
  label: 'VS Code',
  program: 'code',
  args: ['$WORKTREE'],
  cwd: '$WORKTREE',
}

describe('expandPlaceholders', () => {
  it('Windows 反斜杠路径原样代入，不做任何转义', () => {
    expect(expandPlaceholders('$WORKTREE', ctx)).toBe('C:\\Users\\me\\dev\\my repo')
  })

  it('作为子串出现时也要替换', () => {
    expect(expandPlaceholders('$WORKTREE/src', ctx)).toBe('C:\\Users\\me\\dev\\my repo/src')
  })

  it('替换 $BRANCH 与 $HEAD', () => {
    expect(expandPlaceholders('$BRANCH@$HEAD', ctx)).toBe(`${ctx.branch}@${ctx.headHash}`)
  })

  it('路径里的 $& $1 是字面量，不能被当成替换模式', () => {
    // String.replace 的字符串替换会把 $& 解释成「整个匹配」，
    // 必须用函数形式的 replace 才不会被反噬
    const weird: CommandContext = { ...ctx, worktree: 'C:\\repo$&$1x' }
    expect(expandPlaceholders('$WORKTREE', weird)).toBe('C:\\repo$&$1x')
  })

  it('替换结果里的占位符不会被二次展开', () => {
    const nested: CommandContext = { ...ctx, worktree: 'C:\\$BRANCH' }
    expect(expandPlaceholders('$WORKTREE', nested)).toBe('C:\\$BRANCH')
  })

  it('未知占位符原样保留，不变成空串', () => {
    expect(expandPlaceholders('$FOO', ctx)).toBe('$FOO')
  })

  it('$HEADER 不能被当成 $HEAD 加 ER', () => {
    expect(expandPlaceholders('$HEADER', ctx)).toBe('$HEADER')
  })
})

describe('planCustomCommand', () => {
  it('展开 args 与 cwd', () => {
    const plan = planCustomCommand(vscode, ctx)
    expect(plan).toEqual({
      program: 'code',
      args: ['C:\\Users\\me\\dev\\my repo'],
      cwd: 'C:\\Users\\me\\dev\\my repo',
    })
  })

  it('program 本身也支持占位符', () => {
    const plan = planCustomCommand({ ...vscode, program: '$WORKTREE\\run.exe' }, ctx)
    expect(plan?.program).toBe('C:\\Users\\me\\dev\\my repo\\run.exe')
  })

  it('空 args 保持为空数组，不能凭空补出参数', () => {
    // 空参数列表在本项目造成过误伤，这里显式钉住
    expect(planCustomCommand({ ...vscode, args: [] }, ctx)?.args).toEqual([])
  })

  it('cwd 为空串时传 null，让子进程继承默认工作目录', () => {
    expect(planCustomCommand({ ...vscode, cwd: '' }, ctx)?.cwd).toBeNull()
  })

  it('没打开仓库时返回 null —— 按钮据此禁用', () => {
    const empty: CommandContext = { worktree: null, branch: '', headHash: '' }
    expect(planCustomCommand(vscode, empty)).toBeNull()
  })
})
```

- [ ] **Step 2: 跑测试确认失败**

```bash
pnpm test
```

预期：FAIL，`Failed to resolve import '../../src/engine/customCommands'`。

- [ ] **Step 3: 实现 `src/engine/customCommands.ts`**

```ts
/** 一条自定义命令，与 Rust 侧 `CustomCommand` 一一对应。 */
export interface CustomCommand {
  id: string
  label: string
  program: string
  args: string[]
  cwd: string
}

/** 占位符的取值来源，由当前打开的仓库决定。 */
export interface CommandContext {
  /** 当前 worktree 绝对路径；未打开仓库时为 null。 */
  worktree: string | null
  branch: string
  headHash: string
}

/** 展开后可以直接交给后端的一次调用。 */
export interface CustomCommandPlan {
  program: string
  args: string[]
  cwd: string | null
}

/**
 * `\b` 是必须的：没有它，`$HEADER` 会被吃掉前半截变成 `<hash>ER`。
 * 未列出的名字不匹配，因而原样保留。
 */
const PLACEHOLDER = /\$(WORKTREE|BRANCH|HEAD)\b/g

/**
 * 把一段文本里的占位符替换成当前仓库的值。
 *
 * 用函数形式的 replace 而不是字符串形式：字符串替换里 `$&`、`$1` 有特殊含义，
 * 而仓库路径是用户给的、什么字符都可能有。同时整段只扫一遍，
 * 替换进去的值不会被再次展开。
 */
export function expandPlaceholders(text: string, ctx: CommandContext): string {
  return text.replace(PLACEHOLDER, (match, name: string) => {
    switch (name) {
      case 'WORKTREE':
        return ctx.worktree ?? match
      case 'BRANCH':
        return ctx.branch
      case 'HEAD':
        return ctx.headHash
      default:
        return match
    }
  })
}

/**
 * 把一条配置翻译成一次具体的后端调用。
 *
 * 没打开仓库时返回 null——所有默认命令都要用 `$WORKTREE`，
 * 拿不到路径就不该让按钮可点。
 */
export function planCustomCommand(
  cmd: CustomCommand,
  ctx: CommandContext,
): CustomCommandPlan | null {
  if (ctx.worktree === null) return null
  const cwd = expandPlaceholders(cmd.cwd, ctx)
  return {
    program: expandPlaceholders(cmd.program, ctx),
    args: cmd.args.map((arg) => expandPlaceholders(arg, ctx)),
    // 空串会被 Rust 当成一个真实目录去 chdir，必然失败；显式传 null 表示「不设」
    cwd: cwd === '' ? null : cwd,
  }
}
```

- [ ] **Step 4: 跑测试确认通过**

```bash
pnpm test
```

预期：`customCommands.test.ts` 12 个用例全部 PASS，其余既有测试不受影响。

- [ ] **Step 5: 类型检查**

```bash
pnpm build
```

预期：`tsc` 无错误。注意 `tsconfig.json` 的 `include` 只有 `src`，
**测试文件的类型错误只会在 `pnpm test` 里暴露**，不要拿 `pnpm build` 通过来推断测试类型正确。

- [ ] **Step 6: Commit**

```bash
git add src/engine/customCommands.ts tests/vitest/customCommands.test.ts
git commit -m "feat(custom-commands): 占位符展开的纯逻辑与单测

- What: src/engine/customCommands.ts，展开 \$WORKTREE / \$BRANCH / \$HEAD
- Why: 展开逻辑放前端纯函数才能单测；后端只管 spawn
- Impact/Test: pnpm test 通过，覆盖 Windows 路径、\$& 字面量、未知占位符、空 args"
```

- [ ] **Step 7: 更新决策日志**

---

### Task 5: 前端接线与工具栏 UI

**Files:**
- Create: `src/store/customCommandsStore.ts`, `src/components/CustomCommandBar.tsx`
- Modify: `src/components/Toolbar.tsx`, `src/styles.css`

**Interfaces:**
- Consumes: `planCustomCommand`、`CustomCommand`、`CommandContext`（Task 4）；三个 Tauri 命令（Task 3）；`ContextMenu` / `useContextMenu`（既有）
- Produces:
  - `export function useCustomCommands(): { commands: CustomCommand[]; error: string | null; reload: () => Promise<void>; run: (plan: CustomCommandPlan) => Promise<void>; openConfig: () => Promise<void> }`
  - `export function CustomCommandBar({ context }: { context: CommandContext }): JSX.Element`

- [ ] **Step 1: 写 store**

创建 `src/store/customCommandsStore.ts`。自定义命令是**全局**配置而不是仓库状态，
所以单独一个 hook，不塞进 `repoStore`：

```ts
import { invoke } from '@tauri-apps/api/core'
import { useCallback, useEffect, useState } from 'react'
import type { CustomCommand, CustomCommandPlan } from '../engine/customCommands'
import { toErrorMessage } from '../engine/errors'

/**
 * 读取并执行用户自定义命令。
 *
 * 配置改动不做文件监听：多开标签时每个实例都监听同一个文件、
 * 又要和既有的 `repo-changed` 监听抢重入，不值得。用户改完点 Reload 即可。
 */
export function useCustomCommands() {
  const [commands, setCommands] = useState<CustomCommand[]>([])
  const [error, setError] = useState<string | null>(null)

  const reload = useCallback(async () => {
    try {
      const list = await invoke<CustomCommand[]>('read_custom_commands')
      setCommands(list)
      setError(null)
    } catch (err) {
      // 配置坏了就退化成没有命令，但把原因留在界面上
      setCommands([])
      setError(toErrorMessage(err))
    }
  }, [])

  useEffect(() => {
    void reload()
  }, [reload])

  const run = useCallback(async (plan: CustomCommandPlan) => {
    await invoke('run_custom_command', {
      program: plan.program,
      args: plan.args,
      cwd: plan.cwd,
    })
  }, [])

  const openConfig = useCallback(async () => {
    await invoke('open_custom_commands_config')
  }, [])

  return { commands, error, reload, run, openConfig }
}
```

- [ ] **Step 2: 写 CustomCommandBar 组件**

创建 `src/components/CustomCommandBar.tsx`：

```tsx
import { useCallback, useState } from 'react'
import { planCustomCommand, type CommandContext } from '../engine/customCommands'
import { toErrorMessage } from '../engine/errors'
import { useCustomCommands } from '../store/customCommandsStore'
import { ContextMenu, useContextMenu } from './ContextMenu'

/**
 * 工具栏右侧的自定义命令组。
 *
 * 命令是启动外部程序，毫秒级返回，所以不做 busy 态——那是留给 fetch/push 的。
 */
export function CustomCommandBar({ context }: { context: CommandContext }) {
  const { commands, error, reload, run, openConfig } = useCustomCommands()
  const { menu, open, close } = useContextMenu()
  const [failure, setFailure] = useState<string | null>(null)

  const launch = useCallback(
    async (id: string) => {
      const cmd = commands.find((c) => c.id === id)
      if (!cmd) return
      const plan = planCustomCommand(cmd, context)
      if (!plan) return
      try {
        setFailure(null)
        await run(plan)
      } catch (err) {
        setFailure(toErrorMessage(err))
      }
    },
    [commands, context, run],
  )

  const openMenu = (e: React.MouseEvent) => {
    open(e, [
      { label: 'Edit commands…', onSelect: () => void openConfig() },
      { label: 'Reload', onSelect: () => void reload() },
    ])
  }

  return (
    <>
      {commands.map((cmd, i) => (
        <button
          key={`${cmd.id}-${i}`}
          className="toolbar-button"
          disabled={context.worktree === null}
          onClick={() => void launch(cmd.id)}
          data-command={cmd.id}
          title={`${cmd.program} ${cmd.args.join(' ')}`}
        >
          {cmd.label}
        </button>
      ))}
      <button
        className="toolbar-button"
        onClick={openMenu}
        title={error ?? failure ?? '自定义命令'}
        data-command="settings"
      >
        {error || failure ? '⚠' : '⚙'}
      </button>
      <ContextMenu menu={menu} onClose={close} />
    </>
  )
}
```

两个要点：`id` 允许重复（Task 1 已确认不报错），所以 key 用 `${cmd.id}-${i}`；
`⚙` **不跟随 `worktree === null` 禁用**——编辑配置不需要先打开仓库。

- [ ] **Step 3: 接进 Toolbar**

`src/components/Toolbar.tsx` 顶部补两行 import：

```tsx
import type { CommandContext } from '../engine/customCommands'
import { CustomCommandBar } from './CustomCommandBar'
```

给 `ToolbarProps` 加一个字段，在按钮循环之后插入 spacer 和命令组。

```tsx
interface ToolbarProps {
  branch: string
  upstream: UpstreamInfo | null
  onRun: (command: string, args: Record<string, unknown>) => Promise<void>
  /** 占位符取值来源；未打开仓库时 worktree 为 null。 */
  commandContext: CommandContext
}
```

在 `</div>` 之前、`ACTIONS.map(...)` 之后加：

```tsx
      <span className="toolbar-spacer" />
      <CustomCommandBar context={commandContext} />
```

`src/components/RepoTab.tsx:407` 的调用点补上新 prop：

```tsx
      <Toolbar
        branch={state.head}
        upstream={state.upstream}
        onRun={runToolbar}
        commandContext={{ worktree: state.path, branch: state.head, headHash: state.headHash }}
      />
```

- [ ] **Step 4: 加 CSS**

`src/styles.css` 里 `.toolbar-badge` 规则之后追加：

```css
/* 把自定义命令组推到工具栏右端 */
.toolbar-spacer {
  flex: 1;
  min-width: 12px;
}
```

- [ ] **Step 5: 类型检查与测试**

```bash
pnpm build && pnpm test
```

预期：`tsc` 无错误，vitest 全绿。`noUnusedLocals` 会抓出漏掉的 import。

- [ ] **Step 6: Commit**

```bash
git add src/store/customCommandsStore.ts src/components/CustomCommandBar.tsx src/components/Toolbar.tsx src/components/RepoTab.tsx src/styles.css
git commit -m "feat(toolbar): 工具栏右侧的自定义命令组

- What: 新增 useCustomCommands 与 CustomCommandBar，接到 Toolbar 右端
- Why: 让用户一键在编辑器/终端/文件管理器里打开当前 worktree
- Impact/Test: pnpm build + pnpm test 通过；真实窗口验证在下一 Task"
```

- [ ] **Step 7: 更新决策日志**

---

### Task 6: 真实窗口验证与文档

**Files:**
- Modify: `README.md`
- Create: `docs/superpowers/e2e/2026-08-09-custom-toolbar-commands.md`

- [ ] **Step 1: 启动真实窗口**

```bash
pnpm tauri dev
```

浏览器打开 Vite dev server **不等价**：三个命令全都走 `invoke`，
必须在 Tauri 窗口里验证（AGENTS.md §7.3）。

- [ ] **Step 2: 逐条走 E2E 清单**

在真实窗口里确认：

1. 首次启动后 `%APPDATA%\com.momadacoding.jkk\customCommands.json` 被创建，内容是三条默认预设
2. 未打开仓库时三个按钮置灰，`⚙` 仍可点
3. 打开一个仓库后按 Explorer → 文件管理器定位到该 worktree
4. 按 Terminal → 终端在该 worktree 目录里打开
5. 按 VS Code → 编辑器打开该 worktree（若本机没装，确认报错横幅是
   「找不到可执行文件 `code`，请检查 customCommands.json」）
6. `⚙` → Edit commands… → 配置文件被默认程序打开
7. 改配置（加一条命令）→ `⚙` → Reload → 新按钮出现
8. 把配置改成非法 JSON → Reload → `⚙` 变成 `⚠`，按钮组清空，**文件内容未被改写**
9. **路径含空格与非 ASCII 字符的仓库**：另建一个名为 `我的 repo & test` 的
   fixture 仓库并打开，确认三个命令都能正确定位到它

第 9 条是本功能最关键的回归点：argv 直传的意义就在这里。
fixture 仓库用 `git init` 新建在临时目录，**不要拿用户的真实仓库试**。

- [ ] **Step 3: 把清单落成文档**

创建 `docs/superpowers/e2e/2026-08-09-custom-toolbar-commands.md`，
写入 Step 2 的 9 条以及每条的实际结果（通过 / 未通过 / 未验证及原因）。

- [ ] **Step 4: 更新 README**

README 是对外主索引，新增了用户可见的配置文件就必须同步（AGENTS.md §9.3）。
追加这一节（位置：现有功能说明之后、开发相关章节之前）：

````markdown
## 工具栏自定义命令

工具栏右侧可以配置任意本地命令，用来把当前 worktree 交给外部程序，
比如在编辑器里打开、在该目录起一个终端、或者在文件管理器里定位。

配置文件位置：

| 平台 | 路径 |
|---|---|
| Windows | `%APPDATA%\com.momadacoding.jkk\customCommands.json` |
| macOS | `~/Library/Application Support/com.momadacoding.jkk/customCommands.json` |
| Linux | `~/.config/com.momadacoding.jkk/customCommands.json` |

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
````

- [ ] **Step 5: 跑一遍完整的提交前门槛**

```bash
pnpm build && pnpm test
cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test
```

五条全部必须通过。**不得声称「lint 通过」或「CI 通过」**——本仓库两者都没有。

- [ ] **Step 6: Commit**

```bash
git add README.md docs/superpowers/e2e/2026-08-09-custom-toolbar-commands.md docs/superpowers/plans/2026-08-09-custom-toolbar-commands-decisions.md
git commit -m "docs(custom-commands): E2E 清单与 README 说明

- What: 记录 9 条真实窗口验证结果，README 补自定义命令配置说明
- Why: 用户可见的配置文件必须在对外文档里有说明
- Impact/Test: pnpm build/test + cargo fmt/clippy/test 全部通过"
```

- [ ] **Step 7: 独立 review**

按 AGENTS.md §4.1，合并前必须由**独立于实现者**的人或 agent 完成一次 review。
用 `/code-review` 或独立 review subagent，按 §13 rubric 执行。
review 先读决策日志的「偏离计划」条目，再看 diff。高危问题合并前必须修完。

- [ ] **Step 8: 开 Draft PR**

```bash
gh pr create --draft --base master --title "feat(toolbar): 工具栏自定义命令" --body "$(cat <<'EOF'
## Summary

- 工具栏右侧新增一组用户自定义命令，用于把当前 worktree 交给外部程序
  （编辑器 / 终端 / 文件管理器）
- 配置是 app config dir 下的 `customCommands.json`，首次启动写出平台默认预设
- 占位符 `$WORKTREE` / `$BRANCH` / `$HEAD` 在前端纯函数里展开，Rust 侧只负责 spawn
- 全程 argv 数组直传，不经 shell 解释
- 未引入任何新依赖

## Test plan

- [x] `cargo test`（含配置解析 7 例 + launcher 5 例，全部用 tempfile fixture）
- [x] `cargo fmt --check`、`cargo clippy --all-targets`
- [x] `pnpm build`、`pnpm test`
- [x] `pnpm tauri dev` 真实窗口走完 9 条 E2E 清单，
      见 `docs/superpowers/e2e/2026-08-09-custom-toolbar-commands.md`
- [ ] Linux 的 `x-terminal-emulator` 预设未验证：无 Linux 环境

## Review 入口

先读 `docs/superpowers/plans/2026-08-09-custom-toolbar-commands-decisions.md`
的「偏离计划」条目，再看 diff。
EOF
)"
```

默认 Draft。用户明确要求正式 PR 时才省略 `--draft`。
Test plan 里的勾选状态**按实际结果填**，没跑的留空并写明原因。

---

## 附：本计划的已知薄弱点

留给实现者和 reviewer 特别留意：

1. **`.cmd` 转义的假设**（Task 2 Step 2）。这是唯一一条靠记忆写下的行为，
   实测结果若与预期不符，Task 2 的实现要换写法。**不要跳过这一步直接写实现。**
2. **Linux 的 `x-terminal-emulator` 预设没有真机验证条件**。
   本项目主要在 Windows 上开发，Linux 那条预设属于「按惯例填的默认值」，
   在决策日志里如实标注为未验证。
3. **`CustomCommandBar` 在每个仓库标签里各自读一次配置。** 多标签时是重复 IO，
   但配置文件很小、只在挂载和 Reload 时读，暂不优化。若将来标签数量变大再提到全局。
