use app_lib::commands::worktree as cmd;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

/// 主仓库 + 一个位于 `<主仓库>/.worktrees/<name>` 的 worktree，
/// 复刻用户实际的目录布局。
fn repo_with_worktree(name: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let main = dir.path().join("main");
    fs::create_dir_all(&main).unwrap();
    run_git(&main, &["init"]);
    run_git(&main, &["config", "user.email", "test@test.com"]);
    run_git(&main, &["config", "user.name", "Test"]);
    fs::write(main.join("a.txt"), "1").unwrap();
    run_git(&main, &["add", "."]);
    run_git(&main, &["commit", "-m", "c1"]);

    let wt = main.join(".worktrees").join(name);
    run_git(&main, &["worktree", "add", wt.to_str().unwrap(), "-b", name]);
    (dir, wt)
}

fn main_of(wt: &std::path::Path) -> std::path::PathBuf {
    wt.parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
fn delete_removes_a_clean_worktree() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .unwrap();

    assert!(!wt.exists(), "worktree directory should be gone");
    let out = Command::new("git").current_dir(&main).args(["worktree", "list"]).output().unwrap();
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(!listing.contains("feature"), "still listed: {listing}");
}

#[test]
fn delete_surfaces_the_real_git_error() {
    // 界面只显示我们返回的 message。自造一句 "worktree remove failed" 等于把
    // git 唯一说清楚原因的那句话扔了——用户只能靠猜。
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("scratch.txt"), "untracked").unwrap();

    let err = cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .expect_err("git 会拒绝删除有未跟踪文件的 worktree");

    assert_eq!(err.code, "git_cli");
    assert!(
        err.message.contains("contains modified or untracked files"),
        "拿到的是 git 的原文吗？实际是：{}",
        err.message,
    );
    // 干净的拒绝：什么都不该被删
    assert!(wt.join("a.txt").exists());
    // 这是「什么都没动」的失败，别把它说成孤儿目录吓唬人
    assert_ne!(err.code, "worktree_partial");
    assert!(!err.message.contains("手工删除"), "{}", err.message);
}

/// 登记还在的失败不该被改写。
///
/// `git worktree remove` 分两步：先解除登记、再删目录。第一步就被拒（有未提交
/// 改动）和第二步栽了（目录没删干净、登记已经没了）是两种完全不同的处境，前者
/// 原地重试就行，后者留下的目录已经不受 git 管了。混为一谈的话，要么白白吓唬
/// 用户，要么让人看不出目录已经成了孤儿。
#[test]
fn a_failure_before_unregistering_keeps_the_worktree_usable() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("scratch.txt"), "untracked").unwrap();

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .expect_err("有未跟踪文件时会被拒");

    // 登记还在，worktree 还能正常用
    let out = Command::new("git")
        .current_dir(&main)
        .args(["worktree", "list"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("feature"),
        "登记不该消失"
    );
}

#[test]
fn create_puts_the_worktree_under_dot_worktrees() {
    let (_dir, wt) = repo_with_worktree("first");
    let main = main_of(&wt);

    cmd::create_worktree(main.to_str().unwrap().to_string(), "second".to_string(), None).unwrap();

    let created = main.join(".worktrees").join("second");
    assert!(created.exists(), "应该建在 .worktrees 下面，实际没有：{created:?}");
    assert!(created.join("a.txt").exists(), "worktree 里应该有仓库内容");
}

#[test]
fn create_surfaces_the_real_git_error() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    // 同名再建一次，git 会说分支已被占用
    let err = cmd::create_worktree(main.to_str().unwrap().to_string(), "feature".to_string(), None)
        .expect_err("重名的 worktree 应该失败");
    assert_eq!(err.code, "git_cli");
    assert!(!err.message.is_empty());
    assert!(
        !err.message.contains("worktree add failed"),
        "这是我们自造的占位文案，不是 git 的原文：{}",
        err.message,
    );
}
