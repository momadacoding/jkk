//! §6.6 失同步判定。
//!
//! 每条测试都先在一个**干净**仓库上跑完初始化（P1/P2/P3 都得过），再由测试自己
//! 制造外部变更——「有人用 TortoiseSVN 在外面动了」按定义发生在本应用之外。
//! 直接拿一个已经失同步的 fixture 去初始化是造不出来的：向导会在前置校验就拦下。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_sync::{self, SyncVerdict};
use svn_fixtures::{
    svn_cmd, svn_repo, svn_repo_pinned_external_updated_to_head, svn_repo_three_revisions,
    svn_repo_with_same_repo_nested_external, NestedExternal, SvnFixture,
};

fn init(fx: &SvnFixture) {
    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
}

#[test]
fn a_freshly_initialized_repo_is_in_sync() {
    let fx = svn_repo();
    init(&fx);
    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert_eq!(st.verdict, SyncVerdict::InSync, "实际 {st:?}");
    assert_eq!(st.node_revs.len(), 1);
    assert_eq!(st.node_revs[0], st.baseline_rev);
}

#[test]
fn mixed_revision_is_reported_with_every_value() {
    // 外部只把一个文件退回旧 revision —— 工作副本根的 revision **纹丝不动**
    // （spec §6.6.1），只有「nodeRevs 多值」这一条判据能抓到。
    let fx = svn_repo_three_revisions();
    init(&fx);
    svn_cmd(
        &fx.wc(),
        &["update", "-r1", "--non-interactive", "--", "Code/A.cs@"],
    );

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    match &st.verdict {
        SyncVerdict::MixedRevision { revs } => {
            assert!(revs.len() >= 2, "要把每个值都列出来，实际 {revs:?}");
            assert!(revs.contains(&1) && revs.contains(&3), "实际 {revs:?}");
        }
        other => panic!("应当判成 mixed revision，实际 {other:?}"),
    }
}

#[test]
fn a_single_value_that_differs_from_the_baseline_is_diverged() {
    // 同事提交 + 本机 update：整棵白名单一起前进到新 revision。
    let fx = svn_repo_three_revisions();
    init(&fx);
    let baseline = svn_sync::check_sync(&fx.wc_str()).unwrap().baseline_rev;

    // 「同事的机器」发一次外部提交
    let seed = fx.path().join("seed");
    svn_cmd(&seed, &["update", "--non-interactive"]);
    std::fs::write(seed.join("Code/B.cs"), "v4\n").unwrap();
    svn_cmd(&seed, &["commit", "-m", "r4 外部提交", "--non-interactive"]);
    // 本机 update（模拟用户自己在 TortoiseSVN 里点了 update）
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    match st.verdict {
        SyncVerdict::Diverged {
            node_rev,
            baseline_rev,
        } => {
            assert_eq!(baseline_rev, baseline);
            assert!(node_rev > baseline_rev, "这次是被推进，实际 {node_rev}");
        }
        other => panic!("应当判成 diverged，实际 {other:?}"),
    }
}

#[test]
fn a_reset_to_the_externals_peg_is_distinguished_from_a_plain_divergence() {
    // spec §6.6.2：N 恰好等于该 external 的 peg revision ⇒ 有人在项目根跑了递归
    // `svn update`，把白名单拽回了 peg。
    //
    // 这时的出路是**一条 `svn update <白名单目录>`**，不是 §9.4 的完整恢复流程。
    // 判错的代价很具体：`Diverged` 会引导用户去「重建基线」，而那会把被钉死的
    // **旧内容**录成一个标着新 revision 的基线 —— 自己造出 §9.2 的基线撒谎。
    let fx = svn_repo_pinned_external_updated_to_head();
    init(&fx);

    // 有人在项目根跑了递归 update：SVN 按 externals 定义把 Code 拽回 peg
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    match &st.verdict {
        SyncVerdict::PinnedReset {
            node_rev,
            baseline_rev,
            external,
        } => {
            assert!(node_rev < baseline_rev, "被拽回旧 revision");
            assert!(!external.is_empty(), "要点名是哪条 external");
        }
        other => panic!(
            "应当判成 pinned reset，实际 {other:?}（node_revs={:?}）",
            st.node_revs
        ),
    }
}

#[test]
fn node_revs_unions_every_working_copy_including_nested_externals() {
    // spec §6.6.5：不变量是所有白名单所属工作副本的 nodeRevs **并集**是单元素集合。
    // 只查白名单根自己那个 WC，会漏掉底下嵌套 external 那块
    // 「git 管着、SVN 侧没人盯」的盲区。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    init(&fx);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    // 干净的嵌套 external：两个 WC 都在同一个 revision 上，所以并集仍是单值。
    // 这条验的是**它真的去查了第二个 WC**，靠下面那条证伪。
    assert_eq!(st.verdict, SyncVerdict::InSync, "实际 {st:?}");

    // 把嵌套 external 单独退回旧 revision —— 只有并集才看得见
    svn_cmd(
        &fx.wc(),
        &["update", "-r1", "--non-interactive", "--", "Code/Vendor@"],
    );
    let after = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert!(
        !matches!(after.verdict, SyncVerdict::InSync),
        "嵌套 external 停在别的 revision 必须被看见，实际 {after:?}"
    );
}

#[test]
fn root_revs_covers_every_working_copy_and_is_cheap() {
    // §6.6.3 的 O(1) 提示档：每个 wc(P) 各一条。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    init(&fx);
    let got = svn_sync::read_root_revs(&fx.wc_str()).unwrap();
    assert!(
        got.len() >= 2,
        "嵌套 external 是独立工作副本，必须各占一条，实际 {got:?}"
    );
    assert!(got.iter().all(|r| r.rev > 0));
}

#[test]
fn root_revs_changes_when_someone_updates_outside() {
    // 第一档的判据是「与**上次观测值**比」，所以这个值必须真的会变。
    let fx = svn_repo_three_revisions();
    init(&fx);
    let before = svn_sync::read_root_revs(&fx.wc_str()).unwrap();

    let seed = fx.path().join("seed");
    svn_cmd(&seed, &["update", "--non-interactive"]);
    std::fs::write(seed.join("Code/B.cs"), "v4\n").unwrap();
    svn_cmd(&seed, &["commit", "-m", "r4", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let after = svn_sync::read_root_revs(&fx.wc_str()).unwrap();
    assert_ne!(before, after, "外部 update 之后 rootRev 必须变");
}

#[test]
fn a_repo_without_a_baseline_branch_is_a_loud_error() {
    // 没有 svn/base 时 baselineRev 无从谈起。返回 0 或者 InSync 都是撒谎 ——
    // 前者会让「N != 0」把每个仓库都判成失同步，后者会宣称一个不存在的基线是好的。
    let fx = svn_repo();
    // 故意不跑 init_hybrid
    let err = svn_sync::check_sync(&fx.wc_str()).expect_err("没有基线必须报错");
    assert_eq!(err.code, "svn_no_baseline", "实际 {err:?}");
}
