use std::process::Command;
use crate::error::{GitError, Result};
use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct Worktree {
    pub name: String,
    pub path: String,
    pub head: String,
    pub branch: String,
    pub is_main: bool,
}

pub fn read_worktrees(path: &str) -> Result<Vec<Worktree>> {
    let output = Command::new("git")
        .current_dir(path)
        .args(["worktree", "list", "--porcelain", "-z"])
        .output()?;
    if !output.status.success() {
        return Err(GitError::new(
            "git_cli",
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut worktrees = Vec::new();
    for block in stdout.split('\0') {
        let mut name = String::new();
        let mut path_str = String::new();
        let mut head = String::new();
        let mut branch = String::new();
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
            }
        }
        if !path_str.is_empty() {
            let is_main = path_str == path;
            worktrees.push(Worktree {
                name,
                path: path_str,
                head,
                branch,
                is_main,
            });
        }
    }
    Ok(worktrees)
}
