//! 节点状态与基线映射的计算（分析 §6.1、§6.2；计划 Task 2）。
//!
//! # 映射取值（日志「映射取值」条目）
//!
//! ```text
//! F    = 基线树里各文件节点 revision n(f) 与白名单内目录节点 revision n(D) 的最小值
//! c(f) = 最后修改 revision；缺失（added / replaced 没有 <commit>，E13）时取 n(f)
//! c(f) ≤ F → 隐含 F，不单列
//! 其余     → 记 n(f)
//! ```
//!
//! 正确性：f 在 c(f) 之后没改过，所以对任意 r ∈ [c(f), n(f)]，f@r 都等于当前的 pristine。
//! F 与 n(f) 都落在这个区间里，标签是真的。只按 n(f) 记的话，局部 update 会把整个目录写进
//! 映射；周一根目录更新之后各文件也回不到同一个值（E12）。
//!
//! # 为什么目录也要算进 F（Review 1 第 2 条）
//!
//! 文件的标签只担保文件的内容，**有哪些文件**由目录担保：工作副本里 D 的子项集合是 n(D)
//! 时的。只提交不更新时文件到了 r3、目录还停在 r1，同事 r2 新增的 X 不在工作副本里；
//! F 取 3 的话 `D@3` 里有 X、基线树里没有，单值标签就是假的。把 F 降到目录的 revision
//! 总是安全的：c(f) > F 的文件自动单列，标签仍然成立，只是映射多几条。

use crate::engine::svn_basemap::{BaseMap, MAX_MAP_ENTRIES};
use crate::engine::svn_cli::run_svn_targets;
use crate::engine::svn_sync::status_targets;
use crate::engine::svn_wc::{peg_safe, WcIdentity};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};

pub use crate::engine::svn_xml::{parse_node_status, NodeStatus};

/// 对全部 target 跑 `svn status -v --xml --ignore-externals`，合并成一份按路径升序的节点表。
///
/// target 与失同步判定相同（[`status_targets`]）：白名单条目 + 每个 wc(P) 的根。
/// 每个 wc 都要单独作为一个 target，因为 external 是独立的工作副本，`--ignore-externals`
/// 只挡住更深一层，那一层由 `enumerate_wcs` 枚举出来各查一次（Review Focus 1）。全部 target
/// 在同一条命令里传（计划 2026-11-09 Task 3），每个 target 一个 `<target>` 块。
///
/// # 去重
///
/// 同一路径可能出现两次：外层 wc 的输出里 external 根是一条 `item="external"`、没有
/// revision 的标记，它自己那个 wc 的输出里才是真正的目录节点；白名单条目互相嵌套时
/// 也会重复。保留带节点 revision 的那条，都带时保留先见到的（两次 status 读的是同一份
/// wc.db，内容相同）。
pub fn read_node_status(
    repo: &str,
    whitelist: &[String],
    wcs: &[WcIdentity],
) -> Result<Vec<NodeStatus>> {
    let mut merged: BTreeMap<String, NodeStatus> = BTreeMap::new();
    // target 必须过 peg_safe：svn 把最后一个 `@` 之后的部分当 peg revision。
    // 全部 target 一次传（计划 2026-11-09 Task 3）：输出是每个 target 一个 `<target>` 块，
    // 与逐个跑的拼接相同，`parse_node_status` 的 external 守卫按块判（实测 M3）
    let args: Vec<String> = status_targets(whitelist, wcs)?
        .iter()
        .map(|t| peg_safe(t))
        .collect();
    for xml in run_svn_targets(
        repo,
        &["status", "-v", "--xml", "--ignore-externals", "--"],
        &args,
    )? {
        for n in parse_node_status(&xml)? {
            match merged.get(&n.path) {
                Some(old) if old.node_rev.is_some() || n.node_rev.is_none() => {}
                _ => {
                    merged.insert(n.path.clone(), n);
                }
            }
        }
    }
    Ok(merged.into_values().collect())
}

/// 节点里的目录及其 revision：`nodes` 里别的节点的祖先（含白名单根）。
///
/// `svn status` 不给 kind，只能这样认。不能退成「所有不在基线树里的节点」：白名单内的美术
/// 文件没有子项集合可担保，算进来只会无谓地拖低 floor、把映射撑大。
///
/// 一个子项都没有的空目录认不出来：它要低于父目录，得有人对它单独 `update -r`，形态罕见；
/// 漏掉时后果是体检把同事后来加进去的文件报成漏收（方向保守）。
pub(crate) fn dir_revs(nodes: &[NodeStatus]) -> Vec<(&str, u64)> {
    let mut parents: BTreeSet<&str> = BTreeSet::new();
    for n in nodes {
        let mut p = n.path.as_str();
        while let Some((parent, _)) = p.rsplit_once('/') {
            // 已经见过的祖先，它再往上的也都见过了
            if !parents.insert(parent) {
                break;
            }
            p = parent;
        }
    }
    nodes
        .iter()
        .filter(|n| parents.contains(n.path.as_str()))
        .filter_map(|n| n.node_rev.map(|r| (n.path.as_str(), r)))
        .collect()
}

/// 按「映射取值」规则，由录制好的基线树文件清单与节点状态算出 [`BaseMap`]。
///
/// - `files`：基线树里的文件（仓库相对、正斜杠）。键集合取它而不从 status 推：status 不给
///   kind，扩展名为 `*` 时目录也会被当成 W-path（分析 §6.2）
/// - `gone`：调用方给出的墓碑。已重新进入基线树的、以及 floor 已经越过删除时 HEAD 的
///   墓碑在这里去掉（分析 §6.1「墓碑一直带着，直到 floor ≥ 墓碑的 revision」）
///
/// 文件在 `nodes` 里找不到、或没有节点 revision 时报 `svn_map_unknown_node`：那是录制与
/// status 对不上，猜一个 revision 就是撒谎基线。条目超过 [`MAX_MAP_ENTRIES`] 时报
/// `svn_map_too_large`，列出节点 revision 最低的 10 个文件——是它们把 floor 拖低了。
pub fn compute_map(
    repos_root: &str,
    uuid: &str,
    files: &BTreeSet<String>,
    nodes: &[NodeStatus],
    gone: BTreeMap<String, u64>,
) -> Result<BaseMap> {
    let by_path: BTreeMap<&str, &NodeStatus> = nodes.iter().map(|n| (n.path.as_str(), n)).collect();

    // (路径, n(f), c(f))
    let mut revs_of: Vec<(&str, u64, u64)> = Vec::with_capacity(files.len());
    let mut unknown: Vec<&str> = Vec::new();
    for f in files {
        match by_path
            .get(f.as_str())
            .and_then(|n| n.node_rev.map(|r| (r, n)))
        {
            Some((n, node)) => revs_of.push((f, n, node.commit_rev.unwrap_or(n))),
            None => unknown.push(f),
        }
    }
    if !unknown.is_empty() {
        return Err(GitError::new(
            "svn_map_unknown_node",
            format!(
                "基线树里有 {} 个文件在 svn status 里找不到、或没有服务器 revision：\n{}\n\
                 录下的内容与工作副本对不上，没法给它们标 revision。",
                unknown.len(),
                list(&unknown)
            ),
        ));
    }

    let dirs = dir_revs(nodes);
    let lowest_node = revs_of
        .iter()
        .map(|&(_, n, _)| n)
        .chain(dirs.iter().map(|&(_, n)| n))
        .min();
    let floor = match lowest_node {
        Some(f) => f,
        // 白名单里只有一个空目录：没有文件也没有子项可担保，取节点里最小的
        None => nodes
            .iter()
            .filter_map(|n| n.node_rev)
            .min()
            .ok_or_else(|| {
                GitError::new(
                    "svn_map_unknown_node",
                    "基线树是空的，svn status 里也没有一个带服务器 revision 的节点，\
                     定不出基线的 revision。",
                )
            })?,
    };

    let revs: BTreeMap<String, u64> = revs_of
        .iter()
        // c(f) ≤ F：f 在 F 与 n(f) 之间没改过，f@F 就是 pristine
        .filter(|&&(_, _, c)| c > floor)
        .map(|&(p, n, _)| (p.to_string(), n))
        .collect();
    let gone: BTreeMap<String, u64> = gone
        .into_iter()
        .filter(|(p, d)| !files.contains(p) && *d > floor)
        .collect();

    let map = BaseMap {
        repos_root: repos_root.to_string(),
        uuid: uuid.to_string(),
        floor,
        revs,
        gone,
    };
    if map.entry_count() > MAX_MAP_ENTRIES {
        // 目录一起排：floor 也可能是停在旧 revision 的目录拖低的
        let mut lowest: Vec<(&str, u64)> = revs_of.iter().map(|&(p, n, _)| (p, n)).collect();
        lowest.extend(dirs.iter().copied());
        lowest.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));
        let names: Vec<String> = lowest
            .iter()
            .take(10)
            .map(|&(p, n)| format!("r{n}  {p}"))
            .collect();
        return Err(GitError::new(
            "svn_map_too_large",
            format!(
                "基线要单列的文件有 {} 个，超过上限 {MAX_MAP_ENTRIES}。floor 停在 r{floor}，\
                 是下面这些停在旧 revision 的文件或目录把它拖低的：\n{}\n\
                 对它们所在的目录跑一次 SVN Update，floor 就会前进。",
                map.entry_count(),
                names.join("\n")
            ),
        ));
    }
    Ok(map)
}

/// 路径清单给人看：最多 20 条。
fn list(paths: &[&str]) -> String {
    let mut s: Vec<String> = paths.iter().take(20).map(|p| format!("  - {p}")).collect();
    if paths.len() > 20 {
        s.push(format!("  …还有 {} 条", paths.len() - 20));
    }
    s.join("\n")
}
