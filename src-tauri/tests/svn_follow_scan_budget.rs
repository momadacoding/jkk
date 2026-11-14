#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 检测与自动跟进各跑几遍 `svn status` 全量扫描（日志 `2026-11-14-follow-latency-decisions.md`）。
//!
//! 公司机器上一遍全量扫描约 4 秒，跟进的耗时主要由遍数决定。这里把遍数钉住：以后谁在这条
//! 链路上多加一次扫描，测试会变红，而不是等用户说「又慢了」。
//!
//! 单独一个测试二进制，理由同 `svn_follow_reuse_base.rs`：全局的性能记录器一个进程只能装一次。
//! 所以本文件只有一个测试。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_follow::{follow_auto, AutoFollowOutcome};
use app_lib::engine::svn_preflight::run_preflight_with;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

/// 记录文件里真的起了进程的 `svn <sub>`（走扫描缓存的不算）。`skip_flag` 非空时带这个参数的不算。
fn procs(log: &std::path::Path, sub: &str, skip_flag: &str) -> usize {
    let name = format!(r#""name":"svn {sub}""#);
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .filter(|l| l.contains(r#""kind":"proc""#) && l.contains(&name))
        .filter(|l| !l.contains(r#""cached":true"#))
        .filter(|l| skip_flag.is_empty() || !l.contains(&format!(r#""{skip_flag}""#)))
        .count()
}

/// `svn status` 的全量扫描。`--depth=empty` 只看一个节点（P3），不遍历磁盘，不算。
fn scans(log: &std::path::Path) -> usize {
    procs(log, "status", "--depth")
}

fn colleague_changes(fx: &SvnFixture, rel: &str, content: &str) {
    fx.colleague_commits(rel, content);
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
}

#[test]
fn detection_and_auto_follow_stay_within_their_scan_budget() {
    let fx = hybrid_with_colleague();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));

    // 我有一个改着没提交的文件，同事改了另一个，我整个工作副本 update：跟进的常态
    std::fs::write(fx.wc().join("Code/A.cs"), "a-mine\n").unwrap();
    colleague_changes(&fx, "Code/B.cs", "b2\n");

    // 检测：找 external、读节点 revision 是同一遍扫描
    let before = scans(&log);
    let status = check_sync(&fx.wc_str()).unwrap();
    assert!(
        matches!(status.verdict, SyncVerdict::ExternallyAdvanced { .. }),
        "{:?}",
        status.verdict
    );
    assert_eq!(scans(&log) - before, 1, "检测只该扫一遍");

    // 自动跟进：准备阶段一遍（找 external、节点 revision、脏文件共用），发布前锁内复核一遍
    let (before, before_info_r) = (scans(&log), procs(&log, "info", ""));
    let lines_before = records(&log).len();
    let out =
        follow_auto(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message));
    assert!(matches!(out, AutoFollowOutcome::Done { .. }), "{out:?}");
    assert_eq!(scans(&log) - before, 2, "自动跟进只该扫两遍");
    // 记下 svn info 的次数只为了在它变多时有人看一眼，不是硬指标
    println!(
        "svn info during auto follow: {}",
        procs(&log, "info", "") - before_info_r
    );

    // 找被排除节点的 `svn info -R` 是提前放到后台跑的，但必须在磁盘录完（`git add -A`）之后才
    // 起：排在前面的话，两者之间发生的 `--set-depth=exclude` 会让基线静默丢文件（见
    // `svn_init::Indexed::excluded_ahead`）。它要与后面的步骤并行，所以也不能拖到写树之后
    let during: Vec<Rec> = records(&log).split_off(lines_before);
    let one = |name: &str, flag: &str| -> Rec {
        let hits: Vec<&Rec> = during
            .iter()
            .filter(|r| r.name == name && r.flags.iter().any(|f| f == flag))
            .collect();
        assert_eq!(hits.len(), 1, "应当恰好一条 {name} {flag}：{during:?}");
        hits[0].clone()
    };
    let (add, excluded, write_tree) = (
        one("git add", "-A"),
        one("svn info", "-R"),
        one("git write-tree", ""),
    );
    assert!(
        excluded.ts >= add.ts + add.dur,
        "svn info -R 起在 git add 结束之前：{excluded:?} / {add:?}"
    );
    assert!(
        excluded.ts < write_tree.ts,
        "svn info -R 应当在写树之前就起跑（并行），实际排在了后面：{excluded:?} / {write_tree:?}"
    );

    // 跟进完再检测：已经一致，仍然只扫一遍
    let before = scans(&log);
    let status = check_sync(&fx.wc_str()).unwrap();
    assert_eq!(status.verdict, SyncVerdict::InSync);
    assert_eq!(scans(&log) - before, 1, "跟进后的复查只该扫一遍");

    // 前置校验（更新、交付、重建、手动跟进都从它开始）：枚举、脏条目、节点状态共用一遍
    let before = scans(&log);
    run_preflight_with(&fx.wc_str(), &["Code/".to_string()], true).unwrap();
    assert_eq!(scans(&log) - before, 1, "前置校验只该扫一遍");
}

/// 记录文件里的一条子进程记录。
#[derive(Debug, Clone)]
struct Rec {
    name: String,
    /// 起跑时刻与耗时，毫秒。
    ts: f64,
    dur: f64,
    flags: Vec<String>,
}

/// 真的起了进程的记录，按文件里的顺序。没带任何参数的记录给一个空串当 flag，方便按名字找。
fn records(log: &std::path::Path) -> Vec<Rec> {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["kind"] == "proc" && v["meta"]["cached"] != true)
        .map(|v| {
            let mut flags: Vec<String> = v["meta"]["flags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|f| f.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            flags.push(String::new());
            Rec {
                name: v["name"].as_str().unwrap().to_string(),
                ts: v["ts"].as_f64().unwrap(),
                dur: v["dur"].as_f64().unwrap(),
                flags,
            }
        })
        .collect()
}
