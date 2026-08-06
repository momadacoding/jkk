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
    pub author: String,
    /// 提交时间，Unix 秒。
    pub time: i64,
    /// 父提交。stash 是个 merge 提交，第一父是 stash 时所在的那个提交。
    pub parents: Vec<String>,
}

/// 列出所有 stash，最新的在前。
///
/// 除了索引和信息，还要带上作者/时间/父提交：stash 提交不被任何分支引用，
/// 不会出现在 `load_graph` 的输出里，详情面板只能靠这里给的元数据渲染。
pub fn list(path: &str) -> Result<Vec<StashEntry>> {
    // stash_foreach 需要可变借用，所以先把 oid 收集出来，再逐个查提交
    let mut repo = Repository::open(path)?;
    let mut raw = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        raw.push((index, message.to_string(), *oid));
        true
    })?;

    let mut entries = Vec::with_capacity(raw.len());
    for (index, message, oid) in raw {
        let commit = repo.find_commit(oid)?;
        entries.push(StashEntry {
            index,
            message,
            hash: oid.to_string(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        });
    }
    Ok(entries)
}
