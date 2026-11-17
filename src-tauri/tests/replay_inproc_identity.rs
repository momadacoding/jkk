#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 重放的读改走 libgit2 之后，重放出来的提交与原来走 git 命令时逐个 OID 相同
//! （日志 `2026-11-17-replay-inproc-reads-decisions.md`，计划 2026-11-16 §5 C 的应对）。
//!
//! 同一段分支重放两次：第一次走进程内的读；第二次仓库里放一条与历史无关的 `refs/replace/`，
//! `git_read` 的每个新读都因此退回 git 命令。提交者与时间用环境变量钉住，两次的 OID 必须逐个相同。
//!
//! 单独一个测试二进制：要改进程的环境变量，不能和别的测试并发。

use app_lib::engine::git_cli::run_git;
use app_lib::engine::git_read;
use app_lib::engine::svn_replay::{
    commits_between, replay_onto, ReplayCodes, ReplayCommit, ReplayOpts, ReplayOutcome,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const CODES: ReplayCodes = ReplayCodes {
    conflict: "t_conflict",
    too_old: "t_too_old",
    too_old_message: "git 太旧",
    failed: "t_failed",
};

fn git(p: &Path, args: &[&str]) -> String {
    run_git(p.to_str().unwrap(), args)
        .unwrap_or_else(|e| panic!("git {args:?}: {}", e.message))
        .trim()
        .to_string()
}

/// 用 `commit-tree -F` 在 HEAD 上造提交并移动分支：作者、日期、说明逐字节由测试决定。
fn commit(p: &Path, files: &[(&str, &str)], author: (&str, &str, &str), msg: &[u8]) -> String {
    for (rel, body) in files {
        let f = p.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, body).unwrap();
    }
    git(p, &["add", "-A"]);
    let tree = git(p, &["write-tree"]);
    let f = p.join(".git").join("test-msg");
    fs::write(&f, msg).unwrap();
    let out = Command::new("git")
        .current_dir(p)
        .args([
            "commit-tree",
            &tree,
            "-p",
            "HEAD",
            "-F",
            f.to_str().unwrap(),
        ])
        .env("GIT_AUTHOR_NAME", author.0)
        .env("GIT_AUTHOR_EMAIL", author.1)
        .env("GIT_AUTHOR_DATE", author.2)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
    git(p, &["update-ref", "HEAD", &oid]);
    oid
}

fn replay(
    p: &Path,
    base: &str,
    head: &str,
    onto: &str,
) -> (String, BTreeMap<String, String>, Vec<String>) {
    let repo = p.to_str().unwrap();
    let commits = commits_between(repo, base, head, &CODES).unwrap();
    // 第二个提交里 b.txt 换回父提交的版本（走 ls-tree 那条路）
    let second = commits[1].oid.clone();
    let superseded = move |c: &ReplayCommit| -> BTreeSet<String> {
        if c.oid == second {
            BTreeSet::from(["Code/b.txt".to_string()])
        } else {
            BTreeSet::new()
        }
    };
    let opts = ReplayOpts {
        drop_empty: true,
        superseded: &superseded,
        codes: CODES,
    };
    match replay_onto(repo, onto, &commits, &opts, &mut BTreeMap::new()).unwrap() {
        ReplayOutcome::Done {
            tip, map, dropped, ..
        } => (tip, map, dropped),
        other => panic!("应当 Done：{other:?}"),
    }
}

#[test]
fn replaying_through_libgit2_writes_the_same_commits_as_through_git() {
    let d = TempDir::new().unwrap();
    let p = d.path();
    git(p, &["init", "-q", "-b", "work"]);
    git(p, &["config", "user.name", "Base"]);
    git(p, &["config", "user.email", "base@x"]);
    git(p, &["config", "core.autocrlf", "false"]);
    // commit-tree 的提交者取环境：钉住，两次重放才可能逐字节相同
    std::env::set_var("GIT_COMMITTER_NAME", "Replayer");
    std::env::set_var("GIT_COMMITTER_EMAIL", "replayer@x");
    std::env::set_var("GIT_COMMITTER_DATE", "1750000000 +0800");

    let me = ("张三", "zhang@x", "1700000000 -0800");
    fs::write(p.join("seed"), "seed\n").unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-qm", "base"]);
    let base = git(p, &["rev-parse", "HEAD"]);

    commit(
        p,
        &[("Code/a.txt", "a1\n")],
        me,
        "改 a\n\n第二段\r\n".as_bytes(),
    );
    commit(
        p,
        &[("Code/b.txt", "b1\n"), ("Code/c.txt", "c1\n")],
        me,
        b"b and c",
    );
    commit(
        p,
        &[("Code/d.txt", "d-svn\n")],
        ("Other", "o@x", "1700000500 +0000"),
        b"\n\nsame as svn\n",
    );
    commit(p, &[], me, b"originally empty\n");
    commit(
        p,
        &[("Code/e.txt", "e\n")],
        ("T", "t@t", "1700000900 -0000"),
        b"last\n\n\n",
    );
    let head = git(p, &["rev-parse", "HEAD"]);

    // 新基线：已经有 d 的内容（第三个提交重放后变空），另有一个文件
    git(p, &["checkout", "-q", "--detach", &base]);
    let onto = commit(
        p,
        &[("Code/d.txt", "d-svn\n"), ("Code/svn.txt", "svn\n")],
        ("SVN", "svn@x", "1700001000 +0000"),
        b"baseline\n",
    );

    assert!(
        git_read::commit_meta(p.to_str().unwrap(), &head).is_ok(),
        "前提：第一次走进程内的读"
    );
    let fast = replay(p, &base, &head, &onto);
    assert_eq!(fast.2.len(), 1, "前提：变空的那个被丢弃 {fast:?}");

    // 与历史无关的替换：只为了让 git_read 的新读全部退回 git 命令
    let x = git(
        p,
        &[
            "commit-tree",
            &git(p, &["rev-parse", "HEAD^{tree}"]),
            "-m",
            "x",
        ],
    );
    let y = git(
        p,
        &[
            "commit-tree",
            &git(p, &["rev-parse", "HEAD^{tree}"]),
            "-m",
            "y",
        ],
    );
    git(p, &["replace", &x, &y]);
    assert!(
        git_read::commit_meta(p.to_str().unwrap(), &head).is_err()
            && git_read::trees_of(p.to_str().unwrap(), &[head.as_str()]).is_err()
            && git_read::tree_entries(p.to_str().unwrap(), &head, &[]).is_err(),
        "前提：第二次全走 git 命令"
    );
    let slow = replay(p, &base, &head, &onto);

    assert_eq!(fast, slow);
}
