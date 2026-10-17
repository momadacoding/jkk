//! `sync_in` 前置检查（spec §7.2）。全部是**纯读**：每条拒绝都要断言仓库没被动过。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_sync_check::{assert_linear, preview_sync_in};
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use svn_fixtures::{hybrid_with_colleague, svn_cmd};

fn git(fx: &svn_fixtures::SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn head(fx: &svn_fixtures::SvnFixture) -> String {
    git(fx, &["rev-parse", "HEAD"])
}

fn code_of(
    r: Result<app_lib::engine::svn_sync_check::SyncInPreview, app_lib::error::GitError>,
) -> String {
    r.expect_err("应当被拒绝").code
}

#[test]
fn a_clean_hybrid_previews_ok() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.orig_branch, "work");
    assert_eq!(p.orig_head, head(&fx));
    assert_eq!(p.baseline_rev, 1);
    assert_eq!(
        p.target_rev, 2,
        "against revision 就是这次要更新到的 revision"
    );
    assert!(p.outside_changes.is_empty());
}

#[test]
fn detached_head_is_refused() {
    // 必测 #31
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "--detach"]);
    let before = head(&fx);
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_detached_head"
    );
    assert_eq!(head(&fx), before);
}

#[test]
fn being_on_the_baseline_branch_is_refused() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "svn/base"]);
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_on_baseline"
    );
}

#[test]
fn a_merge_in_the_replay_range_is_refused_before_anything() {
    // 必测 #63：merge 独有的文件在普通 rebase 里会无冲突地消失（§7.3.2）
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    git(&fx, &["checkout", "-b", "side"]);
    std::fs::write(wc.join("Code/Side.cs"), "side\n").unwrap();
    git(&fx, &["add", "Code/Side.cs"]);
    git(&fx, &["commit", "-m", "side"]);
    git(&fx, &["checkout", "work"]);
    std::fs::write(wc.join("Code/A.cs"), "a-work\n").unwrap();
    git(&fx, &["commit", "-am", "work"]);
    git(&fx, &["merge", "--no-ff", "-m", "merge side", "side"]);
    let before = head(&fx);
    let err = preview_sync_in(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_merge_in_range");
    assert!(
        err.message.contains(&before[..7]) || err.message.contains(&before),
        "要列出 merge OID"
    );
    assert_eq!(head(&fx), before);
    assert!(wc.join("Code/Side.cs").exists(), "独有文件仍在");
}

#[test]
fn a_failing_merge_query_is_not_treated_as_empty() {
    // §7.3.2：「命令失败 → 检查未完成，停止；不能把失败当作空集合」
    let fx = hybrid_with_colleague();
    let bogus = "0123456789abcdef0123456789abcdef01234567";
    let err = assert_linear(&fx.wc_str(), bogus, &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_in_merge_check_failed");
}

#[test]
fn outside_changes_that_update_will_touch_are_listed() {
    // §7.2 第 5 条的两个方向都要测：只列「本地改过且服务器也改过」的
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap();
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Art/x.psd".to_string()]);
}

#[test]
fn outside_changes_the_update_will_not_touch_are_not_listed() {
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap();
    fx.colleague_commits("Code/A.cs", "a2\n");
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert!(p.outside_changes.is_empty(), "{:?}", p.outside_changes);
}

#[test]
fn a_directory_git_removed_is_not_an_outside_change_when_the_colleague_adds_to_it() {
    // 与交付侧 E1 同一个误报（3a E2E 之后的 review 第 4 条）：`git rm` 删掉目录里最后一个文件时
    // git 顺手删掉空目录，svn 把目录报成 `missing`；同事往这个目录里加文件时目录本身带 incoming，
    // 于是目录被当成「本地改过、更新会覆盖」的文件列进确认框。实测 svn status -u：
    // `Code/Solo` item=missing、repos-status item=modified；`Code/Solo/S.cs` missing；
    // `Code/Solo/T.cs` repos added。update 只会把目录拉回来，不丢任何东西
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Solo/S.cs", "solo\n");
    update_to_head(&fx);
    git(&fx, &["rm", "-q", "Code/Solo/S.cs"]);
    git(&fx, &["commit", "-qm", "drop solo"]);
    assert!(
        !fx.wc().join("Code/Solo").exists(),
        "前提：git 删掉了空目录"
    );
    fx.colleague_commits("Code/Solo/T.cs", "t\n");

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert!(p.outside_changes.is_empty(), "{:?}", p.outside_changes);

    // 不报是对的：真跑一次更新，同事的新文件到位，本地的删除照样重放在上面
    update_to_head(&fx);
    assert!(fx.wc().join("Code/Solo/T.cs").exists());
    assert!(!fx.wc().join("Code/Solo/S.cs").exists());
    assert_eq!(git(&fx, &["log", "-1", "--format=%s"]), "drop solo");
}

#[test]
fn a_removed_directory_still_lists_the_changed_file_inside_it() {
    // 护栏：跳过目录条目之后，目录里真正会被 update 覆盖的非 W-file 仍要报出来
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Art/x.psd", "psd1\n");
    update_to_head(&fx);
    std::fs::remove_dir_all(fx.wc().join("Code/Art")).unwrap();
    fx.colleague_commits("Code/Art/x.psd", "psd2\n");

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/Art/x.psd".to_string()]);
}

#[test]
fn a_directory_deleted_with_files_git_does_not_manage_is_still_listed() {
    // review 第 1 条（2026-10-16 fixture 场景 D）：目录里有 git 管不到的 `x.psd`，用户在资源管理器里
    // 删了整个目录，同事往里加了 `T.cs`。`x.psd` 服务器上没动、不带 incoming，不会单独列出；
    // 这时目录条目是唯一的提醒——update 会把用户删掉的 `x.psd` 一起拉回来
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Solo/S.cs", "solo\n");
    fx.colleague_commits("Code/Solo/x.psd", "psd\n");
    update_to_head(&fx);
    std::fs::remove_dir_all(fx.wc().join("Code/Solo")).unwrap();
    fx.colleague_commits("Code/Solo/T.cs", "t\n");

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/Solo".to_string()]);
}

/// 用「更新」本身把 wc1 和基线一起带到服务器最新（直接 `svn update` 会让基线失同步）。
fn update_to_head(fx: &svn_fixtures::SvnFixture) {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    run_sync_in(
        &fx.wc_str(),
        &SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
}

/// 本地给 `Art/x.psd` 设了属性，同事在服务器上也改了同一个属性——都不碰内容。
fn both_sides_set_a_property(fx: &svn_fixtures::SvnFixture) {
    svn_cmd(&fx.wc(), &["propset", "p", "v-local", "Art/x.psd"]);
    svn_cmd(&fx.wc2(), &["update", "--non-interactive"]);
    svn_cmd(&fx.wc2(), &["propset", "p", "v-server", "Art/x.psd"]);
    svn_cmd(&fx.wc2(), &["commit", "-m", "prop", "--non-interactive"]);
}

#[test]
fn a_props_only_change_on_both_sides_is_listed() {
    // PR #49 review 第三轮 #5（实测）：远端 `item="none" props="modified"`。§7.2 第 5 条
    // 要用户确认的正是这种「git 管不到、update 会碰到」的文件——它会变成属性冲突
    let fx = hybrid_with_colleague();
    both_sides_set_a_property(&fx);
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Art/x.psd".to_string()]);
}

#[test]
fn an_existing_property_conflict_is_refused() {
    // PR #49 review 第三轮 #4（实测）：属性冲突是 `item="normal" props="conflicted"`，
    // 只看 item 与树冲突会把它放过去。同 A6：只更新 Art，避开 nodeRevs 那一条
    let fx = hybrid_with_colleague();
    both_sides_set_a_property(&fx);
    svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive", "Art"],
    );
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_svn_conflicted"
    );
}

#[test]
fn an_svn_conflict_anywhere_is_refused() {
    // A6。只更新 Art，让 Code 仍停在 r1——否则 nodeRevs 先报失同步，测不到这一条
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap();
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive", "Art"],
    );
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_svn_conflicted"
    );
}

#[test]
fn an_externally_advanced_working_copy_previews_a_catch_up() {
    // 计划 A16：外部在项目根 update 过——nodeRevs 单值且比基线新。
    // 预览不拒绝，而是报出要先跟上的 revision；预览本身仍然什么都不改
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.catch_up_rev, Some(2));
    assert_eq!(p.baseline_rev, 1, "baseline_rev 仍是跟上之前的");
    assert_eq!(p.target_rev, 2);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base, "预览不动基线");
}

#[test]
fn an_in_sync_working_copy_previews_no_catch_up() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    assert_eq!(preview_sync_in(&fx.wc_str()).unwrap().catch_up_rev, None);
}

#[test]
fn a_mixed_revision_working_copy_is_still_refused() {
    // A16 只接管「整体前进」。在 wc1 自己身上外部只提交一个文件造出 mixed-revision
    // （spec 必测 #13 的造法），它归 §9.4（2d）
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-external\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "external", "--non-interactive", "Code/A.cs"],
    );
    let base = git(&fx, &["rev-parse", "svn/base"]);
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_out_of_sync"
    );
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base);
}

#[test]
fn a_working_copy_rolled_back_below_the_baseline_is_still_refused() {
    // 反方向的 Diverged：先用 Phase 1 的重建把基线推到 r2，再在外部把工作副本退回 r1
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    app_lib::engine::svn_init::rebuild_baseline(&fx.wc_str()).unwrap();
    svn_cmd(&fx.wc(), &["update", "-r", "1", "--non-interactive"]);
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_out_of_sync"
    );
}

#[test]
fn a_gitignore_that_hides_w_files_is_refused_before_anything() {
    // §7.2 第 6 条后半：当前磁盘的纳入集合预检。不在这里拦，S7 才炸——
    // 那时 svn update 已经做完，用户卡在 svn/base 上。
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/.gitignore"), "*.cs\n").unwrap();
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "include_set_mismatch"
    );
}

#[test]
fn an_unreachable_server_is_refused_before_anything() {
    let fx = hybrid_with_colleague();
    let repo = fx.repo();
    let gone = fx.path().join("repo-gone");
    std::fs::rename(&repo, &gone).unwrap();
    let r = preview_sync_in(&fx.wc_str());
    std::fs::rename(&gone, &repo).unwrap();
    assert_eq!(code_of(r), "sync_in_server_unreachable");
}

#[test]
fn a_rebase_in_progress_is_refused() {
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join(".git/rebase-merge")).unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_git_busy");
}

#[test]
fn an_unfinished_journal_is_refused() {
    use app_lib::engine::svn_journal::{write, Flow, Journal, Phase};
    let fx = hybrid_with_colleague();
    write(
        &fx.wc_str(),
        &Journal {
            opid: "left-behind".into(),
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_unfinished");
}

#[test]
fn a_held_lock_is_refused() {
    let fx = hybrid_with_colleague();
    let _held = app_lib::engine::svn_lock::acquire(&fx.wc_str(), "other").unwrap();
    assert_eq!(code_of(preview_sync_in(&fx.wc_str())), "sync_in_progress");
}

#[test]
fn a_pinned_whitelist_external_is_refused() {
    // A1：两步 update 推迟，pinned 模式下直接拒绝
    let fx = svn_fixtures::svn_repo_pinned_external_updated_to_head();
    app_lib::engine::svn_init::init_hybrid(&app_lib::engine::svn_init::InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
    assert_eq!(
        code_of(preview_sync_in(&fx.wc_str())),
        "sync_in_pinned_unsupported"
    );
}
