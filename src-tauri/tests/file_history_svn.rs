#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 混合仓库的文件历史 / Blame 补全 SVN（计划 2026-11-27-svn-file-history）。
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::file_history::{blame_file, file_history, DeliveredState};
use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::create_baseline_commit_with;
use app_lib::engine::svn_basemap::read_base_map;
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use app_lib::engine::svn_sync_out::{run_sync_out, SyncOutExpect};
use app_lib::engine::svn_sync_out_check::preview_sync_out;
use app_lib::engine::svn_xml::parse_info;
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

const A: &str = "Code/A.cs";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    std::fs::write(fx.wc().join(rel), s).unwrap();
}
fn commit(fx: &SvnFixture, msg: &str) {
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", msg]);
}
/// 「更新」：预览 → 确认 → 执行。
fn sync(fx: &SvnFixture) {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    run_sync_in(
        &fx.wc_str(),
        &SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
}
/// 「交付」：预览 → 确认 → 执行。
fn deliver(fx: &SvnFixture, message: &str) {
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    run_sync_out(
        &fx.wc_str(),
        &SyncOutExpect {
            orig_branch: p.orig_branch.clone(),
            orig_head: p.orig_head.clone(),
            baseline_oid: p.baseline_oid.clone(),
            deliver_tree: p.deliver_tree.clone(),
            changes: p.changes.clone(),
            confirmed_outside: p.outside_changes.clone(),
            message: message.to_string(),
        },
    )
    .unwrap();
}
/// 服务器当前的 HEAD revision。
fn server_head(fx: &SvnFixture) -> u64 {
    let url = svn_fixtures::file_url(&fx.repo());
    let xml = run_svn(&fx.wc_str(), &["info", "--xml", "--", &url]).unwrap();
    parse_info(&xml).unwrap()[0].revision as u64
}
fn history(fx: &SvnFixture, file: &str) -> Vec<app_lib::engine::file_history::FileHistoryEntry> {
    file_history(&fx.wc_str(), file, &["HEAD".to_string()])
        .unwrap()
        .entries
}

// ---------------------------------------------------------------------------
// Task 3：基线 / 交付标注
// ---------------------------------------------------------------------------

#[test]
fn the_init_commit_is_marked_as_a_baseline_with_the_file_rev() {
    let fx = hybrid_with_colleague();
    let h = history(&fx, A);
    let init = h.last().unwrap();
    let svn = init.svn.as_ref().expect("混合仓库应带 svn 标注");
    let b = svn.baseline.as_ref().expect("初始化那条是基线");
    assert_eq!(b.file_rev, 1);
    assert_eq!(b.floor, 1);
    assert_eq!(b.delivered_rev, None);
    assert!(svn.delivered.is_none());
}

#[test]
fn a_local_commit_is_undelivered() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a1\nmine\n");
    commit(&fx, "mine");
    let h = history(&fx, A);
    let svn = h[0].svn.as_ref().unwrap();
    assert!(svn.baseline.is_none());
    assert_eq!(svn.delivered, Some(DeliveredState::Undelivered));
}

#[test]
fn a_delivered_commit_carries_its_svn_rev() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a1\nmine\n");
    commit(&fx, "mine");
    deliver(&fx, "deliver mine");
    let n = server_head(&fx);
    let h = history(&fx, A);
    let mine = h.iter().find(|e| e.subject == "mine").unwrap();
    assert_eq!(
        mine.svn.as_ref().unwrap().delivered,
        Some(DeliveredState::Delivered { rev: n })
    );
}

#[test]
fn a_colleague_update_is_a_baseline_not_a_delivery() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a1\ncolleague\n");
    sync(&fx);
    let h = history(&fx, A);
    let b = h[0].svn.as_ref().unwrap().baseline.as_ref().unwrap();
    assert_eq!(b.file_rev, 2);
    assert_eq!(b.delivered_rev, None);
}

#[test]
fn mixed_revision_baseline_uses_rev_of_not_floor() {
    let fx = hybrid_with_colleague();
    let repo = fx.wc_str();
    let mut map = read_base_map(&repo, "svn/base").unwrap();
    map.floor = 3;
    map.revs.insert(A.to_string(), 5);
    write(&fx, A, "a1\nmixed\n");
    git(&fx, &["add", A]);
    let tree = git(&fx, &["write-tree"]);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    // 顺带带上 Delivered-Rev，验证交付录的基线读得出交付的 revision
    let oid = create_baseline_commit_with(&repo, &tree, Some(&head), &map, &[], Some(7)).unwrap();
    let h = file_history(&repo, A, std::slice::from_ref(&oid))
        .unwrap()
        .entries;
    assert_eq!(h[0].id, oid);
    let b = h[0].svn.as_ref().unwrap().baseline.as_ref().unwrap();
    assert_eq!(b.file_rev, 5);
    assert_eq!(b.floor, 3);
    assert_eq!(b.delivered_rev, Some(7));
}

#[test]
fn plain_git_repo_has_no_svn_annotation() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    run_git(&p, &["init", "-q"]).unwrap();
    std::fs::write(dir.path().join("a.txt"), "x\n").unwrap();
    run_git(&p, &["add", "a.txt"]).unwrap();
    run_git(
        &p,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "c1",
        ],
    )
    .unwrap();
    let h = file_history(&p, "a.txt", &["HEAD".to_string()])
        .unwrap()
        .entries;
    assert!(h.iter().all(|e| e.svn.is_none()));
    let b = blame_file(&p, "a.txt", None).unwrap();
    assert!(b
        .commits
        .values()
        .all(|c| c.baseline.is_none() && c.delivered.is_none()));
}

#[test]
fn blame_commits_carry_baseline_info() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a1\nmine\n");
    commit(&fx, "mine");
    let b = blame_file(&fx.wc_str(), A, None).unwrap();
    let first = &b.lines[0].commit;
    let info = b.commits[first]
        .baseline
        .as_ref()
        .expect("第 1 行来自初始化基线");
    assert_eq!(info.file_rev, 1);
    let second = &b.lines[1].commit;
    assert_eq!(
        b.commits[second].delivered,
        Some(DeliveredState::Undelivered)
    );
}
