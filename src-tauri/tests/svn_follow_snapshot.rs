#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 重放时快照内容按基线走（事故分析 6.10；方案 `2026-11-06-svn-snapshot-replay-design.md`）。
//!
//! 2026-11-05 的现场：分支停在旧基线上，上面有一个「svn up 之后整包提交」的快照提交，里面大多是
//! 别人的 SVN 改动。之后一次跟进只发布了基线、分支没挪；SVN 又改了同一片代码，再跟进时快照里的
//! 旧 SVN 版本和新 SVN 版本撞上，冲突 37 个，另有两处静默合错。
//!
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_follow::fault::{self, Crashed, Point};
use app_lib::engine::svn_follow::{follow, manual_follow, FollowOutcome, FollowReport};
use app_lib::engine::svn_journal;
use app_lib::engine::svn_replay::{commits_between, ReplayCodes};
use app_lib::engine::svn_snapshot::SnapshotRule;
use app_lib::engine::svn_sync_in::{abort_sync, continue_sync, SyncInOutcome};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";
const D: &str = "Code/D.cs";

const CODES: ReplayCodes = ReplayCodes {
    conflict: "t_conflict",
    too_old: "t_too_old",
    too_old_message: "",
    failed: "t_failed",
};

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
fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}
/// `file://` 下作者都是系统用户，不改的话跟进按 F2 ① 把它当成「我自己提交的」。
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
fn colleague(fx: &SvnFixture, rel: &str, content: &str) {
    fx.colleague_commits(rel, content);
    set_author(fx, youngest(fx), "colleague");
}
fn colleague_removes(fx: &SvnFixture, rel: &str) {
    let wc2 = fx.wc2();
    svn_cmd(&wc2, &["update", "--non-interactive"]);
    svn_cmd(&wc2, &["rm", "--", rel]);
    svn_cmd(&wc2, &["commit", "-m", "rm", "--non-interactive"]);
    set_author(fx, youngest(fx), "colleague");
}
fn svn_update(fx: &SvnFixture) {
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code"]);
}
fn run(fx: &SvnFixture) -> FollowOutcome {
    follow(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
fn done(o: FollowOutcome) -> FollowReport {
    match o {
        FollowOutcome::Done { report } => *report,
        other => panic!("应当 Done：{other:?}"),
    }
}
fn show(fx: &SvnFixture, spec: &str) -> String {
    git(fx, &["show", spec])
}
fn tracked(fx: &SvnFixture, rev: &str) -> Vec<String> {
    git(fx, &["ls-tree", "-r", "--name-only", rev, "--", "Code"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// 搭出 11-05 的形状：
///
/// 1. 同事改了 `rel`（`v1`），我 update，点「跟进」：基线 1 里是 `v1`，分支跟上了。
/// 2. 分支回到最初的基线 0 上，再把磁盘（= SVN 工作副本）整包提交进 git，`extra` 里是我自己的改动。
///    分支因此停在基线 0，快照提交里 `rel` 是基线 1 的版本。
///
/// 返回 (基线 0, 快照提交)。
fn lagging_snapshot(
    fx: &SvnFixture,
    rel: &str,
    v1: Option<&str>,
    extra: &[(&str, &str)],
) -> (String, String) {
    let base0 = oid(fx, "svn/base");
    match v1 {
        Some(v) => colleague(fx, rel, v),
        None => colleague_removes(fx, rel),
    }
    svn_update(fx);
    done(run(fx));
    assert_ne!(oid(fx, "svn/base"), base0, "前提：基线 1 已发布");
    // 回到基线 0：checkout 会把旧内容写回磁盘，再把 SVN 的内容写回去，磁盘才是工作副本的样子
    git(fx, &["checkout", "-q", "-B", "work", &base0]);
    match v1 {
        Some(v) => write(fx, rel, v),
        None => {
            let _ = std::fs::remove_file(fx.wc().join(rel));
        }
    }
    for (p, c) in extra {
        write(fx, p, c);
    }
    let snap = commit_all(fx, "svn up 之后整包提交");
    (base0, snap)
}

#[test]
fn snapshot_content_of_a_newer_baseline_yields_to_the_new_baseline() {
    // 快照里 B 是基线 1 的版本，SVN 之后又改了同一行。原来这里冲突
    let fx = hybrid_with_colleague();
    let (_, snap) = lagging_snapshot(&fx, B, Some("b2\n"), &[(A, "a-mine\n")]);
    colleague(&fx, B, "b3\n");
    svn_update(&fx);
    assert_eq!(read(&fx, B), "b3\n");

    let rep = done(run(&fx));

    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "b3", "B 取新基线的版本");
    assert_eq!(
        show(&fx, "HEAD:Code/A.cs"),
        "a-mine",
        "我自己的改动照常保留"
    );
    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"));
    assert_eq!(
        rep.snapshot,
        vec![(snap, vec![B.to_string()])],
        "报告单独列出"
    );
    assert!(rep.drift.is_empty(), "{:?}", rep.drift);
    assert_eq!(git(&fx, &["diff", "--name-only"]), "", "磁盘与 HEAD 一致");
}

#[test]
fn snapshot_does_not_bring_back_a_file_svn_deleted() {
    // 11-05 的 sim_lowmem.lua：快照里有它，SVN 之后把它删了（改名）。原来重放干净地把它加回分支
    let fx = hybrid_with_colleague();
    let (_, snap) = lagging_snapshot(&fx, D, Some("d1\n"), &[(A, "a-mine\n")]);
    colleague_removes(&fx, D);
    svn_update(&fx);
    assert!(!fx.wc().join(D).exists(), "前提：SVN 已经把它从磁盘上删掉");

    let rep = done(run(&fx));

    assert!(
        !tracked(&fx, "HEAD").contains(&D.to_string()),
        "{:?}",
        tracked(&fx, "HEAD")
    );
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a-mine");
    assert_eq!(rep.snapshot, vec![(snap, vec![D.to_string()])]);
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code"]), "");
}

#[test]
fn only_the_real_conflict_is_left_and_manual_follow_sees_only_it() {
    // 37 个冲突里 35 个是快照内容：它们不该出现，真冲突（A，我和同事都改了）照旧交给人
    let fx = hybrid_with_colleague();
    let (_, snap) = lagging_snapshot(&fx, B, Some("b2\n"), &[(A, "a-mine\n")]);
    colleague(&fx, B, "b3\n");
    colleague(&fx, A, "a-colleague\n");
    // 我的 A 没提交到 SVN：update 会让 SVN 也冲突，这里先还原，模拟「已经交给 TortoiseSVN 合好了」
    svn_cmd(&fx.wc(), &["revert", "--", A]);
    svn_update(&fx);

    match run(&fx) {
        FollowOutcome::NeedsManual {
            files, snapshot, ..
        } => {
            assert_eq!(files, vec![A.to_string()]);
            // review 第 2 条：动手之前告诉用户，手动跟进会直接采用基线的是哪些
            assert_eq!(snapshot, vec![(snap, vec![B.to_string()])]);
        }
        other => panic!("应当交给手动跟进：{other:?}"),
    }

    match manual_follow(&fx.wc_str(), "mf1").unwrap() {
        SyncInOutcome::RebaseConflict { files } => assert_eq!(files, vec![A.to_string()]),
        other => panic!("手动跟进的 rebase 也只该停在 A：{other:?}"),
    }
    write(&fx, A, "a-colleague\n");
    git(&fx, &["add", A]);
    let o = continue_sync(&fx.wc_str()).unwrap();
    assert!(matches!(o, SyncInOutcome::Done { .. }), "{o:?}");

    assert_eq!(read(&fx, B), "b3\n", "磁盘上是 SVN 的最新版本");
    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "b3");
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code"]), "");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
}

#[test]
fn aborting_a_manual_follow_with_a_prepared_todo_returns_to_the_original_branch() {
    let fx = hybrid_with_colleague();
    let (_, snap) = lagging_snapshot(&fx, B, Some("b2\n"), &[(A, "a-mine\n")]);
    colleague(&fx, B, "b3\n");
    colleague(&fx, A, "a-colleague\n");
    svn_cmd(&fx.wc(), &["revert", "--", A]);
    svn_update(&fx);
    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));
    match manual_follow(&fx.wc_str(), "mf1").unwrap() {
        SyncInOutcome::RebaseConflict { .. } => {}
        other => panic!("应当停在 rebase 冲突：{other:?}"),
    }
    let j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(
        j.rebase_todo.len(),
        1,
        "待重放列表落盘了：{:?}",
        j.rebase_todo
    );
    assert_ne!(j.rebase_todo[0], snap, "pick 的是换过路径的 C'");

    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);

    assert_eq!(oid(&fx, "HEAD"), snap, "回到原位");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(read(&fx, B), "b3\n", "磁盘放回跟进前的样子");
    assert_eq!(read(&fx, A), "a-colleague\n");
    assert!(!fx.wc().join(".git/rebase-merge").exists());
    assert_eq!(git(&fx, &["stash", "list"]), "");
}

#[test]
fn rule_ignores_local_only_commits_and_reverts_to_the_branch_baseline() {
    let fx = hybrid_with_colleague();
    let (base0, _) = lagging_snapshot(&fx, B, Some("b2\n"), &[]);
    // 退回到分支自己基线的版本：不算 SVN 内容（用户故意停在旧版本）
    write(&fx, B, "b1\n");
    commit_all(&fx, "退回");
    // Local-Only 补丁恰好等于新基线：那是泄漏，不能静默收掉
    write(&fx, B, "b2\n");
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "本地补丁\n\nLocal-Only: true"]);
    let new = oid(&fx, "svn/base");
    let head = oid(&fx, "HEAD");

    let repo = fx.wc_str();
    let rule = SnapshotRule::new(&repo, &new);
    let commits = commits_between(&repo, &base0, &head, &CODES).unwrap();
    let hits: Vec<Vec<String>> = commits
        .iter()
        .map(|c| rule.paths(&base0, c).into_iter().collect())
        .collect();

    assert_eq!(
        hits,
        vec![vec![B.to_string()], vec![], vec![]],
        "只有快照提交命中"
    );
    // 以基线 1 为分支基线时，快照里的 b2 就是自己的基线，不算「更新的」
    assert!(rule.paths(&new, &commits[0]).is_empty());
}

#[test]
fn rule_skips_a_commit_whose_parent_holds_my_own_content() {
    // review 第 1 条：C0 是我自己的改动（没交 SVN）；后来在 SVN 侧撤掉、update 后整包提交 C。
    // C 的内容是基线 1 的，但把 B 写回 C^ 会把撤掉的 b-mine 复活，所以不能命中
    let fx = hybrid_with_colleague();
    let base0 = oid(&fx, "svn/base");
    colleague(&fx, B, "b2\n");
    svn_update(&fx);
    done(run(&fx));
    git(&fx, &["checkout", "-q", "-B", "work", &base0]);
    write(&fx, B, "b-mine\n");
    commit_all(&fx, "我自己的改动");
    write(&fx, B, "b2\n");
    commit_all(&fx, "svn up 之后整包提交");
    let new = oid(&fx, "svn/base");
    let head = oid(&fx, "HEAD");

    let repo = fx.wc_str();
    let rule = SnapshotRule::new(&repo, &new);
    let commits = commits_between(&repo, &base0, &head, &CODES).unwrap();

    assert!(
        rule.paths(&base0, &commits[0]).is_empty(),
        "b-mine 不是 SVN 的"
    );
    assert!(
        rule.paths(&base0, &commits[1]).is_empty(),
        "C^ 里是我的内容，写回它会复活 b-mine"
    );
}

#[test]
fn snapshot_equal_to_the_new_baseline_is_not_reported() {
    // review 第 7 条：快照里的内容就是 NEW 的，换不换都一样，报出来只是噪音
    let fx = hybrid_with_colleague();
    lagging_snapshot(&fx, B, Some("b2\n"), &[]);
    colleague(&fx, A, "a2\n");
    svn_update(&fx);

    let rep = done(run(&fx));

    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "b2");
    assert!(rep.snapshot.is_empty(), "{:?}", rep.snapshot);
    assert!(rep.rewritten.is_empty(), "{:?}", rep.rewritten);
}

#[test]
fn manual_follow_of_a_pure_snapshot_leaves_no_empty_commit() {
    // review 第 3 条：快照里全是 SVN 内容。rebase 会留下「一开始就为空」的 commit，所以根本不 pick
    let fx = hybrid_with_colleague();
    let (_, snap) = lagging_snapshot(&fx, B, Some("b2\n"), &[]);
    // 磁盘就是基线 1：判定 InSync，分支落后基线，可以直接手动跟进
    let o = manual_follow(&fx.wc_str(), "mf1").unwrap();
    assert!(matches!(o, SyncInOutcome::Done { .. }), "{o:?}");

    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"), "没有留下空 commit");
    assert_ne!(oid(&fx, "HEAD"), snap);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(read(&fx, B), "b2\n");
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code"]), "");
}

#[test]
fn manual_follow_resumes_with_the_prepared_todo_after_a_crash() {
    let fx = hybrid_with_colleague();
    lagging_snapshot(&fx, B, Some("b2\n"), &[(A, "a-mine\n")]);
    colleague(&fx, B, "b3\n");
    colleague(&fx, A, "a-colleague\n");
    svn_cmd(&fx.wc(), &["revert", "--", A]);
    svn_update(&fx);
    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));

    // 收起改动之后、rebase 之前进程没了
    let armed = fault::crash_at(Point::ManualStashed);
    let wc = fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| manual_follow(&wc, "mf1")));
    drop(armed);
    match r {
        Err(p) => assert!(p.is::<Crashed>(), "是别的 panic"),
        Ok(o) => panic!("没有走到崩溃点：{:?}", o.map_err(|e| e.message)),
    }
    assert!(!svn_journal::read(&wc)
        .unwrap()
        .unwrap()
        .rebase_todo
        .is_empty());

    match continue_sync(&wc).unwrap() {
        SyncInOutcome::RebaseConflict { files } => assert_eq!(files, vec![A.to_string()]),
        other => panic!("继续时照样用待重放列表，只停在 A：{other:?}"),
    }
}

#[test]
fn a_clean_but_wrong_replay_is_reported_as_drift() {
    // PR 119 遗留：对象层跟进只移 HEAD，重放干净却合错（W14）时照样报完成。用「把两边拼起来、
    // 退出 0」的 merge driver 代替真实现场里 diff3 的错误对齐。
    //
    // 形状：我在 git 里改了 A，也用 TortoiseSVN 把它交了；之后同事又改了 A。没有失焦快照，F2 ①
    // 不生效；我的版本也不在任何基线里，6.10 不命中。正确的重放会冲突，合错时磁盘（= NEW）与 tip 不同
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-git\n");
    commit_all(&fx, "git 里的版本");
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", A],
    );
    colleague(&fx, A, "a-colleague\n");
    svn_update(&fx);
    assert_eq!(read(&fx, A), "a-colleague\n");
    let attrs = fx.wc().join(".git/info/attributes");
    let mut rules = std::fs::read_to_string(&attrs).unwrap_or_default();
    rules.push_str("\nCode/A.cs merge=concat\n");
    std::fs::write(&attrs, rules).unwrap();
    git(&fx, &["config", "merge.concat.driver", "cat %B >> %A"]);

    let rep = done(run(&fx));

    assert_ne!(
        show(&fx, "HEAD:Code/A.cs"),
        "a-colleague",
        "前提：merge driver 合出了别的东西"
    );
    assert_eq!(rep.drift, vec![A.to_string()]);
}
