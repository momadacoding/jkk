//! §6.8 基线体检：让基线可被服务器证伪。
//!
//! 基线记着「上次与 SVN 对齐时是 rN」。这条检查不去推断 revision 号对不对，
//! 而是**直接拿磁盘跟服务器上的 rN 比**——有内容差异即证伪，没有就说明基线
//! 至少在内容上站得住。
//!
//! # 它证明不了什么（spec §6.8.3）
//!
//! 比的是**磁盘**与服务器，不是**基线树**与服务器，两者之间隔着「git 跟踪的集合
//! == 白名单内应跟踪的集合」这个假设。那个假设归 `svn_include_check` 的纳入集合
//! 核对管。两条检查互不替代，不要互相冒充。
//!
//! # 前提
//!
//! 调用时必须满足「磁盘上白名单内的内容 == 基线树的内容」。spec §6.8.5 列的四个
//! 时机全部满足它：HEAD 在 `svn/base` 上且工作区干净，或基线树刚从磁盘录出来。

use crate::engine::svn_cli::run_svn;
// W-file 判据原本是本模块的私有函数，Phase 1c 提到 `svn_include_check` 共用，
// 2026-10-28 再收进 `svn_scope`——`svn_server_set` 等要用**同一套**判据。
// 两份「什么算 W-file」迟早分叉，而分叉的表现是「每次打开都报几千条漏收」或者
// 「永远报没问题」，两头都很难查。
use crate::engine::svn_scope::WScope;
use crate::engine::svn_xml::parse_diff_summary;
use crate::error::{GitError, Result};
use std::collections::BTreeSet;

/// 体检的三种结果。**必须分开**（spec §6.8.6）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Health {
    /// 命令成功，按 §6.8.4 过滤后无 W-file 内容差异 —— 基线可信。
    Verified {
        rev: u64,
        /// 这次检查**覆盖不到**的路径（§6.8.7 的 `S` 里那些）。空表示全覆盖。
        ///
        /// # 为什么必须报出去
        ///
        /// 判据是 `D \ S == ∅`，而 `D` 比的是**磁盘与服务器**——`S` 里那些路径上
        /// 的磁盘内容正是用户改过的，所以这条检查对它们什么也证明不了。工作副本
        /// 干净时 `S = ∅`，覆盖面才完整；而「程序员的工作区 99% 的时间都是脏的」
        /// 正是 §16.1 放松干净要求的理由，于是**不完整才是常态**。
        ///
        /// 2026-09-21 独立 review：后端一直知道这批路径，前端却无条件显示
        /// 「磁盘内容与服务器上该 revision 一致」。那句话在 `S ≠ ∅` 时对这批路径
        /// 就是假的。把它们带出去，文案才能如实说。
        unverified: Vec<String>,
    },
    /// 命令成功，仍有 W-file 内容差异 —— **基线撒谎**。阻断并列出差异文件。
    Lying { files: Vec<String> },
    /// 命令没跑成（断网 / 认证 / URL 不可达 / revision 不存在）。
    ///
    /// **这不是「基线有问题」。** 把「没能验证」当成「验证不通过」会让离线场景
    /// 寸步难行；当成「验证通过」则等于这条检查根本不存在。所以它是独立的第三种。
    NotChecked { reason: String },
}

/// `S`（`explained`）里那些路径的**基线内容**是哪来的。
///
/// 这条检查比的是磁盘与服务器，对 `S` 里的路径什么也证明不了；但「证明不了」
/// 不等于「不可信」——要看基线上那些 blob 是谁写的。所以每个调用点必须表态，
/// 不给默认值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplainedOrigin {
    /// **刚刚由本次录制写进去的**（初始化 / 重建，spec §16 R2b）。
    ///
    /// 那些 blob 是几毫秒前从 `svn cat -r BASE` 取的字节，而 P2 要求
    /// `nodeRevs` 单值、`rev` 就取的那个值——于是 `BASE == rev`，pristine 内容
    /// 就是服务器 `rev` 上的内容。这批路径**由构造保证正确**，不需要再声明未覆盖。
    JustRecorded,
    /// **不知道是谁、什么时候写的**（用户手动点体检）。
    ///
    /// 基线可能是老版本录的、可能有人手工在 `svn/base` 上提交过。这条检查覆盖
    /// 不到它们，必须如实报出去。
    Unknown,
}

/// 拿磁盘跟服务器上的 `rev` 比一次。
///
/// # 命令形态（spec §6.8.2）
///
/// ```text
/// svn diff --old=<svn_url>@<rev> --new=. --summarize --xml -- <白名单根...>
/// ```
///
/// - **用 `--xml`**：文本版靠列位置区分「内容变更」与「属性变更」，还会混进目录
///   条目（spec §6.8.4）。`--xml` 把这三件事拆成 `item` / `props` / `kind` 三个
///   独立属性，那个解析坑从结构上消失。
/// - **不带 `--ignore-externals`**：`svn diff` 会以 exit 1 拒绝它，而且**不需要**
///   ——URL 与工作副本目录比对时本来就不走进 externals（spec §6.8.2 R3）。
/// - **白名单根不过 `peg_safe`**。这是与 `info` / `status` 相反的一条，见下。
///
/// # ⚠️ `svn diff` 的尾随 target 不接受空 peg 后缀
///
/// 2026-09-14 实测：`svn diff --old=… --new=. -- Code@` 报
/// `E155010: The node '…\wc\Code@' was not found` —— 它把 `@` 当成了字面路径的
/// 一部分，而不是 peg 记号。`svn_wc::peg_safe` 的「无条件追加没有代价」那条结论
/// 是在 `info` / `status` 上验的，**对这里不成立**。
///
/// 代价：白名单路径里含 `@` 时这条命令没法用。那种路径本来就过不了
/// `enumerate_wcs`（它反过来**必须**加后缀），所以两边都能工作的写法目前不存在，
/// 记为已知限制。
pub fn check_baseline(
    repo_root: &str,
    svn_url: &str,
    rev: u64,
    // W-file 口径。「尊重 .gitignore」时被它排除、又不在基线里的文件不参与比对。
    scope: &WScope,
    // `explained` 是 §6.8.7 的 `S`——本次录制**实际动过**的路径。这些路径上的差异
    // 已经被换成了服务器内容，不构成基线撒谎。
    //
    // ⚠️ **必须恰好是动过的那批，不能是超集。** `S` 越大 `D \ S` 越小，判据越弱，
    // 而削弱它不会让任何测试变红（spec §16.1）。干净工作副本传空集，判据退化成
    // 原来的 `D == ∅`。
    explained: &BTreeSet<String>,
    // `explained` 里那些路径的基线内容是谁写的。决定 `Verified` 要不要把它们
    // 报成「未覆盖」，见 [`ExplainedOrigin`]。
    origin: ExplainedOrigin,
) -> Result<Health> {
    let roots = scope.roots();
    if roots.is_empty() {
        // 不传 target 时 `svn diff` 会拿**整个工作副本**跟服务器比。对一个 Unity
        // 工程那是几个 GB 的美术资产，而且任何一处正常的本地改动都会报成基线撒谎。
        // 返回 Verified 更糟——等于宣布一个空白名单的基线是健康的。
        return Err(GitError::new(
            "svn_health_empty_whitelist",
            "白名单是空的，体检没有比对范围。不传 target 会让 svn diff 拿整个\
             工作副本去比，既慢又必然误报。",
        ));
    }

    let base = svn_url.trim_end_matches('/');
    if let Some(reason) = url_mismatch(repo_root, base)? {
        return Ok(Health::NotChecked { reason });
    }
    if let Some(reason) = whitelist_not_under_base(repo_root, base, roots)? {
        return Ok(Health::NotChecked { reason });
    }

    // §6.6.5：**每个 `wc(P)` 各体检一次，用它自己的 `url(P)`**。
    //
    // 1b 的做法是从项目根跑**一次** diff，并在白名单底下有 external 时整个降级成
    // `NotChecked`。那是因为 `--old` 那侧是 URL，而 URL 上根本没有 `Code/Vendor`
    // 这个节点（它只是 `Code` 上的一条 `svn:externals` 属性），diff 直接不下沉；
    // 而 `git add -- Code` 会把里面的文件照收进基线，于是 `Verified` 会覆盖一块
    // 从没比对过的内容。
    //
    // 现在按记账单位分开发命令：`enumerate_wcs` 返回的每一条都自带 `url`，
    // 嵌套 external 用它**自己的** URL 去比，那块内容第一次真的被服务器验过。
    let units = crate::engine::svn_wc::enumerate_wcs(repo_root, scope.whitelist())?;
    let mut files: Vec<String> = Vec::new();
    for unit in &units {
        let unit_url = unit.url.trim_end_matches('/');
        let old = format!("--old={unit_url}@{rev}");
        // `--new` 直接指向这个单位，不再用 `--new=.` + 尾随 target。
        //
        // ⚠️ 这里**不能**过 `peg_safe`：带 `--old` / `--new` 时路径是纯字面量，
        // 追加 `@` 会得到 `E155010: The node '…Code@' was not found`（1b 实测）。
        let new = format!("--new={}", unit.wc_root);
        let args: Vec<&str> = vec!["diff", &old, &new, "--summarize", "--xml"];

        // 命令失败一律落到 NotChecked，**不能**当成「没有差异」。
        //
        // 不能用「解析得出来就算成功」：2026-09-14 实测，URL 不可达时 svn 仍然
        // 往 stdout 打出 `<?xml …?>` 和 `<diff><paths>`，错误只在 stderr 和退出码里。
        // 照那种写法，断网时会得到一个空的 paths 列表，于是**报告基线健康**。
        //
        // 一个单位失败就整体 NotChecked，**不能**「其余都干净所以 Verified」：
        // `Verified` 是一句主动的保证，不是「没发现问题」。
        let xml = match run_svn(repo_root, &args) {
            Ok(x) => x,
            Err(e) => {
                return Ok(Health::NotChecked {
                    reason: format!("比对 `{}` 时失败：{}", unit.wc_root, e.message),
                })
            }
        };

        let prefix = format!("{unit_url}/");
        for entry in parse_diff_summary(&xml)? {
            // §6.8.4 的三条过滤，靠 --xml 的三个独立属性完成：
            //   剔除目录条目
            if entry.kind != "file" {
                continue;
            }
            //   剔除纯属性变更（item="none" props="modified"）
            if entry.item == "none" {
                continue;
            }
            // 单文件白名单条目：`--new` 就是那个文件本身，svn 打出来的路径恰好
            // 等于 `unit_url`，没有尾随的子路径可剥。
            let repo_rel = if entry
                .path
                .trim_end_matches('/')
                .eq_ignore_ascii_case(unit_url)
            {
                unit.wc_root.clone()
            } else if let Some(sub) = strip_prefix_ci(&entry.path, &prefix) {
                format!("{}/{}", unit.wc_root, sub)
            } else {
                return Ok(Health::NotChecked {
                    reason: format!(
                        "svn diff 输出里的路径 `{}` 不在本次比对的基准 URL `{unit_url}` 之下，\
                         没法还原成仓库相对路径。这是一个本应用没有预料到的输出形状，\
                         为安全起见判为「未经体检」——既不宣称基线健康，也不指控它撒谎。",
                        entry.path
                    ),
                });
            };
            files.push(repo_rel);
        }
    }
    //   剔除不是 W-file 的（扩展名不在白名单里、或被尊重的 .gitignore 排除）。
    //   整批问一次：「尊重」时每次判定都是一次 git 子进程。
    let mut files: Vec<String> = scope
        .filter(files.iter().map(String::as_str))?
        .into_iter()
        .collect();

    // §6.8.7：判据是 `D \ S == ∅`，不是 `D == ∅`。
    //
    // `D` 是刚刚收集的这批「磁盘与服务器 rN 有内容差异的 W-file」；`S`（`explained`）
    // 是本次录制实际动过的路径——它们的差异**已经被换成服务器内容**，不构成
    // 基线撒谎。剩下解释不了的每一条，才是真的证伪。
    //
    // 工作副本干净时 `S = ∅`，这一步退化成原来的 `D == ∅`，与 §6.8.2 逐字等价。
    files.retain(|f| !explained.contains(f));

    if files.is_empty() {
        // `S` 里的路径被上面那行**整批豁免**了，所以它们一条都没被验证。
        // 报不报出去看它们是谁写的——见 [`ExplainedOrigin`]。
        let unverified = match origin {
            ExplainedOrigin::JustRecorded => Vec::new(),
            ExplainedOrigin::Unknown => explained.iter().cloned().collect(),
        };
        return Ok(Health::Verified { rev, unverified });
    }
    files.sort();
    files.dedup();
    Ok(Health::Lying { files })
}

/// `svn_url` 必须就是**工作副本根**的 URL；不是的话返回一句说得清的理由。
///
/// # 为什么值得多发一条 `svn info`
///
/// `--old=<svn_url>@<rev>` 与 `--new=.` 是一对：svn 把每个 target 分别接在两边
/// 后面去比。`svn_url` 若配成了仓库里的**子路径**（比如把白名单根的 URL 填了
/// 进来），`--old` 那侧指向 `<svn_url>/Code`，一个根本不存在的路径，于是白名单里
/// 每个文件都被报成差异。
///
/// 2026-09-14 实测：这种情况下**前缀照样剥得掉**（输出路径就是
/// `<svn_url>/<target>/…`），所以「剥不掉前缀」那道守卫拦不住它。结果是一份
/// 看起来完全合理的 `Lying` 清单——而按 spec §6.8.6，初始化时 `Lying` 会触发
/// **整个回退**。把一个配置错误变成「基线撒谎」是这条检查最坏的失败形态。
///
/// 判定不成立时返回 `NotChecked` 而不是 `Err`：这确实属于「没能验证」，
/// 而且不该阻断（spec §6.8.6 第三行）。
fn url_mismatch(repo_root: &str, base: &str) -> Result<Option<String>> {
    let xml = match run_svn(repo_root, &["info", "--xml"]) {
        Ok(x) => x,
        // 连 `svn info` 都跑不成，后面的 diff 更不可能成。归到同一个桶里。
        Err(e) => return Ok(Some(e.message)),
    };
    let Some(entry) = crate::engine::svn_xml::parse_info(&xml)?.into_iter().next() else {
        return Ok(Some("svn info 对工作副本根没有输出".to_string()));
    };
    let actual = entry.url.trim_end_matches('/');
    if actual.eq_ignore_ascii_case(base) {
        return Ok(None);
    }
    Ok(Some(format!(
        "配置里的 svnUrl 是 `{base}`，而工作副本根 `{repo_root}` 实际指向 `{actual}`。\
         体检要拿这两者当同一个位置的两侧来比，对不上就没法验证。\
         svnUrl 应当填工作副本根的 URL，不是某个白名单目录的 URL。"
    )))
}

/// 每个白名单根的实际 URL 必须就是 `<base>/<相对路径>`；不是的话体检没法做。
///
/// # 两种会走到这里的真实形态（2026-09-14 实测）
///
/// **一、白名单子树被 `svn switch` 到了别的分支。** 这是 SVN 的常规操作，
/// 而且**四道前置门槛一个都拦不住**：`svn status` 对「target 就是被 switch 的那个
/// 目录」这种情况一条 entry 都不出（P1 过）、`nodeRevs` 单值（P2 过）、
/// 工作副本干净（P3 过）、同一个仓库（P4 过），XML 里连 `switched` 属性都没有。
///
/// 此时 `--old=<base>@N` 指向的是 trunk 的那份内容，而磁盘上是分支的内容，
/// 体检会报出一串 `modified` —— 一个**完全合法**的工作副本被判成「基线撒谎」，
/// 而按 spec §6.8.6 初始化时 `Lying` 会触发整个回退，给出的引导还是无效的。
///
/// **二、白名单根本身就是一条 external。** 那个路径在 `<base>` 底下根本不存在
/// （实测 `E160005: Target path '/Proj/Code' does not exist`），命令直接失败。
/// 走到这里可以给一句说得清的话，而不是把 svn 的英文错误码丢给用户。
///
/// 两种都属于「没能验证」而不是「验证不通过」（spec §6.8.6 第三行）：
/// 基线本身没问题（它录的是磁盘内容配 `nodeRevs`），只是**这条检查的基准 URL
/// 不适用**。所以返回 `NotChecked` 而不是 `Lying`，也不阻断。
fn whitelist_not_under_base(
    repo_root: &str,
    base: &str,
    roots: &[String],
) -> Result<Option<String>> {
    for root in roots {
        let arg = crate::engine::svn_wc::peg_safe(root);
        // 单个 target 一条命令：白名单通常只有个位数条目，而且这是降级路径不是热路径
        let xml = match run_svn(repo_root, &["info", "--xml", "--", &arg]) {
            Ok(x) => x,
            Err(e) => return Ok(Some(e.message)),
        };
        let Some(entry) = crate::engine::svn_xml::parse_info(&xml)?.into_iter().next() else {
            return Ok(Some(format!("svn info 对白名单根 `{root}` 没有输出")));
        };
        let actual = entry.url.trim_end_matches('/');
        let expected = format!("{base}/{root}");
        if !actual.eq_ignore_ascii_case(&expected) {
            return Ok(Some(format!(
                "白名单根 `{root}` 的实际 URL 是 `{actual}`，而体检要拿 `{expected}` 当基准。\
                 最常见的成因是这个目录被 `svn switch` 到了别的分支，或者它本身是一条 external。\
                 两种情况下基线本身都没问题（它录的是磁盘内容配 nodeRevs），\
                 只是这条检查的基准 URL 不适用，所以判为「未经体检」而不是「基线撒谎」。"
            )));
        }
    }
    Ok(None)
}

// ─────────────────────────────────────────────────────────────────────────
// 已删除：`nested_external_not_covered`（1b 引入）
//
// 它的职责是「白名单底下有 external 时，别宣称 Verified」——因为当时从项目根只跑
// **一次** `svn diff`，而 `--old` 那侧是 URL，URL 上没有 `Code/Vendor` 这个节点
// （它只是 `Code` 上的一条 `svn:externals` 属性），diff 不下沉进去。
//
// Phase 1c 把体检改成**按 `enumerate_wcs` 的每个记账单位各发一条命令、各用自己的
// `url(P)`**（spec §6.6.5），嵌套 external 那块内容现在真的被服务器验过了，
// 所以这道「不宣称」的降级不再需要——它会把一个已经验过的干净仓库判成未验。
//
// 反向证据由 `a_lying_baseline_inside_a_nested_external_is_caught` 提供：
// 少了它，「见到 external 就返回 Verified」也能让干净那条变绿。
// ─────────────────────────────────────────────────────────────────────────

/// 大小写不敏感地剥前缀。`file:///C:/…` 里盘符的大小写不保证与配置里一致。
fn strip_prefix_ci(path: &str, prefix: &str) -> Option<String> {
    if path.len() <= prefix.len() {
        return None;
    }
    // 按字节比，避免在多字节字符中间切片。前缀末尾是 `/`，字节段匹配时
    // prefix.len() 必定落在字符边界上。
    if path.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()) {
        Some(path[prefix.len()..].to_string())
    } else {
        None
    }
}
