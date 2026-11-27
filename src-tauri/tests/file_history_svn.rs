#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 混合仓库的文件历史 / Blame 补全 SVN（计划 2026-11-27-svn-file-history）。
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::file_history::{blame_file, file_history, DeliveredState, FileHistoryEntry};
use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::create_baseline_commit_with;
use app_lib::engine::svn_basemap::read_base_map;
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_file_history::{page_with_units, svn_file_history, GroupSpec};
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use app_lib::engine::svn_sync_out::{run_sync_out, SyncOutExpect};
use app_lib::engine::svn_sync_out_check::preview_sync_out;
use app_lib::engine::svn_wc::WcIdentity;
use app_lib::engine::svn_xml::parse_info;
use std::collections::BTreeMap;
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

// ---------------------------------------------------------------------------
// Task 5：svn_file_history
// ---------------------------------------------------------------------------

/// 同事在 wc2 里跑一串 svn 命令（改名、删除这类 `colleague_commits` 做不了的）。
fn colleague(fx: &SvnFixture, args: &[&str]) {
    let out = std::process::Command::new("svn")
        .current_dir(fx.wc2())
        .arg("--non-interactive")
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "svn {args:?}：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn groups_of(entries: &[FileHistoryEntry]) -> Vec<GroupSpec> {
    entries
        .iter()
        .filter_map(|e| {
            let b = e.svn.as_ref()?.baseline.as_ref()?;
            Some(GroupSpec {
                id: e.id.clone(),
                path: e.path.clone(),
                file_rev: b.file_rev,
            })
        })
        .collect()
}
/// 与前端同一口径：基线记的 Delivered-Rev 加上本地提交的「已交付 rN」。
fn hidden_of(entries: &[FileHistoryEntry]) -> Vec<u64> {
    let mut out = Vec::new();
    for e in entries {
        if let Some(s) = &e.svn {
            if let Some(r) = s.baseline.as_ref().and_then(|b| b.delivered_rev) {
                out.push(r);
            }
            if let Some(DeliveredState::Delivered { rev }) = s.delivered {
                out.push(rev);
            }
        }
    }
    out
}
/// 从 `start` 打开 `file` 的 History，把 SVN 补全一页页取完。返回（各组的 rev 列表, 历史条目）。
fn expand_all(
    fx: &SvnFixture,
    file: &str,
    start: &str,
) -> (BTreeMap<String, Vec<u64>>, Vec<FileHistoryEntry>) {
    let entries = file_history(&fx.wc_str(), file, &[start.to_string()])
        .unwrap()
        .entries;
    let groups = groups_of(&entries);
    let hidden = hidden_of(&entries);
    let mut out: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut cursor = None;
    for _ in 0..20 {
        let page = svn_file_history(&fx.wc_str(), &groups, &hidden, cursor).unwrap();
        assert!(page.error.is_none(), "{:?}", page.error);
        for (id, v) in page.groups {
            out.entry(id).or_default().extend(v.iter().map(|e| e.rev));
        }
        cursor = page.cursor;
        if cursor.is_none() {
            return (out, entries);
        }
    }
    panic!("翻了 20 页还没完");
}
fn init_id(entries: &[FileHistoryEntry]) -> String {
    entries.last().unwrap().id.clone()
}

#[test]
fn three_colleague_commits_between_baselines_expand_into_the_later_one() {
    let fx = hybrid_with_colleague();
    for i in 0..3 {
        fx.colleague_commits(A, &format!("a1\nc{i}\n"));
    }
    sync(&fx);
    let (m, h) = expand_all(&fx, A, "HEAD");
    assert_eq!(m[&h[0].id], vec![4, 3, 2]);
    assert_eq!(m[&init_id(&h)], vec![1]);
    // 作者、说明与 svn log 一致
    let page = svn_file_history(&fx.wc_str(), &groups_of(&h), &[], None).unwrap();
    let e = &page.groups[&h[0].id][0];
    let url = format!("{}/Code/A.cs", svn_fixtures::file_url(&fx.repo()));
    let xml = run_svn(&fx.wc_str(), &["log", "--xml", "-r", "4", "--", &url]).unwrap();
    let want = &app_lib::engine::svn_xml::parse_log(&xml).unwrap()[0];
    assert_eq!(
        (e.author.as_str(), e.message.as_str()),
        (want.author.as_str(), want.message.as_str())
    );
    assert_eq!(e.date, want.date);
}

#[test]
fn sixty_revisions_page_by_fifty() {
    let fx = hybrid_with_colleague();
    for i in 0..60 {
        fx.colleague_commits(A, &format!("a1\nc{i}\n"));
    }
    sync(&fx);
    let h = file_history(&fx.wc_str(), A, &["HEAD".to_string()])
        .unwrap()
        .entries;
    let groups = groups_of(&h);
    let p1 = svn_file_history(&fx.wc_str(), &groups, &[], None).unwrap();
    assert_eq!(p1.groups.values().map(Vec::len).sum::<usize>(), 50);
    let p2 = svn_file_history(&fx.wc_str(), &groups, &[], p1.cursor.clone()).unwrap();
    assert_eq!(p2.groups[&h[0].id].len(), 10);
    assert_eq!(
        p2.groups[&init_id(&h)]
            .iter()
            .map(|e| e.rev)
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert!(p2.cursor.is_none());
}

#[test]
fn delivered_rev_is_not_listed_and_colleague_rev_right_after_is() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a1\nmine\n");
    commit(&fx, "mine");
    deliver(&fx, "deliver mine");
    let n = server_head(&fx);
    fx.colleague_commits(A, "a1\nmine\ncolleague\n");
    sync(&fx);
    let (m, _) = expand_all(&fx, A, "HEAD");
    let all: Vec<u64> = m.values().flatten().copied().collect();
    assert!(!all.contains(&n), "交付的 r{n} 不该再列一次：{m:?}");
    assert!(all.contains(&(n + 1)), "{m:?}");
}

#[test]
fn opening_an_old_commit_hides_later_revisions() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a1\nx\n");
    sync(&fx);
    git(&fx, &["branch", "old"]);
    fx.colleague_commits(A, "a1\nx\ny\n");
    sync(&fx);
    let y = server_head(&fx);
    let (m, _) = expand_all(&fx, A, "old");
    let all: Vec<u64> = m.values().flatten().copied().collect();
    assert!(all.contains(&(y - 1)) && !all.contains(&y), "{m:?}");
}

#[test]
fn svn_mv_history_is_followed_in_one_segment() {
    let fx = hybrid_with_colleague();
    colleague(&fx, &["update"]);
    colleague(&fx, &["mv", "Code/A.cs", "Code/A2.cs"]);
    colleague(&fx, &["commit", "-m", "mv"]);
    sync(&fx);
    let (m, h) = expand_all(&fx, "Code/A2.cs", "HEAD");
    assert_eq!(m[&h[0].id], vec![2]);
    // 内容没变，git --follow 认得出改名，初始化那条也在历史里；SVN 跟着复制历史查到 r1
    assert_eq!(h.len(), 2, "{h:?}");
    assert_eq!(m[&init_id(&h)], vec![1]);
}

#[test]
fn git_rename_delivered_switches_peg() {
    let fx = hybrid_with_colleague();
    git(&fx, &["mv", A, "Code/A2.cs"]);
    git(&fx, &["commit", "-qm", "rename"]);
    deliver(&fx, "deliver rename");
    let n = server_head(&fx);
    fx.colleague_commits("Code/A2.cs", "a1\nlater\n");
    sync(&fx);
    let (m, h) = expand_all(&fx, "Code/A2.cs", "HEAD");
    let init = init_id(&h);
    assert_eq!(h.last().unwrap().path, A, "git --follow 应追到改名前");
    assert_eq!(m[&h[0].id], vec![n + 1]);
    assert_eq!(m[&init], vec![1], "换 peg 到 A.cs@1 后才查得到：{m:?}");
}

#[test]
fn delete_and_recreate_switches_peg() {
    let fx = hybrid_with_colleague();
    colleague(&fx, &["update"]);
    colleague(&fx, &["rm", "Code/A.cs"]);
    colleague(&fx, &["commit", "-m", "rm"]);
    std::fs::write(fx.wc2().join(A), "recreated\n").unwrap();
    colleague(&fx, &["add", "Code/A.cs"]);
    colleague(&fx, &["commit", "-m", "recreate"]);
    sync(&fx);
    let (m, h) = expand_all(&fx, A, "HEAD");
    assert_eq!(m[&h[0].id], vec![3]);
    assert_eq!(m[&init_id(&h)], vec![1]);
}

#[test]
fn a_file_deleted_on_the_server_later_still_expands() {
    let fx = hybrid_with_colleague();
    colleague(&fx, &["update"]);
    colleague(&fx, &["rm", "Code/A.cs"]);
    colleague(&fx, &["commit", "-m", "rm"]);
    // 本地还没更新：peg 停在本地基线的 r1
    let (m, h) = expand_all(&fx, A, "HEAD");
    assert_eq!(m[&init_id(&h)], vec![1]);
}

#[test]
fn a_file_never_in_a_baseline_makes_no_svn_call() {
    // 没有基线组时直接返回：连仓库都不碰，给一个不存在的路径也照样成功
    let page = svn_file_history("Z:/no/such/repo", &[], &[], None).unwrap();
    assert!(page.groups.is_empty() && page.cursor.is_none() && page.error.is_none());
}

#[test]
fn a_path_outside_every_unit_is_skipped_silently() {
    let fx = hybrid_with_colleague();
    let groups = vec![GroupSpec {
        id: "x".to_string(),
        path: "Art/x.psd".to_string(),
        file_rev: 1,
    }];
    let page = svn_file_history(&fx.wc_str(), &groups, &[], None).unwrap();
    assert!(page.groups.is_empty(), "{:?}", page.groups);
    assert!(page.error.is_none() && page.cursor.is_none());
}

#[test]
fn unreachable_server_is_reported_in_error_not_as_err() {
    let fx = hybrid_with_colleague();
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let unit = WcIdentity {
        wc_root: "Code".to_string(),
        url: format!("svn://127.0.0.1:{port}/x"),
        repos_root: format!("svn://127.0.0.1:{port}"),
        wc_root_abspath: None,
        uuid: "u".to_string(),
    };
    let groups = vec![GroupSpec {
        id: "b".to_string(),
        path: A.to_string(),
        file_rev: 1,
    }];
    let page = page_with_units(&fx.wc_str(), &[unit], &groups, &[], None).unwrap();
    assert_eq!(page.error.as_ref().unwrap().code, "svn_unreachable");
    assert!(page.cursor.is_some(), "出错时保留游标，用户可以重试");
}
