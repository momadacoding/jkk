#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! Local-Only 标记可取消、可拆分（2026-11-11 使用报告 9.2；设计见
//! `docs/superpowers/plans/2026-11-15-local-only-toggle-decisions.md`）。
//!
//! 全部在临时 fixture 上跑。改写只用 git 对象与 ref，不需要 SVN：fixture 是一个带 `svn/base` 分支的
//! 普通 git 仓库，形状取自报告：基线之上一个普通 commit、一个 Local-Only commit、再一个普通 commit。

use app_lib::engine::svn_journal::{self, Flow, Journal, Phase};
use app_lib::engine::svn_local_only_edit::{
    local_only_files, set_local_only, split_local_only, EditOutcome,
};
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const L_MSG: &str = "add @avatarclass\n\nLocal-Only: true";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "author")
        .env("GIT_AUTHOR_EMAIL", "author@example.invalid")
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn commit(dir: &Path, msg: &str, date: &str) -> String {
    git(dir, &["add", "-A"]);
    let out = Command::new("git")
        .args(["commit", "-q", "-m", msg])
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "原作者")
        .env("GIT_AUTHOR_EMAIL", "orig@example.invalid")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    git(dir, &["rev-parse", "HEAD"])
}

struct Repo {
    _tmp: TempDir,
    dir: std::path::PathBuf,
    /// 基线之上的普通 commit。
    a: String,
    /// Local-Only commit：改了 Proxy/ImpChat.lua 与 Proxy/ImpDatabase.lua。
    l: String,
    /// Local-Only 之后的普通 commit。
    b: String,
}

impl Repo {
    fn path(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
    fn git(&self, args: &[&str]) -> String {
        git(&self.dir, args)
    }
    fn msg(&self, rev: &str) -> String {
        self.git(&["log", "-1", "--format=%B", rev])
    }
    fn changed(&self, rev: &str) -> String {
        self.git(&["diff-tree", "--no-commit-id", "-r", "--name-only", rev])
    }
}

/// `svn/base` ← A ← L(Local-Only) ← B，HEAD 在 work。
fn repo() -> Repo {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    git(&dir, &["init", "-q", "-b", "work"]);
    git(&dir, &["config", "core.autocrlf", "false"]);
    write(&dir, "Proxy/ImpChat.lua", "chat\n");
    write(&dir, "Proxy/ImpDatabase.lua", "db\n");
    write(&dir, "Proxy/Other.lua", "other\n");
    commit(&dir, "baseline", "1700000000 +0800");
    git(&dir, &["branch", "svn/base"]);
    write(&dir, "Proxy/Other.lua", "other 2\n");
    let a = commit(&dir, "normal A", "1700000100 +0800");
    write(&dir, "Proxy/ImpChat.lua", "---@class Avatar\nchat\n");
    write(&dir, "Proxy/ImpDatabase.lua", "---@class Avatar\ndb\n");
    let l = commit(&dir, L_MSG, "1700000200 +0800");
    write(&dir, "Proxy/Other.lua", "other 3\n");
    let b = commit(&dir, "normal B\n\nbody line", "1700000300 +0800");
    Repo {
        _tmp: tmp,
        dir,
        a,
        l,
        b,
    }
}

fn ok(r: Result<EditOutcome, app_lib::error::GitError>) -> EditOutcome {
    r.unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}

fn err_code(r: Result<EditOutcome, app_lib::error::GitError>) -> String {
    match r {
        Ok(o) => panic!("应当拒绝，却改写了：{o:?}"),
        Err(e) => e.code,
    }
}

#[test]
fn unmarking_rewrites_only_the_message_and_keeps_every_tree() {
    let r = repo();
    let old_tip_tree = r.git(&["rev-parse", "HEAD^{tree}"]);
    let out = ok(set_local_only(&r.path(), "op", &r.l, false));

    assert_eq!(out.branch, "work");
    assert_eq!(out.old_tip, r.b);
    assert_eq!(r.git(&["rev-parse", "HEAD"]), out.tip);
    assert_ne!(out.tip, r.b);
    // 末端的树逐字节相同
    assert_eq!(r.git(&["rev-parse", "HEAD^{tree}"]), old_tip_tree);
    // L' 在 A 之上，说明去掉了标记，树与 L 相同
    let l2 = r.git(&["rev-parse", "HEAD~1"]);
    assert_eq!(r.git(&["rev-parse", "HEAD~2"]), r.a);
    assert_eq!(r.msg(&l2), "add @avatarclass");
    assert_eq!(
        r.git(&["rev-parse", &format!("{l2}^{{tree}}")]),
        r.git(&["rev-parse", &format!("{}^{{tree}}", r.l)])
    );
    // B' 的说明原样
    assert_eq!(r.msg("HEAD"), "normal B\n\nbody line");
    // 作者原样，提交者是现在的人
    assert_eq!(
        r.git(&["log", "-1", "--format=%an|%ae|%at", &l2]),
        "原作者|orig@example.invalid|1700000200"
    );
    assert_eq!(
        r.git(&["log", "-1", "--format=%an|%at", "HEAD"]),
        "原作者|1700000300"
    );
    // reflog 里能找回改写前的末端
    let reflog = r.git(&["reflog", "-1", "--format=%gs", "work"]);
    assert!(reflog.starts_with("mygitclient:"), "{reflog}");
}

#[test]
fn the_working_tree_and_the_index_are_not_touched() {
    let r = repo();
    // 一处已暂存、一处没暂存的改动，一个没跟踪的文件
    write(
        &r.dir,
        "Proxy/ImpChat.lua",
        "---@class Avatar\nchat staged\n",
    );
    r.git(&["add", "Proxy/ImpChat.lua"]);
    write(
        &r.dir,
        "Proxy/ImpChat.lua",
        "---@class Avatar\nchat staged and more\n",
    );
    write(&r.dir, "new.txt", "x");
    let status = r.git(&["status", "--porcelain"]);
    let staged = r.git(&["diff", "--cached"]);

    ok(set_local_only(&r.path(), "op", &r.l, false));

    assert_eq!(r.git(&["status", "--porcelain"]), status);
    assert_eq!(r.git(&["diff", "--cached"]), staged);
    assert_eq!(
        std::fs::read_to_string(r.dir.join("Proxy/ImpChat.lua")).unwrap(),
        "---@class Avatar\nchat staged and more\n"
    );
}

#[test]
fn marking_adds_the_trailer_as_its_own_paragraph() {
    let r = repo();
    ok(set_local_only(&r.path(), "op", &r.b, true));
    assert_eq!(r.msg("HEAD"), "normal B\n\nbody line\n\nLocal-Only: true");
    // B 是末端：只改它自己，A、L 原样
    assert_eq!(r.git(&["rev-parse", "HEAD~1"]), r.l);
}

#[test]
fn unmarking_drops_only_the_trailer_line_and_the_blank_lines_it_leaves() {
    let r = repo();
    write(&r.dir, "Proxy/Other.lua", "other 4\n");
    let c = commit(
        &r.dir,
        "subject\n\nbody\n\nSigned-off-by: x <x@example.invalid>\nLocal-Only: true\n",
        "1700000400 +0800",
    );
    ok(set_local_only(&r.path(), "op", &c, false));
    assert_eq!(
        r.msg("HEAD"),
        "subject\n\nbody\n\nSigned-off-by: x <x@example.invalid>"
    );
}

#[test]
fn other_branches_that_contain_the_commit_are_listed_and_left_alone() {
    let r = repo();
    r.git(&["branch", "feature", &r.b]);
    r.git(&["branch", "older", &r.a]);
    let out = ok(set_local_only(&r.path(), "op", &r.l, false));
    assert_eq!(out.other_branches, vec!["feature".to_string()]);
    assert_eq!(r.git(&["rev-parse", "feature"]), r.b);
    assert_eq!(r.git(&["rev-parse", "older"]), r.a);
}

#[test]
fn splitting_moves_the_chosen_files_into_a_normal_commit_before_the_local_only_one() {
    let r = repo();
    let old_tip_tree = r.git(&["rev-parse", "HEAD^{tree}"]);
    let out = ok(split_local_only(
        &r.path(),
        "op",
        &r.l,
        &["Proxy/ImpDatabase.lua".to_string()],
    ));

    assert_eq!(r.git(&["rev-parse", "HEAD^{tree}"]), old_tip_tree);
    assert_eq!(out.tip, r.git(&["rev-parse", "HEAD"]));
    // A ← C1（普通，只有 ImpDatabase）← C2（Local-Only，只有 ImpChat）← B'
    let c2 = r.git(&["rev-parse", "HEAD~1"]);
    let c1 = r.git(&["rev-parse", "HEAD~2"]);
    assert_eq!(r.git(&["rev-parse", "HEAD~3"]), r.a);
    assert_eq!(r.changed(&c1), "Proxy/ImpDatabase.lua");
    assert_eq!(r.msg(&c1), "add @avatarclass");
    assert_eq!(r.changed(&c2), "Proxy/ImpChat.lua");
    assert_eq!(r.msg(&c2), L_MSG);
    assert_eq!(
        r.git(&["log", "-1", "--format=%an|%at", &c1]),
        "原作者|1700000200"
    );
    assert_eq!(r.msg("HEAD"), "normal B\n\nbody line");
}

#[test]
fn splitting_handles_a_deleted_file() {
    let r = repo();
    std::fs::remove_file(r.dir.join("Proxy/Other.lua")).unwrap();
    write(&r.dir, "Proxy/ImpChat.lua", "---@class Avatar\nchat 2\n");
    let c = commit(
        &r.dir,
        "local cleanup\n\nLocal-Only: true",
        "1700000500 +0800",
    );
    ok(split_local_only(
        &r.path(),
        "op",
        &c,
        &["Proxy/Other.lua".to_string()],
    ));
    let c1 = r.git(&["rev-parse", "HEAD~1"]);
    assert_eq!(r.changed(&c1), "Proxy/Other.lua");
    assert!(r
        .git(&["ls-tree", "-r", "--name-only", &c1])
        .lines()
        .all(|l| l != "Proxy/Other.lua"));
    assert_eq!(r.changed("HEAD"), "Proxy/ImpChat.lua");
}

#[test]
fn refuses_when_the_commit_is_already_in_that_state() {
    let r = repo();
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.l, true)),
        "local_only_edit_no_change"
    );
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.a, false)),
        "local_only_edit_no_change"
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), r.b);
}

#[test]
fn refuses_commits_in_the_baseline_or_off_the_current_branch() {
    let r = repo();
    let base = r.git(&["rev-parse", "svn/base"]);
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &base, true)),
        "local_only_edit_in_baseline"
    );
    r.git(&["checkout", "-q", "-b", "side", &r.a]);
    write(&r.dir, "side.txt", "s");
    let s = commit(&r.dir, "side\n\nLocal-Only: true", "1700000600 +0800");
    r.git(&["checkout", "-q", "work"]);
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &s, false)),
        "local_only_edit_not_on_branch"
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), r.b);
}

#[test]
fn refuses_a_detached_head_and_the_baseline_branch() {
    let r = repo();
    r.git(&["checkout", "-q", "--detach", "HEAD"]);
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.l, false)),
        "local_only_edit_not_on_branch"
    );
    r.git(&["checkout", "-q", "svn/base"]);
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.l, false)),
        "local_only_edit_not_on_branch"
    );
}

#[test]
fn refuses_a_merge_in_the_range() {
    let r = repo();
    r.git(&["checkout", "-q", "-b", "side", &r.l]);
    write(&r.dir, "side.txt", "s");
    commit(&r.dir, "side", "1700000600 +0800");
    r.git(&["checkout", "-q", "work"]);
    r.git(&["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let tip = r.git(&["rev-parse", "HEAD"]);
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.l, false)),
        "local_only_edit_not_linear"
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
}

#[test]
fn refuses_while_an_svn_sync_is_unfinished() {
    let r = repo();
    svn_journal::write(
        &r.path(),
        &Journal {
            opid: "op-old".into(),
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.l, false)),
        "local_only_edit_unfinished_sync"
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), r.b);
}

#[test]
fn refuses_while_git_is_in_the_middle_of_a_merge() {
    let r = repo();
    let git_dir = r.dir.join(".git");
    std::fs::write(git_dir.join("MERGE_HEAD"), format!("{}\n", r.a)).unwrap();
    assert_eq!(
        err_code(set_local_only(&r.path(), "op", &r.l, false)),
        "local_only_edit_git_busy"
    );
}

#[test]
fn split_refuses_bad_selections() {
    let r = repo();
    let p = r.path();
    assert_eq!(
        err_code(split_local_only(&p, "op", &r.l, &[])),
        "local_only_split_bad_selection"
    );
    assert_eq!(
        err_code(split_local_only(
            &p,
            "op",
            &r.l,
            &["Proxy/Other.lua".to_string()]
        )),
        "local_only_split_bad_selection"
    );
    assert_eq!(
        err_code(split_local_only(
            &p,
            "op",
            &r.l,
            &[
                "Proxy/ImpChat.lua".to_string(),
                "Proxy/ImpDatabase.lua".to_string()
            ]
        )),
        "local_only_split_bad_selection"
    );
    assert_eq!(
        err_code(split_local_only(
            &p,
            "op",
            &r.b,
            &["Proxy/Other.lua".to_string()]
        )),
        "local_only_split_not_local_only"
    );
    assert_eq!(r.git(&["rev-parse", "HEAD"]), r.b);
}

#[test]
fn the_dialog_lists_exactly_the_paths_the_split_accepts() {
    let r = repo();
    assert_eq!(
        local_only_files(&r.path(), &r.l).unwrap(),
        vec!["Proxy/ImpChat.lua", "Proxy/ImpDatabase.lua"]
    );
}

#[test]
fn the_outcome_names_the_rewritten_commit_so_the_graph_can_select_it() {
    let r = repo();
    let out = ok(set_local_only(&r.path(), "op", &r.l, false));
    assert_eq!(out.commit, r.git(&["rev-parse", "HEAD~1"]));
    let r = repo();
    let out = ok(split_local_only(
        &r.path(),
        "op",
        &r.l,
        &["Proxy/ImpDatabase.lua".to_string()],
    ));
    // 拆分时是留下的那个 Local-Only commit
    assert_eq!(out.commit, r.git(&["rev-parse", "HEAD~1"]));
    assert_eq!(r.msg(&out.commit), L_MSG);
}

#[test]
fn split_refuses_a_commit_that_turned_a_directory_into_a_file() {
    let r = repo();
    write(&r.dir, "Proxy/Dir/x.lua", "x\n");
    commit(&r.dir, "add dir", "1700000400 +0800");
    std::fs::remove_dir_all(r.dir.join("Proxy/Dir")).unwrap();
    write(&r.dir, "Proxy/Dir", "now a file\n");
    write(&r.dir, "Proxy/ImpChat.lua", "chat changed\n");
    let c = commit(
        &r.dir,
        "dir to file\n\nLocal-Only: true",
        "1700000500 +0800",
    );
    let tip = r.git(&["rev-parse", "HEAD"]);
    for pick in ["Proxy/Dir", "Proxy/Dir/x.lua", "Proxy/ImpChat.lua"] {
        assert_eq!(
            err_code(split_local_only(&r.path(), "op", &c, &[pick.to_string()])),
            "local_only_split_dir_file",
            "{pick}"
        );
    }
    assert_eq!(r.git(&["rev-parse", "HEAD"]), tip);
}

#[test]
fn split_accepts_windows_style_paths() {
    let r = repo();
    ok(split_local_only(
        &r.path(),
        "op",
        &r.l,
        &["Proxy\\ImpDatabase.lua".to_string()],
    ));
    assert_eq!(r.changed("HEAD~2"), "Proxy/ImpDatabase.lua");
}
