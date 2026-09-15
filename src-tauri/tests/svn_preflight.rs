//! 初始化向导的前置校验 P1 / P2 / P3（spec §6.5）。
//!
//! P4 在 1a 的 `svn_wc::check_single_repository`，这里只验证它确实被串进了流程。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_preflight::run_preflight;
use app_lib::engine::svn_xml::parse_info;
use svn_fixtures::{
    svn_repo, svn_repo_dirty, svn_repo_mixed_revision, svn_repo_root_rev_behind,
    svn_repo_with_cross_repo_nested_external, svn_repo_with_same_repo_nested_external,
    svn_repo_with_svn_ignored_cs, svn_repo_with_svn_ignored_non_code_file, DirtyKind,
    NestedExternal,
};

fn wl(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// `svn info .` 报的 revision。**不是**记账单位，用来证明 fixture 真的造出了
/// 「rootRev 与 nodeRevs 不等」的形态。
fn read_root_rev(wc: &str) -> i64 {
    let xml = run_svn(wc, &["info", "--xml"]).unwrap();
    parse_info(&xml).unwrap()[0].revision
}

// ---------------------------------------------------------------------------
// P1：白名单内 svn status 必须干净
// ---------------------------------------------------------------------------

#[test]
fn p1_rejects_every_dirty_state() {
    // 逐个造出来，而不是只造一个 M 就宣称「覆盖了 P1」
    // （`AGENTS.md` §13 rubric #9：测的不是 X 的真实形态属虚假信心）。
    for (kind, want) in [
        (DirtyKind::Modified, "modified"),
        (DirtyKind::Unversioned, "unversioned"),
        (DirtyKind::Added, "added"),
        (DirtyKind::Deleted, "deleted"),
    ] {
        let fx = svn_repo_dirty(kind);
        let Err(err) = run_preflight(&fx.wc_str(), &wl(&["Code/"])) else {
            panic!("{want} 必须被 P1 拒绝，实际通过了");
        };
        assert_eq!(err.code, "svn_preflight_dirty", "{want}");
        assert!(err.message.contains(want), "{want} 实际：{}", err.message);
    }
}

#[test]
fn p1_must_pass_no_ignore_or_svn_ignored_files_slip_through() {
    // **反向证据**：svn:ignore 一个 .cs，再造出那个文件。
    // 不带 --no-ignore 时 svn status 不报它，而它匹配白名单扩展名、git 照收 → 假基线。
    // 这条如果哪天开始失败，说明有人把 --no-ignore 删了。
    let fx = svn_repo_with_svn_ignored_cs();
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect_err("SVN-ignored 但 git 会收的文件必须被 P1 拦住");
    assert_eq!(err.code, "svn_preflight_dirty");
    assert!(
        err.message.contains("Generated.cs"),
        "实际：{}",
        err.message
    );
}

#[test]
fn p1_rejects_svn_ignored_files_outside_the_extension_list_too() {
    // 和上一条成对。`Code/Notes.txt` 同样是「SVN 忽略且不在版本控制里」，
    // 区别是它不匹配白名单扩展名、git 不会收它。
    //
    // **它一样要被拦。** spec §6.5 的禁止清单里 `?` 是明写的，判据就是
    // 「白名单内 status 干净」，不按扩展名放行。风险方向也是这边对：误拦看得见
    // （错误信息直接列路径，用户当场能判断），放行一个 SVN 不管的文件看不见。
    let fx = svn_repo_with_svn_ignored_non_code_file();
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect_err("白名单目录里 SVN 不管的文件一律要拦");
    assert_eq!(err.code, "svn_preflight_dirty");
    assert!(err.message.contains("Notes.txt"), "实际：{}", err.message);
}

#[test]
fn a_clean_working_copy_passes_and_reports_the_three_facts() {
    let fx = svn_repo();
    let pre = run_preflight(&fx.wc_str(), &wl(&["Code/"])).unwrap();
    assert_eq!(pre.node_rev, 1);
    assert!(pre.repos_root.starts_with("file:///"), "{}", pre.repos_root);
    assert!(!pre.uuid.is_empty());
}

#[test]
fn an_empty_whitelist_is_rejected_instead_of_recording_an_empty_baseline() {
    // 空白名单一条 svn 命令都不发（spec §12.2），于是 nodeRevs 无从谈起。
    // 不拦住的话会录出一个空基线，而空基线在体检里**看起来是通过的**
    // （没有文件 → 没有差异）。
    let fx = svn_repo();
    let err = run_preflight(&fx.wc_str(), &wl(&[])).expect_err("空白名单必须被拒");
    assert_eq!(err.code, "svn_preflight_empty_whitelist");
}

// ---------------------------------------------------------------------------
// P2：nodeRevs 必须是单元素集合
// ---------------------------------------------------------------------------

#[test]
fn p2_rejects_a_real_mixed_revision_working_copy() {
    // 整体 r3，Code/A.cs 单独退回 r1 → nodeRevs = {1,3}。
    let fx = svn_repo_mixed_revision();
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect_err("mixed-revision 必须被拒");
    assert_eq!(err.code, "svn_preflight_mixed_revision");
    // 错误信息里要带上到底是哪几个 revision，否则用户不知道该更新什么
    assert!(err.message.contains('1') && err.message.contains('3'));
}

#[test]
fn p2_reads_node_revs_not_the_root_revision() {
    // spec §6.5.1 的陷阱：根留在 r1、Code 子树更到 r3 时，
    // `svn info .` 报 1 而 nodeRevs = {3}。
    //
    // 这是**合法**状态，P2 必须放行；但记账单位得取 3。取成 1 的话，初始化
    // 完成的那一瞬间基线 trailer 就写着 r1，而工作副本在 r3 —— 立刻判失同步。
    //
    // 这条测试的价值全在于 fixture 真的造出了两者不等的形态，所以先断言前提。
    let fx = svn_repo_root_rev_behind();
    let root_rev = read_root_rev(&fx.wc_str());
    assert_eq!(root_rev, 1, "fixture 没造出 rootRev 落后的形态");

    let pre = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect("rootRev 落后但 nodeRevs 单值是合法状态，不该被拒");
    assert_eq!(
        pre.node_rev, 3,
        "记账单位必须取 nodeRevs，不是 svn info . 的 revision"
    );
    assert_ne!(pre.node_rev as i64, root_rev);
}

// ---------------------------------------------------------------------------
// P3：不在 svn update / merge 中途
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn p3_rejects_a_working_copy_left_mid_update() {
    use svn_fixtures::svn_repo_interrupted_update;
    let fx = svn_repo_interrupted_update();
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect_err("update 半路失败的工作副本必须被拒");
    assert_eq!(err.code, "svn_preflight_operation_in_progress");
}

#[cfg(windows)]
#[test]
fn p3_still_rejects_after_the_user_ran_svn_cleanup() {
    // **P3 存在的全部理由。** 2026-09-14 实测：cleanup 清掉了 wc-locked，
    // 但工作副本根仍是 item="incomplete"，而白名单根 `Code@` 的 status
    // 一条 entry 都没有 —— P1 全绿、P2 单值。
    //
    // 只查白名单根的 P3 会在这里放行，然后拿一棵没更新完的树去录基线。
    use svn_fixtures::svn_repo_incomplete_after_cleanup;
    let fx = svn_repo_incomplete_after_cleanup();

    // 先把前提钉死：白名单根看起来确实是干净的
    let xml = run_svn(
        &fx.wc_str(),
        &[
            "status",
            "--no-ignore",
            "--ignore-externals",
            "--xml",
            "--",
            "Code@",
        ],
    )
    .unwrap();
    assert!(
        !xml.contains("<entry"),
        "fixture 前提不成立：白名单根并不干净，这条测试就失去意义了\n{xml}"
    );

    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect_err("cleanup 之后仍然 incomplete 的工作副本必须被拒");
    assert_eq!(err.code, "svn_preflight_operation_in_progress");
    assert!(
        err.message.contains("incomplete") || err.message.contains("未完成"),
        "实际：{}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// 同仓库的嵌套 external：P4 按设计放行，所以 P1/P2 必须自己盯住它
//
// spec §6.5.5：「同仓库的嵌套 external 不阻断，但它必须被纳入 §6.6.5 的记账单位
// 集合——也就是 nodeRevs 要在它上面也跑一次并取并集，否则它就是一块 git 管着、
// SVN 侧却没人盯的盲区。」
// spec §6.6.5：「在每个 wc(P) 上各跑一次……这里的『每个 wc(P)』指白名单递归覆盖到的
// 全部工作副本，包括白名单根底下再挂的嵌套 external。」
// ---------------------------------------------------------------------------

#[test]
fn p1_catches_a_dirty_file_inside_a_same_repo_nested_external() {
    // 2026-09-15 实测：`status --ignore-externals -- Code@` 对这个形态只报一条
    // `item="external"`（check_clean 按设计跳过它），脏文件完全看不见。
    // 而 `git add -- Code` 照收 `Code/Vendor/V.cs` —— 未提交的内容进基线。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Dirty);
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect_err("嵌套 external 里的未提交改动必须被 P1 拦住");
    assert_eq!(err.code, "svn_preflight_dirty", "实际：{}", err.message);
    assert!(
        err.message.contains("Vendor/V.cs") || err.message.contains("Vendor\\V.cs"),
        "错误信息必须指出是哪个文件，否则用户无从下手：{}",
        err.message
    );
}

#[test]
fn p2_catches_a_pinned_nested_external_left_at_another_revision() {
    // 根在 r3、`Code/Vendor` 被 `-r1` 钉在 r1。实测今天的 P2 读到 {3} 直接放行，
    // 于是基线会宣称「整棵树 == 服务器 r3」，而 Vendor 里装的是 r1 的内容。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::PinnedOlder);
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
        .expect_err("嵌套 external 停在另一个 revision 时必须被 P2 拦住");
    assert_eq!(
        err.code, "svn_preflight_mixed_revision",
        "实际：{}",
        err.message
    );
}

#[test]
fn a_clean_same_repo_nested_external_still_passes() {
    // **反向证据，比上面两条更重要。** 本设计的核心场景就是「代码在 external 里」
    // （spec §6.6.5），把嵌套 external 一律判脏会让向导在目标仓库上永远过不去。
    // 少了这一条，上面两条用「见到 external 就拒绝」也能变绿。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    let pre =
        run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect("干净的同仓库嵌套 external 必须放行");
    assert!(pre.node_rev > 0);
}

// ---------------------------------------------------------------------------
// P4：串进流程了没有
// ---------------------------------------------------------------------------

#[test]
fn p4_is_wired_into_the_preflight_not_just_available() {
    // 1a 实现了 check_single_repository，但「实现了」和「向导真的会调它」是两回事。
    // 白名单同时覆盖主仓库和一条跨仓库 external 时必须被拒。
    let fx = svn_repo_with_cross_repo_nested_external();
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect_err("白名单跨两个仓库时必须被拒");
    assert_eq!(err.code, "svn_cross_repo");
}
