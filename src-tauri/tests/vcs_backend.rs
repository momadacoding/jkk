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

#[test]
fn backend_stage_then_commit_advances_head() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();
    let before = backend.open().unwrap().head_hash;

    backend.stage(&["a.txt".to_string()]).unwrap();
    let staged = backend.status().unwrap();
    assert_eq!(staged.staged.len(), 1, "a.txt 应当已进暂存区");

    backend.commit("c3", false).unwrap();
    let after = backend.open().unwrap().head_hash;
    assert_ne!(before, after, "提交后 HEAD 必须前进");
}

#[test]
fn backend_unstage_removes_from_index() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.stage(&["a.txt".to_string()]).unwrap();
    assert_eq!(backend.status().unwrap().staged.len(), 1);

    backend.unstage(&["a.txt".to_string()]).unwrap();
    let after = backend.status().unwrap();
    assert_eq!(after.staged.len(), 0);
    // 按路径断言而不是数个数：本项目的 read_status 把**未跟踪文件也算进
    // unstaged**（与 fixtures.rs 的 read_diff_includes_untracked_content 同一约定），
    // 所以 fixture 里的 untracked.txt 也在这一列，数个数会得到 2 而不是 1。
    assert!(
        after.unstaged.iter().any(|f| f.path == "a.txt"),
        "a.txt 应退回未暂存：{:?}",
        after.unstaged.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn backend_discard_restores_tracked_file() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.discard(&["a.txt".to_string()], false).unwrap();
    let content = fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert_eq!(content, "2\n", "应恢复到 c2 的内容");
}

#[test]
fn backend_discard_with_empty_paths_is_a_noop() {
    // 空 pathspec 会被 git 当成「全部」。这条守住的是一次误伤整个仓库的 discard，
    // 是本项目已经踩过的坑（AGENTS.md §6.2）。
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.discard(&[], false).unwrap();

    // 工作区改动必须原封不动
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "3\n");
    assert!(
        dir.path().join("untracked.txt").exists(),
        "未跟踪文件不得被删"
    );
}

#[test]
fn backend_branch_lifecycle() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.create_branch("feat/x", None).unwrap();
    let refs = backend.graph().unwrap().refs;
    assert!(
        refs.iter().any(|r| r.name.ends_with("feat/x")),
        "新分支应出现在 refs 里：{:?}",
        refs.iter().map(|r| &r.name).collect::<Vec<_>>()
    );

    backend.rename_branch("feat/x", "feat/y").unwrap();
    let refs = backend.graph().unwrap().refs;
    assert!(refs.iter().any(|r| r.name.ends_with("feat/y")));
    assert!(!refs.iter().any(|r| r.name.ends_with("feat/x")));

    backend.delete_branch("feat/y").unwrap();
    let refs = backend.graph().unwrap().refs;
    assert!(!refs.iter().any(|r| r.name.ends_with("feat/y")));
}

#[test]
fn backend_create_branch_honours_start_point() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    // 从 HEAD~1（也就是 c1）开分支，它的 tip 必须不等于当前 HEAD
    backend.create_branch("from-c1", Some("HEAD~1")).unwrap();
    let head = backend.open().unwrap().head_hash;
    let refs = backend.graph().unwrap().refs;
    let branch = refs
        .iter()
        .find(|r| r.name.ends_with("from-c1"))
        .expect("分支不存在");
    assert_ne!(branch.hash, head, "start_point 没生效，分支落在了 HEAD 上");
}

#[test]
fn backend_checkout_moves_head() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.create_branch("other", None).unwrap();
    backend.checkout("other").unwrap();
    assert_eq!(backend.open().unwrap().head, "other");
}

#[test]
fn backend_stash_push_then_list_then_pop() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    assert!(backend.list_stashes().unwrap().is_empty());

    backend.stash_push(Some("wip"), false).unwrap();
    let entries = backend.list_stashes().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].message.contains("wip"));
    // stash 之后工作区应回到 HEAD 内容
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "2\n");

    backend.stash_pop(0).unwrap();
    assert!(backend.list_stashes().unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "3\n");
}

#[test]
fn backend_stash_push_untracked_flag_takes_effect() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.stash_push(Some("with-untracked"), true).unwrap();
    // -u 生效的话未跟踪文件也被收走了
    assert!(
        !dir.path().join("untracked.txt").exists(),
        "include_untracked=true 时未跟踪文件应被一并 stash"
    );

    backend.stash_apply(0).unwrap();
    assert!(dir.path().join("untracked.txt").exists());
}

#[test]
fn backend_stash_drop_removes_entry() {
    let dir = repo();
    let path = p(&dir);
    let backend = backend_for(&path).unwrap();

    backend.stash_push(None, false).unwrap();
    assert_eq!(backend.list_stashes().unwrap().len(), 1);

    backend.stash_drop(0).unwrap();
    assert!(backend.list_stashes().unwrap().is_empty());
    // drop 不恢复改动，工作区应仍是 HEAD 内容
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "2\n");
}
