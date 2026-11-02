//! 收件箱读取、配对与清理（计划 Task 3；日志 H3、H4）。

use app_lib::engine::tsvn_inbox::{
    inbox_dir, prune_inbox, read_evidence, write_event, TsvnEvent, EVENT_TTL_MS, MAX_EVENTS,
    ORPHAN_PRE_TTL_MS, PAIR_WINDOW_MS,
};
use std::collections::BTreeSet;
use std::fs;
use tempfile::TempDir;

struct Fx {
    _dir: TempDir,
    repo: String,
}

fn fx() -> Fx {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    Fx {
        repo: repo.display().to_string(),
        _dir: dir,
    }
}

fn ev(kind: &str, at_ms: i64, paths: &[&str], revision: Option<u64>) -> TsvnEvent {
    TsvnEvent {
        version: 1,
        kind: kind.to_string(),
        at_ms,
        paths: paths.iter().map(|p| p.to_string()).collect(),
        revision,
        error: None,
    }
}

fn put(fx: &Fx, e: TsvnEvent) {
    write_event(&fx.repo, &e).unwrap();
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

fn remaining(fx: &Fx) -> Vec<(String, i64)> {
    let mut v: Vec<(String, i64)> = fs::read_dir(inbox_dir(&fx.repo))
        .unwrap()
        .filter_map(|e| {
            let b = fs::read(e.unwrap().path()).ok()?;
            let e: TsvnEvent = serde_json::from_slice(&b).ok()?;
            Some((e.kind, e.at_ms))
        })
        .collect();
    v.sort_by_key(|x| x.1);
    v
}

#[test]
fn empty_or_missing_inbox_is_empty_evidence() {
    let fx = fx();
    let e = read_evidence(&fx.repo);
    assert!(e.commits.is_empty() && e.updates.is_empty());
}

#[test]
fn post_pairs_with_the_latest_preceding_pre_of_the_same_paths() {
    let fx = fx();
    put(&fx, ev("pre_commit", 1_000, &["Code/A.cs"], None));
    put(&fx, ev("pre_commit", 5_000, &["Code/A.cs"], None));
    put(&fx, ev("post_commit", 5_400, &["Code/A.cs"], Some(7)));
    // 晚于 Post 的 Pre 不算
    put(&fx, ev("pre_commit", 6_000, &["Code/A.cs"], None));
    let e = read_evidence(&fx.repo);
    assert_eq!(e.commits.len(), 1);
    let c = &e.commits[0];
    assert_eq!((c.t_commit_ms, c.revision), (Some(5_000), 7));
    assert_eq!(c.paths, set(&["Code/A.cs"]));
}

#[test]
fn orphan_pre_from_a_failed_commit_does_not_pair_with_a_later_post() {
    let fx = fx();
    // 过期失败只留下 Pre（H4），几天后同一路径又提交了一次，那次的 Pre 没记上
    put(&fx, ev("pre_commit", 1_000, &["Code/A.cs"], None));
    put(
        &fx,
        ev(
            "post_commit",
            1_000 + PAIR_WINDOW_MS + 1,
            &["Code/A.cs"],
            Some(9),
        ),
    );
    let e = read_evidence(&fx.repo);
    assert_eq!(e.commits.len(), 1);
    assert_eq!(e.commits[0].t_commit_ms, None);
}

#[test]
fn pre_with_different_paths_does_not_pair() {
    let fx = fx();
    put(
        &fx,
        ev("pre_commit", 1_000, &["Code/A.cs", "Code/B.cs"], None),
    );
    put(&fx, ev("post_commit", 1_200, &["Code/A.cs"], Some(3)));
    assert_eq!(read_evidence(&fx.repo).commits[0].t_commit_ms, None);
}

#[test]
fn post_with_error_is_not_commit_evidence() {
    let fx = fx();
    put(&fx, ev("pre_commit", 1_000, &["Code/A.cs"], None));
    let mut post = ev("post_commit", 1_200, &["Code/A.cs"], Some(3));
    post.error = Some("E155011".into());
    put(&fx, post);
    assert!(read_evidence(&fx.repo).commits.is_empty());
}

#[test]
fn post_without_revision_is_not_commit_evidence() {
    let fx = fx();
    put(&fx, ev("post_commit", 1_200, &["Code/A.cs"], None));
    assert!(read_evidence(&fx.repo).commits.is_empty());
}

#[test]
fn post_update_becomes_update_evidence() {
    let fx = fx();
    put(
        &fx,
        ev("post_update", 2_000, &["Code/A.cs", "Code/B.cs"], Some(5)),
    );
    let e = read_evidence(&fx.repo);
    assert!(e.commits.is_empty());
    assert_eq!(e.updates.len(), 1);
    assert_eq!(e.updates[0].revision, 5);
    assert_eq!(e.updates[0].paths, set(&["Code/A.cs", "Code/B.cs"]));
}

#[test]
fn check_commit_events_are_ignored_by_evidence() {
    let fx = fx();
    put(&fx, ev("check_commit", 1_000, &["Code/A.cs"], None));
    let e = read_evidence(&fx.repo);
    assert!(e.commits.is_empty() && e.updates.is_empty());
}

#[test]
fn garbage_file_in_inbox_is_skipped() {
    let fx = fx();
    put(&fx, ev("post_commit", 1_200, &["Code/A.cs"], Some(3)));
    fs::write(
        inbox_dir(&fx.repo).join("1-1-post_commit.json"),
        b"{not json",
    )
    .unwrap();
    fs::write(
        inbox_dir(&fx.repo).join("2-1-post_commit.json.tmp"),
        b"half",
    )
    .unwrap();
    assert_eq!(read_evidence(&fx.repo).commits.len(), 1);
}

#[test]
fn prune_removes_processed_posts_with_their_pres_and_keeps_the_rest() {
    let fx = fx();
    put(&fx, ev("pre_commit", 1_000, &["Code/A.cs"], None));
    put(&fx, ev("post_commit", 1_100, &["Code/A.cs"], Some(3)));
    put(&fx, ev("pre_commit", 2_000, &["Code/B.cs"], None));
    put(&fx, ev("post_commit", 2_100, &["Code/B.cs"], Some(4)));
    put(&fx, ev("post_update", 3_000, &["Code/C.cs"], Some(5)));
    let now = 4_000;
    prune_inbox(&fx.repo, &|e| e.revision == Some(3), now);
    assert_eq!(
        remaining(&fx),
        vec![
            ("pre_commit".to_string(), 2_000),
            ("post_commit".to_string(), 2_100),
            ("post_update".to_string(), 3_000),
        ]
    );
}

#[test]
fn prune_removes_old_orphan_pres_but_keeps_young_ones() {
    let fx = fx();
    let now = 10 * ORPHAN_PRE_TTL_MS;
    put(
        &fx,
        ev(
            "pre_commit",
            now - ORPHAN_PRE_TTL_MS - 1,
            &["Code/A.cs"],
            None,
        ),
    );
    put(&fx, ev("pre_commit", now - 1_000, &["Code/A.cs"], None));
    prune_inbox(&fx.repo, &|_| false, now);
    assert_eq!(
        remaining(&fx),
        vec![("pre_commit".to_string(), now - 1_000)]
    );
}

#[test]
fn prune_removes_unprocessed_events_past_ttl() {
    let fx = fx();
    let now = 2 * EVENT_TTL_MS;
    put(
        &fx,
        ev(
            "post_commit",
            now - EVENT_TTL_MS - 1,
            &["Code/A.cs"],
            Some(3),
        ),
    );
    put(&fx, ev("post_commit", now - 1, &["Code/B.cs"], Some(4)));
    prune_inbox(&fx.repo, &|_| false, now);
    assert_eq!(remaining(&fx), vec![("post_commit".to_string(), now - 1)]);
}

#[test]
fn prune_caps_count_by_dropping_the_oldest() {
    let fx = fx();
    let n = MAX_EVENTS as i64 + 3;
    for i in 0..n {
        put(&fx, ev("post_update", 1_000 + i, &["Code/A.cs"], Some(5)));
    }
    prune_inbox(&fx.repo, &|_| false, 1_000 + n);
    let left = remaining(&fx);
    assert_eq!(left.len(), MAX_EVENTS);
    assert_eq!(left[0].1, 1_003);
}

#[test]
fn prune_on_missing_inbox_does_nothing() {
    let fx = fx();
    prune_inbox(&fx.repo, &|_| true, 1);
    assert!(!inbox_dir(&fx.repo).exists());
}
