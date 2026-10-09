//! D4：把交付清单登记到 SVN（`svn add --parents` / `svn delete`），以及 spec §8.5 的精确撤销。
//!
//! # 撤销的不变量
//!
//! 工作目录与 `.svn` 的登记都回到 D4 之前的快照（`svnStatusBefore`）。撤完逐条比对
//! `svn status`，**不拿 git 的干净当证据**——git 看不见 `.svn` 里的登记（§8.5.1）。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_journal::SvnNode;
use crate::engine::svn_sync_check::list_paths;
use crate::engine::svn_sync_out_check::DeliverChange;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::parse_status_entries;
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRecord {
    /// 相对 `before` 新增的节点状态（spec §8.5 的 `svnDelta`）。
    pub delta: Vec<SvnNode>,
    /// D5 的 target 必须带上的父目录（spec §8.2.6）。包括用户事先自己 `svn add` 过的
    /// 祖先目录：它们不在差量里，但不进 target 时 D5 会被 `E200009` 拒。
    pub new_parent_dirs: Vec<String>,
}

/// 白名单 target 下所有非干净节点，按路径排序。
///
/// `dir` 在此刻查磁盘：`svn status --xml` 不给 `kind`（计划开工前实测 M2-E1）。
pub fn snapshot(repo: &str, targets: &[String]) -> Result<Vec<SvnNode>> {
    if targets.is_empty() {
        return Err(empty_targets("svn status"));
    }
    let mut out = Vec::new();
    for t in targets {
        let xml = run_svn(
            repo,
            &[
                "status",
                "--xml",
                "--ignore-externals",
                "--non-interactive",
                "--",
                &peg_safe(t),
            ],
        )?;
        for e in parse_status_entries(&xml)? {
            // external 标记带了 --ignore-externals 也照样出现，它不是登记（svn_dirty.rs 记过）
            if e.item == "external" || (e.item == "normal" && e.props == "none") {
                continue;
            }
            let dir = Path::new(repo).join(&e.path).is_dir();
            out.push(SvnNode {
                path: e.path,
                status: e.item,
                dir,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup();
    Ok(out)
}

/// D4。调用方必须**先**把 `before` 写进 journal（spec §7.6 写前）。
pub fn stage(
    repo: &str,
    targets: &[String],
    changes: &[DeliverChange],
    before: &[SvnNode],
) -> Result<StageRecord> {
    if changes.is_empty() {
        return Err(empty_targets("svn add / svn delete")); // §8.3
    }
    let adds: Vec<&str> = changes
        .iter()
        .filter(|c| c.status == "A")
        .map(|c| c.path.as_str())
        .collect();
    let dels: Vec<&str> = changes
        .iter()
        .filter(|c| c.status == "D")
        .map(|c| c.path.as_str())
        .collect();
    if !adds.is_empty() {
        svn_batch(
            repo,
            &["add", "--parents", "--non-interactive", "--"],
            &adds,
        )?;
    }
    if !dels.is_empty() {
        svn_batch(repo, &["delete", "--non-interactive", "--"], &dels)?;
    }
    let after = snapshot(repo, targets)?;
    let delta: Vec<SvnNode> = after
        .iter()
        .filter(|n| !before.contains(n))
        .cloned()
        .collect();

    // A 文件的各级祖先里、此刻是 added 的目录——包括用户自己事先 add 过的
    let added_dirs: BTreeSet<&str> = after
        .iter()
        .filter(|n| n.status == "added" && n.dir)
        .map(|n| n.path.as_str())
        .collect();
    let mut parents = BTreeSet::new();
    for a in &adds {
        let mut p = Path::new(a).parent();
        while let Some(dir) = p {
            let s = dir.to_string_lossy().replace('\\', "/");
            if s.is_empty() {
                break;
            }
            if added_dirs.contains(s.as_str()) {
                parents.insert(s);
            }
            p = dir.parent();
        }
    }
    Ok(StageRecord {
        delta,
        // BTreeSet 有序：父在子之前，与 svn 对 target 顺序的期望一致
        new_parent_dirs: parents.into_iter().collect(),
    })
}

/// spec §8.5：撤到 `before` 为止。只撤**本流程能产生的**两类登记（计划 B6）。
pub fn unstage(repo: &str, targets: &[String], before: &[SvnNode]) -> Result<()> {
    let now = snapshot(repo, targets)?;
    let mut ours: Vec<SvnNode> = now
        .into_iter()
        .filter(|n| matches!(n.status.as_str(), "added" | "deleted") && !before.contains(n))
        .collect();
    // 从深到浅：先 revert 父目录会让子路径的登记状态变得不确定（§8.5）
    ours.sort_by_key(|n| std::cmp::Reverse(n.path.matches('/').count()));
    for n in &ours {
        svn_revert(repo, std::slice::from_ref(&n.path))?;
        if n.status == "deleted" {
            // revert 把它从 base 恢复到了磁盘；D4 之前它是 missing（D3.8 的 checkout 删的）。
            // 用显式文件删除，不用 git clean——那条命令语义太宽（§8.5）
            let p = Path::new(repo).join(&n.path);
            if p.is_file() {
                std::fs::remove_file(&p)?;
            }
        }
    }
    let again = snapshot(repo, targets)?;
    if again != before {
        let diff: Vec<String> = again
            .iter()
            .filter(|n| !before.contains(n))
            .map(|n| format!("{} {}", n.status, n.path))
            .chain(
                before
                    .iter()
                    .filter(|n| !again.contains(n))
                    .map(|n| format!("（应为）{} {}", n.status, n.path)),
            )
            .collect();
        return Err(GitError::new(
            "sync_out_rollback_incomplete",
            format!(
                "撤销 SVN 登记之后，白名单的 svn 状态与交付前不一致：\n{}\n记录已保留。\
                 请用 TortoiseSVN 看一下这些路径，处理好之后在横幅上点「清理」。",
                list_paths(&diff)
            ),
        ));
    }
    Ok(())
}

/// `svn revert`（**不递归**）。空列表拒绝：空 target 会退化成对当前目录操作（必测 #2）。
pub fn svn_revert(repo: &str, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err(empty_targets("svn revert"));
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    svn_batch(repo, &["revert", "--non-interactive", "--"], &refs)
}

fn svn_batch(repo: &str, head: &[&str], paths: &[&str]) -> Result<()> {
    let pegged: Vec<String> = paths.iter().map(|p| peg_safe(p)).collect();
    let mut args: Vec<&str> = head.to_vec();
    args.extend(pegged.iter().map(String::as_str));
    run_svn(repo, &args).map(|_| ()).map_err(|e| {
        GitError::new(
            "sync_out_stage_failed",
            format!("svn {} 失败了：{}", head[0], e.message.trim()),
        )
    })
}

fn empty_targets(what: &str) -> GitError {
    GitError::new(
        "sync_out_empty_targets",
        format!("拒绝发出没有目标的 {what}：空目标会作用于整个工作副本。"),
    )
}
