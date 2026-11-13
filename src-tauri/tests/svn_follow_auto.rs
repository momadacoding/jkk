#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 自动跟进（分析 §6.12；计划 2026-11-06-svn-auto-follow）。
//!
//! 每条条件一个「不满足就不自动、一个 ref 都不动、写明原因」的用例，外加全部满足时自动发布且
//! 磁盘一个字节不动、与同步锁互斥、崩溃之后能恢复。现场的搭法与 `svn_follow.rs` 相同：wc1 是
//! 「你」，wc2 是同事，「TortoiseSVN 提交」就是在 wc1 里直接 `svn commit` 一个文件。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_basemap::read_base_map;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::fault::{self, Crashed, Point};
use app_lib::engine::svn_follow::{
    follow, follow_auto, manual_follow, resume_follow, AutoFollowOutcome, FollowOutcome,
    FollowReport,
};
use app_lib::engine::svn_follow_mark::write_mark;
use app_lib::engine::svn_journal::{self, Flow, FollowStep, Journal, Phase};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_server_set::{audit_server_set_at_baseline, ServerAudit};
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
    // 未收尾的记录（别的同步留下的）：记录不碰，原样留着
    let record = Journal {
        opid: "someone-else".into(),
        flow: Flow::SyncIn,
        phase: Phase::Stashed,
        started_at: 1,
        ..Default::default()
    };
    svn_journal::write(&fx.wc_str(), &record).unwrap();
    let before = state(&fx);
    let (code, _) = skipped(auto(&fx));
    assert_eq!(code, "svn_follow_unfinished");
    assert_eq!(state(&fx), before);
    assert_eq!(
        svn_journal::read(&fx.wc_str()).unwrap().map(|j| j.opid),
        Some("someone-else".to_string()),
        "别人的记录不碰"
    );
    svn_journal::remove(&fx.wc_str()).unwrap();

    // 同步锁被另一次同步拿着
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
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--accept",
            "postpone",
            "--non-interactive",
            "--",
            A,
        ],
    );
    assert!(read(&fx, A).contains("<<<<<<<"), "前提：SVN 冲突");

    assert_skipped_untouched(&fx, "svn_preflight_conflicted");
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

/// 我在分支上改了 B 第 1 行（commit「我的改动」），同事改了 B 第 8 行、我局部 update 了 B：重放干净，
/// 碰到同一个文件。另有一个只改 W 的 commit 排在前面（钉住 `git log -z` 多个 commit 首尾相接时的
/// 解析）。`eol` 是文件里的换行符。
fn overlap_scene(fx: &SvnFixture, eol: &str) {
    let text = |lines: &[&str]| {
        lines
            .iter()
            .map(|l| format!("{l}{eol}"))
            .collect::<String>()
    };
    set_me(fx, "me");
    write(fx, B, &text(&["1", "2", "3", "4", "5", "6", "7", "8"]));
    tortoise_commit(fx, B);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());

    fx.colleague_commits(B, &text(&["1", "2", "3", "4", "5", "6", "7", "colleague"]));
    let r = youngest(fx);
    set_author(fx, r, "colleague");
    write(fx, "Code/W.cs", &text(&["w"]));
    commit_all(fx, "只改 W");
    write(fx, B, &text(&["mine", "2", "3", "4", "5", "6", "7", "8"]));
    commit_all(fx, "我的改动");
    svn_update(fx, B);
    assert_eq!(
        read(fx, B),
        text(&["mine", "2", "3", "4", "5", "6", "7", "colleague"])
    );
}

fn overlap_of(rep: &FollowReport) -> Vec<(String, bool)> {
    rep.overlap
        .iter()
        .map(|o| (o.path.clone(), o.matches_disk))
        .collect()
}

#[test]
fn condition_11_an_overlap_whose_replay_equals_the_disk_is_followed() {
    // 2026-11-11 使用报告第 2 节（方案 B）：重放结果与磁盘（svn update 合出来的）逐字节相同时，
    // 「本地改动」里不会出现 W14 那种反向改动，不必拦
    let fx = hybrid_with_colleague();
    overlap_scene(&fx, "\n");

    let rep = done(auto(&fx));

    assert_eq!(
        show(&fx, "HEAD:Code/B.cs"),
        "mine\n2\n3\n4\n5\n6\n7\ncolleague"
    );
    assert!(modified_paths(&fx).is_empty(), "{:?}", modified_paths(&fx));
    assert_eq!(
        overlap_of(&rep),
        vec![(B.to_string(), true)],
        "报告逐个标出"
    );
    let commits = &rep.overlap[0].commits;
    assert_eq!(commits.len(), 1, "只改 W 的 commit 不算：{commits:?}");
    assert!(
        commits[0].ends_with(" 我的改动"),
        "短 hash + 提交说明：{commits:?}"
    );
}

#[test]
fn condition_11_an_overlap_with_uncommitted_edits_is_not_followed() {
    // 重叠文件里还有没提交的改动：磁盘本来就和 tip 不同，分不清是用户的改动还是合错，交给人看
    let fx = hybrid_with_colleague();
    overlap_scene(&fx, "\n");
    write(&fx, B, "mine\n2\n3\n4\n5\n6\n7\ncolleague\nuncommitted\n");

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_overlap");

    assert!(message.contains(B), "{message}");
    assert!(message.contains("我的改动"), "写出是哪个提交：{message}");
    assert!(
        !message.contains("W.cs"),
        "只改 W 的 commit 不算：{message}"
    );
    assert!(
        !message.contains("对照报告检查"),
        "不再要用户做报告支持不了的核对：{message}"
    );
    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_eq!(
        show(&fx, "HEAD:Code/B.cs"),
        "mine\n2\n3\n4\n5\n6\n7\ncolleague"
    );
    assert_eq!(overlap_of(&rep), vec![(B.to_string(), false)]);
}

#[test]
fn condition_11_a_clean_but_wrong_replay_is_not_followed() {
    // W14：重放没冲突，结果却与磁盘不同。用「把两边拼起来、退出 0」的 merge driver 代替真实现场里
    // diff3 的错误对齐（同 svn_follow_snapshot.rs 的 a_clean_but_wrong_replay_is_reported_as_drift）
    let fx = hybrid_with_colleague();
    overlap_scene(&fx, "\n");
    let attrs = fx.wc().join(".git/info/attributes");
    let mut rules = std::fs::read_to_string(&attrs).unwrap_or_default();
    rules.push_str("\nCode/B.cs merge=concat\n");
    std::fs::write(&attrs, rules).unwrap();
    git(&fx, &["config", "merge.concat.driver", "cat %B >> %A"]);

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_overlap");

    assert!(message.contains(B), "{message}");
    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_ne!(
        show(&fx, "HEAD:Code/B.cs"),
        "mine\n2\n3\n4\n5\n6\n7\ncolleague",
        "前提：merge driver 合出了别的东西"
    );
    assert_eq!(overlap_of(&rep), vec![(B.to_string(), false)]);
}

#[test]
fn condition_11_an_overlap_equal_after_eol_conversion_is_followed() {
    // 磁盘是 CRLF、blob 是 LF：磁盘内容按 `git add` 的过滤规则算（hash-object --stdin-paths）
    let fx = hybrid_with_colleague();
    git(&fx, &["config", "core.autocrlf", "true"]);
    overlap_scene(&fx, "\r\n");

    let rep = done(auto(&fx));

    assert_eq!(overlap_of(&rep), vec![(B.to_string(), true)]);
    assert!(modified_paths(&fx).is_empty(), "{:?}", modified_paths(&fx));
}

/// 我暂存了 A 第 1 行（`mine`），同事改了 A 第 8 行（`colleague_line`），我局部 update 了 A。
fn staged_against_colleague(fx: &SvnFixture, mine: &str, colleague_line: &str) {
    staged_scene(
        fx,
        &format!("{mine}2\n3\n4\n5\n6\n7\n8\n"),
        &format!("{colleague_line}2\n3\n4\n5\n6\n7\ncolleague\n"),
    );
}

/// OLD 里 A 是 `1..8`。我把 A 暂存成 `staged`（磁盘上撤回 OLD），同事把 A 提交成 `colleague`，
/// 我局部 update 了 A。
fn staged_scene(fx: &SvnFixture, staged: &str, colleague: &str) {
    i_commit(fx);
    write(fx, A, "1\n2\n3\n4\n5\n6\n7\n8\n");
    tortoise_commit(fx, A);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());

    fx.colleague_commits(A, colleague);
    let r = youngest(fx);
    set_author(fx, r, "colleague");
    write(fx, A, staged);
    git(fx, &["add", A]);
    write(fx, A, "1\n2\n3\n4\n5\n6\n7\n8\n");
    svn_update(fx, A);
}

#[test]
fn condition_12_a_stage_already_in_svn_is_followed_and_reported_superseded() {
    // 2026-11-11 使用报告第 4 节（方案 C）：暂存只改了第 1 行，同事在同一片区域改得更多（第 1、2
    // 行），三方合并冲突；但暂存的改动 SVN 新版本里已经有了，没有丢任何内容
    let fx = hybrid_with_colleague();
    staged_scene(
        &fx,
        "mine\n2\n3\n4\n5\n6\n7\n8\n",
        "mine\nchanged2\n3\n4\n5\n6\n7\ncolleague\n",
    );

    let rep = done(auto(&fx));

    assert!(rep.stage.lost.is_empty(), "{:?}", rep.stage.lost);
    assert_eq!(rep.stage.superseded, vec![A.to_string()]);
    assert_eq!(
        show(&fx, ":Code/A.cs"),
        "mine\nchanged2\n3\n4\n5\n6\n7\ncolleague"
    );
    let pinned = rep.pinned_ref.expect("原内容照旧钉住");
    assert!(!git(&fx, &["ls-tree", "-r", &pinned]).is_empty());
}

#[test]
fn condition_12_a_partly_superseded_stage_is_left_to_the_button() {
    // 暂存改了第 1、5 行；SVN 上只有第 1 行那处：两处里一处已在
    let fx = hybrid_with_colleague();
    staged_scene(
        &fx,
        "mine\n2\n3\n4\nmine5\n6\n7\n8\n",
        "mine\nchanged2\n3\n4\n5\n6\n7\ncolleague\n",
    );

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_stage");

    assert!(message.contains(A), "{message}");
    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    assert_eq!(rep.stage.lost, vec![A.to_string()]);
    let t = rep.stage.partial.get(A).expect("给出计数");
    assert_eq!((t.present, t.total), (1, 2));
    assert!(rep.stage.superseded.is_empty());
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
    assert!(
        rep.stage.partial.is_empty(),
        "一处都不在 SVN 上：{:?}",
        rep.stage.partial
    );
    assert!(rep.stage.superseded.is_empty());
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
fn a_lock_swapped_for_someone_elses_before_the_baseline_was_published_is_left_alone() {
    // review 低-3：自动跟进拼好索引、还没发布基线时崩了，残锁被手工删掉、别的 git 又拿了锁。
    // journal 里有指纹，「继续」要认出这不是我们的锁，不删
    let (fx, old_head) = crash_scene();
    crash_auto_at(&fx, Point::IndexPrepared);
    let lock = fx.wc().join(".git/index.lock");
    std::fs::write(&lock, b"someone else's lock").unwrap();

    let err = resume_follow(&fx.wc_str()).unwrap_err();

    assert_eq!(err.code, "svn_index_busy", "{}", err.message);
    assert_eq!(std::fs::read(&lock).unwrap(), b"someone else's lock");
    assert_eq!(oid(&fx, "work"), old_head);
    assert!(
        svn_journal::read(&fx.wc_str()).unwrap().is_some(),
        "记录留着"
    );

    // 那个 git 结束之后「继续」照常走完
    std::fs::remove_file(&lock).unwrap();
    assert!(matches!(
        resume_follow(&fx.wc_str()).unwrap(),
        FollowOutcome::Done { .. }
    ));
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

// —— 使用报告 2026-11-12：提交 SVN 前去掉 Local-Only 补丁（计划 2026-11-12-auto-restore-local-only） ——

const PATCH: &str = "---@class Avatar\n";

fn lines8(last: &str) -> String {
    format!("1\n2\n3\n4\n5\n6\n7\n{last}\n")
}

/// 分支上有 Local-Only 提交给 B 加了一行，跟进过一次，磁盘带着补丁。
fn patched_scene(fx: &SvnFixture) {
    i_commit(fx);
    write(fx, B, &lines8("8"));
    tortoise_commit(fx, B);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());
    write(fx, B, &format!("{PATCH}{}", lines8("8")));
    commit_all(fx, L_MSG);
}

#[test]
fn stripping_the_patch_before_an_svn_commit_is_followed_and_the_patch_comes_back() {
    // 真实现场：磁盘上去掉补丁、顺手改了别的，TortoiseSVN 提交。以前重放结果和磁盘对不上被拦
    let fx = hybrid_with_colleague();
    patched_scene(&fx);
    write(&fx, B, &lines8("8 mine"));
    tortoise_commit(&fx, B);

    let rep = done(auto(&fx));

    assert_eq!(
        overlap_of(&rep),
        vec![(B.to_string(), false)],
        "前提：放回前对不上"
    );
    let restore = rep.restore.expect("有放回结果");
    assert_eq!(restore.restored, vec![B.to_string()], "{restore:?}");
    let want = format!("{PATCH}{}", lines8("8 mine"));
    assert_eq!(read(&fx, B), want);
    assert_eq!(show(&fx, "HEAD:Code/B.cs"), want.trim_end());
    assert!(modified_paths(&fx).is_empty(), "{:?}", modified_paths(&fx));
    assert!(staged_paths(&fx).is_empty(), "{:?}", staged_paths(&fx));
}

#[test]
fn a_stripped_patch_with_further_edits_is_not_followed_or_written() {
    // 提交之后又改了这个文件：磁盘不是 SVN 的版本，不能放回，照旧拦
    let fx = hybrid_with_colleague();
    patched_scene(&fx);
    write(&fx, B, &lines8("8 mine"));
    tortoise_commit(&fx, B);
    let edited = format!("{}more\n", lines8("8 mine"));
    write(&fx, B, &edited);

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_overlap");

    assert!(message.contains(B), "{message}");
    assert_eq!(read(&fx, B), edited, "编辑原样保留");
}

#[test]
fn a_stripped_patch_last_changed_by_a_colleague_is_not_restored_automatically() {
    // 有意去掉补丁、还没提交，svn update 带来同事的改动：可能是在准备提交，不替用户放回
    let fx = hybrid_with_colleague();
    patched_scene(&fx);
    write(&fx, B, &lines8("8"));
    fx.colleague_commits(B, &lines8("colleague"));
    let r = youngest(&fx);
    set_author(&fx, r, "colleague");
    svn_update(&fx, B);
    assert_eq!(read(&fx, B), lines8("colleague"));

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_overlap");

    assert!(message.contains(B), "{message}");
    assert!(
        message.contains("Local-Only 补丁"),
        "说清是这种情况（缺陷 7）：{message}"
    );
    assert!(message.contains("不是你"), "{message}");
    assert_eq!(read(&fx, B), lines8("colleague"));

    // 点「跟进」：跟进完成，同样不写磁盘，留给横幅
    let rep = manual_done(follow(&fx.wc_str(), "manual").unwrap());
    let restore = rep.restore.expect("有放回结果");
    assert!(restore.restored.is_empty(), "{restore:?}");
    assert_eq!(read(&fx, B), lines8("colleague"));
}

// —— 部分自动跟进（计划 2026-11-13-partial-auto-follow，使用报告 2026-11-12 第 5.5 节） ——

fn held(rep: &FollowReport) -> Vec<String> {
    rep.held_back.iter().map(|h| h.path.clone()).collect()
}

/// 跟进之后再判一次：SVN 侧还有新内容的文件。
fn still_new(fx: &SvnFixture) -> Vec<String> {
    match check_sync(&fx.wc_str()).unwrap().verdict {
        SyncVerdict::ExternallyAdvanced {
            changed,
            added,
            removed,
        } => {
            let mut v: Vec<String> = changed
                .into_iter()
                .chain(added)
                .map(|c| c.path)
                .chain(removed)
                .collect();
            v.sort();
            v
        }
        SyncVerdict::InSync => Vec::new(),
        other => panic!("意外的判定：{other:?}"),
    }
}

const B_EDITED: &str = "mine\n2\n3\n4\n5\n6\n7\ncolleague\nuncommitted\n";

#[test]
fn one_file_that_does_not_match_the_disk_stays_behind_and_the_rest_is_followed() {
    // 验收：1 个文件对不上时，其余文件照常自动跟进，横幅只剩该文件
    let fx = hybrid_with_colleague();
    overlap_scene(&fx, "\n");
    write(&fx, B, B_EDITED);
    let old_b = show(&fx, "svn/base:Code/B.cs");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let before = wc_snapshot(&fx);

    let rep = done(auto(&fx));

    assert_eq!(wc_snapshot(&fx), before, "磁盘一个字节都不动");
    assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a2", "A 跟上了");
    assert_eq!(show(&fx, "svn/base:Code/B.cs"), old_b, "B 留在旧基线上");
    assert_eq!(held(&rep), vec![B.to_string()]);
    assert!(
        rep.held_back[0].reason.contains("磁盘"),
        "写明为什么留下：{:?}",
        rep.held_back
    );
    assert!(
        rep.changes.iter().all(|c| c.path != B),
        "跟进的文件里不列 B：{:?}",
        rep.changes
    );
    assert!(rep.overlap.iter().all(|o| o.path != B));
    assert_no_residue(&fx);
    assert_eq!(still_new(&fx), vec![B.to_string()], "横幅只剩 B");
    assert_eq!(read(&fx, B), B_EDITED);

    // 再来一次只剩 B：没有可跟的了，照原来整次不做
    assert_skipped_untouched(&fx, "svn_auto_follow_overlap");
}

#[test]
fn a_held_back_file_keeps_its_old_revision_after_a_root_update() {
    // 每天早上根目录 update：所有节点都到 HEAD，floor 跟着到 HEAD。留下的文件要记旧 revision，
    // floor 因此降下来，其余文件按「commit_rev > floor」单列
    let fx = hybrid_with_colleague();
    overlap_scene(&fx, "\n");
    write(&fx, B, B_EDITED);
    let old_map = read_base_map(&fx.wc_str(), "svn/base").unwrap();
    let old_b = show(&fx, "svn/base:Code/B.cs");
    fx.colleague_commits(A, "a-colleague\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    let head = youngest(&fx);

    let rep = done(auto(&fx));

    let map = read_base_map(&fx.wc_str(), "svn/base").unwrap();
    assert_eq!(held(&rep), vec![B.to_string()]);
    assert_eq!(map.rev_of(B), old_map.rev_of(B), "{map:?}");
    assert_eq!(map.rev_of(A), head, "{map:?}");
    assert!(map.floor <= old_map.rev_of(B), "{map:?}");
    assert_eq!(show(&fx, "svn/base:Code/B.cs"), old_b);
    assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a-colleague");
    assert_eq!(still_new(&fx), vec![B.to_string()]);
}

#[test]
fn a_file_deleted_in_the_same_follow_gets_a_tombstone_when_the_floor_drops() {
    // floor 降回留下文件的旧 revision 时，这次跟进删掉的 C 在新 floor 上服务器里还在。完整录制
    // 没给它墓碑（删除不晚于原 floor），服务器文件集合的审计就会把它报成漏收
    let fx = hybrid_with_colleague();
    let c = "Code/C.cs";
    fx.colleague_commits(c, "c\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    manual_done(follow(&fx.wc_str(), "seed-c").unwrap());
    overlap_scene(&fx, "\n");
    write(&fx, B, B_EDITED);
    let wc2 = fx.wc2();
    svn_cmd(&wc2, &["update", "--non-interactive"]);
    svn_cmd(&wc2, &["rm", "--non-interactive", c]);
    std::fs::write(wc2.join(A), "a-colleague\n").unwrap();
    svn_cmd(
        &wc2,
        &[
            "commit",
            "-m",
            "colleague",
            "--non-interactive",
            "--no-auth-cache",
        ],
    );
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let rep = done(auto(&fx));

    assert_eq!(held(&rep), vec![B.to_string()]);
    assert_eq!(rep.removed, vec![c.to_string()]);
    let map = read_base_map(&fx.wc_str(), "svn/base").unwrap();
    assert!(map.gone.contains_key(c), "{map:?}");
    assert_eq!(
        audit_server_set_at_baseline(&fx.wc_str()).unwrap(),
        ServerAudit::Matched
    );
}

#[test]
fn a_replay_conflict_on_one_file_leaves_only_that_file_behind() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);
    fx.colleague_commits(B, "b2\n");
    svn_update(&fx, B);

    let rep = done(auto(&fx));

    assert_eq!(held(&rep), vec![A.to_string()]);
    assert!(
        rep.held_back[0].reason.contains("冲突"),
        "{:?}",
        rep.held_back
    );
    assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a1");
    assert_eq!(show(&fx, "svn/base:Code/B.cs"), "b2");
    assert_eq!(still_new(&fx), vec![A.to_string()]);
    assert_eq!(read(&fx, A), "a3\n", "不写磁盘");
    assert_skipped_untouched(&fx, "svn_auto_follow_conflict");
}

#[test]
fn a_leaked_file_stays_behind_so_the_warning_keeps_showing() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-log\n");
    commit_all(&fx, L_MSG);
    tortoise_commit(&fx, A);
    fx.colleague_commits(B, "b2\n");
    svn_update(&fx, B);

    let rep = done(auto(&fx));

    assert_eq!(held(&rep), vec![A.to_string()]);
    assert!(
        rep.held_back[0].reason.contains("Local-Only"),
        "{:?}",
        rep.held_back
    );
    assert_eq!(show(&fx, "svn/base:Code/B.cs"), "b2");
    assert_eq!(still_new(&fx), vec![A.to_string()]);
    // 只剩 A 时照原来拦下，横幅写泄漏的原因
    let message = assert_skipped_untouched(&fx, "svn_auto_follow_leak");
    assert!(message.contains(A), "{message}");
}

#[test]
fn a_problem_on_a_file_added_in_svn_stops_the_whole_follow() {
    // 只排除外部「修改」的文件：新增的文件出问题，整次不做
    let fx = hybrid_with_colleague();
    let n = "Code/N.cs";
    write(&fx, n, "n-log\n");
    commit_all(&fx, L_MSG);
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", n]);
    tortoise_commit(&fx, n);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let message = assert_skipped_untouched(&fx, "svn_auto_follow_leak");
    assert!(message.contains(n), "{message}");
}

/// 分支上有一个改了 `rel` 的 commit，磁盘上的 `rel` 换回 SVN 版本（免得 update 时撞上本地修改），
/// 再由同事 `op` 把它删掉或改名，同时改了 A；我根目录 update。
fn removed_in_svn_scene(fx: &SvnFixture, rel: &str, op: &[&str]) {
    write(fx, rel, "branch\n");
    commit_all(fx, "改它");
    svn_cmd(&fx.wc(), &["revert", "--non-interactive", "--", rel]);
    let wc2 = fx.wc2();
    svn_cmd(&wc2, &["update", "--non-interactive"]);
    svn_cmd(&wc2, op);
    std::fs::write(wc2.join(A), "a-colleague\n").unwrap();
    svn_cmd(
        &wc2,
        &[
            "commit",
            "-m",
            "colleague",
            "--non-interactive",
            "--no-auth-cache",
        ],
    );
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
}

#[test]
fn a_problem_on_a_file_renamed_in_svn_stops_the_whole_follow() {
    // 改名 = 删 B + 加 B2：只跟一半基线就对不上了，整次不做
    let fx = hybrid_with_colleague();
    removed_in_svn_scene(&fx, B, &["mv", "--non-interactive", B, "Code/B2.cs"]);
    assert!(!fx.wc().join(B).exists(), "前提：B 已在 SVN 上改名");

    // 实测：merge-tree 认出了改名，把分支的改动带到 B2 上，拦下它的是「B2 与磁盘对不上」。
    // B2 是新增的，留不住，所以整次不做
    let message = assert_skipped_untouched(&fx, "svn_auto_follow_overlap");
    assert!(message.contains("Code/B2.cs"), "{message}");
}

#[test]
fn a_problem_in_a_directory_deleted_in_svn_stops_the_whole_follow() {
    let fx = hybrid_with_colleague();
    let s = "Code/Sub/S.cs";
    fx.colleague_commits(s, "s1\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    manual_done(follow(&fx.wc_str(), "seed").unwrap());
    removed_in_svn_scene(&fx, s, &["rm", "--non-interactive", "Code/Sub"]);
    assert!(!fx.wc().join(s).exists(), "前提：目录已在 SVN 上删除");

    assert_skipped_untouched(&fx, "svn_auto_follow_conflict");
}
