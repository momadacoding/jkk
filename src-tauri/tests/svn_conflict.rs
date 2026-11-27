#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! SVN 冲突列表与解决（设计 2026-11-28-update-conflict-resolve-in-jkk §3.1）。全部在临时 fixture 上跑：
//! wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_conflict::{
    list_conflicts, prepare_conflict_merge, resolve_conflict, ResolveChoice, SvnConflict,
};
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn write_in(dir: &std::path::Path, rel: &str, s: &[u8]) {
    std::fs::write(dir.join(rel), s).unwrap();
}

/// 这几个文件都在冲突里（2026-11-28 实测的几种形态各一个）：
///
/// | 路径 | 冲突 |
/// |---|---|
/// | `Art/x.psd` | 内容（文本，可合并） |
/// | `Art/p.psd` | 只有属性 |
/// | `Art/t.bin` | 内容（`svn:mime-type` 二进制，不可合并） |
/// | `Code/A.cs` | 内容，但是 W-file |
/// | `Art`（目录） | 属性 |
///
/// 本地与服务器各自的内容：`x.psd` 是 `a/B-mine/c` 对 `a/B-theirs/c`，属性 `k` 是 `mine` 对 `theirs`。
fn conflicted() -> SvnFixture {
    let fx = hybrid_with_colleague();
    let (wc, wc2) = (fx.wc(), fx.wc2());
    // r2：三个新文件，同事那边先拿到
    write_in(&wc, "Art/x.psd", b"a\nb\nc\n");
    write_in(&wc, "Art/p.psd", b"p\n");
    write_in(&wc, "Art/t.bin", b"\x00\x01");
    svn_cmd(
        &wc,
        &["add", "--non-interactive", "--", "Art/p.psd", "Art/t.bin"],
    );
    svn_cmd(
        &wc,
        &[
            "propset",
            "svn:mime-type",
            "application/octet-stream",
            "Art/t.bin",
        ],
    );
    svn_cmd(&wc, &["commit", "-m", "r2", "--non-interactive"]);
    svn_cmd(&wc2, &["update", "--non-interactive"]);
    // r3：同事一侧
    write_in(&wc2, "Art/x.psd", b"a\nB-theirs\nc\n");
    write_in(&wc2, "Art/t.bin", b"\x00\x02");
    write_in(&wc2, "Code/A.cs", b"a-theirs\n");
    svn_cmd(&wc2, &["propset", "k", "theirs", "Art/p.psd"]);
    svn_cmd(&wc2, &["propset", "k", "theirs", "Art"]);
    svn_cmd(&wc2, &["commit", "-m", "r3", "--non-interactive"]);
    // 我这一侧，再 update
    write_in(&wc, "Art/x.psd", b"a\nB-mine\nc\n");
    write_in(&wc, "Art/t.bin", b"\x00\x03");
    write_in(&wc, "Code/A.cs", b"a-mine\n");
    svn_cmd(&wc, &["propset", "k", "mine", "Art/p.psd"]);
    svn_cmd(&wc, &["propset", "k", "mine", "Art"]);
    svn_cmd(
        &wc,
        &["update", "--accept", "postpone", "--non-interactive"],
    );
    fx
}

fn find<'a>(all: &'a [SvnConflict], path: &str) -> &'a SvnConflict {
    all.iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("列表里没有 {path}：{all:?}"))
}

/// 这个路径此刻还在不在冲突里，以及 svn 留下的临时文件还剩哪些。
fn still_conflicted(fx: &SvnFixture, rel: &str) -> bool {
    list_conflicts(&fx.wc_str())
        .unwrap()
        .iter()
        .any(|c| c.path == rel)
}
fn leftovers(fx: &SvnFixture, rel: &str) -> Vec<String> {
    let name = std::path::Path::new(rel)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let dir = fx.wc().join(rel).parent().unwrap().to_path_buf();
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with(&format!("{name}.")))
        .collect();
    v.sort();
    v
}
fn prop(fx: &SvnFixture, name: &str, rel: &str) -> String {
    run_svn(&fx.wc_str(), &["propget", name, "--", rel])
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn the_list_classifies_every_kind() {
    let fx = conflicted();
    let all = list_conflicts(&fx.wc_str()).unwrap();

    let x = find(&all, "Art/x.psd");
    assert!(x.text && !x.props && !x.tree && !x.dir && !x.git_managed);
    assert!(x.resolvable && x.mergeable, "{x:?}");

    let p = find(&all, "Art/p.psd");
    assert!(!p.text && p.props && p.resolvable && !p.mergeable, "{p:?}");

    let t = find(&all, "Art/t.bin");
    assert!(t.text && t.resolvable, "{t:?}");
    assert!(
        !t.mergeable,
        "二进制冲突没有 prev-wc-file，不能交给合并工具：{t:?}"
    );

    let a = find(&all, "Code/A.cs");
    assert!(a.git_managed && !a.resolvable && !a.mergeable, "{a:?}");

    let dir = find(&all, "Art");
    assert!(dir.dir && dir.props && !dir.resolvable, "{dir:?}");
    assert_eq!(all.len(), 5, "{all:?}");
}

#[test]
fn mine_keeps_my_whole_file_and_cleans_up() {
    let fx = conflicted();
    resolve_conflict(&fx.wc_str(), "Art/x.psd", ResolveChoice::Mine).unwrap();
    assert_eq!(read(&fx, "Art/x.psd"), "a\nB-mine\nc\n");
    assert!(!still_conflicted(&fx, "Art/x.psd"));
    assert_eq!(leftovers(&fx, "Art/x.psd"), Vec::<String>::new());
}

#[test]
fn theirs_takes_the_server_file() {
    let fx = conflicted();
    resolve_conflict(&fx.wc_str(), "Art/x.psd", ResolveChoice::Theirs).unwrap();
    assert_eq!(read(&fx, "Art/x.psd"), "a\nB-theirs\nc\n");
    assert!(!still_conflicted(&fx, "Art/x.psd"));
    assert_eq!(leftovers(&fx, "Art/x.psd"), Vec::<String>::new());
}

#[test]
fn a_binary_conflict_resolves_with_theirs() {
    let fx = conflicted();
    resolve_conflict(&fx.wc_str(), "Art/t.bin", ResolveChoice::Theirs).unwrap();
    assert_eq!(
        std::fs::read(fx.wc().join("Art/t.bin")).unwrap(),
        b"\x00\x02"
    );
    assert!(!still_conflicted(&fx, "Art/t.bin"));
}

#[test]
fn a_property_only_conflict_resolves_either_way() {
    let fx = conflicted();
    resolve_conflict(&fx.wc_str(), "Art/p.psd", ResolveChoice::Theirs).unwrap();
    assert_eq!(prop(&fx, "k", "Art/p.psd"), "theirs");
    assert!(!still_conflicted(&fx, "Art/p.psd"));
    assert_eq!(leftovers(&fx, "Art/p.psd"), Vec::<String>::new());

    let fx = conflicted();
    resolve_conflict(&fx.wc_str(), "Art/p.psd", ResolveChoice::Mine).unwrap();
    assert_eq!(prop(&fx, "k", "Art/p.psd"), "mine");
    assert!(!still_conflicted(&fx, "Art/p.psd"));
}

#[test]
fn merged_refuses_while_markers_are_left_then_accepts_the_saved_result() {
    let fx = conflicted();
    assert!(
        read(&fx, "Art/x.psd").contains("<<<<<<< .mine"),
        "前提：带冲突标记"
    );

    let err = resolve_conflict(&fx.wc_str(), "Art/x.psd", ResolveChoice::Merged).unwrap_err();
    assert_eq!(err.code, "svn_conflict_markers_left", "{}", err.message);
    assert!(still_conflicted(&fx, "Art/x.psd"), "被拒之后仍是冲突");

    std::fs::write(fx.wc().join("Art/x.psd"), "a\nB-merged\nc\n").unwrap();
    resolve_conflict(&fx.wc_str(), "Art/x.psd", ResolveChoice::Merged).unwrap();
    assert_eq!(read(&fx, "Art/x.psd"), "a\nB-merged\nc\n");
    assert!(!still_conflicted(&fx, "Art/x.psd"));
    assert_eq!(leftovers(&fx, "Art/x.psd"), Vec::<String>::new());
}

#[test]
fn merged_refuses_a_property_only_conflict() {
    let fx = conflicted();
    let err = resolve_conflict(&fx.wc_str(), "Art/p.psd", ResolveChoice::Merged).unwrap_err();
    assert_eq!(err.code, "svn_conflict_not_mergeable", "{}", err.message);
    assert!(still_conflicted(&fx, "Art/p.psd"));
}

#[test]
fn refuses_what_it_does_not_handle_and_touches_nothing() {
    let fx = conflicted();
    let root = fx.wc_str();
    let before_a = read(&fx, "Code/A.cs");

    for (file, code) in [
        ("Code/A.cs", "svn_conflict_not_resolvable"),
        ("Art", "svn_conflict_not_resolvable"),
        ("Code/B.cs", "svn_conflict_state_changed"),
        ("Art/nope.psd", "svn_conflict_state_changed"),
        ("../wc2/Art/x.psd", "external_tool_bad_path"),
        ("", "external_tool_bad_path"),
    ] {
        let err = resolve_conflict(&root, file, ResolveChoice::Theirs).unwrap_err();
        assert_eq!(err.code, code, "{file}: {}", err.message);
    }
    assert_eq!(read(&fx, "Code/A.cs"), before_a);
    assert!(still_conflicted(&fx, "Code/A.cs"));
    assert!(still_conflicted(&fx, "Art"));
}

#[test]
fn resolve_is_refused_while_a_merge_tool_has_the_file_open() {
    // 独立 review：工具还开着时 svn 改写文件、删掉 .mine/.rN，用户之后在工具里一保存就盖回去
    let fx = conflicted();
    let held = app_lib::engine::external_launch::try_lock(&fx.wc_str(), "Art/x.psd", "BC").unwrap();

    let err = resolve_conflict(&fx.wc_str(), "Art/x.psd", ResolveChoice::Theirs).unwrap_err();
    assert_eq!(err.code, "external_tool_busy", "{}", err.message);
    assert!(still_conflicted(&fx, "Art/x.psd"));

    drop(held);
    resolve_conflict(&fx.wc_str(), "Art/x.psd", ResolveChoice::Theirs).unwrap();
    assert!(!still_conflicted(&fx, "Art/x.psd"));
}

#[test]
fn prepare_merge_hands_out_svns_own_three_files() {
    let fx = conflicted();
    let m = prepare_conflict_merge(&fx.wc_str(), "Art/x.psd").unwrap();
    let text = |p: &std::path::Path| std::fs::read_to_string(p).unwrap();
    assert_eq!(text(&m.base), "a\nb\nc\n");
    assert_eq!(text(&m.local), "a\nB-mine\nc\n");
    assert_eq!(text(&m.remote), "a\nB-theirs\nc\n");
    assert_eq!(m.merged, fx.wc().join("Art/x.psd"));
    assert_eq!(m.base_title, "共同祖先 r2");
    assert_eq!(m.remote_title, "服务器 r3");
}

#[test]
fn prepare_merge_refuses_binary_and_property_only_conflicts() {
    let fx = conflicted();
    for file in ["Art/t.bin", "Art/p.psd"] {
        let err = prepare_conflict_merge(&fx.wc_str(), file).unwrap_err();
        assert_eq!(
            err.code, "external_merge_unsupported",
            "{file}: {}",
            err.message
        );
    }
    let err = prepare_conflict_merge(&fx.wc_str(), "Code/A.cs").unwrap_err();
    assert_eq!(err.code, "svn_conflict_not_resolvable", "{}", err.message);
}

#[test]
fn no_conflicts_gives_an_empty_list() {
    let fx = hybrid_with_colleague();
    assert_eq!(
        list_conflicts(&fx.wc_str()).unwrap(),
        Vec::<SvnConflict>::new()
    );
}
