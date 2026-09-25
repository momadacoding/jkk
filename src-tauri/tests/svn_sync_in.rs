//! `sync_in` 编排（spec §7.3）。全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::parse_trailer;
use app_lib::engine::svn_journal::{self, Phase};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect, SyncInOutcome};
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}
fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    std::fs::write(fx.wc().join(rel), s).unwrap();
}
fn commit(fx: &SvnFixture, msg: &str) {
    git(fx, &["commit", "-qam", msg]);
}
fn baseline_rev(fx: &SvnFixture) -> u64 {
    parse_trailer(&git(fx, &["log", "-1", "--format=%B", "svn/base"]))
        .unwrap()
        .rev
}
fn expect(fx: &SvnFixture) -> SyncInExpect {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    SyncInExpect {
        orig_head: p.orig_head,
        baseline_oid: p.baseline_oid,
        confirmed_outside: p.outside_changes,
    }
}
fn sync(fx: &SvnFixture) -> SyncInOutcome {
    run_sync_in(&fx.wc_str(), &expect(fx)).unwrap()
}
fn phase(fx: &SvnFixture) -> Option<Phase> {
    svn_journal::read(&fx.wc_str()).unwrap().map(|j| j.phase)
}
fn our_stashes(fx: &SvnFixture) -> usize {
    git(fx, &["stash", "list"])
        .lines()
        .filter(|l| l.contains("mygitclient-sync:"))
        .count()
}

#[test]
fn a_plain_update_advances_the_baseline_and_replays_the_branch() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-local\n");
    commit(&fx, "local");
    let local = git(&fx, &["rev-parse", "HEAD"]);
    write(&fx, "Code/B.cs", "b-wip\n"); // 未提交改动
    write(&fx, "Code/New.cs", "untracked\n"); // 未跟踪的 W-file

    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });

    assert_eq!(baseline_rev(&fx), 2);
    assert_eq!(read(&fx, "Code/A.cs"), "a2\n", "服务器的改动到了磁盘上");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n", "WIP 回来了");
    assert_eq!(read(&fx, "Code/New.cs"), "untracked\n");
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "1");
    assert_ne!(
        git(&fx, &["rev-parse", "HEAD"]),
        local,
        "rebase 之后是新 OID"
    );
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(phase(&fx), None, "S11 删掉 journal");
    assert!(!svn_lock::is_held(&fx.wc_str()), "锁已释放");
    assert_eq!(our_stashes(&fx), 0, "stash 已 drop");
    assert_eq!(
        git(&fx, &["for-each-ref", "refs/mygitclient/"]),
        "",
        "钉住的 ref 已删"
    );
    assert_eq!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync,
        "更新之后不能误报失同步（§7.4：S7 必须跟上 revision）"
    );
}

#[test]
fn nothing_new_on_the_server_still_records_a_baseline_commit() {
    // §7.4：「S7 白名单内无变化时也要落 commit」
    let fx = hybrid_with_colleague();
    let before = git(&fx, &["rev-parse", "svn/base"]);
    let tree_before = git(&fx, &["rev-parse", "svn/base^{tree}"]);
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 1 });
    assert_ne!(git(&fx, &["rev-parse", "svn/base"]), before);
    assert_eq!(git(&fx, &["rev-parse", "svn/base^{tree}"]), tree_before);
}

#[test]
fn a_local_only_patch_survives_sync_in() {
    // 必测 #21（§6.7.1）：Local-Only 就是普通 commit，S9 把它一起搬过去
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-patch\n");
    git(&fx, &["commit", "-qam", "debug patch\n\nLocal-Only: true"]);
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/B.cs"), "b-patch\n");
    assert!(git(&fx, &["log", "-1", "--format=%B", "work"]).contains("Local-Only: true"));
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "1");
}

#[test]
fn a_linear_round_trip_is_not_reported_as_lost() {
    // 必测 #71（§7.3.2）：本地 a1→one→a1，新基线是 one。rebase 后 A 回到 a1，
    // 第一笔因为变空被 --empty=drop 丢掉，不算错误
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "one\n");
    write(&fx, "Code/A.cs", "one\n");
    commit(&fx, "to one");
    write(&fx, "Code/A.cs", "a1\n");
    commit(&fx, "back");
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/A.cs"), "a1\n");
    assert_eq!(git(&fx, &["rev-list", "--count", "svn/base..work"]), "1");
}

#[test]
fn a_rebase_conflict_stops_with_the_lock_released() {
    // 必测 #4 / #69 的 S9 部分：冲突是正常出口，锁必须放开（用户要暂存文件）
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(&fx, "Code/A.cs", "a-local\n");
    commit(&fx, "local A");
    write(&fx, "Code/B.cs", "b-wip\n");
    match sync(&fx) {
        SyncInOutcome::RebaseConflict { files } => assert_eq!(files, vec!["Code/A.cs".to_string()]),
        o => panic!("应当是 rebase 冲突：{o:?}"),
    }
    assert_eq!(phase(&fx), Some(Phase::Rebase));
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert!(fx.wc().join(".git/rebase-merge").exists());
    assert_eq!(baseline_rev(&fx), 2, "基线已推进，冲突发生在它之后");
    assert_eq!(our_stashes(&fx), 1, "WIP 还压着，等 S10");
}

#[test]
fn a_stash_conflict_keeps_the_stash_and_marks_it_applied() {
    // spec §13 S10：「不回滚，stash 与钉住的 ref 都保留不删」
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(&fx, "Code/A.cs", "a-wip\n"); // 未提交
    let oid = match sync(&fx) {
        SyncInOutcome::StashConflict { files, stash_oid } => {
            assert_eq!(files, vec!["Code/A.cs".to_string()]);
            stash_oid
        }
        o => panic!("应当是 stash 冲突：{o:?}"),
    };
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(j.phase, Phase::ApplyingStash);
    assert!(j.stash_applied, "已经 apply 过，继续时不能再 apply");
    assert_eq!(j.stash_oid.as_deref(), Some(oid.as_str()));
    assert_eq!(our_stashes(&fx), 1);
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

#[test]
fn untracked_collision_after_update_stops_before_applying() {
    // Review Focus 1 / A3：服务器新增了与 WIP 里未跟踪文件同名的文件
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "server\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    write(&fx, "Code/N.cs", "mine\n");
    match sync(&fx) {
        SyncInOutcome::StashBlocked { collisions, .. } => {
            assert_eq!(collisions, vec!["Code/N.cs".to_string()])
        }
        o => panic!("应当被撞名挡住：{o:?}"),
    }
    assert_eq!(read(&fx, "Code/B.cs"), "b1\n", "一个字节都没 apply");
    assert_eq!(read(&fx, "Code/N.cs"), "server\n");
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(j.phase, Phase::ApplyingStash);
    assert!(!j.stash_applied);
}

#[test]
fn a_second_sync_while_one_is_running_is_refused() {
    // 必测 #12
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let _other = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_progress");
    assert_eq!(phase(&fx), None);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
}

#[test]
fn new_outside_changes_since_the_preview_require_reconfirmation() {
    // §7.2 第 7 条：确认之后出现了用户没看过的「会被碰到的美术文件」
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    write(&fx, "Art/x.psd", "psd-local\n");
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_needs_reconfirm");
    assert!(err.message.contains("Art/x.psd"));
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn a_branch_moved_since_the_preview_is_refused() {
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    write(&fx, "Code/B.cs", "b-late\n");
    commit(&fx, "committed after preview");
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_state_changed");
    assert_eq!(phase(&fx), None);
}

#[test]
fn a_whitelist_inside_a_same_repo_external_updates() {
    // 必测 #49：公司项目就是这个形态（spec §16.0）。S5 在项目根递归 update 会
    // 连带更新 external；记账单位取 external 那个 WC 的 revision，trailer 的
    // url 取 Repository Root（§6.6.5）
    let fx = svn_fixtures::hybrid_whitelist_in_external();
    fx.colleague_commits("A.cs", "a-server\n");
    write(&fx, "code/New.cs", "wip\n");
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 3 });
    assert_eq!(read(&fx, "code/A.cs"), "a-server\n");
    assert_eq!(read(&fx, "code/New.cs"), "wip\n");
    assert_eq!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    );
    let t = parse_trailer(&git(&fx, &["log", "-1", "--format=%B", "svn/base"])).unwrap();
    assert!(
        t.url.ends_with("/repo"),
        "trailer 记 Repository Root，不是 external 的 URL：{}",
        t.url
    );
}
