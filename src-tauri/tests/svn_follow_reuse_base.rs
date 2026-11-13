#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进录新基线时，本地改过、服务器版本与上次取的时候相同的文件沿用那次的 blob，不再 `svn cat`
//! （`svn_base_blobs`）。
//!
//! 单独一个测试二进制：要装全局的性能记录器数 `svn cat` 起了几次，记录器一个进程只能装一次，
//! 和别的测试放在一起会把它们的子进程也数进来。所以本文件只有一个测试。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_follow::{follow, FollowOutcome};
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";

fn show(fx: &SvnFixture, spec: &str) -> String {
    run_git(&fx.wc_str(), &["show", spec])
        .unwrap_or_else(|e| panic!("git show {spec}: {}", e.message))
        .trim()
        .to_string()
}

/// 记录文件里 `svn cat` 子进程的条数。
fn svn_cats(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter(|l| l.contains(r#""kind":"proc""#) && l.contains(r#""name":"svn cat""#))
        .count()
}

fn follow_done(fx: &SvnFixture) {
    let out = follow(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message));
    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
}

/// 同事改 B，我整个工作副本 update。
fn colleague_changes_b(fx: &SvnFixture, content: &str) {
    fx.colleague_commits(B, content);
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
}

#[test]
fn dirty_files_whose_server_version_did_not_change_reuse_the_last_blob() {
    let fx = hybrid_with_colleague();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));

    // 第一次跟进：A 改着没提交，要取一次服务器版本，记进缓存
    std::fs::write(fx.wc().join(A), "a-mine\n").unwrap();
    colleague_changes_b(&fx, "b2\n");
    let before = svn_cats(&log);
    follow_done(&fx);
    assert_eq!(svn_cats(&log) - before, 1, "第一次没有缓存，A 要取一次");

    // 第二次：A 还改着，服务器上没变；B 被同事再改一次，我也在本地改了
    colleague_changes_b(&fx, "b3\n");
    std::fs::write(fx.wc().join(B), "b-mine\n").unwrap();
    let before = svn_cats(&log);
    follow_done(&fx);

    // 内容：A 是服务器上的 a1，B 是同事的新版本
    assert_eq!(show(&fx, &format!("svn/base:{A}")), "a1");
    assert_eq!(show(&fx, &format!("svn/base:{B}")), "b3");
    // 进程：只有 B 要取服务器内容
    assert_eq!(
        svn_cats(&log) - before,
        1,
        "A 的服务器版本没变，不该再 svn cat"
    );
}
