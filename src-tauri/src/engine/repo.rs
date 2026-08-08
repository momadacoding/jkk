use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub path: String,
    /// 给人看的分支名（游离时是 "HEAD"）。
    pub head: String,
    /// HEAD 解析出的完整 oid。工作区伪行靠它挂到图上。
    pub head_hash: String,
}

pub fn open(path: &str) -> Result<RepoInfo> {
    let repo = Repository::open(path)
        .map_err(|_| GitError::new("not_repo", format!("{} is not a git repository", path)))?;
    let head = repo.head()?;
    let head_name = head.shorthand().unwrap_or("HEAD").to_string();
    // 符号引用要 resolve 之后才有 target
    let head_hash = head
        .resolve()
        .ok()
        .and_then(|r| r.target())
        .map(|oid| oid.to_string())
        .unwrap_or_default();
    Ok(RepoInfo {
        path: path.to_string(),
        head: head_name,
        head_hash,
    })
}
