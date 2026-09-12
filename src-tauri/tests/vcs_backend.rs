//! 后端与 engine 的**等价性**测试。
//!
//! Phase 0 是纯重构，验收标准是「经 trait 调用与直接调 engine 结果一致」。
//! 这比「后端能跑通」强得多——后者一个返回固定值的桩也能通过。

use app_lib::engine;
use app_lib::vcs::backend_for;
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

/// 一个有两条提交、一个未暂存改动、一个未跟踪文件的仓库。
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    // 关掉行尾转换，否则 checkout 出来的是 CRLF，测试就变成在测 autocrlf 了
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);
    fs::write(dir.path().join("a.txt"), "1\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2\n").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    // 留下工作区状态给 status/diff 用
    fs::write(dir.path().join("a.txt"), "3\n").unwrap();
    fs::write(dir.path().join("untracked.txt"), "u\n").unwrap();
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn backend_open_matches_engine() {
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().open().unwrap();
    let via_engine = engine::repo::open(&path).unwrap();
    assert_eq!(via_backend.head, via_engine.head);
    assert_eq!(via_backend.head_hash, via_engine.head_hash);
    assert_eq!(via_backend.path, via_engine.path);
}

#[test]
fn backend_graph_matches_engine() {
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().graph().unwrap();
    let via_engine = engine::graph::load(&path).unwrap();
    assert_eq!(via_backend.commits.len(), via_engine.commits.len());
    assert_eq!(via_backend.refs.len(), via_engine.refs.len());
    let ids: Vec<&String> = via_backend.commits.iter().map(|c| &c.id).collect();
    let expect: Vec<&String> = via_engine.commits.iter().map(|c| &c.id).collect();
    assert_eq!(ids, expect);
}

#[test]
fn backend_status_matches_engine() {
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().status().unwrap();
    let via_engine = engine::diff::read_status(&path).unwrap();
    assert_eq!(via_backend.staged.len(), via_engine.staged.len());
    assert_eq!(via_backend.unstaged.len(), via_engine.unstaged.len());
    // 非空断言：仓库里确实有改动，否则这个测试等于什么都没测
    assert!(!via_backend.unstaged.is_empty(), "fixture 应当有未暂存改动");
}

#[test]
fn backend_diff_matches_engine() {
    // scope 用 "workdir"：这是前端实际传的值（RepoTab.tsx 里 refreshDiff 的调用点），
    // 也是现有 tests/fixtures.rs 用的值。read_diff 只特判 "commit:" 前缀、其余一律
    // 走 workdir 分支，所以随便写个字符串也能通过——但那样测的就不是真实输入形态了。
    let dir = repo();
    let path = p(&dir);
    let via_backend = backend_for(&path).unwrap().diff("workdir").unwrap();
    let via_engine = engine::diff::read_diff(&path, "workdir".to_string()).unwrap();
    assert_eq!(via_backend.files.len(), via_engine.files.len());
    assert!(
        !via_backend.files.is_empty(),
        "fixture 应当有可 diff 的文件"
    );
}

#[test]
fn backend_diff_handles_commit_scope() {
    // 另一条真实形态：详情面板点一条提交时传的是 "commit:<完整 oid>"
    let dir = repo();
    let path = p(&dir);
    let head = backend_for(&path).unwrap().open().unwrap().head_hash;
    let scope = format!("commit:{head}");
    let via_backend = backend_for(&path).unwrap().diff(&scope).unwrap();
    let via_engine = engine::diff::read_diff(&path, scope.clone()).unwrap();
    assert_eq!(via_backend.files.len(), via_engine.files.len());
    assert!(!via_backend.files.is_empty(), "c2 相对 c1 应当有变更");
}

#[test]
fn backend_for_rejects_non_repo() {
    let dir = TempDir::new().unwrap();
    // 空目录不是 git 仓库。backend_for 本身只是构造，错误在 open() 时才暴露——
    // 这与重构前 open_repo 的行为一致，不能提前到构造时报错。
    let backend = backend_for(dir.path().to_str().unwrap()).unwrap();
    let err = backend.open().unwrap_err();
    assert_eq!(err.code, "not_repo");
}
