use std::process::Command;
use crate::error::{GitError, Result};

pub use crate::engine::worktree::{read_worktrees, Worktree};

#[tauri::command]
pub fn read_worktrees_cmd(path: String) -> Result<Vec<Worktree>> {
    read_worktrees(&path)
}

#[tauri::command]
pub fn create_worktree(path: String, name: String, branch: Option<String>) -> Result<()> {
    let worktree_path = std::path::Path::new(&path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(&name);
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
pub fn delete_worktree(path: String, name: String) -> Result<()> {
    let worktree_path = std::path::Path::new(&path)
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join(&name);
    let status = Command::new("git")
        .current_dir(&path)
        .args(["worktree", "remove", worktree_path.to_str().unwrap()])
        .status()?;
    if !status.success() {
        return Err(GitError::new(
            "git_cli",
            "worktree remove failed".to_string(),
        ));
    }
    Ok(())
}
