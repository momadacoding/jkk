#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! `load_graph` 与前端混合图之间的**数据契约**。
//!
//! # 这个文件为一次「四道检查全绿、功能整个不生效」的事故而建
//!
//! 2026-09-16 独立 review 发现：`graph.rs` 的 `message` 取自 `commit.summary()`，
//! 而基线 commit 的 message 是
//!
//! ```text
//! svn: r5
//!
//! git-svn-id: <url>@5 <uuid>
//! ```
//!
//! `summary()` 只返回**第一段**（还会把空白折叠），所以 `git-svn-id:` 那行
//! **永远到不了前端**。前端却在 `svnGraph.ts` 里从 `message` 逐行找这条 trailer：
//!
//! | 后果 | 表现 |
//! |---|---|
//! | `baselineMap` 恒为空 | SVN 历史与 git 历史**不接合**，两段各自悬着 |
//! | `svn/base` 的 ref 不重映射 | 基线分支标签照旧渲染在一个本该消失的 commit 上 |
//! | `isLocalOnly` 恒为 false | 「仅本地，不交付」角标**永远不出现** |
//!
//! 而 Rust 侧 480 条、前端 334 条测试全绿：前端测试直接手写一个带 trailer 的
//! 完整 message 喂进 `spliceHistory`，**跳过了真正断掉的那一环**。
//! 这正是 `AGENTS.md` §13 第 9 条说的「虚假信心」。
//!
//! 所以修复的方向不是「把 message 换成全文再让前端解析」，而是**把 trailer 的
//! 解析留在已有 `parse_trailer` 的 Rust 侧**，由 `load_graph` 直接给出结论。
//! 这个文件锁住那份结论的形状。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::graph;
use app_lib::engine::svn_baseline::{create_baseline_commit_with, move_baseline_ref};
use app_lib::engine::svn_basemap::{read_base_map, BaseMap};
use app_lib::engine::svn_follow::{follow, FollowOutcome};
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_sync;
use svn_fixtures::{
    hybrid_with_colleague, svn_cmd, svn_repo, svn_repo_committed_without_update, SvnFixture,
};

fn init(fx: &SvnFixture) {
    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
}

/// 在工作分支上造一个普通 commit，`body` 为 `None` 时只有标题。
///
/// ⚠️ 文件必须落在 `Code/` 里：混合仓库的 `.git/info/exclude` 把白名单之外的
/// 一切都排除掉了，根目录下的文件 `git add` 会直接被拒。
fn commit(fx: &SvnFixture, name: &str, subject: &str, body: Option<&str>) -> String {
    let file = format!("Code/{name}");
    std::fs::write(fx.wc().join(&file), "x\n").unwrap();
    run_git(&fx.wc_str(), &["add", "--", &file]).unwrap();
    let msg = match body {
        Some(b) => format!("{subject}\n\n{b}\n"),
        None => format!("{subject}\n"),
    };
    run_git(&fx.wc_str(), &["commit", "-m", &msg]).unwrap();
    run_git(&fx.wc_str(), &["rev-parse", "HEAD"])
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn the_baseline_commit_carries_its_revision_in_the_graph() {
    // **本文件的核心断言。** 前端靠这个字段把两段历史接起来；它为 None
    // 就等于整个混合图功能不存在，而界面上只会表现为「图看着有点怪」。
    let fx = svn_repo();
    init(&fx);
    let rev = svn_sync::check_sync(&fx.wc_str()).unwrap().baseline_rev;

    let head = run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"])
        .unwrap()
        .trim()
        .to_string();
    let data = graph::load(&fx.wc_str()).unwrap();
    let node = data
        .commits
        .iter()
        .find(|c| c.id == head)
        .expect("基线 commit 必须在图里");

    assert_eq!(
        node.baseline_rev,
        Some(rev),
        "基线 commit 必须带出它记账的 revision，实际 {:?}",
        node.baseline_rev
    );
}

#[test]
fn the_trailer_is_not_reachable_through_message_alone() {
    // 把**事故本身**钉成回归测试：`message` 是 `summary()`，只有第一段。
    //
    // 将来有人想「简化」成前端从 message 自己解析时，这条会立刻变红并指出
    // 为什么那条路走不通。没有它，同一个洞可以被原样挖回来而不触发任何失败。
    let fx = svn_repo();
    init(&fx);
    let head = run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"])
        .unwrap()
        .trim()
        .to_string();

    let data = graph::load(&fx.wc_str()).unwrap();
    let node = data.commits.iter().find(|c| c.id == head).unwrap();

    assert!(
        !node.message.contains("git-svn-id:"),
        "message 是 summary()，不含正文——若这条失败，说明 message 的语义变了，\
         baseline_rev 这条通路要重新评估。实际 `{}`",
        node.message
    );
}

#[test]
fn a_local_only_commit_is_flagged_in_the_graph() {
    let fx = svn_repo();
    init(&fx);
    let id = commit(
        &fx,
        "patch.cs",
        "只在本地打个补丁",
        Some("Local-Only: true"),
    );

    let data = graph::load(&fx.wc_str()).unwrap();
    let node = data.commits.iter().find(|c| c.id == id).unwrap();
    assert!(node.local_only, "「仅本地，不交付」角标的唯一依据");
    assert_eq!(node.baseline_rev, None, "普通提交不是基线");
}

#[test]
fn an_ordinary_commit_is_neither_baseline_nor_local_only() {
    let fx = svn_repo();
    init(&fx);
    let id = commit(&fx, "normal.cs", "普通提交", None);

    let data = graph::load(&fx.wc_str()).unwrap();
    let node = data.commits.iter().find(|c| c.id == id).unwrap();
    assert!(!node.local_only);
    assert_eq!(node.baseline_rev, None);
}

#[test]
fn a_commit_that_merely_mentions_the_trailer_is_not_a_baseline() {
    // 正文里顺口提到这个格式的 commit 不能被当成基线——基线在前端接合时是要
    // **从图上删掉**的，误判会让一个普通提交连同它的分支线凭空消失。
    //
    // 判据是逐行完整匹配（`parse_trailer`），这里确认那条规则真的作用在图上。
    let fx = svn_repo();
    init(&fx);
    let id = commit(
        &fx,
        "doc.cs",
        "说明一下 trailer 的格式",
        Some("我们用的是 git-svn-id: <url>@<rev> <uuid> 这种写法。"),
    );

    let data = graph::load(&fx.wc_str()).unwrap();
    let node = data.commits.iter().find(|c| c.id == id).unwrap();
    assert_eq!(node.baseline_rev, None, "正文提到 ≠ 是基线");
}

#[test]
fn the_graph_serializes_to_the_camel_case_names_the_frontend_reads() {
    // Tauri 的 `snake_case` → `camelCase` 只作用于**命令参数**，不作用于返回值的
    // 结构体字段——那层要靠 serde 的 `rename_all`，而它没有编译期检查
    // （`AGENTS.md` §7.3）。写错只会让前端静默读到 undefined。
    let fx = svn_repo();
    init(&fx);
    let head = run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"])
        .unwrap()
        .trim()
        .to_string();
    let data = graph::load(&fx.wc_str()).unwrap();
    let node = data.commits.iter().find(|c| c.id == head).unwrap();

    let json = serde_json::to_value(node).unwrap();
    assert!(
        json.get("baselineRev").is_some(),
        "前端读的是 baselineRev，实际序列化成 {json}"
    );
    assert!(
        json.get("baseline_rev").is_none(),
        "不能同时留着 snake_case 的写法：{json}"
    );
}

// —— 混合基线（计划 Task 12，分析 §6.9）——

fn oid(fx: &SvnFixture, rev: &str) -> String {
    run_git(&fx.wc_str(), &["rev-parse", rev])
        .unwrap()
        .trim()
        .to_string()
}

fn node<'a>(data: &'a graph::GraphData, id: &str) -> &'a graph::CommitNode {
    data.commits
        .iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("{id} 必须在图里"))
}

/// 真实形状：TortoiseSVN 只提交 A、不 update，点「跟进」。基线 → r1 + A 在 r2。
fn followed_mixed() -> SvnFixture {
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "a2\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &[
            "commit",
            "-m",
            "tortoise",
            "--non-interactive",
            "--",
            "Code/A.cs",
        ],
    );
    let out = follow(&fx.wc_str(), "op1").unwrap();
    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    fx
}

/// 在现有基线上手工录一条树不变、记账不同的基线。真实流程造起来太绕的记账才用它（墓碑要服务器删文件）。
fn publish(fx: &SvnFixture, edit: impl FnOnce(&mut BaseMap)) -> String {
    let root = fx.wc_str();
    let old = oid(fx, "refs/heads/svn/base");
    let mut map = read_base_map(&root, &old).unwrap();
    edit(&mut map);
    let tree = oid(fx, &format!("{old}^{{tree}}"));
    let new = create_baseline_commit_with(&root, &tree, Some(&old), &map, &[], None).unwrap();
    move_baseline_ref(&root, &new, Some(&old)).unwrap();
    new
}

#[test]
fn a_followed_mixed_baseline_reports_floor_and_newer_but_no_single_revision() {
    // 混合基线没有「那一个 revision」。baseline_rev 有值的话，前端会把它从图上删掉、
    // 挂到 svn:<floor> 上，顶掉 floor 节点原本的基线（分析 §6.9）
    let fx = followed_mixed();
    let data = graph::load(&fx.wc_str()).unwrap();
    let base = node(&data, &oid(&fx, "refs/heads/svn/base"));

    assert_eq!(base.baseline_rev, None);
    assert_eq!(base.baseline_floor, Some(1));
    assert_eq!(base.baseline_newer, Some(1));
    assert_eq!(base.baseline_gone, None, "没有墓碑时不带这一项");
}

#[test]
fn the_mixed_baseline_of_a_mixed_working_copy_init_is_a_root() {
    // 初始化录的基线没有父：spliceHistory 只能靠 floor 把它接到 SVN 主线上
    let fx = svn_repo_committed_without_update();
    init(&fx);
    let data = graph::load(&fx.wc_str()).unwrap();
    let base = node(&data, &oid(&fx, "refs/heads/svn/base"));

    assert!(base.parents.is_empty());
    assert_eq!(base.baseline_rev, None);
    assert_eq!(base.baseline_floor, Some(3));
    assert_eq!(base.baseline_newer, Some(1));
}

#[test]
fn a_mixed_baseline_with_only_tombstones_is_still_mixed() {
    // 没有 revision 不同的文件、只有墓碑：is_single 为假，也不能当成 r<floor>
    let fx = hybrid_with_colleague();
    let id = publish(&fx, |m| {
        m.gone.insert("Code/删掉 的.cs".to_string(), 7);
    });
    let data = graph::load(&fx.wc_str()).unwrap();
    let base = node(&data, &id);

    assert_eq!(base.baseline_rev, None);
    assert_eq!(base.baseline_floor, Some(1));
    assert_eq!(base.baseline_newer, Some(0));
    assert_eq!(base.baseline_gone, Some(1));
}

#[test]
fn a_single_valued_baseline_and_an_ordinary_commit_have_no_mixed_fields() {
    let fx = svn_repo();
    init(&fx);
    let id = commit(&fx, "normal.cs", "普通提交", None);
    let data = graph::load(&fx.wc_str()).unwrap();

    let base = oid(&fx, "refs/heads/svn/base");
    for n in [node(&data, &base), node(&data, &id)] {
        assert_eq!(n.baseline_floor, None);
        assert_eq!(n.baseline_newer, None);
        assert_eq!(n.baseline_gone, None);
        let json = serde_json::to_value(n).unwrap();
        for k in ["baselineFloor", "baselineNewer", "baselineGone"] {
            assert!(json.get(k).is_none(), "不带 {k}：{json}");
        }
    }
}

#[test]
fn the_mixed_fields_serialize_to_the_camel_case_names_the_frontend_reads() {
    let fx = followed_mixed();
    let id = publish(&fx, |m| {
        m.gone.insert("Code/x.cs".to_string(), 7);
    });
    let data = graph::load(&fx.wc_str()).unwrap();

    let json = serde_json::to_value(node(&data, &id)).unwrap();
    assert_eq!(json["baselineFloor"], 1, "{json}");
    assert_eq!(json["baselineNewer"], 1, "{json}");
    assert_eq!(json["baselineGone"], 1, "{json}");
    assert!(json.get("baselineRev").is_none(), "{json}");
    assert!(json.get("baseline_floor").is_none(), "{json}");
}

#[test]
fn a_commit_that_merely_mentions_svn_base_keys_is_not_a_baseline() {
    // 正文里讲格式的提交（包括本仓库自己的）：记账只认最后一段
    let fx = svn_repo();
    init(&fx);
    let id = commit(
        &fx,
        "doc.cs",
        "说明新格式",
        Some("Svn-Base-Floor: 3\nSvn-Base-Rev: 4 Code/A.cs\n\n这两行只是例子。"),
    );
    let data = graph::load(&fx.wc_str()).unwrap();
    let n = node(&data, &id);

    assert_eq!(n.baseline_rev, None);
    assert_eq!(n.baseline_floor, None);
}

#[test]
fn a_damaged_new_trailer_is_not_read_as_the_git_svn_id_next_to_it() {
    // parse_message 的规则：最后一段有 Svn-Base- 行就按新格式严格解析，失败不退回旧格式。
    // 图上照旧用 parse_trailer 的话，这条会被当成单值基线从图上删掉
    let fx = svn_repo();
    init(&fx);
    let id = commit(
        &fx,
        "broken.cs",
        "被改坏的记账",
        Some("Svn-Base-Floor: x\ngit-svn-id: file:///r@5 00000000-0000-0000-0000-000000000000"),
    );
    let data = graph::load(&fx.wc_str()).unwrap();
    let n = node(&data, &id);

    assert_eq!(n.baseline_rev, None);
    assert_eq!(n.baseline_floor, None);
}

#[test]
fn follow_pin_refs_are_not_drawn() {
    // W28：跟进把旧索引钉在 refs/mygitclient/follow/<opid>，那个 commit 的树里只有一个 index blob。
    // 进了图或进了 refs，用户就会看到一条与历史无关的提交，或一个写着全名的标签
    let fx = followed_mixed();
    let pin = oid(&fx, "refs/mygitclient/follow/op1");
    let data = graph::load(&fx.wc_str()).unwrap();

    assert!(
        data.commits.iter().all(|c| c.id != pin),
        "钉住索引的 commit 不能进图"
    );
    assert!(
        data.refs
            .iter()
            .all(|r| !r.name.starts_with("refs/mygitclient/")),
        "实际 {:?}",
        data.refs.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
}

#[test]
fn a_deliver_ref_on_a_commit_of_the_graph_gets_no_pill() {
    // 交付途中 deliver ref 可能就指着分支上的提交（快路径），它在图上有行：refs 里带着它，
    // CommitRow 就画出一个 `refs/mygitclient/deliver/<opid>` 的标签，已交付判据也会把它当 tip
    let fx = svn_repo();
    init(&fx);
    let id = commit(&fx, "work.cs", "要交付的提交", None);
    run_git(
        &fx.wc_str(),
        &["update-ref", "refs/mygitclient/deliver/op-1", &id],
    )
    .unwrap();
    let data = graph::load(&fx.wc_str()).unwrap();

    assert!(
        data.commits.iter().any(|c| c.id == id),
        "提交本身照常在图上"
    );
    assert!(data
        .refs
        .iter()
        .all(|r| !r.name.starts_with("refs/mygitclient/")));
    assert!(
        data.refs.iter().any(|r| r.name == "refs/heads/work"),
        "别的 ref 不受影响"
    );
}
