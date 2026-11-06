#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 节点状态解析与映射计算（计划 Task 2；分析 §6.1、§6.2；日志「映射取值」）。
//!
//! 映射取值：F = 基线树里各文件节点 revision 的最小值；最后修改 revision c(f) ≤ F 的文件
//! 隐含 F，其余记节点 revision n(f)；c(f) 缺失（added / replaced 没有 `<commit>`，E13）时取 n(f)。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_config::read_config;
use app_lib::engine::svn_nodes::{compute_map, read_node_status, NodeStatus};
use app_lib::engine::svn_wc::enumerate_wcs;
use app_lib::engine::svn_xml::parse_node_status;
use std::collections::{BTreeMap, BTreeSet};
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const ROOT: &str = "file:///fixture/repo";
const UUID: &str = "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01";

/// E13 / E14 的形状：属性逐行排列、反斜杠路径、changelist 成员在 `</target>` 之后的平级块里，
/// added 没有 `<commit>`，树冲突没有 revision。
const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target
   path="Code">
<entry
   path="Code">
<wc-status
   props="none"
   item="normal"
   revision="5">
<commit
   revision="3">
<author>alice</author>
<date>2026-10-31T00:54:43.614213Z</date>
</commit>
</wc-status>
</entry>
<entry
   path="Code\A.cs">
<wc-status
   item="normal"
   revision="5"
   props="none">
<commit
   revision="4">
<author>bob</author>
<date>2026-10-31T00:54:43.614213Z</date>
</commit>
</wc-status>
</entry>
<entry
   path="Code\N.cs">
<wc-status
   props="none"
   item="added"
   revision="-1">
</wc-status>
</entry>
<entry
   path="Code\T.cs">
<wc-status
   props="none"
   item="added"
   copied="true"
   tree-conflicted="true">
</wc-status>
</entry>
</target>
<changelist
   name="ignore-on-commit">
<entry
   path="Code\B.cs">
<wc-status
   item="normal"
   revision="5"
   props="none">
<commit
   revision="1">
</commit>
</wc-status>
</entry>
</changelist>
</status>
"#;

fn by_path(nodes: &[NodeStatus], p: &str) -> NodeStatus {
    nodes
        .iter()
        .find(|n| n.path == p)
        .unwrap_or_else(|| panic!("缺 `{p}`：{nodes:?}"))
        .clone()
}

#[test]
fn parse_node_status_reads_commit_rev_author_and_changelist() {
    let nodes = parse_node_status(XML).unwrap();
    let a = by_path(&nodes, "Code/A.cs");
    assert_eq!(
        (a.node_rev, a.commit_rev, a.author.as_deref()),
        (Some(5), Some(4), Some("bob"))
    );
    assert_eq!(a.changelist, None);
    assert_eq!(a.item, "normal");

    // changelist 成员不在 <target> 底下（E14），只遍历 target 会漏掉它
    let b = by_path(&nodes, "Code/B.cs");
    assert_eq!(b.changelist.as_deref(), Some("ignore-on-commit"));
    assert_eq!((b.node_rev, b.commit_rev), (Some(5), Some(1)));
    assert_eq!(b.author, None, "匿名提交没有 <author>");

    // added：revision=-1 不是服务器 revision，也没有 <commit>（E13）
    let n = by_path(&nodes, "Code/N.cs");
    assert_eq!((n.node_rev, n.commit_rev), (None, None));
    assert_eq!(n.item, "added");

    let t = by_path(&nodes, "Code/T.cs");
    assert!(t.tree_conflicted);
    assert_eq!(t.node_rev, None);
    // Review 1 第 4 条：`copied` 要读出来，判定靠它认出「复制过来的」节点
    assert!(t.copied);
    assert!(!a.copied && !n.copied);
    assert!(nodes.iter().all(|n| !n.wc_locked));
}

#[test]
fn parse_node_status_marks_wc_locked_dirs() {
    // E15：被锁的工作副本 status 照样 rc=0，只有目录 entry 带 wc-locked="true"
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target
   path="Code">
<entry
   path="Code">
<wc-status
   props="none"
   wc-locked="true"
   item="normal"
   revision="2">
<commit
   revision="2">
<author>liuhao</author>
</commit>
</wc-status>
</entry>
<entry
   path="Code\A.cs">
<wc-status
   props="none"
   item="modified"
   revision="2">
<commit
   revision="2">
<author>liuhao</author>
</commit>
</wc-status>
</entry>
</target>
</status>
"#;
    let nodes = parse_node_status(xml).unwrap();
    assert!(by_path(&nodes, "Code").wc_locked);
    assert!(!by_path(&nodes, "Code/A.cs").wc_locked);
}

#[test]
fn parse_node_status_refuses_output_without_externals_ignored() {
    // 保留 parse_node_revs 的那道守卫：external 内部的节点混进来会带另一个仓库的 revision 序列
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<status><target path="Code">
<entry path="Code\Vendor"><wc-status item="external" props="none"></wc-status></entry>
<entry path="Code\Vendor\V.cs"><wc-status item="normal" revision="9" props="none"><commit revision="9"/></wc-status></entry>
</target></status>"#;
    let err = parse_node_status(xml).unwrap_err();
    assert_eq!(err.code, "svn_xml_externals_not_ignored");
}

fn nodes_of(fx: &SvnFixture) -> Vec<NodeStatus> {
    let repo = fx.wc_str();
    let cfg = read_config(&repo).unwrap();
    let wcs = enumerate_wcs(&repo, &cfg.whitelist).unwrap();
    read_node_status(&repo, &cfg.whitelist, &wcs).unwrap()
}

fn files(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// wc1 与服务器都在 r3：`Code/A.cs`、`Code/B.cs`（r1）、`Code/Bag/b1.cs`（r2）、`Code/Bag/b2.cs`（r3）。
fn with_bag() -> SvnFixture {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Bag/b1.cs", "b1-v1\n"); // r2
    fx.colleague_commits("Code/Bag/b2.cs", "b2-v1\n"); // r3
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    fx
}

const BAG_FILES: [&str; 4] = ["Code/A.cs", "Code/B.cs", "Code/Bag/b1.cs", "Code/Bag/b2.cs"];

#[test]
fn compute_map_keeps_untouched_files_on_floor_after_partial_update() {
    // E12：局部 update 把 Bag 下所有文件的节点 revision 推到 r4，内容只有 b1 变了
    let fx = with_bag();
    fx.colleague_commits("Code/Bag/b1.cs", "b1-v2\n"); // r4
    svn_cmd(
        &fx.wc(),
        &["update", "--non-interactive", "--", "Code/Bag@"],
    );

    let nodes = nodes_of(&fx);
    // 前提：b2 的节点 revision 真的被推高了，而最后修改 revision 没变
    let b2 = by_path(&nodes, "Code/Bag/b2.cs");
    assert_eq!((b2.node_rev, b2.commit_rev), (Some(4), Some(3)), "{b2:?}");

    let map = compute_map(ROOT, UUID, &files(&BAG_FILES), &nodes, BTreeMap::new()).unwrap();
    assert_eq!(map.floor, 3, "floor 是节点 revision 的最小值");
    assert_eq!(
        map.revs,
        BTreeMap::from([("Code/Bag/b1.cs".to_string(), 4)]),
        "只有内容真的变了的 b1 进映射"
    );
    assert!(map.gone.is_empty());
}

#[test]
fn compute_map_after_root_update_is_single() {
    // 周一根目录 update：所有文件节点 revision 一致 ⇒ 回到单值、旧格式
    let fx = with_bag();
    fx.colleague_commits("Code/Bag/b1.cs", "b1-v2\n"); // r4
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let map = compute_map(
        ROOT,
        UUID,
        &files(&BAG_FILES),
        &nodes_of(&fx),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(map.floor, 4);
    assert!(map.is_single(), "{map:?}");
}

#[test]
fn compute_map_records_own_commit_at_node_rev() {
    // TortoiseSVN 里只提交了 A：A 到 r2，B 原地停在 r1
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-mine\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &[
            "commit",
            "-m",
            "mine",
            "--non-interactive",
            "--",
            "Code/A.cs@",
        ],
    );

    let map = compute_map(
        ROOT,
        UUID,
        &files(&["Code/A.cs", "Code/B.cs"]),
        &nodes_of(&fx),
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(map.floor, 1);
    assert_eq!(map.revs, BTreeMap::from([("Code/A.cs".to_string(), 2)]));
}

fn node(path: &str, n: Option<u64>, c: Option<u64>) -> NodeStatus {
    NodeStatus {
        path: path.to_string(),
        item: "normal".to_string(),
        node_rev: n,
        commit_rev: c,
        author: None,
        wc_locked: false,
        tree_conflicted: false,
        copied: false,
        changelist: None,
    }
}

#[test]
fn compute_map_floor_includes_directory_nodes() {
    // Review 1 第 2 条：只提交 A、B、没 update——文件到了 r3，目录 Code 还停在 r1。
    // 目录的 revision 担保的是「n(D) 时的子项集合」：同事 r2 加的 X 不在工作副本里，
    // F 取 3 的话 `Code@3` 里有 X，基线树里没有，就成了撒谎的单值。F 必须取目录的 1
    let nodes = vec![
        node("Code", Some(1), Some(1)),
        node("Code/A.cs", Some(3), Some(3)),
        node("Code/B.cs", Some(3), Some(3)),
    ];
    let map = compute_map(
        ROOT,
        UUID,
        &files(&["Code/A.cs", "Code/B.cs"]),
        &nodes,
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(map.floor, 1);
    assert_eq!(
        map.revs,
        BTreeMap::from([("Code/A.cs".to_string(), 3), ("Code/B.cs".to_string(), 3)])
    );

    // 不是目录、也不在基线树里的节点（白名单内的美术文件）不拖低 floor：它的子项集合无从谈起，
    // 基线也不收它的内容
    let nodes = vec![
        node("Code", Some(3), Some(3)),
        node("Code/A.cs", Some(3), Some(3)),
        node("Code/x.png", Some(1), Some(1)),
    ];
    let map = compute_map(ROOT, UUID, &files(&["Code/A.cs"]), &nodes, BTreeMap::new()).unwrap();
    assert_eq!(map.floor, 3);
    assert!(map.is_single(), "{map:?}");
}

#[test]
fn compute_map_takes_node_rev_when_commit_rev_is_missing() {
    // replaced 之类没有 <commit>（E13）：读不到 c(f) 就按「内容在 n(f) 变过」记，保守方向
    let mut a = node("Code/A.cs", Some(7), None);
    a.item = "replaced".to_string();
    let nodes = vec![a, node("Code/B.cs", Some(5), Some(2))];
    let map = compute_map(
        ROOT,
        UUID,
        &files(&["Code/A.cs", "Code/B.cs"]),
        &nodes,
        BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(map.floor, 5);
    assert_eq!(map.revs, BTreeMap::from([("Code/A.cs".to_string(), 7)]));
}

#[test]
fn compute_map_refuses_a_file_without_a_node_revision() {
    let nodes = vec![node("Code/A.cs", Some(3), Some(3))];
    let err = compute_map(
        ROOT,
        UUID,
        &files(&["Code/A.cs", "Code/B.cs"]),
        &nodes,
        BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(err.code, "svn_map_unknown_node");
    assert!(err.message.contains("Code/B.cs"), "{}", err.message);

    let added = vec![
        node("Code/A.cs", Some(3), Some(3)),
        node("Code/B.cs", None, None),
    ];
    let err = compute_map(
        ROOT,
        UUID,
        &files(&["Code/A.cs", "Code/B.cs"]),
        &added,
        BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(err.code, "svn_map_unknown_node");
}

#[test]
fn compute_map_drops_tombstones_the_floor_has_passed_or_that_reappeared() {
    // 墓碑带到 floor ≥ 删除时的 HEAD 为止（分析 §6.1）；重新进了基线树的路径不能再是墓碑
    let nodes = vec![
        node("Code/A.cs", Some(5), Some(5)),
        node("Code/Back.cs", Some(5), Some(5)),
    ];
    let gone = BTreeMap::from([
        ("Code/Old.cs".to_string(), 4),
        ("Code/Late.cs".to_string(), 6),
        ("Code/Back.cs".to_string(), 6),
    ]);
    let map = compute_map(
        ROOT,
        UUID,
        &files(&["Code/A.cs", "Code/Back.cs"]),
        &nodes,
        gone,
    )
    .unwrap();
    assert_eq!(map.gone, BTreeMap::from([("Code/Late.cs".to_string(), 6)]));
}

#[test]
fn compute_map_too_large_lists_the_lowest_files() {
    // 超限时要点名拖低 floor 的那几个文件（分析 §6.1）
    let total = app_lib::engine::svn_basemap::MAX_MAP_ENTRIES + 2;
    let mut nodes = vec![node("Code/aaa-low.cs", Some(1), Some(1))];
    let mut list = vec!["Code/aaa-low.cs".to_string()];
    for i in 0..total {
        let p = format!("Code/f{i:05}.cs");
        nodes.push(node(&p, Some(10), Some(10)));
        list.push(p);
    }
    let set: BTreeSet<String> = list.into_iter().collect();
    let err = compute_map(ROOT, UUID, &set, &nodes, BTreeMap::new()).unwrap_err();
    assert_eq!(err.code, "svn_map_too_large");
    assert!(err.message.contains("Code/aaa-low.cs"), "{}", err.message);
}
