use crate::error::Result;
use git2::{BranchType, Repository};
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct UpstreamInfo {
    /// 当前分支名（不带 refs/heads/ 前缀）；游离 HEAD 时是 "HEAD"。
    pub branch: String,
    /// 上游分支的简称，如 "origin/master"。没有设上游时为 None。
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
}

/// 读当前分支相对其上游的领先/落后数。
///
/// 走 libgit2 而不是 `git rev-list --count`：这是读操作，开子进程不划算，
/// 而且它会跟着每次 repo-changed 事件刷新。
pub fn read(path: &str) -> Result<UpstreamInfo> {
    let repo = Repository::open(path)?;
    let head = repo.head()?;
    let branch_name = head.shorthand().unwrap_or("HEAD").to_string();

    let none = |branch: String| UpstreamInfo { branch, upstream: None, ahead: 0, behind: 0 };

    // 游离 HEAD 没有分支，也就没有上游
    if !head.is_branch() {
        return Ok(none(branch_name));
    }

    let branch = match repo.find_branch(&branch_name, BranchType::Local) {
        Ok(b) => b,
        Err(_) => return Ok(none(branch_name)),
    };
    let upstream = match branch.upstream() {
        Ok(u) => u,
        // 新建分支还没 push 过就没有上游，这是正常状态而不是错误
        Err(_) => return Ok(none(branch_name)),
    };

    let local_oid = head.target();
    let upstream_oid = upstream.get().target();
    let upstream_name = upstream.name()?.unwrap_or("").to_string();

    let (ahead, behind) = match (local_oid, upstream_oid) {
        (Some(l), Some(u)) => repo.graph_ahead_behind(l, u)?,
        _ => (0, 0),
    };

    Ok(UpstreamInfo {
        branch: branch_name,
        upstream: Some(upstream_name),
        ahead,
        behind,
    })
}
