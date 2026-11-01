//! §6.6 失同步判定（计划 Task 3 起按文件比较映射，分析 §6.2 第六轮）。
//!
//! # 判据：逐个基线文件比较
//!
//! ```text
//! R    = 基线记下的映射（键 = 基线树的文件 ∪ 墓碑；旧格式 = 每个文件 → N）
//! n(f) = status 的 <wc-status revision>    c(f) = <commit revision>
//!
//! 任一目录 wc-locked              → WcLocked（优先级最高，E15）
//! n(f) < R(f)                     → 回退（Diverged / PinnedReset）
//! c(f) > R(f)                     → 内容变了（changed）
//! 其余                            → 相等，即使 n(f) 被局部 update 推高（E12）
//! R 的键在 status 里找不到        → 消失（removed）
//! 不在基线树里、磁盘上是文件的 W-file → 外部新增（added）；墓碑路径重新出现也算
//! ```
//!
//! 只看文件：基线只包含 W-file 的字节，目录节点的 revision 变化不影响基线内容。
//! 这是对旧判据（`nodeRevs` 单值，W-path 粒度）的有意收窄。混合版本本身不再是失同步：
//! 只提交、不更新是用户每天的常态（分析 §1.3）。
//!
//! # `rootRev` **不是**判据
//!
//! spec §6.6.2 有实测：一次**完全成功**的交付之后 `rootRev != baselineRev` 必然
//! 成立（D6 只更新白名单，根节点纹丝不动）。拿它报失同步，等于每次交付成功后都
//! 给用户挂一个红色横幅。
//!
//! 它的正确用法是**与上次观测值比**：变了就说明「有人动过工作副本」，触发一次
//! 完整的 `nodeRevs` 校验；没变就跳过。它回答的是「要不要花钱查」，不是
//! 「是不是坏了」。见 [`read_root_revs`]。
//!
//! # 上次观测值不存在后端
//!
//! 后端保持**无状态**，比对由前端 store 做——它本来就按仓库路径持有每个标签的
//! 状态。放后端的话要引一张按路径索引的进程内表，还要处理多标签打开同一个仓库
//! 时的重入，换来的只是省掉一次 IPC。
//!
//! # 这条检查抓不到 `svn revert`
//!
//! spec §9.3 实测：`svn revert` 只改磁盘内容，**不动任何 revision**，所以
//! `nodeRevs == baselineRev` 照样成立。那类外部操作只能由 git 侧看见（工作区
//! 出现修改或改动凭空消失），由现有的 watcher + `git status` 负责，**不归本模块**。

use crate::engine::svn_basemap::{read_base_map, BaseMap};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::read_config;
use crate::engine::svn_nodes::{read_node_status, NodeStatus};
use crate::engine::svn_scope::WScope;
use crate::engine::svn_sync_check::current_branch;
use crate::engine::svn_wc::{enumerate_wcs, peg_safe, WcIdentity};
use crate::error::{GitError, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 基线分支名。与 `svn_config::BASELINE_BRANCH` 恒为 `svn/base`（spec §6.2.1）。
const BASELINE_BRANCH: &str = "svn/base";

/// 失同步判定的结论。
///
/// # ⚠️ `rename_all` 与 `rename_all_fields` 是两件事
///
/// 枚举上的 `rename_all` 只重命名**变体名**（`Diverged` → `diverged`），
/// 变体**内部的字段**一律原样发出。2026-09-16 独立 review 发现：少了
/// `rename_all_fields` 时这里发的是 `node_rev` / `baseline_rev`，而前端
/// `svnSync.ts` 读 `nodeRev` / `baselineRev`，两个都拿到 `undefined`：
/// `forward = undefined > undefined` 恒为 false，于是**工作副本前进被说成
/// 「被回退到基线之前」**，方向正好反了，文案里还插值出 `r undefined`。
///
/// 这层没有编译期检查（`AGENTS.md` §7.3），只能靠 `svn_backend.rs` 里那条
/// 逐个变体的序列化断言兜住——只测无字段的 `InSync` 是测不到的。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SyncVerdict {
    /// 每个基线文件都「相等」（可能是混合版本，见模块头）。
    InSync,
    /// 有目录带 `wc-locked`：svn 操作正在进行或中途被杀（E15，分不出活锁与陈旧锁）。
    ///
    /// 优先级最高：锁着的工作副本可能停在半更新的状态，其余判定都可能看到中间态。
    WcLocked { dirs: Vec<String> },
    /// SVN 侧有新内容：外部提交、局部 update 拉下的同事改动、外部新增、提交删除。
    ///
    /// 区分不了「自己提交的」与「同事的」，所以带上作者，由界面说明（分析 §6.10）。
    ExternallyAdvanced {
        changed: Vec<ExternalChange>,
        added: Vec<ExternalChange>,
        removed: Vec<String>,
    },
    /// 有文件的节点 revision 比基线记的还旧（被回退），且不是 pinned 造成的。
    /// 取路径序第一个回退的文件的 n(f) 与 R(f)。
    Diverged { node_rev: u64, baseline_rev: u64 },
    /// 回退，且 `node_rev` **恰好等于某条 external 的 peg revision**。
    ///
    /// spec §6.6.2 点名的那个「具体且常见」的成因：有人在项目根跑了递归
    /// `svn update`，把白名单所在的 external 拽回了 peg。
    ///
    /// **必须与 [`Self::Diverged`] 分开**：两者的出路完全相反。这里只需要一条
    /// `svn update <白名单所属目录>` 就能拉回 HEAD；而按 `Diverged` 去引导用户
    /// 「重建基线」，会把**被钉死的旧内容**录成一个标着新 revision 的基线——
    /// 自己造出 §9.2 的基线撒谎。
    PinnedReset {
        node_rev: u64,
        baseline_rev: u64,
        /// 那条 external 的本地路径，用来在 UI 上点名。
        external: String,
    },
}

/// SVN 侧有新内容的一个文件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalChange {
    /// 仓库相对、正斜杠。
    pub path: String,
    /// 最后修改 revision c(f)；status 没给（added / replaced）时取节点 revision n(f)。
    pub rev: u64,
    /// `<commit><author>`；匿名提交为 `None`。
    pub author: Option<String>,
}

/// 基线映射的摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseMapSummary {
    pub floor: u64,
    /// 单列在更新 revision 的文件数。
    pub newer: usize,
    /// 墓碑数。
    pub gone: usize,
}

impl BaseMapSummary {
    pub(crate) fn of(map: &BaseMap) -> Self {
        Self {
            floor: map.floor,
            newer: map.revs.len(),
            gone: map.gone.len(),
        }
    }
}

/// 一次完整校验的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// 基线映射的 floor。旧格式基线就是 `git-svn-id` 里的那个 N。
    pub baseline_rev: u64,
    /// 基线文件与外部新增文件的节点 revision **并集**，升序。只给人看，不参与判定。
    pub node_revs: Vec<u64>,
    pub verdict: SyncVerdict,
    pub base: BaseMapSummary,
    /// 当前分支不包含 `svn/base`，且分叉点是一条基线 commit（「分支跟进」入口，分析 §6.11.3）。
    pub head_behind_base: bool,
}

/// 一个工作副本根的 revision，`rootRev` 提示档用。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RootRev {
    pub wc_root: String,
    pub rev: u64,
}

/// 完整校验（spec §6.6.3 的**第二档**）。
///
/// 时机：打开仓库、`sync_in` / `sync_out` 第一步、恢复向导前——这几处是正确性的
/// 闸门，必须问到底。高频的 `repo-changed` 刷新走 [`read_root_revs`] 那一档。
pub fn check_sync(repo_root: &str) -> Result<SyncStatus> {
    Ok(check_sync_with_nodes(repo_root)?.0)
}

/// 同 [`check_sync`]，另外交出这次读到的全部节点（含目录）。
///
/// `sync_in` 的 A16 接管要知道「所有节点是否在同一个 revision」，与判定共用同一次 status。
pub(crate) fn check_sync_with_nodes(repo_root: &str) -> Result<(SyncStatus, Vec<NodeStatus>)> {
    let oid = baseline_oid(repo_root)?;
    // 映射与文件清单取自同一个 OID：分两次按 ref 名读，中间基线被移动就会拼出一个不存在的 R
    let map = read_base_map(repo_root, &oid)?;
    let files = tree_files(repo_root, &oid)?;
    let cfg = read_config(repo_root)?;
    let wcs = enumerate_wcs(repo_root, &cfg.whitelist)?;
    let nodes = read_node_status(repo_root, &cfg.whitelist, &wcs)?;
    let scope = WScope::for_repo(repo_root, &cfg)?;

    let j = judge(&map, &files, &nodes, |cands| {
        // 外部新增要看磁盘是文件：status 不给 kind，扩展名为 `*` 时目录也会过 W-path 判断
        let on_disk: Vec<&str> = cands
            .iter()
            .copied()
            .filter(|p| Path::new(repo_root).join(p).is_file())
            .collect();
        scope.filter(on_disk)
    })?;
    let verdict = if !j.locked.is_empty() {
        SyncVerdict::WcLocked { dirs: j.locked }
    } else if let Some((n, r)) = j.backward {
        match pinned_external_at(repo_root, &cfg.whitelist, n)? {
            Some(ext) => SyncVerdict::PinnedReset {
                node_rev: n,
                baseline_rev: r,
                external: ext,
            },
            None => SyncVerdict::Diverged {
                node_rev: n,
                baseline_rev: r,
            },
        }
    } else if j.changed.is_empty() && j.added.is_empty() && j.removed.is_empty() {
        SyncVerdict::InSync
    } else {
        SyncVerdict::ExternallyAdvanced {
            changed: j.changed,
            added: j.added,
            removed: j.removed,
        }
    };
    let status = SyncStatus {
        baseline_rev: map.floor,
        node_revs: j.node_revs.into_iter().collect(),
        verdict,
        base: BaseMapSummary::of(&map),
        head_behind_base: head_behind_base(repo_root, &oid),
    };
    Ok((status, nodes))
}

/// 基线发布了、当前分支没跟上：上次跟进冲突，或者切到了一条旧分支（分析 §6.11.3）。
///
/// 口径与跟进的 `branch_baseline` 相同：分叉点必须是一条基线 commit，否则跟进只会拒绝。detached
/// 与基线分支本身不算，跟进同样不接。只是给入口的提示，读不出来都按不落后，不挡校验。
fn head_behind_base(repo_root: &str, base: &str) -> bool {
    if !matches!(current_branch(repo_root), Ok(Some(b)) if b != BASELINE_BRANCH) {
        return false;
    }
    let Ok(Some(fork)) = crate::engine::git_read::merge_base(repo_root, "HEAD", base) else {
        return false;
    };
    fork != base && read_base_map(repo_root, &fork).is_ok()
}

/// [`judge`] 的中间结果。
struct Judged {
    locked: Vec<String>,
    /// 路径序第一个回退的文件：(n(f), R(f))。
    backward: Option<(u64, u64)>,
    changed: Vec<ExternalChange>,
    added: Vec<ExternalChange>,
    removed: Vec<String>,
    node_revs: BTreeSet<u64>,
}

/// svn 不认识的条目：没有服务器节点，对「这个文件还在不在 SVN 里」无可奉告。
///
/// 基线文件变成 `unversioned` 的现实成因是外部 `svn rm --keep-local` 后提交：服务器上
/// 已经删了，磁盘上还留着，所以按「消失」处理，而不是「找到了、相等」。
fn unknown_to_svn(n: &NodeStatus) -> bool {
    matches!(n.item.as_str(), "unversioned" | "ignored" | "external")
}

/// 逐键判定（分析 §6.2 第六轮）。`w_files_on_disk` 从候选里挑出「磁盘上是文件的 W-file」。
fn judge(
    map: &BaseMap,
    files: &BTreeSet<String>,
    nodes: &[NodeStatus],
    w_files_on_disk: impl FnOnce(&[&str]) -> Result<BTreeSet<String>>,
) -> Result<Judged> {
    let by_path: BTreeMap<&str, &NodeStatus> = nodes
        .iter()
        .filter(|n| !unknown_to_svn(n))
        .map(|n| (n.path.as_str(), n))
        .collect();
    let locked: Vec<String> = nodes
        .iter()
        .filter(|n| n.wc_locked)
        .map(|n| n.path.clone())
        .collect();

    let mut backward = None;
    let mut changed = Vec::new();
    let mut removed = Vec::new();
    let mut node_revs = BTreeSet::new();
    for f in files {
        let r = map.rev_of(f);
        let Some(node) = by_path.get(f.as_str()) else {
            removed.push(f.clone());
            continue;
        };
        if let Some(n) = node.node_rev {
            node_revs.insert(n);
            if n < r {
                backward.get_or_insert((n, r));
                continue;
            }
        }
        // 本地新增 / 替换 / 复制来的节点，<commit> 要么没有（E13），要么是**复制源**的
        // revision 与作者（Review 1 第 4 条）：都不是这个路径的 c(f)，只是本地改动，不算前进
        let local_only = matches!(node.item.as_str(), "added" | "replaced")
            || node.copied
            || node.node_rev.is_none();
        if local_only {
            continue;
        }
        if let Some(c) = node.commit_rev.filter(|&c| c > r) {
            changed.push(ExternalChange {
                path: f.clone(),
                rev: c,
                author: node.author.clone(),
            });
        }
    }

    // 外部新增：有服务器 revision、不是本地新增、不在基线树里。墓碑路径不排除——
    // 它重新出现在 status 里就是服务器上又有了；没出现时什么都不用做（视同相等）
    let cands: Vec<&str> = nodes
        .iter()
        .filter(|n| n.node_rev.is_some() && !unknown_to_svn(n) && n.item != "added")
        .filter(|n| !files.contains(&n.path))
        .map(|n| n.path.as_str())
        .collect();
    let added_paths = if cands.is_empty() {
        BTreeSet::new()
    } else {
        w_files_on_disk(&cands)?
    };
    let mut added = Vec::new();
    for p in &added_paths {
        let Some(node) = by_path.get(p.as_str()) else {
            continue;
        };
        let Some(n) = node.node_rev else { continue };
        node_revs.insert(n);
        added.push(ExternalChange {
            path: p.clone(),
            rev: node.commit_rev.unwrap_or(n),
            author: node.author.clone(),
        });
    }

    Ok(Judged {
        locked,
        backward,
        changed,
        added,
        removed,
        node_revs,
    })
}

/// `svn/base` 顶端的 OID。分支不存在时报 `svn_no_baseline`（理由见 [`read_baseline_rev`]）。
fn baseline_oid(repo_root: &str) -> Result<String> {
    let reference = format!("refs/heads/{BASELINE_BRANCH}");
    let no_baseline = |detail: &str| {
        GitError::new(
            "svn_no_baseline",
            format!(
                "读不到基线分支 `{BASELINE_BRANCH}`，这个仓库还没有基线，\
                 失同步无从判定。原始错误：{detail}"
            ),
        )
    };
    crate::engine::git_read::resolve(repo_root, &reference)
        .map_err(|e| no_baseline(e.message.trim()))?
        .ok_or_else(|| no_baseline("ref 不存在"))
}

/// 基线树里的文件：映射的键集合（分析 §6.2：不从 status 推）。
pub(crate) fn tree_files(repo_root: &str, oid: &str) -> Result<BTreeSet<String>> {
    crate::engine::git_read::tree_paths(repo_root, oid)
}

/// 每个工作副本根的 revision（spec §6.6.3 的**第一档**，O(1)）。
///
/// 走 `svn info --show-item revision`：实测 stdout 就是裸的十进制数字，
/// 不用过 XML 解析。**`svn info` 绝不能带 `--ignore-externals`**（它拒绝这个参数）。
///
/// 返回值给前端与**上次观测值**比对用；任一条变了就该跑一次 [`check_sync`]。
pub fn read_root_revs(repo_root: &str) -> Result<Vec<RootRev>> {
    let cfg = read_config(repo_root)?;
    let wcs = enumerate_wcs(repo_root, &cfg.whitelist)?;
    let mut out = Vec::new();
    for wc in &wcs {
        let target = peg_safe(&wc.wc_root);
        let text = run_svn(
            repo_root,
            &["info", "--show-item", "revision", "--", &target],
        )?;
        let rev = text.trim().parse::<u64>().map_err(|_| {
            GitError::new(
                "svn_sync_bad_root_rev",
                format!(
                    "svn info --show-item revision 对 `{}` 返回了不是数字的内容：`{}`",
                    wc.wc_root,
                    text.trim()
                ),
            )
        })?;
        out.push(RootRev {
            wc_root: wc.wc_root.clone(),
            rev,
        });
    }
    Ok(out)
}

/// 基线分支顶端的完整记账（混合基线也读得出）。分支不存在时报 `svn_no_baseline`。
pub(crate) fn read_baseline_map(repo_root: &str) -> Result<BaseMap> {
    let oid = baseline_oid(repo_root)?;
    read_base_map(repo_root, &oid)
}

/// 基线分支顶端的 floor：旧格式基线就是 `git-svn-id` 里的 N（计划 Task 5）。
///
/// 只要一个数字的地方（「更新」的结果、交付后的报告）统一走这里，不再各自解析 trailer：
/// 混合基线没有 `git-svn-id`，旧写法会把它读成「不是基线」。分支不存在时报
/// `svn_no_baseline`，读不出记账时报 `svn_baseline_no_trailer`——不能返回 0，那会让
/// 「N != 0」把每个仓库都判成失同步；也不能返回成功，那等于宣称一个不存在的基线是好的。
pub(crate) fn read_baseline_rev(repo_root: &str) -> Result<u64> {
    Ok(read_baseline_map(repo_root)?.floor)
}

/// `svn status -v --xml --ignore-externals` 要逐个跑的 target：白名单条目 + 每个 wc(P) 的根。
///
/// 失同步判定与映射计算（`svn_nodes::read_node_status`）共用这一份，两边的 target
/// 分了家，判定看到的节点与录制看到的就不是同一批。
///
/// # 为什么必须逐个工作副本查
///
/// 白名单底下可能挂着嵌套 external，它们是**完全独立的工作副本**（自己的 `.svn`、
/// URL、Revision）。只查白名单根那一个 WC 的话，嵌套 external 就成了一块
/// 「git 管着、SVN 侧没人盯」的盲区——它会被录进基线、进交付清单，却从不参与
/// 失同步校验。
///
/// # 两个参数一个都不能省
///
/// - **`-v`**：不带它时 status 只报告有改动的节点，干净工作副本得到空输出，
///   于是 `nodeRevs` 变成空集，「单值不变量」以看起来通过的方式被绕过。
/// - **`--ignore-externals`**：不带它时 external **内部**的节点会混进来，而它们的
///   revision 来自**另一个仓库的序列**（实测 `{29825, 1938156}`），后果是永久
///   误报失同步（spec §6.6.4 陷阱一）。
///
/// 这里的 `--ignore-externals` 是**在每个 WC 内部**用的，挡住更深一层的嵌套
/// external；那一层由 `enumerate_wcs` 递归枚举出来各查一次。从项目根用它去覆盖
/// 白名单才是错的（会把白名单整个跳过，spec §6.6.5）。
///
/// 两条违约 `parse_node_status` 自己都会拒绝，不靠调用方记得。
pub(crate) fn status_targets(whitelist: &[String], wcs: &[WcIdentity]) -> Result<Vec<String>> {
    // ⚠️ `WcIdentity::wc_root` 是**仓库内相对路径**，而且指的是「这条白名单条目
    // 或 external 根」——不是 SVN 意义上的工作副本根（那是 `wc_root_abspath`）。
    // 它本身就是 §6.6.5 要的记账单位，所以 targets 直接取它，不需要再换算。
    //
    // 这里踩过一次：按「wc_root 是绝对路径」去算相对路径，结果恒为 None，
    // **嵌套 external 从没进过 targets**，而白名单根那一次查询照样返回单值，
    // 于是判成 InSync —— 一个看起来完全正常的盲区。
    let mut targets = normalize(whitelist);
    for wc in wcs {
        let rel = wc.wc_root.trim_end_matches('/').to_string();
        if !rel.is_empty() && !targets.iter().any(|t| t.eq_ignore_ascii_case(&rel)) {
            targets.push(rel);
        }
    }
    if targets.is_empty() {
        return Err(GitError::new(
            "svn_sync_empty_whitelist",
            "白名单是空的，失同步没有校验范围。不传 target 会让 svn status 走\
             整个工作副本，既慢又会把白名单外的改动算进来（spec §12.2）。",
        ));
    }
    Ok(targets)
}

/// `n` 是否恰好等于某条被钉死的 external 的 peg revision；是则返回它的本地路径。
fn pinned_external_at(repo_root: &str, whitelist: &[String], n: u64) -> Result<Option<String>> {
    let pinned = crate::engine::svn_externals::detect_pinned(repo_root, whitelist)?;
    Ok(pinned
        .into_iter()
        .find(|p| p.pinned_rev == n)
        .map(|p| p.local_path))
}

/// 白名单条目归一成「仓库内相对路径、正斜杠、无尾斜杠」。
fn normalize(whitelist: &[String]) -> Vec<String> {
    whitelist
        .iter()
        .map(|w| w.replace('\\', "/").trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}
