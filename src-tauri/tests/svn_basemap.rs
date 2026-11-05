#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 基线记账格式 `BaseMap`（分析 §6.1，计划 Task 1）。
//!
//! 两条硬要求：
//!
//! 1. 单值基线的 message **逐字节**等于现行格式——旧版 jkk、`git log --grep=^git-svn-id`
//!    与图上的基线标注都靠它，哪怕多一个空格都会让它们读不出基线
//! 2. 混合基线写进 commit 再读回来，映射必须完全相等，路径里的空格、`%`、中文一个字节都不能变

use app_lib::engine::git_cli::{run_git, run_git_bytes};
use app_lib::engine::svn_baseline::{create_baseline_commit_with, format_trailer, Trailer};
use app_lib::engine::svn_basemap::{
    decode_path, encode_path, format_message, parse_message, read_base_map, BaseMap,
};
use std::collections::BTreeMap;

const ROOT: &str = "https://svn.example.com/game";
const UUID: &str = "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01";

fn mixed() -> BaseMap {
    let mut revs = BTreeMap::new();
    revs.insert("道具/装备 名称.lua".to_string(), 1240);
    revs.insert("a%b.lua".to_string(), 1241);
    // 末尾是全角空格：解析时若 `trim_end` 就会少一个字符
    revs.insert("文档/尾部全角空格\u{3000}".to_string(), 1241);
    let mut gone = BTreeMap::new();
    gone.insert("UI/Old/旧 面板.lua".to_string(), 1242);
    BaseMap {
        repos_root: ROOT.into(),
        uuid: UUID.into(),
        floor: 1234,
        revs,
        gone,
    }
}

/// 只有一个空 tree 的临时仓库：`commit-tree` 只需要 tree 对象存在。
fn repo_with_empty_tree() -> (tempfile::TempDir, String) {
    let d = tempfile::tempdir().unwrap();
    let r = d.path().to_str().unwrap();
    run_git(r, &["init", "-q"]).unwrap();
    run_git(r, &["config", "user.email", "t@t"]).unwrap();
    run_git(r, &["config", "user.name", "T"]).unwrap();
    let tree = run_git(r, &["write-tree"]).unwrap().trim().to_string();
    (d, tree)
}

/// commit 对象里 message 的原始字节：头部到第一个空行为止，后面原样就是 message。
fn raw_message(repo: &str, oid: &str) -> Vec<u8> {
    let raw = run_git_bytes(repo, &["cat-file", "commit", oid]).unwrap();
    let at = raw
        .windows(2)
        .position(|w| w == b"\n\n")
        .expect("commit 对象没有头部与 message 的分隔");
    raw[at + 2..].to_vec()
}

#[test]
fn single_map_formats_exactly_like_legacy_trailer() {
    let t = Trailer {
        url: ROOT.into(),
        rev: 12346,
        uuid: UUID.into(),
    };
    let map = BaseMap::single(ROOT, UUID, 12346);
    assert!(map.is_single());

    // 现行格式（`create_baseline_commit_with` 改造前的拼法），手抄在这里而不是调用新代码
    let legacy = format!("svn: r12346\n\n{}\n", format_trailer(&t));
    assert_eq!(format_message(&map, &[], None), legacy);
    let legacy_delivered = format!(
        "svn: r12346\n\nDelivered-Rev: 12340\n{}\n",
        format_trailer(&t)
    );
    assert_eq!(format_message(&map, &[], Some(12340)), legacy_delivered);

    // 改走 `-F` 之后，写进 commit 对象的字节也必须与原来 `-m` 写进去的一样
    let (d, tree) = repo_with_empty_tree();
    let r = d.path().to_str().unwrap();
    let oid = create_baseline_commit_with(r, &tree, None, &map, &[], Some(12340)).unwrap();
    assert_eq!(raw_message(r, &oid), legacy_delivered.as_bytes());
    let via_m = run_git(r, &["commit-tree", &tree, "-m", &legacy_delivered])
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(raw_message(r, &oid), raw_message(r, &via_m));
}

#[test]
fn mixed_map_round_trips_with_spaces_percent_and_cjk() {
    let map = mixed();
    let msg = format_message(&map, &["跟进 r1240–r1242：张三".to_string()], None);
    assert_eq!(parse_message(&msg), Some(map.clone()));

    // 编码后的路径里不能再有会打断「一行一条、空格分字段」的字符
    for p in ["道具/装备 名称.lua", "a%b.lua", "x\ty\r\nz", "\u{1}"] {
        let e = encode_path(p);
        assert!(
            !e.chars().any(|c| c == ' ' || (c as u32) < 0x20),
            "编码后仍有空白或控制字符：{e:?}"
        );
        assert_eq!(decode_path(&e).as_deref(), Some(p));
    }
    assert_eq!(encode_path("a%b c"), "a%25b%20c");
    assert_eq!(decode_path("a%2"), None, "截断的 %XX 不能被猜着接受");
    assert_eq!(decode_path("a%zz"), None);

    // 映射的读法
    assert_eq!(map.rev_of("道具/装备 名称.lua"), 1240);
    assert_eq!(map.rev_of("未列出的文件.lua"), 1234);
    assert_eq!(map.entry_count(), 4);
    assert!(!map.is_single());
}

#[test]
fn mixed_message_has_no_git_svn_id_line() {
    let msg = format_message(&mixed(), &[], Some(1243));
    assert!(
        !msg.lines().any(|l| l.contains("git-svn-id")),
        "混合基线写 git-svn-id 等于对旧读者撒谎：{msg}"
    );
    assert!(
        app_lib::engine::svn_baseline::parse_trailer(&msg).is_none(),
        "旧解析器必须读不出它，而不是读错"
    );
    let first = msg.lines().next().unwrap();
    assert_eq!(
        first,
        "svn: r1234（另有 3 个文件在更新的 revision，1 个文件已删除）"
    );
    // trailer 只在最后一段，Delivered-Rev 在最末
    let last_para = msg.trim_end().rsplit("\n\n").next().unwrap();
    assert!(last_para.starts_with("Svn-Base-Repo: "));
    assert!(last_para.ends_with("Delivered-Rev: 1243"));
}

#[test]
fn parse_message_reads_legacy_baseline() {
    // 现网已有的基线，`\r\n` 也照常读
    let legacy = format!("svn: r7\r\n\r\ngit-svn-id: {ROOT}@7 {UUID}\r\n");
    assert_eq!(parse_message(&legacy), Some(BaseMap::single(ROOT, UUID, 7)));
    assert_eq!(parse_message("随手写的 commit\n\n没有 trailer\n"), None);

    // 最后一段有 Svn-Base-* 但写坏了：按新格式判失败，不退回去猜旧格式
    let broken = format!(
        "svn: r7\n\ngit-svn-id: {ROOT}@7 {UUID}\n\nSvn-Base-Repo: {ROOT} {UUID}\nSvn-Base-Rev: 9 a.lua\n"
    );
    assert_eq!(
        parse_message(&broken),
        None,
        "缺 Floor 的新 trailer 不能被接受"
    );
}

#[test]
fn message_with_1200_entries_round_trips_through_commit_tree() {
    let mut revs = BTreeMap::new();
    for i in 0..1100 {
        revs.insert(format!("道具/装备 名称 {i}.lua"), 1300 + (i % 7));
    }
    let mut gone = BTreeMap::new();
    for i in 0..100 {
        gone.insert(format!("旧%目录/文件 {i}.lua"), 1310);
    }
    let map = BaseMap {
        repos_root: ROOT.into(),
        uuid: UUID.into(),
        floor: 1299,
        revs,
        gone,
    };
    assert_eq!(map.entry_count(), 1200);

    let (d, tree) = repo_with_empty_tree();
    let r = d.path().to_str().unwrap();
    let summary = vec!["跟进 r1300–r1310：张三、李四".to_string()];
    let msg = format_message(&map, &summary, None);
    // 超过 Windows 命令行约 32K 字符的上限：走 `-m` 的话这里会直接失败
    assert!(msg.len() > 40_000, "用例没有越过命令行上限：{}", msg.len());

    let oid = create_baseline_commit_with(r, &tree, None, &map, &summary, None).unwrap();
    assert_eq!(raw_message(r, &oid), msg.as_bytes());
    assert_eq!(read_base_map(r, &oid).unwrap(), map);

    // 临时 message 文件用完即删，不在 .git 里留垃圾
    let git_dir = d.path().join(".git");
    let leftovers: Vec<_> = std::fs::read_dir(&git_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("mygitclient-"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件：{leftovers:?}");

    // 不是基线的 commit：沿用 svn_baseline_no_trailer
    let plain = run_git(r, &["commit-tree", &tree, "-m", "plain"])
        .unwrap()
        .trim()
        .to_string();
    assert_eq!(
        read_base_map(r, &plain).unwrap_err().code,
        "svn_baseline_no_trailer"
    );
}
