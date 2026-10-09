//! `sync_out` 的前置检查（spec §8.1 第 0–8 条）与 D1 / D2 清单。**纯读**：不拿锁、
//! 不写 journal、不建 ref、不碰磁盘。确认框里的全部事实都来自这里（计划 B1）。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_baseline::parse_trailer;
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_deliver::{plan_deliver, LocalOnlyCommit};
use crate::engine::svn_externals::detect_pinned;
use crate::engine::svn_include_check::{is_w_file, normalize_roots};
use crate::engine::svn_journal;
use crate::engine::svn_lock;
use crate::engine::svn_preflight::svn_targets;
use crate::engine::svn_sync::{check_sync, SyncVerdict};
use crate::engine::svn_sync_check::{
    check_git_idle, current_branch, list_paths, precheck_include_set, rev_parse, unfinished_error,
};
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_info, parse_update_status};
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};

/// D1 清单里的一条。`status` 只会是 `A` / `M` / `D`（`--no-renames`，其余状态拒绝）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverChange {
    pub status: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutPreview {
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    pub baseline_rev: u64,
    /// 服务器 HEAD revision。执行时这一次查询就是 D3.5（计划 B13）。
    pub remote_rev: u64,
    pub deliver_oid: String,
    pub deliver_tree: String,
    pub changes: Vec<DeliverChange>,
    pub local_only: Vec<LocalOnlyCommit>,
    /// 白名单目录里 git 管不到、有本地修改的文件（§8.1 第 2 条后半）。D6 会走到它们
    /// 头上，所以**非空时必须由用户确认**，确认只授权 update，不授权任何 revert。
    pub outside_changes: Vec<String>,
}

/// 给界面用的入口：锁被拿着、或有未收尾的 journal 时直接拒绝。
pub fn preview_sync_out(repo: &str) -> Result<SyncOutPreview> {
    svn_lock::assert_not_syncing(repo)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    check_sync_out(repo)
}

/// §8.1 第 0–8 条 + D0.5（对象层）+ D1 / D2。**不查锁**：持锁者在锁内复核时调它。
///
/// 顺序：纯本地的先查、便宜的先查，走网络的放最后（§8.1 第 5 条）。
pub(crate) fn check_sync_out(repo: &str) -> Result<SyncOutPreview> {
    if detect_repo_kind(repo) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库。",
        ));
    }
    let cfg = read_config(repo)?;

    check_git_idle(repo, "sync_out_git_busy", "交付")?; // 第 3 条
    let orig_branch = attached_branch(repo)?; // 第 4 条
    let orig_head = rev_parse(repo, "HEAD")?;
    let baseline_oid = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    require_on_latest_baseline(repo, &baseline_oid, &orig_head, &orig_branch)?; // 第 0 条

    if !detect_pinned(repo, &cfg.whitelist)?.is_empty() {
        return Err(GitError::new(
            "sync_out_pinned_unsupported",
            "白名单所在的 external 被 `-r` 钉住了（pinned 模式）。本版本的「更新」还不支持\
             这种形态，交付之后基线也就跟不上，所以不在这里冒险。请先用 TortoiseSVN 交付。",
        ));
    }

    let status = check_sync(repo)?; // 第 1 条
    if !matches!(status.verdict, SyncVerdict::InSync) {
        return Err(GitError::new(
            "sync_out_out_of_sync",
            format!(
                "基线已经失同步（白名单 revision {:?}，基线记的是 r{}）。在失同步的基线上算出来的\
                 「改了什么」不可信，请先处理顶部的失同步横幅（多半点「更新」即可）。",
                status.node_revs, status.baseline_rev
            ),
        ));
    }

    let dirty = git_dirty(repo, &cfg)?; // 第 2 条前半
    if !dirty.is_empty() {
        return Err(GitError::new(
            "sync_out_dirty",
            format!(
                "白名单里还有没提交的改动：\n{}\n交付的是提交过的内容，这些改动不会被悄悄带上，\
                 也不会被悄悄排除。请先提交（不打算交付的就勾「仅本地，不交付」提交），或者撤销。",
                list_paths(&dirty)
            ),
        ));
    }

    // 第 6、7 条与 D0.5：对象层，不碰磁盘（计划 B2）
    let plan = plan_deliver(repo, &baseline_oid, &orig_head)?;
    let changes = changed_files(repo, &cfg, &baseline_oid, &plan.deliver_oid)?; // D1 / D2
    if changes.is_empty() {
        return Err(nothing_to_deliver(plan.local_only.len()));
    }

    precheck_include_set(repo, &cfg.whitelist, &cfg.extensions)?; // 第 8 条

    let outside_changes = outside_changes(repo, &cfg)?; // 第 2 条后半

    // 第 5 条：走网络，放最后
    let root = repos_root(repo, &baseline_oid)?;
    let remote_rev = remote_head_rev(repo, &root)?;
    if remote_rev != status.baseline_rev {
        return Err(GitError::new(
            "sync_out_server_ahead",
            format!(
                "服务器上已经有 r{remote_rev}，你的基线还在 r{}。请先「更新」，再交付——\
                 在旧 revision 上交付，SVN 也会拒绝有冲突的文件。什么都没有改动。",
                status.baseline_rev
            ),
        ));
    }

    Ok(SyncOutPreview {
        orig_branch,
        orig_head,
        baseline_oid,
        baseline_rev: status.baseline_rev,
        remote_rev,
        deliver_oid: plan.deliver_oid,
        deliver_tree: plan.deliver_tree,
        changes,
        local_only: plan.local_only,
        outside_changes,
    })
}

fn attached_branch(repo: &str) -> Result<String> {
    let Some(branch) = current_branch(repo)? else {
        let at = rev_parse(repo, "HEAD").unwrap_or_default();
        return Err(GitError::new(
            "sync_out_detached_head",
            format!(
                "HEAD 没有附着在分支上（停在 {}）。交付之后要把一个分支接到新基线上，\
                 detached HEAD 没有分支可接。请 `git switch -c <新分支名>` 或切回原来的分支。",
                &at[..at.len().min(10)]
            ),
        ));
    };
    if branch == BASELINE_BRANCH {
        return Err(GitError::new(
            "sync_out_on_baseline",
            format!("当前在基线分支 `{BASELINE_BRANCH}` 上。请切回你的工作分支再交付。"),
        ));
    }
    Ok(branch)
}

/// §8.1 第 0 条：唯一挡得住「拿旧基线的分支交付」的一条（spec §8.1.2）。
fn require_on_latest_baseline(repo: &str, base: &str, head: &str, branch: &str) -> Result<()> {
    let (code, _) = run_git_code(repo, &["merge-base", "--is-ancestor", base, head])?;
    match code {
        0 => Ok(()),
        1 => Err(GitError::new(
            "sync_out_behind_baseline",
            format!(
                "分支 `{branch}` 还停在旧基线上，没有包含最新的 `{BASELINE_BRANCH}`。\
                 强行交付会把别人（或你自己另一个分支）上一次交付的文件当成删除提上去。\
                 请先对这个分支点一次「更新」，再交付。什么都没有改动。"
            ),
        )),
        _ => Err(GitError::new(
            "sync_out_behind_baseline",
            format!(
                "查不出分支 `{branch}` 是否包含最新基线（git 退出 {code}）。查不成不等于没问题，这里停下。"
            ),
        )),
    }
}

/// 白名单内 git 的未提交改动，含未跟踪文件。
fn git_dirty(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Err(GitError::new("svn_empty_whitelist", "白名单是空的。")); // §12.2
    }
    let mut args = vec!["status", "--porcelain", "-z", "--untracked-files=all", "--"];
    args.extend(roots.iter().map(String::as_str));
    let out = run_git(repo, &args)?;
    let mut paths = Vec::new();
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = it.next() {
        if entry.len() > 3 {
            paths.push(entry[3..].to_string());
        }
        // `-z` 下改名 / 复制的条目后面紧跟一个原路径字段
        if entry.starts_with('R') || entry.starts_with('C') {
            it.next();
        }
    }
    Ok(paths)
}

/// D1 + D2：`git diff --name-status --no-renames base deliver`，再核对白名单（计划 B9）。
pub(crate) fn changed_files(
    repo: &str,
    cfg: &SvnConfig,
    base: &str,
    deliver: &str,
) -> Result<Vec<DeliverChange>> {
    let out = run_git(
        repo,
        &["diff", "--name-status", "--no-renames", "-z", base, deliver],
    )?;
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    let mut changes = Vec::new();
    let mut odd = Vec::new();
    while let Some(st) = it.next() {
        let Some(path) = it.next() else { break };
        match st {
            "A" | "M" | "D" => changes.push(DeliverChange {
                status: st.to_string(),
                path: path.to_string(),
            }),
            other => odd.push(format!("{other} {path}")),
        }
    }
    if !odd.is_empty() {
        return Err(GitError::new(
            "sync_out_unsupported_change",
            format!(
                "下面这些变化不是普通的新增 / 修改 / 删除（多半是文件和符号链接互换）：\n{}\n\
                 SVN 侧没有对应的操作，请用 TortoiseSVN 处理它们。",
                list_paths(&odd)
            ),
        ));
    }
    let roots = normalize_roots(&cfg.whitelist);
    let outside: Vec<String> = changes
        .iter()
        .filter(|c| !is_w_file(&c.path, &roots, &cfg.extensions))
        .map(|c| c.path.clone())
        .collect();
    if !outside.is_empty() {
        return Err(GitError::new(
            "sync_out_outside_whitelist",
            format!(
                "下面这些改动不在白名单里，本应用不替你交付它们：\n{}\n它们多半是用 `git add -f` \
                 加进来的。交付之后分支会接到服务器的新内容上，这些改动会从分支上消失——所以这里\
                 停下，不悄悄跳过。请先把它们移出这个分支的提交，或用 TortoiseSVN 提交。",
                list_paths(&outside)
            ),
        ));
    }
    Ok(changes)
}

/// 白名单目录里 git 管不到、有本地修改的文件（W-path 减 W-file，§6.3.2）。有 svn 冲突态就拒绝。
fn outside_changes(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    let mut out = Vec::new();
    let mut conflicted = Vec::new();
    for t in svn_targets(repo, &cfg.whitelist)? {
        let xml = run_svn(
            repo,
            &[
                "status",
                "--xml",
                "--ignore-externals",
                "--non-interactive",
                "--",
                &peg_safe(&t),
            ],
        )?;
        for e in parse_update_status(&xml)?.entries {
            if e.is_conflicted() {
                conflicted.push(e.path.clone());
                continue;
            }
            if is_w_file(&e.path, &roots, &cfg.extensions) {
                continue; // git 管着，上面的 git_dirty 已经要求它干净
            }
            let local = matches!(e.item.as_str(), "modified" | "missing" | "replaced")
                || e.props == "modified";
            if local {
                out.push(e.path.clone());
            }
        }
    }
    if !conflicted.is_empty() {
        return Err(GitError::new(
            "sync_out_svn_conflicted",
            format!(
                "白名单里有 SVN 冲突还没处理：\n{}\n请先用 TortoiseSVN 解决（Resolve）后再交付。",
                list_paths(&conflicted)
            ),
        ));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// 基线 trailer 里的 Repository Root（spec §6.4.1：trailer 的 `<url>` 就是它）。
fn repos_root(repo: &str, baseline: &str) -> Result<String> {
    let msg = run_git(repo, &["log", "-1", "--format=%B", baseline])?;
    parse_trailer(&msg)
        .map(|t| t.url)
        .ok_or_else(|| GitError::new("svn_baseline_trailer", "基线 commit 上读不到 git-svn-id。"))
}

/// 服务器 HEAD revision。`<root>@HEAD` 的 peg 写法顺带避开 URL 里含 `@` 的歧义。
pub(crate) fn remote_head_rev(repo: &str, repos_root: &str) -> Result<u64> {
    let target = format!("{repos_root}@HEAD");
    let xml = run_svn(repo, &["info", "--xml", "--non-interactive", &target]).map_err(|e| {
        GitError::new(
            "sync_out_server_unreachable",
            format!(
                "问服务器当前 revision 失败了，什么都没有改动。svn 原话：{}",
                e.message.trim()
            ),
        )
    })?;
    let e = parse_info(&xml)?
        .into_iter()
        .next()
        .ok_or_else(|| GitError::new("svn_xml", "svn info 没有返回任何 entry"))?;
    u64::try_from(e.revision).map_err(|_| {
        GitError::new(
            "svn_xml",
            format!("svn info 给了一个负的 revision：{}", e.revision),
        )
    })
}

fn nothing_to_deliver(local_only: usize) -> GitError {
    let tail = if local_only > 0 {
        format!(
            "这个分支上有 {local_only} 个本地补丁（标了「仅本地，不交付」），它们被排除了——\
             如果其中有本该交付的，去掉那个标记后再来。"
        )
    } else {
        "分支和基线的内容完全一样。".to_string()
    };
    GitError::new(
        "sync_out_nothing_to_deliver",
        format!("没有可交付的改动。{tail}"),
    )
}
