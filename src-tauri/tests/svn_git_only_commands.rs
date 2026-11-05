//! 第 69 条：不走 `trait Vcs` 的 git 专属写命令，在 SVN 混合仓库上必须被后端拒绝。
//!
//! 原来只靠前端按 `capabilities` 把入口藏起来，已经漏过两次。这里逐条直接调命令
//! （等同于绕过界面 `invoke`），断言全部被拒、而且仓库没有任何变化。
//! 「新加的 git 专属命令漏了守卫」由 `svn_write_guard.rs` 的结构测试守着。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::commands::{ops, remote, worktree};
use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
use app_lib::error::GitError;
use std::fs;
use std::process::Command;
use svn_fixtures::{svn_repo, SvnFixture};
use tempfile::TempDir;

fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// 有一条提交、一个远程分支 ref、一个 tag 的混合仓库。远程分支 ref 是手写的：
/// 不需要真的有远程，命令在动手之前就该被拦下。
fn hybrid_with_history() -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    git(&wc, &["init", "-b", "work"]);
    git(&wc, &["config", "user.email", "t@t"]);
    git(&wc, &["config", "user.name", "T"]);
    fs::write(
        wc.join(".git/mygitclient.json"),
        r#"{"version":1,"mode":"svn-git","svnUrl":"file:///whatever",
             "whitelist":["Code/"],"extensions":[".cs"],
             "baselineBranch":"svn/base"}"#,
    )
    .unwrap();
    write_exclude(
        &fx.wc_str(),
        &generate_exclude(&["Code/".to_string()], &[".cs".to_string()]),
    )
    .unwrap();
    git(&wc, &["commit", "--allow-empty", "-m", "c1"]);
    git(&wc, &["update-ref", "refs/remotes/origin/feat", "HEAD"]);
    git(&wc, &["tag", "v1"]);
    git(&wc, &["remote", "add", "origin", "file:///nowhere"]);
    fx
}

/// 仓库里能被这些命令改到的全部东西：refs、HEAD、worktree 登记、工作区状态。
fn snapshot(wc: &std::path::Path) -> String {
    [
        git(wc, &["for-each-ref"]),
        git(wc, &["rev-parse", "HEAD"]),
        git(wc, &["symbolic-ref", "HEAD"]),
        git(wc, &["worktree", "list", "--porcelain"]),
        git(wc, &["status", "--porcelain", "--untracked-files=all"]),
    ]
    .join("\n")
}

fn assert_refused<T: std::fmt::Debug>(what: &str, r: Result<T, GitError>) {
    match r {
        Err(e) => assert_eq!(e.code, "svn_not_supported", "{what}：错误码不对：{e:?}"),
        Ok(v) => panic!("{what} 在 SVN 混合仓库上没有被拒：{v:?}"),
    }
}

#[test]
fn git_only_commands_are_refused_on_an_svn_hybrid_and_change_nothing() {
    let fx = hybrid_with_history();
    let wc = fx.wc();
    let p = fx.wc_str();
    let before = snapshot(&wc);
    let patch = TempDir::new().unwrap();
    let patch_out = patch.path().join("c1.patch");

    assert_refused(
        "checkout_remote_branch",
        ops::checkout_remote_branch(p.clone(), "origin/feat".into()),
    );
    assert_refused(
        "delete_remote_branch",
        ops::delete_remote_branch(p.clone(), "origin".into(), "feat".into()),
    );
    assert_refused(
        "delete_remote_ref",
        ops::delete_remote_ref(p.clone(), "origin/feat".into()),
    );
    assert_refused(
        "create_tag",
        ops::create_tag(p.clone(), "v2".into(), "HEAD".into()),
    );
    assert_refused("push_tag", remote::push_tag(p.clone(), "v1".into()));
    assert_refused("cherry_pick", ops::cherry_pick(p.clone(), "HEAD".into()));
    assert_refused(
        "save_patch",
        ops::save_patch(
            p.clone(),
            "HEAD".into(),
            patch_out.to_string_lossy().to_string(),
        ),
    );
    assert_refused("fetch", remote::fetch(p.clone(), true));
    assert_refused("pull", remote::pull(p.clone(), false));
    assert_refused("push", remote::push(p.clone(), false));
    assert_refused(
        "create_worktree",
        worktree::create_worktree(p.clone(), "wt".into(), Some("wt".into())),
    );
    assert_refused(
        "delete_worktree",
        worktree::delete_worktree(p.clone(), p.clone(), true),
    );

    assert_eq!(snapshot(&wc), before, "被拒的命令改动了仓库");
    assert!(!patch_out.exists(), "save_patch 被拒却写出了文件");
    assert!(
        !wc.join(".worktrees").exists(),
        "create_worktree 被拒却建了目录"
    );
}

#[test]
fn reading_worktrees_still_works_on_an_svn_hybrid() {
    // 打开仓库时无条件读一次 worktree 列表（repoStore.openRepo），拦了它整个仓库就打不开
    let fx = hybrid_with_history();
    assert!(worktree::read_worktrees_cmd(fx.wc_str()).is_ok());
}

#[test]
fn a_plain_git_repo_still_runs_the_git_only_commands() {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    git(d, &["init", "-b", "main"]);
    git(d, &["config", "user.email", "t@t"]);
    git(d, &["config", "user.name", "T"]);
    git(d, &["commit", "--allow-empty", "-m", "c1"]);
    let p = d.to_string_lossy().to_string();

    ops::create_tag(p.clone(), "v1".into(), "HEAD".into()).unwrap();
    assert_eq!(git(d, &["tag"]).trim(), "v1");
    worktree::create_worktree(p, "wt".into(), Some("wt".into())).unwrap();
    assert!(d.join(".worktrees/wt").is_dir());
}
