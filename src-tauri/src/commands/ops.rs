use crate::engine::git_cli::run_git;
use crate::error::Result;

/// 丢弃改动。分类逻辑在 `engine::ops::discard`。
#[tauri::command(async)]
pub fn discard(path: String, paths: Vec<String>, staged: bool) -> Result<()> {
    crate::vcs::backend_for(&path)?.discard(&paths, staged)
}

#[tauri::command(async)]
pub fn create_branch(path: String, name: String, start_point: Option<String>) -> Result<()> {
    crate::vcs::backend_for(&path)?.create_branch(&name, start_point.as_deref())
}

#[tauri::command(async)]
pub fn delete_branch(path: String, name: String) -> Result<()> {
    crate::vcs::backend_for(&path)?.delete_branch(&name)
}

/// 删除分支前的确认框要显示的信息：对应的远程分支、还有多少提交没合并进来。
///
/// `merged_heads`（JS 侧 `mergedHeads`）是这个分支已合并 PR 的合并点，只作 squash
/// 识别的候选，见 [`crate::engine::branch::delete_info_with_merged_heads`]。
/// 可选：没有 PR 信息（gh 不可用、非 GitHub 仓库）时前端不传。
#[tauri::command(async)]
pub fn branch_delete_info(
    path: String,
    name: String,
    merged_heads: Option<Vec<String>>,
) -> Result<crate::engine::branch::DeleteInfo> {
    crate::engine::branch::delete_info_with_merged_heads(
        &path,
        &name,
        merged_heads.as_deref().unwrap_or_default(),
    )
}

#[tauri::command(async)]
pub fn delete_remote_branch(path: String, remote: String, branch: String) -> Result<()> {
    // 不走 trait Vcs：SVN 混合仓库由这一行拦（第 69 条），不能只靠界面藏入口
    crate::vcs::require_capability(&path, "远程分支", |c| c.remotes)?;
    // 护栏放在 push 之前：默认分支删掉就没了，不能等服务端心情好才挡
    crate::engine::branch::ensure_deletable_on_remote(&path, &remote, &branch)?;
    run_git(&path, &["push", &remote, "--delete", &branch])?;
    Ok(())
}

/// 只跑护栏、不删任何东西：删除序列的第一步不可撤销时，先用它确认后面不会被**护栏**拒绝。
/// push 自身的失败（断网、分支已经不在）它防不了。
///
/// 删除顺序是 worktree → 远程 → 本地分支。`git remote add` 接上的远程没有本地的
/// `origin/HEAD`，弹框勾选框不会灰掉，到远程那一步才被护栏拒绝的话 worktree 已经没了。
/// 答案留给紧接着的 [`delete_remote_branch`]，不多连一次网。
#[tauri::command(async)]
pub fn check_remote_deletable(path: String, remote: String, branch: String) -> Result<()> {
    crate::engine::branch::check_deletable_on_remote(&path, &remote, &branch)
}

/// 用户表明要删远程时调：后台先问好远程默认分支，点确认时护栏直接复用。
///
/// 这趟查询是一次完整的 SSH 握手，放在用户读弹框的那几秒里做，删除时就只剩
/// push 一次连接。只读，失败不报——失败不留痕，护栏到时会自己问，见
/// [`crate::engine::remote_head_cache`]。
///
/// 放到独立线程、命令立刻返回：调用方不等结果，而 `run_git` 没有超时，半开连接下这次
/// 查询可能永远不返回，不能让它占住 tokio 工作线程，那会拖慢所有其他命令。
#[tauri::command(async)]
pub fn prefetch_remote_default(path: String, remote: String) {
    std::thread::spawn(move || {
        crate::engine::branch::prefetch_remote_default(&path, &remote);
    });
}

/// 同 [`prefetch_remote_default`]，按侧边栏给的 `origin/feat/x` 形式定位远程。
#[tauri::command(async)]
pub fn prefetch_remote_default_for_ref(path: String, remote_branch: String) {
    std::thread::spawn(move || {
        crate::engine::branch::prefetch_remote_default_for_ref(&path, &remote_branch);
    });
}

/// 按 `origin/feat/x` 这种完整简称删除远程分支。
///
/// 侧边栏的远程分支那一栏拿到的就是这个形式，拆分放在后端做——远程名可以带
/// 斜杠，前端按第一个 `/` 去切会切错。
#[tauri::command(async)]
pub fn delete_remote_ref(path: String, remote_branch: String) -> Result<()> {
    // 下面的 delete_remote_branch 也会拦，这里先拦是为了连 split 那次读都不做
    crate::vcs::require_capability(&path, "远程分支", |c| c.remotes)?;
    let (remote, branch) = crate::engine::branch::split_remote_ref(&path, &remote_branch)?;
    delete_remote_branch(path, remote, branch)
}

/// 检出一个远程分支，返回落地的本地分支名。
///
/// **不能直接 `git checkout origin/x`**——那会进游离 HEAD，界面上看着像切过去了，
/// 实际上此后的提交不属于任何分支。要建一条跟踪它的本地分支。
///
/// 同名本地分支已经存在时直接切过去，不去动它指向哪：那多半就是用户上次检出
/// 留下的分支，静默把它重置到远程会丢掉本地提交。
#[tauri::command(async)]
pub fn checkout_remote_branch(path: String, remote_branch: String) -> Result<String> {
    crate::vcs::require_capability(&path, "远程分支", |c| c.remotes)?;
    let (local, exists) = crate::engine::branch::local_target_for(&path, &remote_branch)?;
    if exists {
        run_git(&path, &["checkout", &local])?;
    } else {
        run_git(
            &path,
            &["checkout", "-b", &local, "--track", &remote_branch],
        )?;
    }
    Ok(local)
}

#[tauri::command(async)]
pub fn rename_branch(path: String, old: String, new: String) -> Result<()> {
    crate::vcs::backend_for(&path)?.rename_branch(&old, &new)
}

#[tauri::command(async)]
pub fn checkout(path: String, rev: String) -> Result<()> {
    crate::vcs::backend_for(&path)?.checkout(&rev)
}

/// 在某条提交上打一个轻量 tag。
#[tauri::command(async)]
pub fn create_tag(path: String, name: String, rev: String) -> Result<()> {
    crate::vcs::require_capability(&path, "tag", |c| c.tags)?;
    // 不带 -f：同名 tag 已存在时让 git 报错，静默改写一个 tag 的指向
    // 是能把别人已经拉走的引用悄悄挪位置的操作
    run_git(&path, &["tag", &name, &rev])?;
    Ok(())
}

/// 把一条提交摘到当前分支上。
#[tauri::command(async)]
pub fn cherry_pick(path: String, rev: String) -> Result<()> {
    crate::vcs::require_capability(&path, "cherry-pick", |c| c.cherry_pick)?;
    run_git(&path, &["cherry-pick", &rev])?;
    Ok(())
}

/// 生成一条撤销指定提交的新提交。
#[tauri::command(async)]
pub fn revert(path: String, rev: String) -> Result<()> {
    // 它移动 HEAD 并重写工作目录，是不折不扣的写操作（spec §5.1），
    // 却不走 `trait Vcs`——`vcs/svn.rs` 上那 12 个守卫拦不到它。
    // 2026-09-25 独立 review 抓到：`commitActions.ts` 的能力过滤只关掉
    // `new-tag` / `cherry-pick` / `save-patch`，revert 走 `?? true`，
    // 在 SVN 混合仓库上是**完全可用**的。
    crate::engine::svn_lock::assert_not_syncing(&path)?;
    // HEAD 停在基线上时 revert 会移动 `svn/base`（第 70 条）。只在混合仓库里查：
    // 普通 git 仓库里叫这个名字的分支就是一条普通分支
    if crate::vcs::detect_repo_kind(&path) == crate::vcs::RepoKind::SvnHybrid {
        crate::vcs::svn::refuse_while_on_baseline(&path)?;
    }
    // --no-edit：GUI 里没有终端，不加这个 git 会拉起编辑器等一个永远不会来的输入
    run_git(&path, &["revert", "--no-edit", &rev])?;
    Ok(())
}

/// 把一条提交导出成 `git am` 能回放的 .patch 文件。
///
/// 用 `format-patch --stdout` 自己写文件，而不是让 git 往目录里生成：
/// git 会按提交说明自动编号命名，用户在存盘框里选的文件名就白选了。
#[tauri::command(async)]
pub fn save_patch(path: String, rev: String, out: String) -> Result<()> {
    crate::vcs::require_capability(&path, "导出 patch", |c| c.patches)?;
    // -1 限定只导出这一条，不带上它的祖先
    let patch =
        crate::engine::git_cli::run_git_bytes(&path, &["format-patch", "-1", "--stdout", &rev])?;
    std::fs::write(&out, patch)?;
    Ok(())
}

/// Stage / Unstage 只写 index，前端随后自己就地刷新。登记成「自己写的」，
/// 监听就不再为这次 index 变化补发一整轮刷新（`commands::watch::OwnIndexWrite`）。
#[tauri::command(async)]
pub fn stage(path: String, paths: Vec<String>) -> Result<()> {
    let _own = crate::commands::watch::OwnIndexWrite::begin(&path);
    crate::vcs::backend_for(&path)?.stage(&paths)
}

#[tauri::command(async)]
pub fn unstage(path: String, paths: Vec<String>) -> Result<()> {
    let _own = crate::commands::watch::OwnIndexWrite::begin(&path);
    crate::vcs::backend_for(&path)?.unstage(&paths)
}

#[tauri::command(async)]
pub fn commit(path: String, message: String, amend: bool) -> Result<()> {
    crate::vcs::backend_for(&path)?.commit(&message, amend)
}
