use app_lib::commands::ops;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    // 关掉行尾转换，否则 checkout 出来的是 CRLF，测试就变成在测 autocrlf 了
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);
    fs::write(dir.path().join("tracked.txt"), "original\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn discards_a_worktree_modification() {
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();

    ops::discard(p(&dir), vec!["tracked.txt".into()], false).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "original\n");
}

#[test]
fn discarding_unstaged_keeps_what_was_already_staged() {
    // Unstaged 区的 Discard 只该回退「工作区相对暂存区」的那部分。
    // 连暂存的改动一起抹掉是另一回事，那是 Staged 区的语义。
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "staged version\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);
    fs::write(dir.path().join("tracked.txt"), "further edits\n").unwrap();

    ops::discard(p(&dir), vec!["tracked.txt".into()], false).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "staged version\n");
}

#[test]
fn discarding_staged_goes_all_the_way_back_to_head() {
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "staged version\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);

    ops::discard(p(&dir), vec!["tracked.txt".into()], true).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "original\n");
    let out = Command::new("git").current_dir(dir.path()).args(["diff", "--cached", "--name-only"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty(), "暂存区应该也干净了");
}

#[test]
fn deletes_an_untracked_file() {
    // git 里没有它的任何副本，除了删掉别无选择
    let dir = repo();
    fs::write(dir.path().join("scratch.png"), "junk").unwrap();

    ops::discard(p(&dir), vec!["scratch.png".into()], false).unwrap();

    assert!(!dir.path().join("scratch.png").exists());
}

#[test]
fn discarding_a_staged_new_file_removes_it_from_index_and_disk() {
    let dir = repo();
    fs::write(dir.path().join("fresh.txt"), "new\n").unwrap();
    run_git(dir.path(), &["add", "fresh.txt"]);

    ops::discard(p(&dir), vec!["fresh.txt".into()], true).unwrap();

    assert!(!dir.path().join("fresh.txt").exists());
    let out = Command::new("git").current_dir(dir.path()).args(["diff", "--cached", "--name-only"]).output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty(), "索引里不该还留着它");
}

#[test]
fn handles_a_mixed_selection_in_one_call() {
    // 多选之后一次 discard，三类文件混在一起是常态
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();
    fs::write(dir.path().join("scratch.png"), "junk").unwrap();
    fs::write(dir.path().join("second.txt"), "also new").unwrap();

    ops::discard(
        p(&dir),
        vec!["tracked.txt".into(), "scratch.png".into(), "second.txt".into()],
        false,
    )
    .unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "original\n");
    assert!(!dir.path().join("scratch.png").exists());
    assert!(!dir.path().join("second.txt").exists());
}

#[test]
fn leaves_files_that_were_not_selected_alone() {
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();
    fs::write(dir.path().join("keep-me.txt"), "precious").unwrap();

    ops::discard(p(&dir), vec!["tracked.txt".into()], false).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("keep-me.txt")).unwrap(), "precious");
}

#[test]
fn an_empty_selection_is_a_no_op() {
    // 空 pathspec 会被 git 当成「全部」，那就成了一次误伤全仓库的 discard
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();

    ops::discard(p(&dir), vec![], false).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("tracked.txt")).unwrap(), "messed up\n");
}

#[test]
fn discards_a_file_inside_a_subdirectory() {
    let dir = repo();
    fs::create_dir_all(dir.path().join("src/deep")).unwrap();
    fs::write(dir.path().join("src/deep/a.txt"), "v1\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c2"]);
    fs::write(dir.path().join("src/deep/a.txt"), "broken\n").unwrap();

    ops::discard(p(&dir), vec!["src/deep/a.txt".into()], false).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("src/deep/a.txt")).unwrap(), "v1\n");
}
