//! §16 R2a：把 `svn status` 的条目分成「取回 BASE」「从树里剔除」「不用动」。
//!
//! 这是一层纯逻辑，但它决定基线里每个文件的内容来源，判错一条就是一个假基线。
//! 所以八种 item 逐个立一行，而不是测两三种就宣称覆盖了（`AGENTS.md` §13 #9）。

use app_lib::engine::svn_dirty::{classify, Fix};
use app_lib::engine::svn_xml::StatusEntry;

fn entry(path: &str, item: &str) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        item: item.into(),
        props: "none".into(),
        wc_locked: false,
    }
}

fn entry_with_props(path: &str, item: &str, props: &str) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        item: item.into(),
        props: props.into(),
        wc_locked: false,
    }
}

#[test]
fn every_svn_status_item_maps_to_exactly_one_outcome() {
    // 左列是 svn `--xml` 里 `<wc-status item>` 的真实取值。
    for (item, want) in [
        // 磁盘上的不是服务器内容（或磁盘上根本没有）→ 取服务器那一版
        ("modified", Some(Fix::TakeBase)),
        ("replaced", Some(Fix::TakeBase)),
        ("conflicted", Some(Fix::TakeBase)),
        ("deleted", Some(Fix::TakeBase)),
        ("missing", Some(Fix::TakeBase)),
        // 服务器 rN 上根本没有它们 → 不能进基线树
        ("added", Some(Fix::Drop)),
        ("unversioned", Some(Fix::Drop)),
        ("ignored", Some(Fix::Drop)),
        // 不用动
        ("normal", None),
        ("external", None),
    ] {
        let got = classify(&[entry("Code/X.cs", item)]).unwrap();
        assert_eq!(
            got.get("Code/X.cs").copied(),
            want,
            "item={item} 的归类不对。实际整份结果：{got:?}"
        );
    }
}

#[test]
fn a_property_only_change_is_not_touched() {
    // `item="normal" props="modified"` 是「只改了属性、内容没变」（2026-09-14 实测，
    // 文本版 status 显示成第二列的 M）。
    //
    // 它**必须**落在「不用动」里。内容本来就等于服务器内容，换不换字节都一样；
    // 而让它进结果就等于让它进体检的 `S`，`D \ S` 随之变小——判据被悄悄削弱，
    // 却没有任何测试会因此变红（spec §16.1「S 必须恰好」）。
    let got = classify(&[entry_with_props("Code/A.cs", "normal", "modified")]).unwrap();
    assert!(got.is_empty(), "实际：{got:?}");
}

#[test]
fn an_obstructed_entry_is_a_bug_not_a_category() {
    // P1 在 §16.1 之后唯一仍然阻断的就是它，所以它不该走到这一层。
    // 静默当成「不用动」会把一个说不清是什么的位置按磁盘录进基线。
    let err = classify(&[entry("Code/A.cs", "obstructed")]).unwrap_err();
    assert_eq!(err.code, "svn_dirty_obstructed", "实际：{}", err.message);
}

#[test]
fn an_unknown_item_stops_instead_of_being_silently_ignored() {
    // svn 将来加一个新的 item，或者有人打错字。默认当成「不用动」意味着那些文件
    // 会被按磁盘录进基线——正是本期要消灭的那种错误，而且没有任何信号。
    let err = classify(&[entry("Code/A.cs", "brand-new-thing")]).unwrap_err();
    assert_eq!(err.code, "svn_dirty_unknown_item");
    assert!(
        err.message.contains("brand-new-thing"),
        "必须指名是哪个 item，否则无从下手：{}",
        err.message
    );
}

#[test]
fn an_empty_list_classifies_to_nothing() {
    // 干净工作副本的形态。必须是空结果而不是错误——`S = ∅` 时体检判据退化成
    // `D == ∅`，那是 spec §6.8.2 的原判据（§6.8.7）。
    assert!(classify(&[]).unwrap().is_empty());
}

#[test]
fn an_empty_path_is_rejected() {
    // 空路径会变成 `git update-index -- ""` 之类的调用。本仓库已经因为空 pathspec
    // 误伤过一次（`AGENTS.md` §6.2），这类边界一律显式拒绝。
    let err = classify(&[entry("", "modified")]).unwrap_err();
    assert_eq!(err.code, "svn_dirty_empty_path");
}

#[test]
fn paths_come_back_sorted_and_deduped() {
    // 结果直接决定 `S`，而 `S` 要和体检报出的 `D` 做差集。重复条目会让同一个路径
    // 被 cat 两次（白费一次子进程），顺序不稳则会让错误信息每次不一样。
    let got = classify(&[
        entry("Code/B.cs", "modified"),
        entry("Code/A.cs", "deleted"),
        entry("Code/B.cs", "modified"),
    ])
    .unwrap();
    let keys: Vec<&str> = got.keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["Code/A.cs", "Code/B.cs"]);
}

#[test]
fn backslashes_are_normalized_so_the_keys_match_git_paths() {
    // 结果的键要和 `git ls-files` / 白名单对齐，那两边都是正斜杠。
    // 解析层已经归一过一次，这里再兜一次——跨平台路径是本项目的高发缺陷区
    // （`AGENTS.md` §8）。
    let got = classify(&[entry("Code\\Deep\\D.cs", "modified")]).unwrap();
    assert_eq!(
        got.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["Code/Deep/D.cs"]
    );
}
