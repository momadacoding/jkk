use std::process::Command;
use crate::error::{GitError, Result};

fn run_git(path: &str, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn create_branch(path: String, name: String, start_point: Option<String>) -> Result<()> {
    let mut args = vec!["branch", &name];
    if let Some(start) = &start_point {
        args.push(start);
    }
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn delete_branch(path: String, name: String) -> Result<()> {
    run_git(&path, &["branch", "-D", &name])?;
    Ok(())
}

#[tauri::command]
pub fn rename_branch(path: String, old: String, new: String) -> Result<()> {
    run_git(&path, &["branch", "-m", &old, &new])?;
    Ok(())
}

#[tauri::command]
pub fn checkout(path: String, rev: String) -> Result<()> {
    run_git(&path, &["checkout", &rev])?;
    Ok(())
}

#[tauri::command]
pub fn stage(path: String, paths: Vec<String>) -> Result<()> {
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut args: Vec<&str> = vec!["add"];
    args.extend(path_refs);
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn unstage(path: String, paths: Vec<String>) -> Result<()> {
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut args: Vec<&str> = vec!["reset", "HEAD"];
    args.extend(path_refs);
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn commit(path: String, message: String, amend: bool) -> Result<()> {
    if amend {
        run_git(&path, &["commit", "--amend", "-m", &message])?;
    } else {
        run_git(&path, &["commit", "-m", &message])?;
    }
    Ok(())
}
