use crate::error::Result;
use git2::Repository;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    /// stash@{N} 里的 N。0 是最新的一条。
    pub index: usize,
    pub message: String,
    /// stash 本身就是一个提交，前端拿它复用现成的 `commit:<hash>` diff scope。
    pub hash: String,
}

/// 列出所有 stash，最新的在前。
pub fn list(path: &str) -> Result<Vec<StashEntry>> {
    // stash_foreach 需要可变借用
    let mut repo = Repository::open(path)?;
    let mut entries = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        entries.push(StashEntry {
            index,
            message: message.to_string(),
            hash: oid.to_string(),
        });
        true
    })?;
    Ok(entries)
}
