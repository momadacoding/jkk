#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 体检的本地 floor：形状规整的单元不发 floor `svn diff`，由 wc.db + `svn status` 回答
//! （日志 `docs/superpowers/plans/2026-11-17-floor-check-by-status-decisions.md`）。
//!
//! 每个场景同一个工作副本跑两遍体检：一遍注入 `FloorShapeRefused`，全部照旧问服务器；一遍放开。
//! 断言两遍的结论**逐项相同**，并断言放开那遍走没走本地（`floor_answered_locally_last`）——
//! 光看结论相同分不出本地那条路有没有被走到，规整的场景必须真的走了本地，不规整的必须没走。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_basemap::BaseMap;
use app_lib::engine::svn_health::{
    check_baseline, check_baseline_map, floor_answered_locally_last, ExplainedOrigin, Health,
    HealthMap,
};
use app_lib::engine::svn_nodes::{compute_map, read_node_status};
use app_lib::engine::svn_scope::WScope;
use app_lib::engine::svn_tombstone::fault::{self, Point};
use app_lib::engine::svn_wc::enumerate_wcs;
use app_lib::engine::svn_wcdb::{self, FloorShape};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use svn_fixtures::{
    file_url, svn_cmd, svn_repo_committed_without_update, svn_repo_for_health,
    svn_repo_mixed_revision, svn_repo_obstructed, svn_repo_with_native_eol,
    svn_repo_with_same_repo_nested_external, svn_repo_with_switched_subtree, NestedExternal,
    SvnFixture,
};

fn wl(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn scope(whitelist: &[&str]) -> WScope {
    WScope::plain(&wl(whitelist), &[".cs".to_string()])
}

/// 照录制的口径算映射（同 `tests/svn_health.rs` 的 `map_of`）：键 = 磁盘上带服务器 revision 的 `.cs`。
fn map_of(fx: &SvnFixture, whitelist: &[&str]) -> BaseMap {
    let wc = fx.wc_str();
    let us = enumerate_wcs(&wc, &wl(whitelist)).unwrap();
    let nodes = read_node_status(&wc, &wl(whitelist), &us).unwrap();
    let files: BTreeSet<String> = nodes
        .iter()
        .filter(|n| {
            n.node_rev.is_some() && n.path.ends_with(".cs") && fx.wc().join(&n.path).is_file()
        })
        .map(|n| n.path.clone())
        .collect();
    compute_map(
        &us[0].repos_root,
        &us[0].uuid,
        &files,
        &nodes,
        BTreeMap::new(),
    )
    .unwrap()
}

/// 两遍体检，返回（结论，放开那遍由本地回答的单元）。
fn both(
    fx: &SvnFixture,
    url: &str,
    whitelist: &[&str],
    map: &BaseMap,
    explained: &BTreeSet<String>,
    origin: ExplainedOrigin,
) -> (HealthMap, Vec<String>) {
    let sc = scope(whitelist);
    let run = || check_baseline_map(&fx.wc_str(), url, map, &sc, explained, origin).unwrap();
    let server = {
        let _refuse = fault::arm(Point::FloorShapeRefused);
        let h = run();
        assert!(
            floor_answered_locally_last().is_empty(),
            "注入之后不该走本地"
        );
        h
    };
    let local = run();
    let used = floor_answered_locally_last();
    assert_eq!(
        wording_aside(&local),
        wording_aside(&server),
        "本地 floor 与 svn diff 的结论不同（本地单元：{used:?}）"
    );
    (local, used)
}

/// 失败原因的措辞两条路不同（「连不上服务器」对「比对 `Code` 时失败」），其余逐项比。
fn wording_aside(h: &HealthMap) -> HealthMap {
    match h.clone() {
        HealthMap::Verified {
            health: Health::NotChecked { .. },
            new_gone,
        } => HealthMap::Verified {
            health: Health::NotChecked {
                reason: String::new(),
            },
            new_gone,
        },
        HealthMap::Unconfirmed { files, .. } => HealthMap::Unconfirmed {
            files,
            reason: String::new(),
        },
        other => other,
    }
}

fn url(fx: &SvnFixture) -> String {
    file_url(&fx.repo())
}

/// 白名单 `Code/`、S = ∅、`Unknown`，返回结论与本地单元。
fn code(fx: &SvnFixture) -> (HealthMap, Vec<String>) {
    let map = map_of(fx, &["Code/"]);
    both(
        fx,
        &url(fx),
        &["Code/"],
        &map,
        &BTreeSet::new(),
        ExplainedOrigin::Unknown,
    )
}

fn health(h: HealthMap) -> Health {
    match h {
        HealthMap::Verified { health, .. } => health,
        other => panic!("期望 Verified 分支，实际 {other:?}"),
    }
}

fn is_verified(h: HealthMap) -> bool {
    matches!(health(h), Health::Verified { .. })
}

fn lying(h: HealthMap) -> Vec<String> {
    match health(h) {
        Health::Lying { files } => files,
        other => panic!("期望 Lying，实际 {other:?}"),
    }
}

fn local() -> Vec<String> {
    vec!["Code".to_string()]
}

fn none() -> Vec<String> {
    Vec::new()
}

/// `Code/` 单元在 wc.db 里为什么不规整。「没走本地」光看结论分不出是不是因为别的原因（读不了
/// wc.db 之类），这里钉住原因。
fn irregular_because(fx: &SvnFixture, floor: u64) -> String {
    let us = enumerate_wcs(&fx.wc_str(), &wl(&["Code/"])).unwrap();
    match svn_wcdb::floor_shapes(&fx.wc_str(), &us, floor)
        .unwrap()
        .remove("Code")
        .unwrap()
    {
        FloorShape::Irregular(why) => why,
        regular => panic!("期望不规整，实际 {regular:?}"),
    }
}

/// `--non-interactive` 紧跟在子命令后面：放到末尾会落在 `--` 之后、被当成路径。
fn svn(wc: &Path, args: &[&str]) {
    let mut a = args.to_vec();
    a.insert(1, "--non-interactive");
    svn_cmd(wc, &a);
}

// ---------------------------------------------------------------------------
// 规整：必须走本地
// ---------------------------------------------------------------------------

#[test]
fn a_clean_working_copy() {
    let fx = svn_repo_for_health();
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_modified_w_file_is_reported() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "mine\n").unwrap();
    let (h, used) = code(&fx);
    assert_eq!(lying(h), vec!["Code/A.cs".to_string()]);
    assert_eq!(used, local());
}

#[test]
fn a_modified_file_that_is_not_a_w_file_is_ignored() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/Art.psd"), "p-mine\n").unwrap();
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_modified_file_explained_by_the_recording_is_not_lying() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "mine\n").unwrap();
    let map = map_of(&fx, &["Code/"]);
    let s: BTreeSet<String> = ["Code/A.cs".to_string()].into();
    let (h, used) = both(
        &fx,
        &url(&fx),
        &["Code/"],
        &map,
        &s,
        ExplainedOrigin::JustRecorded,
    );
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_same_size_edit_that_kept_its_mtime_is_missed_by_both() {
    // T1 实测：内容变了、时间与大小没变，svn 两条命令都不读内容。这里钉住「两边一样」，
    // 不是说这是对的——它是 svn 自己的判据
    let fx = svn_repo_for_health();
    let p = fx.wc().join("Code/A.cs");
    let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
    std::fs::write(&p, "v9\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_property_only_change_is_ignored() {
    let fx = svn_repo_for_health();
    svn(&fx.wc(), &["propset", "foo", "bar", "Code/A.cs"]);
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn an_unversioned_w_file_is_not_reported() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/New.cs"), "n\n").unwrap();
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_missing_file_is_not_reported() {
    let fx = svn_repo_for_health();
    std::fs::remove_file(fx.wc().join("Code/Deep/D.cs")).unwrap();
    let map = map_of(&fx, &["Code/"]);
    let (h, used) = both(
        &fx,
        &url(&fx),
        &["Code/"],
        &map,
        &BTreeSet::new(),
        ExplainedOrigin::Unknown,
    );
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn after_a_colleague_commit_and_an_update() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "v2\n").unwrap();
    svn(&fx.wc(), &["commit", "-m", "r2"]);
    svn(&fx.wc(), &["update"]);
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_w_file_committed_without_update_is_left_to_its_own_group() {
    // 文件超前（n = c = r4 > floor r3）：它在映射里单列，floor 组的结果本来就不算它
    let fx = svn_repo_committed_without_update();
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_non_w_file_committed_without_update_is_ignored() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/Art.psd"), "p2\n").unwrap();
    svn(&fx.wc(), &["commit", "-m", "r2"]);
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn a_whitelist_entry_below_the_working_copy_root() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/Deep/D.cs"), "mine\n").unwrap();
    let map = map_of(&fx, &["Code/Deep/"]);
    let (h, used) = both(
        &fx,
        &url(&fx),
        &["Code/Deep/"],
        &map,
        &BTreeSet::new(),
        ExplainedOrigin::Unknown,
    );
    assert_eq!(lying(h), vec!["Code/Deep/D.cs".to_string()]);
    assert_eq!(used, vec!["Code/Deep".to_string()]);
}

#[test]
fn native_eol_files() {
    let fx = svn_repo_with_native_eol();
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, local());
}

#[test]
fn an_excluded_node_is_still_checked_by_name() {
    // 被排除的节点整单元 diff 本来就不报，体检另外点名比；本地 floor 跳过它们，点名照旧
    let fx = svn_repo_for_health();
    svn(
        &fx.wc(),
        &["update", "--set-depth=exclude", "--", "Code/Deep@"],
    );
    let map = map_of(&fx, &["Code/"]);
    let (h, used) = both(
        &fx,
        &url(&fx),
        &["Code/"],
        &map,
        &BTreeSet::new(),
        ExplainedOrigin::JustRecorded,
    );
    // 服务器 HEAD 上还有 Code/Deep/D.cs：漏收
    assert!(matches!(h, HealthMap::Excluded { .. }), "{h:?}");
    assert_eq!(used, local());
}

#[test]
fn a_nested_external_is_its_own_unit() {
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    let (h, used) = code(&fx);
    assert!(is_verified(h));
    assert_eq!(used, vec!["Code".to_string(), "Code/Vendor".to_string()]);
}

#[test]
fn a_dirty_nested_external() {
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Dirty);
    let (h, used) = code(&fx);
    assert!(!lying(h).is_empty());
    assert_eq!(used, vec!["Code".to_string(), "Code/Vendor".to_string()]);
}

#[test]
fn a_switched_whitelist_root_is_refused_before_any_comparison() {
    // 白名单根整个 switch 过：比对之前的那道核对（whitelist_not_under_base）就判 NotChecked，
    // 两条路都走不到 floor
    let fx = svn_repo_with_switched_subtree();
    let base = format!("{}/trunk", url(&fx));
    let map = map_of(&fx, &["Code/"]);
    let (h, used) = both(
        &fx,
        &base,
        &["Code/"],
        &map,
        &BTreeSet::new(),
        ExplainedOrigin::Unknown,
    );
    assert!(matches!(health(h), Health::NotChecked { .. }));
    assert_eq!(used, none());
}

// ---------------------------------------------------------------------------
// 不规整：必须照旧问服务器
// ---------------------------------------------------------------------------

#[test]
fn a_stale_baseline_rev_goes_to_the_server() {
    // 用户手动体检、基线停在 r1、工作副本在 r2：A.cs 在 r2 改过（c > F），只有服务器知道 r1 上是什么
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "v2\n").unwrap();
    svn(&fx.wc(), &["commit", "-m", "r2"]);
    svn(&fx.wc(), &["update"]);
    let sc = scope(&["Code/"]);
    let run = || {
        check_baseline(
            &fx.wc_str(),
            &url(&fx),
            1,
            &sc,
            &BTreeSet::new(),
            ExplainedOrigin::Unknown,
        )
        .unwrap()
    };
    let server = {
        let _refuse = fault::arm(Point::FloorShapeRefused);
        run()
    };
    let local_run = run();
    assert_eq!(local_run, server);
    assert_eq!(
        local_run,
        Health::Lying {
            files: vec!["Code/A.cs".to_string()]
        }
    );
    assert_eq!(floor_answered_locally_last(), none());
    assert!(irregular_because(&fx, 1).contains("目录 `Code`"));
}

#[test]
fn a_file_behind_the_rest_goes_to_the_server() {
    let fx = svn_repo_mixed_revision();
    let (_, used) = code(&fx);
    assert_eq!(used, none());
    let floor = map_of(&fx, &["Code/"]).floor;
    assert!(irregular_because(&fx, floor).contains("目录"));
}

#[test]
fn a_scheduled_delete_goes_to_the_server() {
    let fx = svn_repo_for_health();
    svn(&fx.wc(), &["delete", "Code/Deep/D.cs"]);
    let (h, used) = code(&fx);
    assert_eq!(lying(h), vec!["Code/Deep/D.cs".to_string()]);
    assert_eq!(used, none());
    assert!(irregular_because(&fx, 1).contains("增删"));
}

#[test]
fn a_scheduled_add_goes_to_the_server() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/New.cs"), "n\n").unwrap();
    svn(&fx.wc(), &["add", "Code/New.cs"]);
    let (h, used) = code(&fx);
    assert_eq!(lying(h), vec!["Code/New.cs".to_string()]);
    assert_eq!(used, none());
    assert!(irregular_because(&fx, 1).contains("增删"));
}

#[test]
fn a_committed_delete_without_update_goes_to_the_server() {
    // not-present：父目录还在 r1，F 上有 D.cs、工作副本里没有
    let fx = svn_repo_for_health();
    svn(&fx.wc(), &["delete", "Code/Deep/D.cs"]);
    svn(&fx.wc(), &["commit", "-m", "r2"]);
    let (_, used) = code(&fx);
    assert_eq!(used, none());
    assert!(irregular_because(&fx, 1).contains("not-present"));
}

#[test]
fn a_sparse_directory_goes_to_the_server() {
    let fx = svn_repo_for_health();
    svn(
        &fx.wc(),
        &["update", "--set-depth=empty", "--", "Code/Deep@"],
    );
    let (_, used) = code(&fx);
    assert_eq!(used, none());
    assert!(irregular_because(&fx, 1).contains("稀疏"));
}

#[test]
fn a_switched_subdirectory_goes_to_the_server() {
    let fx = svn_repo_for_health();
    let wc = fx.wc();
    let deep = format!("{}/Code/Deep", url(&fx));
    let other = format!("{}/Other", url(&fx));
    svn(&wc, &["copy", &deep, &other, "-m", "r2"]);
    svn(&wc, &["update"]);
    svn(&wc, &["switch", &other, "Code/Deep"]);
    let (_, used) = code(&fx);
    assert_eq!(used, none());
    let floor = map_of(&fx, &["Code/"]).floor;
    assert!(irregular_because(&fx, floor).contains("switch"));
}

#[test]
fn a_text_conflict_goes_to_the_server() {
    let fx = svn_repo_for_health();
    let wc = fx.wc();
    std::fs::write(wc.join("Code/A.cs"), "v2\n").unwrap();
    svn(&wc, &["commit", "-m", "r2"]);
    svn(&wc, &["update", "-r1", "Code/A.cs"]);
    std::fs::write(wc.join("Code/A.cs"), "mine\n").unwrap();
    svn(&wc, &["update", "--accept", "postpone"]);
    let (_, used) = code(&fx);
    assert_eq!(used, none());
    let floor = map_of(&fx, &["Code/"]).floor;
    assert!(irregular_because(&fx, floor).contains("冲突"));
}

#[test]
fn an_obstructed_file_goes_to_the_server() {
    let fx = svn_repo_obstructed();
    let map = map_of(&fx, &["Code/"]);
    let sc = scope(&["Code/"]);
    let run = || {
        check_baseline_map(
            &fx.wc_str(),
            &url(&fx),
            &map,
            &sc,
            &BTreeSet::new(),
            ExplainedOrigin::Unknown,
        )
        .map_err(|e| e.code)
    };
    let server = {
        let _refuse = fault::arm(Point::FloorShapeRefused);
        run()
    };
    assert_eq!(run(), server);
    assert_eq!(floor_answered_locally_last(), none());
    // wc.db 里它还是个正常的文件节点，是 status 的 `obstructed` 让这个单元改回 diff
    let us = enumerate_wcs(&fx.wc_str(), &wl(&["Code/"])).unwrap();
    let shapes = svn_wcdb::floor_shapes(&fx.wc_str(), &us, map.floor).unwrap();
    assert!(
        matches!(shapes.get("Code"), Some(FloorShape::Regular { .. })),
        "{shapes:?}"
    );
}

// ---------------------------------------------------------------------------
// 连不上服务器：结论与原来相同（NotChecked），本地 floor 不冒充「验过」
// ---------------------------------------------------------------------------

#[test]
fn an_unreachable_server_is_not_checked_either_way() {
    let fx = svn_repo_for_health();
    let map = map_of(&fx, &["Code/"]);
    std::fs::rename(fx.repo(), fx.path().join("repo-moved")).unwrap();
    for origin in [ExplainedOrigin::Unknown, ExplainedOrigin::JustRecorded] {
        let (h, _) = both(&fx, &url(&fx), &["Code/"], &map, &BTreeSet::new(), origin);
        assert!(
            matches!(health(h), Health::NotChecked { .. }),
            "断网必须是 NotChecked（{origin:?}）"
        );
    }
}

#[test]
fn a_failing_first_server_contact_is_not_checked() {
    // 原来第一条 diff 失败 = NotChecked；本地 floor 时第一次连服务器是可达性探测，同一个注入点
    let fx = svn_repo_for_health();
    let map = map_of(&fx, &["Code/"]);
    let _fail = fault::arm(Point::FirstDiff);
    let h = check_baseline_map(
        &fx.wc_str(),
        &url(&fx),
        &map,
        &scope(&["Code/"]),
        &BTreeSet::new(),
        ExplainedOrigin::JustRecorded,
    )
    .unwrap();
    assert!(matches!(health(h), Health::NotChecked { .. }));
}
