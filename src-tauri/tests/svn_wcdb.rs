#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 直接读 `wc.db` 找被排除的节点，必须与 `svn info -R --xml` 给的逐条相同。
//!
//! 表结构没有对外文档，依据只有 Subversion 源码里的 `wc-metadata.sql`（见
//! `docs/superpowers/plans/2026-11-15-wcdb-excluded-decisions.md`）。所以这里不断言「wc.db
//! 里应该是什么样」，只断言**与 svn 自己给的结果相同**：每个场景两边各跑一次、比整张表。
//! 每个场景还另外断言 svn 那一侧不是空的（该有的时候），免得两边都空、比对空过。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_tombstone::fault::{self, Point};
use app_lib::engine::svn_wc::{enumerate_wcs, peg_safe, WcIdentity};
use app_lib::engine::svn_wcdb;
use app_lib::engine::svn_xml::parse_excluded_nodes;
use std::collections::BTreeMap;
use std::path::Path;
use svn_fixtures::{
    svn_cmd, svn_repo_for_health, svn_repo_with_same_repo_nested_external, NestedExternal,
    SvnFixture,
};

fn units(fx: &SvnFixture, whitelist: &[&str]) -> Vec<WcIdentity> {
    let wl: Vec<String> = whitelist.iter().map(|s| s.to_string()).collect();
    enumerate_wcs(&fx.wc_str(), &wl).unwrap()
}

/// 改动前体检的做法：全部单元一次 `svn info -R --xml`，挑出 depth=exclude 的 entry。
fn by_svn(fx: &SvnFixture, units: &[WcIdentity]) -> BTreeMap<String, String> {
    let mut args = vec!["info", "-R", "--xml", "--"];
    let targets: Vec<String> = units.iter().map(|u| peg_safe(&u.wc_root)).collect();
    args.extend(targets.iter().map(String::as_str));
    let xml = run_svn(&fx.wc_str(), &args).unwrap();
    parse_excluded_nodes(&xml).unwrap().into_iter().collect()
}

fn by_wcdb(fx: &SvnFixture, units: &[WcIdentity]) -> BTreeMap<String, String> {
    svn_wcdb::excluded_nodes(&fx.wc_str(), units).unwrap_or_else(|e| panic!("wc.db 读不了：{e}"))
}

/// 两边相同，并返回结果给调用方再断言形状。
fn same(fx: &SvnFixture, whitelist: &[&str]) -> BTreeMap<String, String> {
    let u = units(fx, whitelist);
    let svn = by_svn(fx, &u);
    assert_eq!(by_wcdb(fx, &u), svn, "wc.db 的结果与 svn info -R 不同");
    svn
}

fn exclude(wc: &Path, rel: &str) {
    let target = format!("{rel}@");
    svn_cmd(
        wc,
        &[
            "update",
            "--set-depth=exclude",
            "--non-interactive",
            "--",
            &target,
        ],
    );
}

fn keys(m: &BTreeMap<String, String>) -> Vec<(&str, &str)> {
    m.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

#[test]
fn nothing_excluded() {
    let fx = svn_repo_for_health();
    assert!(same(&fx, &["Code/"]).is_empty());
}

#[test]
fn an_excluded_file() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    assert_eq!(
        keys(&same(&fx, &["Code/"])),
        vec![("Code/Deep/D.cs", "file")]
    );
}

#[test]
fn an_excluded_directory() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep");
    assert_eq!(keys(&same(&fx, &["Code/"])), vec![("Code/Deep", "dir")]);
}

#[test]
fn excluded_nodes_outside_the_whitelist_are_not_reported() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Art/boss.fbx");
    exclude(&fx.wc(), "Code/Art.psd");
    assert_eq!(keys(&same(&fx, &["Code/"])), vec![("Code/Art.psd", "file")]);
}

#[test]
fn a_whitelist_entry_below_the_working_copy_root() {
    // 单元的根不是工作副本的根：wc.db 里的路径要换算成仓库相对
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    assert_eq!(
        keys(&same(&fx, &["Code/Deep/"])),
        vec![("Code/Deep/D.cs", "file")]
    );
}

#[test]
fn two_whitelist_entries() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep");
    exclude(&fx.wc(), "Art/boss.fbx");
    assert_eq!(
        keys(&same(&fx, &["Code/", "Art/"])),
        vec![("Art/boss.fbx", "file"), ("Code/Deep", "dir")]
    );
}

#[test]
fn a_sibling_whose_name_extends_the_whitelist_entry() {
    // `Code` 的前缀匹配不能把 `Code2` 也算进来
    let fx = svn_repo_for_health();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code2")).unwrap();
    std::fs::write(wc.join("Code2/X.cs"), "x\n").unwrap();
    svn_cmd(&wc, &["add", "Code2", "--non-interactive"]);
    svn_cmd(&wc, &["commit", "-m", "r2", "--non-interactive"]);
    svn_cmd(&wc, &["update", "--non-interactive"]);
    exclude(&wc, "Code2/X.cs");
    assert!(same(&fx, &["Code/"]).is_empty());
}

#[test]
fn a_non_ascii_directory() {
    let fx = svn_repo_for_health();
    let wc = fx.wc();
    let dir = wc.join("Code/中文目录");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.cs"), "x\n").unwrap();
    svn_cmd(&wc, &["add", "--force", "--non-interactive", "--", "Code@"]);
    svn_cmd(&wc, &["commit", "-m", "r2", "--non-interactive"]);
    svn_cmd(&wc, &["update", "--non-interactive"]);
    // svn.exe 收不了中文参数（开发机代码页），以那个目录为 cwd 排除它，再 cleanup 补删目录
    let _ = std::process::Command::new("svn")
        .current_dir(&dir)
        .args(["update", "--set-depth=exclude", "--non-interactive", "."])
        .output()
        .unwrap();
    svn_cmd(&wc, &["cleanup", "--non-interactive"]);
    assert_eq!(keys(&same(&fx, &["Code/"])), vec![("Code/中文目录", "dir")]);
}

#[test]
fn inside_a_nested_external() {
    // external 是另一个工作副本，有自己的 wc.db
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    exclude(&fx.wc().join("Code/Vendor"), "V.cs");
    let u = units(&fx, &["Code/"]);
    assert_eq!(u.len(), 2, "前提：external 是单独的单元 {u:?}");
    assert_eq!(
        keys(&same(&fx, &["Code/"])),
        vec![("Code/Vendor/V.cs", "file")]
    );
}

#[test]
fn a_shallow_directory_leaves_no_excluded_node() {
    // depth=empty 不是 exclude：两边都不报（服务器侧核对另管，见 svn_server_set）
    let fx = svn_repo_for_health();
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--set-depth=empty",
            "--non-interactive",
            "--",
            "Code/Deep@",
        ],
    );
    assert!(same(&fx, &["Code/"]).is_empty());
}

#[test]
fn an_excluded_child_of_a_directory_scheduled_for_deletion() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    svn_cmd(
        &fx.wc(),
        &["delete", "--non-interactive", "--", "Code/Deep@"],
    );
    // 实测：父目录待删时 svn 仍报这个被排除的节点（schedule 是 normal）
    assert_eq!(
        keys(&same(&fx, &["Code/"])),
        vec![("Code/Deep/D.cs", "file")]
    );
}

#[test]
fn an_excluded_child_inside_a_local_copy() {
    // 工作副本内的 svn copy：被排除的节点出现在 WORKING 层（wc-metadata.sql 的 presence 注释）
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    svn_cmd(
        &fx.wc(),
        &["copy", "--non-interactive", "--", "Code/Deep@", "Code/Copy"],
    );
    // 实测：副本里的那一行在 op_depth 2、presence excluded，svn 两处都报
    assert_eq!(
        keys(&same(&fx, &["Code/"])),
        vec![("Code/Copy/D.cs", "file"), ("Code/Deep/D.cs", "file")]
    );
}

#[test]
fn an_excluded_node_brought_back() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep");
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--set-depth=infinity",
            "--non-interactive",
            "--",
            "Code/Deep@",
        ],
    );
    assert!(same(&fx, &["Code/"]).is_empty());
}

// ---------------------------------------------------------------------------
// 认不出来的一律不读：交回调用方去跑 svn info -R
// ---------------------------------------------------------------------------

fn wc_db(fx: &SvnFixture) -> std::path::PathBuf {
    fx.wc().join(".svn").join("wc.db")
}

#[test]
fn an_unknown_format_is_refused() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    let u = units(&fx, &["Code/"]);
    // 只改这个临时 fixture 的 wc.db
    let db = rusqlite::Connection::open(wc_db(&fx)).unwrap();
    db.pragma_update(None, "user_version", 32).unwrap();
    drop(db);
    let err = svn_wcdb::excluded_nodes(&fx.wc_str(), &u).unwrap_err();
    assert!(err.contains("32"), "{err}");
}

#[test]
fn an_unfinished_operation_is_refused() {
    // 被打断的操作在 WORK_QUEUE 里留了活：此刻 wc.db 与磁盘可能不一致。只读的 svn info 不因此
    // 拒绝，这里拒绝是我们自己保守
    let fx = svn_repo_for_health();
    let u = units(&fx, &["Code/"]);
    let db = rusqlite::Connection::open(wc_db(&fx)).unwrap();
    db.execute("INSERT INTO work_queue (work) VALUES (x'00')", [])
        .unwrap();
    drop(db);
    let err = svn_wcdb::excluded_nodes(&fx.wc_str(), &u).unwrap_err();
    assert!(err.contains("WORK_QUEUE"), "{err}");
}

#[test]
fn a_unit_without_a_working_copy_root_is_refused() {
    let fx = svn_repo_for_health();
    let mut u = units(&fx, &["Code/"]);
    u[0].wc_root_abspath = None;
    let err = svn_wcdb::excluded_nodes(&fx.wc_str(), &u).unwrap_err();
    assert!(err.contains("没有工作副本根"), "{err}");
}

#[test]
fn a_whitelist_root_spelled_in_another_case_is_refused() {
    // 白名单写成 `code`、svn 记的是 `Code`：单元的根在 wc.db 里查不到（local_relpath 区分
    // 大小写），不能当成「没有被排除的」
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    let mut u = units(&fx, &["Code/"]);
    u[0].wc_root = "code".to_string();
    let err = svn_wcdb::excluded_nodes(&fx.wc_str(), &u).unwrap_err();
    assert!(err.contains("查不到"), "{err}");
}

#[test]
fn a_repo_root_spelled_in_another_case_still_reads() {
    // 打开仓库时的写法（盘符、目录名大小写）与 svn 记的工作副本根不同：Windows 上是同一个目录
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    let u = units(&fx, &["Code/"]);
    let flipped: String = fx
        .wc_str()
        .chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                c.to_ascii_uppercase()
            }
        })
        .collect();
    let got = svn_wcdb::excluded_nodes(&flipped, &u);
    if cfg!(windows) {
        assert_eq!(keys(&got.unwrap()), vec![("Code/Deep/D.cs", "file")]);
    } else {
        assert!(got.is_err());
    }
}

#[test]
fn overlapping_whitelist_entries() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    assert_eq!(
        keys(&same(&fx, &["Code/", "Code/Deep/"])),
        vec![("Code/Deep/D.cs", "file")]
    );
}

#[test]
fn an_excluded_child_inside_a_moved_directory() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    svn_cmd(
        &fx.wc(),
        &[
            "move",
            "--non-interactive",
            "--",
            "Code/Deep@",
            "Code/Moved",
        ],
    );
    assert!(!same(&fx, &["Code/"]).is_empty());
}

// ---------------------------------------------------------------------------
// 录完时查一次，体检拿那份
// ---------------------------------------------------------------------------

#[test]
fn an_adopted_probe_is_used_once_and_only_while_its_guard_lives() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    let u = units(&fx, &["Code/"]);
    let probe = svn_wcdb::probe(&fx.wc_str(), &u);
    assert!(probe.is_ok());
    // 查过之后排除被撤销：拿到的是查的那一刻的结果，只拿一次
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--set-depth=infinity",
            "--non-interactive",
            "--",
            "Code/Deep@",
        ],
    );
    let guard = probe.adopt();
    assert_eq!(keys(&by_wcdb(&fx, &u)), vec![("Code/Deep/D.cs", "file")]);
    assert!(by_wcdb(&fx, &u).is_empty(), "第二次是现查的");
    drop(guard);

    // 守卫 drop 之后没被取走的作废
    let probe = svn_wcdb::probe(&fx.wc_str(), &u);
    exclude(&fx.wc(), "Code/Deep/D.cs");
    drop(probe.adopt());
    assert_eq!(keys(&by_wcdb(&fx, &u)), vec![("Code/Deep/D.cs", "file")]);
}

#[test]
fn an_adopted_probe_only_answers_the_same_units() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    let u = units(&fx, &["Code/"]);
    let _guard = svn_wcdb::probe(&fx.wc_str(), &units(&fx, &["Art/"])).adopt();
    assert_eq!(keys(&by_wcdb(&fx, &u)), vec![("Code/Deep/D.cs", "file")]);
}

#[test]
fn a_refused_probe_sends_the_check_straight_to_svn() {
    // 录完时查不了（例如锁住超时）就不让体检再查一遍、再等一轮
    let fx = svn_repo_for_health();
    let u = units(&fx, &["Code/"]);
    let probe = {
        let _armed = fault::arm(Point::WcdbRefused);
        svn_wcdb::probe(&fx.wc_str(), &u)
    };
    assert!(!probe.is_ok());
    let _guard = probe.adopt();
    assert!(svn_wcdb::excluded_nodes(&fx.wc_str(), &u).is_err());
}

#[test]
fn reading_leaves_wc_db_untouched() {
    let fx = svn_repo_for_health();
    exclude(&fx.wc(), "Code/Deep/D.cs");
    // svn 自己会留一个 0 字节的 wc.db-journal（journal_mode = TRUNCATE，实测），所以比的是
    // 全部 wc.db* 文件的名字与字节，而不是「只剩 wc.db」
    let snapshot = || -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(fx.wc().join(".svn"))
            .unwrap()
            .map(|e| e.unwrap())
            .filter(|e| e.file_name().to_string_lossy().starts_with("wc.db"))
            .map(|e| {
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    let before = snapshot();
    let u = units(&fx, &["Code/"]);
    by_wcdb(&fx, &u);
    assert!(before.iter().any(|(n, _)| n == "wc.db"));
    assert!(snapshot() == before, "wc.db* 被改过");
}
