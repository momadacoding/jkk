use app_lib::engine;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success());
}

fn linear_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "a.txt"]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    dir
}

#[test]
fn load_linear_graph() {
    let dir = linear_repo();
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(data.commits.len(), 2);
    assert_eq!(data.commits[0].parents.len(), 1); // c2 -> c1
    assert_eq!(data.commits[1].parents.len(), 0); // c1 root
}

#[test]
fn read_status_detects_unstaged() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].status, "modified");
}

#[test]
fn read_diff_for_workdir() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let diff = engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    assert_eq!(diff.files.len(), 1);
}

#[test]
fn create_and_read_worktree() {
    let dir = linear_repo();
    let wt_dir = dir.path().parent().unwrap().join("wt");
    let status = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["worktree", "add", wt_dir.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let wts = engine::worktree::read_worktrees(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(wts.len(), 2);
}

#[test]
fn open_invalid_repo_fails() {
    let dir = TempDir::new().unwrap();
    let result = engine::repo::open(dir.path().to_str().unwrap());
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "not_repo");
}
