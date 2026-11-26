use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub path: String,
    /// 给人看的分支名（游离时是 "HEAD"）。
    pub head: String,
    /// HEAD 解析出的完整 oid。工作区伪行靠它挂到图上。
    pub head_hash: String,
    /// HEAD 的父提交，顺序同 git（第一个是提交 / 合并前的 HEAD）。根提交为空。
    /// 前端据此认出「叠在原来 HEAD 上的单父新提交」，本应用提交后不必再跑 SVN 第二档
    /// （`svnSync.ts` 的 `isPlainCommitOn`、`repoStore.ts` 的 `noteOwnCommit`）。
    pub head_parents: Vec<String>,
}

pub fn open(path: &str) -> Result<RepoInfo> {
    let repo = Repository::open(path)
        .map_err(|_| GitError::new("not_repo", format!("{} is not a git repository", path)))?;
    let head = repo.head()?;
    let head_name = head.shorthand().unwrap_or("HEAD").to_string();
    // 符号引用要 resolve 之后才有 target
    let head_oid = head.resolve().ok().and_then(|r| r.target());
    let head_hash = head_oid.map(|oid| oid.to_string()).unwrap_or_default();
    // 读不出来按空：前端拿空列表一律当「要校验」，只是少省一次
    let head_parents = head_oid
        .and_then(|oid| repo.find_commit(oid).ok())
        .map(|c| c.parent_ids().map(|p| p.to_string()).collect())
        .unwrap_or_default();
    Ok(RepoInfo {
        path: path.to_string(),
        head: head_name,
        head_hash,
        head_parents,
    })
}

/// 提交前的读取（第 74 条）：返回 `git commit` 需要追加的参数。
///
/// - index 里还有冲突：直接拒绝。git 自己也会拒，但只给英文原文，用户看不出还剩几个
/// - 停在**单个** cherry-pick / revert 中间：追加 `--allow-empty`。冲突解决成与 HEAD 相同时
///   裸 `git commit` 报 nothing to commit，操作一直停着，界面上又没有别的出口。
///   merge 不需要：合并提交本身就允许树与 HEAD 相同。一次挑多个（sequencer）时不加：
///   提交只了结当前这一个，结果为空的那一个该 `--skip`，不替用户记空提交
///
/// 不在任何操作中间时什么都不加——平时什么都没暂存就不该凭空生成空提交。
///
/// `amend` 时 merge / cherry-pick / revert 进行中一律拒绝。前两种 git 自己也拒（英文）；
/// revert 实测 git **不拒**，会把上一个提交改写成 revert 的结果并结束 revert，改错了提交。
/// rebase 不拦：停在 edit 上 amend 正是它的用法。
///
/// 冲突检查只是提前给个中文说法，判断不了就交给 git：libgit2 读不了的 index
/// （split index 等）时跳过，git 自己照样会拒绝未解决的冲突。
pub fn commit_extra_args(path: &str, amend: bool) -> Result<&'static [&'static str]> {
    use git2::RepositoryState as S;
    if let Ok(paths) = crate::engine::git_read::unmerged_paths(path) {
        if !paths.is_empty() {
            return Err(GitError::new(
                "commit_unresolved_conflicts",
                format!(
                    "还有 {} 个冲突文件未解决，解决并暂存之后才能提交",
                    paths.len()
                ),
            ));
        }
    }
    let Ok(repo) = Repository::open(path) else {
        return Ok(&[]);
    };
    let state = repo.state();
    if amend {
        let op = match state {
            S::Merge => Some("merge"),
            S::CherryPick | S::CherryPickSequence => Some("cherry-pick"),
            S::Revert | S::RevertSequence => Some("revert"),
            _ => None,
        };
        if let Some(op) = op {
            return Err(GitError::new(
                "commit_amend_in_progress",
                format!("{op} 进行中不能 Amend，取消勾选后再提交"),
            ));
        }
    }
    Ok(match state {
        S::CherryPick | S::Revert => &["--allow-empty"],
        _ => &[],
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
        S::CherryPick => "cherry-pick",
        S::Revert => "revert",
        // 一次挑 / 撤多个提交：提交一下只了结当前这一个，收尾是 `--continue`（第 51 条）。
        // 和单个分开，前端才不会说「提交以完成」（第 74 条）
        S::CherryPickSequence => "cherry-pick-sequence",
        S::RevertSequence => "revert-sequence",
        // bisect 不会留下冲突文件，也不挡提交，不值得占一条横幅
        S::Clean | S::Bisect => "normal",
    })
}
