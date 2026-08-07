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

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

/// bare 远程 + 一个工作仓库，全程走本地路径，不碰网络。
fn repo_with_remote() -> (TempDir, TempDir) {
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
fn reports_the_upstream_and_splits_off_the_remote_name() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/x"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["push", "-u", "origin", "feat/x"]);
    // 回到别的分支，否则删的是当前分支
    run_git(work.path(), &["checkout", "-q", "-"]);

    let info = branch::delete_info(&p(&work), "feat/x").unwrap();
    assert_eq!(info.upstream.as_deref(), Some("origin/feat/x"));
    // 远程名和分支名必须分开给：前端按第一个 / 去切会把带斜杠的分支名切错
    assert_eq!(info.remote.as_deref(), Some("origin"));
    assert_eq!(info.remote_branch.as_deref(), Some("feat/x"));
}

#[test]
fn finds_the_remote_branch_even_without_a_configured_upstream() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/y"]);
    commit(work.path(), "c2");
    // 不带 -u：这样推上去的分支没有配置上游，只看 upstream 会以为它没有远程分支，
    // 勾选框就不会出现
    run_git(work.path(), &["push", "origin", "feat/y"]);
    run_git(work.path(), &["checkout", "-q", "-"]);

    let info = branch::delete_info(&p(&work), "feat/y").unwrap();
    assert_eq!(info.upstream.as_deref(), Some("origin/feat/y"));
    assert_eq!(info.remote.as_deref(), Some("origin"));
    assert_eq!(info.remote_branch.as_deref(), Some("feat/y"));
}

#[test]
fn a_purely_local_branch_has_no_remote() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "local-only"]);

    let info = branch::delete_info(&p(&work), "local-only").unwrap();
    assert_eq!(info.upstream, None);
    assert_eq!(info.remote, None);
    assert_eq!(info.remote_branch, None);
    // 刚从 HEAD 开出来，没有任何独有提交
    assert_eq!(info.unmerged, 0);
}

#[test]
fn counts_commits_that_are_not_on_the_current_branch() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/z"]);
    commit(work.path(), "c2");
    commit(work.path(), "c3");
    run_git(work.path(), &["checkout", "-q", "-"]);

    // 这两个提交只在 feat/z 上，删掉分支就只能靠 reflog 找回
    let info = branch::delete_info(&p(&work), "feat/z").unwrap();
    assert_eq!(info.unmerged, 2);
}

#[test]
fn reports_the_worktree_that_has_the_branch_checked_out() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "feat/wt"]);
    let wt = work.path().join("wt-dir");
    run_git(
        work.path(),
        &["worktree", "add", wt.to_str().unwrap(), "feat/wt"],
    );

    let info = branch::delete_info(&p(&work), "feat/wt").unwrap();
    let target = info.worktree.expect("被 worktree 检出的分支要报出来");
    assert_eq!(target.name, "wt-dir");
    // 路径要给全的：`git worktree remove` 收路径，靠名字拼是拼不出来的
    assert!(target.path.ends_with("wt-dir"), "{}", target.path);
}

/// 这条断言的是这个功能存在的理由，不是锦上添花：分支被 worktree 占着时
/// `git branch -D` 直接拒绝，没有那个勾选框这类分支在界面上根本删不掉。
#[test]
fn git_refuses_to_delete_a_branch_a_worktree_has_checked_out() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "feat/busy"]);
    let wt = work.path().join("busy-dir");
    let wt_path = wt.to_str().unwrap();
    run_git(work.path(), &["worktree", "add", wt_path, "feat/busy"]);

    let refused = Command::new("git")
        .current_dir(work.path())
        .args(["branch", "-D", "feat/busy"])
        .status()
        .unwrap();
    assert!(!refused.success(), "git 应当拒绝删除被检出的分支");

    // 先把 worktree 摘掉，同一条命令就通了——界面走的正是这个顺序
    run_git(work.path(), &["worktree", "remove", wt_path]);
    run_git(work.path(), &["branch", "-D", "feat/busy"]);
}

/// 主工作区不给勾选框：它删不掉，给了也是死路。
#[test]
fn the_main_worktree_is_not_offered() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/here"]);

    let info = branch::delete_info(&p(&work), "feat/here").unwrap();
    assert_eq!(info.worktree, None);
}

#[test]
fn a_branch_without_a_worktree_reports_none() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "plain"]);

    let info = branch::delete_info(&p(&work), "plain").unwrap();
    assert_eq!(info.worktree, None);
}

#[test]
fn a_merged_branch_reports_nothing_unmerged() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/merged"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["checkout", "-q", "-"]);
    run_git(work.path(), &["merge", "--no-ff", "-m", "merge", "feat/merged"]);

    let info = branch::delete_info(&p(&work), "feat/merged").unwrap();
    assert_eq!(info.unmerged, 0);
}
