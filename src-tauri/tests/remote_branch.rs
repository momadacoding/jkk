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

/// bare 远程 + 用 `git remote add` 接上去的工作仓库——**没有**
/// `refs/remotes/origin/HEAD`，那条 ref 只有 clone 才会写下。
///
/// fork 工作流里的 `upstream` 远程正是这么加的，而它恰恰是删错分支代价最大的
/// 那个远程（那是别人的主仓库）。
fn repo_with_added_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let work = TempDir::new().unwrap();
    run_git(work.path(), &["init"]);
    run_git(work.path(), &["config", "user.email", "test@test.com"]);
    run_git(work.path(), &["config", "user.name", "Test"]);
    commit(work.path(), "c1");
    run_git(work.path(), &["remote", "add", "origin", &url]);
    run_git(work.path(), &["push", "-u", "origin", "HEAD"]);

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
    assert_eq!(
        git_out(work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat/x"
    );
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

    assert_eq!(
        git_out(work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat/x"
    );
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

/// 远程的默认分支一律不给删，push 都不发出去。
///
/// 这是算错要删哪条分支时的最后一道护栏，不能把安全性外包给服务端：GitHub 会
/// 用 `refusing to delete the current branch` 挡住默认分支，但自建 Gitea/裸仓库
/// 未必挡，而且**只有默认分支**受这个待遇——别的分支删了就是删了。
#[test]
fn refuses_to_delete_the_remotes_default_branch() {
    let (_remote, work) = cloned_repo();
    // 克隆下来的 refs/remotes/origin/HEAD 指着远程的默认分支
    let default_ref = git_out(work.path(), &["rev-parse", "--abbrev-ref", "origin/HEAD"]);
    let default_branch = default_ref
        .strip_prefix("origin/")
        .expect(&default_ref)
        .to_string();

    let err = ops::delete_remote_ref(p(&work), default_ref.clone()).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);

    // 而且远程上它必须原封不动地还在
    assert!(
        !git_out(
            work.path(),
            &["ls-remote", "--heads", "origin", &default_branch]
        )
        .is_empty(),
        "默认分支被删掉了"
    );
}

/// 远程是 `git remote add` 接上去的（没有本地 HEAD 记录），护栏照样要挡住。
///
/// 只读 `refs/remotes/<remote>/HEAD` 的话这里会直接放行——而这正是最该拦住的
/// 场景：fork 工作流里 `upstream` 指的是别人的主仓库，本地 `main` 跟踪
/// `upstream/main` 时**名字是对得上的**，[`find_remote_branch`] 那层修复
/// 拦不住它，只剩这道护栏。
#[test]
fn refuses_to_delete_the_default_branch_of_a_remote_added_by_hand() {
    let (remote, work) = repo_with_added_remote();
    // 拿远程自己的 HEAD 当事实来源，不去猜 init.defaultBranch 是什么
    let default_ref = git_out(remote.path(), &["symbolic-ref", "HEAD"]);
    let default_branch = default_ref
        .strip_prefix("refs/heads/")
        .expect(&default_ref)
        .to_string();

    // 前提确认：本地确实没有这条 ref，纯靠它判断必然放行
    assert!(
        git_out(
            work.path(),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "refs/remotes/origin/HEAD"
            ]
        )
        .is_empty(),
        "这个 fixture 的前提是本地没有 origin/HEAD 记录"
    );

    let err = ops::delete_remote_ref(p(&work), format!("origin/{default_branch}")).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(
        !git_out(
            work.path(),
            &["ls-remote", "--heads", "origin", &default_branch]
        )
        .is_empty(),
        "默认分支被删掉了"
    );
}

/// 护栏只挡默认分支，别的分支该删还是要删得掉。
///
/// 加护栏最容易犯的错是拦过头——真拦过头的话用户所有的删远程分支操作都废了，
/// 而这条路径上没有 CI 会告诉我们。
#[test]
fn an_ordinary_branch_is_still_deletable_on_a_remote_without_a_head_ref() {
    let (_remote, work) = repo_with_added_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/ordinary"]);
    commit(work.path(), "c2");
    run_git(
        work.path(),
        &["push", "-q", "-u", "origin", "feat/ordinary"],
    );
    run_git(work.path(), &["checkout", "-q", "-"]);

    ops::delete_remote_ref(p(&work), "origin/feat/ordinary".into()).unwrap();

    assert!(git_out(
        work.path(),
        &["ls-remote", "--heads", "origin", "feat/ordinary"]
    )
    .is_empty());
}
