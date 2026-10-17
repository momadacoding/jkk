//! 编译期从 git 取「这次构建对应哪个提交」，写进窗口标题。
//!
//! 只给 build.rs 用（`tests/build_info.rs` 按路径引入它做 fixture 测试），
//! 不进 lib：运行期的 git 调用一律走 `engine::git_cli::run_git`，这里是构建机上的
//! 一次性调用，不经过 GUI 进程，也就没有 console 黑框的问题。

use std::path::{Path, PathBuf};
use std::process::Command;

/// `git describe --tags --always --match v[0-9]*`。
///
/// - `--tags`：本仓库的版本 tag 是 lightweight，不加取不到
/// - `--match`：只认版本号 tag，别的 tag 再近也不能顶替
/// - `--always`：一个版本 tag 都没有时退化成短 hash，而不是失败
/// - 不加 `--dirty`：build.rs 只在 HEAD/refs 变化时重跑，盯不住工作区改动，
///   带上它会在改完代码后仍显示旧的「干净」或旧的「dirty」，不如不报
///
/// 取不到（没装 git、不在仓库里）返回 `None`。
pub fn git_describe(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["describe", "--tags", "--always", "--match", "v[0-9]*"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// describe 结果会随这些文件变化：当前 worktree 的 HEAD，以及共享的分支/tag 引用。
///
/// 只返回真实存在的路径——cargo 遇到不存在的 rerun-if-changed 路径会**每次**都重跑
/// build.rs，连带整个 app crate 重编。
pub fn watch_paths(dir: &Path) -> Vec<PathBuf> {
    let out = match Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "--git-dir", "--git-common-dir"])
        .output()
    {
        Ok(out) if out.status.success() => out,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    // 两者都可能是相对 dir 的路径；join 遇到绝对路径会直接取后者
    let (Some(git_dir), Some(common_dir)) = (lines.next(), lines.next()) else {
        return Vec::new();
    };
    let git_dir = dir.join(git_dir.trim());
    let common_dir = dir.join(common_dir.trim());

    let mut paths = vec![
        git_dir.join("HEAD"),
        common_dir.join("refs").join("tags"),
        common_dir.join("packed-refs"),
        // reftable 后端（git 2.45+）下 HEAD 和 refs/ 只是永不变化的兼容占位，
        // 引用全在 reftable/ 里；linked worktree 的 HEAD 在它自己 git-dir 的 reftable/ 里
        common_dir.join("reftable"),
        git_dir.join("reftable"),
    ];
    // 只盯当前分支的 ref 文件，不盯整个 refs/heads：同一仓库常有多个 worktree
    // 并行提交，别的分支一动就重编 app crate 不值得。
    // 分支 ref 被 pack 掉（gc 之后）时没有松散文件，下一次提交会**新建**它——
    // 盯不存在的文件等于每次重跑，只能退回盯目录（cargo 会递归看 mtime）。
    // detached HEAD 时提交号就写在 HEAD 里，不需要额外盯。
    if let Some(branch_ref) = symbolic_head(dir) {
        let loose = common_dir.join(&branch_ref);
        paths.push(if loose.is_file() {
            loose
        } else {
            common_dir.join("refs").join("heads")
        });
    }
    // 主 checkout 里 git-dir 与 common-dir 是同一个目录（从子目录调用时一个是绝对路径、
    // 一个是 `../.git`），reftable/ 会以两种写法各出现一次，按规范化后的路径去重
    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for p in paths {
        let Ok(canon) = std::fs::canonicalize(&p) else {
            continue; // 不存在
        };
        if !seen.contains(&canon) {
            seen.push(canon);
            out.push(p);
        }
    }
    out
}

/// `refs/heads/<branch>`；detached HEAD 或出错时为 `None`。
fn symbolic_head(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}
