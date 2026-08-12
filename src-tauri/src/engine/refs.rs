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
        // 附注 tag 的 refs/tags/x 指向一个 tag object，不是提交；target() 拿到的
        // oid 在提交图里根本不存在，前端按 hash 精确匹配提交行，这条 tag 就永远
        // 画不出 pill。必须 peel 一层拿到真正的提交。
        //
        // peel 失败时退回 target()：tag 可以合法地指向 blob 或 tree
        // （git.git 的 junio-gpg-pub 就是挂在一个 blob 上的），这种 tag 没有对应的
        // 提交行，但它确实存在，侧边栏必须把它列出来。丢掉它会让 TAGS 的数量
        // 和 `git tag` 对不上，而且没有任何报错解释为什么少了一条。
        //
        // 退回来的 oid 匹配不到提交行，前端画不出 pill、点击会提示「不在提交图中」，
        // 这是准确的描述，比静默消失好。
        //
        // 只对 tag 做，分支和远程分支本来就直接指向提交。
        let target = if kind == "tag" {
            reference
                .peel_to_commit()
                .ok()
                .map(|c| c.id())
                .or_else(|| reference.target())
        } else {
            // 符号引用（如 refs/remotes/origin/HEAD）的 target() 为 None，跳过
            reference.target()
        };
        if let Some(oid) = target {
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
