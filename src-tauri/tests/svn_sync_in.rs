//! `sync_in` 编排（spec §7.3）。全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::parse_trailer;
use app_lib::engine::svn_baseline::{create_baseline_commit, TempIndex, Trailer};
use app_lib::engine::svn_config::read_config;
use app_lib::engine::svn_init::record_server_tree_into;
use app_lib::engine::svn_journal::{self, Phase};
use app_lib::engine::svn_journal::{Flow, Journal};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_preflight::run_preflight;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{abort_sync, continue_sync};
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect, SyncInOutcome};
use app_lib::engine::svn_sync_stash::stash_wip;
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

#[test]
fn a_tag_named_like_the_branch_does_not_detach_head() {
    // PR #49 review #1（实测）：有同名 tag 时 `symbolic-ref --short HEAD` 给的是
    // `heads/work`，而 `git checkout heads/work` 退出 0 却让 HEAD detached——S8 报错，
    // 之后继续与放弃都认不出分支，WIP 困在 stash 里
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    git(&fx, &["tag", "work"]);
    write(&fx, "Code/B.cs", "b-wip\n");

    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(git(&fx, &["symbolic-ref", "HEAD"]), "refs/heads/work");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn continue_after_a_rebase_conflict_with_a_tag_named_like_the_branch() {
    // 同一个根因的另一处：A14 拿 `refs/heads/<origBranch>` 与 rebase 的 head-name 比
    let fx = hybrid_with_colleague();
    git(&fx, &["tag", "work"]);
    rebase_conflict(&fx);
    write(&fx, "Code/A.cs", "a-resolved\n");
    git(&fx, &["add", "Code/A.cs"]);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(git(&fx, &["symbolic-ref", "HEAD"]), "refs/heads/work");
}

// —— 造现场 ——

/// 同事改了 A.cs；你在 work 上提交过 B.cs，手上还有一份未提交的 B.cs 改动。
fn with_wip(fx: &SvnFixture) {
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(fx, "Code/B.cs", "b-local\n");
    commit(fx, "local");
    write(fx, "Code/B.cs", "b-wip\n");
}

/// 刚走完 S0。
fn s0(fx: &SvnFixture, opid: &str) -> Journal {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    let j = Journal {
        opid: opid.into(),
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: 1,
        orig_branch: Some(p.orig_branch),
        orig_head: Some(p.orig_head),
        baseline_rev_before: Some(p.baseline_rev),
        baseline_oid_before: Some(p.baseline_oid),
        ..Default::default()
    };
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    j
}

/// 刚走完 S1：WIP 已压栈，phase = stashed。
fn s1(fx: &SvnFixture, opid: &str) -> Journal {
    let mut j = s0(fx, opid);
    stash_wip(&fx.wc_str(), &mut j, &["Code/".to_string()]).unwrap();
    j
}

/// 刚走完 S5：HEAD 在 svn/base、工作副本已更新，phase = capturing-baseline。
fn s5(fx: &SvnFixture, opid: &str) -> Journal {
    let mut j = s1(fx, opid);
    git(fx, &["checkout", "-q", "svn/base"]);
    svn_fixtures::svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    j.phase = Phase::CapturingBaseline;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    j
}

/// 按 S7 的做法录出新基线 commit（不发布）。
fn built_commit(fx: &SvnFixture, j: &Journal, rev_override: Option<u64>) -> (String, String) {
    let root = fx.wc_str();
    let cfg = read_config(&root).unwrap();
    let pre = run_preflight(&root, &cfg.whitelist).unwrap();
    let (tree, _) =
        record_server_tree_into(&root, &cfg, &pre, &TempIndex::from_head(&root).unwrap()).unwrap();
    let t = Trailer {
        url: pre.repos_root,
        rev: rev_override.unwrap_or(pre.node_rev),
        uuid: pre.uuid,
    };
    let c = create_baseline_commit(&root, &tree, j.baseline_oid_before.as_deref(), &t).unwrap();
    (tree, c)
}

fn rebase_conflict(fx: &SvnFixture) {
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(fx, "Code/A.cs", "a-local\n");
    commit(fx, "local A");
    write(fx, "Code/B.cs", "b-wip\n");
    assert!(matches!(sync(fx), SyncInOutcome::RebaseConflict { .. }));
}

// —— start ——

#[test]
fn continue_from_start_just_clears_the_record() {
    let fx = hybrid_with_colleague();
    s0(&fx, "op-s0");
    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(phase(&fx), None);
}

#[test]
fn abort_from_start_just_clears_the_record() {
    let fx = hybrid_with_colleague();
    s0(&fx, "op-s0a");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(phase(&fx), None);
}

// —— stashing（必测 #16 的 S1 部分、#32）——

#[test]
fn continue_from_stashing_with_a_landed_push() {
    // 崩在 `git stash push` 之后、写 stashOid 之前：journal 只说 stashing
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s0(&fx, "op-landed");
    j.phase = Phase::Stashing;
    j.stash_ref_before = None;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(
        &fx,
        &[
            "stash",
            "push",
            "-u",
            "-m",
            "mygitclient-sync:op-landed",
            "--",
            "Code",
        ],
    );

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(
        read(&fx, "Code/B.cs"),
        "b-wip\n",
        "不静默丢掉那条匿名 stash"
    );
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn abort_from_stashing_that_never_landed_changes_nothing() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s0(&fx, "op-never");
    j.phase = Phase::Stashing;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

// —— stashed ——

#[test]
fn continue_from_stashed_on_the_original_branch() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s1(&fx, "op-st1");
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
}

#[test]
fn continue_from_stashed_on_the_baseline_branch() {
    // 崩在 S3 之后
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s1(&fx, "op-st2");
    git(&fx, &["checkout", "-q", "svn/base"]);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
}

#[test]
fn continue_from_stashed_refuses_changes_made_after_the_crash() {
    // A11：锁在崩溃后不存在，用户可能已经在工作区里继续改了
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s1(&fx, "op-st3");
    write(&fx, "Code/A.cs", "edited-after-crash\n");
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_in_state_changed"
    );
    assert_eq!(phase(&fx), Some(Phase::Stashed), "记录原样保留");
    assert_eq!(read(&fx, "Code/A.cs"), "edited-after-crash\n");
}

#[test]
fn abort_from_stashed_restores_the_wip() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let j = s1(&fx, "op-st4");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
}

// —— capturing-baseline ——

#[test]
fn continue_from_capturing_baseline() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s5(&fx, "op-cap");
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(baseline_rev(&fx), 2);
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
}

#[test]
fn abort_after_the_update_returns_to_the_original_branch() {
    // A5：S5 已经做完，SVN 退不回去；回到原分支、放回 WIP，基线不动 → 失同步
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let j = s5(&fx, "op-cap-a");
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(read(&fx, "Code/A.cs"), "a1\n", "工作区回到原分支的内容");
    assert_eq!(baseline_rev(&fx), 1, "基线没发布");
    assert!(
        matches!(
            check_sync(&fx.wc_str()).unwrap().verdict,
            SyncVerdict::Diverged { .. }
        ),
        "工作副本已在 r2、基线还是 r1——如实报失同步，引导重建基线"
    );
}

#[test]
fn abort_after_the_update_refuses_to_overwrite_unknown_changes() {
    // A5 的护栏：W-file 在 svn 看来不干净，说明磁盘上不全是服务器内容，不能 -f
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    s5(&fx, "op-cap-g");
    write(&fx, "Code/A.cs", "someone-edited\n");
    assert_eq!(
        abort_sync(&fx.wc_str()).unwrap_err().code,
        "sync_in_unsafe_to_return"
    );
    assert_eq!(
        read(&fx, "Code/A.cs"),
        "someone-edited\n",
        "一个字节都没覆盖"
    );
    assert_eq!(phase(&fx), Some(Phase::CapturingBaseline));
}

// —— publishing-baseline（必测 #73 的 S7 部分）——

#[test]
fn continue_from_publishing_with_the_commit_built_but_not_published() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s5(&fx, "op-pub1");
    let (tree, c) = built_commit(&fx, &j, None);
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c.clone());
    svn_journal::write(&fx.wc_str(), &j).unwrap();

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(
        git(&fx, &["rev-parse", "svn/base"]),
        c,
        "发布的就是记下来的那个 commit，没有重录"
    );
}

#[test]
fn continue_from_publishing_after_the_ref_already_moved() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s5(&fx, "op-pub2");
    let (tree, c) = built_commit(&fx, &j, None);
    let old = j.baseline_oid_before.clone().unwrap();
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c.clone());
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &c, &old]); // 崩在移动 ref 之后

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), c);
}

#[test]
fn continue_from_publishing_refuses_a_baseline_moved_by_someone_else() {
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let mut j = s5(&fx, "op-pub3");
    let (tree, c) = built_commit(&fx, &j, None);
    let (_, other) = built_commit(&fx, &j, Some(99));
    let old = j.baseline_oid_before.clone().unwrap();
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c);
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &other, &old]);

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "svn_baseline_moved"
    );
    assert_eq!(
        git(&fx, &["rev-parse", "svn/base"]),
        other,
        "别人的不被覆盖"
    );
    assert_eq!(phase(&fx), Some(Phase::PublishingBaseline));
}

// —— rebase（必测 #69 的 S9 部分）——

#[test]
fn continue_after_resolving_a_rebase_conflict() {
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    write(&fx, "Code/A.cs", "a-resolved\n");
    git(&fx, &["add", "Code/A.cs"]);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(read(&fx, "Code/A.cs"), "a-resolved\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn continue_with_unresolved_files_is_refused() {
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    let err = continue_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unresolved");
    assert!(err.message.contains("Code/A.cs"));
    assert_eq!(phase(&fx), Some(Phase::Rebase));
}

#[test]
fn abort_a_rebase_conflict_goes_back_to_before() {
    let fx = hybrid_with_colleague();
    let before_head = {
        fx.colleague_commits("Code/A.cs", "a-server\n");
        write(&fx, "Code/A.cs", "a-local\n");
        commit(&fx, "local A");
        write(&fx, "Code/B.cs", "b-wip\n");
        git(&fx, &["rev-parse", "HEAD"])
    };
    assert!(matches!(sync(&fx), SyncInOutcome::RebaseConflict { .. }));
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(read(&fx, "Code/A.cs"), "a-local\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(
        baseline_rev(&fx),
        2,
        "基线保持新的；分支落后于它，是 Phase 1 就有的合法状态"
    );
    assert!(!fx.wc().join(".git/rebase-merge").exists());
}

#[test]
fn continue_refuses_when_the_branch_moved_during_a_conflict() {
    // Review Focus 4 / A14：冲突期间锁是放开的
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    git(&fx, &["rebase", "--abort"]);
    git(&fx, &["checkout", "-q", "-b", "elsewhere"]);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_in_state_changed"
    );
    assert_eq!(phase(&fx), Some(Phase::Rebase));
    assert_eq!(our_stashes(&fx), 1, "WIP 还在");
}

#[test]
fn continue_replays_again_when_the_rebase_was_aborted_outside() {
    // A14 的另一半：用户在外面 `git rebase --abort`，人还在原分支上、分支没动过。
    // 绝不能直接进 S10——那会把 WIP 放到没有重放过的旧分支上，然后 drop 掉 stash、
    // 宣布完成。原先在这里拒绝；PR #49 review #6 之后改为重新走 S9（与 rebase
    // 没能启动是同一个现场，分不出来，也不需要分）：冲突会原样再出现一次
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    git(&fx, &["rebase", "--abort"]);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert!(matches!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::RebaseConflict { .. }
    ));
    assert_eq!(phase(&fx), Some(Phase::Rebase));
    assert_eq!(our_stashes(&fx), 1, "WIP 还在，没有放到旧分支上");
}

#[test]
fn continue_retries_a_rebase_that_never_started() {
    // PR #49 review #6（实测 git 行为）：rebase 启动就失败时退出 1、不留 rebase 目录、
    // HEAD 不动。挡路的东西清掉之后，「继续」必须能重新走 S9，而不是永远劝用户放弃
    let fx = hybrid_with_colleague();
    s8_left_baseline_bytes(&fx, "op-rb", "b-local\n"); // 没有残留，只是停在 S8 之后
    let lock = fx.wc().join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    let err = continue_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_rebase_failed", "{}", err.message);
    assert_eq!(phase(&fx), Some(Phase::Rebase));
    std::fs::remove_file(&lock).unwrap();

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(read(&fx, "Code/B.cs"), "b-local\n");
    assert_eq!(read(&fx, "Code/A.cs"), "a2\n");
    assert_eq!(read(&fx, "Code/New.cs"), "wip-new\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn abort_does_not_abort_a_rebase_that_is_not_ours() {
    // PR #49 review #4：冲突期间锁是放开的。用户在外面结束了本次的 rebase、又去
    // rebase 别的分支并停在冲突上——此时点横幅上的「放弃」不能把那次 rebase 中止掉
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    git(&fx, &["rebase", "--abort"]);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    let side = git(&fx, &["rev-parse", "HEAD"]);
    assert!(
        run_git(&fx.wc_str(), &["rebase", "svn/base"]).is_err(),
        "side 也停在冲突上"
    );

    let err = abort_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_state_changed", "{}", err.message);
    assert!(
        fx.wc().join(".git/rebase-merge").exists(),
        "别人的 rebase 原样还在"
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join(".git/rebase-merge/head-name"))
            .unwrap()
            .trim(),
        "refs/heads/side"
    );
    assert_eq!(git(&fx, &["rev-parse", "side"]), side);
    assert_eq!(phase(&fx), Some(Phase::Rebase), "记录原样保留");
    assert_eq!(our_stashes(&fx), 1, "WIP 还在");
}

// —— applying-stash ——

#[test]
fn continue_after_resolving_a_stash_conflict_drops_the_stash() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a-server\n");
    write(&fx, "Code/A.cs", "a-wip\n");
    assert!(matches!(sync(&fx), SyncInOutcome::StashConflict { .. }));
    write(&fx, "Code/A.cs", "a-merged\n");
    git(&fx, &["add", "Code/A.cs"]);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(our_stashes(&fx), 0);
    assert_eq!(read(&fx, "Code/A.cs"), "a-merged\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn continue_after_moving_a_colliding_file_applies_the_stash() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "server\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    write(&fx, "Code/N.cs", "mine\n");
    assert!(matches!(sync(&fx), SyncInOutcome::StashBlocked { .. }));
    std::fs::rename(fx.wc().join("Code/N.cs"), fx.wc().join("Code/N.server.cs")).unwrap();
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(read(&fx, "Code/N.cs"), "mine\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn abort_while_applying_the_stash_keeps_the_stash_for_the_user() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "server\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    write(&fx, "Code/N.cs", "mine\n");
    let oid = match sync(&fx) {
        SyncInOutcome::StashBlocked { stash_oid, .. } => stash_oid,
        o => panic!("应当被撞名挡住：{o:?}"),
    };
    // 2c 独立 review I1：这里一个字节都没放回，结局不能是 `Aborted`——
    // 界面对 `Aborted` 说的是「已回到更新前的分支，未提交的改动已放回」，两句都是假的
    assert_eq!(
        abort_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Released {
            stash_oid: oid,
            rolled_back: false
        }
    );
    assert_eq!(read(&fx, "Code/B.cs"), "b1\n", "确实没放回");
    assert_eq!(our_stashes(&fx), 1, "备份留在 stash 列表里给用户");
    assert_eq!(
        git(&fx, &["for-each-ref", "refs/mygitclient/"]),
        "",
        "钉住的 ref 已删"
    );
    assert_eq!(phase(&fx), None);
}

/// S5 冲突 → 回退；服务器这次新增的 `Code/N.cs`（svn 的文件，不在旧基线里，
/// `checkout -f` 不碰它）挡住了放回你那份同名未跟踪文件。回退停在 `applying-stash`。
fn a_rollback_blocked_by_a_server_added_file(fx: &SvnFixture) -> String {
    write(fx, "Art/x.psd", "psd-local\n");
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    fx.colleague_commits("Code/N.cs", "server\n");
    write(fx, "Code/B.cs", "b-wip\n");
    write(fx, "Code/N.cs", "mine\n");
    let head = git(fx, &["rev-parse", "HEAD"]);
    let err = run_sync_in(&fx.wc_str(), &expect(fx)).unwrap_err();
    assert_eq!(err.code, "sync_in_update_conflict", "{}", err.message);
    assert!(
        err.message.contains("放回未提交的改动时遇到问题"),
        "{}",
        err.message
    );
    assert_eq!(git(fx, &["rev-parse", "HEAD"]), head, "分支回到了更新前");
    assert_eq!(baseline_rev(fx), 1, "基线没有推进");
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(j.phase, Phase::ApplyingStash);
    j.stash_oid.unwrap()
}

#[test]
fn continue_after_a_blocked_rollback_is_not_reported_as_an_update() {
    // PR #49 review #3：`applying-stash` 两个方向共用。往回走时继续不能报 `Done`
    // ——前端对 Done 不弹任何提示，用户会以为已经更新好了
    let fx = hybrid_with_colleague();
    a_rollback_blocked_by_a_server_added_file(&fx);
    std::fs::rename(fx.wc().join("Code/N.cs"), fx.wc().join("Code/N.server.cs")).unwrap();

    assert_eq!(continue_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(read(&fx, "Code/N.cs"), "mine\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(our_stashes(&fx), 0);
    assert_eq!(phase(&fx), None);
}

#[test]
fn abort_after_a_blocked_rollback_says_the_branch_went_back() {
    // PR #49 review #3 的另一半：`released` 原先一律说「分支已经在新基线上」
    let fx = hybrid_with_colleague();
    let oid = a_rollback_blocked_by_a_server_added_file(&fx);
    assert_eq!(
        abort_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Released {
            stash_oid: oid,
            rolled_back: true
        }
    );
    assert_eq!(read(&fx, "Code/N.cs"), "server\n", "一个字节都没放回");
    assert_eq!(our_stashes(&fx), 1, "备份留在 stash 列表里给用户");
    assert_eq!(phase(&fx), None);
}

// —— 边界 ——

#[test]
fn a_sync_out_journal_is_not_touched() {
    // A13
    let fx = hybrid_with_colleague();
    let j = Journal {
        opid: "out".into(),
        flow: Flow::SyncOut,
        phase: Phase::Prepare,
        started_at: 1,
        ..Default::default()
    };
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_out_not_supported"
    );
    assert_eq!(
        abort_sync(&fx.wc_str()).unwrap_err().code,
        "sync_out_not_supported"
    );
    assert_eq!(phase(&fx), Some(Phase::Prepare));
}

#[test]
fn continue_without_a_record_says_so() {
    let fx = hybrid_with_colleague();
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_no_unfinished"
    );
}

// —— S3 / S4 / S5 失败回退（spec §13 前五行）——

#[test]
fn a_lying_baseline_is_caught_at_s4_and_rolled_back() {
    // S4 是双保险（§7.4）：revision 对得上，内容却对不上
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let old = git(&fx, &["rev-parse", "svn/base"]);
    let idx = TempIndex::from_head(&root).unwrap();
    idx.replace_blob(&root, "Code/A.cs", b"lie\n").unwrap();
    let tree = idx.write_tree(&root).unwrap();
    let t = parse_trailer(&git(&fx, &["log", "-1", "--format=%B", "svn/base"])).unwrap();
    let lie = create_baseline_commit(&root, &tree, Some(&old), &t).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &lie, &old]);
    write(&fx, "Code/B.cs", "b-wip\n");
    let head = git(&fx, &["rev-parse", "HEAD"]);

    let err = run_sync_in(&root, &expect(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_in_baseline_stale");
    assert!(err.message.contains("Code/A.cs"), "{}", err.message);
    assert!(err.message.contains("已回到分支 `work`"), "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(read(&fx, "Code/A.cs"), "a1\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
    assert_eq!(
        git(&fx, &["rev-parse", "svn/base"]),
        lie,
        "失败只回滚自己（§12.4），不去修基线"
    );
}

#[test]
fn update_failure_returns_to_the_original_branch() {
    // Review Focus 3：从「S3 之后崩溃」继续，此时服务器不可达
    let fx = hybrid_with_colleague();
    with_wip(&fx);
    let j = s1(&fx, "op-net");
    git(&fx, &["checkout", "-q", "svn/base"]);
    let repo = fx.repo();
    let gone = fx.path().join("repo-gone");
    std::fs::rename(&repo, &gone).unwrap();
    let r = continue_sync(&fx.wc_str());
    std::fs::rename(&gone, &repo).unwrap();

    let err = r.unwrap_err();
    assert_eq!(err.code, "sync_in_update_failed");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 1);
    assert_eq!(phase(&fx), None);
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

#[test]
fn an_update_conflict_outside_git_rolls_back_and_points_to_tortoisesvn() {
    // §13 第 5 行 / §7.2.1：用户确认过的美术文件在 update 时冲突
    let fx = hybrid_with_colleague();
    write(&fx, "Art/x.psd", "psd-local\n");
    fx.colleague_commits("Art/x.psd", "psd-server\n");
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-wip\n");
    let e = expect(&fx);
    assert_eq!(e.confirmed_outside, vec!["Art/x.psd".to_string()]);

    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_update_conflict");
    assert!(err.message.contains("Art/x.psd") && err.message.contains("TortoiseSVN"));
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 1, "基线不推进");
    assert_eq!(phase(&fx), None);
    let st = app_lib::engine::svn_cli::run_svn(&fx.wc_str(), &["status", "Art/x.psd"]).unwrap();
    assert!(st.starts_with('C'), "冲突现场原样留给 TortoiseSVN：{st}");
}

#[cfg(windows)]
#[test]
fn a_file_held_open_during_checkout_is_named_and_rolled_back() {
    // Review Focus 2 / A15：Unity 式的 share=Read 占用下，git checkout 退出 0
    // 却没把文件写进去。必须指名文件、回到原分支
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-local\n");
    commit(&fx, "local B"); // 切到 svn/base 必须改写 B.cs
    write(&fx, "Code/New.cs", "wip\n");
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let e = expect(&fx);

    const FILE_SHARE_READ: u32 = 0x1;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(fx.wc().join("Code/B.cs"))
        .unwrap();
    let r = run_sync_in(&fx.wc_str(), &e);
    drop(held);

    let err = r.unwrap_err();
    assert_eq!(err.code, "sync_in_checkout_failed");
    assert!(err.message.contains("Code/B.cs"), "{}", err.message);
    assert!(err.message.contains("已回到分支 `work`"), "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(read(&fx, "Code/B.cs"), "b-local\n");
    assert_eq!(read(&fx, "Code/New.cs"), "wip\n");
    assert_eq!(phase(&fx), None);
}

#[test]
fn a_tree_conflict_rolls_back() {
    // 必测 #9 / 计划 A4：树冲突节点的 `item` 不是 `conflicted`，只看 item 会漏掉，
    // 把一个带树冲突的工作副本当成更新成功继续录基线
    let fx = hybrid_with_colleague();
    write(&fx, "Art/x.psd", "psd-local\n");
    svn_fixtures::svn_cmd(&fx.wc2(), &["update", "--non-interactive"]);
    svn_fixtures::svn_cmd(&fx.wc2(), &["delete", "Art", "--non-interactive"]);
    svn_fixtures::svn_cmd(&fx.wc2(), &["commit", "-m", "del Art", "--non-interactive"]);
    write(&fx, "Code/B.cs", "b-wip\n");
    let e = expect(&fx);

    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_update_conflict", "{}", err.message);
    assert!(err.message.contains("Art"), "{}", err.message);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 1, "基线不推进");
    assert_eq!(phase(&fx), None);
}

/// 10 行的文件内容，`edits` 里的行（从 1 数）换成给定文本。
///
/// 外部 update 的三方合并要有「不同的行」可合：单行文件上服务器与本地一改就是 svn 冲突，
/// 测不到本 Task 要测的「静默合并」。
fn ten_lines(tag: &str, edits: &[(usize, &str)]) -> String {
    (1..=10)
        .map(|i| match edits.iter().find(|(n, _)| *n == i) {
            Some((_, s)) => format!("{s}\n"),
            None => format!("{tag}{i}\n"),
        })
        .collect()
}

/// 先让同事把 A / B 换成 10 行的文件（r2、r3），再走一次正常更新，基线停在 r3。
fn with_ten_line_files(fx: &SvnFixture) {
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[]));
    fx.colleague_commits("Code/B.cs", &ten_lines("b", &[]));
    assert_eq!(sync(fx), SyncInOutcome::Done { new_rev: 3 });
}

/// diff 里真正的增删行（去掉 `+++` / `---` 文件头与 `@@` hunk 头）。
fn changed_lines(diff: &str) -> String {
    diff.lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// TortoiseSVN 在项目根点 update 的等价物。
fn external_root_update(fx: &SvnFixture) {
    svn_fixtures::svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive"],
    );
}

#[test]
fn an_external_root_update_is_taken_over_and_separated_again() {
    // 用户 2026-09-25 提的日常场景（计划 A16）：本地 2 个 commit + 未提交改动，
    // 早上在 TortoiseSVN 里对项目根点了 update
    let fx = hybrid_with_colleague();
    with_ten_line_files(&fx);
    write(&fx, "Code/A.cs", &ten_lines("a", &[(1, "a1-commit")]));
    commit(&fx, "c1");
    write(&fx, "Code/B.cs", &ten_lines("b", &[(1, "b1-commit")]));
    commit(&fx, "c2");
    write(
        &fx,
        "Code/A.cs",
        &ten_lines("a", &[(1, "a1-commit"), (9, "a9-wip")]),
    );
    write(
        &fx,
        "Code/B.cs",
        &ten_lines("b", &[(1, "b1-commit"), (9, "b9-wip")]),
    );
    write(&fx, "Code/New.cs", "new-wip\n");
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server")])); // r4
    external_root_update(&fx);

    // 前提：git 这一侧确实混在一起了——测的是真实形状，不是想象的形状
    let mixed = git(&fx, &["diff", "HEAD", "--", "Code/A.cs"]);
    assert!(
        mixed.contains("+a5-server") && mixed.contains("+a9-wip"),
        "{mixed}"
    );

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(
        (p.catch_up_rev, p.baseline_rev, p.target_rev),
        (Some(4), 3, 4)
    );
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 4 });

    assert_eq!(baseline_rev(&fx), 4);
    assert_eq!(
        git(&fx, &["rev-list", "--count", "svn/base..work"]),
        "2",
        "两个 commit 都在"
    );
    // 分开了：相对 HEAD 只剩自己没提交的；相对基线只剩自己提交的
    // 只看改动行：git 会把 hunk 前一行当「函数名」印进 `@@ … @@ a5-server` 头里，
    // 那是上下文，不是改动（2026-09-26 实测）
    let wip = changed_lines(&git(&fx, &["diff", "HEAD"]));
    assert!(wip.contains("+a9-wip") && wip.contains("+b9-wip"), "{wip}");
    assert!(
        !wip.contains("a5-server"),
        "服务器的改动不该再显示成本地修改：\n{wip}"
    );
    let committed = changed_lines(&git(&fx, &["diff", "svn/base", "HEAD"]));
    assert!(
        committed.contains("+a1-commit") && committed.contains("+b1-commit"),
        "{committed}"
    );
    assert!(
        !committed.contains("a5-server") && !committed.contains("wip"),
        "{committed}"
    );
    // 磁盘上三样都在
    assert_eq!(
        read(&fx, "Code/A.cs"),
        ten_lines("a", &[(1, "a1-commit"), (5, "a5-server"), (9, "a9-wip")])
    );
    assert_eq!(read(&fx, "Code/New.cs"), "new-wip\n");
    assert_eq!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    );
    assert_eq!(phase(&fx), None);
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn a_file_the_server_added_does_not_block_the_catch_up() {
    // 计划 A17：外部 update 带来的新文件 git 视为未跟踪，S1 的 -u 把它收进 stash；
    // 基线跟上之后它又作为已跟踪文件回到磁盘。三处同一个 blob，不能停在 StashBlocked
    let fx = hybrid_with_colleague();
    write(&fx, "Code/B.cs", "b-wip\n");
    fx.colleague_commits("Code/S.cs", "server-new\n"); // r2
    external_root_update(&fx);
    assert_eq!(
        git(&fx, &["status", "--porcelain", "--", "Code/S.cs"]),
        "?? Code/S.cs",
        "前提"
    );

    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 2 });
    assert_eq!(read(&fx, "Code/S.cs"), "server-new\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(
        git(&fx, &["status", "--porcelain"]),
        "M Code/B.cs",
        "S.cs 已被基线跟踪、与 HEAD 一致"
    );
    assert_eq!(our_stashes(&fx), 0);
}

#[test]
fn a_catch_up_then_continues_to_a_newer_server_revision() {
    // 外部 update 到 r4 之后同事又提交了 r5：先跟上 r4，再照常更新到 r5
    let fx = hybrid_with_colleague();
    with_ten_line_files(&fx);
    write(&fx, "Code/B.cs", &ten_lines("b", &[(9, "b9-wip")]));
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server")])); // r4
    external_root_update(&fx);
    fx.colleague_commits(
        "Code/A.cs",
        &ten_lines("a", &[(5, "a5-server"), (7, "a7-server")]),
    ); // r5

    let p = preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!((p.catch_up_rev, p.target_rev), (Some(4), 5));
    assert_eq!(sync(&fx), SyncInOutcome::Done { new_rev: 5 });
    assert_eq!(
        read(&fx, "Code/A.cs"),
        ten_lines("a", &[(5, "a5-server"), (7, "a7-server")])
    );
    assert_eq!(git(&fx, &["diff", "HEAD", "--name-only"]), "Code/B.cs");
    // 基线链上 r4 与 r5 各有一个 commit：接管录的 r4 是 S7 的父提交
    let revs: Vec<u64> = git(&fx, &["log", "--format=%H", "-3", "svn/base"])
        .lines()
        .map(|oid| {
            parse_trailer(&git(&fx, &["log", "-1", "--format=%B", oid]))
                .unwrap()
                .rev
        })
        .collect();
    assert_eq!(revs, vec![5, 4, 3]);
}

#[test]
fn an_external_update_next_to_my_own_edit_stops_as_a_stash_conflict_without_losing_anything() {
    // 2026-09-25 实测：服务器改第 5 行、我没提交的改动在第 6 行时，svn 的三方合并静默
    // 成功，而 git stash apply 把相邻行当成冲突。结局是 StashConflict：内容都在、备份保留
    let fx = hybrid_with_colleague();
    with_ten_line_files(&fx);
    write(&fx, "Code/A.cs", &ten_lines("a", &[(6, "a6-wip")]));
    fx.colleague_commits("Code/A.cs", &ten_lines("a", &[(5, "a5-server")])); // r4
    external_root_update(&fx);
    assert_eq!(
        read(&fx, "Code/A.cs"),
        ten_lines("a", &[(5, "a5-server"), (6, "a6-wip")]),
        "前提：svn 静默合并成功"
    );

    match sync(&fx) {
        SyncInOutcome::StashConflict { files, .. } => {
            assert_eq!(files, vec!["Code/A.cs".to_string()])
        }
        o => panic!("相邻行应当是 stash 冲突：{o:?}"),
    }
    assert_eq!(baseline_rev(&fx), 4, "基线已经跟上，冲突发生在它之后");
    let a = read(&fx, "Code/A.cs");
    assert!(
        a.contains("a5-server") && a.contains("a6-wip"),
        "两边都在：\n{a}"
    );
    assert_eq!(our_stashes(&fx), 1, "原始备份还在");
    assert_eq!(phase(&fx), Some(Phase::ApplyingStash));
}

#[test]
fn a_catch_up_is_refused_when_the_branch_moved_since_the_preview() {
    // 接管会移动基线指针，所以锁内复核必须在它**之前**：确认之后分支被动过就一个字节都不动
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_fixtures::svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    let e = expect(&fx);
    let base = git(&fx, &["rev-parse", "svn/base"]);
    write(&fx, "Code/B.cs", "b-late\n");
    commit(&fx, "committed after preview");
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_state_changed");
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base, "基线没被接管");
    assert_eq!(phase(&fx), None);
}

// —— S8 切回原分支没切干净（2c 独立 review C1）——

/// 停在 `baseline-ready`、HEAD 已在 `work` 上，但 `Code/B.cs` 还是基线的字节。
///
/// 这就是 A15 实测的 L1 形状发生在 S8：文件被 Unity 以 share=Read 占着时
/// `git checkout work` 退出 0、HEAD 已切走，那个文件却没写进去。这里不真的去占
/// 文件（那只在 Windows 上造得出），直接造出 checkout 之后的磁盘状态。
///
/// 你在 `work` 上提交过 `B.cs = b-local`；未提交的改动是一个新文件 `New.cs`。
fn s8_left_baseline_bytes(fx: &SvnFixture, opid: &str, residue: &str) -> Journal {
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(fx, "Code/B.cs", "b-local\n");
    commit(fx, "local B");
    write(fx, "Code/New.cs", "wip-new\n");
    let mut j = s5(fx, opid);
    let (tree, c) = built_commit(fx, &j, None);
    let old = j.baseline_oid_before.clone().unwrap();
    git(fx, &["update-ref", "refs/heads/svn/base", &c, &old]); // S7 发布
    git(fx, &["read-tree", "svn/base"]); // S7 索引对齐
    git(fx, &["checkout", "-q", "work"]); // S8
    write(fx, "Code/B.cs", residue); // checkout 没写进去的那个文件
    j.phase = Phase::BaselineReady;
    j.capture_tree = Some(tree);
    j.new_baseline_oid = Some(c);
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    j
}

#[test]
fn continue_after_s8_left_baseline_bytes_does_not_revert_my_commit() {
    // review 实测：`rebase.autoStash=true` 时 rebase 把残留收起、重放、再放回，退出 0，
    // 结局报 Done——磁盘上却是不含你那笔提交的内容。显式打开它，让测试不随开发机配置变化
    let fx = hybrid_with_colleague();
    s8_left_baseline_bytes(&fx, "op-s8c", "b1\n");
    git(&fx, &["config", "rebase.autoStash", "true"]);

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 2 }
    );
    assert_eq!(
        read(&fx, "Code/B.cs"),
        "b-local\n",
        "你提交过的内容在磁盘上"
    );
    assert_eq!(read(&fx, "Code/A.cs"), "a2\n");
    assert_eq!(read(&fx, "Code/New.cs"), "wip-new\n");
    assert_eq!(
        git(&fx, &["status", "--porcelain", "--", "Code/B.cs"]),
        "",
        "B.cs 与 HEAD 一致，不显示成「撤销提交」的本地修改"
    );
}

#[test]
fn abort_after_s8_left_baseline_bytes_does_not_revert_my_commit() {
    let fx = hybrid_with_colleague();
    let j = s8_left_baseline_bytes(&fx, "op-s8a", "b1\n");

    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert_eq!(read(&fx, "Code/B.cs"), "b-local\n");
    assert_eq!(read(&fx, "Code/New.cs"), "wip-new\n");
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code/B.cs"]), "");
}

#[test]
fn the_record_says_head_may_have_left_the_branch_once_s3_starts() {
    // PR #49 review #2 的前提：`leftOrig` 必须由 S3 自己在 checkout 之前落盘
    let fx = hybrid_with_colleague();
    rebase_conflict(&fx);
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert!(j.left_orig);
}

#[test]
fn abort_without_wip_does_not_skip_what_a_rollback_left_behind() {
    // PR #49 review #2：没有可压的改动时 phase 一路是 `stashed`、stashOid 为空。
    // S4 / S5 失败回退，那次 checkout 退出 0 却留下了基线的 B.cs，回退本身报错、
    // 记录保留。之后点「放弃」：HEAD 已在原分支上，原先残留比对被整个跳过，
    // 结局报「已放回」，磁盘上却是撤掉了你那笔提交的内容
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/B.cs", "b-local\n");
    commit(&fx, "local B");
    let mut j = s1(&fx, "op-nowip");
    assert!(j.stash_oid.is_none(), "没有 WIP");
    j.left_orig = true; // S3 写的（上一条测试守着）
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    git(&fx, &["checkout", "-q", "svn/base"]);
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, "Code/B.cs", "b1\n"); // 回退那次 checkout 没写进去

    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(
        read(&fx, "Code/B.cs"),
        "b-local\n",
        "你提交过的内容在磁盘上"
    );
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code/"]), "");
    assert_eq!(phase(&fx), None);
}

/// 服务器这次新增了 `Code/N.cs`；S8 切回 `work` 时它没被删掉（Unity 刚导入、正占着），
/// 作为未跟踪文件留在原分支上。`tracked_changes` 只看被跟踪的文件，看不见它。
fn s8_left_a_server_added_file(fx: &SvnFixture, opid: &str, bytes: &str) -> Journal {
    fx.colleague_commits("Code/N.cs", "n-server\n");
    let j = s8_left_baseline_bytes(fx, opid, "b-local\n");
    assert!(
        !fx.wc().join("Code/N.cs").exists(),
        "S8 正常时 git 会删掉它"
    );
    write(fx, "Code/N.cs", bytes);
    j
}

#[test]
fn continue_after_s8_left_a_server_added_file_behind() {
    // PR #49 review #7（实测 git 行为）：同名未跟踪文件即使内容相同也会挡住 S9 的
    // checkout，rebase 启动就失败。它是新基线里原样存着的字节，删掉不丢任何东西
    let fx = hybrid_with_colleague();
    s8_left_a_server_added_file(&fx, "op-n", "n-server\n");

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Done { new_rev: 3 }
    );
    assert_eq!(read(&fx, "Code/N.cs"), "n-server\n", "S9 之后由 git 写回");
    assert_eq!(read(&fx, "Code/B.cs"), "b-local\n");
    assert_eq!(read(&fx, "Code/New.cs"), "wip-new\n");
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code/N.cs"]), "");
}

#[test]
fn abort_after_s8_left_a_server_added_file_behind() {
    let fx = hybrid_with_colleague();
    let j = s8_left_a_server_added_file(&fx, "op-na", "n-server\n");

    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), j.orig_head.unwrap());
    assert!(
        !fx.wc().join("Code/N.cs").exists(),
        "与一次切干净的 checkout 相同：原分支上没有这个文件"
    );
    assert_eq!(read(&fx, "Code/New.cs"), "wip-new\n");
}

#[test]
fn a_leftover_whose_bytes_are_not_the_baseline_is_not_deleted() {
    let fx = hybrid_with_colleague();
    s8_left_a_server_added_file(&fx, "op-nx", "someone-else\n");

    let err = continue_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unsafe_to_return", "{}", err.message);
    assert!(err.message.contains("Code/N.cs"), "{}", err.message);
    let err = abort_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unsafe_to_return", "{}", err.message);
    assert_eq!(read(&fx, "Code/N.cs"), "someone-else\n", "一个字节都没动");
    assert_eq!(phase(&fx), Some(Phase::BaselineReady));
}

#[cfg(windows)]
#[test]
fn a_file_that_checkout_could_not_delete_is_named_and_rolled_back() {
    // PR #49 review #7 的源头：你提交过的新文件在 S3 切到基线时被占着删不掉，
    // checkout 退出 0，文件作为未跟踪文件留在基线分支上。原先 A15 只核对被跟踪的文件
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/A.cs", "a2\n");
    write(&fx, "Code/Mine.cs", "mine\n");
    git(&fx, &["add", "Code/Mine.cs"]);
    commit(&fx, "add Mine");
    write(&fx, "Code/B.cs", "b-wip\n");
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let e = expect(&fx);

    const FILE_SHARE_READ: u32 = 0x1;
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(fx.wc().join("Code/Mine.cs"))
        .unwrap();
    let r = run_sync_in(&fx.wc_str(), &e);
    drop(held);

    let err = r.unwrap_err();
    assert_eq!(err.code, "sync_in_checkout_failed", "{}", err.message);
    assert!(err.message.contains("Code/Mine.cs"), "{}", err.message);
    assert!(err.message.contains("已回到分支 `work`"), "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(read(&fx, "Code/Mine.cs"), "mine\n");
    assert_eq!(read(&fx, "Code/B.cs"), "b-wip\n");
    assert_eq!(baseline_rev(&fx), 1);
    assert_eq!(phase(&fx), None);
}

#[test]
fn bytes_that_are_neither_the_branch_nor_the_baseline_are_not_overwritten() {
    // 残留不是基线的字节 ⇒ 不知道是谁改的，继续与放弃都不能覆盖
    let fx = hybrid_with_colleague();
    s8_left_baseline_bytes(&fx, "op-s8x", "edited-after-crash\n");

    let err = continue_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unsafe_to_return", "{}", err.message);
    assert!(err.message.contains("Code/B.cs"), "{}", err.message);
    let err = abort_sync(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unsafe_to_return", "{}", err.message);
    assert_eq!(
        read(&fx, "Code/B.cs"),
        "edited-after-crash\n",
        "一个字节都没覆盖"
    );
    assert_eq!(phase(&fx), Some(Phase::BaselineReady), "记录原样保留");
    assert_eq!(our_stashes(&fx), 1, "WIP 还在");
}
