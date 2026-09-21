//! §6.6 失同步判定。
//!
//! # 判据只有两条
//!
//! | 观察 | 含义 |
//! |---|---|
//! | `nodeRevs` 有多个值 | mixed revision，外部提交或局部 update 过 |
//! | `nodeRevs = {N}` 但 `N != baselineRev` | 整体被外部推进或回退 |
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

use crate::engine::git_cli::run_git;
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::read_config;
use crate::engine::svn_wc::{enumerate_wcs, peg_safe, WcIdentity};
use crate::engine::svn_xml::parse_node_revs;
use crate::error::{GitError, Result};
use serde::Serialize;
use std::collections::BTreeSet;

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
    /// `nodeRevs` 是单值且等于 `baselineRev`。
    InSync,
    /// `nodeRevs` 多值。**每个值都要列出来**——用户要据此判断是哪一块掉队了。
    MixedRevision { revs: Vec<u64> },
    /// 单值但与基线不等，且不是 pinned 造成的。
    Diverged { node_rev: u64, baseline_rev: u64 },
    /// 单值但与基线不等，且 `node_rev` **恰好等于某条 external 的 peg revision**。
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

/// 一次完整校验的结果。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub baseline_rev: u64,
    /// 所有白名单所属工作副本的 `nodeRevs` **并集**，升序。
    pub node_revs: Vec<u64>,
    pub verdict: SyncVerdict,
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
    let baseline_rev = read_baseline_rev(repo_root)?;
    let cfg = read_config(repo_root)?;
    let wcs = enumerate_wcs(repo_root, &cfg.whitelist)?;
    let revs = node_revs(repo_root, &cfg.whitelist, &wcs)?;

    let node_revs: Vec<u64> = revs.iter().copied().collect();
    let verdict = if node_revs.len() > 1 {
        SyncVerdict::MixedRevision {
            revs: node_revs.clone(),
        }
    } else {
        // 空集不可能走到这里：parse_node_revs 自己会对空输出报错
        // （svn_xml_empty_node_revs），那是 1b 独立 review 的阻断项修出来的。
        let n = node_revs[0];
        if n == baseline_rev {
            SyncVerdict::InSync
        } else if let Some(ext) = pinned_external_at(repo_root, &cfg.whitelist, n)? {
            SyncVerdict::PinnedReset {
                node_rev: n,
                baseline_rev,
                external: ext,
            }
        } else {
            SyncVerdict::Diverged {
                node_rev: n,
                baseline_rev,
            }
        }
    };
    Ok(SyncStatus {
        baseline_rev,
        node_revs,
        verdict,
    })
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

/// 基线分支顶端 commit 的 `git-svn-id` trailer 里记的 revision——**权威记账**。
pub(crate) fn read_baseline_rev(repo_root: &str) -> Result<u64> {
    let msg = run_git(
        repo_root,
        &[
            "log",
            "-1",
            "--format=%B",
            &format!("refs/heads/{BASELINE_BRANCH}"),
        ],
    )
    .map_err(|e| {
        // 分支不存在时 git 报的是它自己的话，对用户没有意义。
        //
        // 这里必须报错而不是返回 0：返回 0 会让「N != 0」把**每一个**仓库都判成
        // 失同步；返回 InSync 则等于宣称一个不存在的基线是好的。两种都是撒谎。
        GitError::new(
            "svn_no_baseline",
            format!(
                "读不到基线分支 `{BASELINE_BRANCH}`，这个仓库还没有基线，\
                 失同步无从判定。原始错误：{}",
                e.message.trim()
            ),
        )
    })?;
    let trailer = crate::engine::svn_baseline::parse_trailer(&msg).ok_or_else(|| {
        // 「解析失败即视为该 commit 不是 baseline」——不猜、不部分接受（spec §6.4.1）。
        GitError::new(
            "svn_baseline_no_trailer",
            "基线分支最新的 commit 上没有可解析的 git-svn-id trailer，\
             它不是本应用录出来的基线。请重建基线。",
        )
    })?;
    Ok(trailer.rev)
}

/// 所有白名单所属工作副本的 `nodeRevs` **并集**（spec §6.6.5 推广后的不变量）。
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
/// 两条违约 `parse_node_revs` 自己都会拒绝，不靠这里记得。
fn node_revs(repo_root: &str, whitelist: &[String], wcs: &[WcIdentity]) -> Result<BTreeSet<u64>> {
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

    let mut out = BTreeSet::new();
    for t in &targets {
        // status 的 target **必须**过 peg_safe：svn 把最后一个 `@` 之后的部分当
        // peg revision，`--` 挡不住。（`svn log` 的相对路径正好相反，见 svn_log。）
        let arg = peg_safe(t);
        let xml = run_svn(
            repo_root,
            &["status", "-v", "--xml", "--ignore-externals", "--", &arg],
        )?;
        for r in parse_node_revs(&xml)? {
            // parse_node_revs 给的是 i64（svn 的 revision 不会是负数，
            // 但解析层如实反映 XML 写了什么）。这里收窄成 u64 并拒绝负数。
            let v = u64::try_from(r).map_err(|_| {
                GitError::new(
                    "svn_sync_negative_rev",
                    format!("svn status 报出了负的 revision `{r}`，这不是合法的 SVN revision"),
                )
            })?;
            out.insert(v);
        }
    }
    Ok(out)
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
