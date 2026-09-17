//! 初始化向导的前置校验 P1 / P2 / P3（spec §6.5）。
//!
//! P4 在 1a 的 `svn_wc::check_single_repository`，这里只验证它确实被串进了流程。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_preflight::run_preflight;
use app_lib::engine::svn_xml::parse_info;
use svn_fixtures::{
    svn_repo, svn_repo_dirty, svn_repo_mixed_revision, svn_repo_obstructed,
    svn_repo_root_rev_behind, svn_repo_with_cross_repo_nested_external,
    svn_repo_with_same_repo_nested_external, svn_repo_with_svn_ignored_cs,
    svn_repo_with_svn_ignored_non_code_file, DirtyKind, NestedExternal,
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
// P1：白名单内 svn status 只拦 `~`，其余作为脏清单交给 R2a（spec §16.1）
// ---------------------------------------------------------------------------

/// 脏清单里有没有这条路径。
fn dirty_has(pre: &app_lib::engine::svn_preflight::Preflight, path: &str) -> bool {
    pre.dirty.iter().any(|e| e.path == path)
}

#[test]
fn p1_passes_every_ordinary_dirty_state_and_reports_it() {
    // 逐个造出来，而不是只造一个 M 就宣称「覆盖了 P1」
    // （`AGENTS.md` §13 rubric #9：测的不是 X 的真实形态属虚假信心）。
    //
    // spec §16.1 之前这条测的是「每一种脏都必须被拒」。那条门槛让重建基线在真实
    // 开发节奏下**永远点不动**（git 与 SVN 共用工作目录，git commit 之后
    // svn status 仍是 M），所以现在反过来：每一种都必须**放行**，并且如实出现在
    // 脏清单里——R2a 要靠这份清单决定哪些文件改取服务器内容。
    for (kind, path, want_item) in [
        (DirtyKind::Modified, "Code/A.cs", "modified"),
        (DirtyKind::Unversioned, "Code/New.cs", "unversioned"),
        (DirtyKind::Added, "Code/Added.cs", "added"),
        (DirtyKind::Deleted, "Code/B.cs", "deleted"),
    ] {
        let fx = svn_repo_dirty(kind);
        let pre = run_preflight(&fx.wc_str(), &wl(&["Code/"]))
            .unwrap_or_else(|e| panic!("{want_item} 必须放行，实际被拒：{}", e.message));

        let entry = pre
            .dirty
            .iter()
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("{want_item} 必须出现在脏清单里。实际清单：{:?}", pre.dirty));
        assert_eq!(entry.item, want_item, "{path} 的 item");
        assert_eq!(pre.node_rev, 1, "{want_item} 不该影响记账单位");
    }
}

#[test]
fn p1_must_pass_no_ignore_or_svn_ignored_files_slip_through() {
    // **反向证据**：svn:ignore 一个 .cs，再造出那个文件。
    // 不带 --no-ignore 时 svn status 根本不报它，而它匹配白名单扩展名、git 照收。
    //
    // §16.1 之后它不再导致拒绝，但**仍然必须被看见**——R2a 要靠脏清单把它从基线树
    // 里剔除（服务器上没有它）。看不见的话它会被录进基线，交付时必然对不上。
    // 这条如果哪天开始失败，说明有人把 --no-ignore 删了。
    let fx = svn_repo_with_svn_ignored_cs();
    let pre = run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect("SVN-ignored 的文件不再阻断");
    assert!(
        dirty_has(&pre, "Code/Generated.cs"),
        "被 svn:ignore 挡住的 .cs 必须出现在脏清单里。实际清单：{:?}",
        pre.dirty
    );
}

#[test]
fn p1_also_sees_svn_ignored_files_outside_the_extension_list() {
    // 和上一条成对。`Code/Notes.txt` 同样是「SVN 忽略且不在版本控制里」，
    // 区别是它不匹配白名单扩展名、git 不会收它。
    //
    // 它照样要**出现在清单里**：脏清单是 svn status 的如实转述，按扩展名过滤是
    // 下游（R2a / 纳入集合核对）的事，不是这一层的事。这一层多报的代价是零
    // （下游会忽略它），少报的代价是漏掉一个该剔除的文件。
    let fx = svn_repo_with_svn_ignored_non_code_file();
    let pre = run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect("非代码文件同样不再阻断");
    assert!(
        dirty_has(&pre, "Code/Notes.txt"),
        "实际清单：{:?}",
        pre.dirty
    );
}

#[test]
fn p1_still_blocks_an_obstructed_node() {
    // spec §16.1：`~` 是唯一仍然阻断的脏状态——svn 自己都说不清那个位置上是什么，
    // `svn cat -r BASE` 拿不到可信内容，录不出「服务器在 rN 的样子」。
    let fx = svn_repo_obstructed();
    let err = run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect_err("obstructed 必须仍然被拦住");
    assert_eq!(
        err.code, "svn_preflight_obstructed",
        "实际：{} / {}",
        err.code, err.message
    );
    assert!(
        err.message.contains("Code/A.cs"),
        "必须指名是哪一条。实际：{}",
        err.message
    );
    // **不自动修复**：revert / 删目录都是破坏性的，只拦住并引导（spec §6.5 P1）。
    assert!(
        fx.wc().join("Code/A.cs").is_dir(),
        "拦住之后磁盘必须原样——本应用不替用户做破坏性操作"
    );
}

#[test]
fn a_clean_working_copy_reports_an_empty_dirty_list() {
    // 干净时脏清单必须是空的。这条是体检判据 `D \ S == ∅` 退化成 `D == ∅` 的前提
    // （spec §6.8.7）：S 一旦凭空多出条目，判据就被悄悄削弱了。
    let fx = svn_repo();
    let pre = run_preflight(&fx.wc_str(), &wl(&["Code/"])).unwrap();
    assert!(pre.dirty.is_empty(), "实际：{:?}", pre.dirty);
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

#[test]
fn read_node_rev_reports_the_current_value_without_running_the_other_checks() {
    // spec §16 R3c 用它复查「录制期间工作副本有没有被外部改动」。
    //
    // 它必须**只查 nodeRevs**：复用 run_preflight 的话，失败时报出来的会是
    // 「obstructed」或「操作未完成」，而真正发生的事是「录制期间被改了」——
    // 给错原因比不给更糟。
    use app_lib::engine::svn_preflight::read_node_rev;

    // 脏工作副本上照样要能读出来（R3c 跑在录制之后，那时工作副本当然是脏的）
    let fx = svn_repo_dirty(DirtyKind::Modified);
    assert_eq!(read_node_rev(&fx.wc_str(), &wl(&["Code/"])).unwrap(), 1);

    // obstructed 不该妨碍它——那是 P1 的职责，不是这一条的
    let ob = svn_repo_obstructed();
    assert_eq!(read_node_rev(&ob.wc_str(), &wl(&["Code/"])).unwrap(), 1);

    // 但 mixed-revision 仍然要报错：那正是它要挡的形态
    let mixed = svn_repo_mixed_revision();
    let err = read_node_rev(&mixed.wc_str(), &wl(&["Code/"])).expect_err("多值必须报错");
    assert_eq!(err.code, "svn_preflight_mixed_revision");
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
fn p1_sees_a_dirty_file_inside_a_same_repo_nested_external() {
    // 2026-09-15 实测：`status --ignore-externals -- Code@` 对这个形态只报一条
    // `item="external"`（清点按设计跳过它），脏文件完全看不见。
    // 而 `git add -- Code` 照收 `Code/Vendor/V.cs` —— 未提交的内容进基线。
    //
    // §16.1 之后它不再阻断，但**必须仍然被看见**，而且这条比从前更要紧：脏清单
    // 现在是 R2a 的输入，`Code/Vendor/V.cs` 漏掉的话它会被原样录进基线，
    // 而基线宣称自己是服务器 rN 的内容。看不见 = 假基线，不再只是「漏了一道拦截」。
    //
    // 这条测的其实是 `check_targets` 有没有把嵌套 external 的根也当成 target
    // （spec §6.5.5 / §6.6.5）。只查白名单根的话这里会是空清单。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Dirty);
    let pre =
        run_preflight(&fx.wc_str(), &wl(&["Code/"])).expect("嵌套 external 里的未提交改动不再阻断");
    let found = pre
        .dirty
        .iter()
        .find(|e| e.path == "Code/Vendor/V.cs")
        .unwrap_or_else(|| {
            panic!(
                "嵌套 external 里的脏文件必须出现在清单里，否则它会被当成干净内容\
                 原样录进基线。实际清单：{:?}",
                pre.dirty
            )
        });
    assert_eq!(found.item, "modified");
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
