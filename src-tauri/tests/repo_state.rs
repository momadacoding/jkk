use app_lib::commands::repo::repo_state;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {:?} failed", args);
}

/// 允许失败的 git 调用（merge 冲突这类预期非零退出）。
fn try_git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success()
}

fn state_of(dir: &Path) -> String {
    repo_state(dir.to_str().unwrap().to_string()).unwrap()
}

/// 主仓库：`base` 上一个提交，`other` 分支改了 `conflict.txt`，
/// 调用方再在别处改同一个文件就能撞出冲突。
fn repo_with_other_branch() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    run_git(p, &["init", "-q", "-b", "base"]);
    run_git(p, &["config", "user.email", "test@test.com"]);
    run_git(p, &["config", "user.name", "Test"]);
    fs::write(p.join("conflict.txt"), "base\n").unwrap();
    run_git(p, &["add", "."]);
    run_git(p, &["commit", "-qm", "base"]);
    run_git(p, &["branch", "other"]);
    run_git(p, &["checkout", "-q", "other"]);
    fs::write(p.join("conflict.txt"), "other side\n").unwrap();
    run_git(p, &["commit", "-qam", "other"]);
    run_git(p, &["checkout", "-q", "base"]);
    dir
}

/// 在 `dir` 当前分支上改同一个文件并提交，然后 merge `other` 撞出冲突。
fn conflict_merge_in(dir: &Path) {
    fs::write(dir.join("conflict.txt"), "this side\n").unwrap();
    run_git(dir, &["commit", "-qam", "this"]);
    assert!(!try_git(dir, &["merge", "other"]), "merge 应当冲突");
}

#[test]
fn normal_repo_reports_normal() {
    let dir = repo_with_other_branch();
    assert_eq!(state_of(dir.path()), "normal");
}

#[test]
fn merge_conflict_in_main_worktree_reports_merge() {
    let dir = repo_with_other_branch();
    conflict_merge_in(dir.path());
    assert_eq!(state_of(dir.path()), "merge");
}

#[test]
fn merge_conflict_in_linked_worktree_reports_merge() {
    // linked worktree 里 `.git` 是一个指向 `<主仓库>/.git/worktrees/<name>` 的文件，
    // MERGE_HEAD 写在那个目录下。拿 `<path>/.git/MERGE_HEAD` 去找永远找不到，
    // 于是在 worktree 里冲突时顶部横幅从来不出现。
    let dir = repo_with_other_branch();
    let wt = dir.path().join("wt");
    run_git(
        dir.path(),
        &["worktree", "add", "-q", "-b", "wt-branch", "wt", "base"],
    );
    conflict_merge_in(&wt);
    assert_eq!(state_of(&wt), "merge");
    // 主工作区不受 worktree 里的 merge 影响
    assert_eq!(state_of(dir.path()), "normal");
}

#[test]
fn rebase_conflict_in_linked_worktree_reports_rebase() {
    let dir = repo_with_other_branch();
    let wt = dir.path().join("wt");
    run_git(
        dir.path(),
        &["worktree", "add", "-q", "-b", "wt-branch", "wt", "base"],
    );
    fs::write(wt.join("conflict.txt"), "this side\n").unwrap();
    run_git(&wt, &["commit", "-qam", "this"]);
    assert!(!try_git(&wt, &["rebase", "other"]), "rebase 应当冲突");
    assert_eq!(state_of(&wt), "rebase");
}

#[test]
fn cherry_pick_conflict_reports_cherry_pick() {
    let dir = repo_with_other_branch();
    fs::write(dir.path().join("conflict.txt"), "this side\n").unwrap();
    run_git(dir.path(), &["commit", "-qam", "this"]);
    assert!(
        !try_git(dir.path(), &["cherry-pick", "other"]),
        "cherry-pick 应当冲突"
    );
    assert_eq!(state_of(dir.path()), "cherry-pick");
}

#[test]
fn revert_conflict_reports_revert() {
    // 应用里有 revert 入口，它同样会停在冲突上等用户处理；
    // 旧实现不认 REVERT_HEAD，这种状态下横幅不出现。
    let dir = repo_with_other_branch();
    fs::write(dir.path().join("conflict.txt"), "one\n").unwrap();
    run_git(dir.path(), &["commit", "-qam", "one"]);
    fs::write(dir.path().join("conflict.txt"), "two\n").unwrap();
    run_git(dir.path(), &["commit", "-qam", "two"]);
    assert!(
        !try_git(dir.path(), &["revert", "--no-edit", "HEAD~1"]),
        "revert 应当冲突"
    );
    assert_eq!(state_of(dir.path()), "revert");
}
