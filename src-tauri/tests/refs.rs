use app_lib::engine::refs;
use git2::Repository;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

/// 本地仓库 + bare 远程，master 已设上游，全程不碰网络。
fn repo_with_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let local = TempDir::new().unwrap();
    run_git(local.path(), &["init"]);
    run_git(local.path(), &["config", "user.email", "test@test.com"]);
    run_git(local.path(), &["config", "user.name", "Test"]);
    commit(local.path(), "c1");
    run_git(
        local.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    run_git(local.path(), &["push", "-u", "origin", "HEAD"]);
    (local, remote)
}

fn find<'a>(refs: &'a [refs::RefPointer], name: &str) -> &'a refs::RefPointer {
    refs.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no ref {name}"))
}

#[test]
fn local_branch_in_sync_reports_zero_ahead_behind() {
    let (local, _remote) = repo_with_remote();
    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    let master = find(&all, "refs/heads/master");
    assert_eq!(master.ahead, Some(0));
    assert_eq!(master.behind, Some(0));
}

#[test]
fn local_branch_counts_its_own_ahead_and_behind() {
    // 侧边栏每个分支右边都要显示自己的 ↑↓，不能只有当前分支有
    let (local, _remote) = repo_with_remote();
    commit(local.path(), "c2");
    commit(local.path(), "c3");

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let master = find(&all, "refs/heads/master");
    assert_eq!(master.ahead, Some(2));
    assert_eq!(master.behind, Some(0));
}

#[test]
fn a_branch_without_upstream_reports_none() {
    // None 和 Some(0) 含义不同：没有上游时界面根本不该画徽章
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["branch", "solo"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let solo = find(&all, "refs/heads/solo");
    assert_eq!(solo.ahead, None);
    assert_eq!(solo.behind, None);
}

#[test]
fn remote_branches_carry_no_counts() {
    let (local, _remote) = repo_with_remote();
    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    let origin = find(&all, "refs/remotes/origin/master");
    assert_eq!(origin.kind, "remote");
    assert_eq!(origin.ahead, None);
    assert_eq!(origin.behind, None);
}

#[test]
fn counts_are_per_branch_not_per_head() {
    // 曾经只有当前分支有计数。切到别的分支后，原分支的计数不能跟着 HEAD 跑。
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["checkout", "-b", "feature"]);
    commit(local.path(), "f1");
    run_git(local.path(), &["push", "-u", "origin", "feature"]);
    commit(local.path(), "f2");
    run_git(local.path(), &["checkout", "master"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    // HEAD 在 master 上，但 feature 自己领先 1
    assert_eq!(find(&all, "refs/heads/master").ahead, Some(0));
    assert_eq!(find(&all, "refs/heads/feature").ahead, Some(1));
}
