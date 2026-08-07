use crate::error::Result;
use git2::{BranchType, Repository};
use serde::Serialize;

/// 检出了待删分支的那个 worktree。
#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeTarget {
    /// worktree 目录名，界面上显示这个。
    pub name: String,
    /// 完整路径，`git worktree remove` 要用它。
    pub path: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeleteInfo {
    /// 对应的远程分支简称，如 `origin/feat/x`；没有就是 None，界面上不显示勾选框。
    pub upstream: Option<String>,
    /// 上游所在的远程名，如 `origin`。删远程分支时要用。
    pub remote: Option<String>,
    /// 该分支在远程上的名字（去掉远程名前缀），如 `feat/x`。
    ///
    /// 不让前端去切 `origin/feat/x` 这个字符串：远程名本身可以带斜杠，
    /// 按第一个 `/` 切会把分支名切错。
    pub remote_branch: Option<String>,
    /// 有多少提交只在这个分支上、不在当前 HEAD 上。
    ///
    /// 界面拿它提示「删了就找不回来」。语义对齐 `git branch -d` 的判断：
    /// 它正是在这个数不为 0 时拒绝删除。
    pub unmerged: usize,
    /// 检出了这个分支的 worktree（主工作区除外），没有就是 None。
    ///
    /// 这不是锦上添花的信息：分支被 worktree 占着时 `git branch -D` 会直接拒绝
    /// （`cannot delete branch 'x' used by worktree at '...'`），非删掉那个
    /// worktree 不可。界面据此多给一个勾选框。
    pub worktree: Option<WorktreeTarget>,
}

/// 找出这个分支对应的远程分支。
///
/// 先看配置的上游；没配上游时再扫一遍各个远程下的同名引用——用
/// `git push` 而不是 `git push -u` 推上去的分支就属于这种，
/// 光看上游会以为它没有远程分支，勾选框就不会出现。
fn find_remote_branch(repo: &Repository, short: &str) -> Option<(String, String, String)> {
    let full = format!("refs/heads/{short}");

    if let Ok(branch) = repo.find_branch(short, BranchType::Local) {
        if let Ok(upstream) = branch.upstream() {
            if let Ok(Some(shorthand)) = upstream.name() {
                let remote = repo
                    .branch_upstream_remote(&full)
                    .ok()
                    .and_then(|buf| buf.as_str().map(|s| s.to_string()));
                if let Some(remote) = remote {
                    let on_remote = shorthand
                        .strip_prefix(&format!("{remote}/"))
                        .unwrap_or(short)
                        .to_string();
                    return Some((shorthand.to_string(), remote, on_remote));
                }
            }
        }
    }

    for remote in repo.remotes().ok()?.iter().flatten() {
        let shorthand = format!("{remote}/{short}");
        if repo
            .find_reference(&format!("refs/remotes/{shorthand}"))
            .is_ok()
        {
            return Some((shorthand, remote.to_string(), short.to_string()));
        }
    }
    None
}

/// 找出检出了这个分支的 worktree。
///
/// **跳过主工作区**：它删不掉（`git worktree remove` 拒绝），给出勾选框只会
/// 是个死路。主工作区占着某个分支时让 git 的原话直接报出来更诚实。
///
/// 查不出来（`git worktree list` 失败、仓库根本没有 worktree）就当作没有：
/// 这一项只影响多不多一个勾选框，不该因为它让整个确认框打不开。
fn find_worktree(path: &str, name: &str) -> Option<WorktreeTarget> {
    crate::engine::worktree::read_worktrees(path)
        .ok()?
        .into_iter()
        .find(|w| !w.is_main && w.branch == name)
        .map(|w| WorktreeTarget {
            name: w.name,
            path: w.path,
        })
}

/// 把 `origin/feat/x` 这种完整简称拆成（远程名, 远程上的分支名）。
///
/// 不能简单按第一个 `/` 切：远程名本身可以带斜杠。所以拿仓库里真实配置的
/// 远程名去匹配，并且**取最长的那个匹配**——同时存在 `origin` 和 `origin/mirror`
/// 两个远程时，`origin/mirror/x` 只有取长的才切得对。
pub fn split_remote_ref(path: &str, remote_ref: &str) -> Result<(String, String)> {
    let repo = Repository::open(path)?;
    let mut best: Option<(String, String)> = None;
    for remote in repo.remotes()?.iter().flatten() {
        if let Some(rest) = remote_ref.strip_prefix(&format!("{remote}/")) {
            let better = best
                .as_ref()
                .map(|(r, _): &(String, String)| remote.len() > r.len())
                .unwrap_or(true);
            if better {
                best = Some((remote.to_string(), rest.to_string()));
            }
        }
    }
    best.ok_or_else(|| {
        crate::error::GitError::new("bad_ref", format!("{remote_ref} 不属于任何已配置的远程"))
    })
}

/// 远程分支该检出成哪个本地分支名，以及这个本地分支是不是已经存在。
pub fn local_target_for(path: &str, remote_ref: &str) -> Result<(String, bool)> {
    let (_, local) = split_remote_ref(path, remote_ref)?;
    let repo = Repository::open(path)?;
    let exists = repo.find_branch(&local, BranchType::Local).is_ok();
    Ok((local, exists))
}

/// 删除分支前要在确认框里显示的信息。
pub fn delete_info(path: &str, name: &str) -> Result<DeleteInfo> {
    let repo = Repository::open(path)?;
    let (upstream, remote, remote_branch) = match find_remote_branch(&repo, name) {
        Some((u, r, b)) => (Some(u), Some(r), Some(b)),
        None => (None, None, None),
    };

    // 算不出来就当作 0：这个数只用于多显示一行警告，
    // 不该因为它失败而让整个确认框打不开
    let unmerged = (|| {
        let branch = repo.find_branch(name, BranchType::Local).ok()?;
        let branch_oid = branch.get().target()?;
        let head_oid = repo.head().ok()?.peel_to_commit().ok()?.id();
        let (ahead, _) = repo.graph_ahead_behind(branch_oid, head_oid).ok()?;
        Some(ahead)
    })()
    .unwrap_or(0);

    Ok(DeleteInfo {
        upstream,
        remote,
        remote_branch,
        unmerged,
        worktree: find_worktree(path, name),
    })
}
