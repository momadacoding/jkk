#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! §16 R2a：把 `svn status` 的条目分成「取回 BASE」「从树里剔除」「不用动」。
//!
//! 这是一层纯逻辑，但它决定基线里每个文件的内容来源，判错一条就是一个假基线。
//! 所以八种 item 逐个立一行，而不是测两三种就宣称覆盖了（`AGENTS.md` §13 #9）。
//!
//! 文件末尾那条例外：`read_dirty` 的磁盘遍历需要真工作副本，只能上 fixture。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_dirty::{classify, Fix};
use app_lib::engine::svn_xml::StatusEntry;

/// 一个**已提交过**的节点（`revision="1"`、不是复制来的）。
fn entry(path: &str, item: &str) -> StatusEntry {
    node(path, item, Some(1), false)
}

fn entry_with_props(path: &str, item: &str, props: &str) -> StatusEntry {
    StatusEntry {
        props: props.into(),
        ..entry(path, item)
    }
}

/// `revision` / `copied` 照 `svn status -v --xml` 的原样给：复制来的节点没有
/// `revision` 属性（`None`），`svn add` 还没提交的是 `Some(-1)`。
fn node(path: &str, item: &str, revision: Option<i64>, copied: bool) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        item: item.into(),
        props: "none".into(),
        wc_locked: false,
        revision,
        copied,
        schedule: None,
    }
}

/// `missing` + `revision="-1"`，带上 `read_dirty` 从 `svn info` 补来的 schedule。
fn missing_uncommitted(path: &str, schedule: Option<&str>) -> StatusEntry {
    StatusEntry {
        schedule: schedule.map(str::to_string),
        ..node(path, "missing", Some(-1), false)
    }
}

#[test]
fn every_svn_status_item_maps_to_exactly_one_outcome() {
    // 左列是 svn `--xml` 里 `<wc-status item>` 的真实取值。
    for (item, want) in [
        // 磁盘上的不是服务器内容（或磁盘上根本没有）→ 取服务器那一版
        ("modified", Some(Fix::TakeBase)),
        // `replaced` 不在这张表里：不带历史的离线取不到旧内容，一律停下（事故分析 6.2），
        // 带历史的按复制子树判，见下面几条
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

// ---------------------------------------------------------------------------
// `read_dirty` 的磁盘遍历：跟随 junction，但必须停得下来
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 「服务器 rN 上有没有这个路径」：只看 item 说不清的那几种（2026-11-06 实测形状）
// ---------------------------------------------------------------------------

#[test]
fn a_file_added_then_deleted_from_disk_is_dropped_not_fetched() {
    // 2026-11-05 真实工作副本上的形态：整个目录 `svn add`、没提交，里面的文件又从
    // 磁盘上删掉了。它与「已提交后丢失」同是 `missing`，只有 `revision="-1"` 不同。
    // 当成 TakeBase 的话 `svn cat -r BASE` 报 E200009，录制整个中止，重试也没用。
    // `svn info` 说它们是 `add`（实测），这是摘掉它们的唯一依据。
    let got = classify(&[
        node("Code/New", "added", Some(-1), false),
        missing_uncommitted("Code/New/main.cs", Some("add")),
        missing_uncommitted("Code/New/tests", Some("add")),
        missing_uncommitted("Code/New/tests/t.cs", Some("add")),
    ])
    .unwrap();
    for p in ["Code/New/main.cs", "Code/New/tests/t.cs"] {
        assert_eq!(got.get(p), Some(&Fix::Drop), "{p}：实际整份结果 {got:?}");
    }
}

#[test]
fn a_replaced_file_deleted_from_disk_is_refused_not_dropped() {
    // 独立 review 实测：`svn rm A.cs` 后同名 `svn add`（或 `svn cp`），再从磁盘删掉，
    // status 也是 `missing revision="-1"`，与上一条一模一样；只有 `svn info` 的
    // schedule 是 `replace`。服务器上**有** A.cs，摘掉就是基线静默少一个文件。
    let err = classify(&[missing_uncommitted("Code/A.cs", Some("replace"))]).unwrap_err();
    assert_eq!(
        err.code, "svn_dirty_replaced_missing",
        "实际：{}",
        err.message
    );
    assert!(err.message.contains("Code/A.cs"), "实际：{}", err.message);
}

#[test]
fn an_uncommitted_missing_file_without_a_known_schedule_is_refused() {
    // 没查到 schedule、或是没见过的值：说不清服务器上有没有。只有正面证据才摘，
    // 也不落回 TakeBase（`revision="-1"` 的节点取不到服务器内容）。
    for schedule in [None, Some("normal"), Some("delete")] {
        let err = classify(&[missing_uncommitted("Code/A.cs", schedule)]).unwrap_err();
        assert_eq!(
            err.code, "svn_dirty_missing_unresolved",
            "schedule={schedule:?}：{}",
            err.message
        );
    }
}

#[test]
fn a_committed_file_deleted_from_disk_is_still_fetched_from_the_server() {
    // 反面：上一条的修法不能把真正「服务器上有、本地删了」的文件也摘掉，
    // 那会让基线悄悄少一个文件。
    let got = classify(&[node("Code/A.cs", "missing", Some(1), false)]).unwrap();
    assert_eq!(got.get("Code/A.cs"), Some(&Fix::TakeBase), "实际：{got:?}");
}

#[test]
fn every_node_under_a_copied_directory_is_dropped() {
    // `svn cp Code/A Code/B`（`svn mv` 的目标一样）：服务器 rN 上根本没有 `Code/B`。
    //
    // 修之前两条路都录出假基线、且没有任何报错：
    // - 干净的子文件是 `normal`，被当成「和服务器一样」按磁盘录进去
    // - 改过的子文件 `svn cat -r BASE` **成功**，拿到的是复制源 `Code/A` 的内容
    // 删掉的子文件 copied 标记跟着消失，只剩 `revision="-1"`（实测）。
    let got = classify(&[
        node("Code/B", "added", None, true),
        node("Code/B/clean.cs", "normal", None, true),
        node("Code/B/edited.cs", "modified", None, true),
        node("Code/B/gone.cs", "missing", Some(-1), false),
        node("Code/B/removed.cs", "deleted", None, true),
        // 中间目录 `svn status -v` 一定会列出来（`normal copied`），read_dirty 也保留它
        node("Code/B/Sub", "normal", None, true),
        node("Code/B/Sub/deep.cs", "normal", None, true),
    ])
    .unwrap();
    for p in [
        "Code/B/clean.cs",
        "Code/B/edited.cs",
        "Code/B/gone.cs",
        "Code/B/removed.cs",
        "Code/B/Sub/deep.cs",
    ] {
        assert_eq!(got.get(p), Some(&Fix::Drop), "{p}：实际整份结果 {got:?}");
    }
}

#[test]
fn a_copied_file_on_its_own_is_dropped() {
    // `svn cp Code/A.cs Code/C.cs`：复制根自己就是文件。
    let got = classify(&[node("Code/C.cs", "added", None, true)]).unwrap();
    assert_eq!(got.get("Code/C.cs"), Some(&Fix::Drop), "实际：{got:?}");
}

#[test]
fn a_replaced_node_inside_a_copied_directory_is_dropped_too() {
    // 实测：复制源里有一个 replaced 的文件，复制出来的那份也是 `replaced copied`。
    // 它的父目录服务器上就没有，谈不上「服务器上的旧内容」。
    let got = classify(&[
        node("Code/B", "added", None, true),
        node("Code/B/x.cs", "replaced", None, true),
    ])
    .unwrap();
    assert_eq!(got.get("Code/B/x.cs"), Some(&Fix::Drop), "实际：{got:?}");
}

#[test]
fn a_file_replaced_by_a_copy_is_refused() {
    // `svn rm Code/A.cs; svn cp Code/B.cs Code/A.cs`：服务器 rN 上 `Code/A.cs` 是**旧**
    // 内容，而 `svn cat -r BASE` 返回的是复制源 `Code/B.cs`（实测）。离线拿不到旧内容，
    // 录下去就是假基线，只能停下来让用户先提交或还原。
    let err = classify(&[node("Code/A.cs", "replaced", None, true)]).unwrap_err();
    assert_eq!(
        err.code, "svn_dirty_replaced_with_history",
        "实际：{}",
        err.message
    );
    assert!(
        err.message.contains("Code/A.cs"),
        "要点名是哪个路径：{}",
        err.message
    );
}

#[test]
fn everything_under_a_directory_replaced_by_a_copy_is_refused() {
    // 复制根是 replaced 的目录：服务器上这个目录有旧内容，里面哪些路径服务器上也有，
    // 离线判断不了。
    let err = classify(&[
        node("Code/D", "replaced", None, true),
        node("Code/D/x.cs", "normal", None, true),
    ])
    .unwrap_err();
    assert_eq!(
        err.code, "svn_dirty_replaced_with_history",
        "实际：{}",
        err.message
    );
}

#[test]
fn a_replaced_node_without_history_is_refused() {
    // 事故分析 6.2：`svn rm` 后同名 `svn add`（不带历史）：`revision="1"`、没有 copied。
    // 服务器 rN 上有旧内容，但 `svn cat -r BASE` 报 E200009，离线取不到。原来走 TakeBase，
    // 停在 `svn_baseline_classification_conflict`；选定的做法是在分类时就明确拒绝，说清怎么办，
    // 不为它破例联网
    for e in [
        node("Code/A.cs", "replaced", Some(1), false),
        entry("Code/A.cs", "replaced"),
    ] {
        let err = classify(&[e]).unwrap_err();
        assert_eq!(
            err.code, "svn_dirty_replaced_without_history",
            "实际：{}",
            err.message
        );
        assert!(err.message.contains("Code/A.cs"), "{}", err.message);
        assert!(err.message.contains("提交"), "{}", err.message);
        assert!(err.message.contains("还原"), "{}", err.message);
    }
}

#[test]
fn files_under_a_directory_replaced_without_history_are_refused_not_dropped() {
    // PR 116 遗留：`svn rm D; mkdir D; svn add D`。D 底下的同名文件是 `added`、schedule `add`，
    // 单看它会被摘掉，可服务器上旧的 D 里有它。D 自己是不带历史的 replaced，整个停下
    let err = classify(&[
        node("Code/D", "replaced", Some(1), false),
        node("Code/D/x.cs", "added", Some(-1), false),
    ])
    .unwrap_err();
    assert_eq!(err.code, "svn_dirty_replaced_without_history");
    assert!(err.message.contains("Code/D"), "{}", err.message);

    // 子节点排在前面也一样：结果是整个出错，不会只摘掉子节点
    let err = classify(&[
        node("Code/D/x.cs", "added", Some(-1), false),
        node("Code/D", "replaced", Some(1), false),
    ])
    .unwrap_err();
    assert_eq!(err.code, "svn_dirty_replaced_without_history");
}

#[test]
fn a_copied_directory_deleted_from_disk_as_a_whole_is_dropped() {
    // 独立 review 实测：`svn cp E C1; rm -rf C1`（或 `svn mv E M1` 后在资源管理器里删掉
    // 目标）。整棵子树都是 `missing revision="-1"`、都没有 copied 标记；根的 schedule 是
    // `add`，子节点是 `normal`。根要被认成 added 的复制根，子节点才不会因 schedule 说不清
    // 而误停。
    let got = classify(&[
        missing_uncommitted("Code/C1", Some("add")),
        missing_uncommitted("Code/C1/x.cs", Some("normal")),
        missing_uncommitted("Code/C1/sub", Some("normal")),
        missing_uncommitted("Code/C1/sub/y.cs", Some("normal")),
    ])
    .unwrap();
    for p in ["Code/C1/x.cs", "Code/C1/sub/y.cs"] {
        assert_eq!(got.get(p), Some(&Fix::Drop), "{p}：实际 {got:?}");
    }
}

#[test]
fn a_directory_replaced_by_a_copy_and_deleted_from_disk_is_refused() {
    // 同形，但根是 `svn rm D; svn cp E D` 后删掉的：schedule 是 `replace`，服务器上 D 有旧内容。
    // `D` 自己报「替换后丢失」，子节点报「复制顶替」，先撞上哪条取决于顺序——两条都是拒绝。
    let err = classify(&[
        missing_uncommitted("Code/D", Some("replace")),
        missing_uncommitted("Code/D/x.cs", Some("normal")),
    ])
    .unwrap_err();
    assert!(
        matches!(
            err.code.as_str(),
            "svn_dirty_replaced_missing" | "svn_dirty_replaced_with_history"
        ),
        "实际：{} {}",
        err.code,
        err.message
    );
}

#[test]
fn a_whole_subdirectory_deleted_inside_a_copied_directory_is_dropped() {
    // 实测：`svn cp A C; rm -rf C/sub` 后，`C/sub` 与 `C/sub/x.cs` 都是
    // `missing revision="-1"`、都丢了 copied 标记，`svn info` 的 schedule 是 `normal`，
    // 而 `svn cat -r BASE C/sub/x.cs` **成功**返回复制源的内容。祖先链必须穿过这两层
    // 找到 `C`，否则要么静默录进复制源内容，要么因 schedule 说不清而误停。
    let got = classify(&[
        node("Code/C", "added", None, true),
        missing_uncommitted("Code/C/sub", Some("normal")),
        missing_uncommitted("Code/C/sub/x.cs", Some("normal")),
    ])
    .unwrap();
    assert_eq!(
        got.get("Code/C/sub/x.cs"),
        Some(&Fix::Drop),
        "实际：{got:?}"
    );
}

#[test]
fn an_external_inside_a_copied_directory_is_not_swallowed_by_the_copy() {
    // 独立 review 实测：带 `svn:externals` 的目录被 `svn cp` 后再 `svn up`，external
    // 拉进复制目录。external 里改过的文件是 `modified revision="2"`、没有 copied 标记，
    // 服务器上（external 的源）确实有它。external 根自己是干净的、不在清单里，
    // 它就是边界——越过它找到外面的复制根，会把这个文件误摘出基线。
    let got = classify(&[
        node("Proj2", "added", None, true),
        node("Proj2/Code/ext/l.cs", "modified", Some(2), false),
    ])
    .unwrap();
    assert_eq!(
        got.get("Proj2/Code/ext/l.cs"),
        Some(&Fix::TakeBase),
        "实际：{got:?}"
    );
}

#[test]
fn a_copy_whose_origin_lies_above_the_whitelist_is_refused_with_its_own_reason() {
    // 白名单根 `Proj2/Code` 自己是 `normal copied`，真正的复制起点 `Proj2` 不在本次
    // 检查的清单里。既不能当新加的摘掉，也不该被说成「先 svn rm 再 svn cp」。
    let err = classify(&[
        node("Proj2/Code", "normal", None, true),
        node("Proj2/Code/x.cs", "modified", None, true),
    ])
    .unwrap_err();
    assert_eq!(
        err.code, "svn_dirty_copy_origin_unknown",
        "实际：{}",
        err.message
    );
}

#[test]
fn a_sibling_whose_name_extends_a_copied_directory_is_not_mistaken_for_its_child() {
    // 祖先判断按路径段，不按字符串前缀：`Code/B` 被复制，不代表 `Code/Bar.cs` 也是。
    let got = classify(&[
        node("Code/B", "added", None, true),
        node("Code/Bar.cs", "missing", Some(1), false),
    ])
    .unwrap();
    assert_eq!(
        got.get("Code/Bar.cs"),
        Some(&Fix::TakeBase),
        "实际：{got:?}"
    );
}

#[test]
fn read_dirty_keeps_clean_files_under_a_copied_directory() {
    // `normal` + `props=none` 本来会被当成干净条目丢掉，但复制来的干净文件服务器上
    // 没有，丢掉就等于宣称它「和服务器一样」。
    let fx = svn_fixtures::svn_repo();
    let wc = fx.wc();
    svn_fixtures::svn_cmd(&wc, &["mkdir", "Code/Lib"]);
    std::fs::write(wc.join("Code/Lib/L.cs"), "l\n").unwrap();
    svn_fixtures::svn_cmd(&wc, &["add", "Code/Lib/L.cs"]);
    svn_fixtures::svn_cmd(&wc, &["commit", "-m", "lib", "--non-interactive"]);
    svn_fixtures::svn_cmd(&wc, &["update", "--non-interactive"]);
    svn_fixtures::svn_cmd(&wc, &["copy", "Code/Lib", "Code/Lib2"]);

    let dirty =
        app_lib::engine::svn_dirty::read_dirty(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    let copied = dirty
        .iter()
        .find(|e| e.path == "Code/Lib2/L.cs")
        .unwrap_or_else(|| panic!("复制来的干净文件被当成干净条目丢掉了：{dirty:?}"));
    assert_eq!(copied.item, "normal");
    assert!(copied.copied, "实际：{copied:?}");
    assert!(
        !dirty.iter().any(|e| e.path == "Code/Lib/L.cs"),
        "复制源本身是真干净的，不该进清单：{dirty:?}"
    );
}

#[test]
fn read_dirty_matches_the_schedule_back_to_a_non_ascii_path() {
    // `svn info` 输出的 path 要与 status 的 path 对得上，否则 schedule 留空、录制被拦。
    // 中文路径是本项目的高发缺陷区（开发机线程代码页 936），必须走真 svn 实测。
    let fx = svn_fixtures::svn_repo();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code/新建 目录")).unwrap();
    std::fs::write(wc.join("Code/新建 目录/类@名.cs"), "n\n").unwrap();
    // 参数只给 ASCII：svn.exe 按系统代码页取参数，中文路径传不进去（ACP 1252 上实测
    // 变成 `?`）。`--force` 让它自己递归加上 `Code` 底下未版本控制的项
    svn_fixtures::svn_cmd(&wc, &["add", "--force", "Code"]);
    std::fs::remove_file(wc.join("Code/新建 目录/类@名.cs")).unwrap();

    let dirty =
        app_lib::engine::svn_dirty::read_dirty(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    let e = dirty
        .iter()
        .find(|e| e.path == "Code/新建 目录/类@名.cs")
        .unwrap_or_else(|| panic!("清单里没有中文路径的条目：{dirty:?}"));
    assert_eq!(
        (e.item.as_str(), e.revision, e.schedule.as_deref()),
        ("missing", Some(-1), Some("add"))
    );
}

#[test]
fn read_dirty_tells_an_added_missing_file_from_a_replaced_one() {
    // 两条在 svn status 里一模一样（`missing revision="-1"`），schedule 必须由
    // read_dirty 从真 `svn info` 里补出来，否则 classify 只能一律停下。
    let fx = svn_fixtures::svn_repo();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code/New")).unwrap();
    std::fs::write(wc.join("Code/New/n.cs"), "n\n").unwrap();
    svn_fixtures::svn_cmd(&wc, &["add", "Code/New"]);
    std::fs::remove_file(wc.join("Code/New/n.cs")).unwrap();
    svn_fixtures::svn_cmd(&wc, &["rm", "Code/A.cs"]);
    std::fs::write(wc.join("Code/A.cs"), "new\n").unwrap();
    svn_fixtures::svn_cmd(&wc, &["add", "Code/A.cs"]);
    std::fs::remove_file(wc.join("Code/A.cs")).unwrap();

    let dirty =
        app_lib::engine::svn_dirty::read_dirty(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    let shape = |p: &str| {
        let e = dirty
            .iter()
            .find(|e| e.path == p)
            .unwrap_or_else(|| panic!("清单里没有 {p}：{dirty:?}"));
        (e.item.clone(), e.revision, e.schedule.clone())
    };
    assert_eq!(
        shape("Code/New/n.cs"),
        ("missing".to_string(), Some(-1), Some("add".to_string()))
    );
    assert_eq!(
        shape("Code/A.cs"),
        ("missing".to_string(), Some(-1), Some("replace".to_string()))
    );
    // 其余条目不额外起 svn info
    assert!(
        dirty
            .iter()
            .filter(|e| !(e.item == "missing" && e.revision == Some(-1)))
            .all(|e| e.schedule.is_none()),
        "实际：{dirty:?}"
    );
}

#[cfg(windows)]
#[test]
fn read_dirty_follows_a_junction_but_stops_at_a_loop() {
    // 2026-09-21 独立 review。这条钉的是**终止性**，而且刻意不经过 `git`——
    // 端到端那条（`tests/svn_init.rs`）受 git 自己的递归深度影响，结局不稳定。
    //
    // 为什么非要有祖先链守卫：实测环形 junction 的**每一层** `Path::is_dir()`
    // 都是 `true`，所以这个遍历除了守卫之外**没有任何终止条件**。git 自己也
    // 只是撞上 Windows 重解析点 63 层的上限才停的（探针实测 65 个索引条目）。
    // 拿「操作系统恰好会拦住」当终止性论证不成立——那是别人的实现细节。
    let fx = svn_fixtures::svn_repo();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code/NewDir")).unwrap();
    std::fs::write(wc.join("Code/NewDir/New.cs"), "class N {}").unwrap();
    let link = wc.join("Code").join("NewDir").join("Loop");
    if !svn_fixtures::make_junction(&link, &wc.join("Code").join("NewDir")) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }

    let dirty = app_lib::engine::svn_dirty::read_dirty(&fx.wc_str(), &["Code/".to_string()])
        .expect("环形 junction 不该让读脏清单失败");
    let paths: Vec<String> = dirty.iter().map(|e| e.path.replace('\\', "/")).collect();

    // 跟随：未版本控制目录里的文件必须被摊出来（svn 只报那条目录）
    assert!(
        paths.iter().any(|p| p == "Code/NewDir/New.cs"),
        "没跟进未版本控制目录，`New.cs` 不见了：{paths:?}"
    );
    // 停得下来：一层都不许绕进环里
    assert!(
        !paths.iter().any(|p| p.contains("Loop/")),
        "环没被拦住，遍历绕进去了：{paths:?}"
    );
    assert!(
        dirty.len() < 10,
        "条目数爆了（{}），说明在环里转了很多圈：{paths:?}",
        dirty.len()
    );

    std::fs::remove_dir(&link).unwrap();
}
