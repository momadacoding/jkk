#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 自动跟进（分析 §6.12；计划 2026-11-06-svn-auto-follow）。
//!
//! 每条条件一个「不满足就不自动、一个 ref 都不动、写明原因」的用例，外加全部满足时自动发布且
//! 磁盘一个字节不动、与同步锁互斥、崩溃之后能恢复。现场的搭法与 `svn_follow.rs` 相同：wc1 是
//! 「你」，wc2 是同事，「TortoiseSVN 提交」就是在 wc1 里直接 `svn commit` 一个文件。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::fault::{self, Crashed, Point};
use app_lib::engine::svn_follow::{
    follow, follow_auto, manual_follow, resume_follow, AutoFollowOutcome, FollowOutcome,
    FollowReport,
};
use app_lib::engine::svn_follow_mark::write_mark;
use app_lib::engine::svn_journal::{self, FollowStep};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_in::SyncInOutcome;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";
const L_MSG: &str = "本地补丁\n\nLocal-Only: true";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args)
        .unwrap_or_else(|e| panic!("git {args:?}: {}", e.message))
        .trim()
        .to_string()
}
fn oid(fx: &SvnFixture, rev: &str) -> String {
    git(fx, &["rev-parse", rev])
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}
fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn commit_all(fx: &SvnFixture, msg: &str) -> String {
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", msg]);
    oid(fx, "HEAD")
}
fn show(fx: &SvnFixture, spec: &str) -> String {
    git(fx, &["show", spec])
}
fn tortoise_commit(fx: &SvnFixture, rel: &str) {
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", rel],
    );
}
fn svn_update(fx: &SvnFixture, rel: &str) {
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", rel]);
}
fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}
/// file:// 下作者都是系统用户，同事的 revision 要在 update 之前改 revprop（同 `svn_follow.rs`）。
fn set_author(fx: &SvnFixture, rev: u64, who: &str) {
    let f = fx.path().join(format!("author-{rev}.txt"));
    std::fs::write(&f, who).unwrap();
    let out = Command::new("svnadmin")
        .args([
            "setrevprop",
            &fx.repo().to_string_lossy(),
            "-r",
            &rev.to_string(),
            "svn:author",
            &f.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn set_me(fx: &SvnFixture, who: &str) {
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some(who.to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();
}
/// 「我」取 wc1 里提交的作者（同 `svn_follow.rs` 的 `i_commit`）。
fn i_commit(fx: &SvnFixture) {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(["info", "--show-item", "last-changed-author", "--", A])
        .output()
        .unwrap();
    assert!(out.status.success());
    set_me(fx, String::from_utf8_lossy(&out.stdout).trim());
}

fn auto(fx: &SvnFixture) -> AutoFollowOutcome {
    follow_auto(&fx.wc_str(), "auto1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
fn done(o: AutoFollowOutcome) -> FollowReport {
    match o {
        AutoFollowOutcome::Done { report } => *report,
        other => panic!("应当自动跟进完成：{other:?}"),
    }
}
/// 没自动跟进：返回原因的 code 与文案。
fn skipped(o: AutoFollowOutcome) -> (String, String) {
    match o {
        AutoFollowOutcome::Skipped { code, message } => (code, message),
        other => panic!("应当不自动跟进：{other:?}"),
    }
}
fn manual_done(o: FollowOutcome) -> FollowReport {
    match o {
        FollowOutcome::Done { report } => *report,
        other => panic!("点「跟进」应当完成：{other:?}"),
    }
}

/// 全部 ref（含钉住旧索引的 `refs/mygitclient/follow/*`）与索引的字节。不自动时一个都不能变。
fn state(fx: &SvnFixture) -> (String, Vec<u8>) {
    (
        git(fx, &["for-each-ref"]),
        std::fs::read(fx.wc().join(".git/index")).unwrap(),
    )
}
fn assert_no_residue(fx: &SvnFixture) {
    assert!(
        !fx.wc().join(".git/index.lock").exists(),
        "不该留下 index.lock"
    );
    assert!(
        svn_journal::read(&fx.wc_str()).unwrap().is_none(),
        "不该留下 journal"
    );
    assert!(!svn_lock::is_held(&fx.wc_str()), "不该还拿着同步锁");
}
/// 不自动跟进的共同断言：原因对得上，一个字节都没动，什么都没留下。
fn assert_skipped_untouched(fx: &SvnFixture, code: &str) -> String {
    let before = state(fx);
    let (got, message) = skipped(auto(fx));
    assert_eq!(got, code, "{message}");
    assert_eq!(state(fx), before, "不自动时 ref 与索引一个字节都不能变");
    assert_no_residue(fx);
    assert!(!message.is_empty());
    message
}
fn staged_paths(fx: &SvnFixture) -> Vec<String> {
    git(fx, &["diff", "--cached", "--name-only"])
        .lines()
        .map(str::to_string)
        .collect()
}
fn modified_paths(fx: &SvnFixture) -> Vec<String> {
    git(fx, &["diff", "--name-only"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// 工作副本全部文件（不含 `.git`、`.svn`）的内容与 mtime。
fn wc_snapshot(fx: &SvnFixture) -> BTreeMap<String, (Vec<u8>, std::time::SystemTime)> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, (Vec<u8>, std::time::SystemTime)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" || name == ".svn" {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let mtime = e.metadata().unwrap().modified().unwrap();
                out.insert(rel, (std::fs::read(&p).unwrap(), mtime));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&fx.wc(), &fx.wc(), &mut out);
    out
}

// —— 全部满足：自动发布 ——

#[test]
fn tortoise_commit_is_followed_automatically_without_touching_the_disk() {
    // 11-05 的源头：TortoiseSVN 提交了 A，B 还在改。自动跟进之后「本地改动」只剩 B
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    write(&fx, "Code/Untracked.cs", "u\n");
    write(&fx, "Art/y.psd", "psd\n");
    tortoise_commit(&fx, A);
    let before = wc_snapshot(&fx);

    let rep = done(auto(&fx));

    assert_eq!(
        wc_snapshot(&fx),
        before,
        "磁盘一个字节、一个 mtime 都不能动"
    );
    assert_ne!(oid(&fx, "svn/base"), old_base);
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
    assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a2");
    assert_eq!(modified_paths(&fx), vec![B]);
    assert!(rep.verified);
    assert_eq!(rep.changes.len(), 1);
    assert_no_residue(&fx);
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
}

#[test]
fn branch_commits_on_other_files_are_replayed_automatically() {
    // 条件 11 只管碰到同一批文件的 commit：分支上改的是 W，SVN 上新的是 A
    let fx = hybrid_with_colleague();
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    done(auto(&fx));

    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"));
    assert_eq!(show(&fx, "HEAD:Code/W.cs"), "w");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn a_partly_committed_commit_is_followed_when_only_its_other_paths_remain() {
    // 条件 11 按重放之后算：C 改了 A 与 B，A 用 TortoiseSVN 提交了，C' 只剩 B，不与外部变更相交
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    let c = commit_all(&fx, "改了两个文件");
    write(&fx, A, "a3\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);

    let rep = done(auto(&fx));

    assert_eq!(rep.rewritten, vec![(c, vec![A.to_string()])]);
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a3");
    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "b2");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn an_intermediate_stage_of_my_own_commit_is_discarded_automatically() {
    // 条件 12 不拦 discarded：作者是我、确定早于提交，是判得准的那一行。原内容钉住
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a-mid\n");
    git(&fx, &["add", A]);
    write(&fx, A, "a-final\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);

    let rep = done(auto(&fx));

    assert_eq!(rep.stage.discarded, vec![A.to_string()]);
    assert!(staged_paths(&fx).is_empty());
    let pin = "refs/mygitclient/follow/auto1:index";
    assert!(!git(&fx, &["rev-parse", pin]).is_empty(), "旧索引钉住了");
}

// —— 条件不满足：不自动，一个 ref 都不动 ——

#[test]
fn nothing_to_follow_is_not_followed() {
    let fx = hybrid_with_colleague();
    assert_skipped_untouched(&fx, "svn_follow_nothing_to_do");
}

#[test]
fn a_lagging_branch_without_new_svn_content_is_left_to_the_button() {
    // 偏离计划：分支跟进入口不自动（切到旧分支是用户的选择）。冲突的那次跟进只发布了基线
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);
    assert!(matches!(
        follow(&fx.wc_str(), "seed").unwrap(),
        FollowOutcome::NeedsManual { .. }
    ));
    assert!(check_sync(&fx.wc_str()).unwrap().head_behind_base);

    assert_skipped_untouched(&fx, "svn_auto_follow_branch_behind");
}

#[test]
fn condition_1_an_unfinished_record_or_a_held_lock_stops_it() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let held = svn_lock::acquire(&fx.wc_str(), "someone").unwrap();
    let before = state(&fx);
    let (code, _) = skipped(auto(&fx));
    assert_eq!(code, "sync_in_progress");
    assert_eq!(state(&fx), before);
    drop(held);

    done(auto(&fx));
}

#[test]
fn condition_2_git_busy_stops_it() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    std::fs::write(fx.wc().join(".git/index.lock"), b"").unwrap();
    let before = state(&fx);

    let (code, _) = skipped(auto(&fx));

    assert_eq!(code, "svn_index_busy");
    assert_eq!(state(&fx), before);
    assert!(fx.wc().join(".git/index.lock").exists(), "别人的锁不碰");
}

#[test]
fn condition_3_detached_head_stops_it() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    git(&fx, &["checkout", "-q", "--detach"]);
    assert_skipped_untouched(&fx, "svn_follow_detached_head");
}

#[test]
fn condition_4_an_svn_conflict_stops_it() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a-colleague\n");
    write(&fx, A, "a-mine\n");
    svn_update(&fx, A);
    assert!(read(&fx, A).contains("<<<<<<<"), "前提：SVN 冲突");
    let before = state(&fx);

    let (code, _) = skipped(auto(&fx));

    assert!(code.starts_with("svn_"), "{code}");
    assert_eq!(state(&fx), before);
    assert_no_residue(&fx);
}

#[test]
fn condition_6_an_unverified_baseline_is_not_published() {
    // 连不上服务器：录制照样成（NotChecked），但没核对过的基线不自动发布
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let repo = fx.repo();
    let away = fx.path().join("repo-offline");
    std::fs::rename(&repo, &away).unwrap();
    let message = {
        let r = catch_unwind(AssertUnwindSafe(|| {
            assert_skipped_untouched(&fx, "svn_auto_follow_not_verified")
        }));
        std::fs::rename(&away, &repo).unwrap();
        r.unwrap()
    };
    assert!(message.contains("「跟进」"), "{message}");

    // 按钮仍然可以完成（这次连得上）
    manual_done(follow(&fx.wc_str(), "manual").unwrap());
}

#[test]
fn condition_7_a_replay_conflict_does_not_even_publish_the_baseline() {
    // 点「跟进」时冲突会先发布基线（NeedsManual）。自动跟进连基线都不动，留给按钮
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_conflict");

    assert!(message.contains(A), "{message}");
    assert!(matches!(
        follow(&fx.wc_str(), "manual").unwrap(),
        FollowOutcome::NeedsManual { .. }
    ));
}

#[test]
fn condition_7_a_stale_disk_is_left_to_manual_follow() {
    // 同 svn_follow.rs 的 a_stale_disk_on_a_lagging_branch_also_stops_a_recording_follow
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());
    git(&fx, &["checkout", "-q", "-b", "old", &old_base]);
    fx.colleague_commits(B, "b2\n");
    svn_update(&fx, B);

    assert_skipped_untouched(&fx, "svn_auto_follow_stale_disk");

    // 按钮走原来的路：发布基线、转手动跟进，手动跟进把新内容写回磁盘
    assert!(matches!(
        follow(&fx.wc_str(), "manual").unwrap(),
        FollowOutcome::NeedsManual {
            stale_disk: true,
            ..
        }
    ));
    assert!(matches!(
        manual_follow(&fx.wc_str(), "manual2").unwrap(),
        SyncInOutcome::Done { .. }
    ));
    assert_eq!((read(&fx, A), read(&fx, B)), ("a2\n".into(), "b2\n".into()));
}

#[test]
fn condition_8_a_local_only_leak_is_not_followed_silently() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-log\n");
    commit_all(&fx, L_MSG);
    tortoise_commit(&fx, A);

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_leak");

    assert!(message.contains(A), "{message}");
    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_eq!(rep.leaks.len(), 1, "按钮照常完成，报告里列出泄漏");
}

#[test]
fn condition_11_a_branch_commit_on_a_file_svn_also_changed_is_not_followed() {
    // 我在分支上改了 B 第 1 行，同事改了 B 第 8 行、我局部 update 了 B：重放干净，但碰到同一个文件
    let fx = hybrid_with_colleague();
    set_me(&fx, "me");
    let lines = "1\n2\n3\n4\n5\n6\n7\n8\n";
    write(&fx, B, lines);
    tortoise_commit(&fx, B);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());

    fx.colleague_commits(B, "1\n2\n3\n4\n5\n6\n7\ncolleague\n");
    let r = youngest(&fx);
    set_author(&fx, r, "colleague");
    write(&fx, B, "mine\n2\n3\n4\n5\n6\n7\n8\n");
    commit_all(&fx, "我的改动");
    svn_update(&fx, B);
    assert_eq!(read(&fx, B), "mine\n2\n3\n4\n5\n6\n7\ncolleague\n");

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_overlap");

    assert!(message.contains(B), "{message}");
    manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_eq!(
        show(&fx, "HEAD:Code/B.cs"),
        "mine\n2\n3\n4\n5\n6\n7\ncolleague"
    );
}

/// 我暂存了 A 第 1 行（`mine`），同事改了 A 第 8 行（`colleague_line`），我局部 update 了 A。
fn staged_against_colleague(fx: &SvnFixture, mine: &str, colleague_line: &str) {
    i_commit(fx);
    write(fx, A, "1\n2\n3\n4\n5\n6\n7\n8\n");
    tortoise_commit(fx, A);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());

    fx.colleague_commits(A, &format!("{colleague_line}2\n3\n4\n5\n6\n7\ncolleague\n"));
    let r = youngest(fx);
    set_author(fx, r, "colleague");
    write(fx, A, &format!("{mine}2\n3\n4\n5\n6\n7\n8\n"));
    git(fx, &["add", A]);
    write(fx, A, "1\n2\n3\n4\n5\n6\n7\n8\n");
    svn_update(fx, A);
}

#[test]
fn condition_12_a_clean_three_way_merge_of_the_stage_is_followed_and_reported() {
    // 分析 §6.12 条件 12 只拦合并冲突；合得干净的写进通知（同 svn_follow.rs 的
    // colleague_change_merges_with_staged_content）
    let fx = hybrid_with_colleague();
    staged_against_colleague(&fx, "mine\n", "1\n");

    let rep = done(auto(&fx));

    assert!(matches!(
        rep.stage.decisions.get(A),
        Some(app_lib::engine::svn_follow_index::StageDecision::Merged(_))
    ));
    assert_eq!(show(&fx, ":Code/A.cs"), "mine\n2\n3\n4\n5\n6\n7\ncolleague");
    assert!(rep.stage.lost.is_empty());
}

#[test]
fn condition_12_a_stage_that_cannot_be_kept_is_left_to_the_button() {
    // 同事也改了第 1 行：三方合并冲突，暂存内容保不住
    let fx = hybrid_with_colleague();
    staged_against_colleague(&fx, "mine\n", "theirs\n");

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_stage");

    assert!(message.contains(A), "{message}");
    // assert_skipped_untouched 已比过全部 ref；这里点名：拼过索引也不留这次钉住的旧索引
    assert!(
        git(&fx, &["for-each-ref", "refs/mygitclient/follow/auto1"]).is_empty(),
        "没发布就不留钉住的索引"
    );
    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_eq!(
        rep.stage.lost,
        vec![A.to_string()],
        "按钮照原规则处理，报告里写明"
    );
}

#[test]
fn condition_12_an_uncertain_stage_is_left_to_the_button() {
    // 同 svn_follow.rs 的 stage_after_the_mark_is_kept_and_reported_uncertain
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, "a-final\n");
    tortoise_commit(&fx, A);
    write(&fx, A, "a-after\n");
    git(&fx, &["add", A]);

    assert_skipped_untouched(&fx, "svn_auto_follow_stage");

    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
}

// —— 崩溃恢复：自动模式先拼索引、后发布基线 ——

/// 同 svn_follow.rs 的 crash_scene：work 上有 commit、暂存里有存档、另一条分支有 commit。
fn crash_scene() -> (SvnFixture, String) {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, "Code/F.cs", "f\n");
    commit_all(&fx, "feature");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, B, "b-disk\n");
    write(&fx, A, "a2\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);
    let old_head = oid(&fx, "HEAD");
    (fx, old_head)
}

fn crash_auto_at(fx: &SvnFixture, p: Point) {
    let armed = fault::crash_at(p);
    let wc = fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| follow_auto(&wc, "auto1")));
    drop(armed);
    match r {
        Err(payload) => assert!(payload.is::<Crashed>(), "{p:?} 处是别的 panic"),
        Ok(o) => panic!("没有走到崩溃点 {p:?}：{:?}", o.map_err(|e| e.message)),
    }
}

#[test]
fn a_crash_at_any_step_of_an_automatic_follow_is_recoverable() {
    for at in [
        Point::Journaled,
        Point::BeforePrepare,
        Point::IndexLocked,
        Point::IndexPrepared,
        Point::BaseMoved,
        Point::HeadChecked,
        Point::BranchMoved,
        Point::BeforeSwap,
        Point::IndexSwapped,
        Point::OtherMoved,
        Point::BeforeFinish,
    ] {
        let (fx, old_head) = crash_scene();
        let old_base = oid(&fx, "svn/base");
        crash_auto_at(&fx, at);
        let j = svn_journal::read(&fx.wc_str())
            .unwrap()
            .unwrap_or_else(|| panic!("{at:?}：崩溃之后 journal 要在"));
        let step = j.follow_steps.as_ref().unwrap().step;
        // 基线没动之前，恢复绝不能往前移分支（record_prepared 的步骤）
        if oid(&fx, "svn/base") == old_base {
            assert_eq!(step, FollowStep::PrepareIndex, "{at:?}");
            assert_eq!(oid(&fx, "work"), old_head, "{at:?}");
        }
        // 不能留下「分支已移、索引是旧的、index.lock 不在」
        if oid(&fx, "work") != old_head && !fx.wc().join(".git/index.lock").exists() {
            assert_eq!(staged_paths(&fx), vec![B], "{at:?}");
        }

        let out = resume_follow(&fx.wc_str())
            .unwrap_or_else(|e| panic!("{at:?}：{}: {}", e.code, e.message));

        assert!(matches!(out, FollowOutcome::Done { .. }), "{at:?}：{out:?}");
        let base = oid(&fx, "svn/base");
        assert_no_residue(&fx);
        assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
        assert_eq!(oid(&fx, "HEAD^"), base, "{at:?}");
        assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a2", "{at:?}");
        assert_eq!(show(&fx, ":Code/B.cs"), "b-staged", "{at:?}");
        assert_eq!(read(&fx, B), "b-disk\n", "{at:?}");
        assert_eq!(oid(&fx, "feature^"), base, "{at:?}");
    }
}

#[test]
fn the_journal_stays_at_prepare_index_until_the_baseline_is_published() {
    let cases = [
        (Point::Journaled, FollowStep::PrepareIndex),
        (Point::IndexLocked, FollowStep::PrepareIndex),
        (Point::IndexPrepared, FollowStep::PrepareIndex),
        (Point::BaseMoved, FollowStep::PrepareIndex),
        (Point::HeadChecked, FollowStep::MoveBranch),
        (Point::BeforeSwap, FollowStep::SwapIndex),
    ];
    for (at, want) in cases {
        let (fx, _) = crash_scene();
        crash_auto_at(&fx, at);
        let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
        assert_eq!(j.follow_steps.unwrap().step, want, "{at:?}");
    }
}
