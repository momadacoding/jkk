#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! `git_read::line_stats` 与 `git diff --numstat` 逐路径对照（计划
//! `2026-11-16-follow-report-usability.md` 的 B：跟进报告写「分支比 SVN 少 7 行」）。
//!
//! AGENTS.md §6.2：libgit2 与 git CLI 可能不一致，所以不只断言「看起来对」，而是在同一个 fixture
//! 上和 git 的结果比。

use app_lib::engine::git_read::{self, LineStat};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn ok(dir: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn lines(n: usize, eol: &str) -> String {
    (1..=n).map(|i| format!("line {i}{eol}")).collect()
}

/// `git diff --numstat` 的结果，二进制记成 `binary: true`、行数为 0。
fn numstat(p: &Path, from: &str, to: &str, paths: &[String]) -> BTreeMap<String, LineStat> {
    // libgit2 固定用 myers：钉住 git 这边，不随开发机的全局 diff.algorithm 变（独立 review nit 3）
    let mut args = vec![
        "-c",
        "diff.algorithm=myers",
        "diff",
        "--numstat",
        "-z",
        "--no-renames",
        from,
        to,
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    let out = ok(p, &args);
    // -z：`<增>\t<删>\t<路径>` NUL
    String::from_utf8(out)
        .unwrap()
        .split('\0')
        .filter(|r| !r.is_empty())
        .map(|r| {
            let mut f = r.splitn(3, '\t');
            let (a, d, path) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
            let binary = a == "-";
            let n = |x: &str| if binary { 0 } else { x.parse().unwrap() };
            (
                path.to_string(),
                LineStat {
                    added: n(a),
                    removed: n(d),
                    binary,
                },
            )
        })
        .collect()
}

/// 第一个提交 → 第二个提交，覆盖增、删、改、CRLF、末尾换行、二进制、`-diff` 属性、中文路径，以及只有
/// 一边有 blob 时的二进制与空文件（`line_stats` 对这两种另走一条路）。
fn scene() -> (TempDir, String, String, Vec<String>) {
    let d = TempDir::new().unwrap();
    let p = d.path();
    ok(p, &["init", "-q", "-b", "work"]);
    ok(p, &["config", "user.email", "t@t"]);
    ok(p, &["config", "user.name", "T"]);
    // 两边比的都是对象库里的 blob，不经过过滤，autocrlf 本来就不参与；开着它只是照混合仓库的配置搭
    ok(p, &["config", "core.autocrlf", "true"]);
    ok(p, &["config", "core.quotepath", "false"]);
    let w = |rel: &str, bytes: &[u8]| {
        let f = p.join(rel);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, bytes).unwrap();
    };
    w(".gitattributes", b"*.dat -diff\n");
    w("Lua/mod.lua", lines(20, "\n").as_bytes());
    w("Lua/del.lua", lines(4, "\n").as_bytes());
    w("Lua/same.lua", b"same\n");
    w("Lua/crlf.cs", lines(5, "\r\n").as_bytes());
    w("Lua/noeol.txt", b"a\nb");
    w("Lua/bin.png", &[0, 1, 2, 3, 0, 5]);
    w("Lua/data.dat", b"text but -diff\n");
    w("Lua/中文.lua", "一\n二\n".as_bytes());
    // 下一个提交里删掉：只有旧的一边有 blob
    w("Lua/gone.dat", b"deleted, -diff\n");
    w("Lua/gone.png", &[0, 0, 7]);
    w("Lua/empty-old.txt", b"");
    // 不让 autocrlf 改写 CRLF 文件：先用 -c 关掉再 add，blob 里就是 CRLF（混合仓库的真实形状）
    ok(p, &["-c", "core.autocrlf=false", "add", "-A"]);
    ok(p, &["commit", "-qm", "one"]);
    let from = String::from_utf8(ok(p, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_string();

    // 改：去掉 7 行、加 2 行
    let mut m: Vec<String> = lines(20, "\n").lines().map(|l| format!("{l}\n")).collect();
    m.drain(3..10);
    m.insert(5, "new a\n".into());
    m.insert(6, "new b\n".into());
    w("Lua/mod.lua", m.concat().as_bytes());
    fs::remove_file(p.join("Lua/del.lua")).unwrap();
    w("Lua/add.lua", lines(3, "\n").as_bytes());
    w(
        "Lua/crlf.cs",
        lines(5, "\r\n").replace("line 3", "LINE 3").as_bytes(),
    );
    w("Lua/noeol.txt", b"a\nb\n");
    w("Lua/bin.png", &[0, 1, 2, 9, 0, 5]);
    w("Lua/data.dat", b"text but -diff, changed\n");
    w("Lua/中文.lua", "一\n三\n".as_bytes());
    for gone in ["Lua/gone.dat", "Lua/gone.png", "Lua/empty-old.txt"] {
        fs::remove_file(p.join(gone)).unwrap();
    }
    // 只有新的一边有 blob
    w("Lua/new.png", &[0, 1, 0, 2]);
    w("Lua/new.dat", b"added, -diff\n");
    w("Lua/empty-new.txt", b"");
    ok(p, &["-c", "core.autocrlf=false", "add", "-A"]);
    ok(p, &["commit", "-qm", "two"]);
    let to = String::from_utf8(ok(p, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_string();

    let paths = [
        "Lua/mod.lua",
        "Lua/del.lua",
        "Lua/add.lua",
        "Lua/same.lua",
        "Lua/crlf.cs",
        "Lua/noeol.txt",
        "Lua/bin.png",
        "Lua/data.dat",
        "Lua/中文.lua",
        "Lua/gone.dat",
        "Lua/gone.png",
        "Lua/empty-old.txt",
        "Lua/new.png",
        "Lua/new.dat",
        "Lua/empty-new.txt",
        "Lua/nowhere.lua",
    ]
    .map(String::from)
    .to_vec();
    (d, from, to, paths)
}

#[test]
fn line_stats_match_git_numstat() {
    let (d, from, to, paths) = scene();
    let p = d.path();
    let cli = numstat(p, &from, &to, &paths);
    assert_eq!(
        cli.get("Lua/mod.lua"),
        Some(&LineStat {
            added: 2,
            removed: 7,
            binary: false
        }),
        "前提：{cli:?}"
    );
    for bin in [
        "Lua/bin.png",
        "Lua/data.dat",
        "Lua/gone.dat",
        "Lua/gone.png",
        "Lua/new.png",
        "Lua/new.dat",
    ] {
        assert!(cli[bin].binary, "前提：{bin} 是二进制：{cli:?}");
    }
    let empty = LineStat {
        added: 0,
        removed: 0,
        binary: false,
    };
    assert_eq!(
        (cli.get("Lua/empty-old.txt"), cli.get("Lua/empty-new.txt")),
        (Some(&empty), Some(&empty)),
        "前提：numstat 对新增 / 删除的空文件写 0 0"
    );
    assert!(!cli.contains_key("Lua/same.lua") && !cli.contains_key("Lua/nowhere.lua"));

    assert_eq!(git_read::line_stats(s(p), &from, &to, &paths).unwrap(), cli);
    // 反过来比也一样：跟进时 from 是新基线、to 是重放后的分支
    assert_eq!(
        git_read::line_stats(s(p), &to, &from, &paths).unwrap(),
        numstat(p, &to, &from, &paths)
    );
}

#[test]
fn line_stats_only_answers_for_the_paths_asked() {
    let (d, from, to, _) = scene();
    let p = d.path();
    let got = git_read::line_stats(s(p), &from, &to, &["Lua/add.lua".to_string()]).unwrap();
    assert_eq!(
        got,
        BTreeMap::from([(
            "Lua/add.lua".to_string(),
            LineStat {
                added: 3,
                removed: 0,
                binary: false
            }
        )])
    );
    assert!(git_read::line_stats(s(p), &from, &to, &[])
        .unwrap()
        .is_empty());
}
