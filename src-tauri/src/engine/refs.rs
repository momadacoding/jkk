use crate::error::Result;
use git2::{BranchType, Repository};
use serde::Serialize;

#[derive(Serialize)]
pub struct RefPointer {
    pub name: String,
    pub hash: String,
    pub kind: String, // "head", "branch", "remote", "tag"
    /// 本地分支相对其上游的领先数。没有上游、或不是本地分支时为 None。
    ///
    /// None 和 Some(0) 含义不同：没上游时侧边栏根本不画徽章，同步时画一个 0
    /// 也是噪音——前端靠这个区别决定显不显示。
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
}

/// 一个本地分支相对其上游的领先/落后数。
///
/// 每个分支各算各的，不跟着 HEAD 走：侧边栏要在每一行右边显示它自己的 ↑↓。
fn upstream_counts(repo: &Repository, full_name: &str) -> Option<(usize, usize)> {
    let short = full_name.strip_prefix("refs/heads/")?;
    let branch = repo.find_branch(short, BranchType::Local).ok()?;
    let upstream = branch.upstream().ok()?;
    let local_oid = branch.get().target()?;
    let upstream_oid = upstream.get().target()?;
    repo.graph_ahead_behind(local_oid, upstream_oid).ok()
}

pub fn collect(repo: &Repository) -> Result<Vec<RefPointer>> {
    let mut refs = Vec::new();
    for reference in repo.references()? {
        let reference = reference?;
        let name = reference.name().unwrap_or("").to_string();
        let kind = if name == "HEAD" {
            "head"
        } else if name.starts_with("refs/heads/") {
            "branch"
        } else if name.starts_with("refs/remotes/") {
            "remote"
        } else if name.starts_with("refs/tags/") {
            "tag"
        } else {
            "other"
        };
        // 符号引用（如 refs/remotes/origin/HEAD）的 target() 为 None，跳过
        if let Some(oid) = reference.target() {
            let counts = if kind == "branch" {
                upstream_counts(repo, &name)
            } else {
                None
            };
            refs.push(RefPointer {
                name: name.clone(),
                hash: oid.to_string(),
                kind: kind.to_string(),
                ahead: counts.map(|(a, _)| a),
                behind: counts.map(|(_, b)| b),
            });
        }
    }

    // HEAD 不在 references() 的遍历范围内（那只列 refs/*），必须单独取。
    // 少了它前端就认不出当前 commit，圆点强调和 .ref-pill.head 样式都用不上。
    if let Ok(head) = repo.head() {
        if let Some(oid) = head.resolve().ok().and_then(|r| r.target()) {
            refs.push(RefPointer {
                name: "HEAD".to_string(),
                hash: oid.to_string(),
                kind: "head".to_string(),
                ahead: None,
                behind: None,
            });
        }
    }

    Ok(refs)
}
