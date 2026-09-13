use app_lib::engine::git_cli;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// fixture 自己发的 git 命令。故意**不**走 `git_cli::run_git`——用被测对象搭测试
/// 的脚手架，它坏了测试也跟着一起坏，断言就失去意义了。
fn fixture_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

/// 一个临时仓库 + 一个提交（`AGENTS.md` §8：git 写操作一律在临时 fixture 里做）。
fn repo_with_commit() -> TempDir {
    let dir = TempDir::new().unwrap();
    fixture_git(dir.path(), &["init"]);
    fixture_git(dir.path(), &["config", "user.email", "test@test.com"]);
    fixture_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    fixture_git(dir.path(), &["add", "a.txt"]);
    fixture_git(dir.path(), &["commit", "-m", "c1"]);
    dir
}

#[test]
fn git_command_disables_terminal_prompt() {
    // GUI 里没人能回答 git 的 "Username:" 提示，子进程会永久挂死。
    // 这个变量是唯一的防线，改动 git_cli 时不能把它弄丢。
    let cmd = git_cli::git_command(".");
    let envs: Vec<_> = cmd.get_envs().collect();
    let prompt = envs
        .iter()
        .find(|(k, _)| *k == std::ffi::OsStr::new("GIT_TERMINAL_PROMPT"))
        .expect("GIT_TERMINAL_PROMPT not set");
    assert_eq!(prompt.1, Some(std::ffi::OsStr::new("0")));
}

#[test]
fn run_git_surfaces_stderr_on_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let err = git_cli::run_git(dir.path().to_str().unwrap(), &["rev-parse", "HEAD"])
        .expect_err("expected failure outside a repo");
    assert_eq!(err.code, "git_cli");
    assert!(!err.message.is_empty());
}

#[test]
fn run_git_returns_stdout_on_success() {
    let out = git_cli::run_git(".", &["--version"]).unwrap();
    assert!(out.starts_with("git version"), "got {out}");
}

#[test]
fn run_git_env_passes_the_variable_through_to_the_child() {
    // GIT_INDEX_FILE 是本期唯一的用途，直接拿它做断言：指向一个还不存在的路径时
    // git 把它当成空索引，于是 ls-files 什么都不输出；与此同时真实索引里那个文件
    // 原样还在——「录基线不碰用户的暂存区」这条正是 spec §16 R2 的全部安全性所在。
    let dir = repo_with_commit();
    let root = dir.path().to_str().unwrap();
    fs::write(dir.path().join("extra.txt"), "x").unwrap();
    fixture_git(dir.path(), &["add", "--", "extra.txt"]);
    let real_before = git_cli::run_git(root, &["ls-files"]).unwrap();
    assert!(
        real_before.contains("extra.txt"),
        "前提不成立：{real_before}"
    );

    let tmp = dir.path().join("tmp.index");
    let out = git_cli::run_git_env(
        root,
        &["ls-files"],
        &[("GIT_INDEX_FILE", tmp.to_str().unwrap())],
    )
    .unwrap();
    assert_eq!(out.trim(), "", "临时索引应当是空的，实际：{out}");

    let real_after = git_cli::run_git(root, &["ls-files"]).unwrap();
    assert_eq!(real_before, real_after, "真实索引被动过了");
}

#[test]
fn run_git_env_still_maps_failure_to_giterror() {
    // 多了一层注入不能把失败吞掉——`run_git` 的错误语义必须原样保留
    let dir = repo_with_commit();
    let err = git_cli::run_git_env(
        dir.path().to_str().unwrap(),
        &["cat-file", "-p", "deadbeef"],
        &[],
    )
    .expect_err("不存在的对象必须报错");
    assert_eq!(err.code, "git_cli");
}
