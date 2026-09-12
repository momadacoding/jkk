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
