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
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_sync;
use svn_fixtures::{svn_repo, SvnFixture};

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
