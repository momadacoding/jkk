use app_lib::commands::remote;
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
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

fn init(dir: &std::path::Path) {
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
}

/// bare 远程 + 两个克隆，全程走本地路径，不碰网络。
fn trio() -> (TempDir, TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let a = TempDir::new().unwrap();
    init(a.path());
    commit(a.path(), "c1");
    run_git(a.path(), &["remote", "add", "origin", &url]);
    run_git(a.path(), &["push", "-u", "origin", "HEAD"]);

    let b = TempDir::new().unwrap();
    run_git(b.path(), &["clone", &url, b.path().to_str().unwrap()]);
    run_git(b.path(), &["config", "user.email", "test@test.com"]);
    run_git(b.path(), &["config", "user.name", "Test"]);

    (remote, a, b)
}

#[test]
fn fetch_updates_remote_tracking_ref() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);

    let before = git_out(b.path(), &["rev-parse", "origin/HEAD"]);
    remote::fetch(b.path().to_str().unwrap().to_string(), false).unwrap();
    let after = git_out(b.path(), &["rev-parse", "FETCH_HEAD"]);
    assert_ne!(before, after);
}

#[test]
fn pull_brings_commits_into_working_branch() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);

    remote::pull(b.path().to_str().unwrap().to_string(), false).unwrap();
    assert!(b.path().join("c2.txt").exists());
}

#[test]
fn pull_with_rebase_keeps_history_linear() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    commit(b.path(), "local-side");

    remote::pull(b.path().to_str().unwrap().to_string(), true).unwrap();
    // rebase 不产生 merge 提交：所有提交都只有一个父
    let parents = git_out(b.path(), &["log", "--pretty=%p"]);
    for line in parents.lines() {
        assert!(
            line.split_whitespace().count() <= 1,
            "found a merge commit: {line}"
        );
    }
}

#[test]
fn push_sets_upstream_for_a_fresh_branch() {
    // 新分支没有上游，裸 `git push` 会失败；实现必须自动补 -u origin <branch>
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["checkout", "-b", "feature"]);
    commit(b.path(), "f1");

    remote::push(b.path().to_str().unwrap().to_string(), false).unwrap();
    let upstream = git_out(
        b.path(),
        &["rev-parse", "--abbrev-ref", "feature@{upstream}"],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn push_propagates_rejection_as_error() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    commit(b.path(), "local-side");

    // b 落后于远程，非快进推送必须被拒绝并转成 GitError 而不是静默成功
    let err = remote::push(b.path().to_str().unwrap().to_string(), false)
        .expect_err("non-fast-forward push should fail");
    assert_eq!(err.code, "git_cli");
}

#[test]
fn push_tag_puts_the_tag_on_the_remote() {
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["tag", "-a", "v1", "-m", "release one"]);

    remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into()).unwrap();

    let listed = git_out(b.path(), &["ls-remote", "origin", "refs/tags/v1"]);
    assert!(
        listed.contains("refs/tags/v1"),
        "远端没有这条 tag：{listed:?}"
    );
}

#[test]
fn push_tag_rejects_an_empty_name() {
    // 空名不能交给 git：`git push origin refs/tags/:refs/tags/` 是删除远端引用的
    // 形态，绝不能因为界面传了个空串就走到那里去
    let (_remote, _a, b) = trio();
    let path = b.path().to_str().unwrap().to_string();

    for empty in ["", "   "] {
        let err = remote::push_tag(path.clone(), empty.into())
            .expect_err("empty tag name should be rejected");
        assert_eq!(err.code, "bad_arg", "empty name {empty:?} 没被挡住");
    }
}

#[test]
fn push_tag_works_when_the_remote_is_not_named_origin() {
    // 远端叫 upstream 的仓库不少（fork 工作流）。硬写 origin 会让这类仓库
    // 永远推不出去，而界面上没有地方能改远端名。
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["remote", "rename", "origin", "upstream"]);
    run_git(b.path(), &["tag", "v1"]);

    remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into()).unwrap();

    let listed = git_out(b.path(), &["ls-remote", "upstream", "refs/tags/v1"]);
    assert!(listed.contains("refs/tags/v1"), "tag 没推上去：{listed:?}");
}

#[test]
fn push_tag_without_any_remote_reports_a_clear_error() {
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["remote", "remove", "origin"]);
    run_git(b.path(), &["tag", "v1"]);

    let err = remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into())
        .expect_err("没有远端时应当报错");
    assert_eq!(err.code, "no_remote");
}

#[test]
fn push_tag_refuses_to_guess_between_several_remotes() {
    // 多个远端又没有 origin：猜哪个都可能把 tag 推到错的地方
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["remote", "rename", "origin", "upstream"]);
    run_git(
        b.path(),
        &["remote", "add", "fork", "https://example.invalid/x"],
    );
    run_git(b.path(), &["tag", "v1"]);

    let err = remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into())
        .expect_err("多个远端且没有 origin 时应当报错");
    assert_eq!(err.code, "ambiguous_remote");
}

#[test]
fn push_tag_does_not_push_a_same_named_branch() {
    // 短名 `git push origin v1` 在同时存在分支 v1 和 tag v1 时是有歧义的。
    // 必须用全限定 refspec，否则可能推错东西。
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["branch", "v1"]);
    run_git(b.path(), &["tag", "v1"]);

    remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into()).unwrap();

    let tag = git_out(b.path(), &["ls-remote", "origin", "refs/tags/v1"]);
    assert!(tag.contains("refs/tags/v1"), "tag 没推上去：{tag:?}");
    let branch = git_out(b.path(), &["ls-remote", "origin", "refs/heads/v1"]);
    assert!(branch.is_empty(), "同名分支被一起推上去了：{branch:?}");
}

#[test]
fn force_push_overwrites_remote() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    run_git(b.path(), &["fetch"]);
    commit(b.path(), "local-side");
    run_git(b.path(), &["reset", "--hard", "HEAD"]);

    remote::push(b.path().to_str().unwrap().to_string(), true).unwrap();
    let local = git_out(b.path(), &["rev-parse", "HEAD"]);
    let pushed = git_out(a.path(), &["ls-remote", "origin", "HEAD"]);
    assert!(
        pushed.starts_with(&local[..8]),
        "remote HEAD {pushed} != local {local}"
    );
}
