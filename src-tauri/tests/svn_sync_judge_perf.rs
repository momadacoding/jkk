#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 判定（`check_sync`）里进程内的几段工作要有打点，外部新增的候选只对 W 候选查磁盘
//! （日志 `2026-11-24-follow-perf-pr1-decisions.md` Task 1、2）。
//!
//! 单独一个测试二进制：全局的性能记录器一个进程只能装一次，所以本文件只有一个测试。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use svn_fixtures::{hybrid_with_colleague, svn_cmd};

fn inproc(log: &std::path::Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["kind"] == "inproc")
        .collect()
}

#[test]
fn judge_records_its_inproc_stages_and_checks_disk_only_for_w_candidates() {
    let fx = hybrid_with_colleague();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));

    // 同事在白名单里加了一个 W-file、一个非 W 文件、一个子目录（里面一个 W-file）
    fx.colleague_commits("Code/C.cs", "c1\n");
    fx.colleague_commits("Code/notes.txt", "n1\n");
    fx.colleague_commits("Code/Sub/D.cs", "d1\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let status = check_sync(&fx.wc_str()).unwrap();
    let SyncVerdict::ExternallyAdvanced { added, .. } = &status.verdict else {
        panic!("{:?}", status.verdict);
    };
    let mut added: Vec<&str> = added.iter().map(|c| c.path.as_str()).collect();
    added.sort_unstable();
    assert_eq!(added, ["Code/C.cs", "Code/Sub/D.cs"], "判定出的集合不变");

    let recs = inproc(&log);
    let names: Vec<&str> = recs.iter().filter_map(|r| r["name"].as_str()).collect();
    for want in [
        "sync read base map",
        "sync baseline tree",
        "svn status parse",
        "sync w scope",
        "sync judge",
        "sync added w-files on disk",
    ] {
        assert!(names.contains(&want), "缺打点 `{want}`：{names:?}");
    }

    // 候选：Code 本身、C.cs、notes.txt、Sub、Sub/D.cs。按路径是 W 的只有 Code（白名单根）、C.cs、Sub/D.cs，
    // 只有这三个该碰磁盘
    let disk = recs
        .iter()
        .find(|r| r["name"] == "sync added w-files on disk")
        .unwrap();
    assert_eq!(disk["meta"]["candidates"], 5, "{disk}");
    assert_eq!(disk["meta"]["disk_checks"], 3, "{disk}");
    assert_eq!(disk["meta"]["added"], 2, "{disk}");
}
