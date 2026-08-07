use app_lib::engine::tree;
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

fn rev(dir: &std::path::Path, spec: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", spec])
        .output()
        .expect("git not found");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 两个提交：c1 建 src/a.txt 和 README.md，c2 改 a.txt 并加 src/deep/b.txt。
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    // 关掉行尾转换，否则读出来的是 CRLF，测试就变成在测 autocrlf 了
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);

    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.txt"), "first\n").unwrap();
    fs::write(dir.path().join("README.md"), "readme\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);

    fs::create_dir_all(dir.path().join("src/deep")).unwrap();
    fs::write(dir.path().join("src/a.txt"), "second\n").unwrap();
    fs::write(dir.path().join("src/deep/b.txt"), "deep\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c2"]);
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn list_tree_returns_files_with_full_paths() {
    let dir = repo();
    let head = rev(dir.path(), "HEAD");
    let listing = tree::list_tree(&p(&dir), &head).unwrap();

    let mut paths = listing.paths.clone();
    paths.sort();
    // 只列文件，目录本身不出现——目录由前端从路径前缀推出来
    assert_eq!(paths, ["README.md", "src/a.txt", "src/deep/b.txt"]);
    assert!(!listing.truncated);
}

#[test]
fn list_tree_reads_that_commit_not_the_workdir() {
    let dir = repo();
    let first = rev(dir.path(), "HEAD~1");
    // 工作区里再加一个文件，它不该出现在旧提交的树里
    fs::write(dir.path().join("untracked.txt"), "x\n").unwrap();

    let listing = tree::list_tree(&p(&dir), &first).unwrap();
    let mut paths = listing.paths.clone();
    paths.sort();
    // c1 时还没有 src/deep/b.txt
    assert_eq!(paths, ["README.md", "src/a.txt"]);
}

#[test]
fn read_blob_returns_the_content_at_that_commit() {
    let dir = repo();
    // 工作区改成第三版，但读的是提交里的那两版
    fs::write(dir.path().join("src/a.txt"), "workdir version\n").unwrap();

    let at_head = tree::read_blob(&p(&dir), &rev(dir.path(), "HEAD"), "src/a.txt").unwrap();
    assert_eq!(at_head.text.as_deref(), Some("second\n"));
    assert!(!at_head.binary);
    assert!(!at_head.truncated);
    assert_eq!(at_head.size, 7);

    // 看历史提交时工作区那份可能早变了，必须走 tree 而不是磁盘
    let at_first = tree::read_blob(&p(&dir), &rev(dir.path(), "HEAD~1"), "src/a.txt").unwrap();
    assert_eq!(at_first.text.as_deref(), Some("first\n"));
}

#[test]
fn read_blob_marks_binary_files_and_skips_the_text() {
    let dir = repo();
    // 带 NUL 字节，libgit2 据此判定为二进制
    fs::write(dir.path().join("logo.png"), b"\x89PNG\r\n\x1a\n\x00\x00\x00rest").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "add binary"]);

    let blob = tree::read_blob(&p(&dir), &rev(dir.path(), "HEAD"), "logo.png").unwrap();
    assert!(blob.binary);
    // 二进制内容不该被当成文本送到前端
    assert!(blob.text.is_none());
    // 8 字节 PNG 魔数 + 3 个 NUL + "rest"
    assert_eq!(blob.size, 15);
}

#[test]
fn read_blob_fails_for_a_path_not_in_that_commit() {
    let dir = repo();
    // b.txt 是 c2 才加的，在 c1 的树里不存在
    let result = tree::read_blob(&p(&dir), &rev(dir.path(), "HEAD~1"), "src/deep/b.txt");
    assert!(result.is_err(), "旧提交里没有的文件不该读出内容");
}
