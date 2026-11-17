#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 重放路径上改走 libgit2 的读操作，与它替换掉的 git CLI 逐个场景对照
//! （日志 `2026-11-17-replay-inproc-reads-decisions.md`）。
//!
//! AGENTS.md §6.2：CLI 一侧照抄调用点原来的命令和解析，比的是调用点最终拿到的值。认不出来的形状
//! 必须返回 `Err`（调用方经 `or_cli` 退回 git），不能给出和 git 不同的答案。

use app_lib::engine::git_read::{self, CommitMeta};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .envs(env.iter().copied())
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn ok(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git_env(dir, args, &[])).unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn repo() -> TempDir {
    let d = TempDir::new().unwrap();
    let p = d.path();
    ok(p, &["init", "-q", "-b", "work"]);
    ok(p, &["config", "user.email", "t@t"]);
    ok(p, &["config", "user.name", "T"]);
    ok(p, &["config", "core.autocrlf", "false"]);
    fs::write(p.join("a.txt"), "a\n").unwrap();
    ok(p, &["add", "-A"]);
    ok(p, &["commit", "-qm", "one"]);
    d
}

/// 用 `commit-tree -F` 造一个提交：作者、日期、说明逐字节由测试决定。
fn commit_with(p: &Path, name: &str, email: &str, date: &str, msg: &[u8]) -> String {
    let f = p.join(".git").join("test-msg");
    fs::write(&f, msg).unwrap();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]);
    let out = git_env(
        p,
        &["commit-tree", tree.trim(), "-p", "HEAD", "-F", s(&f)],
        &[
            ("GIT_AUTHOR_NAME", name),
            ("GIT_AUTHOR_EMAIL", email),
            ("GIT_AUTHOR_DATE", date),
        ],
    );
    String::from_utf8(out).unwrap().trim().to_string()
}

// ---------------------------------------------------------------------------
// rev-parse <rev>^{tree}
// ---------------------------------------------------------------------------

#[test]
fn trees_of_matches_rev_parse() {
    let d = repo();
    let p = d.path();
    fs::write(p.join("b.txt"), "b\n").unwrap();
    ok(p, &["add", "-A"]);
    ok(p, &["commit", "-qm", "two"]);
    let head = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    let first = ok(p, &["rev-parse", "HEAD^"]).trim().to_string();
    let cli: Vec<String> = ok(
        p,
        &[
            "rev-parse",
            &format!("{head}^{{tree}}"),
            &format!("{first}^{{tree}}"),
        ],
    )
    .lines()
    .map(str::to_string)
    .collect();
    assert_eq!(
        git_read::trees_of(s(p), &[head.as_str(), first.as_str()]).unwrap(),
        cli
    );
    assert!(git_read::trees_of(s(p), &[]).unwrap().is_empty());
}

#[test]
fn trees_of_hands_replaced_commits_back_to_git() {
    // git 取树时按 refs/replace/ 换掉提交，libgit2 不换：有替换就不接
    let d = repo();
    let p = d.path();
    let head = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    let other = commit_with(p, "X", "x@x", "1700000000 +0000", b"other\n");
    ok(p, &["replace", &head, &other]);
    assert!(git_read::trees_of(s(p), &[head.as_str()]).is_err());
}

// ---------------------------------------------------------------------------
// log -1 --date=raw --format=%an%x00%ae%x00%ad%x00%B（commit_like 的解析照抄）
// ---------------------------------------------------------------------------

fn meta_by_cli(p: &Path, rev: &str) -> CommitMeta {
    let meta = ok(
        p,
        &[
            "log",
            "-1",
            "--date=raw",
            "--format=%an%x00%ae%x00%ad%x00%B",
            rev,
        ],
    );
    let mut f = meta.splitn(4, '\0');
    CommitMeta {
        name: f.next().unwrap_or("").to_string(),
        email: f.next().unwrap_or("").to_string(),
        date: f.next().unwrap_or("").to_string(),
        message: f.next().unwrap_or("").trim_end_matches('\n').to_string(),
    }
}

#[test]
fn commit_meta_matches_git_log() {
    let d = repo();
    let p = d.path();
    let cases: &[(&str, &str, &str, &[u8])] = &[
        ("T", "t@t", "1700000000 +0800", b"subject\n\nbody line\n"),
        ("T", "t@t", "1700000000 -0800", b"no trailing newline"),
        ("T", "t@t", "1700000000 +0530", b"\n\nleading blank lines\n"),
        ("T", "t@t", "1700000000 +0000", b"crlf\r\nbody\r\n"),
        ("T", "t@t", "1700000000 -0000", b"many trailing\n\n\n\n"),
        (
            "张三",
            "zhang@例子.cn",
            "100000000 +0100",
            "中文说明\n\n第二段\n".as_bytes(),
        ),
        (
            "Name With  Spaces",
            "n@x",
            "1700000000 +1400",
            b"inner spaces\n",
        ),
        ("T", "t@t", "1700000000 +0000", b""),
        (
            "T",
            "t@t",
            "1700000000 +0000",
            b"Local-Only: true\n\nmore\n",
        ),
    ];
    for (name, email, date, msg) in cases {
        let oid = commit_with(p, name, email, date, msg);
        let cli = meta_by_cli(p, &oid);
        assert_eq!(
            git_read::commit_meta(s(p), &oid).unwrap(),
            cli,
            "{name:?} {date:?} {:?}",
            String::from_utf8_lossy(msg)
        );
    }
}

#[test]
fn commit_meta_ignores_mailmap_like_git_does() {
    // log.mailmap 默认开着，但只影响 %aN / %aE；%an / %ae 仍是提交里原样的作者（git 2.51 实测）。
    // 重放沿用的就该是原样的作者：映射之后再写进新提交，作者就被改了
    let d = repo();
    let p = d.path();
    let oid = commit_with(p, "Old", "old@x", "1700000000 +0000", b"m\n");
    fs::write(p.join(".mailmap"), "New <new@x> Old <old@x>\n").unwrap();
    let mapped = String::from_utf8(git_env(p, &["log", "-1", "--format=%aN", &oid], &[])).unwrap();
    assert_eq!(mapped.trim(), "New", "前提：.mailmap 生效了");
    let cli = meta_by_cli(p, &oid);
    assert_eq!((cli.name.as_str(), cli.email.as_str()), ("Old", "old@x"));
    assert_eq!(git_read::commit_meta(s(p), &oid).unwrap(), cli);
}

#[test]
fn commit_meta_hands_other_encodings_back_to_git() {
    let d = repo();
    let p = d.path();
    let oid = commit_with(p, "Old", "old@x", "1700000000 +0000", b"m\n");

    // 输出编码不是 UTF-8：git 会转码
    ok(p, &["config", "i18n.logOutputEncoding", "gbk"]);
    assert!(git_read::commit_meta(s(p), &oid).is_err());
    ok(p, &["config", "--unset", "i18n.logOutputEncoding"]);

    // 提交带 encoding 头
    ok(p, &["config", "i18n.commitEncoding", "ISO-8859-1"]);
    let latin = commit_with(p, "L", "l@x", "1700000000 +0000", b"caf\xe9\n");
    ok(p, &["config", "--unset", "i18n.commitEncoding"]);
    assert!(git_read::commit_meta(s(p), &latin).is_err());
}

#[test]
fn commit_meta_hands_malformed_idents_back_to_git() {
    // 时区不是四位、分钟超过 59：git 按整数原样打（%+05d），libgit2 会换算成别的样子
    let d = repo();
    let p = d.path();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]);
    for author in [
        "author T <t@t> 1700000000 +9999",
        "author T <t@t> 1700000000 +08",
        "author T <t@t> 01700000000 +0800",
        "author T  <t@t> 1700000000 +0800",
        "author T <t@t> > 1700000000 +0800",
    ] {
        let raw = format!(
            "tree {}\nauthor{}\ncommitter T <t@t> 1700000000 +0000\n\nm\n",
            tree.trim(),
            author.trim_start_matches("author")
        );
        check_raw(p, &raw, author);
    }
    // 两条 author：git 取最后一条（pretty.c 的 parse_commit_header 不停下），独立 review 第 1 条
    let t = tree.trim();
    for raw in [
        format!("tree {t}\nauthor A <a@x> 1700000000 +0000\nauthor B <b@x> 1700000001 +0100\ncommitter C <c@x> 1700000000 +0000\n\nm\n"),
        format!("tree {t}\nauthor A <a@x> 1700000000 +0000\ncommitter C <c@x> 1700000000 +0000\nauthor B <b@x> 1700000001 +0100\n\nm\n"),
    ] {
        let oid = check_raw(p, &raw, &raw);
        assert_eq!(meta_by_cli(p, &oid).name, "B", "前提：git 取最后一条");
        assert!(git_read::commit_meta(s(p), &oid).is_err(), "{raw}");
    }
    // 头后面没有空行：libgit2 把说明当成头，git 的 %B 读到对象末尾之外（独立 review nit 3）
    let raw = format!(
        "tree {t}\nauthor A <a@x> 1700000000 +0000\ncommitter C <c@x> 1700000000 +0000\nno blank line\n"
    );
    let oid = check_raw(p, &raw, &raw);
    assert!(git_read::commit_meta(s(p), &oid).is_err());
}

/// 写一个原样的提交对象：`commit_meta` 要么交回 git，要么与 git 一字不差。返回它的 OID。
fn check_raw(p: &Path, raw: &str, what: &str) -> String {
    let f = p.join(".git").join("raw-commit");
    fs::write(&f, raw).unwrap();
    let oid = ok(
        p,
        &["hash-object", "-t", "commit", "-w", "--literally", s(&f)],
    )
    .trim()
    .to_string();
    if let Ok(m) = git_read::commit_meta(s(p), &oid) {
        // 接了就必须和 git 一样
        assert_eq!(m, meta_by_cli(p, &oid), "{what}");
    }
    oid
}

// ---------------------------------------------------------------------------
// ls-tree --full-tree -z <rev> -- <paths>（svn_replay / svn_snapshot 的解析照抄）
// ---------------------------------------------------------------------------

fn entries_by_cli(p: &Path, rev: &str, paths: &[&str]) -> BTreeMap<String, (String, String)> {
    let wanted: BTreeSet<&str> = paths.iter().copied().collect();
    let mut args = vec![
        "--literal-pathspecs",
        "ls-tree",
        "--full-tree",
        "-z",
        rev,
        "--",
    ];
    args.extend_from_slice(paths);
    let out = ok(p, &args);
    let mut found = BTreeMap::new();
    for rec in out.split('\0').filter(|r| !r.is_empty()) {
        let (meta, path) = rec.split_once('\t').unwrap();
        if !wanted.contains(path) {
            continue;
        }
        let mut m = meta.split(' ');
        let (mode, _, oid) = (m.next().unwrap(), m.next().unwrap(), m.next().unwrap());
        found.insert(path.to_string(), (mode.to_string(), oid.to_string()));
    }
    found
}

#[test]
fn tree_entries_match_ls_tree() {
    let d = repo();
    let p = d.path();
    ok(p, &["config", "core.ignorecase", "true"]);
    let w = |rel: &str, body: &str| {
        let f = p.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, body).unwrap();
    };
    w("Code/A.cs", "a\n");
    w("Code/sub/B.cs", "b\n");
    w("Code/[x].cs", "glob\n");
    w("Code/中文.lua", "z\n");
    w("Code/run.sh", "#!/bin/sh\n");
    ok(p, &["add", "-A"]);
    ok(p, &["update-index", "--chmod=+x", "Code/run.sh"]);
    let blob = ok(p, &["rev-parse", "HEAD:a.txt"]);
    // 符号链接与子模块指针直接写进索引：Windows 上不一定建得出真的
    ok(
        p,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("120000,{},Code/link", blob.trim()),
        ],
    );
    let head = ok(p, &["rev-parse", "HEAD"]);
    ok(
        p,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},Code/module", head.trim()),
        ],
    );
    ok(p, &["commit", "-qm", "tree"]);
    let rev = ok(p, &["rev-parse", "HEAD"]).trim().to_string();

    let paths = [
        "Code/A.cs",
        "Code/sub/B.cs",
        "Code/sub",
        "Code/[x].cs",
        "Code/中文.lua",
        "Code/run.sh",
        "Code/link",
        "Code/module",
        "Code/missing.cs",
        "Code/A.cs/inside",
        "code/a.cs",
        "a.txt",
    ];
    let owned = |ps: &[&str]| ps.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let cli = entries_by_cli(p, &rev, &paths);
    // 有更深的路径时 ls-tree 钻进目录、不列目录自己（git 2.51 实测）
    assert!(!cli.contains_key("Code/sub"), "前提：{cli:?}");
    assert!(cli.contains_key("Code/sub/B.cs"), "前提：{cli:?}");
    assert!(!cli.contains_key("code/a.cs"), "前提：ls-tree 区分大小写");
    assert_eq!(
        git_read::tree_entries(s(p), &rev, &owned(&paths)).unwrap(),
        cli
    );

    // 只问目录：列出目录自己的 tree 条目
    let alone = ["Code/sub", "Code/A.cs"];
    let cli = entries_by_cli(p, &rev, &alone);
    assert_eq!(
        cli.get("Code/sub").map(|e| e.0.as_str()),
        Some("040000"),
        "前提：{cli:?}"
    );
    assert_eq!(
        git_read::tree_entries(s(p), &rev, &owned(&alone)).unwrap(),
        cli
    );

    // 更深的路径不存在也一样钻进去
    let ghost = ["Code/sub", "Code/sub/nothing.cs"];
    let cli = entries_by_cli(p, &rev, &ghost);
    assert!(cli.is_empty(), "前提：{cli:?}");
    assert_eq!(
        git_read::tree_entries(s(p), &rev, &owned(&ghost)).unwrap(),
        cli
    );
}

#[test]
fn tree_entries_hand_unusual_paths_back_to_git() {
    // 末尾带 `/` 时 ls-tree 列的是目录里面的条目，与「这条路径的条目」不是一回事
    let d = repo();
    let p = d.path();
    let rev = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    for bad in [
        "",
        "dir/",
        "/a.txt",
        "a//b",
        "./a.txt",
        "a/../a.txt",
        "a\\b",
    ] {
        assert!(
            git_read::tree_entries(s(p), &rev, &[bad.to_string()]).is_err(),
            "{bad:?}"
        );
    }
}
