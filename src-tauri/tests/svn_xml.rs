//! svn `--xml` 解析的测试。
//!
//! 这里的输入**不是手写的理想 XML**，而是从真实 svn 输出复制来的形态：属性
//! 逐行排列、CRLF 行尾、`<wc-status>` 里还嵌着一个同名属性的 `<commit>`。
//! 用理想输入测这个模块等于没测（`AGENTS.md` §13 rubric #9）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_xml::{
    parse_diff_summary, parse_external_roots, parse_externals_props, parse_info, parse_list,
    parse_log, parse_node_revs, parse_status_entries,
};
use svn_fixtures::svn_repo_with_cross_repo_nested_external;

/// 真实 `svn status -v --xml` 的形态：属性跨行 + CRLF + 内层 <commit>。
const STATUS_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\".\">\r\n\
<entry\r\n   path=\"A.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"29825\"\r\n   props=\"none\">\r\n\
<commit\r\n   revision=\"29293\">\r\n\
<author>someone</author>\r\n\
</commit>\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"B.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"29825\"\r\n   props=\"none\">\r\n\
<commit\r\n   revision=\"1640\">\r\n\
<author>other</author>\r\n\
</commit>\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\Vendor\">\r\n\
<wc-status\r\n   item=\"external\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
</target>\r\n\
</status>\r\n";

#[test]
fn node_revs_take_wc_status_revision_not_commit_revision() {
    // 这是整个模块最重要的一条。取错会让 nodeRevs 永久多值 → 永久误报失同步
    // （spec §6.6.4 陷阱二：实测 <commit revision> 有 2194 个不同取值）。
    let revs = parse_node_revs(STATUS_XML).unwrap();
    assert_eq!(revs.len(), 1, "应当只有一个值，实际 {revs:?}");
    assert!(revs.contains(&29825));
    assert!(
        !revs.contains(&29293),
        "29293 是 <commit revision>，不该进来"
    );
    assert!(!revs.contains(&1640), "1640 是 <commit revision>，不该进来");
}

#[test]
fn node_revs_skip_entries_without_a_revision() {
    // external 根的 <wc-status item="external"> 没有 revision 属性。
    // 跳过它而不是当成 0——0 会凭空制造一个多值集合。
    let revs = parse_node_revs(STATUS_XML).unwrap();
    assert!(!revs.contains(&0));
}

#[test]
fn external_roots_are_listed_by_item_attribute() {
    // 枚举 external 根走 item="external"，比解析 svn:externals 属性文本可靠
    // ——后者要处理 -r peg、^/ 相对 URL、引号和多行格式（spec §6.6.4）。
    let roots = parse_external_roots(STATUS_XML).unwrap();
    assert_eq!(roots.len(), 1);
}

#[test]
fn external_root_paths_come_back_with_windows_separators_verbatim() {
    // 2026-09-13 实测：Windows 上 `svn status -v --xml` 的 path 属性用**反斜杠**
    // （`Code\A.cs`）。这里**故意不在解析层规范化**——解析层的职责是如实反映 svn
    // 说了什么；规范化是消费方的事（`engine::svn_wc` 要拿它和正斜杠形式的白名单
    // 比对，不转换就永远比不中，会把嵌套 external 反复塞回待处理队列）。
    //
    // 这条测试的作用是把「消费方必须转换」变成一个有断言兜着的约定，而不是
    // 某个人碰巧记得的细节。
    let roots = parse_external_roots(STATUS_XML).unwrap();
    assert_eq!(roots[0], r"Code\Vendor", "解析层不做分隔符转换");
}

/// 真实形态：**没带** `--ignore-externals` 的 status，external 内部节点混了进来。
///
/// 2026-09-13 临时 fixture 实测（`Code/` 底下一条跨仓库 external `Vendor`）：
/// 带该参数时 `Code\Vendor` 只出现一次且是 `item="external"`；不带时它出现两次，
/// 且多出一条 `Code\Vendor\V.cs`。下面照这个形态构造，external 内部的 revision
/// 用另一个仓库那种百万量级的号（实测真实值就是 1938156）。
const STATUS_XML_EXTERNALS_NOT_IGNORED: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\"Code\">\r\n\
<entry\r\n   path=\"Code\\A.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"29825\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\Vendor\">\r\n\
<wc-status\r\n   item=\"external\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\Vendor\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"1938156\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\Vendor\\V.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"1938156\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
</target>\r\n\
</status>\r\n";

#[test]
fn node_revs_refuses_input_that_was_not_run_with_ignore_externals() {
    // 2026-09-13 review 的 finding 3：光在文档里写「输入必须带 --ignore-externals」
    // 等于靠调用方记得，而 parse_external_roots 要的恰恰是**不带**的输出、两者
    // 签名又分不出来——拿同一份输出喂给两者是最自然的写法。
    //
    // 不拦住的后果不是报错，而是 nodeRevs 静默混进另一个仓库的 revision 序列
    // → 永久误报失同步、初始化向导永远过不去，一切看起来都正常（spec §6.6.4 陷阱一）。
    let err = parse_node_revs(STATUS_XML_EXTERNALS_NOT_IGNORED)
        .expect_err("含 external 内部节点的输入必须被拒");
    assert_eq!(err.code, "svn_xml_externals_not_ignored");
    assert!(err.message.contains("Vendor"), "实际：{}", err.message);
}

#[test]
fn node_revs_still_accepts_a_properly_ignored_status_with_an_external_root() {
    // 对照组：带 --ignore-externals 时 external 根**仍然出现**（只是没有内容），
    // 所以判据不能是「有没有 item=external」——那会把正常输入也拒掉。
    // STATUS_XML 里就有一条 Code\Vendor 的 external 根。
    let revs = parse_node_revs(STATUS_XML).unwrap();
    assert_eq!(revs.len(), 1);
}

#[test]
fn the_guard_holds_against_real_svn_output_in_both_directions() {
    // 上面两条用的是我手写的常量——常量是照实测抄的，但「照着抄」和「真的成立」
    // 是两回事（`AGENTS.md` §14.1：自造 fixture 不会包含设计者没想到的东西）。
    // 这条拿**真实 svn 输出**把两个方向都过一遍。
    let fx = svn_repo_with_cross_repo_nested_external();
    let wc = fx.wc_str();

    // 正确用法：带 --ignore-externals → 通过，且是单值
    let good = run_svn(
        &wc,
        &["status", "-v", "--xml", "--ignore-externals", "--", "Code"],
    )
    .unwrap();
    let revs = parse_node_revs(&good).expect("带 --ignore-externals 的输出必须被接受");
    assert_eq!(revs.len(), 1, "白名单内应当是单值，实际 {revs:?}");

    // 错误用法：不带 → 必须被拦下，而不是静默算出一个多值集合
    let bad = run_svn(&wc, &["status", "-v", "--xml", "--", "Code"]).unwrap();
    let err = parse_node_revs(&bad).expect_err("不带 --ignore-externals 的输出必须被拒");
    assert_eq!(err.code, "svn_xml_externals_not_ignored");
}

#[test]
fn node_revs_refuses_a_status_that_was_run_without_dash_v() {
    // 2026-09-13 独立 review 的阻断项 B2。
    //
    // 输入契约写的是 `svn status -v --xml --ignore-externals`，但守卫只挡住了
    // `--ignore-externals` 那一半。**漏了 `-v` 时它完全不响**：干净工作副本上
    // 不带 `-v` 的 status 只输出 external 标记，没有任何带 revision 的 entry，
    // 于是守卫沉默、`parse_node_revs` 返回 `Ok(空集)`。
    //
    // 空集正是本模块开头写明的那个失败形态：「单值不变量」以一种看起来通过了的
    // 方式被绕过。返回 Err 才是对的。
    let fx = svn_repo_with_cross_repo_nested_external();
    let wc = fx.wc_str();

    let no_v = run_svn(
        &wc,
        &["status", "--xml", "--ignore-externals", "--", "Code"],
    )
    .unwrap();
    let err = parse_node_revs(&no_v).expect_err("漏了 -v 的输出必须被拒，不能返回空集");
    assert_eq!(err.code, "svn_xml_empty_node_revs");

    // 对照组：加上 -v 之后同一条命令必须通过，且是单值。
    // 没有这一半，上面那条也可能是被别的原因拒的。
    let with_v = run_svn(
        &wc,
        &["status", "-v", "--xml", "--ignore-externals", "--", "Code"],
    )
    .unwrap();
    let revs = parse_node_revs(&with_v).expect("带 -v 的输出必须被接受");
    assert_eq!(revs.len(), 1, "实际 {revs:?}");
}

#[test]
fn node_revs_accepts_an_external_root_as_the_target() {
    // 上一条新加的「空集即报错」判据不能误伤合法输入。最窄的那种合法输入是
    // 「target 就是 external 根本身」——它在自己的工作副本里是个正常节点，
    // 带 `-v` 跑出来仍然有带 revision 的 entry。
    let fx = svn_repo_with_cross_repo_nested_external();
    let out = run_svn(
        &fx.wc_str(),
        &[
            "status",
            "-v",
            "--xml",
            "--ignore-externals",
            "--",
            "Code/Vendor",
        ],
    )
    .unwrap();
    let revs = parse_node_revs(&out).expect("external 根自己当 target 时必须通过");
    assert!(!revs.is_empty(), "实际 {revs:?}");
}

#[test]
fn parse_node_revs_on_garbage_errors_instead_of_returning_empty() {
    // 空集合会让「单值不变量」以看起来通过的方式被绕过。解析不了必须报错。
    let err = parse_node_revs("this is not xml").expect_err("非 XML 必须报错");
    assert_eq!(err.code, "svn_xml");
}

/// 真实 `svn info --xml` 的形态。
///
/// ⚠️ 属性顺序、`<wc-info>` 里的 `<schedule>` / `<depth>`、以及 `<entry>` 底下那个
/// **`<commit revision>` 直接子节点**都照 2026-09-13 的真实输出抄的。最后这个是
/// 关键：它是「该节点最后被改动是哪个 revision」，与 entry 自身的 `revision`
/// 属性不是一回事，这里故意让两者不同（9 vs 4）——取错就会被
/// `info_takes_the_entry_revision_not_the_commit_child` 抓到。
const INFO_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<info>\r\n\
<entry\r\n   kind=\"dir\"\r\n   path=\"Code\"\r\n   revision=\"9\">\r\n\
<url>file:///C:/repo/Code</url>\r\n\
<relative-url>^/Code</relative-url>\r\n\
<repository>\r\n\
<root>file:///C:/repo</root>\r\n\
<uuid>0f098dbb-30ec-1142-aedc-0eea138a0008</uuid>\r\n\
</repository>\r\n\
<wc-info>\r\n\
<wcroot-abspath>C:/wc1</wcroot-abspath>\r\n\
<schedule>normal</schedule>\r\n\
<depth>infinity</depth>\r\n\
</wc-info>\r\n\
<commit\r\n   revision=\"4\">\r\n\
<author>liuhao</author>\r\n\
<date>2026-09-13T07:26:05.899080Z</date>\r\n\
</commit>\r\n\
</entry>\r\n\
</info>\r\n";

#[test]
fn info_reads_url_root_uuid_and_wcroot() {
    let entries = parse_info(INFO_XML).unwrap();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.path, "Code");
    assert_eq!(e.revision, 9);
    assert_eq!(e.url, "file:///C:/repo/Code");
    assert_eq!(e.repos_root, "file:///C:/repo");
    assert_eq!(e.uuid, "0f098dbb-30ec-1142-aedc-0eea138a0008");
    assert_eq!(e.wc_root.as_deref(), Some("C:/wc1"));
}

#[test]
fn info_takes_the_entry_revision_not_the_commit_child() {
    // 与 §6.6.4 陷阱二同源的另一处：`svn info --xml` 的 <entry> 底下也挂着一个
    // <commit revision>（该节点最后被改动的 revision）。取它而不是 entry 自身的
    // revision 属性，会让记账单位判定读到一个偏旧的号——而且在「刚 checkout 完
    // 两者恰好相等」的 fixture 上测不出来。这里故意让两者不同。
    let e = &parse_info(INFO_XML).unwrap()[0];
    assert_eq!(e.revision, 9, "应当取 entry 的 revision 属性");
    assert_ne!(e.revision, 4, "4 是 <commit revision> 子节点，不是它");
}

#[test]
fn info_distinguishes_repository_root_from_working_copy_url() {
    // 这两个值在本设计里用途完全不同：断网查证用 Repository Root，
    // 基线体检用工作副本 URL（spec §6.2.1 那张表）。混用会让查证查不到
    // 只碰了别的工作副本的那次提交。
    let e = &parse_info(INFO_XML).unwrap()[0];
    assert_ne!(e.url, e.repos_root);
}

// ---------------------------------------------------------------------------
// Phase 1b Task 2：status / externals / diff --summarize 三个解析器
//
// 下面四个常量都是 2026-09-14 从**真实 svn 输出**逐字抄来的（临时 fixture，跑完
// 即删）。属性逐行排列、markup 用 CRLF、属性值里保留仓库里存的 LF——手写理想 XML
// 测不出这些形态（`AGENTS.md` §13 rubric #9）。
// ---------------------------------------------------------------------------

/// `svn status --no-ignore --ignore-externals --xml -- Code@` 的真实输出。
///
/// 三个脏状态各一条，外加 external 标记条目——**带了 `--ignore-externals` 它们
/// 照样在**（见 `assert_externals_were_ignored` 的实测表）。
const STATUS_DIRTY_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\"Code\">\r\n\
<entry\r\n   path=\"Code\\A.cs\">\r\n\
<wc-status\r\n   item=\"modified\"\r\n   revision=\"2\"\r\n   props=\"none\">\r\n\
<commit\r\n   revision=\"1\">\r\n\
<author>someone</author>\r\n\
</commit>\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\Added.cs\">\r\n\
<wc-status\r\n   item=\"added\"\r\n   revision=\"-1\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\New.cs\">\r\n\
<wc-status\r\n   item=\"unversioned\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\follow\">\r\n\
<wc-status\r\n   item=\"external\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
<entry\r\n   path=\"Code\\pinA\">\r\n\
<wc-status\r\n   item=\"external\"\r\n   props=\"none\">\r\n\
</wc-status>\r\n\
</entry>\r\n\
</target>\r\n\
</status>\r\n";

/// 只有属性被本地改过、内容没动的节点。文本版 status 显示为第二列的 `M`。
const STATUS_PROPS_ONLY_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\"Code\">\r\n\
<entry\r\n   path=\"Code\\A.cs\">\r\n\
<wc-status\r\n   item=\"normal\"\r\n   revision=\"1\"\r\n   props=\"modified\">\r\n\
<commit\r\n   revision=\"1\">\r\n\
<author>someone</author>\r\n\
</commit>\r\n\
</wc-status>\r\n\
</entry>\r\n\
</target>\r\n\
</status>\r\n";

/// `svn propget svn:externals -R --xml` 的真实输出，五种钉死写法。
///
/// `<target path>` 是**绝对路径 + 正斜杠**，与 status 的相对 + 反斜杠完全不同形状
/// ——同一个 CLI 两种写法（实测 E7）。属性值里的换行是仓库里存的 LF。
const EXTERNALS_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<properties>\r\n\
<target\r\n   path=\"C:/Users/x/AppData/Local/Temp/probe/wc/Code\">\r\n\
<property\r\n   name=\"svn:externals\">^/lib follow\n\
-r1 ^/lib pinA\n\
-r 1 ^/lib pinB\n\
^/lib@1 pinC\n\
pinD -r1 ^/lib\n\
</property>\r\n\
</target>\r\n\
</properties>\r\n";

/// `svn diff --old=URL/Code@1 --new=Code --summarize --xml` 的真实输出。
///
/// ⚠️ **注意前两条与第三条的属性顺序不同**（`kind,item,props` 对 `item,props,kind`）
/// ——这是 svn 真实输出里就有的，原样保留。任何按顺序取值的写法都会错。
const DIFF_SUMMARY_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<diff>\r\n\
<paths>\r\n\
<path\r\n   kind=\"file\"\r\n   item=\"modified\"\r\n   props=\"none\">file:///C:/repo/Code/A.cs</path>\r\n\
<path\r\n   kind=\"file\"\r\n   item=\"added\"\r\n   props=\"none\">file:///C:/repo/Code/Added.cs</path>\r\n\
<path\r\n   item=\"none\"\r\n   props=\"modified\"\r\n   kind=\"dir\">file:///C:/repo/Code</path>\r\n\
</paths>\r\n\
</diff>\r\n";

#[test]
fn status_entries_come_back_with_forward_slashes() {
    // 2026-09-14 实测：svn status --xml 的 path 用**反斜杠**，而白名单、
    // git ls-files、期望集合全是正斜杠。不归一的话 P1 的比对永远对不上。
    let got = parse_status_entries(STATUS_DIRTY_XML).unwrap();
    let paths: Vec<&str> = got.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "Code/A.cs",
            "Code/Added.cs",
            "Code/New.cs",
            "Code/follow",
            "Code/pinA",
        ]
    );
    assert!(
        !paths.iter().any(|p| p.contains('\\')),
        "还有反斜杠漏出来：{paths:?}"
    );
}

#[test]
fn status_entries_report_each_dirty_item_verbatim() {
    let got = parse_status_entries(STATUS_DIRTY_XML).unwrap();
    let items: Vec<(&str, &str)> = got
        .iter()
        .map(|e| (e.path.as_str(), e.item.as_str()))
        .collect();
    assert_eq!(items[0], ("Code/A.cs", "modified"));
    assert_eq!(items[1], ("Code/Added.cs", "added"));
    assert_eq!(items[2], ("Code/New.cs", "unversioned"));
}

#[test]
fn status_entries_keep_external_markers_because_they_are_not_dirt() {
    // ⚠️ 这条是给 P1 的警告，不是给解析器的。`item="external"` 的条目**即使带了
    // `--ignore-externals` 也照样出现**（2026-09-14 实测，五条 external 全在）。
    // 本设计的核心场景恰恰是「代码就在 external 里」，所以 P1 若把「status 有
    // 输出」等同于「工作副本脏」，向导在目标仓库上**永远过不去**。
    //
    // 解析层原样给出、由 P1 显式剔除，而不是在这里偷偷过滤：偷偷过滤会连
    // 「external 根自己被 svn add 了」这种真脏也一起吞掉。
    let got = parse_status_entries(STATUS_DIRTY_XML).unwrap();
    let ext: Vec<&str> = got
        .iter()
        .filter(|e| e.item == "external")
        .map(|e| e.path.as_str())
        .collect();
    assert_eq!(ext, vec!["Code/follow", "Code/pinA"]);
}

#[test]
fn status_entries_carry_props_so_a_property_only_change_is_visible() {
    // 2026-09-14 实测：只改属性不改内容的节点是 item="normal" + props="modified"
    // （文本版显示成第二列的 M）。只看 item 的话它读起来是「干净」，而工作副本
    // 确实脏着——P1 会放行，基线录下去就带着一处未提交的属性改动。
    let got = parse_status_entries(STATUS_PROPS_ONLY_XML).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].item, "normal", "内容确实没动");
    assert_eq!(got[0].props, "modified", "脏在属性上");
}

#[test]
fn a_clean_status_yields_no_entries() {
    // 干净工作副本上不带 -v 的 status 只有 <target>，没有 <entry>。
    // 这里**必须返回空列表而不是报错**——和 parse_node_revs 的空集守卫相反：
    // 那边空集是「输入本身就不对」，这边空集是「工作副本是干净的」，是好消息。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\"Code\">\r\n\
</target>\r\n\
</status>\r\n";
    assert!(parse_status_entries(xml).unwrap().is_empty());
}

#[test]
fn a_status_entry_without_wc_status_is_a_loud_error() {
    // 悄悄跳过等于「这个节点不脏」，而 P1 的全部价值就在于没漏。
    // 最现实的触发方式是喂错输入——`svn info --xml` 的 <entry> 也叫 entry。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<status>\r\n\
<target\r\n   path=\"Code\">\r\n\
<entry\r\n   path=\"Code\\A.cs\">\r\n\
</entry>\r\n\
</target>\r\n\
</status>\r\n";
    let err = parse_status_entries(xml).expect_err("缺 wc-status 必须报错");
    assert_eq!(err.code, "svn_xml");
}

#[test]
fn externals_props_keep_the_raw_text_verbatim() {
    // 判定钉没钉住是 svn_externals 的事，这里只负责原样取出来——五行写法的字段
    // 顺序不同（新格式 URL 在前、旧格式本地路径在前），解析层一旦试图分列就会分错。
    let got = parse_externals_props(EXTERNALS_XML).unwrap();
    assert_eq!(got.len(), 1);
    // 绝对路径 + 正斜杠，**不归一**，原样保留（与 status 的反斜杠形状不同，实测 E7）
    assert_eq!(got[0].target, "C:/Users/x/AppData/Local/Temp/probe/wc/Code");
    let lines: Vec<&str> = got[0].raw.lines().collect();
    assert_eq!(lines.len(), 5, "实际：{:?}", got[0].raw);
    assert_eq!(lines[0], "^/lib follow");
    assert_eq!(
        lines[4], "pinD -r1 ^/lib",
        "旧格式的本地路径在前，不能被重排"
    );
    assert!(got[0].raw.ends_with('\n'), "尾随换行也不许 trim");
}

#[test]
fn externals_props_ignore_other_property_names() {
    // 合成输入：真实的 `propget svn:externals` 只会回这一个属性，但调用方哪天
    // 改成 `proplist -v --xml` 就会混进别的属性，那时候把 svn:ignore 当 externals
    // 解析会凭空判出一堆「没钉死的 external」。守卫写在这里，不指望调用方记得。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<properties>\r\n\
<target\r\n   path=\"C:/wc/lib\">\r\n\
<property\r\n   name=\"svn:ignore\">nul.txt\n</property>\r\n\
<property\r\n   name=\"svn:externals\">^/lib pinned\n</property>\r\n\
</target>\r\n\
</properties>\r\n";
    let got = parse_externals_props(xml).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].raw.trim_end(), "^/lib pinned");
}

#[test]
fn no_externals_anywhere_is_an_empty_list_not_an_error() {
    // 真实形状：一个没有任何 externals 的子树回的是空的 <properties>，不是报错。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<properties>\r\n</properties>\r\n";
    assert!(parse_externals_props(xml).unwrap().is_empty());
}

#[test]
fn externals_props_survive_real_svn_output() {
    // 端到端：真的跑一次 svn，别让上面那些常量变成「只测自己抄得对不对」。
    let fx = svn_repo_with_cross_repo_nested_external();
    let xml = run_svn(
        &fx.wc_str(),
        &["propget", "svn:externals", "-R", "--xml", "--", "Code@"],
    )
    .unwrap();
    let got = parse_externals_props(&xml).unwrap();
    assert_eq!(got.len(), 1, "实际：{got:?}");
    assert!(
        got[0].target.replace('\\', "/").ends_with("/Code"),
        "target 应当是属性所在目录的绝对路径，实际 {}",
        got[0].target
    );
    assert!(got[0].raw.contains("Vendor"), "实际 raw={:?}", got[0].raw);
}

#[test]
fn diff_summary_separates_content_from_property_changes() {
    // spec §6.8.4 讲的是文本版 --summarize 靠**列位置**区分内容变更与属性变更，
    // 且会混进目录条目。2026-09-14 实测 --xml 被接受，且 item / props / kind
    // 是三个独立属性——本期一律用 --xml，那个解析坑从结构上消失。
    let got = parse_diff_summary(DIFF_SUMMARY_XML).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].item, "modified");
    assert_eq!(got[0].props, "none");
    assert_eq!(got[0].kind, "file");
    assert_eq!(got[1].item, "added");
    // 目录的纯属性变更：内容没动、属性动了、kind 是 dir——三个事实各有归属
    assert_eq!(got[2].item, "none");
    assert_eq!(got[2].props, "modified");
    assert_eq!(got[2].kind, "dir");
}

#[test]
fn diff_summary_does_not_depend_on_attribute_order() {
    // 实测发现：同一次输出里，前两条是 kind,item,props，第三条是 item,props,kind。
    // svn 不保证属性顺序，按序取值会静默错位——第三条会被读成 item="dir"。
    let got = parse_diff_summary(DIFF_SUMMARY_XML).unwrap();
    assert_ne!(got[2].item, "dir", "按属性顺序取值了");
    assert_eq!(got[2].kind, "dir");
}

#[test]
fn diff_summary_keeps_the_path_as_a_url() {
    // 剥 URL 前缀是 §6.8.4 调用方的事——解析层只说 XML 写了什么。
    let got = parse_diff_summary(DIFF_SUMMARY_XML).unwrap();
    assert!(got[0].path.starts_with("file:///"), "实际 {}", got[0].path);
    assert!(got[0].path.ends_with("/Code/A.cs"));
}

#[test]
fn an_empty_diff_is_an_empty_list() {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<diff>\r\n<paths>\r\n</paths>\r\n</diff>\r\n";
    assert!(parse_diff_summary(xml).unwrap().is_empty());
}

// ─────────────────────────────────────────────────────────────────────────
// Phase 1c：svn log / svn ls 的解析
//
// 输入同样取自 2026-09-15 的真实实测输出（计划的 A 组 / D 组），
// 属性逐行排列 + CRLF。
// ─────────────────────────────────────────────────────────────────────────

/// 真实 `svn log --xml` 的形态：`<logentry` 与 `revision="…"` 不在同一行。
const LOG_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<log>\r\n\
<logentry\r\n   revision=\"3\">\r\n\
<author>liuhao</author>\r\n\
<date>2026-09-15T15:21:22.337313Z</date>\r\n\
<msg>r3 deep change plus new dir</msg>\r\n\
</logentry>\r\n\
<logentry\r\n   revision=\"2\">\r\n\
<author>liuhao</author>\r\n\
<date>2026-09-15T15:21:22.106490Z</date>\r\n\
<msg>r2 subject line\r\n\
\r\n\
body line with &lt;angle&gt; &amp; ampersand</msg>\r\n\
</logentry>\r\n\
</log>\r\n";

#[test]
fn log_reads_revision_author_and_date() {
    let got = parse_log(LOG_XML).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].revision, 3);
    assert_eq!(got[0].author, "liuhao");
    assert_eq!(got[0].date, "2026-09-15T15:21:22.337313Z");
    // 默认降序（实测 M1b）。解析层原样保留顺序，不自己重排。
    assert_eq!(got[1].revision, 2);
}

#[test]
fn log_decodes_entities_and_normalizes_crlf_inside_the_message() {
    // 两件事一起钉住：
    // ① XML 实体必须已解码 —— 否则 commit message 里会显示字面的 `&lt;`
    // ② CRLF 必须按 XML 规范归一成 LF —— svn 在 Windows 上输出 CRLF，
    //    不归一的话每行结尾多一个 \r，前端渲染出来是一串空行
    let got = parse_log(LOG_XML).unwrap();
    assert_eq!(
        got[1].message,
        "r2 subject line\n\nbody line with <angle> & ampersand"
    );
}

#[test]
fn log_treats_a_missing_msg_as_empty_rather_than_an_error() {
    // 实测 Q5：`svn log -q` 的输出里 <msg> **整个元素不出现**（不是空元素）。
    // 本期不用 -q，但把「缺失 = 空」钉死，是为了让将来有人为省流量加上 -q 时
    // 不会得到一个「历史拉取整体失败」。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<log>\r\n\
<logentry\r\n   revision=\"4\">\r\n\
<author>liuhao</author>\r\n\
<date>2026-09-15T15:23:50.323496Z</date>\r\n\
</logentry>\r\n\
</log>\r\n";
    let got = parse_log(xml).unwrap();
    assert_eq!(got[0].message, "");
    assert_eq!(got[0].revision, 4);
}

#[test]
fn log_treats_a_missing_author_as_empty() {
    // 匿名提交或 revprop 被删时 <author> 不出现。这是内容缺失，不是格式错误。
    let xml = "<log>\r\n<logentry\r\n   revision=\"1\">\r\n\
<date>2026-09-15T15:23:50Z</date>\r\n<msg>m</msg>\r\n</logentry>\r\n</log>";
    assert_eq!(parse_log(xml).unwrap()[0].author, "");
}

#[test]
fn an_empty_log_from_an_empty_repository_is_not_an_error() {
    // 实测 P9：HEAD=0 的仓库返回 exit 0 + <log></log>。空历史是合法状态。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<log>\r\n</log>\r\n";
    assert!(parse_log(xml).unwrap().is_empty());
}

#[test]
fn log_rejects_a_logentry_without_a_parsable_revision() {
    // revision 是这条记录的身份，缺了或不是数字都只能报错 —— 猜一个出来会让
    // 缓存和图上出现一个不存在的节点。
    assert!(parse_log("<log><logentry><author>x</author></logentry></log>").is_err());
    assert!(
        parse_log("<log><logentry revision=\"abc\"><author>x</author></logentry></log>").is_err()
    );
}

/// 真实 `svn ls -R --xml <url>@N` 的形态（实测 M8 / P3）。
const LIST_XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<lists>\r\n\
<list\r\n   path=\"file:///C:/tmp/repo\">\r\n\
<entry\r\n   kind=\"dir\">\r\n\
<name>Code</name>\r\n\
<commit\r\n   revision=\"1\">\r\n\
<author>liuhao</author>\r\n\
<date>2026-09-15T15:22:51.983752Z</date>\r\n\
</commit>\r\n\
</entry>\r\n\
<entry\r\n   kind=\"file\">\r\n\
<name>Code/A.cs</name>\r\n\
<size>10</size>\r\n\
<commit\r\n   revision=\"1\">\r\n\
<author>liuhao</author>\r\n\
<date>2026-09-15T15:22:51.983752Z</date>\r\n\
</commit>\r\n\
</entry>\r\n\
</list>\r\n\
</lists>\r\n";

#[test]
fn list_reads_the_base_url_and_recursive_names_with_kind() {
    let got = parse_list(LIST_XML).unwrap();
    assert_eq!(got.base, "file:///C:/tmp/repo");
    assert_eq!(got.entries.len(), 2);
    assert_eq!(got.entries[0].kind, "dir");
    assert_eq!(got.entries[0].name, "Code");
    assert_eq!(got.entries[1].kind, "file");
    // 实测 N4：<name> 是**相对 <list path> 的递归路径**，正斜杠，不是仓库绝对
    // 路径。调用方要自己接前缀 —— 这与同模块 parse_diff_summary 给出完整 URL
    // 的形状又不一样，同一个 CLI 三种路径写法。
    assert_eq!(got.entries[1].name, "Code/A.cs");
}

#[test]
fn list_does_not_confuse_the_inner_commit_author_with_an_entry() {
    // 每个 entry 里嵌着 <commit><author>。按 tag 名做全文 descendants 搜索的
    // 写法会把别的东西一起捞进来；这条钉住只取 entry 的直接子元素。
    let got = parse_list(LIST_XML).unwrap();
    assert!(got
        .entries
        .iter()
        .all(|e| e.name == "Code" || e.name == "Code/A.cs"));
}

#[test]
fn a_legitimately_empty_listing_is_not_an_error() {
    // 实测 P10：空仓库 @0 返回 exit 0 + 空 <lists>。
    // ⚠️ 路径不存在时（P1）stdout 也长这样，但那次 exit 是 1 —— 那道门由
    // run_svn 的非零退出判定把守，不在本函数里。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<lists>\r\n<list\r\n   path=\"file:///C:/tmp/empty\">\r\n</list>\r\n</lists>\r\n";
    let got = parse_list(xml).unwrap();
    assert_eq!(got.base, "file:///C:/tmp/empty");
    assert!(got.entries.is_empty());
}

#[test]
fn list_rejects_truncated_output() {
    // 实测 P2：revision 不存在时 svn 把 XML 截断在半路（<list> 没闭合）。
    // 必须报错，不能当成「空清单」—— 空清单会让漏收检测的差集恒为空，
    // 于是用修 NB-2 的代码重新造一个 NB-2。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n\
<lists>\r\n<list\r\n   path=\"file:///C:/tmp/repo\">\r\n";
    assert!(parse_list(xml).is_err());
}
