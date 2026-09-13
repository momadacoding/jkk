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
/// **取 `<wc-status revision>`，不是 `<commit revision>`。** 前者是该节点在工作
/// 副本里处于哪个 revision，后者是它最后被改动是哪个 revision；实测同一个工作
/// 副本里前者 1 个取值、后者 2194 个取值，取错就是永久误报（spec §6.6.4 陷阱二）。
///
/// 用 `descendants()` 按 tag 名直接筛 `wc-status`，天然不会碰到内层的 `<commit>`
/// ——这正是选 DOM 而不是流式 parser 的理由。
pub fn parse_node_revs(xml: &str) -> Result<BTreeSet<i64>> {
    let doc = parse_doc(xml)?;
    let mut revs = BTreeSet::new();
    for st in doc.descendants().filter(|n| n.has_tag_name("wc-status")) {
        // external 根的 wc-status 没有 revision 属性。跳过而不是记成 0——
        // 0 会凭空造出一个多值集合，把正常仓库判成 mixed-revision。
        let Some(raw) = st.attribute("revision") else {
            continue;
        };
        let rev = raw
            .parse::<i64>()
            .map_err(|_| GitError::new("svn_xml", format!("wc-status revision 不是整数: {raw}")))?;
        revs.insert(rev);
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
