#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 体检里改回问服务器的单元，原因按类计数写进性能记录，且不带路径
//! （日志 `2026-11-24-follow-perf-pr1-decisions.md` Task 5）。
//!
//! 单独一个测试二进制：全局的性能记录器一个进程只能装一次，所以本文件只有一个测试。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_health::{check_baseline_map, ExplainedOrigin};
use app_lib::engine::svn_nodes::{compute_map, read_node_status};
use app_lib::engine::svn_scope::WScope;
use app_lib::engine::svn_wc::enumerate_wcs;
use std::collections::{BTreeMap, BTreeSet};
use svn_fixtures::{file_url, svn_cmd, svn_repo_for_health};

#[test]
fn units_sent_to_the_server_are_counted_by_reason_without_paths() {
    let fx = svn_repo_for_health();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));

    // 本地新增：wc.db 的 WORKING 层，这个单元必须问服务器
    std::fs::write(fx.wc().join("Code/New.cs"), "n\n").unwrap();
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "Code/New.cs"]);

    let wc = fx.wc_str();
    let whitelist = vec!["Code/".to_string()];
    let us = enumerate_wcs(&wc, &whitelist).unwrap();
    let nodes = read_node_status(&wc, &whitelist, &us).unwrap();
    let files: BTreeSet<String> = nodes
        .iter()
        .filter(|n| {
            n.node_rev.is_some() && n.path.ends_with(".cs") && fx.wc().join(&n.path).is_file()
        })
        .map(|n| n.path.clone())
        .collect();
    let map = compute_map(
        &us[0].repos_root,
        &us[0].uuid,
        &files,
        &nodes,
        BTreeMap::new(),
    )
    .unwrap();
    let scope = WScope::plain(&whitelist, &[".cs".to_string()]);
    check_baseline_map(
        &wc,
        &file_url(&fx.repo()),
        &map,
        &scope,
        &BTreeSet::new(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();

    let text = std::fs::read_to_string(&log).unwrap();
    let rec: serde_json::Value = text
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .find(|v| v["name"] == "health floor answered locally")
        .unwrap_or_else(|| panic!("没有 floor 记录：{text}"));
    assert_eq!(rec["meta"]["local"], 0, "{rec}");
    assert_eq!(rec["meta"]["server"], 1, "{rec}");
    assert_eq!(
        rec["meta"]["server_reasons"],
        serde_json::json!({ "`…` 有本地的增删或复制（svn add / delete / copy）": 1 }),
        "{rec}"
    );
    assert!(!rec.to_string().contains("New.cs"), "原因不该带路径：{rec}");
}
