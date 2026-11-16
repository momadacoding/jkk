#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! merge / cherry-pick / revert 停在冲突上之后的提交收尾（第 74 条）。
//!
//! 每个用例自建临时 fixture 仓库，`TempDir` 析构时清掉。
use app_lib::commands::ops;
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
    assert!(status.success(), "git {args:?} failed");
}

fn try_git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success()
}

fn out(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(o.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn s(p: &Path) -> String {
    p.to_str().unwrap().to_string()
}

fn state_of(dir: &Path) -> String {
    repo_state(s(dir)).unwrap()
}

/// `main` 上 a.txt = "main"，`side` 上 a.txt = "side"，共同祖先是 "base"。
fn diverged_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    run_git(p, &["init", "-q", "-b", "main"]);
    run_git(p, &["config", "user.email", "test@test.com"]);
    run_git(p, &["config", "user.name", "Test"]);
    run_git(p, &["config", "core.autocrlf", "false"]);
    fs::write(p.join("a.txt"), "base\n").unwrap();
    run_git(p, &["add", "."]);
    run_git(p, &["commit", "-qm", "base"]);
    run_git(p, &["checkout", "-qb", "side"]);
    fs::write(p.join("a.txt"), "side\n").unwrap();
    run_git(p, &["commit", "-qam", "side"]);
    run_git(p, &["checkout", "-q", "main"]);
    fs::write(p.join("a.txt"), "main\n").unwrap();
    run_git(p, &["commit", "-qam", "main"]);
    dir
}

/// 把冲突解决成与 HEAD 相同并暂存：之后 Staged 是空的。
fn resolve_to_head(dir: &Path) {
    fs::write(dir.join("a.txt"), "main\n").unwrap();
    run_git(dir, &["add", "a.txt"]);
    assert_eq!(
        out(dir, &["diff", "--cached", "--name-only"]),
        "",
        "Staged 应为空"
    );
}

fn commit(dir: &Path, msg: &str) -> app_lib::error::Result<()> {
    ops::commit(s(dir), msg.to_string(), false)
}

// ---------- 未解决的冲突 ----------

#[test]
fn unresolved_merge_conflict_is_refused_in_chinese_and_nothing_changes() {
    let dir = diverged_repo();
    let p = dir.path();
    assert!(!try_git(p, &["merge", "side"]), "merge 应当冲突");
    let head = out(p, &["rev-parse", "HEAD"]);

    let err = commit(p, "x").unwrap_err();
    assert_eq!(err.code, "commit_unresolved_conflicts");
    assert!(
        err.message.contains("1 个冲突文件未解决"),
        "{}",
        err.message
    );

    assert_eq!(out(p, &["rev-parse", "HEAD"]), head, "不应生成提交");
    assert_eq!(state_of(p), "merge", "合并应仍在进行中");
}

#[test]
fn unresolved_cherry_pick_conflict_is_refused() {
    let dir = diverged_repo();
    let p = dir.path();
    assert!(
        !try_git(p, &["cherry-pick", "side"]),
        "cherry-pick 应当冲突"
    );
    let err = commit(p, "x").unwrap_err();
    assert_eq!(err.code, "commit_unresolved_conflicts");
    assert_eq!(state_of(p), "cherry-pick");
}

#[test]
fn unresolved_conflict_count_covers_every_conflicted_path() {
    let dir = diverged_repo();
    let p = dir.path();
    // 两边各在第二个文件上也撞一次
    run_git(p, &["checkout", "-q", "side"]);
    fs::write(p.join("b.txt"), "side b\n").unwrap();
    run_git(p, &["add", "b.txt"]);
    run_git(p, &["commit", "-qm", "side b"]);
    run_git(p, &["checkout", "-q", "main"]);
    fs::write(p.join("b.txt"), "main b\n").unwrap();
    run_git(p, &["add", "b.txt"]);
    run_git(p, &["commit", "-qm", "main b"]);
    assert!(!try_git(p, &["merge", "side"]), "merge 应当冲突");

    let err = commit(p, "x").unwrap_err();
    assert!(
        err.message.contains("2 个冲突文件未解决"),
        "{}",
        err.message
    );
}

// ---------- 冲突已解决、结果与 HEAD 相同 ----------

#[test]
fn merge_resolved_to_head_with_nothing_staged_commits_and_finishes_the_merge() {
    let dir = diverged_repo();
    let p = dir.path();
    assert!(!try_git(p, &["merge", "side"]), "merge 应当冲突");
    resolve_to_head(p);

    commit(p, "merge side").unwrap();

    assert_eq!(state_of(p), "normal", "MERGE_HEAD 应已消失");
    let parents = out(p, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        parents.split(' ').count(),
        3,
        "应是两个父提交的合并提交：{parents}"
    );
}

#[test]
fn cherry_pick_resolved_to_head_commits_and_finishes_the_cherry_pick() {
    // 裸 `git commit` 在这里报 nothing to commit，cherry-pick 一直停着
    let dir = diverged_repo();
    let p = dir.path();
    let before = out(p, &["rev-parse", "HEAD"]);
    assert!(
        !try_git(p, &["cherry-pick", "side"]),
        "cherry-pick 应当冲突"
    );
    resolve_to_head(p);

    commit(p, "pick side").unwrap();

    assert_eq!(state_of(p), "normal", "CHERRY_PICK_HEAD 应已消失");
    assert_eq!(out(p, &["rev-parse", "HEAD~1"]), before);
}

#[test]
fn revert_resolved_to_head_commits_and_finishes_the_revert() {
    let dir = diverged_repo();
    let p = dir.path();
    // main 上再叠一个改 a.txt 的提交，revert 它的父提交「main」就会冲突
    fs::write(p.join("a.txt"), "later\n").unwrap();
    run_git(p, &["commit", "-qam", "later"]);
    assert!(
        !try_git(p, &["revert", "--no-edit", "HEAD~1"]),
        "revert 应当冲突"
    );
    fs::write(p.join("a.txt"), "later\n").unwrap();
    run_git(p, &["add", "a.txt"]);
    let before = out(p, &["rev-parse", "HEAD"]);

    commit(p, "revert main").unwrap();

    assert_eq!(state_of(p), "normal", "REVERT_HEAD 应已消失");
    assert_eq!(
        out(p, &["rev-parse", "HEAD~1"]),
        before,
        "应在原 HEAD 上生成了提交"
    );
    assert_eq!(out(p, &["log", "-1", "--format=%s"]), "revert main");
}

// ---------- Amend ----------

#[test]
fn amend_during_revert_is_refused_instead_of_rewriting_the_previous_commit() {
    // 实测：revert 进行中 git 不拒绝 --amend，会把上一个提交改写成 revert 的结果、REVERT_HEAD 一并消失
    let dir = diverged_repo();
    let p = dir.path();
    fs::write(p.join("a.txt"), "later\n").unwrap();
    run_git(p, &["commit", "-qam", "later"]);
    assert!(
        !try_git(p, &["revert", "--no-edit", "HEAD~1"]),
        "revert 应当冲突"
    );
    fs::write(p.join("a.txt"), "resolved\n").unwrap();
    run_git(p, &["add", "a.txt"]);
    let head = out(p, &["rev-parse", "HEAD"]);

    let err = ops::commit(s(p), "amended".into(), true).unwrap_err();
    assert_eq!(err.code, "commit_amend_in_progress");
    assert!(
        err.message.contains("revert 进行中不能 Amend"),
        "{}",
        err.message
    );
    assert_eq!(out(p, &["rev-parse", "HEAD"]), head, "上一个提交不应被改写");
    assert_eq!(state_of(p), "revert");
}

#[test]
fn amend_during_merge_is_refused_in_chinese() {
    let dir = diverged_repo();
    let p = dir.path();
    assert!(!try_git(p, &["merge", "side"]), "merge 应当冲突");
    resolve_to_head(p);
    let err = ops::commit(s(p), "amended".into(), true).unwrap_err();
    assert_eq!(err.code, "commit_amend_in_progress");
    assert_eq!(state_of(p), "merge");
}

#[test]
fn amend_outside_any_operation_still_works() {
    let dir = diverged_repo();
    let p = dir.path();
    let parent = out(p, &["rev-parse", "HEAD~1"]);
    ops::commit(s(p), "main amended".into(), true).unwrap();
    assert_eq!(out(p, &["rev-parse", "HEAD~1"]), parent);
    assert_eq!(out(p, &["log", "-1", "--format=%s"]), "main amended");
}

// ---------- 一次挑多个提交（sequencer）----------

/// side 上再叠一个只加 c.txt 的提交，`cherry-pick side~1 side` 停在第一个的冲突上
fn sequence_cherry_pick() -> TempDir {
    let dir = diverged_repo();
    let p = dir.path();
    run_git(p, &["checkout", "-q", "side"]);
    fs::write(p.join("c.txt"), "c\n").unwrap();
    run_git(p, &["add", "c.txt"]);
    run_git(p, &["commit", "-qm", "side c"]);
    run_git(p, &["checkout", "-q", "main"]);
    assert!(
        !try_git(p, &["cherry-pick", "side~1", "side"]),
        "cherry-pick 应当停在第一个提交的冲突上"
    );
    dir
}

#[test]
fn sequence_cherry_pick_reports_its_own_state() {
    // 一次挑多个时，提交一下只了结当前这一个，后面的还在 .git/sequencer 里等 --continue。
    // 状态要和单个 cherry-pick 分开，前端才不会说「提交以完成 cherry-pick」
    let dir = sequence_cherry_pick();
    assert_eq!(state_of(dir.path()), "cherry-pick-sequence");
}

#[test]
fn sequence_cherry_pick_resolved_to_head_is_not_committed_as_empty() {
    // 序列里结果为空的那一个该 --skip（第 51 条），不替用户记一个空提交
    let dir = sequence_cherry_pick();
    let p = dir.path();
    let head = out(p, &["rev-parse", "HEAD"]);
    resolve_to_head(p);
    assert!(commit(p, "pick").is_err());
    assert_eq!(out(p, &["rev-parse", "HEAD"]), head);
    assert_eq!(state_of(p), "cherry-pick-sequence");
}

// ---------- libgit2 读不了的 index：交给 git 自己判断 ----------

#[test]
fn split_index_repo_still_commits() {
    let dir = diverged_repo();
    let p = dir.path();
    run_git(p, &["config", "core.splitIndex", "true"]);
    run_git(p, &["update-index", "--split-index"]);
    fs::write(p.join("d.txt"), "d\n").unwrap();
    run_git(p, &["add", "d.txt"]);
    let before = out(p, &["rev-parse", "HEAD"]);

    commit(p, "add d").unwrap();

    assert_eq!(out(p, &["rev-parse", "HEAD~1"]), before);
}

#[test]
fn merge_in_linked_worktree_is_guarded_and_finished_the_same_way() {
    let dir = diverged_repo();
    run_git(
        dir.path(),
        &["worktree", "add", "-q", "-b", "wt-branch", "wt", "main"],
    );
    let wt = dir.path().join("wt");
    assert!(!try_git(&wt, &["merge", "side"]), "merge 应当冲突");

    let err = commit(&wt, "x").unwrap_err();
    assert_eq!(err.code, "commit_unresolved_conflicts");

    resolve_to_head(&wt);
    commit(&wt, "merge side").unwrap();
    assert_eq!(state_of(&wt), "normal");
    assert_eq!(state_of(dir.path()), "normal");
}

// ---------- 不在任何操作中间：行为不变 ----------

#[test]
fn normal_commit_with_nothing_staged_still_fails() {
    // 空提交只放给 cherry-pick / revert 收尾用；平时什么都没暂存就不该凭空生成提交
    let dir = diverged_repo();
    let p = dir.path();
    let head = out(p, &["rev-parse", "HEAD"]);
    assert!(commit(p, "empty").is_err());
    assert_eq!(out(p, &["rev-parse", "HEAD"]), head);
}
