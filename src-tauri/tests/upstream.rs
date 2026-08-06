use app_lib::engine::upstream;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

/// 一个带本地 bare「远程」的仓库，全程不碰网络。
fn repo_with_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);

    let local = TempDir::new().unwrap();
    run_git(local.path(), &["init"]);
    run_git(local.path(), &["config", "user.email", "test@test.com"]);
    run_git(local.path(), &["config", "user.name", "Test"]);
    commit(local.path(), "c1");
    let url = remote.path().to_str().unwrap();
    run_git(local.path(), &["remote", "add", "origin", url]);
    run_git(local.path(), &["push", "-u", "origin", "HEAD"]);
    (local, remote)
}

#[test]
fn reports_zero_when_in_sync() {
    let (local, _remote) = repo_with_remote();
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert!(info.upstream.is_some());
    assert_eq!(info.ahead, 0);
    assert_eq!(info.behind, 0);
}

#[test]
fn counts_local_commits_as_ahead() {
    let (local, _remote) = repo_with_remote();
    commit(local.path(), "c2");
    commit(local.path(), "c3");
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert_eq!(info.ahead, 2);
    assert_eq!(info.behind, 0);
}

#[test]
fn reports_no_upstream_for_fresh_branch() {
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["checkout", "-b", "solo"]);
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert_eq!(info.branch, "solo");
    assert!(info.upstream.is_none());
    // 没有 upstream 时计数必须是 0，不能是脏值——界面靠它决定不显示徽章
    assert_eq!(info.ahead, 0);
    assert_eq!(info.behind, 0);
}

#[test]
fn reports_branch_name_without_refs_prefix() {
    let (local, _remote) = repo_with_remote();
    let info = upstream::read(local.path().to_str().unwrap()).unwrap();
    assert!(!info.branch.starts_with("refs/"), "got {}", info.branch);
}
