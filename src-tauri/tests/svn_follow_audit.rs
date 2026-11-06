#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 手动跟进收尾核对（`svn_follow_audit`）的判据，直接在纯 git 的临时仓库上测。
//!
//! 端到端的形状（真 SVN fixture、走完整个手动跟进）在 `tests/svn_follow.rs`；这里钉的是那边
//! 搭不出来、或者搭起来很绕的变体。

use app_lib::engine::svn_follow_audit::audit;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, s: &str) {
    std::fs::write(dir.join(rel), s).unwrap();
}

fn commit(dir: &Path, msg: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-qm", msg]);
    git(dir, &["rev-parse", "HEAD"])
}

fn repo() -> TempDir {
    let d = TempDir::new().unwrap();
    git(d.path(), &["init", "-q"]);
    git(d.path(), &["config", "user.email", "t@t"]);
    git(d.path(), &["config", "user.name", "t"]);
    git(d.path(), &["config", "core.autocrlf", "false"]);
    d
}

/// 旧基线有 `A.cs`、`Gone.cs`；新基线（SVN 之后的版本）改了 `A.cs`、删掉了 `Gone.cs`。
/// 返回 (旧基线, 新基线)。
fn baselines(dir: &Path) -> (String, String) {
    write(dir, "A.cs", "a1\n");
    write(dir, "Gone.cs", "g1\n");
    let old = commit(dir, "svn: r1");
    write(dir, "A.cs", "a2\n");
    std::fs::remove_file(dir.join("Gone.cs")).unwrap();
    let new = commit(dir, "svn: r3");
    (old, new)
}

#[test]
fn a_file_svn_deleted_is_reported_even_when_it_stays_on_disk() {
    // 独立 review：磁盘本来就是旧内容时（分支落后、或白名单里没有改动不用 stash），快照提交带回的
    // 文件跟进前后都在磁盘上。只看「磁盘上没有」会漏掉它，要看旧基线里有没有它。
    let d = repo();
    let dir = d.path();
    let (old, new) = baselines(dir);
    // 原分支：落在旧基线上的一笔快照提交，Gone.cs 还在
    git(dir, &["checkout", "-q", "-b", "work", &old]);
    write(dir, "Gone.cs", "g-snapshot\n");
    // 你自己在 git 里新建的文件：跟进前后都在，旧基线里没有，不能报
    write(dir, "Mine.cs", "mine\n");
    let orig_head = commit(dir, "svn up 之后整包提交");
    // 重放后：同一笔改动叠到新基线上，Gone.cs 被加回来；磁盘就是重放后的样子
    git(dir, &["checkout", "-q", "-B", "work", &new]);
    write(dir, "Gone.cs", "g-snapshot\n");
    write(dir, "Mine.cs", "mine\n");
    commit(dir, "重放后的快照提交");

    let drift = audit(
        dir.to_str().unwrap(),
        &orig_head,
        None,
        &orig_head,
        &old,
        &new,
    )
    .unwrap();

    assert_eq!(drift.resurrected, vec!["Gone.cs".to_string()]);
    // A.cs 跟进前是原分支的旧内容、跟进后是新基线的内容：被正确地挪到了新版本，不报
    assert!(drift.changed.is_empty(), "{:?}", drift.changed);
}

#[test]
fn a_file_moved_to_the_new_baseline_is_not_reported_but_one_merged_wrong_is() {
    // 模块文档的例外：跟进前等于原分支 HEAD、跟进后等于新基线 → 不报；两头都不等 → 报
    let d = repo();
    let dir = d.path();
    let (old, new) = baselines(dir);
    git(dir, &["checkout", "-q", "-b", "work", &new]);
    let orig_head = old.clone();
    // 合错：既不是跟进前的内容，也不是新基线的内容
    write(dir, "A.cs", "a2\na1\n");

    let drift = audit(
        dir.to_str().unwrap(),
        &orig_head,
        None,
        &orig_head,
        &old,
        &new,
    )
    .unwrap();

    assert_eq!(drift.changed, vec!["A.cs".to_string()]);
    assert!(drift.resurrected.is_empty(), "{:?}", drift.resurrected);
}

#[test]
fn a_clean_follow_reports_nothing() {
    // 对照组：跟进前后磁盘一样，什么都不报
    let d = repo();
    let dir = d.path();
    let (old, new) = baselines(dir);
    git(dir, &["checkout", "-q", "-b", "work", &new]);

    let drift = audit(dir.to_str().unwrap(), &new, None, &new, &old, &new).unwrap();

    assert!(drift.is_empty(), "{drift:?}");
}
