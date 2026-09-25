//! `sync_in` 前置检查（spec §7.2）。全部是**纯读**：每条拒绝都要断言仓库没被动过。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_sync_check::{assert_linear, preview_sync_in};
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
fn an_out_of_sync_baseline_is_refused() {
    // §7.2 第 2 条：外部 TortoiseSVN update 过，基线已经失同步。
    // ⚠️ 只活到 Task 7A：A16 之后「整体前进」改由执行时接管，7A 会删掉这条、换成四条更细的
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
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
