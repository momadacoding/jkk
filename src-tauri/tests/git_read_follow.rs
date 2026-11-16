#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进关键路径上改走 libgit2 的读操作，与它替换掉的 git CLI 逐个场景对照
//! （日志 `2026-11-16-follow-git-inproc-reads-decisions.md`）。
//!
//! AGENTS.md §6.2：libgit2 与 git CLI 在 refs、大小写、索引标志上都可能不一致，所以每个函数都在
//! 同一个 fixture 上和原来那条命令的结果比，而不是只断言「看起来对」。CLI 一侧照抄调用点原来的
//! 命令和解析。

use app_lib::engine::git_read;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn git_raw(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> Vec<u8> {
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git not found");
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn ok(dir: &Path, args: &[&str]) -> String {
    String::from_utf8(git_raw(dir, args, None)).unwrap()
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

/// 两个提交、三条分支（含嵌套名字的 `feature/x`）。
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
    ok(p, &["branch", "side"]);
    ok(p, &["branch", "feature/x"]);
    fs::write(p.join("a.txt"), "a2\n").unwrap();
    ok(p, &["commit", "-qam", "two"]);
    d
}

// ---------------------------------------------------------------------------
// hash-object --no-filters --stdin（不写对象库）
// ---------------------------------------------------------------------------

#[test]
fn blob_oid_matches_hash_object_without_filters() {
    let d = repo();
    let p = d.path();
    // autocrlf 打开时 --no-filters 也不能换行尾：比的是字节
    ok(p, &["config", "core.autocrlf", "true"]);
    for bytes in [
        &b""[..],
        b"a\n",
        b"a\r\nb\r\n",
        "中文\n".as_bytes(),
        &[0u8, 159, 146, 150][..],
    ] {
        let cli = String::from_utf8(git_raw(
            p,
            &["hash-object", "--no-filters", "--stdin"],
            Some(bytes),
        ))
        .unwrap();
        assert_eq!(git_read::blob_oid(s(p), bytes).unwrap(), cli.trim());
    }
    // 不写对象库
    let oid = git_read::blob_oid(s(p), b"never written\n").unwrap();
    let check = String::from_utf8(git_raw(
        p,
        &["cat-file", "--batch-check"],
        Some(format!("{oid}\n").as_bytes()),
    ))
    .unwrap();
    assert!(check.contains("missing"), "{check}");
}

// ---------------------------------------------------------------------------
// cat-file --batch-check：对象库里确实有的 blob
// ---------------------------------------------------------------------------

fn blobs_by_cli(p: &Path, oids: &[&str]) -> BTreeSet<String> {
    let input: String = oids.iter().map(|o| format!("{o}\n")).collect();
    String::from_utf8(git_raw(
        p,
        &["cat-file", "--batch-check"],
        Some(input.as_bytes()),
    ))
    .unwrap()
    .lines()
    .filter_map(|l| {
        let mut f = l.split(' ');
        let (oid, kind) = (f.next()?, f.next()?);
        (kind == "blob").then(|| oid.to_string())
    })
    .collect()
}

#[test]
fn present_blobs_match_cat_file_batch_check() {
    let d = repo();
    let p = d.path();
    let blob = ok(p, &["rev-parse", "HEAD:a.txt"]).trim().to_string();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let commit = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    // i-t-a 在 diff-index 里是空 blob，对象库里未必有
    let empty = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
    let missing = "1111111111111111111111111111111111111111";
    // 打包进 pack 的也要找得到
    ok(p, &["gc", "-q"]);
    let oids = [
        blob.as_str(),
        tree.as_str(),
        commit.as_str(),
        empty,
        missing,
    ];
    let fast = git_read::present_blobs(s(p), oids.iter().copied()).unwrap();
    assert_eq!(fast, blobs_by_cli(p, &oids));
    assert_eq!(fast, BTreeSet::from([blob.clone()]));
}

// ---------------------------------------------------------------------------
// for-each-ref --format=%(refname) %(objectname) %(upstream) refs/heads/
// ---------------------------------------------------------------------------

fn branches_by_cli(p: &Path) -> Vec<(String, String, bool)> {
    ok(
        p,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(upstream)",
            "refs/heads/",
        ],
    )
    .lines()
    .filter_map(|line| {
        let mut f = line.split(' ');
        let (name, oid) = (f.next()?, f.next()?);
        let name = name.strip_prefix("refs/heads/")?;
        Some((
            name.to_string(),
            oid.to_string(),
            f.next().is_some_and(|u| !u.is_empty()),
        ))
    })
    .collect()
}

#[test]
fn local_branches_match_for_each_ref() {
    let d = repo();
    let p = d.path();
    // 有 upstream 且远端分支在 / 有 upstream 但远端分支还没取回 / 上游是本地分支（remote = .）
    ok(p, &["update-ref", "refs/remotes/origin/work", "HEAD"]);
    ok(
        p,
        &[
            "config",
            "remote.origin.url",
            "https://example.invalid/r.git",
        ],
    );
    ok(
        p,
        &[
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    );
    ok(p, &["config", "branch.work.remote", "origin"]);
    ok(p, &["config", "branch.work.merge", "refs/heads/work"]);
    ok(p, &["config", "branch.side.remote", "origin"]);
    ok(
        p,
        &["config", "branch.side.merge", "refs/heads/never-fetched"],
    );
    ok(p, &["config", "branch.feature/x.remote", "."]);
    ok(p, &["config", "branch.feature/x.merge", "refs/heads/work"]);
    // 打包的 ref 也要列出来
    ok(p, &["pack-refs", "--all"]);
    ok(p, &["branch", "loose"]);
    let cli = branches_by_cli(p);
    assert_eq!(cli.len(), 4, "前提：{cli:?}");
    assert_eq!(git_read::local_branches(s(p)).unwrap(), cli);
}

#[test]
fn a_branch_whose_upstream_remote_is_not_defined() {
    // 只配了 branch.x.remote / merge、没有那个 remote
    let d = repo();
    let p = d.path();
    ok(p, &["config", "branch.side.remote", "gone"]);
    ok(p, &["config", "branch.side.merge", "refs/heads/side"]);
    let cli = branches_by_cli(p);
    assert_eq!(git_read::local_branches(s(p)).unwrap(), cli);
}

// ---------------------------------------------------------------------------
// for-each-ref --sort=-refname --sort=-committerdate --format=%(refname) <prefix>
// ---------------------------------------------------------------------------

#[test]
fn refs_newest_first_match_for_each_ref_sorting() {
    let d = repo();
    let p = d.path();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    // 同一秒的两条按名字倒序，不同秒的按提交时间倒序
    for (name, when) in [
        ("100a", "1700000000"),
        ("100b", "1700000000"),
        ("090z", "1700000500"),
        ("200a", "1690000000"),
    ] {
        let out = Command::new("git")
            .current_dir(p)
            .args(["commit-tree", &tree, "-m", name])
            .env("GIT_COMMITTER_DATE", format!("{when} +0800"))
            .output()
            .unwrap();
        let oid = String::from_utf8(out.stdout).unwrap();
        ok(
            p,
            &[
                "update-ref",
                &format!("refs/mygitclient/follow/{name}"),
                oid.trim(),
            ],
        );
    }
    let cli: Vec<String> = ok(
        p,
        &[
            "for-each-ref",
            "--sort=-refname",
            "--sort=-committerdate",
            "--format=%(refname)",
            "refs/mygitclient/follow/",
        ],
    )
    .lines()
    .map(str::to_string)
    .collect();
    assert_eq!(cli.len(), 4);
    assert_eq!(
        git_read::refs_newest_first(s(p), "refs/mygitclient/follow/").unwrap(),
        cli
    );
}

// ---------------------------------------------------------------------------
// diff-tree -r -z --no-renames <a> <b>（raw / --name-only / --name-status）
// ---------------------------------------------------------------------------

/// 用底层命令搭一棵树：`(mode, 内容或 commit OID, 路径)`。不经过工作区，符号链接、gitlink、
/// 大小写不同的同名路径在 Windows 上也搭得出来。
fn tree_of(p: &Path, entries: &[(&str, &str, &str)]) -> String {
    let index = p.join(".git").join("tmp-index");
    let _ = fs::remove_file(&index);
    let mut input = Vec::new();
    for (mode, content, path) in entries {
        let oid = if *mode == "160000" {
            content.to_string()
        } else {
            String::from_utf8(git_raw(
                p,
                &["hash-object", "-w", "--no-filters", "--stdin"],
                Some(content.as_bytes()),
            ))
            .unwrap()
            .trim()
            .to_string()
        };
        input.extend_from_slice(format!("{mode} {oid}\t{path}").as_bytes());
        input.push(0);
    }
    let run = |args: &[&str], stdin: Option<&[u8]>| {
        let mut child = Command::new("git")
            .current_dir(p)
            .args(args)
            .env("GIT_INDEX_FILE", &index)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.unwrap_or_default())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{args:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    run(&["update-index", "-z", "--index-info"], Some(&input));
    let tree = run(&["write-tree"], None);
    let _ = fs::remove_file(&index);
    tree.trim().to_string()
}

fn commit_of(p: &Path, tree: &str, parent: Option<&str>, msg: &str) -> String {
    let mut args = vec!["commit-tree", tree];
    if let Some(parent) = parent {
        args.extend(["-p", parent]);
    }
    String::from_utf8(git_raw(p, &args, Some(msg.as_bytes())))
        .unwrap()
        .trim()
        .to_string()
}

/// 两棵树之间各种形状的变化都摆一遍。
fn two_trees() -> (TempDir, String, String) {
    let d = repo();
    let p = d.path();
    // gitlink 不在这里：它交回 git（`gitlinks_are_left_to_git`）
    let old = tree_of(
        p,
        &[
            ("100644", "same\n", "keep.txt"),
            ("100644", "v1\n", "mod.cs"),
            ("100644", "gone\n", "del/x.cs"),
            ("100644", "x\n", "exec.sh"),
            ("100644", "target\n", "link"),
            ("100644", "f\n", "becomes-dir"),
            ("100644", "c\n", "Case.cs"),
            ("100644", "中\n", "中文/文件.cs"),
            ("100644", "sp\n", "with space/a b.txt"),
            ("100644", "dot\n", "a.txt"),
        ],
    );
    let new = tree_of(
        p,
        &[
            ("100644", "same\n", "keep.txt"),
            ("100644", "v2\n", "mod.cs"),
            ("100644", "new\n", "add/y.cs"),
            ("100755", "x\n", "exec.sh"),
            ("120000", "target", "link"),
            ("100644", "inner\n", "becomes-dir/inner.cs"),
            ("100644", "c\n", "case.cs"),
            ("100644", "中2\n", "中文/文件.cs"),
            ("100644", "sp2\n", "with space/a b.txt"),
            ("100644", "slash\n", "a/b.txt"),
            ("100644", "dot\n", "a.txt"),
        ],
    );
    let old_c = commit_of(p, &old, None, "old\n");
    let new_c = commit_of(p, &new, Some(&old_c), "new\n");
    (d, old_c, new_c)
}

fn raw_by_cli(p: &Path, a: &str, b: &str) -> Vec<git_read::RawChange> {
    let out = git_raw(p, &["diff-tree", "-r", "-z", "--no-renames", a, b], None);
    let mut fields = out.split(|b| *b == 0).filter(|f| !f.is_empty());
    let mut v = Vec::new();
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta = String::from_utf8(meta.to_vec()).unwrap();
        let f: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        v.push(git_read::RawChange {
            old_mode: f[0].to_string(),
            new_mode: f[1].to_string(),
            old_oid: f[2].to_string(),
            new_oid: f[3].to_string(),
            status: f[4].to_string(),
            path: String::from_utf8(path.to_vec()).unwrap(),
        });
    }
    v
}

#[test]
fn tree_changes_match_diff_tree_raw() {
    let (d, old, new) = two_trees();
    let p = d.path();
    let cli = raw_by_cli(p, &old, &new);
    let statuses: BTreeSet<&str> = cli.iter().map(|c| c.status.as_str()).collect();
    assert_eq!(
        statuses,
        BTreeSet::from(["A", "D", "M", "T"]),
        "前提：四种变化都有 {cli:?}"
    );
    assert_eq!(git_read::tree_changes(s(p), &old, &new).unwrap(), cli);
    // 反过来比
    assert_eq!(
        git_read::tree_changes(s(p), &new, &old).unwrap(),
        raw_by_cli(p, &new, &old)
    );
    // 两边一样：没有变化
    assert!(git_read::tree_changes(s(p), &old, &old).unwrap().is_empty());
}

#[test]
fn tree_changes_name_status_and_name_only_match_cli() {
    let (d, old, new) = two_trees();
    let p = d.path();
    let fast = git_read::tree_changes(s(p), &old, &new).unwrap();
    let name_status = ok(
        p,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--name-status",
            &old,
            &new,
        ],
    );
    let mine: String = fast
        .iter()
        .map(|c| format!("{}\0{}\0", c.status, c.path))
        .collect();
    assert_eq!(mine, name_status);
    let name_only = ok(
        p,
        &[
            "--literal-pathspecs",
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            &old,
            &new,
        ],
    );
    let mine: String = fast.iter().map(|c| format!("{}\0", c.path)).collect();
    assert_eq!(mine, name_only);
}

// ---------------------------------------------------------------------------
// log --reverse --topo-order --format=%H%x00%P%x00%s%x00%B%x1e <base>..<head>
// ---------------------------------------------------------------------------

/// 一条线性历史，提交说明是各种难对齐的形状。
fn linear_history() -> (TempDir, String, String, Vec<&'static str>) {
    let d = repo();
    let p = d.path();
    let base = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let messages = vec![
        "plain\n",
        "no trailing newline",
        "\n\n  leading blank lines\n\nbody\n",
        "multi\nline subject  \nhere\t\n\nbody one\n\nbody two\n",
        "crlf subject\r\nsecond\r\n\r\nbody\r\n",
        "中文主题\n\nLocal-Only: yes\n",
        "trailing   \n\n\n",
        "   \n",
    ];
    let mut head = base.clone();
    for m in &messages {
        head = commit_of(p, &tree, Some(&head), m);
    }
    (d, base, head, messages)
}

fn commits_by_cli(p: &Path, base: &str, head: &str) -> Vec<git_read::LogCommit> {
    let out = ok(
        p,
        &[
            "log",
            "--reverse",
            "--topo-order",
            "--format=%H%x00%P%x00%s%x00%B%x1e",
            &format!("{base}..{head}"),
        ],
    );
    out.split('\u{1e}')
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
        .map(|r| {
            let f: Vec<&str> = r.splitn(4, '\0').collect();
            git_read::LogCommit {
                oid: f[0].to_string(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                subject: f[2].to_string(),
                body: f[3].to_string(),
            }
        })
        .collect()
}

#[test]
fn linear_commits_match_git_log() {
    let (d, base, head, messages) = linear_history();
    let p = d.path();
    let cli = commits_by_cli(p, &base, &head);
    assert_eq!(cli.len(), messages.len());
    assert_eq!(git_read::linear_commits(s(p), &base, &head).unwrap(), cli);
    // 空区间
    assert!(git_read::linear_commits(s(p), &head, &head)
        .unwrap()
        .is_empty());
}

#[test]
fn a_range_with_a_merge_is_not_taken() {
    // 有 merge 时 topo-order 的排法两边可能不同：交回 git log
    let d = repo();
    let p = d.path();
    let base = ok(p, &["rev-parse", "HEAD~1"]).trim().to_string();
    let a = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let b = commit_of(p, &tree, Some(&base), "b\n");
    let merge = String::from_utf8(git_raw(
        p,
        &["commit-tree", &tree, "-p", &a, "-p", &b],
        Some(b"merge\n"),
    ))
    .unwrap();
    assert!(git_read::linear_commits(s(p), &base, merge.trim()).is_err());
}

#[test]
fn a_commit_with_a_declared_encoding_is_not_taken() {
    let d = repo();
    let p = d.path();
    let base = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    let tree = ok(p, &["rev-parse", "HEAD^{tree}"]).trim().to_string();
    let raw = format!(
        "tree {tree}\nparent {base}\nauthor T <t@t> 1700000000 +0800\n\
         committer T <t@t> 1700000000 +0800\nencoding GBK\n\nmsg\n"
    );
    let oid = String::from_utf8(git_raw(
        p,
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        Some(raw.as_bytes()),
    ))
    .unwrap();
    assert!(git_read::linear_commits(s(p), &base, oid.trim()).is_err());
    assert!(git_read::bodies(s(p), &base, oid.trim()).is_err());
}

#[test]
fn bodies_match_git_log_b() {
    let (d, base, head, _) = linear_history();
    let p = d.path();
    let cli: BTreeSet<String> = ok(p, &["log", "--format=%B%x1e", &format!("{base}..{head}")])
        .split('\u{1e}')
        .map(|r| r.trim_start_matches('\n').to_string())
        .filter(|r| !r.is_empty())
        .collect();
    // 调用方原来按 `\x1e` 切、去掉开头的换行（记录之间的那个换行与说明自己开头的空行分不开），
    // 两边经过同样的处理再比
    let fast: BTreeSet<String> = git_read::bodies(s(p), &base, &head)
        .unwrap()
        .into_iter()
        .map(|r| r.trim_start_matches('\n').to_string())
        .filter(|r| !r.is_empty())
        .collect();
    assert_eq!(fast, cli);
}

// ---------------------------------------------------------------------------
// 索引：ls-files -v -z / --stage -z / -z / -u，diff-index --cached --no-renames -z
// ---------------------------------------------------------------------------

fn git_env(p: &Path, args: &[&str], index: &Path, stdin: Option<&[u8]>) -> Vec<u8> {
    let mut child = Command::new("git")
        .current_dir(p)
        .args(args)
        .env("GIT_INDEX_FILE", index)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.unwrap_or_default())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// 真实索引里各种形状的条目都摆一遍：相对 HEAD 有暂存的增、删、改、mode 变化、类型变化、
/// 大小写不同的改名，外加 i-t-a、skip-worktree、assume-unchanged、两者都有。
fn busy_index() -> TempDir {
    let d = repo();
    let p = d.path();
    fs::create_dir_all(p.join("中文")).unwrap();
    for (f, c) in [
        ("keep.txt", "k\n"),
        ("mod.cs", "m1\n"),
        ("del.cs", "d\n"),
        ("exec.sh", "x\n"),
        ("Case.cs", "c\n"),
        ("skip.cs", "s\n"),
        ("assume.cs", "a\n"),
        ("both.cs", "b\n"),
        ("to-link", "t\n"),
        ("ita-over.cs", "o\n"),
        ("中文/文件.cs", "z\n"),
        ("with space.txt", "w\n"),
    ] {
        fs::write(p.join(f), c).unwrap();
    }
    ok(p, &["add", "-A"]);
    ok(p, &["commit", "-qm", "busy"]);
    fs::write(p.join("mod.cs"), "m2\n").unwrap();
    fs::write(p.join("new.cs"), "n\n").unwrap();
    fs::write(p.join("ita.cs"), "i\n").unwrap();
    ok(p, &["add", "mod.cs", "new.cs"]);
    ok(p, &["add", "-N", "ita.cs"]);
    // 树里有、索引里换成了 i-t-a 的路径
    ok(p, &["rm", "-q", "--cached", "ita-over.cs"]);
    ok(p, &["add", "-N", "ita-over.cs"]);
    ok(p, &["rm", "-q", "--cached", "del.cs"]);
    ok(p, &["update-index", "--chmod=+x", "exec.sh"]);
    // 大小写不同的改名、文件 → 符号链接：只改索引，不靠文件系统
    let blob = |c: &str| {
        String::from_utf8(git_raw(
            p,
            &["hash-object", "-w", "--stdin"],
            Some(c.as_bytes()),
        ))
        .unwrap()
        .trim()
        .to_string()
    };
    let (case_blob, link_blob) = (blob("c\n"), blob("target"));
    let info = format!(
        "0 {z}\tCase.cs\x00100644 {case_blob}\tcase.cs\x00120000 {link_blob}\tto-link\x00",
        z = "0".repeat(40)
    );
    git_raw(
        p,
        &["update-index", "-z", "--index-info"],
        Some(info.as_bytes()),
    );
    ok(
        p,
        &["update-index", "--skip-worktree", "skip.cs", "both.cs"],
    );
    ok(
        p,
        &["update-index", "--assume-unchanged", "assume.cs", "both.cs"],
    );
    d
}

fn index_of(p: &Path) -> std::path::PathBuf {
    p.join(".git").join("index")
}

#[test]
fn ls_files_listings_match_cli_byte_for_byte() {
    let d = busy_index();
    let p = d.path();
    let records = git_read::index_records(s(p), &index_of(p)).unwrap();
    let cli_v = String::from_utf8(git_raw(p, &["ls-files", "-v", "-z"], None)).unwrap();
    for tag in ["H ", "S ", "h ", "s "] {
        assert!(
            cli_v.split('\0').any(|r| r.starts_with(tag)),
            "前提：有 {tag:?} 标记 {cli_v:?}"
        );
    }
    assert_eq!(git_read::ls_files_v(&records).unwrap(), cli_v);
    let cli_stage = String::from_utf8(git_raw(p, &["ls-files", "--stage", "-z"], None)).unwrap();
    assert!(cli_stage.contains("120000 "), "前提：有符号链接条目");
    assert_eq!(git_read::ls_files_stage(&records).unwrap(), cli_stage);
    let cli_plain = String::from_utf8(git_raw(p, &["ls-files", "-z"], None)).unwrap();
    assert_eq!(git_read::ls_files(&records).unwrap(), cli_plain);
    assert!(!git_read::has_unmerged(&records));
}

#[test]
fn an_index_file_given_by_path_reads_like_git_index_file() {
    // 跟进拼的索引在 .git/index.lock，候选索引在 .git 里的临时文件：都靠 GIT_INDEX_FILE 指过去
    let d = busy_index();
    let p = d.path();
    let other = p.join(".git").join("other-index");
    fs::copy(index_of(p), &other).unwrap();
    git_env(
        p,
        &["update-index", "--no-skip-worktree", "skip.cs"],
        &other,
        None,
    );
    let records = git_read::index_records(s(p), &other).unwrap();
    assert_eq!(
        git_read::ls_files_v(&records).unwrap(),
        String::from_utf8(git_env(p, &["ls-files", "-v", "-z"], &other, None)).unwrap()
    );
    // 不存在的索引文件：git 当成空索引
    let none = p.join(".git").join("no-such-index");
    let records = git_read::index_records(s(p), &none).unwrap();
    assert_eq!(
        git_read::ls_files_stage(&records).unwrap(),
        String::from_utf8(git_env(p, &["ls-files", "--stage", "-z"], &none, None)).unwrap()
    );
}

#[test]
fn index_version_4_reads_the_same() {
    let d = busy_index();
    let p = d.path();
    ok(p, &["update-index", "--index-version", "4"]);
    let records = git_read::index_records(s(p), &index_of(p)).unwrap();
    assert_eq!(
        git_read::ls_files_stage(&records).unwrap(),
        String::from_utf8(git_raw(p, &["ls-files", "--stage", "-z"], None)).unwrap()
    );
    assert_eq!(
        git_read::ls_files_v(&records).unwrap(),
        String::from_utf8(git_raw(p, &["ls-files", "-v", "-z"], None)).unwrap()
    );
}

#[test]
fn a_split_index_is_not_taken() {
    let d = busy_index();
    let p = d.path();
    ok(p, &["update-index", "--split-index"]);
    assert!(git_read::index_records(s(p), &index_of(p)).is_err());
}

#[test]
fn a_sparse_index_is_not_taken() {
    let d = busy_index();
    let p = d.path();
    ok(p, &["config", "index.sparse", "true"]);
    assert!(git_read::index_records(s(p), &index_of(p)).is_err());
}

#[test]
fn conflicts_are_seen_and_conflicted_listings_are_not_emulated() {
    let d = repo();
    let p = d.path();
    let blob = |c: &str| {
        String::from_utf8(git_raw(
            p,
            &["hash-object", "-w", "--stdin"],
            Some(c.as_bytes()),
        ))
        .unwrap()
        .trim()
        .to_string()
    };
    let (b1, b2, b3) = (blob("1\n"), blob("2\n"), blob("3\n"));
    let info = format!(
        "0 {z}\ta.txt\x00100644 {b1} 1\ta.txt\x00100644 {b2} 2\ta.txt\x00100644 {b3} 3\ta.txt\x00",
        z = "0".repeat(40)
    );
    git_raw(
        p,
        &["update-index", "-z", "--index-info"],
        Some(info.as_bytes()),
    );
    let cli_u = git_raw(p, &["ls-files", "-u", "-z"], None);
    assert!(!cli_u.is_empty(), "前提：有冲突条目");
    let records = git_read::index_records(s(p), &index_of(p)).unwrap();
    assert!(git_read::has_unmerged(&records));
    // 有非 0 stage 时不冒充 ls-files 的列表：交回 git
    assert!(git_read::ls_files_v(&records).is_err());
    assert!(git_read::ls_files_stage(&records).is_err());
}

#[test]
fn replace_refs_are_left_to_git() {
    // git 的 log / diff-tree / cat-file 默认按 refs/replace/ 换掉对象，libgit2 不换：两边会给出
    // 不同的答案。仓库里有替换时一律交回 git
    let (d, old, new) = two_trees();
    let p = d.path();
    let other = commit_of(
        p,
        ok(p, &["rev-parse", &format!("{old}^{{tree}}")]).trim(),
        None,
        "x\n",
    );
    ok(p, &["replace", &new, &other]);
    let cli = raw_by_cli(p, &old, &new);
    assert!(
        cli.is_empty(),
        "前提：替换生效，git 看到的 NEW 与 OLD 同树 {cli:?}"
    );
    assert!(git_read::tree_changes(s(p), &old, &new).is_err());
    assert!(git_read::linear_commits(s(p), &old, &new).is_err());
    assert!(git_read::bodies(s(p), &old, &new).is_err());
    assert!(git_read::present_blobs(s(p), [old.as_str()]).is_err());
    // for-each-ref 的 committerdate 读替换之后的 commit
    assert!(git_read::refs_newest_first(s(p), "refs/heads/").is_err());
}

#[test]
fn grafts_and_shallow_are_left_to_git() {
    for file in ["info/grafts", "shallow"] {
        let (d, old, new) = two_trees();
        let p = d.path();
        let path = p.join(".git").join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, format!("{new}\n")).unwrap();
        assert!(git_read::tree_changes(s(p), &old, &new).is_err(), "{file}");
        assert!(
            git_read::linear_commits(s(p), &old, &new).is_err(),
            "{file}"
        );
    }
}

#[test]
fn a_non_utf8_log_output_encoding_is_left_to_git() {
    // 中文 Windows 上常见 i18n.logOutputEncoding = gbk：git log 会把说明转码后输出
    for key in ["i18n.logOutputEncoding", "i18n.commitEncoding"] {
        let (d, base, head, _) = linear_history();
        let p = d.path();
        ok(p, &["config", key, "gbk"]);
        assert!(
            git_read::linear_commits(s(p), &base, &head).is_err(),
            "{key}"
        );
        assert!(git_read::bodies(s(p), &base, &head).is_err(), "{key}");
    }
    let (d, base, head, _) = linear_history();
    let p = d.path();
    ok(p, &["config", "i18n.logOutputEncoding", "UTF-8"]);
    assert_eq!(
        git_read::linear_commits(s(p), &base, &head).unwrap(),
        commits_by_cli(p, &base, &head)
    );
}

#[test]
fn gitlinks_are_left_to_git() {
    // 子模块的 ignore 设置会影响 diff：一律交回 git，不比
    let d = repo();
    let p = d.path();
    let head = ok(p, &["rev-parse", "HEAD"]).trim().to_string();
    let info = format!("160000 {head}\tsub\x00");
    git_raw(
        p,
        &["update-index", "-z", "--index-info"],
        Some(info.as_bytes()),
    );
    let tree = ok(p, &["write-tree"]).trim().to_string();
    assert!(git_read::tree_changes(s(p), &head, &tree).is_err());
}

// ---------------------------------------------------------------------------
// worktree list --porcelain 里的 `branch refs/heads/*`
// ---------------------------------------------------------------------------

fn worktree_branches_by_cli(p: &Path) -> BTreeSet<String> {
    ok(p, &["worktree", "list", "--porcelain"])
        .lines()
        .filter_map(|l| l.strip_prefix("branch refs/heads/"))
        .map(str::to_string)
        .collect()
}

#[test]
fn worktree_branches_match_worktree_list() {
    let d = repo();
    let p = d.path().join("main");
    fs::create_dir(&p).unwrap();
    // 主工作区放在子目录里，worktree 建在旁边
    for f in std::fs::read_dir(d.path()).unwrap() {
        let f = f.unwrap();
        if f.file_name() != "main" {
            fs::rename(f.path(), p.join(f.file_name())).unwrap();
        }
    }
    let wt = |name: &str| d.path().join(name);
    ok(&p, &["worktree", "add", "-q", s(&wt("wt-side")), "side"]);
    ok(
        &p,
        &["worktree", "add", "-q", "--detach", s(&wt("wt-detached"))],
    );
    ok(
        &p,
        &["worktree", "add", "-q", s(&wt("wt-gone")), "feature/x"],
    );
    // 目录被删掉、还没 prune 的 worktree：git 仍然列出它，分支照样算检出
    fs::remove_dir_all(wt("wt-gone")).unwrap();
    let cli = worktree_branches_by_cli(&p);
    assert_eq!(
        cli,
        BTreeSet::from(["work".into(), "side".into(), "feature/x".into()]),
        "前提"
    );
    assert_eq!(git_read::worktree_branches(s(&p)).unwrap(), cli);
    // 从 linked worktree 里问，结果一样
    assert_eq!(git_read::worktree_branches(s(&wt("wt-side"))).unwrap(), cli);

    // `ref:` 后面没有空格、带 CRLF：git 都认（独立 review 第 2 条）
    let side_head = p
        .join(".git")
        .join("worktrees")
        .join("wt-side")
        .join("HEAD");
    for text in ["ref:refs/heads/side\n", "ref: \trefs/heads/side\r\n"] {
        fs::write(&side_head, text).unwrap();
        let cli = worktree_branches_by_cli(&p);
        assert!(cli.contains("side"), "前提：git 认 {text:?}：{cli:?}");
        assert_eq!(git_read::worktree_branches(s(&p)).unwrap(), cli, "{text:?}");
    }
    // 读不懂的 HEAD 不能当成游离（那会漏报一条已检出的分支）：交回 git
    fs::write(&side_head, "garbage\n").unwrap();
    assert!(git_read::worktree_branches(s(&p)).is_err());
}
