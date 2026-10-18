//! 启动外部 diff / merge 工具：占位符展开、临时目录、同一文件的互斥锁、等进程退出。

use crate::engine::launcher::resolve_program;
use crate::engine::path::normalize;
use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 展开参数数组里的占位符，`vars` 的名字不带 `$`。
///
/// 每个元素单趟扫描：替换进去的值（文件名、提交标题）原样落地，不会被再展开一次。
/// 同一位置能匹配多个名字时取最长的，`$LOCAL_TITLE` 才不会被读成 `$LOCAL` + `_TITLE`；
/// 名字后面紧跟字母数字下划线时不算匹配，`$LOCALX` 这类没列出的名字原样保留。
pub fn expand_template(args: &[String], vars: &[(&str, String)]) -> Vec<String> {
    let mut names: Vec<&(&str, String)> = vars.iter().collect();
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    args.iter().map(|arg| expand_one(arg, &names)).collect()
}

fn expand_one(arg: &str, names: &[&(&str, String)]) -> String {
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(pos) = rest.find('$') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        let hit = names.iter().find(|(name, _)| {
            after.starts_with(name)
                && !after[name.len()..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        match hit {
            Some((name, value)) => {
                out.push_str(value);
                rest = &after[name.len()..];
            }
            None => {
                out.push('$');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// 应用自己的临时根目录；每次调用外部工具在它下面建一个独立子目录。
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("jkk-external")
}

/// 进程号 + 纳秒时间戳 + 进程内计数：同一纳秒内连点两次也不会撞名，且不需要新依赖。
fn unique_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos}-{n}", std::process::id())
}

/// 一次外部工具调用专属的临时子目录。
///
/// 正常路径显式调用 `remove`；没调用就被丢掉时（导出失败、启动失败，回调闭包随之被丢弃）
/// 由 Drop 兜底删除，校验规则与 `remove` 相同。
#[derive(Debug)]
pub struct TempWorkspace {
    dir: PathBuf,
    removed: bool,
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.remove_checked();
        }
    }
}

impl TempWorkspace {
    pub fn create() -> Result<Self> {
        Self::create_in(&temp_root())
    }

    /// 在指定根目录下建子目录。只有测试需要换根目录；`remove` 仍按 `temp_root()` 校验，
    /// 所以建在别处的目录永远删不掉。
    pub fn create_in(root: &Path) -> Result<Self> {
        std::fs::create_dir_all(root)?;
        let dir = root.join(unique_name());
        // 用 create_dir 而不是 create_dir_all：目录已存在说明撞名，宁可报错也不和别人共用
        std::fs::create_dir(&dir)?;
        Ok(Self {
            dir,
            removed: false,
        })
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 删除整个子目录。
    ///
    /// 删之前确认它是 `temp_root()` 的**直接子目录**、且不是符号链接，
    /// 否则拒绝：这里删的是递归目录，路径算错一次就可能删到用户文件。
    pub fn remove(mut self) -> Result<()> {
        // 不管成败都不再让 Drop 重试：拒绝删除的目录重试一次也还是拒绝
        self.removed = true;
        self.remove_checked()
    }

    fn remove_checked(&self) -> Result<()> {
        let meta = match std::fs::symlink_metadata(&self.dir) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        let refuse = |why: &str| {
            GitError::new(
                "external_tool_cleanup",
                format!("拒绝删除临时目录 {}：{why}", self.dir.display()),
            )
        };
        if !meta.is_dir() {
            return Err(refuse("不是普通目录"));
        }
        let root = std::fs::canonicalize(temp_root()).map_err(|_| refuse("临时根目录不存在"))?;
        let dir = std::fs::canonicalize(&self.dir)?;
        if dir.parent() != Some(root.as_path()) {
            return Err(refuse("不在应用临时根目录内"));
        }
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}

/// key → 占着这个文件的工具名。
fn locks() -> &'static Mutex<HashMap<String, String>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_table() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
    // 锁表里只有字符串，持锁线程 panic 也不会留下半改的状态，直接接着用
    locks().lock().unwrap_or_else(|e| e.into_inner())
}

/// 占着「worktree + 文件」的凭据，Drop 时释放。
#[derive(Debug)]
pub struct ToolLockGuard {
    key: String,
}

impl Drop for ToolLockGuard {
    fn drop(&mut self) {
        lock_table().remove(&self.key);
    }
}

/// 占住「worktree + 文件」，所有外部 diff / merge 入口共用一把。
///
/// 要在导出临时文件**之前**调用：同一个冲突文件开两个 BC 各自保存，后存的会悄悄覆盖先存的。
pub fn try_lock(worktree: &str, file: &str, tool_label: &str) -> Result<ToolLockGuard> {
    let key = format!("{}\0{}", normalize(worktree), normalize(file));
    let mut table = lock_table();
    if let Some(holder) = table.get(&key) {
        return Err(GitError::new(
            "external_tool_busy",
            format!("{file} 已在 {holder} 中打开"),
        ));
    }
    table.insert(key.clone(), tool_label.to_string());
    Ok(ToolLockGuard { key })
}

/// 外部进程的结局。退出码只用于报告工具故障，**永远不用来判断冲突是否已解决**。
#[derive(Debug, Clone, PartialEq)]
pub struct ExitReport {
    /// 被信号杀掉（Unix）时没有退出码。
    pub code: Option<i32>,
    /// 等待进程本身失败时的原因。
    pub error: Option<String>,
}

/// 外部工具退出后发给前端的事件 `external-tool-exited` 的内容。
///
/// 带上 worktree，前端只刷新对应的仓库；多开标签时别的仓库不受影响。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalToolExited {
    pub worktree: String,
    pub file: String,
    pub tool_label: String,
    pub code: Option<i32>,
    pub error: Option<String>,
}

/// 启动外部工具，进程结束后在后台线程里回调 `on_exit`。
///
/// 启动失败（找不到程序、没有权限）同步返回 `launcher` 错误，`on_exit` 不会被调用，
/// 调用方据此当场释放锁和临时目录。参数作为数组交给 std，不经过 shell；
/// `.cmd` / `.bat` 由 std 交给 cmd.exe 并按 cmd 规则转义（见 `launcher::resolve_program`）。
pub fn spawn_watched(
    program: &str,
    args: &[String],
    on_exit: impl FnOnce(ExitReport) + Send + 'static,
) -> Result<()> {
    let not_found = || GitError::new("launcher", format!("找不到可执行文件 `{program}`"));
    let resolved = resolve_program(program).map_err(|_| not_found())?;
    let mut command = hidden_command(&resolved);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => not_found(),
        _ => GitError::new("launcher", format!("启动 `{program}` 失败：{e}")),
    })?;
    std::thread::spawn(move || {
        let report = match child.wait() {
            Ok(status) => ExitReport {
                code: status.code(),
                error: None,
            },
            Err(e) => ExitReport {
                code: None,
                error: Some(format!("等待外部工具退出失败：{e}")),
            },
        };
        on_exit(report);
    });
    Ok(())
}
