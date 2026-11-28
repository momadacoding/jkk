#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 混合仓库的文件历史 / Blame 补全 SVN（计划 2026-11-27-svn-file-history）。
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::diff::{DiffData, DiffViewOptions};
use app_lib::engine::file_history::{blame_file, file_history, DeliveredState, FileHistoryEntry};
use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::create_baseline_commit_with;
use app_lib::engine::svn_basemap::read_base_map;
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_file_history::{
    overlay_with_units, page_with_units, svn_blame_overlay, svn_file_history, svn_rev_file_diff,
    GroupSpec, OverlayResult, OverlayTarget,
};
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
fn a_git_svn_commit_from_another_svn_repository_is_not_a_baseline() {
    // 工作分支历史里可能混着别的 SVN 库导入的 git-svn commit（见 svn_init::last_baseline_in_head）。
    // 当成基线的话，它的 revision 会拿去查当前服务器，列出无关的人（ultrareview 第 2 条）
    let fx = hybrid_with_colleague();
    write(&fx, A, "a1\nold server\n");
    commit(
        &fx,
        "imported\n\ngit-svn-id: https://old.example/svn/proj/trunk@40 \
         0f0e0d0c-0b0a-0908-0706-050403020100",
    );
    let h = history(&fx, A);
    let imported = h.iter().find(|e| e.subject == "imported").unwrap();
    let svn = imported.svn.as_ref().unwrap();
    assert!(svn.baseline.is_none(), "别的 SVN 库的提交不是基线");
    assert_eq!(svn.delivered, Some(DeliveredState::Undelivered));
    // 自己的初始化基线照旧是基线
    assert!(h.last().unwrap().svn.as_ref().unwrap().baseline.is_some());
    let blame = blame_file(&fx.wc_str(), A, None).unwrap();
    assert!(blame.commits[&imported.id].baseline.is_none());
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

// ---------------------------------------------------------------------------
// Task 6：svn_rev_file_diff
// ---------------------------------------------------------------------------

fn lines(d: &DiffData, origin: char) -> Vec<String> {
    d.files
        .iter()
        .flat_map(|f| f.hunks.iter())
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.origin == origin)
        .map(|l| l.content.trim_end_matches(['\r', '\n']).to_string())
        .collect()
}
/// `.git/objects` 底下的文件总数（含 pack）。
fn object_files(fx: &SvnFixture) -> usize {
    fn walk(p: &std::path::Path) -> usize {
        std::fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path())
                } else {
                    1
                }
            })
            .sum()
    }
    walk(&fx.wc().join(".git/objects"))
}

#[test]
fn a_colleague_revision_diff_matches_svn_diff() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a1\nc\n");
    let d = svn_rev_file_diff(&fx.wc_str(), A, 2, 2, &DiffViewOptions::default()).unwrap();
    assert_eq!(d.files.len(), 1);
    assert_eq!(d.files[0].path, A);
    assert_eq!(d.files[0].status, "modified");
    let url = format!("{}/Code/A.cs", svn_fixtures::file_url(&fx.repo()));
    let svn = run_svn(&fx.wc_str(), &["diff", "-c", "2", "--", &url]).unwrap();
    let plus: Vec<String> = svn
        .lines()
        .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
        .map(|l| l[1..].to_string())
        .collect();
    assert_eq!(lines(&d, '+'), plus);
    assert!(lines(&d, '-').is_empty());
}

#[test]
fn expanded_keywords_are_not_shown_as_changed() {
    // `svn cat` 默认展开 $Id$：每个版本的展开结果都不同，diff 会多出一行与这次提交无关的改动，
    // 和 `svn diff -c N`、TortoiseSVN 看到的不一致（ultrareview 第 4 条）
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/K.cs", "$Id$\nk1\n");
    colleague(&fx, &["propset", "svn:keywords", "Id", "Code/K.cs"]);
    colleague(&fx, &["commit", "-m", "keywords"]);
    fx.colleague_commits("Code/K.cs", "$Id$\nk2\n");
    let n = server_head(&fx);
    let d =
        svn_rev_file_diff(&fx.wc_str(), "Code/K.cs", n, n, &DiffViewOptions::default()).unwrap();
    assert_eq!(lines(&d, '-'), vec!["k1"]);
    assert_eq!(lines(&d, '+'), vec!["k2"]);
}

#[test]
fn the_add_revision_shows_the_whole_file_as_added() {
    let fx = hybrid_with_colleague();
    let d = svn_rev_file_diff(&fx.wc_str(), A, 1, 1, &DiffViewOptions::default()).unwrap();
    assert_eq!(d.files[0].status, "added");
    assert_eq!(lines(&d, '+'), vec!["a1"]);
}

#[test]
fn ignore_whitespace_option_applies() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a1   \n");
    let plain = svn_rev_file_diff(&fx.wc_str(), A, 2, 2, &DiffViewOptions::default()).unwrap();
    assert_eq!(lines(&plain, '+').len(), 1);
    let ws = DiffViewOptions {
        ignore_whitespace: true,
        ..Default::default()
    };
    let d = svn_rev_file_diff(&fx.wc_str(), A, 2, 2, &ws).unwrap();
    assert!(
        lines(&d, '+').is_empty() && lines(&d, '-').is_empty(),
        "{d:?}"
    );
}

#[test]
fn binary_content_is_reported_as_binary() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a1\n\0\x01\x02\n");
    let d = svn_rev_file_diff(&fx.wc_str(), A, 2, 2, &DiffViewOptions::default()).unwrap();
    assert!(d.files[0].binary);
}

#[test]
fn no_object_is_written_to_the_users_git() {
    let fx = hybrid_with_colleague();
    // 同事的新内容从没进过本地 git：diff 若往 .git 写 blob，这里一定多出文件
    fx.colleague_commits(A, "a1\nnever seen by git\n");
    let before = object_files(&fx);
    svn_rev_file_diff(&fx.wc_str(), A, 2, 2, &DiffViewOptions::default()).unwrap();
    assert_eq!(object_files(&fx), before);
}

// ---------------------------------------------------------------------------
// Task 7：svn_blame_overlay
// ---------------------------------------------------------------------------

fn target(id: &str, path: &str, rev: u64) -> OverlayTarget {
    OverlayTarget {
        baseline_id: id.to_string(),
        orig_path: path.to_string(),
        file_rev: rev,
    }
}
fn mapped(r: &OverlayResult) -> Vec<Option<u64>> {
    match r {
        OverlayResult::Mapped { lines } => {
            lines.iter().map(|l| l.as_ref().map(|b| b.rev)).collect()
        }
        other => panic!("应当映射成功：{other:?}"),
    }
}

#[test]
fn baseline_lines_map_to_svn_blame() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a1\nc\n");
    sync(&fx);
    let b = blame_file(&fx.wc_str(), A, None).unwrap();
    let line2 = &b.lines[1].commit;
    let info = b.commits[line2]
        .baseline
        .as_ref()
        .expect("第 2 行归更新录的基线");
    let line1 = &b.lines[0].commit;
    let r = svn_blame_overlay(
        &fx.wc_str(),
        &[target(line2, A, info.file_rev), target(line1, A, 1)],
    )
    .unwrap();
    assert_eq!(mapped(&r[0]), vec![Some(1), Some(2)]);
    assert_eq!(mapped(&r[1]), vec![Some(1)]);
    // 作者与 svn log 一致
    let url = format!("{}/Code/A.cs", svn_fixtures::file_url(&fx.repo()));
    let xml = run_svn(&fx.wc_str(), &["log", "--xml", "-r", "2", "--", &url]).unwrap();
    let want = &app_lib::engine::svn_xml::parse_log(&xml).unwrap()[0];
    let OverlayResult::Mapped { lines } = &r[0] else {
        unreachable!()
    };
    assert_eq!(lines[1].as_ref().unwrap().author, want.author);
}

#[test]
fn line_count_mismatch_is_reported() {
    let fx = hybrid_with_colleague();
    let repo = fx.wc_str();
    // 一条记账说 A.cs 在 r1、树里却是三行的假基线：SVN 那边 A.cs@1 只有一行
    let map = read_base_map(&repo, "svn/base").unwrap();
    write(&fx, A, "x\ny\nz\n");
    git(&fx, &["add", A]);
    let tree = git(&fx, &["write-tree"]);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let oid = create_baseline_commit_with(&repo, &tree, Some(&head), &map, &[], None).unwrap();
    let r = svn_blame_overlay(&repo, &[target(&oid, A, 1)]).unwrap();
    match &r[0] {
        OverlayResult::Mismatch {
            svn_lines,
            git_lines,
        } => assert_eq!((*svn_lines, *git_lines), (1, 3)),
        other => panic!("{other:?}"),
    }
}

#[test]
fn keywords_and_crlf_files_still_map() {
    let fx = hybrid_with_colleague();
    colleague(&fx, &["update"]);
    std::fs::write(fx.wc2().join(A), "a1\n// $Id$\nz\n").unwrap();
    colleague(&fx, &["propset", "svn:keywords", "Id", "Code/A.cs"]);
    colleague(&fx, &["propset", "svn:eol-style", "CRLF", "Code/A.cs"]);
    colleague(&fx, &["commit", "-m", "kw"]);
    sync(&fx);
    let b = blame_file(&fx.wc_str(), A, None).unwrap();
    let line2 = &b.lines[1].commit;
    let rev = b.commits[line2].baseline.as_ref().unwrap().file_rev;
    let r = svn_blame_overlay(&fx.wc_str(), &[target(line2, A, rev)]).unwrap();
    // 以 svn 自己的 blame 为准：改 eol-style 那次 svn 把第 1 行也算成 r2（2026-11-28 实测）
    let url = format!("{}/Code/A.cs@{rev}", svn_fixtures::file_url(&fx.repo()));
    let xml = run_svn(&fx.wc_str(), &["blame", "--xml", "--", &url]).unwrap();
    let want: Vec<Option<u64>> = app_lib::engine::svn_xml::parse_blame(&xml)
        .unwrap()
        .iter()
        .map(|l| l.as_ref().map(|b| b.rev))
        .collect();
    assert_eq!(want.len(), 3);
    assert_eq!(mapped(&r[0]), want);
}

#[test]
fn more_than_eight_targets_are_skipped() {
    let fx = hybrid_with_colleague();
    let b = blame_file(&fx.wc_str(), A, None).unwrap();
    let id = b.lines[0].commit.clone();
    let targets: Vec<OverlayTarget> = (0..9).map(|_| target(&id, A, 1)).collect();
    let r = svn_blame_overlay(&fx.wc_str(), &targets).unwrap();
    assert_eq!(r.len(), 9);
    assert!(r[..8]
        .iter()
        .all(|x| matches!(x, OverlayResult::Mapped { .. })));
    assert!(matches!(r[8], OverlayResult::Skipped));
}

#[test]
fn an_unreachable_server_fails_only_that_target() {
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
    let b = blame_file(&fx.wc_str(), A, None).unwrap();
    let id = b.lines[0].commit.clone();
    let r = overlay_with_units(&fx.wc_str(), &[unit], &[target(&id, A, 1)]).unwrap();
    match &r[0] {
        OverlayResult::Failed { error } => assert_eq!(error.code, "svn_unreachable"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_unreachable_unit_is_asked_once_not_once_per_target_and_reload() {
    // 连不上时，同一单元剩下的目标不再逐个等超时；Blame 重读（每次保存都会）也不再去连
    // （ultrareview 第 3、5 条）。假服务器接下连接就断开，数它被连了几次
    let fx = hybrid_with_colleague();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for c in l.incoming() {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(c);
        }
    });
    let unit = WcIdentity {
        wc_root: "Code".to_string(),
        url: format!("svn://127.0.0.1:{port}/x"),
        repos_root: format!("svn://127.0.0.1:{port}"),
        wc_root_abspath: None,
        uuid: format!("drop-{port}"),
    };
    let b = blame_file(&fx.wc_str(), A, None).unwrap();
    let id = b.lines[0].commit.clone();
    let targets = [target(&id, A, 1), target(&id, "Code/B.cs", 1)];
    for _ in 0..2 {
        let r = overlay_with_units(&fx.wc_str(), std::slice::from_ref(&unit), &targets).unwrap();
        for x in &r {
            match x {
                OverlayResult::Failed { error } => {
                    assert_eq!(error.code, "svn_unreachable", "{}", error.message)
                }
                other => panic!("{other:?}"),
            }
        }
    }
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

// ---------------------------------------------------------------------------
// 独立 review 第 C1、C2 条：文件不在 r1 创建、删除后用复制补回原路径
// ---------------------------------------------------------------------------

/// 纯 SVN 仓库（`svn_repo`：r1 有 Code/A.cs、Code/B.cs），配一个指向 `Code` 的单元。
fn plain_svn() -> (SvnFixture, WcIdentity) {
    let fx = svn_fixtures::svn_repo();
    let unit = WcIdentity {
        wc_root: "Code".to_string(),
        url: format!("{}/Code", svn_fixtures::file_url(&fx.repo())),
        repos_root: svn_fixtures::file_url(&fx.repo()),
        wc_root_abspath: None,
        uuid: "u".to_string(),
    };
    (fx, unit)
}
fn svn_wc(fx: &SvnFixture, args: &[&str]) {
    let mut full = args.to_vec();
    full.push("--non-interactive");
    svn_fixtures::svn_cmd(&fx.wc(), &full);
}
fn edit_commit(fx: &SvnFixture, rel: &str, content: &str, msg: &str) {
    let p = fx.wc().join(rel);
    let new = !p.exists();
    std::fs::write(&p, content).unwrap();
    if new {
        svn_wc(fx, &["add", rel]);
    }
    svn_wc(fx, &["commit", "-m", msg]);
    svn_wc(fx, &["update"]);
}
fn spec(id: &str, path: &str, rev: u64) -> GroupSpec {
    GroupSpec {
        id: id.to_string(),
        path: path.to_string(),
        file_rev: rev,
    }
}
fn revs(page: &app_lib::engine::svn_file_history::SvnHistoryPage, id: &str) -> Vec<u64> {
    page.groups
        .get(id)
        .map(|v| v.iter().map(|e| e.rev).collect())
        .unwrap_or_default()
}

#[test]
fn the_add_revision_after_r1_shows_the_whole_file_as_added() {
    // 真实仓库里文件几乎都不在 r1 创建：旧侧 `svn cat -r <创建版 - 1>` 报 E195012
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/N.cs", "n1\nn2\n");
    let d =
        svn_rev_file_diff(&fx.wc_str(), "Code/N.cs", 2, 2, &DiffViewOptions::default()).unwrap();
    assert_eq!(d.files[0].status, "added");
    assert_eq!(lines(&d, '+'), vec!["n1", "n2"]);
}

#[test]
fn a_full_page_ending_at_the_creation_revision_moves_on_to_older_groups() {
    let (fx, unit) = plain_svn();
    edit_commit(&fx, "Code/B.cs", "b2\n", "r2"); // r2：N 还不存在
    for i in 0..50 {
        edit_commit(&fx, "Code/N.cs", &format!("n{i}\n"), &format!("n{i}")); // r3..r52
    }
    let groups = vec![spec("new", "Code/N.cs", 52), spec("old", "Code/A.cs", 1)];
    let p1 = page_with_units(
        &fx.wc_str(),
        std::slice::from_ref(&unit),
        &groups,
        &[],
        None,
    )
    .unwrap();
    assert_eq!(revs(&p1, "new").len(), 50);
    assert!(p1.error.is_none(), "{:?}", p1.error);
    let p2 = page_with_units(&fx.wc_str(), &[unit], &groups, &[], p1.cursor).unwrap();
    assert!(p2.error.is_none(), "翻到创建版之前不该报错：{:?}", p2.error);
    assert_eq!(revs(&p2, "old"), vec![1]);
    assert!(p2.cursor.is_none());
}

#[test]
fn a_path_replaced_by_a_copy_keeps_each_baseline_on_its_own_lineage() {
    // B 在 r1、r2；X 在 r3 新增、r4 修改、r5 删除；r6 把 B 复制成 X。
    // `svn log X@6` 跟着复制历史列出 r6、r2、r1——那是 B 的提交，不能算到旧 X 的基线上
    let (fx, unit) = plain_svn();
    edit_commit(&fx, "Code/B.cs", "b2\n", "r2");
    edit_commit(&fx, "Code/X.cs", "x1\n", "r3");
    edit_commit(&fx, "Code/X.cs", "x2\n", "r4");
    svn_wc(&fx, &["rm", "Code/X.cs"]);
    svn_wc(&fx, &["commit", "-m", "r5"]);
    svn_wc(&fx, &["update"]);
    svn_wc(&fx, &["cp", "Code/B.cs", "Code/X.cs"]);
    svn_wc(&fx, &["commit", "-m", "r6"]);
    let groups = vec![spec("b1", "Code/X.cs", 6), spec("b2", "Code/X.cs", 4)];
    let mut got: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    let mut cursor = None;
    for _ in 0..5 {
        let p = page_with_units(
            &fx.wc_str(),
            std::slice::from_ref(&unit),
            &groups,
            &[],
            cursor,
        )
        .unwrap();
        assert!(p.error.is_none(), "{:?}", p.error);
        for (id, v) in &p.groups {
            got.entry(id.clone())
                .or_default()
                .extend(v.iter().map(|e| e.rev));
        }
        cursor = p.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(got.get("b1").cloned().unwrap_or_default(), vec![6]);
    assert_eq!(got.get("b2").cloned().unwrap_or_default(), vec![4, 3]);
}
