//! 白名单**递归覆盖到的全部**工作副本的枚举与 P4 校验。
//!
//! 为什么不是「对每个白名单根跑一次 `svn info`」：白名单根底下还可以再挂
//! external，那时只查根会拿到根所属仓库的 UUID，P4 干净通过，而一个**另一个
//! 仓库**的子树已经进了白名单——它会被 git 跟踪、进基线、进交付清单，却从不
//! 参与 `nodeRevs` 校验，最后在 D5 撞上 `E200007`（spec §6.5.5）。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::{parse_external_roots, parse_info};
use crate::error::{GitError, Result};

/// 一个工作副本的身份三元组。
#[derive(Debug, Clone, PartialEq)]
pub struct WcIdentity {
    /// 相对仓库根的路径，**正斜杠**。用它给用户报位置。
    ///
    /// ⚠️ 名字里的 `wc_root` 指的是「这条白名单条目/external 根」，**不是** SVN
    /// 意义上的工作副本根。后者是下面的 [`Self::wc_root_abspath`]。一条落在主
    /// 工作副本里的白名单（`Code`）两者完全不同：这里是 `Code`，那里是仓库根。
    pub wc_root: String,
    pub url: String,
    pub repos_root: String,
    /// `svn info` 的 `<wcroot-abspath>`：这个节点**所属工作副本的根**的绝对路径。
    ///
    /// P3 要用它。「操作没跑完」的痕迹（`wc-locked` / `item="incomplete"`）落在
    /// **工作副本根**上，而不是白名单根上——2026-09-14 实测，`svn cleanup` 之后
    /// 白名单根的 status 一条 entry 都没有，工作副本根却仍写着 `incomplete`。
    ///
    /// target 是 URL 形式时没有这一节，故为 `Option`。
    pub wc_root_abspath: Option<String>,
    /// 仓库身份的唯一可靠依据——URL 会因代理 / 镜像 / `http` 与 `https` 写法
    /// 不同而不可靠（spec §6.2.1）。
    pub uuid: String,
}

/// 枚举白名单递归覆盖到的全部工作副本。
///
/// 空白名单返回空列表，**一条 svn 命令都不发**（spec §12.2）。
pub fn enumerate_wcs(repo_root: &str, whitelist: &[String]) -> Result<Vec<WcIdentity>> {
    let mut out: Vec<WcIdentity> = Vec::new();
    let mut pending: Vec<String> = whitelist
        .iter()
        .map(|w| normalize(w))
        .filter(|w| !w.is_empty())
        .collect();

    while let Some(target) = pending.pop() {
        // 已经收过的子树不重复展开：两条白名单可能落在同一个 WC 里，
        // 而 external 也可能被两条白名单同时覆盖到。不去重会死循环。
        if out.iter().any(|w| w.wc_root == target) {
            continue;
        }
        // ⚠️ `svn info` **不接受** `--ignore-externals`（spec §4.4）。它默认也
        // 不遍历 externals，所以这里本来就不需要那个参数。
        let arg = peg_safe(&target);
        let xml = run_svn(repo_root, &["info", "--xml", "--", &arg])?;
        let entry = parse_info(&xml)?
            .into_iter()
            .next()
            .ok_or_else(|| GitError::new("svn_wc", format!("svn info 对 {target} 没有输出")))?;
        out.push(WcIdentity {
            wc_root: target.clone(),
            url: entry.url,
            repos_root: entry.repos_root,
            uuid: entry.uuid,
            wc_root_abspath: entry.wc_root,
        });

        // 找这棵子树里的嵌套 external 根。**这一次故意不带 `--ignore-externals`**
        // ——要的就是让它们出现（`item="external"`），见 spec §6.6.4 的枚举方式。
        let st = run_svn(repo_root, &["status", "--xml", "--", &arg])?;
        for rel in parse_external_roots(&st)? {
            // svn 在 Windows 上返回反斜杠（`Code\Vendor`，2026-09-13 实测）。
            // 不转换就和正斜杠形式的 `wc_root` 比不中，去重失效，这个 external
            // 会被反复塞回队列——解析层有意不做这层转换，见 `svn_xml` 的
            // `external_root_paths_come_back_with_windows_separators_verbatim`。
            let rel = normalize(&rel);
            if !out.iter().any(|w| w.wc_root == rel) && !pending.contains(&rel) {
                pending.push(rel);
            }
        }
    }
    // 顺序稳定，便于测试与报错信息可复现
    out.sort_by(|a, b| a.wc_root.cmp(&b.wc_root));
    Ok(out)
}

/// 把一个工作副本内的路径变成安全的 svn target：**追加一个空的 peg revision
/// 后缀 `@`**。
///
/// SVN 把 target 里**最后一个** `@` 之后的部分当 peg revision 解析，而 `--`
/// 只管「这之后不是选项」，**挡不住 peg 解析**。`Code/Team@Home` 是一个完全
/// 合法的目录名、也过得了配置校验（`@` 不是通配符），送进去却是：
///
/// | 命令 | 报错 |
/// |---|---|
/// | `svn info -- Code/Team@Home` | `E205000: Syntax error parsing peg revision 'Home'` |
/// | `svn status -- Code/Team@Home` | `E200009: a peg revision is not allowed here` |
///
/// 两条 `enumerate_wcs` 都要发，所以整个白名单条目直接不可用。
///
/// # 为什么无条件追加，而不是「含 `@` 才加」
///
/// 2026-09-14 实测：**不含 `@` 的普通路径追加尾缀同样合法**（`svn info -- Code@`
/// exit 0，`<url>` 里也没有多出来的 `@`）。无条件追加因此没有代价，却省掉了
/// 「哪天新增一个 target 构造点忘了判断」这个缺口——条件分支是会被忘掉的，
/// 恒等变换不会。
///
/// ⚠️ 只用在**命令行参数**上。`wc_root` 这类内部键必须保持原始路径：带上尾缀
/// 会让去重比对失效，也会原样显示给用户。
///
/// # ⚠️ `svn diff` 的尾随 target 是例外，**不要**用本函数
///
/// 上面「无条件追加没有代价」那条结论是在 `info` / `status` 上验的。2026-09-14
/// 实测 `svn diff --old=… --new=. -- Code@` 报
/// `E155010: The node '…\wc\Code@' was not found` —— 带 `--old` / `--new` 时
/// 尾随 target 是**纯路径**，`@` 被当成字面字符，不走 peg 解析。
///
/// 于是两边的要求正好相反：`enumerate_wcs` 必须加后缀，`svn_health` 必须不加。
/// 连带的已知限制是白名单路径含 `@` 时体检没法跑，见 `svn_health::check_baseline`。
pub fn peg_safe(target: &str) -> String {
    format!("{target}@")
}

/// 统一成正斜杠、去掉尾部 `/`。
///
/// 这里去尾斜杠是对的：`wc_root` 是拿来当 svn target 和做相等比较的，
/// `Code` 与 `Code/` 必须是同一个键。配置侧保留尾斜杠是另一回事——那边它承载
/// 「目录 vs 单文件」的语义（spec §6.3），两处不要互相套用。
fn normalize(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

/// P4：所有工作副本必须属于**同一个仓库**。
///
/// 理由不是洁癖：`svn commit` 对跨仓库的 target 列表**动手之前就拒绝**
/// （`E200007`，spec §6.6.5 E5），交付永远做不成。前置拦住是为了不让用户
/// 白走完整个确认流程才在 D5 撞墙。
pub fn check_single_repository(wcs: &[WcIdentity]) -> Result<()> {
    let Some(first) = wcs.first() else {
        return Ok(());
    };
    if let Some(bad) = wcs.iter().find(|w| w.uuid != first.uuid) {
        return Err(GitError::new(
            "svn_cross_repo",
            format!(
                "白名单跨了两个仓库：{} 属于 {}，而 {} 属于 {}。\
                 svn commit 对跨仓库的 target 列表会直接拒绝（E200007），交付做不成。",
                first.wc_root, first.uuid, bad.wc_root, bad.uuid
            ),
        ));
    }
    Ok(())
}
