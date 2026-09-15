//! `svn log` 拉取编排的测试。
//!
//! 这里的每一条都对应 2026-09-15 在临时 fixture 上实测出来的一个 svn 行为，
//! 而不是对 svn 应该怎样的猜测。实测编号见计划文件的 A / B / C 三组表。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_log;
use svn_fixtures::{
    file_url, svn_repo, svn_repo_baseline_rev_outside_whitelist,
    svn_repo_with_same_repo_nested_external, NestedExternal, SvnFixture,
};

/// 仓库根 URL（= Repository Root）。
fn repos_root(fx: &SvnFixture) -> String {
    file_url(&fx.repo())
}

fn init(fx: &SvnFixture) {
    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
}

#[test]
fn head_revision_reads_the_servers_latest() {
    let fx = svn_repo();
    let head = svn_log::head_revision(&fx.wc_str(), &repos_root(&fx)).unwrap();
    assert_eq!(head, 1, "fixture 只提交过一次");
}

#[test]
fn fetch_range_returns_entries_newest_first_for_a_descending_range() {
    // 实测 M3/M4：区间方向决定输出顺序。解析层不重排，所以这里验的是编排层
    // 原样把 svn 的顺序交出来。
    let fx = svn_repo_baseline_rev_outside_whitelist();
    let got = svn_log::fetch_range(&fx.wc_str(), &repos_root(&fx), 4, 1, 100).unwrap();
    let revs: Vec<u64> = got.iter().map(|e| e.revision).collect();
    assert_eq!(revs, vec![4, 3, 2, 1]);
}

#[test]
fn fetch_range_clamps_to_head_instead_of_failing() {
    // 实测 P4/P5：-r 的**任一端**越界，整条命令 exit 1 (E160006)。
    //
    // 增量拉取的自然写法就是「上次覆盖到 N，这次拉 N+1..HEAD」——没有新提交时
    // N+1 > HEAD，于是**每一次「没有新东西」都会变成一次错误**。
    // 把它判成「拉到 0 条」才是它真实的含义。
    let fx = svn_repo();
    let head = svn_log::head_revision(&fx.wc_str(), &repos_root(&fx)).unwrap();
    let got = svn_log::fetch_range(&fx.wc_str(), &repos_root(&fx), head + 5, head + 9, 100)
        .expect("整段都在 HEAD 之上不该报错");
    assert!(got.is_empty(), "应当返回空，实际 {got:?}");
}

#[test]
fn fetch_range_clamps_only_the_out_of_range_end() {
    // 半越界：from 在范围内、to 超出 HEAD。这是「拉到最新」最常见的写法，
    // 不能因为 to 越界就整段丢掉。
    let fx = svn_repo_baseline_rev_outside_whitelist();
    let got = svn_log::fetch_range(&fx.wc_str(), &repos_root(&fx), 3, 999, 100).unwrap();
    let revs: Vec<u64> = got.iter().map(|e| e.revision).collect();
    assert_eq!(revs, vec![3, 4], "3:999 被钳成 3:4，升序");
}

#[test]
fn fetch_range_never_passes_limit_zero_to_svn() {
    // 实测 P6：`--limit 0` → exit 1 (E200004)。而 limit 是调用方算出来的剩余
    // 配额，算到 0 是正常的（比如首屏 200 条已经拉满）。
    let fx = svn_repo();
    let got = svn_log::fetch_range(&fx.wc_str(), &repos_root(&fx), 1, 1, 0)
        .expect("limit=0 应当被判成「不用拉」，而不是送给 svn");
    assert!(got.is_empty());
}

#[test]
fn a_baseline_revision_that_never_touched_the_whitelist_is_invisible_there() {
    // 这条**不测我们的代码**，测的是 svn 本身的行为（实测 Q1）。
    // 它是 fetch_anchor 存在的全部理由，钉住它才能防止将来有人
    // 「优化」成直接打给白名单 URL。
    let fx = svn_repo_baseline_rev_outside_whitelist();
    let whitelist_url = format!("{}/Code", repos_root(&fx));
    let none = svn_log::fetch_range(&fx.wc_str(), &whitelist_url, 4, 4, 10).unwrap();
    assert!(
        none.is_empty(),
        "白名单 URL 上不该有 r4 —— 这正是孤儿 baseline 的成因，实际 {none:?}"
    );
}

#[test]
fn fetch_anchor_gets_it_from_the_repository_root() {
    // 实测 Q2：同一个 revision 打给 Repository Root 就拿得到。
    let fx = svn_repo_baseline_rev_outside_whitelist();
    let got = svn_log::fetch_anchor(&fx.wc_str(), &repos_root(&fx), 4).unwrap();
    let entry = got.expect("锚点必须打给 Repository Root 才拿得到");
    assert_eq!(entry.revision, 4);
    assert_eq!(entry.message, "r4 other only");
}

#[test]
fn fetch_anchor_returns_none_for_a_revision_above_head() {
    // 缓存里记着的 baseline 可能指向一个本仓库没有的 revision（换了远端、
    // 或者仓库被重建过）。这时要返回 None 让调用方跳过，不是整次拉取失败。
    let fx = svn_repo();
    let got = svn_log::fetch_anchor(&fx.wc_str(), &repos_root(&fx), 999).unwrap();
    assert!(got.is_none());
}

#[test]
fn log_targets_returns_one_url_per_working_copy() {
    // 实测 Q4：`svn log <url1> <url2>` 被拒（E205000），所以是每个 WC 一次调用、
    // 客户端合并。这条钉住「嵌套 external 各占一个 target」——漏掉它就会漏掉
    // 一整块工作副本的历史。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    init(&fx);
    let targets = svn_log::log_targets(&fx.wc_str()).unwrap();
    assert!(
        targets.len() >= 2,
        "嵌套 external 是独立工作副本，必须各占一个 target，实际 {targets:?}"
    );
}

#[test]
fn log_targets_never_appends_the_peg_safe_suffix() {
    // 实测 R5：`svn log <URL> "Code@"` → E160013: File not found: path '/Code@'。
    //
    // 这与 `svn info` / `svn status` 的要求**正好相反**（那两个必须加 `@`），
    // 是本仓库第三处 peg 规则互相矛盾的地方。统一加后缀会让这里整个不可用。
    let fx = svn_repo();
    init(&fx);
    let targets = svn_log::log_targets(&fx.wc_str()).unwrap();
    assert!(!targets.is_empty());
    for t in &targets {
        assert!(!t.ends_with('@'), "log 的 target 不能带 peg 尾缀：{t}");
    }
}

#[test]
fn log_targets_dedupes_working_copies_that_share_a_url() {
    let fx = svn_repo();
    init(&fx);
    let targets = svn_log::log_targets(&fx.wc_str()).unwrap();
    let mut sorted = targets.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), targets.len(), "有重复 target：{targets:?}");
}
