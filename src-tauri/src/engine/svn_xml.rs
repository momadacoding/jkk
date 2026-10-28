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
    Ok(parse_node_rev_entries(xml)?
        .into_iter()
        .map(|(_, rev)| rev)
        .collect())
}

/// 与 [`parse_node_revs`] 同一份输入、同一套守卫，但逐节点给出 `(路径, revision)`。
///
/// 只在 `nodeRevs` 多值、要告诉用户**哪些文件**掉了队时用。路径是 entry 的 path
/// 属性，统一成正斜杠，按路径升序。
pub fn parse_node_rev_entries(xml: &str) -> Result<Vec<(String, i64)>> {
    let doc = parse_doc(xml)?;
    assert_externals_were_ignored(&doc)?;
    let mut revs: Vec<(String, i64)> = Vec::new();
    let mut skipped_uncommitted = 0usize;
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
        // 负 revision 只可能来自**还没提交的节点**。2026-09-16 实测，`svn add` 了
        // 没提交的文件在 `status -v --xml` 里是：
        //
        // ```xml
        // <entry path="Code\Added.cs">
        // <wc-status item="added" revision="-1" props="none">
        // ```
        //
        // 它不能进集合。`nodeRevs` 问的是「工作副本停在哪个**服务器** revision」，
        // 而一个还没提交的节点没有服务器 revision，对这个问题无可奉告；照收会让
        // 集合变成多值，用户看到的是「工作副本是 mixed-revision，请先跑一次完整
        // svn update」——一条做了也没用的引导，真实原因是他有个没提交的新文件。
        //
        // spec §16.1 之前这条不需要，因为 P1 要求白名单干净，未提交的 add 走不到
        // 这一步；放松 P1 之后它会正常出现。
        if rev < 0 {
            skipped_uncommitted += 1;
            continue;
        }
        let path = st
            .parent_element()
            .filter(|p| p.has_tag_name("entry"))
            .ok_or_else(|| GitError::new("svn_xml", "svn status 的 <wc-status> 不在 <entry> 底下"))
            .and_then(|entry| attr(&entry, "path"))?;
        revs.push((norm_sep(&path), rev));
    }
    revs.sort();
    // 全被跳过、一个正 revision 都没剩：不能套用下面那条「漏了 -v」的说法，
    // 那会把用户引向一个完全无关的方向。
    if revs.is_empty() && skipped_uncommitted > 0 {
        return Err(GitError::new(
            "svn_xml_all_nodes_uncommitted",
            format!(
                "这份 svn status 输出里的 {skipped_uncommitted} 个节点**全都还没提交**\
                 （revision=\"-1\"），一个有服务器 revision 的节点都没有。\
                 白名单指向的内容整个都是本地新增的，还没进过 SVN——\
                 这种工作副本上录不出「服务器在 rN 的样子」，\
                 请先把这些文件提交到 SVN，再回来建库或重建基线。"
            ),
        ));
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

/// `svn status --xml` 里的一条 entry。
///
/// # 归一策略：**只有这一个解析器归一路径**
///
/// 本模块三个新解析器的路径形状互不相同（2026-09-14 实测）：
///
/// | 解析器 | 源 XML 里的形状 | 是否归一 |
/// |---|---|---|
/// | [`parse_status_entries`] | 相对 target + **反斜杠**（`Code\A.cs`） | **归一**成正斜杠 |
/// | [`parse_externals_props`] | **绝对路径 + 正斜杠**（`C:/…/wc/Code`） | 不归一 |
/// | [`parse_diff_summary`] | **URL**（`file:///…/repo/Code/A.cs`） | 不归一 |
///
/// 同一个 CLI 三种写法，不是笔误。解析层的规矩是「只说 XML 写了什么」，归一属于
/// 知道上下文的调用方（[`parse_external_roots`] 立的规矩）；status 是唯一例外，
/// 因为「相对 target 的反斜杠」没有任何调用方会想要原样保留——白名单、
/// `git ls-files`、期望集合全是正斜杠。
#[derive(Debug, Clone, PartialEq)]
pub struct StatusEntry {
    /// 相对 target 的路径，已归一成**正斜杠**。
    pub path: String,
    /// `<wc-status item>` 原样：`modified` / `added` / `unversioned` /
    /// `deleted` / `normal` / `external` …
    pub item: String,
    /// `<wc-status props>` 原样：`none` / `modified`。
    ///
    /// **单独留一个字段是必须的**：只改属性、没改内容的节点是
    /// `item="normal" props="modified"`（2026-09-14 实测，文本版显示成第二列的
    /// `M`）。丢掉这个字段，P1 看到的就是一个「normal」的干净节点，于是带着
    /// 一处未提交的属性改动去录基线。
    pub props: String,
    /// `<wc-status wc-locked="true">`：该节点上有一把**未释放的工作副本写锁**，
    /// 说明上一次 svn 操作没跑完（文本版 status 的第三列 `L`）。
    ///
    /// 与 `item == "incomplete"` 是**两件不同的事**，P3 两个都要查
    /// （2026-09-14 实测）：`svn cleanup` 清掉这把锁，却**不会**清掉
    /// `incomplete`。只查锁的话，用户跑完 cleanup 就能带着一棵没更新完的树
    /// 通过校验。
    pub wc_locked: bool,
}

/// 解析 `svn status --xml`，列出每一条 entry。
///
/// # ⚠️ `item="external"` 不是脏状态
///
/// external 标记条目**即使带了 `--ignore-externals` 也照样出现**（实测见
/// [`assert_externals_were_ignored`] 的表）。本设计的核心场景恰恰是「代码就在
/// external 里」，所以把「有输出」等同于「工作副本脏」的调用方，在目标仓库上
/// **永远过不去**。P1 必须显式剔除 `item == "external"` 的条目。
///
/// 这里不偷偷过滤：偷偷过滤会连「external 根自己被 `svn add` 了」这种真脏
/// 一起吞掉，而那是调用方看不见的。
///
/// # 空列表是好消息，不是错误
///
/// 与 [`parse_node_revs`] 的空集守卫相反。那边空集意味着输入本身就不对
/// （漏了 `-v`），这边空集意味着工作副本是干净的——P1 想要的正是它。
///
/// 结果按 path 升序排，让报错信息可复现。
pub fn parse_status_entries(xml: &str) -> Result<Vec<StatusEntry>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let path = norm_sep(&attr(&entry, "path")?);
        // 缺 <wc-status> 一律报错，不跳过。悄悄跳过等于宣称「这个节点不脏」，
        // 而 P1 的全部价值就在于没漏。最现实的触发方式是喂错输入——
        // `svn info --xml` 的 <entry> 也叫 entry，但底下没有 wc-status。
        let st = entry
            .children()
            .find(|n| n.has_tag_name("wc-status"))
            .ok_or_else(|| {
                GitError::new(
                    "svn_xml",
                    format!("svn status 的 entry `{path}` 底下没有 <wc-status>"),
                )
            })?;
        out.push(StatusEntry {
            path,
            item: attr(&st, "item")?,
            props: attr(&st, "props")?,
            // 没锁时这个属性根本不出现，所以这里只能用「有且等于 true」判定，
            // 不能像 item / props 那样要求它必须存在。
            wc_locked: st.attribute("wc-locked") == Some("true"),
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `svn status [-u] --xml` 里 `sync_in` 关心的字段（spec §7.2 第 4、5 条，§7.3 S5）。
///
/// 与 [`StatusEntry`] 分开：那个结构被初始化、重建、体检、失同步判定共用，
/// 给它加字段会牵动所有构造点；这里要的三样东西（`repos-status`、树冲突、
/// `against`）只有更新流程用。
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStatusEntry {
    /// 相对 target 的路径，正斜杠。
    pub path: String,
    pub item: String,
    pub props: String,
    /// `<wc-status tree-conflicted="true">`。
    ///
    /// **不能只看 `item == "conflicted"`**：2026-09-25 实测，树冲突节点是
    /// `item="added" tree-conflicted="true"`，只看 item 会把它当成一个普通的
    /// 本地新增放过去。
    pub tree_conflicted: bool,
    /// `<repos-status item>`：带 `-u` 时才有。`modified` / `added` / `deleted`
    /// 表示这次 update 会碰到它的内容，`none` 表示不会。
    pub repos_item: Option<String>,
    /// `<repos-status props>`：带 `-u` 时才有。只改属性的远端改动是
    /// `item="none" props="modified"`（2026-09-26 实测），单看 `repos_item` 会漏掉。
    pub repos_props: Option<String>,
}

impl UpdateStatusEntry {
    /// 冲突态：内容冲突、树冲突、属性冲突任一种。
    ///
    /// 三种都要看（PR #49 review 第三轮 #4，实测）：树冲突节点的 item 可以是 `added`，
    /// 属性冲突节点是 `item="normal" props="conflicted"`。
    pub fn is_conflicted(&self) -> bool {
        self.item == "conflicted" || self.tree_conflicted || self.props == "conflicted"
    }

    /// 这次 update 会碰到它（内容或属性）。只对带 `-u` 的结果有意义。
    pub fn is_incoming(&self) -> bool {
        self.repos_item.as_deref().is_some_and(|r| r != "none")
            || self.repos_props.as_deref().is_some_and(|r| r != "none")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStatus {
    pub entries: Vec<UpdateStatusEntry>,
    /// `<against revision>`：服务器此刻的 HEAD，也就是这次会更新到的 revision。
    /// 带 `-u` 时才有；实测**没有任何可更新内容时也会给出**。
    pub against: Option<u64>,
}

pub fn parse_update_status(xml: &str) -> Result<UpdateStatus> {
    let doc = parse_doc(xml)?;
    let mut entries = Vec::new();
    for entry in doc.descendants().filter(|n| n.has_tag_name("entry")) {
        let path = norm_sep(&attr(&entry, "path")?);
        // 与 parse_status_entries 同一条理由：缺 <wc-status> 一律报错，不跳过
        let st = entry
            .children()
            .find(|n| n.has_tag_name("wc-status"))
            .ok_or_else(|| {
                GitError::new(
                    "svn_xml",
                    format!("svn status 的 entry `{path}` 底下没有 <wc-status>"),
                )
            })?;
        let (repos_item, repos_props) =
            match entry.children().find(|n| n.has_tag_name("repos-status")) {
                Some(r) => (Some(attr(&r, "item")?), Some(attr(&r, "props")?)),
                None => (None, None),
            };
        entries.push(UpdateStatusEntry {
            path,
            item: attr(&st, "item")?,
            props: attr(&st, "props")?,
            tree_conflicted: st.attribute("tree-conflicted") == Some("true"),
            repos_item,
            repos_props,
        });
    }
    // 项目根的 <target> 在最前；external 各有自己的 <target>/<against>，
    // 它们与根同属一个仓库时数值相同（P4 保证白名单不跨仓库），取第一个即可。
    let against = match doc.descendants().find(|n| n.has_tag_name("against")) {
        Some(a) => {
            let raw = attr(&a, "revision")?;
            Some(raw.parse::<u64>().map_err(|_| {
                GitError::new("svn_xml", format!("<against revision> 不是数字：{raw}"))
            })?)
        }
        None => None,
    };
    Ok(UpdateStatus { entries, against })
}

/// `svn propget svn:externals -R --xml` 里的一处属性。
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalsProp {
    /// 属性所在目录的**绝对路径 + 正斜杠**，原样保留（不归一，理由见
    /// [`StatusEntry`] 的那张表）。调用方要拿它和仓库根求相对。
    pub target: String,
    /// 属性值原文，**一个字符都不动**。
    ///
    /// 不 trim、不按行 trim、不重排：`svn:externals` 有新旧两种格式，新格式是
    /// `URL 本地路径`、旧格式是 `本地路径 URL`，两者靠空格分列且可以混在同一个
    /// 属性里（实测五种写法共存）。解析层一旦试图分列就会把旧格式分反。
    /// 判定钉没钉住是 `engine::svn_externals` 的事。
    pub raw: String,
}

/// 解析 `svn propget svn:externals -R --xml`。
///
/// 只取 `name="svn:externals"` 的 property。真实的 `propget <属性名>` 只会回这
/// 一个，但调用方哪天改成 `proplist -v --xml` 就会混进 `svn:ignore` 之类——
/// 那时候把它们当 externals 解析会凭空判出一堆「没钉死的 external」。
///
/// 没有任何 externals 时 svn 回一个空的 `<properties>`（实测），这里返回空列表。
pub fn parse_externals_props(xml: &str) -> Result<Vec<ExternalsProp>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for target in doc.descendants().filter(|n| n.has_tag_name("target")) {
        let path = attr(&target, "path")?;
        for prop in target
            .children()
            .filter(|n| n.has_tag_name("property"))
            .filter(|n| n.attribute("name") == Some("svn:externals"))
        {
            out.push(ExternalsProp {
                target: path.clone(),
                // 属性被设成空串时没有文本子节点。空串是「这里没挂 external」，
                // 是合法状态，不报错。
                raw: prop.text().unwrap_or("").to_string(),
            });
        }
    }
    Ok(out)
}

/// `svn diff --summarize --xml` 的一条。
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    /// 仍然是 **URL**，剥前缀是调用方的事（spec §6.8.4）。
    pub path: String,
    /// 内容变更：`modified` / `added` / `deleted` / `none`。
    pub item: String,
    /// 属性变更：`modified` / `none`。
    pub props: String,
    /// `file` / `dir`。目录条目会混在结果里，调用方按需过滤。
    pub kind: String,
}

/// 解析 `svn diff --summarize --xml`。
///
/// # 为什么用 `--xml`
///
/// spec §6.8.4 用一整节讲文本版 `--summarize` 的解析坑：内容变更与属性变更靠
/// **列位置**区分，且输出里混着目录条目。2026-09-14 实测 `--summarize` 接受
/// `--xml`，`item` / `props` / `kind` 是三个独立属性，那个坑从结构上消失。
///
/// # ⚠️ 属性顺序不固定
///
/// 同一次输出里实测出现过两种顺序（`kind,item,props` 与 `item,props,kind`）。
/// 必须按名字取，任何按序取值的写法都会静默错位。
///
/// # 缺字段填空串而不是报错
///
/// 这里和 [`parse_info`] 的「缺字段一律报错」相反，因为**空串在下游是安全方向**：
/// 体检判「没有差异」的条件是 `item == "none" && props == "none"`，空串两个都不
/// 等于，于是缺字段的条目被当成「有差异」——保守地报出来，而不是悄悄放行。
pub fn parse_diff_summary(xml: &str) -> Result<Vec<DiffEntry>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for p in doc.descendants().filter(|n| n.has_tag_name("path")) {
        out.push(DiffEntry {
            path: p.text().unwrap_or("").to_string(),
            item: p.attribute("item").unwrap_or("").to_string(),
            props: p.attribute("props").unwrap_or("").to_string(),
            kind: p.attribute("kind").unwrap_or("").to_string(),
        });
    }
    Ok(out)
}

/// `svn log --xml` 的一条 entry。
///
/// # 为什么 `author` / `message` 缺失算空，而 `revision` 缺失算错
///
/// `revision` 是这条记录的**身份**：猜一个出来会让缓存和图上多一个不存在的节点。
/// 另外两个只是内容——匿名提交没有 `<author>`；`svn log -q` 的输出里 `<msg>`
/// **整个元素不出现**（2026-09-15 实测，不是空元素）。本期不用 `-q`，但把
/// 「缺失 = 空」钉死，是为了让将来有人为省流量加上 `-q` 时，不会得到一个
/// 「历史拉取整体失败」。
#[derive(Debug, Clone, PartialEq)]
pub struct SvnLogEntry {
    pub revision: u64,
    pub author: String,
    /// ISO8601 原样（`2026-09-15T15:21:22.337313Z`）。
    ///
    /// **不在这里解析成时间戳**：归一化留给前端，免得 Rust 侧和 TS 侧各有一套
    /// 时区处理，而图上显示的时间只由前端那套决定。
    pub date: String,
    pub message: String,
}

/// 解析 `svn log --xml`。
///
/// ⚠️ 属性逐行排列（`<logentry⏎   revision="3">`），行内正则会静默扫出空集，
/// 见本文件头部与 spec §6.6.4 陷阱三。必须走 DOM。
///
/// 顺序**原样保留**，不在这里重排：svn 默认降序，而区间方向会改变它
/// （`-r 2:3` 升序 / `-r 3:2` 降序，2026-09-15 实测）。排序是调用方按用途决定的事。
pub fn parse_log(xml: &str) -> Result<Vec<SvnLogEntry>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("logentry")) {
        let revision = node
            .attribute("revision")
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| {
                GitError::new(
                    "svn_xml_log_no_revision",
                    "svn log 的 logentry 缺少可解析的 revision 属性。revision 是这条\
                     记录的身份，猜不得——缺了只能整条拒绝。",
                )
            })?;
        out.push(SvnLogEntry {
            revision,
            author: joined_child_text(&node, "author"),
            date: joined_child_text(&node, "date"),
            message: joined_child_text(&node, "msg"),
        });
    }
    Ok(out)
}

/// `svn ls -R --xml` 的一条 entry。
#[derive(Debug, Clone, PartialEq)]
pub struct SvnListEntry {
    /// **相对 [`SvnListResult::base`] 的递归路径，正斜杠。**
    ///
    /// 2026-09-15 实测：target 换成子路径 URL 时，这里相对**那个子路径**重新
    /// 编号（`<url>/Code` → `A.cs`、`Deep`、`Deep/D.cs`），**不是**仓库绝对路径。
    /// 调用方必须自己接前缀。
    ///
    /// ⚠️ 同一个 CLI 在本模块里已经有三种路径写法了：`status` 的 `path` 属性是
    /// **反斜杠相对路径**、`diff --summarize` 的 `<path>` 是**完整 URL**、
    /// 这里是**相对基准 URL 的正斜杠路径**。不要凭印象套用。
    pub name: String,
    /// `"file"` 或 `"dir"`。**目录也是 entry**，调用方要自己筛。
    pub kind: String,
}

/// `svn ls -R --xml` 的解析结果。
pub struct SvnListResult {
    /// `<list path="…">` 的值，即本次列举的基准 URL（已去尾斜杠）。
    pub base: String,
    pub entries: Vec<SvnListEntry>,
}

/// 解析 `svn ls -R --xml`。
///
/// # ⚠️ 空清单在这里是合法的，判「是不是失败」不归本函数
///
/// 2026-09-15 实测：路径不存在时 svn **exit 1**，但 stdout 仍是一份结构完整的
/// 空 `<lists><list></list></lists>`，错误只在 stderr。而空仓库 `@0` 是
/// **exit 0** 加同样的空清单——两者在 stdout 上**一模一样**。
///
/// 若调用方忽略退出码，一个打错的 URL 就会得到空的服务器清单；而
/// 「服务器有、磁盘没有」的差集在空集上恒为空，于是**报告没有漏收**——正是
/// 本期要修的 NB-2 被重新造一遍。
///
/// 所以：调用方必须走 [`crate::engine::svn_cli::run_svn`]（非零退出即 `Err`），
/// **禁止**用 `run_svn_code` 自行判断。
pub fn parse_list(xml: &str) -> Result<SvnListResult> {
    let doc = parse_doc(xml)?;
    let list = doc
        .descendants()
        .find(|n| n.has_tag_name("list"))
        .ok_or_else(|| GitError::new("svn_xml_list_no_list", "svn ls 的输出里没有 <list> 元素"))?;
    let base = list
        .attribute("path")
        .unwrap_or("")
        .trim_end_matches('/')
        .to_string();
    let mut entries = Vec::new();
    // 只认 <list> 的**直接子元素** entry，且只取 entry 的直接子 <name>：
    // 每个 entry 里还嵌着 <commit><author>，全文 descendants 搜索会把别的捞进来。
    for node in list.children().filter(|n| n.has_tag_name("entry")) {
        let Some(name) = node.children().find(|c| c.has_tag_name("name")) else {
            continue;
        };
        let Some(text) = name.text() else {
            continue;
        };
        entries.push(SvnListEntry {
            name: norm_sep(text),
            kind: node.attribute("kind").unwrap_or("").to_string(),
        });
    }
    Ok(SvnListResult { base, entries })
}

/// 取某个直接子元素的全部文本，缺失时返回空串。
///
/// # 为什么拼接而不是 `.text()`，以及这条**当前没有测试证明其必要性**
///
/// `Node::text()` 只返回**第一个**文本子节点。原本担心 commit message 里的实体
/// （`&lt;`、`&amp;`）会把内容拆成多个文本节点、于是 `.text()` 静默截断。
///
/// **2026-09-15 变异测试证明这个担心在当前输入上不成立**：把本函数改回
/// `child.text()` 之后，含 `&lt;` `&amp;` 的多行 message 那条测试**照样通过**——
/// roxmltree 会把实体展开后仍合并成单个文本节点。
///
/// 保留拼接版是**防御**，防的是 CDATA、注释穿插这类真会产生多个文本节点的形态；
/// 代价是零。但要说清楚：**现有测试挡不住有人把它改回 `.text()`**，不要以为
/// 那条 message 测试在守着这件事。
///
/// CRLF → LF 的归一由 XML 规范保证（svn 在 Windows 上输出 CRLF，不归一的话每行
/// 结尾会多一个 `\r`，前端渲染成一串空行），这里不需要再处理——那一条**有**
/// 测试守着（`log_decodes_entities_and_normalizes_crlf_inside_the_message`）。
fn joined_child_text(node: &Node, tag: &str) -> String {
    let Some(child) = node.children().find(|n| n.has_tag_name(tag)) else {
        return String::new();
    };
    child
        .children()
        .filter_map(|n| n.text())
        .collect::<Vec<_>>()
        .concat()
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
