#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 部分跟进之前先判暂存区（计划 `2026-11-22-follow-perf-and-refresh-loop.md` T4 测试 a；
//! 2026-11-21 使用报告 §2.2）。
//!
//! 现场：自动跟进第一遍被重放冲突拦下，第二遍部分跟进跑了约 35 s（体检 28 s），发布前拼索引才发现
//! 暂存的文件保不住，整次跳过。暂存区的判定在第一遍录完新基线时就能做。
//!
//! 单独一个测试二进制：要数性能记录里体检的次数，全局的性能记录器一个进程只能装一次。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::perf::{self, PerfLog};
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::{follow, follow_auto, AutoFollowOutcome, FollowOutcome};
use app_lib::engine::svn_journal;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";
const OLD_B: &str = "1\n2\n3\n4\n5\n6\n7\n8\n";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args)
        .unwrap_or_else(|e| panic!("git {args:?}: {}", e.message))
        .trim()
        .to_string()
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    std::fs::write(fx.wc().join(rel), s).unwrap();
}
fn tortoise_commit(fx: &SvnFixture, rel: &str) {
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", rel],
    );
}
fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}
/// file:// 下改作者只能改 revprop（同 `svn_follow_auto.rs`）。
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
/// 「我」取 wc1 里提交的作者（同 `svn_follow_auto.rs` 的 `i_commit`）。
fn i_commit(fx: &SvnFixture) {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(["info", "--show-item", "last-changed-author", "--", A])
        .output()
        .unwrap();
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some(String::from_utf8_lossy(&out.stdout).trim().to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();
}
fn count(log: &std::path::Path, name: &str, since: f64) -> usize {
    std::fs::read_to_string(log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["name"] == name && v["ts"].as_f64().unwrap() >= since)
        .count()
}

#[test]
fn partial_follow_that_would_lose_staging_skips_before_the_second_pass() {
    let fx = hybrid_with_colleague();
    let log = fx.path().join("perf.jsonl");
    assert!(perf::install(PerfLog::create(log.clone()).unwrap()));
    i_commit(&fx);

    // B 先落成 1..8（同 `svn_follow_auto.rs` 的 `staged_scene`）
    write(&fx, B, OLD_B);
    tortoise_commit(&fx, B);
    assert!(matches!(
        follow(&fx.wc_str(), "seed").unwrap(),
        FollowOutcome::Done { .. }
    ));
    // 分支上 A 的中间版本与提交到 SVN 的不同：第一遍重放冲突，A 留在旧基线上走第二遍
    write(&fx, A, "a2\n");
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "中间版本"]);
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);
    // 同事改了 B 的第 1 行，我暂存的 B 也改了第 1 行：三方合并冲突，暂存保不住
    fx.colleague_commits(B, "theirs\n2\n3\n4\n5\n6\n7\ncolleague\n");
    let r = youngest(&fx);
    set_author(&fx, r, "colleague");
    write(&fx, B, "mine\n2\n3\n4\n5\n6\n7\n8\n");
    git(&fx, &["add", B]);
    write(&fx, B, OLD_B);
    // 根目录整体 update（每天早上的常态）：全部节点到最新 revision。A 留在旧基线上会让 floor 往下掉，
    // 第二遍因此要重新确认墓碑、再体检一次（现场 28 s 的来源，使用报告 §2.3）
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let refs_before = git(&fx, &["for-each-ref"]);
    let index_before = std::fs::read(fx.wc().join(".git/index")).unwrap();
    let started = perf::now_ms();
    let out = follow_auto(&fx.wc_str(), "auto1").unwrap();

    let AutoFollowOutcome::Skipped { code, message, .. } = out else {
        panic!("应当不自动跟进：{out:?}");
    };
    assert_eq!(code, "svn_auto_follow_stage", "{message}");
    assert!(message.contains(B), "{message}");
    assert_eq!(git(&fx, &["for-each-ref"]), refs_before, "一个 ref 都没动");
    assert_eq!(
        std::fs::read(fx.wc().join(".git/index")).unwrap(),
        index_before,
        "索引一个字节都没动"
    );
    assert!(!fx.wc().join(".git/index.lock").exists());
    // 只看这一次的：铺场景时那次跟进钉住的旧索引照规矩留着
    assert!(git(&fx, &["for-each-ref", "refs/mygitclient/follow/auto1"]).is_empty());
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    // 第二遍没跑：体检只有第一遍那一次。第二遍的 floor 比第一遍低，重新确认墓碑时会再体检一次、
    // 再读一次 wc.db 形状（floor 不同，T3 的缓存不命中）；以它为标记
    assert_eq!(
        count(&log, "wc.db floor shape", started),
        1,
        "部分跟进的第二遍不该再跑"
    );
}
