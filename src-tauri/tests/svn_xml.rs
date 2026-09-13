//! svn `--xml` 解析的测试。
//!
//! 这里的输入**不是手写的理想 XML**，而是从真实 svn 输出复制来的形态：属性
//! 逐行排列、CRLF 行尾、`<wc-status>` 里还嵌着一个同名属性的 `<commit>`。
//! 用理想输入测这个模块等于没测（`AGENTS.md` §13 rubric #9）。

use app_lib::engine::svn_xml::{parse_external_roots, parse_info, parse_node_revs};

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
