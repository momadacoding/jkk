//! `svn --xml` 输出的解析。
//!
//! **这里是全项目唯一允许接触 XML 文本的地方**，对外只暴露结构体。
//!
//! 禁止行内正则：svn 把每个属性单独放一行（且是 CRLF），`<wc-status` 与
//! `revision="…"` 不在同一行，`grep -o '<wc-status[^>]*revision="[0-9]*"'` 这类
//! 写法**不会报错，只会返回空集**——于是 `nodeRevs` 变成空集合，「单值不变量」
//! 以一种看起来通过了的方式被绕过（spec §4.4、§6.6.4 陷阱三）。

use crate::error::{GitError, Result};
use roxmltree::{Document, Node};
use std::collections::BTreeSet;

/// `svn info --xml` 的一条 entry。
#[derive(Debug, Clone, PartialEq)]
pub struct SvnInfoEntry {
    /// entry 的 path 属性，原样保留 svn 给的写法。
    pub path: String,
    /// 该节点自己的 URL。**不是**仓库根。
    pub url: String,
    /// `<repository><root>`。断网查证与基线 trailer 用它，见 spec §6.2.1 的用途表。
    pub repos_root: String,
    /// `<repository><uuid>`。仓库身份的唯一可靠依据——URL 会因代理 / 镜像 /
    /// `http` 与 `https` 写法不同而不可靠（spec §6.2.1）。
    pub uuid: String,
    /// entry 的 revision 属性。
    pub revision: i64,
    /// `<wc-info><wcroot-abspath>`。URL 形式的 target 没有这一节，故为 Option。
    pub wc_root: Option<String>,
}

fn parse_doc(xml: &str) -> Result<Document<'_>> {
    Document::parse(xml).map_err(|e| GitError::new("svn_xml", format!("svn 输出不是合法 XML: {e}")))
}

/// 解析 `svn info --xml`。
pub fn parse_info(xml: &str) -> Result<Vec<SvnInfoEntry>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        // 缺字段一律报错而不是填默认值：这些字段是仓库身份判定的输入，
        // 一个空 uuid 会让 P4 的「所有 UUID 必须相同」在两个空串上通过。
        let path = attr(&entry, "path")?;
        let revision = attr(&entry, "revision")?
            .parse::<i64>()
            .map_err(|_| GitError::new("svn_xml", "entry 的 revision 不是整数"))?;
        let url = child_text(&entry, "url")?;
        let repository = entry
            .children()
            .find(|n| n.has_tag_name("repository"))
            .ok_or_else(|| GitError::new("svn_xml", "svn info 输出缺 <repository>"))?;
        let repos_root = child_text(&repository, "root")?;
        let uuid = child_text(&repository, "uuid")?;
        let wc_root = entry
            .children()
            .find(|n| n.has_tag_name("wc-info"))
            .and_then(|w| w.children().find(|n| n.has_tag_name("wcroot-abspath")))
            .and_then(|n| n.text())
            .map(str::to_string);
        out.push(SvnInfoEntry {
            path,
            url,
            repos_root,
            uuid,
            revision,
            wc_root,
        });
    }
    Ok(out)
}

/// 从 `svn status -v --xml` 里取每个已版本控制节点的 revision 集合。
///
/// # ⚠️ 输入必须来自带 `-v` **和** `--ignore-externals` 的 status
///
/// 本函数**不区分**节点属于哪个工作副本——它看到什么就收什么。调用方必须保证
/// 输入是 `svn status -v --xml --ignore-externals -- <target>` 的产物。
///
/// 不带那个参数的话，external **内部**的节点会一起进来，而它们的 revision 来自
/// **另一个仓库的序列**（实测 `{29825, 1938156}`）。后果是 `nodeRevs` 永久多值
/// → 永久误报失同步、初始化向导永远过不去，而且**看起来完全正常**
/// （spec §6.6.4 陷阱一）。
///
/// 这个坑之所以容易踩：同模块的 [`parse_external_roots`] 要的恰恰是**不带**
/// `--ignore-externals` 的输出，而两个函数都只收 `&str`，签名上分不出来。
/// 拿同一份 status 输出喂给两者是最自然、也最危险的写法。
///
/// **所以本函数自己检查这件事，不靠调用方记得**：违约时返回
/// `svn_xml_externals_not_ignored`，见 [`assert_externals_were_ignored`]。
///
/// # 取哪个 revision
///
/// **取 `<wc-status revision>`，不是 `<commit revision>`。** 前者是该节点在工作
/// 副本里处于哪个 revision，后者是它最后被改动是哪个 revision；实测同一个工作
/// 副本里前者 1 个取值、后者 2194 个取值，取错就是永久误报（spec §6.6.4 陷阱二）。
///
/// 用 `descendants()` 按 tag 名直接筛 `wc-status`，天然不会碰到内层的 `<commit>`
/// ——这正是选 DOM 而不是流式 parser 的理由。
pub fn parse_node_revs(xml: &str) -> Result<BTreeSet<i64>> {
    let doc = parse_doc(xml)?;
    assert_externals_were_ignored(&doc)?;
    let mut revs = BTreeSet::new();
    for st in doc.descendants().filter(|n| n.has_tag_name("wc-status")) {
        // external **根**的 wc-status 没有 revision 属性，跳过而不是记成 0——
        // 0 会凭空造出一个多值集合，把正常仓库判成 mixed-revision。
        //
        // ⚠️ 这条只处理 external 的**根节点**。它**不能**替代 `--ignore-externals`：
        // external **内部**的节点都是带 revision 的，跳过逻辑碰不到它们。见上方
        // 的输入契约。
        let Some(raw) = st.attribute("revision") else {
            continue;
        };
        let rev = raw
            .parse::<i64>()
            .map_err(|_| GitError::new("svn_xml", format!("wc-status revision 不是整数: {raw}")))?;
        revs.insert(rev);
    }
    // 空集必须报错，不能 Ok 返回。
    //
    // 2026-09-13 独立 review 的阻断项：上面那道守卫只挡住了 `--ignore-externals`
    // 那一半，**漏了 `-v` 时它完全不响**——干净工作副本上不带 `-v` 的 status 只
    // 输出 external 标记与「有改动的」节点，一个带 revision 的 entry 都没有，
    // 于是守卫沉默、这里返回空集。而空集正是本模块开头写明的那个失败形态：
    // 「nodeRevs 单值」这条不变量以一种**看起来通过了**的方式被绕过。
    //
    // 判据不会误伤：合法输入里最窄的一种是「target 就是 external 根本身」，
    // 它在自己的工作副本里是个正常节点，带 `-v` 跑出来仍有带 revision 的 entry
    // （`node_revs_accepts_an_external_root_as_the_target` 拿真实 svn 钉住了这点）。
    if revs.is_empty() {
        return Err(GitError::new(
            "svn_xml_empty_node_revs",
            "这份 svn status 输出里一个带 revision 的节点都没有。\
             最常见的原因是漏了 `-v`——不带它时 status 只报告有改动的节点，\
             干净的工作副本会得到空输出。空集会让「nodeRevs 单值」这条不变量\
             以看起来通过的方式被绕过，所以这里报错而不是返回空集。\
             取数请走 engine::svn_wc，不要在调用点自己拼 svn 命令。",
        ));
    }
    Ok(revs)
}

/// 从 `svn status --xml`（**不带** `--ignore-externals`）里列出 external 根。
///
/// 走 `item="external"` 而不是解析 `svn:externals` 属性文本：后者要处理 `-r` peg、
/// `^/` 相对 URL、引号和多行格式，实测前者能干净列出（spec §6.6.4）。
pub fn parse_external_roots(xml: &str) -> Result<Vec<String>> {
    let doc = parse_doc(xml)?;
    let mut roots = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let is_external = entry
            .children()
            .find(|n| n.has_tag_name("wc-status"))
            .and_then(|n| n.attribute("item"))
            == Some("external");
        if is_external {
            roots.push(attr(&entry, "path")?);
        }
    }
    Ok(roots)
}

/// 判定这份 status 输出是不是带 `--ignore-externals` 跑出来的，不是就报错。
///
/// **判据来自实测**（2026-09-13，临时 fixture：`Code/` 底下挂一条跨仓库 external
/// `Vendor`）：
///
/// | entry path | 带 `--ignore-externals` | 不带 |
/// |---|---|---|
/// | `Code` | normal | normal |
/// | `Code\A.cs` | normal | normal |
/// | `Code\Vendor` | **external** | **external** |
/// | `Code\Vendor` | — | normal ← external 自己的 WC 根 |
/// | `Code\Vendor\V.cs` | — | normal ← external 的内容 |
///
/// 关键观察：`item="external"` **两种情况下都出现**，所以不能靠它的有无判定；
/// 但带该参数时，external 根**及其底下**只有那一条 `item="external"` 条目。
/// 于是判据是：**是否存在「位于某个 external 根之内、而自己不是那条 external
/// 条目」的 entry**。
///
/// 为什么值得写这么一段而不是只写条注释：不带参数时 `nodeRevs` 会混进另一个仓库
/// 的 revision 序列（实测 `{29825, 1938156}`），后果是永久误报失同步、初始化向导
/// 永远过不去，**而且一切看起来都正常**（spec §6.6.4 陷阱一）。把它变成一个响亮的
/// 错误，比指望下一个人读到文档可靠。
pub fn assert_externals_were_ignored(doc: &Document) -> Result<()> {
    let externals: Vec<String> = doc
        .descendants()
        .filter(|n| n.has_tag_name("entry"))
        .filter(|n| {
            n.children()
                .find(|c| c.has_tag_name("wc-status"))
                .and_then(|c| c.attribute("item"))
                == Some("external")
        })
        .filter_map(|n| n.attribute("path"))
        .map(norm_sep)
        .collect();
    if externals.is_empty() {
        return Ok(());
    }
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let is_external_marker = entry
            .children()
            .find(|c| c.has_tag_name("wc-status"))
            .and_then(|c| c.attribute("item"))
            == Some("external");
        if is_external_marker {
            continue;
        }
        let Some(path) = entry.attribute("path").map(norm_sep) else {
            continue;
        };
        if let Some(root) = externals
            .iter()
            .find(|r| path == **r || path.starts_with(&format!("{r}/")))
        {
            return Err(GitError::new(
                "svn_xml_externals_not_ignored",
                format!(
                    "这份 svn status 输出里含 external `{root}` **内部**的节点（`{path}`），\
                     说明它不是带 --ignore-externals 跑出来的。\
                     直接拿它算 nodeRevs 会把另一个仓库的 revision 序列混进来，\
                     造成永久误报失同步（spec §6.6.4 陷阱一）。\
                     取数请走 engine::svn_wc，不要在调用点自己拼 svn 命令。"
                ),
            ));
        }
    }
    Ok(())
}

/// svn 在 Windows 上返回反斜杠，比较路径前统一成正斜杠。
fn norm_sep(p: &str) -> String {
    p.replace('\\', "/")
}

fn attr(node: &Node, name: &str) -> Result<String> {
    node.attribute(name)
        .map(str::to_string)
        .ok_or_else(|| GitError::new("svn_xml", format!("svn 输出的节点缺 {name} 属性")))
}

fn child_text(node: &Node, tag: &str) -> Result<String> {
    node.children()
        .find(|n| n.has_tag_name(tag))
        .and_then(|n| n.text())
        .map(str::to_string)
        .ok_or_else(|| GitError::new("svn_xml", format!("svn 输出缺 <{tag}>")))
}
