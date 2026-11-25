#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
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
fn a_commit_without_update_previews_a_catch_up_at_the_new_floor() {
    // 计划 Task 5：任何 ExternallyAdvanced 都由 A16 接管（重建已能录混合版本）。
    // 在 wc1 自己身上外部只提交一个文件造出 mixed-revision（spec 必测 #13 的造法）：
    // A.cs 在 r2、B.cs 仍在 r1，接管后的 floor 是 1。改造前这里报 sync_in_out_of_sync
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-external\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "external", "--non-interactive", "Code/A.cs"],
    );
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.catch_up_rev, Some(1), "catch_up_rev 是接管之后的 floor");
    assert_eq!(p.baseline_rev, 1);
    assert_eq!(p.target_rev, 2);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base, "预览不动基线");
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

/// 用「更新」把基线推进到服务器最新，断言成功。
fn sync_in(fx: &svn_fixtures::SvnFixture) {
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

/// 2026-11-25 现场：工具目录的编译产物 `obj/*.cs` 是 SVN 忽略（或未版本控制）的，被 SVN
/// 里的 `.gitignore` 挡在 git 之外。它们不是服务器内容，录基线时本来就剔掉——预检不该拦。
fn preview_passes_with_gitignored_build_output(svn_ignore: bool) {
    let fx = hybrid_with_colleague();
    fx.colleague_adds_tool_with_gitignored_obj(svn_ignore);
    sync_in(&fx);
    fx.build_tool_obj();
    fx.colleague_commits("Code/A.cs", "a2\n");
    let before = head(&fx);

    let p = preview_sync_in(&fx.wc_str()).expect("SVN 不管的编译产物不该挡住更新");
    assert_eq!(head(&fx), before);

    // 端到端：预检放行之后，录基线那道最终核对也放行，基线里没有 obj 下的文件
    run_sync_in(
        &fx.wc_str(),
        &SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .expect("预检放行后最终核对也该放行");
    let tree = git(&fx, &["ls-tree", "-r", "--name-only", "svn/base"]);
    assert!(tree.contains("Code/Tool/Main.cs"), "基线：{tree}");
    assert!(
        !tree.contains("/obj/"),
        "obj 下的编译产物混进了基线：{tree}"
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a2\n"
    );
}

#[test]
fn svn_ignored_build_output_hidden_by_a_gitignore_does_not_block_the_preview() {
    preview_passes_with_gitignored_build_output(true);
}

#[test]
fn unversioned_build_output_hidden_by_a_gitignore_does_not_block_the_preview() {
    preview_passes_with_gitignored_build_output(false);
}

#[test]
fn a_gitignore_from_svn_that_hides_a_versioned_w_file_is_still_refused() {
    // 回归：挡住的是 SVN 里受版本控制的 `.cs`（不是 Drop），换成只读预检后照样要拦。
    //
    // 同事把 `.gitignore` 改成连 `*.cs` 都挡，我用 TortoiseSVN 更新拿到了它。预检从空索引
    // 预言（与重建基线同一口径），`Main.cs` 收不进来，所以拦下。更新的 S7 从 HEAD 起步，
    // `Main.cs` 已在基线里、不受 .gitignore 影响，单看这一次其实能录成——拦下是为了不让
    // 基线变成一棵「下次重建就录不出来」的树（独立 review 第 4 条；改造前也是这样拦）。
    let fx = hybrid_with_colleague();
    fx.colleague_adds_tool_with_gitignored_obj(true);
    sync_in(&fx);
    fx.colleague_commits("Code/Tool/.gitignore", "obj/\n*.cs\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    let err = preview_sync_in(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "include_set_mismatch");
    let msg = &err.message;
    assert!(msg.contains("Code/Tool/Main.cs"), "{msg}");
    assert!(msg.starts_with("更新前的检查没通过"), "{msg}");
    // 挡住它的 `.gitignore` 是 SVN 里的文件：不能再建议改它
    assert!(msg.contains("是 SVN 里的文件，不要改它们"), "{msg}");
    assert!(!msg.contains("挪走"), "{msg}");
    assert!(msg.contains("TortoiseSVN"), "{msg}");
}

/// update 被打断（白名单里一个文件被独占占住），用户照提示跑了 `svn cleanup`：白名单根
/// `Code` 停在 `item="incomplete"`。预检不该拿它去问 `classify`（会报 `svn_dirty_unknown_item`
/// 「假基线」）——接下来的 svn update 会把它补齐（独立 review 第 2 条，2026-11-25 实测复现）。
#[cfg(windows)]
#[test]
fn an_interrupted_update_after_cleanup_does_not_trip_the_include_set_precheck() {
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    // 按 2026-11-25 复现出问题的形态造：先用外部 update 走到 r2，再让子目录里的文件被占住。
    // 不先 update、只占住 `Code` 直属文件的形态实测预检本来就放行，测不到这条
    fx.colleague_commits("Code/Sub/C.cs", "c1\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    fx.colleague_commits("Code/A.cs", "a2\n");
    fx.colleague_commits("Code/Sub/C.cs", "c2\n");
    {
        let _hold = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(fx.wc().join("Code/Sub/C.cs"))
            .unwrap();
        let ok = std::process::Command::new("svn")
            .current_dir(fx.wc())
            .args(["update", "--non-interactive"])
            .output()
            .unwrap()
            .status
            .success();
        assert!(!ok, "fixture 前提不成立：被独占占住的文件居然更新成功了");
    }
    svn_cmd(&fx.wc(), &["cleanup", "--non-interactive"]);

    // 改造前预览在这里放行（预检不看脏清单），改造后也该放行
    let p = preview_sync_in(&fx.wc_str()).expect("被打断又 cleanup 过的工作副本照常能预览");
    assert_eq!(p.target_rev, 4);
}

#[test]
fn a_local_gitignore_that_hides_w_files_still_gets_the_move_it_advice() {
    // 不在 SVN 里的 `.gitignore`（最常见：以前在这里用过 git，只删了 .git）照旧建议挪走
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/.gitignore"), "*.cs\n").unwrap();
    let msg = preview_sync_in(&fx.wc_str()).unwrap_err().message;
    assert!(msg.contains("不在 SVN 里，把它们挪走"), "{msg}");
    assert!(!msg.contains("不要改它们"), "{msg}");
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
