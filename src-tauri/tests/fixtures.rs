use app_lib::engine;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success());
}

/// 允许失败的 git 调用（merge 冲突这类预期非零退出）。
fn try_git(dir: &std::path::Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success()
}

fn linear_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "a.txt"]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    dir
}

#[test]
fn load_linear_graph() {
    let dir = linear_repo();
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(data.commits.len(), 2);
    assert_eq!(data.commits[0].parents.len(), 1); // c2 -> c1
    assert_eq!(data.commits[1].parents.len(), 0); // c1 root
}

#[test]
fn read_status_detects_unstaged() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].status, "modified");
}

#[test]
fn read_diff_for_workdir() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let diff = engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    assert_eq!(diff.files.len(), 1);
}

#[test]
fn read_status_counts_changed_lines() {
    let dir = linear_repo();
    // c2 里 a.txt 是单行 "2"，替换成三行 → 删 1 行、加 3 行
    fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    let a = status
        .unstaged
        .iter()
        .find(|f| f.path == "a.txt")
        .expect("a.txt in unstaged");
    assert_eq!((a.additions, a.deletions), (3, 1));
}

#[test]
fn read_status_reports_untracked_as_added() {
    let dir = linear_repo();
    fs::write(dir.path().join("new.txt"), "x\ny\n").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    let new = status
        .unstaged
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("new.txt in unstaged");
    // 未跟踪文件曾经被 INDEX_* 判断误标成 modified、行数恒为 0
    assert_eq!(new.status, "added");
    assert_eq!((new.additions, new.deletions), (2, 0));
}

#[test]
fn read_status_counts_staged_lines_separately() {
    let dir = linear_repo();
    fs::write(dir.path().join("new.txt"), "x\ny\n").unwrap();
    run_git(dir.path(), &["add", "new.txt"]);
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    let staged = status
        .staged
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("new.txt in staged");
    assert_eq!(staged.status, "added");
    assert_eq!((staged.additions, staged.deletions), (2, 0));
}

#[test]
fn read_status_lists_conflicted_files() {
    let dir = linear_repo();
    run_git(dir.path(), &["checkout", "-q", "-b", "other"]);
    fs::write(dir.path().join("conflict.txt"), "other side\n").unwrap();
    run_git(dir.path(), &["add", "conflict.txt"]);
    run_git(dir.path(), &["commit", "-qm", "other"]);
    run_git(dir.path(), &["checkout", "-q", "-"]);
    fs::write(dir.path().join("conflict.txt"), "this side\n").unwrap();
    run_git(dir.path(), &["add", "conflict.txt"]);
    run_git(dir.path(), &["commit", "-qm", "this"]);
    assert!(!try_git(dir.path(), &["merge", "other"]), "merge 应当冲突");

    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    // 冲突文件既不在 INDEX_* 也不在 WT_* 里，曾经完全不出现在列表中
    let conflicted = status
        .unstaged
        .iter()
        .find(|f| f.path == "conflict.txt")
        .expect("conflict.txt in unstaged");
    assert_eq!(conflicted.status, "conflicted");
}

#[test]
fn read_diff_includes_untracked_content() {
    let dir = linear_repo();
    fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
    let diff = engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    let file = diff
        .files
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("未跟踪文件也要有 diff");
    assert!(file
        .hunks
        .iter()
        .any(|h| h.lines.iter().any(|l| l.origin == '+' && l.content == "hello")));
}

#[test]
fn read_diff_excludes_header_lines() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    fs::write(dir.path().join("new.txt"), "added\n").unwrap();
    let diff = engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    // libgit2 的文件头 'F' / hunk 头 'H' 曾经被当成内容行渲染出来
    for file in &diff.files {
        for hunk in &file.hunks {
            for line in &hunk.lines {
                assert!(
                    matches!(line.origin, '+' | '-' | ' '),
                    "{} 里出现了非内容行: {:?} {:?}",
                    file.path,
                    line.origin,
                    line.content
                );
            }
        }
    }
}

#[test]
fn create_and_read_worktree() {
    let dir = linear_repo();
    let wt_dir = dir.path().join("wt");
    let status = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["worktree", "add", wt_dir.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let wts = engine::worktree::read_worktrees(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(wts.len(), 2);
    // 主工作区排第一且不可删；每项的分支名都要解析出来（曾经恒为空串）
    assert!(wts[0].is_main);
    assert!(!wts[1].is_main);
    assert!(!wts[0].branch.is_empty(), "主工作区分支名为空");
    assert_eq!(wts[1].branch, "wt");
    assert_eq!(wts[1].name, "wt");
}

#[test]
fn open_invalid_repo_fails() {
    let dir = TempDir::new().unwrap();
    let result = engine::repo::open(dir.path().to_str().unwrap());
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "not_repo");
}

#[test]
fn open_repo_reports_both_head_name_and_hash() {
    let dir = linear_repo();
    let info = engine::repo::open(dir.path().to_str().unwrap()).unwrap();
    // head 仍是给人看的分支名
    assert!(info.head == "master" || info.head == "main", "head was {}", info.head);
    // head_hash 是完整 oid，且必须等于图里最新的那条提交——
    // 工作区伪行就是靠这个 hash 挂到 HEAD 上的
    assert_eq!(info.head_hash.len(), 40);
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(info.head_hash, data.commits[0].id);
}
