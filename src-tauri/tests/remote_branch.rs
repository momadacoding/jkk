use app_lib::commands::ops;
use app_lib::engine::branch;
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

fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git").current_dir(dir).args(args).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

/// bare 远程 + 一个克隆，远程上带一条 feat/x 分支。全程走本地路径，不碰网络。
fn cloned_repo() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let seed = TempDir::new().unwrap();
    run_git(seed.path(), &["init"]);
    run_git(seed.path(), &["config", "user.email", "test@test.com"]);
    run_git(seed.path(), &["config", "user.name", "Test"]);
    commit(seed.path(), "c1");
    run_git(seed.path(), &["remote", "add", "origin", &url]);
    run_git(seed.path(), &["push", "-u", "origin", "HEAD"]);
    run_git(seed.path(), &["checkout", "-q", "-b", "feat/x"]);
    commit(seed.path(), "c2");
    run_git(seed.path(), &["push", "-u", "origin", "feat/x"]);

    let work = TempDir::new().unwrap();
    run_git(work.path(), &["clone", &url, work.path().to_str().unwrap()]);
    run_git(work.path(), &["config", "user.email", "test@test.com"]);
    run_git(work.path(), &["config", "user.name", "Test"]);

    (remote, work)
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn splits_the_remote_name_off_the_ref() {
    let (_remote, work) = cloned_repo();
    let (remote, branch_name) = branch::split_remote_ref(&p(&work), "origin/feat/x").unwrap();
    // 分支名里的斜杠必须留着，只能切掉远程名那一段
    assert_eq!(remote, "origin");
    assert_eq!(branch_name, "feat/x");
}

#[test]
fn a_ref_from_an_unknown_remote_is_an_error() {
    let (_remote, work) = cloned_repo();
    // 不能凭第一个斜杠瞎猜：upstream 这个远程根本不存在
    let result = branch::split_remote_ref(&p(&work), "upstream/feat/x");
    assert!(result.is_err());
}

#[test]
fn checkout_creates_a_local_branch_that_tracks_the_remote() {
    let (_remote, work) = cloned_repo();

    let local = ops::checkout_remote_branch(p(&work), "origin/feat/x".into()).unwrap();
    assert_eq!(local, "feat/x");

    // 必须是真的在分支上，不能是游离 HEAD——那样之后的提交不属于任何分支
    assert_eq!(git_out(work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]), "feat/x");
    // 而且上游要设好，否则接下来 pull/push 都得手动指定
    assert_eq!(
        git_out(work.path(), &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
}

#[test]
fn checkout_switches_to_an_existing_local_branch_without_resetting_it() {
    let (_remote, work) = cloned_repo();
    // 先检出一次，再在本地多提交一条，然后回到 master
    ops::checkout_remote_branch(p(&work), "origin/feat/x".into()).unwrap();
    commit(work.path(), "local-only");
    let tip = git_out(work.path(), &["rev-parse", "HEAD"]);
    run_git(work.path(), &["checkout", "-q", "-"]);

    ops::checkout_remote_branch(p(&work), "origin/feat/x".into()).unwrap();

    assert_eq!(git_out(work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]), "feat/x");
    // 分支已经存在时只切过去，不去动它指向哪：静默重置到远程会丢掉这条本地提交
    assert_eq!(git_out(work.path(), &["rev-parse", "HEAD"]), tip);
}

#[test]
fn delete_remote_ref_removes_the_branch_from_the_remote() {
    let (_remote, work) = cloned_repo();
    assert!(git_out(work.path(), &["ls-remote", "--heads", "origin", "feat/x"]).contains("feat/x"));

    ops::delete_remote_ref(p(&work), "origin/feat/x".into()).unwrap();

    assert!(git_out(work.path(), &["ls-remote", "--heads", "origin", "feat/x"]).is_empty());
}
