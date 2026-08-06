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
