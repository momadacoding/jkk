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

/// 仓库是否停在某个需要用户收尾的操作中间（前端据此显示顶部横幅）。
///
/// 交给 libgit2 判断而不是自己拼 `<path>/.git/MERGE_HEAD`：linked worktree 里
/// `.git` 是个文件，状态文件在 `<主仓库>/.git/worktrees/<name>/` 下，
/// 手工拼路径在 worktree 里永远判成 normal。
pub fn state(path: &str) -> Result<&'static str> {
    use git2::RepositoryState as S;
    let repo = Repository::open(path)?;
    Ok(match repo.state() {
        S::Merge => "merge",
        S::Rebase
        | S::RebaseInteractive
        | S::RebaseMerge
        | S::ApplyMailbox
        | S::ApplyMailboxOrRebase => "rebase",
        S::CherryPick | S::CherryPickSequence => "cherry-pick",
        S::Revert | S::RevertSequence => "revert",
        // bisect 不会留下冲突文件，也不挡提交，不值得占一条横幅
        S::Clean | S::Bisect => "normal",
    })
}
