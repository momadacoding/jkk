#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! `engine::git_read`（libgit2）与它替换掉的 git CLI 查询逐条对照。
//!
//! 这些查询原先走 CLI，改走 libgit2 是为了省掉起进程的开销（C1a 决策日志「性能 P3」）。
//! AGENTS.md §6.2：两条路径在 refs 解析、大小写上可能不一致，所以每个函数都在同一个 fixture
//! 上和原来的 CLI 输出比，而不是只断言「看起来对」。

use app_lib::engine::git_read;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    )
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let (code, out) = git(dir, args);
    assert_eq!(code, 0, "git {args:?} 失败");
    out
}

/// work 上两个提交（第二个改 a、加 Code/中文.cs），side 从第一个分出来另改 a。
fn repo() -> TempDir {
    let d = TempDir::new().unwrap();
    let p = d.path();
    ok(p, &["init", "-q", "-b", "work"]);
    ok(p, &["config", "user.email", "t@t"]);
    ok(p, &["config", "user.name", "T"]);
    ok(p, &["config", "core.autocrlf", "false"]);
    fs::create_dir_all(p.join("Code/Sub")).unwrap();
    fs::write(p.join("a.txt"), "a\n").unwrap();
    fs::write(p.join("Code/Sub/b.cs"), "b\n").unwrap();
    ok(p, &["add", "-A"]);
    ok(p, &["commit", "-qm", "one\n\nSvn-Base-Floor: 1\n"]);
    ok(p, &["branch", "side"]);
    fs::write(p.join("a.txt"), "a2\n").unwrap();
    fs::write(p.join("Code/中文.cs"), "c\n").unwrap();
    ok(p, &["add", "-A"]);
    ok(p, &["commit", "-qm", "two"]);
    ok(p, &["switch", "-q", "side"]);
    fs::write(p.join("a.txt"), "side\n").unwrap();
    ok(p, &["commit", "-qam", "side"]);
    ok(p, &["switch", "-q", "work"]);
    d
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn resolve_matches_rev_parse_verify() {
    let d = repo();
    let p = d.path();
    for spec in [
        "HEAD",
        "refs/heads/work",
        "side",
        "HEAD^1",
        "HEAD^{tree}",
        "HEAD^1^{commit}",
        "HEAD:a.txt",
        "HEAD:Code/中文.cs",
    ] {
        let (code, cli) = git(p, &["rev-parse", "-q", "--verify", spec]);
        assert_eq!(code, 0, "{spec}");
        assert_eq!(git_read::resolve(s(p), spec).unwrap(), Some(cli), "{spec}");
    }
    for spec in [
        "refs/heads/nope",
        "HEAD^1^1",
        "HEAD:nope.txt",
        "HEAD^{tree}^{commit}",
    ] {
        let (code, _) = git(p, &["rev-parse", "-q", "--verify", spec]);
        assert_ne!(code, 0, "前提：CLI 认为 {spec} 不存在");
        assert_eq!(git_read::resolve(s(p), spec).unwrap(), None, "{spec}");
    }
}

#[test]
fn a_garbage_ref_is_an_error_although_the_cli_calls_it_absent() {
    // 「ref 损坏」当成「不存在」会让调用方以为基线没建，覆盖掉一段真实历史。
    // **这里有意与 CLI 不同**：CLI 警告 `ignoring broken ref` 后退出 1，等于说「没有」；
    // 我们报错，宁可让操作停下（独立 review 2026-11-01 实测）
    let d = repo();
    let p = d.path();
    fs::write(p.join(".git/refs/heads/broken"), "not an oid\n").unwrap();
    let (code, _) = git(p, &["rev-parse", "-q", "--verify", "refs/heads/broken"]);
    assert_eq!(code, 1);
    assert!(git_read::resolve(s(p), "refs/heads/broken").is_err());
}

#[test]
fn a_dangling_ref_is_an_error_not_absent() {
    // ref 文件是合法 OID、对象却不在（断电丢对象、对象库被外部工具弄坏）。CLI 照样打印
    // 这个 OID；libgit2 报 NotFound（类别 Odb），不能因此说成「没有基线」
    let d = repo();
    let p = d.path();
    let missing = "0123456789abcdef0123456789abcdef01234567";
    fs::write(p.join(".git/refs/heads/dangling"), format!("{missing}\n")).unwrap();
    let (code, out) = git(p, &["rev-parse", "-q", "--verify", "refs/heads/dangling"]);
    assert_eq!((code, out.trim()), (0, missing));
    assert!(git_read::resolve(s(p), "refs/heads/dangling").is_err());
    assert!(git_read::resolve(s(p), "refs/heads/dangling^{commit}").is_err());
}

#[test]
fn symbolic_head_matches_symbolic_ref_including_detached() {
    let d = repo();
    let p = d.path();
    assert_eq!(
        git_read::symbolic_head(s(p)).unwrap().as_deref(),
        Some(ok(p, &["symbolic-ref", "-q", "HEAD"]).as_str())
    );
    let head = ok(p, &["rev-parse", "HEAD"]);
    ok(p, &["switch", "-q", "--detach", &head]);
    assert_ne!(git(p, &["symbolic-ref", "-q", "HEAD"]).0, 0);
    assert_eq!(git_read::symbolic_head(s(p)).unwrap(), None);
}

#[test]
fn commit_message_is_the_raw_body_of_cat_file() {
    let d = repo();
    let p = d.path();
    let raw = Command::new("git")
        .current_dir(p)
        .args(["cat-file", "commit", "HEAD^"])
        .output()
        .unwrap()
        .stdout;
    let at = raw.windows(2).position(|w| w == b"\n\n").unwrap();
    assert_eq!(
        git_read::commit_message(s(p), "HEAD^").unwrap(),
        &raw[at + 2..]
    );
}

#[test]
fn merge_base_and_is_ancestor_match_the_cli() {
    let d = repo();
    let p = d.path();
    assert_eq!(
        git_read::merge_base(s(p), "work", "side").unwrap(),
        Some(ok(p, &["merge-base", "work", "side"]))
    );
    for (a, b) in [
        ("HEAD^", "HEAD"),
        ("HEAD", "HEAD"),
        ("HEAD", "HEAD^"),
        ("side", "work"),
    ] {
        let cli = git(p, &["merge-base", "--is-ancestor", a, b]).0 == 0;
        assert_eq!(git_read::is_ancestor(s(p), a, b).unwrap(), cli, "{a} {b}");
    }
    // 没有共同祖先
    ok(p, &["switch", "-q", "--orphan", "lonely"]);
    fs::write(p.join("z.txt"), "z\n").unwrap();
    ok(p, &["add", "z.txt"]);
    ok(p, &["commit", "-qm", "lonely"]);
    assert_ne!(git(p, &["merge-base", "lonely", "work"]).0, 0);
    assert_eq!(git_read::merge_base(s(p), "lonely", "work").unwrap(), None);
}

#[test]
fn tree_paths_matches_ls_tree_recursive_name_only() {
    let d = repo();
    let p = d.path();
    let cli: std::collections::BTreeSet<String> = ok(
        p,
        &[
            "-c",
            "core.quotePath=false",
            "ls-tree",
            "-r",
            "--name-only",
            "HEAD",
        ],
    )
    .lines()
    .map(str::to_string)
    .collect();
    assert_eq!(git_read::tree_paths(s(p), "HEAD").unwrap(), cli);
}

#[test]
fn unmerged_paths_matches_diff_filter_u() {
    let d = repo();
    let p = d.path();
    assert!(git_read::unmerged_paths(s(p)).unwrap().is_empty());
    assert_ne!(git(p, &["merge", "side"]).0, 0, "前提：合并冲突");
    let cli: Vec<String> = ok(p, &["diff", "--name-only", "--diff-filter=U"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(cli, vec!["a.txt".to_string()]);
    assert_eq!(git_read::unmerged_paths(s(p)).unwrap(), cli);
}

#[test]
fn ignore_case_follows_core_ignorecase() {
    let d = repo();
    let p = d.path();
    ok(p, &["config", "core.ignoreCase", "true"]);
    assert!(git_read::ignore_case(s(p)));
    ok(p, &["config", "core.ignoreCase", "false"]);
    assert!(!git_read::ignore_case(s(p)));
    // 仓库里不设时取全局 / 系统配置，与 CLI 同一个来源
    ok(p, &["config", "--unset", "core.ignoreCase"]);
    let cli = git(p, &["config", "--bool", "--get", "core.ignoreCase"]).1 == "true";
    assert_eq!(git_read::ignore_case(s(p)), cli);
}

#[test]
fn git_dir_matches_rev_parse_absolute_git_dir() {
    let d = repo();
    let p = d.path();
    let cli = ok(p, &["rev-parse", "--absolute-git-dir"]);
    let got = git_read::git_dir(s(p)).unwrap();
    assert_eq!(
        fs::canonicalize(&got).unwrap(),
        fs::canonicalize(&cli).unwrap()
    );
    // linked worktree 里是 `.git/worktrees/<名字>`
    let wt = d.path().join("wt");
    ok(p, &["worktree", "add", "-q", wt.to_str().unwrap(), "side"]);
    let cli = ok(&wt, &["rev-parse", "--absolute-git-dir"]);
    assert_eq!(
        fs::canonicalize(git_read::git_dir(wt.to_str().unwrap()).unwrap()).unwrap(),
        fs::canonicalize(&cli).unwrap()
    );
}
