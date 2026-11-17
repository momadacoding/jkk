#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进时重放一段有 10 个提交的分支，起了哪些 git 进程（日志
//! `2026-11-17-replay-inproc-reads-decisions.md`）。
//!
//! 2026-11-16 Client 仓库 11 个提交的重放里，每个提交一次的只读进程（`log -1`、`rev-parse`、
//! `ls-tree`）约 30 个、2.4 秒。这里把「只读进程不随提交数增长」钉住：以后谁在每个提交的循环里
//! 加一次 git 读，测试会变红。
//!
//! 单独一个测试二进制，理由同 `svn_follow_scan_budget.rs`：全局的性能记录器一个进程只能装一次。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_follow::{follow, FollowOutcome};
use std::collections::BTreeMap;
use svn_fixtures::{hybrid_with_colleague, svn_cmd};

const COMMITS: usize = 10;

/// 记录文件里 `since` 之后真的起了的 git 进程，按 `git <sub>` 计数。
fn git_procs(log: &std::path::Path, since: usize) -> BTreeMap<String, usize> {
    let mut n = BTreeMap::new();
    for line in std::fs::read_to_string(log).unwrap().lines().skip(since) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let name = v["name"].as_str().unwrap_or("");
        if v["kind"] == "proc" && name.starts_with("git ") && v["cached"] != true {
            *n.entry(name.to_string()).or_insert(0) += 1;
        }
    }
    n
}

#[test]
fn replaying_a_branch_does_not_start_a_git_read_per_commit() {
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(fx.wc())
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    // 分支上 10 个提交，各改一个只在 git 里的文件：SVN 上没有，「已全部提交」的捷径不适用，逐个重放
    for i in 0..COMMITS {
        let rel = format!("Code/Mine{i}.cs");
        std::fs::write(fx.wc().join(&rel), format!("mine {i}\n")).unwrap();
        git(&["add", "--", &rel]);
        git(&["commit", "-qm", &format!("我的第 {i} 个提交\n\n说明")]);
    }
    fx.colleague_commits("Code/B.cs", "b2\n");
    svn_cmd(
        &fx.wc(),
        &["update", "--non-interactive", "--", "Code/B.cs"],
    );

    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));
    let since = std::fs::read_to_string(&log).unwrap().lines().count();
    let out = follow(&wc, "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message));
    let FollowOutcome::Done { report } = out else {
        panic!("应当 Done：{out:?}");
    };
    assert!(
        report.dropped.is_empty(),
        "前提：10 个提交都保留 {report:?}"
    );

    let procs = git_procs(&log, since);
    println!("git processes during follow ({COMMITS} commits): {procs:#?}");
    // 写操作每个提交各一次：merge-tree 合、commit-tree 建
    for write in ["git merge-tree", "git commit-tree"] {
        assert!(
            procs.get(write).copied().unwrap_or(0) >= COMMITS,
            "前提：真的逐个重放了（{write}）{procs:#?}"
        );
    }
    // 除了每个提交本来就要的写操作，别的 git 进程都不随提交数增长。反过来列白名单：谁在循环里新加
    // 一种读（`show`、`for-each-ref`…），不用先想到把它加进清单，测试照样变红（独立 review 第 2 条）
    let per_commit_writes = [
        "git merge-tree",
        "git commit-tree",
        "git update-ref",
        "git read-tree",
        "git write-tree",
        "git update-index",
    ];
    for (name, n) in &procs {
        if per_commit_writes.contains(&name.as_str()) {
            continue;
        }
        assert!(
            *n < COMMITS / 2,
            "{name} 起了 {n} 次，随提交数增长了：{procs:#?}"
        );
    }
}
