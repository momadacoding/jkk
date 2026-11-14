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

use crate::engine::svn_cli::{
    chunk_targets, cmdline_estimate, in_scan_cache, prefetch_scan, run_svn, run_svn_code,
    run_svn_targets, ScanAhead, MAX_TARGETS_CMDLINE,
};
// W-file 判据原本是本模块的私有函数，Phase 1c 提到 `svn_include_check` 共用，
// 2026-10-28 再收进 `svn_scope`——`svn_server_set` 等要用**同一套**判据。
// 两份「什么算 W-file」迟早分叉，而分叉的表现是「每次打开都报几千条漏收」或者
// 「永远报没问题」，两头都很难查。
use crate::engine::svn_basemap::BaseMap;
use crate::engine::svn_nodes::{read_node_status, NodeStatus};
use crate::engine::svn_scope::WScope;
use crate::engine::svn_tombstone::fault::{self, Point};
use crate::engine::svn_tombstone::{
    confirm_absent, decode_url, encode_url_path, error_codes, server_head_rev,
};
use crate::engine::svn_wc::WcIdentity;
use crate::engine::svn_xml::{parse_diff_summary, parse_excluded_nodes, parse_list};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};

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
    // 单值包装（计划 Task 4）：映射只有 floor，并且**不产生墓碑**——floor 组里没被解释的
    // D 照旧算差异。于是发出的命令（每个单元一条整单元 diff，不带 `--ignore-properties`）
    // 与判据都与改造前逐字相同。`repos_root` / `uuid` 只有产生墓碑时才用，这里留空。
    let map = BaseMap::single(String::new(), String::new(), rev);
    let opts = Grouped {
        tombstoning: false,
        nodes: None,
    };
    match check_grouped(repo_root, svn_url, &map, scope, explained, origin, opts)? {
        HealthMap::Verified { health, .. } => Ok(health),
        // 不产生墓碑就不会去问服务器，走不到这里；万一走到，按「撒谎」报，不放行
        HealthMap::Excluded { files } | HealthMap::Unconfirmed { files, .. } => {
            Ok(Health::Lying { files })
        }
    }
}

/// 混合基线的体检结论。
#[derive(Debug, Clone, PartialEq)]
pub enum HealthMap {
    /// 没有漏收。`health` 是照常的三种结论之一（`Verified` / `Lying` / `NotChecked`）。
    Verified {
        health: Health,
        /// 只在 [`ExplainedOrigin::JustRecorded`] 时产生：floor 组里没被墓碑解释、也不在 S
        /// 里的 D，向服务器 HEAD 确认已不存在的，成为新墓碑（值 = 当时的 HEAD）。
        /// `health` 不是 `Verified` 时恒为空。
        new_gone: BTreeMap<String, u64>,
    },
    /// 这些路径服务器 HEAD 上仍然存在，工作副本里却没有（漏收，E17），调用方应拒绝录制
    /// （`svn_baseline_excluded_paths`）。
    Excluded { files: Vec<String> },
    /// 只在 [`ExplainedOrigin::JustRecorded`] 时产生：有该向服务器确认的东西，却没确认成——
    /// 读不到服务器 HEAD、`svn info` 核对失败、找不出被排除的节点、列不出被排除节点在 floor
    /// 上的内容。调用方必须拒绝录制（`svn_baseline_unconfirmed`）。
    ///
    /// # 为什么不能是 `NotChecked`（Review 1 第 1 条）
    ///
    /// 录制接受 `NotChecked`（离线时照样能录）。可这里 `files` 是**已知**的、floor 上有而
    /// 工作副本里没有的文件：改造前同一状态是 Lying（拒绝）；降成 NotChecked 放行，发布的
    /// 基线就静默丢了它们，也没有墓碑——比改造前更差。
    ///
    /// `files`：待确认的路径（floor 组没被解释的 D、被排除节点列出来的 W-file；列不出内容的
    /// 被排除节点报节点本身）；扫描被排除节点那一步就失败时为空。
    Unconfirmed { files: Vec<String>, reason: String },
}

/// 按映射分组体检（分析 §6.4，命令形状照日志 E18）。
///
/// - floor 组：每个单元一条整单元 `svn diff --old=<unit_url>@<F> --new=<wc_root>`，
///   结果里剔除映射单列的文件（它们归各自的组）；映射不是单值时加 `--ignore-properties`
///   去掉根目录那条假的属性差异
/// - 其余每个 revision 组：`--old=<unit_url>@<r> --new=<wc_root> -- <PATH>...`，PATH 相对单元根，
///   按命令行长度分批（[`MAX_DIFF_CMDLINE`]）；未版本化 / 丢失的 PATH 发之前剔除
///   （前者让整条命令 E155010；后者 svn 本来就不报，内容由 S 负责）
/// - 墓碑：`<unit_url>/<rel>@<d>` 必须不存在，否则 `Lying`
/// - `JustRecorded` 时，floor 组没被解释的 D 向服务器 HEAD 确认：不存在 → `new_gone`；
///   仍存在 → [`HealthMap::Excluded`]；确认不成 → [`HealthMap::Unconfirmed`]。
///   `--set-depth=exclude` 掉的节点整单元 diff 不报，另用 `svn info -R` 找出来、点名再比一次
///   （点不了名的用 `svn ls -R` 列出 floor 上的内容），让它们也走这条确认
///
/// 判据：各组 `D \ S` 的并集为空，且墓碑全部成立。
pub fn check_baseline_map(
    repo_root: &str,
    svn_url: &str,
    map: &BaseMap,
    scope: &WScope,
    explained: &BTreeSet<String>,
    origin: ExplainedOrigin,
) -> Result<HealthMap> {
    check_baseline_map_with_nodes(repo_root, svn_url, map, scope, explained, origin, None)
}

/// 同 [`check_baseline_map`]，调用方已经读过节点状态时传进来，省掉剔除尾随 PATH 那一次
/// `svn status -v`（录制 R2 刚读过，计划 Task 5）。`None` 时自己读。
pub(crate) fn check_baseline_map_with_nodes(
    repo_root: &str,
    svn_url: &str,
    map: &BaseMap,
    scope: &WScope,
    explained: &BTreeSet<String>,
    origin: ExplainedOrigin,
    nodes: Option<&[NodeStatus]>,
) -> Result<HealthMap> {
    let opts = Grouped {
        tombstoning: origin == ExplainedOrigin::JustRecorded,
        nodes,
    };
    check_grouped(repo_root, svn_url, map, scope, explained, origin, opts)
}

/// [`check_grouped`] 的两个开关。
#[derive(Clone, Copy)]
struct Grouped<'a> {
    /// 为假时不产生墓碑、不问服务器 HEAD、不扫被排除的节点（单值包装 [`check_baseline`]）。
    tombstoning: bool,
    /// 调用方已读过的节点状态；`None` 时需要就自己读。
    nodes: Option<&'a [NodeStatus]>,
}

/// 尾随 PATH 那批命令的命令行上限（字符数按 UTF-8 字节估，偏保守）。
///
/// Windows 的命令行上限是 32767 个 UTF-16 单元，留出余量给 svn 路径、引号与全局参数。
/// `svn diff` 没有 `--targets`（分析 §6.4），只能分批。与多目标批量同一个上限。
pub const MAX_DIFF_CMDLINE: usize = MAX_TARGETS_CMDLINE;

/// 一条体检用的 `svn diff` 负责哪些结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffScope {
    /// floor 组：结果里剔除映射单列（`revs`）的文件，其余都算；D 可能成为墓碑。
    Floor,
    /// 只看这些文件（仓库相对路径），其余结果属于别的组。
    Only(BTreeSet<String>),
}

/// 将要发出的一条 `svn diff`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffCommand {
    /// 所属单元的 `wc_root`（仓库相对）。
    pub wc_root: String,
    /// 所属单元的 URL（去掉尾部 `/`），结果路径要剥掉它。
    pub unit_url: String,
    pub rev: u64,
    /// 传给 `run_svn` 的参数（不含 `--non-interactive`，那由 `run_svn` 注入）。
    pub args: Vec<String>,
    pub scope: DiffScope,
    /// 非空 = 这是一条把多个单元的 floor diff 合在一起的命令（[`plan_diffs_for`]），这里是它
    /// 替代的逐单元命令。合并命令失败时按顺序改跑它们，失败语义就与改动前相同。
    /// 合并命令的 `wc_root` 是空串、`unit_url` 是 svnUrl。
    pub fallback: Vec<DiffCommand>,
}

/// [`plan_diffs`] 的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffPlan {
    pub commands: Vec<DiffCommand>,
    /// 被排除的节点里没法用尾随 PATH 点名的（就是单元根本身，或路径含非 ASCII 字符，
    /// 见 [`plan_diffs`]）。调用方改用 `svn ls -R <url>@<F>` 列出内容再按 W-file 筛（D8）。
    pub unaddressable: Vec<String>,
}

/// 由映射算出体检要发的全部 `svn diff`。纯函数，不发命令。
///
/// - `skip`：要剔除的 PATH（未版本化 / 丢失），仓库相对
/// - `excluded`：`--set-depth=exclude` 掉的节点，仓库相对；用 floor revision 点名比一次
///
/// 映射是单值、`excluded` 为空时，结果就是每个单元一条与改造前完全相同的命令。
///
/// # 为什么含非 ASCII 字符的 PATH 不走尾随参数
///
/// 2026-10-31 实测（SlikSvn 1.14.2，系统代码页 1252）：`svn diff ... -- "sp ace/中文.cs"`
/// 报 `E155010: The node '…\sp ace\?? .cs' was not found`——svn.exe 按系统代码页取命令行
/// 参数，代码页表示不了的字符变成 `?`。这类文件改走整单元 diff、只看它们的结果，
/// 慢一些，但不依赖代码页。
pub fn plan_diffs(
    units: &[WcIdentity],
    map: &BaseMap,
    skip: &BTreeSet<String>,
    excluded: &BTreeSet<String>,
) -> Result<DiffPlan> {
    let mixed = !map.is_single();
    let mut commands = Vec::new();
    for u in units {
        let url = u.url.trim_end_matches('/');
        let mut args = vec![
            "diff".to_string(),
            format!("--old={url}@{}", map.floor),
            format!("--new={}", u.wc_root),
            "--summarize".to_string(),
            "--xml".to_string(),
        ];
        if mixed {
            args.push("--ignore-properties".to_string());
        }
        commands.push(DiffCommand {
            wc_root: u.wc_root.clone(),
            unit_url: url.to_string(),
            rev: map.floor,
            args,
            scope: DiffScope::Floor,
            fallback: Vec::new(),
        });
    }

    // (单元, revision) → [(仓库相对路径, 相对单元根)]
    let mut groups: BTreeMap<(usize, u64), Vec<(String, String)>> = BTreeMap::new();
    for (p, &r) in &map.revs {
        if skip.contains(p) {
            continue;
        }
        let (i, rel) = unit_of(units, p).ok_or_else(|| outside_units(p))?;
        groups.entry((i, r)).or_default().push((p.clone(), rel));
    }
    for ((i, r), files) in groups {
        let u = &units[i];
        let (pathable, whole): (Vec<_>, Vec<_>) = files
            .into_iter()
            .partition(|(_, rel)| !rel.is_empty() && rel.is_ascii());
        let head = group_head(u, r);
        if !whole.is_empty() {
            commands.push(DiffCommand {
                wc_root: u.wc_root.clone(),
                unit_url: u.url.trim_end_matches('/').to_string(),
                rev: r,
                args: head[..head.len() - 1].to_vec(),
                scope: DiffScope::Only(whole.into_iter().map(|(p, _)| p).collect()),
                fallback: Vec::new(),
            });
        }
        push_batches(&mut commands, u, r, head, pathable, false);
    }

    let mut unaddressable = Vec::new();
    let mut ex_by_unit: BTreeMap<usize, Vec<(String, String)>> = BTreeMap::new();
    for p in excluded {
        match unit_of(units, p) {
            Some((i, rel)) if !rel.is_empty() && rel.is_ascii() => {
                ex_by_unit.entry(i).or_default().push((p.clone(), rel));
            }
            _ => unaddressable.push(p.clone()),
        }
    }
    for (i, files) in ex_by_unit {
        let u = &units[i];
        push_batches(
            &mut commands,
            u,
            map.floor,
            group_head(u, map.floor),
            files,
            true,
        );
    }
    Ok(DiffPlan {
        commands,
        unaddressable,
    })
}

/// 同 [`plan_diffs`]，再把 URL 就在 `svn_url` 底下的单元的 floor diff 合成一条
/// （计划 2026-11-09 Task 4）：
///
/// ```text
/// svn diff --old=<svn_url>@<F> --new=. --summarize --xml [--ignore-properties] -- <wc_root...>
/// ```
///
/// # 为什么
///
/// 这一步问服务器。公司机器上 23 个白名单目录逐个比一次跟进要 33.3 s，单目录最长 6.3 s；
/// 每条命令都要单独起进程、连服务器、认证。
///
/// # 为什么结论相同
///
/// 实测 M5：`--old=<root>@N --new=. -- Code Tools` 的结果就是逐单元
/// `--old=<root>/Code@N --new=Code` 结果的并集，同样不下沉进 external；旧 revision 上没有的
/// 单元报 `added` 而不是失败（M6）。输出路径都是 `<svn_url>/<wc_root>/…`，剥掉 `svn_url`
/// 就是仓库相对路径。
///
/// 只合并：单元的**实际** URL 恰好是「同一个前缀 + `/` + `wc_root`」（逐字节相同，大小写也要
/// 一致），且这个前缀与 svnUrl 只差大小写；`wc_root` 纯 ASCII（尾随 PATH 走系统代码页，见
/// [`plan_diffs`]）、不含 `@`（尾随 PATH 的 `@` 是字面字符，`--new=` 那侧怎么解析没实测过，
/// 不冒险）。嵌套 external 的 URL 在别处，照旧单独比。可合并的不到两个时原样返回。
///
/// # 为什么 `--old` 用单元的实际 URL，不用 svnUrl（独立 review 第 1 条）
///
/// 前面几道核对（`url_mismatch`、`whitelist_not_under_base`）比 URL 都不分大小写。svnUrl 与
/// 实际 URL 只差大小写时，大小写敏感的服务器上 `<svnUrl>@F` 根本不存在；按 M6，旧 revision 上
/// 不存在的目标报 `added` 而不是失败，于是整片白名单被报成差异、判成 Lying，而命令成功了，
/// 不会退回。改动前逐单元的命令用的是 `u.url`，不受影响，合并命令也必须如此。`wc_root` 与 URL
/// 末段只差大小写时（白名单写成 `code/`、服务器上是 `Code`）同理不合并：尾随 PATH 会被同时
/// 接到 `--old` 与 `--new` 两侧。
///
/// 合并命令带着它替代的逐单元命令（[`DiffCommand::fallback`]），失败时改跑它们。
pub fn plan_diffs_for(
    units: &[WcIdentity],
    map: &BaseMap,
    skip: &BTreeSet<String>,
    excluded: &BTreeSet<String>,
    svn_url: &str,
) -> Result<DiffPlan> {
    let mut plan = plan_diffs(units, map, skip, excluded)?;
    let base = svn_url.trim_end_matches('/');

    // `plan_diffs` 的前 `units.len()` 条就是逐单元的 floor 命令，与 `units` 同序。这层约定
    // 只在本文件里成立，这里逐条核对一次（独立 review 第 3 条）：对不上就不合并，宁可慢
    let aligned = plan.commands.len() >= units.len()
        && units.iter().zip(&plan.commands).all(|(u, c)| {
            c.wc_root == u.wc_root
                && c.scope == DiffScope::Floor
                && c.fallback.is_empty()
                && !c.args.iter().any(|a| a == "--")
        });
    if !aligned {
        return Ok(plan);
    }

    // 每个单元的实际 URL 去掉末尾的 `/<wc_root>`（逐字节匹配）就是它的前缀；不满足条件的为 None
    let prefix_of = |u: &WcIdentity| -> Option<String> {
        let root = u.wc_root.trim_end_matches('/');
        if root.is_empty() || !root.is_ascii() || root.contains('@') {
            return None;
        }
        let url = u.url.trim_end_matches('/');
        let prefix = url.strip_suffix(root)?.strip_suffix('/')?;
        prefix
            .eq_ignore_ascii_case(base)
            .then(|| prefix.to_string())
    };
    let prefixes: Vec<Option<String>> = units.iter().map(prefix_of).collect();
    // 只合并与第一个可合并单元同一前缀的：前缀不同（写法不一致）的照旧单独比
    let Some(real_base) = prefixes.iter().flatten().next().cloned() else {
        return Ok(plan);
    };
    let mergeable: Vec<bool> = prefixes
        .iter()
        .map(|p| p.as_deref() == Some(real_base.as_str()))
        .collect();
    if mergeable.iter().filter(|&&m| m).count() < 2 {
        return Ok(plan);
    }

    let rest = plan.commands.split_off(units.len());
    let floor_cmds = std::mem::take(&mut plan.commands);
    let mut roots: Vec<String> = Vec::new();
    let mut replaced: Vec<DiffCommand> = Vec::new();
    let mut kept: Vec<DiffCommand> = Vec::new();
    for ((u, cmd), m) in units.iter().zip(floor_cmds).zip(&mergeable) {
        if *m {
            roots.push(u.wc_root.trim_end_matches('/').to_string());
            replaced.push(cmd);
        } else {
            kept.push(cmd);
        }
    }

    let old = format!("--old={real_base}@{}", map.floor);
    let mut head: Vec<&str> = vec!["diff", &old, "--new=.", "--summarize", "--xml"];
    if !map.is_single() {
        head.push("--ignore-properties");
    }
    head.push("--");
    let mut commands = Vec::new();
    let mut replaced = replaced.into_iter();
    for batch in chunk_targets(&head, &roots, MAX_DIFF_CMDLINE) {
        let mut args: Vec<String> = head.iter().map(|a| a.to_string()).collect();
        args.extend(batch.iter().cloned());
        commands.push(DiffCommand {
            wc_root: String::new(),
            unit_url: real_base.clone(),
            rev: map.floor,
            args,
            scope: DiffScope::Floor,
            fallback: replaced.by_ref().take(batch.len()).collect(),
        });
    }
    commands.extend(kept);
    commands.extend(rest);
    plan.commands = commands;
    Ok(plan)
}

/// svn 的失败是不是「整台服务器都够不着」：连不上、认证失败、仓库打不开、网络超时。
///
/// 合并命令失败时，这类失败退回逐单元也只会在第一条上再失败一次（合并的单元都在同一个 URL
/// 前缀下），白等一轮超时（独立 review 第 2 条）。别的失败（某个单元的路径不对之类）才退回。
/// 只认错误码：svn 的提示语会本地化。认错了的代价是退回时多跑一条，不会改变结论。
pub fn server_unreachable(stderr: &str) -> bool {
    const CODES: &[&str] = &[
        "E170013", // Unable to connect to a repository at URL
        "E170001", // Authorization failed
        "E215004", // No more credentials or we tried too many times
        "E175002", // 连接 / DAV 请求失败
        "E175012", // Connection timed out
        "E180001", // Unable to open an ra_local session（file:// 仓库不在了）
        "E210002", // svnserve 网络连接被关闭
        "E670002", // 域名解析失败
        "E670008", // 域名解析失败
        "E730054", // WSAECONNRESET
        "E730060", // WSAETIMEDOUT
        "E730061", // WSAECONNREFUSED
        "E731001", // No such host is known
    ];
    crate::engine::svn_tombstone::error_codes(stderr)
        .iter()
        .any(|c| CODES.contains(&c.as_str()))
}

/// 尾随 PATH 那类命令的固定前缀，最后一个参数是 `--`。
fn group_head(u: &WcIdentity, rev: u64) -> Vec<String> {
    vec![
        "diff".to_string(),
        format!("--old={}@{rev}", u.url.trim_end_matches('/')),
        format!("--new={}", u.wc_root),
        "--summarize".to_string(),
        "--xml".to_string(),
        "--ignore-properties".to_string(),
        "--".to_string(),
    ]
}

/// 把 `files` 接在 `head` 后面，按 [`MAX_DIFF_CMDLINE`] 切成若干条命令。
fn push_batches(
    out: &mut Vec<DiffCommand>,
    u: &WcIdentity,
    rev: u64,
    head: Vec<String>,
    files: Vec<(String, String)>,
    floor: bool,
) {
    let base_len = cmdline_len(&head);
    let mut args = head.clone();
    let mut set: BTreeSet<String> = BTreeSet::new();
    let mut len = base_len;
    let mut flush = |args: &mut Vec<String>, set: &mut BTreeSet<String>| {
        out.push(DiffCommand {
            wc_root: u.wc_root.clone(),
            unit_url: u.url.trim_end_matches('/').to_string(),
            rev,
            args: std::mem::replace(args, head.clone()),
            scope: if floor {
                DiffScope::Floor
            } else {
                DiffScope::Only(std::mem::take(set))
            },
            fallback: Vec::new(),
        });
        set.clear();
    };
    for (p, rel) in files {
        let add = rel.len() + 3;
        if !set.is_empty() && len + add > MAX_DIFF_CMDLINE {
            flush(&mut args, &mut set);
            len = base_len;
        }
        args.push(rel);
        set.insert(p);
        len += add;
    }
    if !set.is_empty() {
        flush(&mut args, &mut set);
    }
}

/// 命令行长度的保守估计，口径见 [`cmdline_estimate`]。
fn cmdline_len(args: &[String]) -> usize {
    cmdline_estimate(&[], args)
}

/// 仓库相对路径归哪个单元：取最长的 `wc_root` 前缀（嵌套 external 比外层更具体）。
/// 返回单元下标与相对单元根的路径（就是单元根本身时为空串）。
fn unit_of(units: &[WcIdentity], path: &str) -> Option<(usize, String)> {
    let mut best: Option<(usize, usize, String)> = None;
    for (i, u) in units.iter().enumerate() {
        let root = u.wc_root.trim_end_matches('/');
        let rel = if path.eq_ignore_ascii_case(root) {
            Some(String::new())
        } else {
            strip_prefix_ci(path, &format!("{root}/"))
        };
        if let Some(rel) = rel {
            let longer = match &best {
                None => true,
                Some(b) => root.len() > b.1,
            };
            if longer {
                best = Some((i, root.len(), rel));
            }
        }
    }
    best.map(|(i, _, rel)| (i, rel))
}

fn outside_units(p: &str) -> GitError {
    GitError::new(
        "svn_health_path_outside_units",
        format!(
            "基线记账里的 `{p}` 不在任何一个白名单工作副本底下，没法决定拿哪个 URL 去比。\
             白名单改过之后要先重建基线。"
        ),
    )
}

/// 墓碑 / 待确认路径在服务器上的 `<url>@<rev>`。
fn server_url_at(units: &[WcIdentity], p: &str, rev: u64) -> Option<String> {
    let (i, rel) = unit_of(units, p)?;
    let url = units[i].url.trim_end_matches('/');
    Some(if rel.is_empty() {
        format!("{url}@{rev}")
    } else {
        format!("{url}/{}@{rev}", encode_url_path(&rel))
    })
}

/// 每个单元 `svn info -R --xml` 一次，找出被 `--set-depth=exclude` 掉的节点
/// （仓库相对路径 → kind）。纯本地读，不写 wc.db（E19）。
fn excluded_nodes(repo_root: &str, units: &[WcIdentity]) -> Result<BTreeMap<String, String>> {
    fault::hit(Point::ExcludedScan)?;
    let mut out = BTreeMap::new();
    // 全部单元一次传（计划 2026-11-09 Task 3）：输出是各单元递归结果的拼接（实测 M4）。
    // 公司机器上逐单元跑一次跟进要 22.9 s
    for xml in run_svn_targets(repo_root, EXCLUDED_SCAN, &excluded_scan_targets(units))? {
        for (p, kind) in parse_excluded_nodes(&xml)? {
            out.insert(p, kind);
        }
    }
    Ok(out)
}

/// [`excluded_nodes`] 发的命令（target 之前的部分）。
const EXCLUDED_SCAN: &[&str] = &["info", "-R", "--xml", "--"];

/// [`excluded_nodes`] 的 target。info 的 target 必须过 peg_safe（与 diff 的尾随 PATH 相反，
/// 见 `svn_wc::peg_safe`）。
fn excluded_scan_targets(units: &[WcIdentity]) -> Vec<String> {
    units
        .iter()
        .map(|u| crate::engine::svn_wc::peg_safe(&u.wc_root))
        .collect()
}

/// 把 [`excluded_nodes`] 那条 `svn info -R` 提前放到后台线程里跑（`svn_cli::prefetch_scan`），
/// 让它与调用方接下来的工作并行。体检开始之前调用方 `adopt` 一下，[`excluded_nodes`] 发同一条
/// 命令时就直接拿到输出。
///
/// 这条命令要把白名单里每个节点的信息都打出来，是纯本地读，与录磁盘内容互不依赖；跟进里原来
/// 排在录完之后串行等它（fixture 上约 0.7 秒，公司机器上按节点数放大）。
///
/// 只在扫描缓存范围内起作用（跟进的准备阶段）；不在范围内返回 `None`，体检照常自己跑。
/// 这里读不出工作副本时也返回 `None`：错误留给体检在原来的位置报。
pub(crate) fn prefetch_excluded_nodes(repo_root: &str, whitelist: &[String]) -> Option<ScanAhead> {
    // 先看在不在范围里：不在的话下面的枚举没有缓存可用，会白跑一次 `svn info` 和一遍全量扫描
    // （初始化、重建、更新的录制都不在范围里）
    if !in_scan_cache() {
        return None;
    }
    let units = crate::engine::svn_wc::enumerate_wcs(repo_root, whitelist).ok()?;
    prefetch_scan(repo_root, EXCLUDED_SCAN, &excluded_scan_targets(&units))
}

/// 点不了名的被排除节点 `p` 在 floor 上有哪些文件（仓库相对路径），D8。
///
/// 走百分号编码成 ASCII 的 URL，不受系统代码页影响，这正是它点不了名的原因（`plan_diffs`）。
/// `kind` 是 `svn info` 给的节点类型：文件的清单就是它自己，名字不用拼。
///
/// floor 上还没有这个节点（之后才加进来又被排除）时 svn rc=1、W160013：返回空，与点名
/// diff 在同一形态下的结果（rc=0、无输出）一致——floor 上没有，基线就不欠它什么。
/// 别的失败一律报错，不能当成空：`parse_list` 的文档写明了失败时 stdout 也是一份合法的空清单。
fn excluded_files_at_floor(
    repo_root: &str,
    units: &[WcIdentity],
    p: &str,
    kind: Option<&str>,
    floor: u64,
) -> Result<Vec<String>> {
    fault::hit(Point::ListAtFloor)?;
    let url = server_url_at(units, p, floor).ok_or_else(|| outside_units(p))?;
    let out = run_svn_code(repo_root, &["ls", "-R", "--xml", "--", &url])?;
    if out.code != 0 {
        let codes = error_codes(&out.stderr);
        let absent = codes.iter().any(|c| c == "W160013")
            && codes.iter().all(|c| c == "W160013" || c == "E200009");
        if absent {
            return Ok(Vec::new());
        }
        return Err(GitError::new(
            "svn_health_list_failed",
            format!("svn ls 没跑成。svn 原话：{}", out.stderr.trim()),
        ));
    }
    let is_file = kind == Some("file");
    let root = p.trim_end_matches('/');
    Ok(parse_list(&out.stdout)?
        .entries
        .into_iter()
        .filter(|e| e.kind == "file")
        .map(|e| {
            if is_file {
                root.to_string()
            } else {
                format!("{root}/{}", e.name)
            }
        })
        .collect())
}

/// 把 `svn diff --xml` 输出里的路径还原成仓库相对路径。`Err` 是说明为什么还原不了。
fn repo_rel_of(path: &str, cmd: &DiffCommand, prefix: &str) -> std::result::Result<String, String> {
    fault::hit(Point::DiffPath).map_err(|e| e.message)?;
    // 单文件白名单条目：`--new` 就是那个文件本身，svn 打出来的路径恰好
    // 等于 `unit_url`，没有尾随的子路径可剥。合并命令里单文件条目的路径是
    // `<svnUrl>/<文件>`，走下面的通则；它的 `unit_url` 是 svnUrl（一个目录），不会命中这里
    if !cmd.wc_root.is_empty()
        && path
            .trim_end_matches('/')
            .eq_ignore_ascii_case(&cmd.unit_url)
    {
        return Ok(cmd.wc_root.clone());
    }
    let Some(sub) = strip_prefix_ci(path, prefix) else {
        return Err(format!(
            "svn diff 输出里的路径 `{path}` 不在本次比对的基准 URL `{}` 之下，\
             没法还原成仓库相对路径。这是一个本应用没有预料到的输出形状，\
             为安全起见判为「未经体检」——既不宣称基线健康，也不指控它撒谎。",
            cmd.unit_url
        ));
    };
    // 输出是 URL：空格是 `%20`、中文是 `%XX` 序列、`%` 是 `%25`。
    // 不解码的话，这类路径既对不上 S，报给用户的也是编码后的样子
    match decode_url(&sub) {
        // 合并命令（`wc_root` 为空）的前缀就是 svnUrl，剥完就是仓库相对路径
        Some(d) if cmd.wc_root.is_empty() => Ok(d),
        Some(d) => Ok(format!("{}/{}", cmd.wc_root, d)),
        None => Err(format!(
            "svn diff 输出里的路径 `{path}` 不是合法的百分号编码，没法还原成\
             仓库相对路径。为安全起见判为「未经体检」。"
        )),
    }
}

/// 体检本体。`tombstoning` 为假时不产生墓碑、不问服务器 HEAD、不扫被排除的节点
/// （单值包装 [`check_baseline`] 走这条，行为与改造前相同）。
fn check_grouped(
    repo_root: &str,
    svn_url: &str,
    map: &BaseMap,
    scope: &WScope,
    explained: &BTreeSet<String>,
    origin: ExplainedOrigin,
    opts: Grouped,
) -> Result<HealthMap> {
    let tombstoning = opts.tombstoning;
    let done = |health: Health| {
        Ok(HealthMap::Verified {
            health,
            new_gone: BTreeMap::new(),
        })
    };
    let not_checked = |reason: String| done(Health::NotChecked { reason });
    // 比对已经开始之后的失败。第一条 diff 有了结果，「没验成」就不再等于「什么都不知道」：
    // 录制时必须拒绝，降成 NotChecked 放行会静默丢掉已知的删除（理由见 HealthMap::Unconfirmed）
    let incomplete = |known: Vec<String>, reason: String| {
        if tombstoning {
            Ok(HealthMap::Unconfirmed {
                files: known,
                reason,
            })
        } else {
            not_checked(reason)
        }
    };

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
        return not_checked(reason);
    }
    if let Some(reason) = whitelist_not_under_base(repo_root, base, roots)? {
        return not_checked(reason);
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

    // 尾随 PATH 里不能有未版本化的（整条 E155010）；丢失的 svn 不报，同样剔除。
    // 单值映射没有尾随 PATH，不必多跑这次 status。
    let mut skip: BTreeSet<String> = BTreeSet::new();
    if !map.revs.is_empty() {
        let read;
        let nodes: &[NodeStatus] = match opts.nodes {
            Some(n) => n,
            None => match read_node_status(repo_root, scope.whitelist(), &units) {
                Ok(n) => {
                    read = n;
                    &read
                }
                Err(e) => return not_checked(format!("读不到工作副本的节点状态：{}", e.message)),
            },
        };
        let by_path: BTreeMap<&str, &NodeStatus> =
            nodes.iter().map(|n| (n.path.as_str(), n)).collect();
        for p in map.revs.keys() {
            let gone = match by_path.get(p.as_str()) {
                None => true,
                Some(n) => matches!(
                    n.item.as_str(),
                    "unversioned" | "missing" | "ignored" | "external"
                ),
            };
            if gone {
                skip.insert(p.clone());
            }
        }
    }

    let excluded = if tombstoning {
        match excluded_nodes(repo_root, &units) {
            Ok(x) => x,
            // 不是 NotChecked：不知道有没有漏收，就不能发布（见 HealthMap::Unconfirmed）
            Err(e) => {
                return Ok(HealthMap::Unconfirmed {
                    files: Vec::new(),
                    reason: format!("找不出被排除（depth=exclude）的节点：{}", e.message),
                })
            }
        }
    } else {
        BTreeMap::new()
    };
    let excluded_paths: BTreeSet<String> = excluded.keys().cloned().collect();
    let DiffPlan {
        commands,
        unaddressable,
    } = match plan_diffs_for(&units, map, &skip, &excluded_paths, base) {
        Ok(p) => p,
        Err(e) => return not_checked(e.message),
    };

    // (仓库相对路径, item, 是否 floor 组)
    let mut diffs: Vec<(String, String, bool)> = Vec::new();
    // 到此刻为止已知的 floor 上有、工作副本里没有的文件
    let deleted_so_far = |diffs: &[(String, String, bool)]| -> Vec<String> {
        let known: BTreeSet<String> = diffs
            .iter()
            .filter(|(p, item, floor)| {
                *floor && item == "deleted" && !explained.contains(p) && !map.gone.contains_key(p)
            })
            .map(|(p, _, _)| p.clone())
            .collect();
        known.into_iter().collect()
    };
    // 合并命令失败时把它替代的逐单元命令插回队头（`DiffCommand::fallback`）
    let mut queue: std::collections::VecDeque<DiffCommand> = commands.into();
    // 已经跑成的命令数。改动前按下标判「第一条」；失败即返回，所以下标为 0 ⇔ 一条都还没跑成，
    // 合并命令失败退回之后按这个数判，逐单元那几条的失败语义与改动前相同
    let mut ran = 0usize;
    while let Some(cmd) = queue.pop_front() {
        // 命令失败一律落到 NotChecked，**不能**当成「没有差异」。
        //
        // 不能用「解析得出来就算成功」：2026-09-14 实测，URL 不可达时 svn 仍然
        // 往 stdout 打出 `<?xml …?>` 和 `<diff><paths>`，错误只在 stderr 和退出码里。
        // 照那种写法，断网时会得到一个空的 paths 列表，于是**报告基线健康**。
        //
        // 一条失败就整体 NotChecked，**不能**「其余都干净所以 Verified」：
        // `Verified` 是一句主动的保证，不是「没发现问题」。
        //
        // 例外是录制时的第二条起（见 `incomplete`）：第一条就失败时（通常是断网）什么都不知道，
        // 离线录制照旧放行；后面失败时前面的结果已经摆在这里了。
        let point = if ran == 0 {
            Point::FirstDiff
        } else {
            Point::LaterDiff
        };
        let merged = !cmd.fallback.is_empty();
        let result = {
            let args: Vec<&str> = cmd.args.iter().map(String::as_str).collect();
            fault::hit(point)
                .and_then(|()| {
                    if merged {
                        fault::hit(Point::MergedDiff)
                    } else {
                        Ok(())
                    }
                })
                .and_then(|()| run_svn(repo_root, &args))
        };
        let xml = match result {
            Ok(x) => x,
            // 不拿合并命令的失败下结论：一个单元的问题（比如磁盘上没了，实测 M7）会让整条
            // 失败，按逐单元跑才知道是哪一个、前面那几个比出了什么。服务器整个够不着时例外：
            // 合并的单元都在同一个 URL 前缀下，退回只会在第一条上再等一轮超时
            Err(e) if merged && !server_unreachable(&e.message) => {
                for f in cmd.fallback.into_iter().rev() {
                    queue.push_front(f);
                }
                continue;
            }
            Err(e) => {
                let what = if cmd.wc_root.is_empty() {
                    let roots: Vec<&str> =
                        cmd.fallback.iter().map(|f| f.wc_root.as_str()).collect();
                    roots.join("、")
                } else {
                    cmd.wc_root.clone()
                };
                let reason = format!("比对 `{what}`（r{}）时失败：{}", cmd.rev, e.message);
                return if ran == 0 {
                    not_checked(reason)
                } else {
                    incomplete(deleted_so_far(&diffs), reason)
                };
            }
        };
        ran += 1;

        let prefix = format!("{}/", cmd.unit_url);
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
            let repo_rel = match repo_rel_of(&entry.path, &cmd, &prefix) {
                Ok(p) => p,
                Err(reason) => return incomplete(deleted_so_far(&diffs), reason),
            };
            let mine = match &cmd.scope {
                DiffScope::Floor => !map.revs.contains_key(&repo_rel),
                DiffScope::Only(set) => set.contains(&repo_rel),
            };
            if mine {
                diffs.push((repo_rel, entry.item, matches!(cmd.scope, DiffScope::Floor)));
            }
        }
    }
    //   剔除不是 W-file 的（扩展名不在白名单里、或被尊重的 .gitignore 排除）。
    //   整批问一次：「尊重」时每次判定都是一次 git 子进程。
    let w = scope.filter(diffs.iter().map(|d| d.0.as_str()))?;

    // §6.8.7：判据是 `D \ S == ∅`，不是 `D == ∅`。
    //
    // `D` 是刚刚收集的这批「磁盘与服务器有内容差异的 W-file」；`S`（`explained`）
    // 是本次录制实际动过的路径——它们的差异**已经被换成服务器内容**，不构成
    // 基线撒谎。剩下解释不了的每一条，才是真的证伪。
    //
    // 工作副本干净时 `S = ∅`，这一步退化成原来的 `D == ∅`，与 §6.8.2 逐字等价。
    let mut lying: BTreeSet<String> = BTreeSet::new();
    let mut candidates: BTreeSet<String> = BTreeSet::new();
    for (p, item, floor) in &diffs {
        if !w.contains(p) || explained.contains(p) {
            continue;
        }
        if item == "deleted" && *floor {
            // floor 上有、工作副本里没有：墓碑说过它已删除，由下面的墓碑核对负责
            if map.gone.contains_key(p) {
                continue;
            }
            if tombstoning {
                candidates.insert(p.clone());
                continue;
            }
        }
        lying.insert(p.clone());
    }
    if !lying.is_empty() {
        return done(Health::Lying {
            files: lying.into_iter().collect(),
        });
    }

    // 已有墓碑：`<url>/<rel>@<d>` 必须不存在（分析 §4.1）
    if !map.gone.is_empty() {
        let mut urls = Vec::with_capacity(map.gone.len());
        for (p, &d) in &map.gone {
            match server_url_at(&units, p, d) {
                Some(u) => urls.push((p.clone(), u)),
                None => {
                    return incomplete(
                        candidates.iter().cloned().collect(),
                        outside_units(p).message,
                    )
                }
            }
        }
        let found = match confirm_absent(repo_root, &urls) {
            Ok(f) => f,
            Err(e) => {
                return incomplete(
                    candidates.iter().cloned().collect(),
                    format!("核对删除记录失败：{}", e.message),
                )
            }
        };
        let reappeared: Vec<String> = found
            .into_iter()
            .filter(|&(_, exists)| exists)
            .map(|(p, _)| p)
            .collect();
        if !reappeared.is_empty() {
            return done(Health::Lying { files: reappeared });
        }
    }

    let mut new_gone: BTreeMap<String, u64> = BTreeMap::new();
    if tombstoning {
        // 走到这里 candidates 是**已知**的 floor 上有、工作副本里没有的文件。从这往下任何一步
        // 没做成都是 Unconfirmed，不是 NotChecked（理由见 HealthMap::Unconfirmed）
        let unconfirmed = |files: &BTreeSet<String>, reason: String| {
            Ok(HealthMap::Unconfirmed {
                files: files.iter().cloned().collect(),
                reason,
            })
        };

        // D8：点不了名的被排除节点，列出它在 floor 上的内容，按 W-file 口径筛。筛出来的与
        // 点得了名的走同一条确认；一个 W-file 都没有的不阻断
        for p in &unaddressable {
            let kind = excluded.get(p).map(String::as_str);
            let listed = match excluded_files_at_floor(repo_root, &units, p, kind, map.floor) {
                Ok(l) => l,
                Err(e) => {
                    let mut files = candidates.clone();
                    files.insert(p.clone());
                    return unconfirmed(
                        &files,
                        format!("列不出被排除的 `{p}` 里有什么：{}", e.message),
                    );
                }
            };
            let w_files = scope.filter(listed.iter().map(String::as_str))?;
            candidates.extend(
                w_files
                    .into_iter()
                    .filter(|f| !explained.contains(f) && !map.gone.contains_key(f)),
            );
        }

        let mut refused: BTreeSet<String> = BTreeSet::new();
        if !candidates.is_empty() {
            // P4 保证所有单元同属一个仓库，HEAD 取哪个单元的 Repository Root 都一样
            let head = match server_head_rev(repo_root, &units[0].repos_root) {
                Ok(h) => h,
                Err(e) => return unconfirmed(&candidates, e.message),
            };
            let mut urls = Vec::with_capacity(candidates.len());
            for p in &candidates {
                match server_url_at(&units, p, head) {
                    Some(u) => urls.push((p.clone(), u)),
                    None => return unconfirmed(&candidates, outside_units(p).message),
                }
            }
            let found = match confirm_absent(repo_root, &urls) {
                Ok(f) => f,
                Err(e) => {
                    return unconfirmed(&candidates, format!("向服务器确认删除失败：{}", e.message))
                }
            };
            for (p, exists) in found {
                if exists {
                    refused.insert(p);
                } else {
                    new_gone.insert(p, head);
                }
            }
        }
        if !refused.is_empty() {
            return Ok(HealthMap::Excluded {
                files: refused.into_iter().collect(),
            });
        }
    }

    // `S` 里的路径被上面那行**整批豁免**了，所以它们一条都没被验证。
    // 报不报出去看它们是谁写的——见 [`ExplainedOrigin`]。发命令前剔掉的 PATH 同样没比过。
    let mut unverified: BTreeSet<String> = match origin {
        ExplainedOrigin::JustRecorded => BTreeSet::new(),
        ExplainedOrigin::Unknown => explained.clone(),
    };
    unverified.extend(skip.into_iter().filter(|p| !explained.contains(p)));
    Ok(HealthMap::Verified {
        health: Health::Verified {
            rev: map.floor,
            unverified: unverified.into_iter().collect(),
        },
        new_gone,
    })
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
    // 全部白名单根一次传（计划 2026-11-09 Task 3）。每次体检都要过这一步，公司机器上白名单
    // 有 23 个目录，逐个跑约 1.25 s。entry 按传入顺序对回（实测 M1）
    let args: Vec<String> = roots
        .iter()
        .map(|r| crate::engine::svn_wc::peg_safe(r))
        .collect();
    let outputs = match run_svn_targets(repo_root, &["info", "--xml", "--"], &args) {
        Ok(x) => x,
        Err(e) => return Ok(Some(e.message)),
    };
    let mut entries = Vec::new();
    for xml in &outputs {
        entries.extend(crate::engine::svn_xml::parse_info(xml)?);
    }
    if entries.len() != roots.len() {
        return Ok(Some(format!(
            "svn info 对 {} 个白名单根返回了 {} 条结果，没法对回各自的目录",
            roots.len(),
            entries.len()
        )));
    }
    for (root, entry) in roots.iter().zip(entries) {
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
