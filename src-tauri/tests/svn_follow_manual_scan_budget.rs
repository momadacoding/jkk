#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 手动跟进跑几遍 `svn status` 全量扫描（计划 `2026-11-22-follow-perf-and-refresh-loop.md` T2；
//! 2026-11-21 使用报告 §2.5）。
//!
//! 现场手动跟进开头连跑两次全量 `svn status -v`（2.97 s、2.56 s）：判定一致与前置校验各扫一遍，
//! 外面没有扫描缓存。两者都只读、之间工作副本没动，应当共用一遍。
//!
//! 单独一个测试二进制，理由同 `svn_follow_scan_budget.rs`：全局的性能记录器一个进程只能装一次。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_follow::{follow, manual_follow, FollowOutcome};
use app_lib::engine::svn_sync_in::SyncInOutcome;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";

fn git(fx: &SvnFixture, args: &[&str]) {
    run_git(&fx.wc_str(), args).unwrap_or_else(|e| panic!("git {args:?}: {}", e.message));
}

/// 真的起了进程的全量 `svn status -v`（`svn_cli::STATUS_SCAN`；走扫描缓存的不算）。
fn scans(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["kind"] == "proc" && v["name"] == "svn status" && v["meta"]["cached"] != true)
        .filter(|v| {
            v["meta"]["flags"]
                .as_array()
                .is_some_and(|fs| fs.iter().any(|f| f == "-v"))
        })
        .count()
}

#[test]
fn manual_follow_prechecks_share_one_status_scan() {
    let fx = hybrid_with_colleague();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));

    // W10-R2（同 `svn_follow.rs` 的 `conflict_scene`）：git 里的中间版本与提交到 SVN 的不同，
    // 跟进只发布了基线，分支要手动跟进
    std::fs::write(fx.wc().join(A), "a-git\n").unwrap();
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "git 里的版本"]);
    std::fs::write(fx.wc().join(A), "a-svn\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", A],
    );
    let out = follow(&fx.wc_str(), "op1").unwrap();
    assert!(
        matches!(
            out,
            FollowOutcome::NeedsManual {
                baseline_published: true,
                ..
            }
        ),
        "{out:?}"
    );

    let before = scans(&log);
    let out = manual_follow(&fx.wc_str(), "mf1").unwrap();
    assert!(
        matches!(out, SyncInOutcome::RebaseConflict { .. }),
        "{out:?}"
    );
    // 判定一致（`check_sync_with_nodes`）与前置校验（`run_preflight_with`）共用一遍；stash 与 rebase
    // 改了工作副本，之后的读取必须是新的，不在这一遍里
    assert_eq!(scans(&log) - before, 1, "手动跟进的前置检查只该扫一遍");
}
