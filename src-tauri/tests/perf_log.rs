#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 记录模式下真的 git 子进程会留下 `proc` 记录，且记录里没有路径和提交说明。
//!
//! 本文件只有一个测试、装的是**全局**记录器：每个 tests/*.rs 是独立的进程，
//! 装了也影响不到别的测试二进制（决策日志 D3）。不要在这里再加别的测试。

use app_lib::engine::{git_cli, perf, svn_cli};
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

/// fixture 自己发的 git 命令，不走被测的 `run_git`，也不进记录。
fn fixture_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn rows(path: &std::path::Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn git_subprocesses_are_recorded_without_paths_or_messages() {
    let repo = TempDir::new().unwrap();
    let repo_dir = repo.path().join("secret-project-x");
    fs::create_dir(&repo_dir).unwrap();
    fixture_git(&repo_dir, &["init"]);
    fixture_git(&repo_dir, &["config", "user.email", "test@test.com"]);
    fixture_git(&repo_dir, &["config", "user.name", "Test"]);
    fs::write(repo_dir.join("classified-file.txt"), "1").unwrap();
    fixture_git(&repo_dir, &["add", "classified-file.txt"]);

    let logs = TempDir::new().unwrap();
    let file = logs.path().join("perf").join("p.jsonl");
    assert!(!perf::enabled());
    assert!(perf::install(perf::PerfLog::create(file.clone()).unwrap()));
    assert!(perf::enabled());

    let path = repo_dir.to_str().unwrap();
    // 成功的一条（走 output_or_error）
    git_cli::run_git(
        path,
        &[
            "commit",
            "-m",
            "launch codes 1234",
            "--",
            "classified-file.txt",
        ],
    )
    .unwrap();
    // 失败的一条（非零退出）
    assert!(git_cli::run_git(path, &["rev-parse", "--verify", "no-such-ref"]).is_err());
    // 走 stdin 管道那条路（spawn + wait）
    git_cli::run_git_stdin(path, &["hash-object", "--stdin"], b"payload").unwrap();
    // 进程起不来：工作目录不存在
    assert!(svn_cli::run_svn_code(r"Z:\definitely\missing\dir", &["info"]).is_err());

    let rows = rows(&file);
    let procs: Vec<&Value> = rows.iter().filter(|r| r["kind"] == "proc").collect();
    let named = |name: &str| -> &Value {
        procs
            .iter()
            .find(|r| r["name"] == name)
            .unwrap_or_else(|| panic!("没有 {name}：{rows:?}"))
    };

    let commit = named("git commit");
    assert_eq!(commit["ok"], true);
    assert_eq!(commit["src"], "rust");
    assert_eq!(commit["meta"]["exit"], 0);
    assert_eq!(commit["meta"]["flags"], serde_json::json!(["-m", "--"]));
    assert_eq!(commit["meta"]["argc"], 5);
    assert_eq!(commit["meta"]["repo"], "secret-project-x");
    assert!(commit["dur"].as_f64().unwrap() > 0.0);
    assert!(commit["ts"].as_f64().unwrap() > 1.7e12);

    let failed = named("git rev-parse");
    assert_eq!(failed["ok"], false);
    assert_ne!(failed["meta"]["exit"], 0);

    assert_eq!(named("git hash-object")["ok"], true);

    let svn = named("svn info");
    assert_eq!(svn["ok"], false);

    let text = fs::read_to_string(&file).unwrap();
    let tmp = repo.path().to_str().unwrap().replace('\\', "\\\\");
    for leak in [
        "launch codes",
        "classified-file",
        "no-such-ref",
        "payload",
        tmp.as_str(),
    ] {
        assert!(!text.contains(leak), "记录里出现了 {leak}：\n{text}");
    }
}
