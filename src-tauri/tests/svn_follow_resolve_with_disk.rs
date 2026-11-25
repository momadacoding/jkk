#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 「用磁盘上的内容跟进」端到端（设计 `2026-11-25-follow-resolve-with-disk.md`）。
//!
//! 现场照 2026-11-25 的事故搭：同事在 wc2 改了 A 的第 2、8 行；你在 git 里提交过第 2 行的改动；
//! wc1 里 `svn update` 冲突，你在 SVN 里合好并 `svn resolve`。跟进把提交重放到新基线上时
//! 第 2 行又冲突了，而磁盘上就是答案。
//!
//! 判定的各条变体在 `tests/svn_follow_disk.rs`（纯 git）；这里钉的是跟进的编排：
//! 冲突框拿到什么、点了之后分支、索引、磁盘、其他分支各是什么样。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_follow::{
    follow, follow_with_disk, ConflictDetail, DiskVerdict, FollowOutcome, FollowReport,
};
use app_lib::engine::svn_journal;
use app_lib::engine::svn_lock;
use std::process::Command;
use std::time::SystemTime;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";

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
    std::fs::write(fx.wc().join(rel), s).unwrap();
}
fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn mtime(fx: &SvnFixture, rel: &str) -> SystemTime {
    std::fs::metadata(fx.wc().join(rel))
        .unwrap()
        .modified()
        .unwrap()
}
fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}
/// 同事的 revision 改成别人的名字：file:// 下作者都是系统用户，不改就会被当成「我提交的」。
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
    assert!(out.status.success());
}
fn colleague(fx: &SvnFixture, body: &str) {
    fx.colleague_commits(A, body);
    let r = youngest(fx);
    set_author(fx, r, "colleague");
}
fn svn_update(fx: &SvnFixture) {
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", A]);
}

/// 10 行（第 n 行是 `n`），`repl` 替换某些行。
fn lines(repl: &[(usize, &str)]) -> String {
    (1..=10)
        .map(|n| match repl.iter().find(|(k, _)| *k == n) {
            Some((_, v)) => format!("{v}\n"),
            None => format!("{n}\n"),
        })
        .collect()
}
fn merged() -> String {
    lines(&[(2, "two-mine-and-svn"), (8, "eight-svn")])
}

/// 搭到「SVN 里冲突了、还没解决」为止。返回你的提交。
fn conflicted() -> (SvnFixture, String) {
    let fx = hybrid_with_colleague();
    colleague(&fx, &lines(&[]));
    svn_update(&fx);
    assert!(
        matches!(run(&fx), FollowOutcome::Done { .. }),
        "先把 10 行版本跟进来"
    );
    write(&fx, A, &lines(&[(2, "two-mine")]));
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "mine"]);
    let c = oid(&fx, "HEAD");
    colleague(&fx, &lines(&[(2, "two-svn"), (8, "eight-svn")]));
    svn_update(&fx);
    assert!(
        read(&fx, A).contains("<<<<<<<"),
        "SVN 报了冲突：{}",
        read(&fx, A)
    );
    (fx, c)
}

/// 在 SVN 里解决：磁盘写成 `body`，标记 Resolved。
fn resolve_in_svn(fx: &SvnFixture, body: Option<&str>) {
    if let Some(b) = body {
        write(fx, A, b);
    }
    svn_cmd(
        &fx.wc(),
        &[
            "resolve",
            "--accept",
            "working",
            "--non-interactive",
            "--",
            A,
        ],
    );
}

fn run(fx: &SvnFixture) -> FollowOutcome {
    follow(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
/// 冲突框里点按钮：`base` 取此刻的 `svn/base`，就是冲突框给人对比的那个新基线。
fn with_disk(fx: &SvnFixture, files: &[&str]) -> FollowOutcome {
    with_disk_on(fx, files, &oid(fx, "svn/base"))
}
fn with_disk_on(fx: &SvnFixture, files: &[&str], base: &str) -> FollowOutcome {
    let files: Vec<String> = files.iter().map(|s| s.to_string()).collect();
    follow_with_disk(&fx.wc_str(), "op2", &files, base)
        .unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
fn done(o: FollowOutcome) -> FollowReport {
    match o {
        FollowOutcome::Done { report } => *report,
        other => panic!("应当 Done：{other:?}"),
    }
}

/// (disk_ready, details, base)
fn manual(o: FollowOutcome) -> (bool, Vec<ConflictDetail>, String) {
    match o {
        FollowOutcome::NeedsManual {
            disk_ready,
            details,
            base,
            stale_disk,
            ..
        } => {
            assert!(!stale_disk);
            (disk_ready, details, base)
        }
        other => panic!("应当 NeedsManual：{other:?}"),
    }
}
fn reason(d: &ConflictDetail) -> String {
    match &d.disk {
        Some(DiskVerdict::Refused { reason }) => reason.clone(),
        other => panic!("应当不能用：{other:?}"),
    }
}
fn assert_no_residue(fx: &SvnFixture) {
    assert!(!fx.wc().join(".git/index.lock").exists());
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

#[test]
fn resolved_in_svn_follows_with_disk_content_without_writing_the_worktree() {
    let (fx, c) = conflicted();
    resolve_in_svn(&fx, Some(&merged()));

    let (ready, details, base) = manual(run(&fx));
    assert!(ready, "{details:?}");
    assert_eq!(details.len(), 1);
    assert_eq!(details[0].path, A);
    assert_eq!(details[0].disk, Some(DiskVerdict::Usable));
    assert_eq!(base, oid(&fx, "svn/base"), "对比的左边是新基线");
    assert_eq!(oid(&fx, "HEAD"), c, "只探测，分支不动");

    let before = mtime(&fx, A);
    let rep = done(with_disk(&fx, &[A]));

    assert_eq!(rep.disk_resolved, vec![(c.clone(), vec![A.to_string()])]);
    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"), "提交挂在新基线上");
    assert_eq!(git(&fx, &["log", "-1", "--format=%s"]), "mine");
    assert_eq!(
        git(&fx, &["show", &format!("HEAD:{A}")]),
        merged().trim_end()
    );
    assert_eq!(read(&fx, A), merged(), "磁盘没被改写");
    assert_eq!(mtime(&fx, A), before, "连修改时间都没动");
    assert_eq!(
        git(&fx, &["status", "--porcelain", "--", "Code"]),
        "",
        "本地改动里没有它了"
    );
    assert_no_residue(&fx);
}

#[test]
fn markers_kept_through_resolve_leave_only_manual_follow() {
    // TortoiseSVN 允许带着冲突标记点 Resolved
    let (fx, c) = conflicted();
    resolve_in_svn(&fx, None);

    let (ready, details, _) = manual(run(&fx));
    assert!(!ready);
    assert!(reason(&details[0]).contains("冲突标记"));

    // 硬点也不行：回到冲突框，什么都不动
    let (ready, _, _) = manual(with_disk(&fx, &[A]));
    assert!(!ready);
    assert_eq!(oid(&fx, "HEAD"), c);
    assert_no_residue(&fx);
}

#[test]
fn disk_edited_after_the_dialog_is_checked_again() {
    let (fx, c) = conflicted();
    resolve_in_svn(&fx, Some(&merged()));
    assert!(manual(run(&fx)).0);

    // 弹框之后又在冲突处以外改了一行
    write(
        &fx,
        A,
        &lines(&[(2, "two-mine-and-svn"), (5, "five-later"), (8, "eight-svn")]),
    );
    let (ready, details, _) = manual(with_disk(&fx, &[A]));
    assert!(!ready);
    assert!(reason(&details[0]).contains("冲突处以外"), "{details:?}");
    assert_eq!(oid(&fx, "HEAD"), c, "什么都没动");
    assert_no_residue(&fx);
}

#[test]
fn disk_edited_inside_the_conflict_after_the_dialog_still_counts() {
    // 设计 §4 第 3 步：在对比工具里改了也算数
    let (fx, _) = conflicted();
    resolve_in_svn(&fx, Some(&merged()));
    assert!(manual(run(&fx)).0);
    let edited = lines(&[(2, "two-edited-in-the-diff-tool"), (8, "eight-svn")]);
    write(&fx, A, &edited);
    done(with_disk(&fx, &[A]));
    assert_eq!(git(&fx, &["show", &format!("HEAD:{A}")]), edited.trim_end());
}

#[test]
fn a_baseline_other_than_the_one_compared_against_is_not_used() {
    // 独立 review 低 5：冲突框之后 SVN 又前进了，用户对比时看的左边是旧基线
    let (fx, c) = conflicted();
    resolve_in_svn(&fx, Some(&merged()));
    let (ready, _, _) = manual(with_disk_on(&fx, &[A], &"0".repeat(40)));
    assert!(ready, "照样能用，只是基线不是冲突框里那个");
    assert_eq!(oid(&fx, "HEAD"), c);
    assert_no_residue(&fx);
}

// —— 一次跟进里两个提交先后冲突（独立 review 中 3）——

const B: &str = "Code/B.cs";

fn colleague_on(fx: &SvnFixture, rel: &str, body: &str) {
    fx.colleague_commits(rel, body);
    let r = youngest(fx);
    set_author(fx, r, "colleague");
}

/// X 改 A 第 2 行，Y 改 B 第 3 行；同事两处都改了同一行；wc1 update 两个文件都冲突。返回 (X, Y)。
fn two_conflicts() -> (SvnFixture, String, String) {
    let fx = hybrid_with_colleague();
    colleague_on(&fx, A, &lines(&[]));
    colleague_on(&fx, B, &lines(&[]));
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code"]);
    assert!(matches!(run(&fx), FollowOutcome::Done { .. }));
    write(&fx, A, &lines(&[(2, "two-mine")]));
    git(&fx, &["commit", "-qam", "X"]);
    let x = oid(&fx, "HEAD");
    write(&fx, B, &lines(&[(3, "three-mine")]));
    git(&fx, &["commit", "-qam", "Y"]);
    let y = oid(&fx, "HEAD");
    colleague_on(&fx, A, &lines(&[(2, "two-svn")]));
    colleague_on(&fx, B, &lines(&[(3, "three-svn")]));
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code"]);
    assert!(read(&fx, A).contains("<<<<<<<") && read(&fx, B).contains("<<<<<<<"));
    (fx, x, y)
}
fn resolve_file(fx: &SvnFixture, rel: &str, body: &str) {
    write(fx, rel, body);
    svn_cmd(
        &fx.wc(),
        &[
            "resolve",
            "--accept",
            "working",
            "--non-interactive",
            "--",
            rel,
        ],
    );
}

#[test]
fn conflicts_in_two_commits_both_resolved_in_svn_follow_in_one_click() {
    let (fx, x, y) = two_conflicts();
    git(&fx, &["branch", "other"]);
    let a = lines(&[(2, "two-both")]);
    let b = lines(&[(3, "three-both")]);
    resolve_file(&fx, A, &a);
    resolve_file(&fx, B, &b);

    let (ready, details, _) = manual(run(&fx));
    assert!(ready, "{details:?}");
    let seen: Vec<(String, Option<String>)> = details
        .iter()
        .map(|d| (d.path.clone(), d.commit.clone()))
        .collect();
    assert_eq!(
        seen,
        vec![
            (A.to_string(), Some(x.clone())),
            (B.to_string(), Some(y.clone()))
        ],
        "每个文件记着自己在哪个 commit 冲突"
    );

    let rep = done(with_disk(&fx, &[A, B]));
    assert_eq!(
        rep.disk_resolved,
        vec![
            (x.clone(), vec![A.to_string()]),
            (y.clone(), vec![B.to_string()])
        ]
    );
    assert_eq!(oid(&fx, "HEAD~2"), oid(&fx, "svn/base"));
    assert_eq!(git(&fx, &["show", &format!("HEAD:{A}")]), a.trim_end());
    assert_eq!(git(&fx, &["show", &format!("HEAD:{B}")]), b.trim_end());
    assert_eq!(git(&fx, &["status", "--porcelain", "--", "Code"]), "");
    assert_eq!(oid(&fx, "other"), y, "另一条分支留在原地");
    assert_no_residue(&fx);
}

#[test]
fn second_commit_not_usable_leaves_only_manual_follow() {
    let (fx, x, y) = two_conflicts();
    resolve_file(&fx, A, &lines(&[(2, "two-both")]));
    // B 合好之后又在冲突处以外改了一行
    resolve_file(&fx, B, &lines(&[(3, "three-both"), (9, "nine-later")]));

    let (ready, details, _) = manual(run(&fx));
    assert!(!ready);
    assert_eq!(details.len(), 2, "{details:?}");
    assert_eq!(details[0].disk, Some(DiskVerdict::Usable));
    assert_eq!(details[0].commit.as_deref(), Some(x.as_str()));
    assert!(reason(&details[1]).contains("冲突处以外"));
    assert_eq!(details[1].commit.as_deref(), Some(y.as_str()));
    let (ready, _, _) = manual(with_disk(&fx, &[A, B]));
    assert!(!ready);
    assert_eq!(oid(&fx, "HEAD"), y, "什么都没动");
    assert_no_residue(&fx);
}

#[test]
fn files_the_dialog_did_not_list_are_not_replaced() {
    let (fx, c) = conflicted();
    resolve_in_svn(&fx, Some(&merged()));
    let (ready, _, _) = manual(with_disk(&fx, &[]));
    assert!(ready, "照样能用，只是没确认过");
    assert_eq!(oid(&fx, "HEAD"), c);
    assert_no_residue(&fx);
}

#[test]
fn another_branch_with_the_same_commit_is_not_moved_with_disk_content() {
    // 设计 §11 第 2 条：磁盘只属于当前分支
    let (fx, c) = conflicted();
    git(&fx, &["branch", "other"]);
    resolve_in_svn(&fx, Some(&merged()));
    // 照真实顺序：先「跟进」发布基线、弹出冲突框，再点按钮
    assert!(manual(run(&fx)).0);
    let rep = done(with_disk(&fx, &[A]));
    assert_eq!(oid(&fx, "other"), c, "另一条分支留在原地");
    assert!(
        rep.behind_branches.iter().any(|(b, _)| b == "other"),
        "{:?}",
        rep.behind_branches
    );
    assert!(!rep.moved_branches.contains(&"other".to_string()));
}
