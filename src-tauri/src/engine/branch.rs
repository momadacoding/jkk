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
    /// 有多少提交只在这个分支上、不在 [`compared_to`](Self::compared_to) 上。
    ///
    /// 界面拿它提示「删了就找不回来」。语义对齐 `git branch -d` 的判断：
    /// 它正是在这个数不为 0 时拒绝删除。
    pub unmerged: usize,
    /// 上面那个数是拿哪个 ref 比出来的，如 `main`、`origin/main`。
    ///
    /// 必须显示出来。**不写清楚的话这个数字无从判断**：用 worktree 时主仓库的
    /// HEAD 常年停在某个不相干的地方，甚至是游离的，只说「不在当前分支上」
    /// 等于什么都没说。挑的是候选目标里落下最少的那个，见 [`merge_targets`]。
    pub compared_to: String,
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

/// 算 patch-id 用的 diff 选项：**不带上下文行**。
///
/// 这不是为了省字节，是正确性问题。patch-id 会剥掉 hunk 头里的行号，但**不剥
/// 上下文行**。squash 提交的补丁是相对「合并时的主干」算出来的，分支的净改动是
/// 相对**分叉点**算出来的；分叉之后、squash 之前主干动过的行会落进两边各自的
/// 上下文，于是增删的行一字不差，指纹却不一样——而「合并之前主干先合了别的 PR」
/// 正是 GitHub 上最普通的一天。
///
/// 真实仓库上复现过：分支改了 33 个文件，其中 32 个的补丁完全一致，只有 README
/// 因为中间那个 PR 删掉了它的一行上下文而对不上。整体指纹是全有或全无的，一个
/// 文件差一行上下文就把整个判定拖下水。压到零上下文后两边立刻一致。
///
/// 代价是判定略微放宽：同样的增删行出现在文件里的**另一个位置**也会算命中。
/// 这一项只决定警告的措辞，真正的护栏是照实报出来的 `unmerged` 数；比起每
/// squash 合并一个分支就误报一次、喊到最后没人再看，这个放宽是划算的。
fn no_context_diff() -> git2::DiffOptions {
    let mut opts = git2::DiffOptions::new();
    opts.context_lines(0);
    opts
}

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
///    分支改过的文件在目标上一字不差，就说明它的活已经落地。
/// 2. 第一步不成立时才上 **patch-id**：把分支的净改动整体算一个指纹，去
///    `merge-base..目标` 里找同指纹的提交。第一步失败有两种原因——目标后来又改了
///    这些文件（其实早合并了），或者分支上有 squash 之后的新提交（真没合并）——
///    只有 patch-id 分得开。
///
/// 注意第 2 步算的是**净改动的整体指纹**，不是逐个提交比对；`git cherry` 那样
/// 逐个比对认不出 squash，因为 N 个补丁被合成了 1 个。而且这个指纹必须**在零
/// 上下文下**算，见 [`no_context_diff`]。
///
/// 任何一步出错都当作 false：这一项只决定警告用什么颜色，宁可多喊一次狼来了，
/// 也不要因为算不出来就把真正的未合并提交轻描淡写过去。
/// `deep` 为假时只跑第一步。判「没合并」要把整个扫描跑完才收工，对每个候选目标
/// 都跑一遍实测能到两秒多；深挖只留给最可能命中的那个目标，其余的用便宜的那一步
/// 兜一下就够了。
fn content_is_already_in(
    repo: &Repository,
    branch_oid: git2::Oid,
    target: git2::Oid,
    deep: bool,
) -> bool {
    (|| {
        let base = repo.merge_base(target, branch_oid).ok()?;

        let base_tree = repo.find_commit(base).ok()?.tree().ok()?;
        let branch_tree = repo.find_commit(branch_oid).ok()?.tree().ok()?;
        let head_tree = repo.find_commit(target).ok()?.tree().ok()?;

        // 分支从分叉点以来碰过哪些路径。零上下文只影响 patch-id，不影响这里要的
        // 文件清单——delta 是按文件给的，跟带几行上下文无关。
        let mut patch_opts = no_context_diff();
        let own = repo
            .diff_tree_to_tree(Some(&base_tree), Some(&branch_tree), Some(&mut patch_opts))
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
        if !deep {
            return Some(false);
        }

        // 第二步：patch-id 兜底
        let want = own.patchid(None).ok()?;
        let mut walk = repo.revwalk().ok()?;
        walk.push(target).ok()?;
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
                .diff_tree_to_tree(Some(&parent_tree), Some(&tree), Some(&mut patch_opts))
                .ok()?;
            if diff.patchid(None).ok()? == want {
                return Some(true);
            }
        }
        Some(false)
    })()
    .unwrap_or(false)
}

/// 「合并进哪里算数」的候选目标，按显示名 + commit 给出。
///
/// **不能只看 HEAD。** 实测踩过：主仓库处于游离 HEAD，停在某个功能分支的 tip 上，
/// 那个 tip 落后主干两个提交、正好缺了目标分支的 squash 合并提交，于是一个早就
/// 合并进 main 的分支被判成「有 2 个提交没合并」。数字相对 HEAD 是对的，但用户
/// 想知道的从来都是「进主干了吗」。用 worktree 的人尤其容易撞上——主仓库的 HEAD
/// 常年停在某个不相干的地方。
///
/// 候选顺序：主干（本地的默认分支优先，其次远程的），最后才是 HEAD。三个都算，
/// 取最有利的结果——只要这些活在任何一处留着，删掉分支就不丢东西。
fn merge_targets(repo: &Repository) -> Vec<(String, git2::Oid)> {
    let mut out: Vec<(String, git2::Oid)> = Vec::new();
    let mut push = |name: String, oid: git2::Oid| {
        if !out.iter().any(|(_, o)| *o == oid) {
            out.push((name, oid));
        }
    };

    // 默认分支叫什么，先问 origin/HEAD，问不到再按常见名字猜
    let default_name = repo
        .find_reference("refs/remotes/origin/HEAD")
        .ok()
        .and_then(|r| r.resolve().ok())
        .and_then(|r| r.shorthand().map(|s| s.to_string()))
        .and_then(|s| s.strip_prefix("origin/").map(|s| s.to_string()));

    let names: Vec<String> = match default_name {
        Some(n) => vec![n],
        None => vec!["main".to_string(), "master".to_string()],
    };
    for n in names {
        if let Ok(b) = repo.find_branch(&n, BranchType::Local) {
            if let Some(oid) = b.get().target() {
                push(n.clone(), oid);
            }
        }
        // 本地那条可能很久没 pull 了，远程跟踪分支一并算上
        if let Ok(b) = repo.find_branch(&format!("origin/{n}"), BranchType::Remote) {
            if let Some(oid) = b.get().target() {
                push(format!("origin/{n}"), oid);
            }
        }
    }

    if let Some(oid) = repo.head().ok().and_then(|h| h.target()) {
        let name = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(|s| s.to_string()))
            .unwrap_or_else(|| "HEAD".to_string());
        push(name, oid);
    }
    out
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

    // 算不出来就当作「零个未合并、目标是 HEAD」：这两项只用于多显示一行提示，
    // 不该因为它们失败而让整个确认框打不开
    let (unmerged, compared_to, content_merged) = (|| {
        let branch_oid = repo
            .find_branch(name, BranchType::Local)
            .ok()?
            .get()
            .target()?;
        let targets = merge_targets(&repo);
        if targets.is_empty() {
            return None;
        }

        // 挑落下最少的那个目标来报数——这些活只要在任何一处留着就不算丢
        let mut best: Option<(String, git2::Oid, usize)> = None;
        for (label, oid) in &targets {
            let ahead = repo
                .graph_ahead_behind(branch_oid, *oid)
                .map(|(a, _)| a)
                .unwrap_or(usize::MAX);
            // 等价于 `is_none_or`，但那个方法 1.82 才稳定，而本项目的 MSRV 是
            // 1.77.2（跟 Tauri 2 对齐）。用它会让 1.77~1.81 的人按 README 装完
            // 环境却编译不过。
            if best.as_ref().map_or(true, |(_, _, n)| ahead < *n) {
                best = Some((label.clone(), *oid, ahead));
            }
            if ahead == 0 {
                break;
            }
        }
        let (label, best_oid, ahead) = best?;
        if ahead == 0 || ahead == usize::MAX {
            return Some((0, label, false));
        }

        // 提交对象没进去，内容可能已经进去了（squash / rebase 合并）。落下最少的
        // 那个目标最可能已经含着这些活，深挖它；其余的只跑便宜的第一步兜一下
        let merged = content_is_already_in(&repo, branch_oid, best_oid, true)
            || targets
                .iter()
                .filter(|(_, oid)| *oid != best_oid)
                .any(|(_, oid)| content_is_already_in(&repo, branch_oid, *oid, false));
        Some((ahead, label, merged))
    })()
    .unwrap_or((0, "HEAD".to_string(), false));

    Ok(DeleteInfo {
        upstream,
        remote,
        remote_branch,
        unmerged,
        compared_to,
        content_merged,
        worktree: find_worktree(path, name),
    })
}
