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
    /// `unmerged` 不为 0，但分支的内容其实已经全在当前 HEAD 上了。
    ///
    /// squash merge 会把 N 个提交压成一个**全新的提交对象**，SHA 全变，于是
    /// 原分支上那几个提交确实不是 HEAD 的祖先，`unmerged` 照实报了个大于 0 的
    /// 数——数字是真的，它暗示的风险却是假的：内容早就进去了。GitHub 默认走
    /// squash 的话，每合并一个分支就要误报一次，喊到最后真有未合并提交时也不会
    /// 有人看了。
    ///
    /// 判据见 [`content_is_already_in_head`]。
    pub content_merged: bool,
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

/// patch-id 兜底时最多往回扫多少个提交。
///
/// 这一步只在快路径没结论时才跑，每个提交都要算一次 diff。删一个很旧的分支时
/// `merge-base..HEAD` 可能上千，扫到底会把弹框卡住；扫不完就当作没合并，
/// 顶多多显示一次警告。
const PATCHID_SCAN_LIMIT: usize = 500;

/// 分支自己的那部分改动，是不是已经在当前 HEAD 上了。
///
/// **不能拿两边的整棵 tree 去比。** 那只在分支不落后于主干时才成立，而最常见的
/// 情形恰恰是「合并完主干继续往前走」——主干上随便多一个无关提交，整棵树就对
/// 不上了。实测一个已 squash 合并的分支：全树 diff 报 14 个文件有差异，全是合并
/// **之后**主干上的新工作，跟这个分支毫无关系。
///
/// 所以要问的是「分支的贡献进去了吗」，分两步：
///
/// 1. **只比分支碰过的那些路径**。便宜（不扫历史），且覆盖绝大多数「合并完就删」。
///    分支改过的文件在 HEAD 上一字不差，就说明它的活已经落地。
/// 2. 第一步不成立时才上 **patch-id**：把分支的净改动整体算一个指纹，去
///    `merge-base..HEAD` 里找同指纹的提交。第一步失败有两种原因——主干后来又改了
///    这些文件（其实早合并了），或者分支上有 squash 之后的新提交（真没合并）——
///    只有 patch-id 分得开。
///
/// 注意第 2 步算的是**净改动的整体指纹**，不是逐个提交比对；`git cherry` 那样
/// 逐个比对认不出 squash，因为 N 个补丁被合成了 1 个。
///
/// 任何一步出错都当作 false：这一项只决定警告用什么颜色，宁可多喊一次狼来了，
/// 也不要因为算不出来就把真正的未合并提交轻描淡写过去。
fn content_is_already_in_head(repo: &Repository, name: &str) -> bool {
    (|| {
        let branch_oid = repo
            .find_branch(name, BranchType::Local)
            .ok()?
            .get()
            .target()?;
        let head_oid = repo.head().ok()?.peel_to_commit().ok()?.id();
        let base = repo.merge_base(head_oid, branch_oid).ok()?;

        let base_tree = repo.find_commit(base).ok()?.tree().ok()?;
        let branch_tree = repo.find_commit(branch_oid).ok()?.tree().ok()?;
        let head_tree = repo.find_commit(head_oid).ok()?.tree().ok()?;

        // 分支从分叉点以来碰过哪些路径
        let own = repo
            .diff_tree_to_tree(Some(&base_tree), Some(&branch_tree), None)
            .ok()?;
        if own.deltas().len() == 0 {
            return Some(true); // 分支什么都没改，删了当然不丢东西
        }
        let paths: Vec<Vec<u8>> = own
            .deltas()
            .flat_map(|d| [d.old_file().path_bytes(), d.new_file().path_bytes()])
            .flatten()
            .map(|p| p.to_vec())
            .collect();

        let mut only_own_paths = git2::DiffOptions::new();
        for p in &paths {
            only_own_paths.pathspec(p.as_slice());
        }

        // 第一步：只比这些路径
        let scoped = repo
            .diff_tree_to_tree(
                Some(&head_tree),
                Some(&branch_tree),
                Some(&mut only_own_paths),
            )
            .ok()?;
        if scoped.deltas().len() == 0 {
            return Some(true);
        }

        // 第二步：patch-id 兜底
        let want = own.patchid(None).ok()?;
        let mut walk = repo.revwalk().ok()?;
        walk.push(head_oid).ok()?;
        walk.hide(base).ok()?;
        for oid in walk.take(PATCHID_SCAN_LIMIT).flatten() {
            let commit = repo.find_commit(oid).ok()?;
            // 合并提交没有单一的「这个提交改了什么」，跳过
            if commit.parent_count() != 1 {
                continue;
            }
            let parent_tree = commit.parent(0).ok()?.tree().ok()?;
            let tree = commit.tree().ok()?;

            // 先用限定路径的 diff 筛一道。要找的那个提交必定碰过分支改的文件，
            // 碰都没碰过的连算 patch-id 都不用——主干上绝大多数提交都属于这类。
            // 少了这一道筛，判「未合并」时要对 500 个提交逐个算完整 diff，
            // 实测慢到两三秒，而这恰恰是最常见的情形。
            let touched = repo
                .diff_tree_to_tree(Some(&parent_tree), Some(&tree), Some(&mut only_own_paths))
                .ok()?;
            if touched.deltas().len() == 0 {
                continue;
            }

            let diff = repo
                .diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)
                .ok()?;
            if diff.patchid(None).ok()? == want {
                return Some(true);
            }
        }
        Some(false)
    })()
    .unwrap_or(false)
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
        // unmerged 为 0 时这一项没有意义，别白跑一次 tree 比对
        content_merged: unmerged > 0 && content_is_already_in_head(&repo, name),
        worktree: find_worktree(path, name),
    })
}
