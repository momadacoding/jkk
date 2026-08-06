use crate::engine::git_cli::run_git;
use crate::error::Result;

pub use crate::engine::worktree::{read_worktrees, Worktree};

#[tauri::command]
pub fn read_worktrees_cmd(path: String) -> Result<Vec<Worktree>> {
    read_worktrees(&path)
}

/// 主仓库的根目录。
///
/// `path` 可能是主仓库，也可能是任意一个 worktree。`--git-common-dir` 在两种情况下
/// 都指向主仓库的 .git，它的父目录就是主仓库根。新 worktree 统一建在那下面的
/// `.worktrees/`，否则从一个 worktree 里新建会建到仓库外面去。
fn main_repo(path: &str) -> Result<std::path::PathBuf> {
    let common = run_git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let git_dir = std::path::PathBuf::from(common.trim());
    Ok(git_dir.parent().unwrap_or(&git_dir).to_path_buf())
}

#[tauri::command]
pub fn create_worktree(path: String, name: String, branch: Option<String>) -> Result<()> {
    let worktree_path = main_repo(&path)?.join(".worktrees").join(&name);
    let target = worktree_path.to_string_lossy().to_string();

    let mut args: Vec<&str> = vec!["worktree", "add", &target];
    // 不带 -b 的话 git 会把它当成 commit-ish 检出成游离 HEAD
    if let Some(b) = &branch {
        args.push("-b");
        args.push(b);
    }
    run_git(&path, &args)?;
    Ok(())
}

/// 删除一个 worktree。
///
/// 收的是 worktree 的**完整路径**而不是名字：名字得靠 `parent(当前仓库)/名字` 拼，
/// 只有当前打开的恰好是它的兄弟 worktree 时才碰巧对；从主仓库删算出来的是仓库外面
/// 的路径。`Worktree` 结构体本来就带 path，直接用。
///
/// 失败时把 git 的原话带回去。`git worktree remove` 拒绝的理由很多（有未跟踪文件、
/// 目录被别的进程占着……），而且它**不是原子的**——先删文件、最后删目录，最后一步
/// 失败时前面删掉的东西和管理记录都回不来了。这种时候用户必须看到 git 到底说了什么。
#[tauri::command]
pub fn delete_worktree(path: String, worktree_path: String) -> Result<()> {
    run_git(&path, &["worktree", "remove", &worktree_path])?;
    Ok(())
}
