#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 「更新」「交付」的预览各跑几遍 `svn status` 全量扫描。
//!
//! 纳入集合预检改成按脏清单修正的只读预言之后（2026-11-25），它要读一遍 SVN 脏清单。那条命令与
//! 失同步判定发的是同一条（`svn_cli::STATUS_SCAN`），整个预览在一个扫描缓存范围里，只该遍历一遍。
//! 公司机器上一遍约 4 秒，这里把遍数钉住。
//!
//! 单独一个测试二进制，理由同 `svn_follow_scan_budget.rs`：全局的性能记录器一个进程只能装一次。
//! 所以本文件只有一个测试。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_out_check::preview_sync_out;
use svn_fixtures::hybrid_with_colleague;

/// 真的起了进程（没走扫描缓存）的全量扫描：`svn status` 带 `-v`。
fn scans(log: &std::path::Path) -> usize {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["kind"] == "proc" && v["meta"]["cached"] != true)
        .filter(|v| v["name"] == "svn status")
        .filter(|v| {
            v["meta"]["flags"]
                .as_array()
                .is_some_and(|a| a.iter().any(|f| f == "-v"))
        })
        .count()
}

#[test]
fn each_sync_preview_scans_the_whitelist_once() {
    let fx = hybrid_with_colleague();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));

    let before = scans(&log);
    preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(
        scans(&log) - before,
        1,
        "更新的预览：失同步判定与纳入集合预检共用一遍"
    );

    std::fs::write(fx.wc().join("Code/A.cs"), "a2\n").unwrap();
    run_git(&fx.wc_str(), &["commit", "-qam", "work"]).unwrap();
    let before = scans(&log);
    preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(
        scans(&log) - before,
        1,
        "交付的预览：失同步判定与第 8 条预检共用一遍"
    );
}
