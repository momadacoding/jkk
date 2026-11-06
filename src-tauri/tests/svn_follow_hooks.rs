#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进按 TortoiseSVN 钩子事件判定（C1h 计划 Task 5；分析 §6.13.3）。
//!
//! fixture 与 `svn_follow.rs` 相同：wc1 是「你」，wc2 是同事，「TortoiseSVN 提交」就是在 wc1
//! 里 `svn commit` 一个文件。钩子事件直接用 `write_event` 写进收件箱，健康状态用
//! `fault::pretend_hook_health` 注入：测试不读写注册表。
//!
//! 判定对照：事件说的与作者 + 快照推断（C1a）说的相反时，结论跟事件走，这样才证明事件真的
//! 被用上了。所以多数用例故意把「我的 SVN 用户名」设成别人，或者造一个旧规则会丢弃的形状。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::fault;
use app_lib::engine::svn_follow::{follow, Evidence, FollowOutcome, FollowReport};
use app_lib::engine::svn_follow_index::StageDecision;
use app_lib::engine::svn_follow_mark::{mark_path, write_mark};
use app_lib::engine::tsvn_hooks::HookHealth;
use app_lib::engine::tsvn_inbox::{inbox_dir, write_event, TsvnEvent};
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}
fn oid(fx: &SvnFixture, rev: &str) -> String {
    git(fx, &["rev-parse", rev])
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    std::fs::write(fx.wc().join(rel), s).unwrap();
}
fn show(fx: &SvnFixture, spec: &str) -> String {
    git(fx, &["show", spec])
}
fn commit_all(fx: &SvnFixture, msg: &str) -> String {
    git(fx, &["add", "-A"]);
    git(fx, &["commit", "-qm", msg]);
    oid(fx, "HEAD")
}
fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}
fn tortoise_commit(fx: &SvnFixture, rel: &str) -> u64 {
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", rel],
    );
    youngest(fx)
}
fn svn_update(fx: &SvnFixture, rel: &str) {
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", rel]);
}
fn set_me(fx: &SvnFixture, who: &str) {
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some(who.to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();
}
/// file:// 下提交者就是系统用户：把「我」设成它，旧规则就会认为提交是我做的。
fn i_am_the_committer(fx: &SvnFixture) {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(["info", "--show-item", "last-changed-author", "--", A])
        .output()
        .unwrap();
    set_me(fx, String::from_utf8_lossy(&out.stdout).trim());
}
fn staged_blob(fx: &SvnFixture, rel: &str) -> String {
    show(fx, &format!(":{rel}"))
}

/// 快照写好后把它的时刻改成 `ms`：事件的时刻是测试自己定的，两者要在同一条时间轴上。
fn mark_at(fx: &SvnFixture, ms: i64) {
    write_mark(&fx.wc_str()).unwrap();
    let p = mark_path(&fx.wc_str());
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
    v["atMs"] = serde_json::json!(ms);
    std::fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
}

fn event(fx: &SvnFixture, kind: &str, at_ms: i64, paths: &[&str], revision: Option<u64>) {
    write_event(
        &fx.wc_str(),
        &TsvnEvent {
            version: 1,
            kind: kind.to_string(),
            at_ms,
            paths: paths.iter().map(|p| p.to_string()).collect(),
            revision,
            error: None,
        },
    )
    .unwrap();
}

/// 一次带钩子的 TortoiseSVN 提交：真提交，再写一对 Pre / Post。
fn hook_commit(fx: &SvnFixture, rel: &str, pre_ms: i64, post_ms: i64) -> u64 {
    event(fx, "pre_commit", pre_ms, &[rel], None);
    let r = tortoise_commit(fx, rel);
    event(fx, "post_commit", post_ms, &[rel], Some(r));
    r
}

fn healthy() -> fault::Armed {
    fault::pretend_hook_health(HookHealth::Healthy { block_leaks: true })
}

fn done(fx: &SvnFixture) -> FollowReport {
    match follow(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message)) {
        FollowOutcome::Done { report } => *report,
        other => panic!("应当 Done：{other:?}"),
    }
}

fn inbox_count(fx: &SvnFixture) -> usize {
    std::fs::read_dir(inbox_dir(&fx.wc_str()))
        .map(|d| d.count())
        .unwrap_or(0)
}

/// 暂存了中间版 a-mid，磁盘是最终版 a-final；快照（时刻 1000）在暂存之后、提交之前。
fn staged_intermediate(fx: &SvnFixture) {
    write(fx, A, "a-mid\n");
    git(fx, &["add", A]);
    write(fx, A, "a-final\n");
    mark_at(fx, 1_000);
}

#[test]
fn with_hooks_stage_older_than_mark_is_discarded_for_my_commit_event() {
    let fx = hybrid_with_colleague();
    // 旧规则下作者不是我：会走三方合并、丢失暂存。丢弃只能来自事件
    set_me(&fx, "somebody-else");
    staged_intermediate(&fx);
    hook_commit(&fx, A, 2_000, 2_100);
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.evidence, Evidence::Hooks);
    assert_eq!(rep.stage.discarded, vec![A.to_string()]);
    assert!(rep.stage.uncertain.is_empty());
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn with_hooks_stage_differing_from_mark_is_kept_uncertain() {
    let fx = hybrid_with_colleague();
    i_am_the_committer(&fx);
    // 快照里没有这份暂存：暂存可能是提交之后才做的
    mark_at(&fx, 1_000);
    write(&fx, A, "a-final\n");
    hook_commit(&fx, A, 2_000, 2_100);
    write(&fx, A, "a-after\n");
    git(&fx, &["add", A]);
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
    assert_eq!(staged_blob(&fx, A), "a-after");
}

#[test]
fn with_hooks_mark_not_before_t_commit_is_uncertain() {
    let fx = hybrid_with_colleague();
    // 旧规则会丢弃（作者是我、暂存在快照里）；事件说快照不早于提交，拿不准
    i_am_the_committer(&fx);
    staged_intermediate(&fx);
    mark_at(&fx, 3_000);
    hook_commit(&fx, A, 2_000, 2_100);
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
    assert!(rep.stage.discarded.is_empty());
    assert_eq!(staged_blob(&fx, A), "a-mid");
}

#[test]
fn with_hooks_post_without_pre_is_uncertain() {
    let fx = hybrid_with_colleague();
    i_am_the_committer(&fx);
    staged_intermediate(&fx);
    let r = tortoise_commit(&fx, A);
    event(&fx, "post_commit", 2_100, &[A], Some(r));
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
    assert!(rep.stage.discarded.is_empty());
}

#[test]
fn with_hooks_my_author_without_event_is_uncertain_not_discarded() {
    // 分析 R22：另一台机器、命令行 svn 的提交没有本机事件。旧规则按作者丢弃，钩子健康时不丢
    let fx = hybrid_with_colleague();
    i_am_the_committer(&fx);
    staged_intermediate(&fx);
    tortoise_commit(&fx, A);
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.evidence, Evidence::Hooks);
    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
    assert!(rep.stage.discarded.is_empty());
}

/// 同事改了 A 的最后一行，我暂存了第一行；作者在 file:// 下与我相同。
/// 返回同事的 revision。
fn colleague_change_with_my_stage(fx: &SvnFixture) -> u64 {
    i_am_the_committer(fx);
    write(fx, A, "1\n2\n3\n4\n5\n6\n7\n8\n");
    tortoise_commit(fx, A);
    let _h = healthy();
    done(fx);
    drop(_h);
    fx.colleague_commits(A, "1\n2\n3\n4\n5\n6\n7\ncolleague\n");
    let r = youngest(fx);
    write(fx, A, "mine\n2\n3\n4\n5\n6\n7\n8\n");
    git(fx, &["add", A]);
    svn_update(fx, A);
    r
}

#[test]
fn with_hooks_update_event_path_is_three_way_merged() {
    // 旧规则：作者是我、没有快照 → 拿不准、原样保留。更新事件说变化来自服务器 → 三方合并
    let fx = hybrid_with_colleague();
    let r = colleague_change_with_my_stage(&fx);
    event(&fx, "post_update", 9_000, &[A], Some(r));
    let _h = healthy();

    let rep = done(&fx);

    assert!(
        matches!(rep.stage.decisions.get(A), Some(StageDecision::Merged(_))),
        "{:?}",
        rep.stage
    );
    assert_eq!(staged_blob(&fx, A), "mine\n2\n3\n4\n5\n6\n7\ncolleague");
}

#[test]
fn with_hooks_later_update_beats_earlier_commit_and_tie_goes_to_commit() {
    // 更新的 revision 更高：按更新（三方合并）
    let fx = hybrid_with_colleague();
    let r = colleague_change_with_my_stage(&fx);
    event(&fx, "pre_commit", 8_000, &[A], None);
    event(&fx, "post_commit", 8_100, &[A], Some(r - 1));
    event(&fx, "post_update", 9_000, &[A], Some(r));
    let _h = healthy();
    let rep = done(&fx);
    assert!(
        matches!(rep.stage.decisions.get(A), Some(StageDecision::Merged(_))),
        "{:?}",
        rep.stage
    );
    drop(_h);

    // revision 相同：按提交。快照（1000）早于 t_commit（2000），暂存也在快照里 → 中间版本，丢弃。
    // 按更新的话会是三方合并，两种结论能分开
    let fx = hybrid_with_colleague();
    let r = colleague_change_with_my_stage(&fx);
    mark_at(&fx, 1_000);
    event(&fx, "pre_commit", 2_000, &[A], None);
    event(&fx, "post_commit", 2_100, &[A], Some(r));
    event(&fx, "post_update", 9_000, &[A], Some(r));
    let _h = healthy();
    let rep = done(&fx);
    assert_eq!(rep.stage.discarded, vec![A.to_string()], "{:?}", rep.stage);
}

/// 把 `rel` 铺成 8 行并收进基线：后面的用例要让 commit 与 SVN 改不同的行，重放才不冲突，
/// 「换成 SVN 版本」和「照常重放」的结果才分得开。
fn seed_lines(fx: &SvnFixture, rel: &str) {
    write(fx, rel, "1\n2\n3\n4\n5\n6\n7\n8\n");
    tortoise_commit(fx, rel);
    let _h = healthy();
    done(fx);
}

#[test]
fn with_hooks_my_author_without_event_commit_path_is_replayed_not_superseded() {
    // 分析 R22 的 commit 版本：B 的 SVN 提交作者是我，却没有本机事件。旧规则按作者把 commit 里的
    // B 换成 SVN 版本，「mine」一行就丢了。钩子健康时只认事件，B 照常重放
    let fx = hybrid_with_colleague();
    i_am_the_committer(&fx);
    seed_lines(&fx, B);
    write(&fx, A, "a2\n");
    write(&fx, B, "mine\n2\n3\n4\n5\n6\n7\n8\n");
    let c = commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    mark_at(&fx, 1_000);
    hook_commit(&fx, A, 2_000, 2_100);
    fx.colleague_commits(B, "1\n2\n3\n4\n5\n6\n7\nsvn\n");
    svn_update(&fx, B);
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.rewritten, vec![(c, vec![A.to_string()])]);
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a3");
    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "mine\n2\n3\n4\n5\n6\n7\nsvn");
}

#[test]
fn with_hooks_commit_made_after_the_svn_commit_keeps_its_work() {
    // 快照晚于这次 TortoiseSVN 提交：快照里的 commit 不一定早于提交，可能是提交之后的新工作。
    // 换成 SVN 版本会把「mine」一行静默丢掉，所以照常重放
    let fx = hybrid_with_colleague();
    seed_lines(&fx, A);
    write(&fx, A, "1\n2\n3\n4\n5\n6\n7\nfinal\n");
    hook_commit(&fx, A, 2_000, 2_100);
    write(&fx, A, "mine\n2\n3\n4\n5\n6\n7\nfinal\n");
    commit_all(&fx, "提交之后的新工作");
    mark_at(&fx, 3_000);
    let _h = healthy();

    let rep = done(&fx);

    assert!(rep.rewritten.is_empty(), "{:?}", rep.rewritten);
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "mine\n2\n3\n4\n5\n6\n7\nfinal");
}

#[test]
fn with_hooks_intermediate_commit_is_superseded_for_an_event_path() {
    // U3：git 里提交过中间版本，TortoiseSVN 提交了最终版。旧规则作者不是我 → 不替换、冲突
    let fx = hybrid_with_colleague();
    set_me(&fx, "somebody-else");
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    let c = commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    mark_at(&fx, 1_000);
    hook_commit(&fx, A, 2_000, 2_100);
    let _h = healthy();

    let rep = done(&fx);

    assert_eq!(rep.rewritten, vec![(c, vec![A.to_string()])]);
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a3");
    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "b2");
}

#[test]
fn stale_hooks_fall_back_to_heuristic() {
    let fx = hybrid_with_colleague();
    set_me(&fx, "somebody-else");
    staged_intermediate(&fx);
    hook_commit(&fx, A, 2_000, 2_100);
    let _h = fault::pretend_hook_health(HookHealth::Stale {
        reasons: vec!["exe 换了位置".into()],
    });

    let rep = done(&fx);

    assert_eq!(rep.evidence, Evidence::Heuristic);
    assert!(rep.stage.discarded.is_empty(), "失效时不能按事件丢弃");
}

#[test]
fn not_registered_reports_heuristic_and_keeps_inbox() {
    let fx = hybrid_with_colleague();
    set_me(&fx, "somebody-else");
    staged_intermediate(&fx);
    hook_commit(&fx, A, 2_000, 2_100);
    let _h = fault::pretend_hook_health(HookHealth::NotRegistered);

    let rep = done(&fx);

    assert_eq!(rep.evidence, Evidence::Heuristic);
    assert_eq!(inbox_count(&fx), 2, "没用上的事件不清理");
}

#[test]
fn follow_prunes_processed_events() {
    let fx = hybrid_with_colleague();
    set_me(&fx, "somebody-else");
    staged_intermediate(&fx);
    hook_commit(&fx, A, 2_000, 2_100);
    // 跟进开始之后才到的事件（时刻在很远的将来）：留着给下一次
    event(&fx, "post_update", i64::MAX / 2, &[B], Some(99));
    let _h = healthy();

    done(&fx);

    assert_eq!(inbox_count(&fx), 1);
}
