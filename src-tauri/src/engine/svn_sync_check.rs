//! `sync_in` 的前置检查（spec §7.2 第 1–6 条）。**全部是纯读。**
//!
//! 第 7 条（拿锁、锁内复核）属于执行，见 `svn_sync_in::run_sync_in`：它在锁里
//! 再调一次 [`check_sync_in`]，拿结果与预览时给用户看过的比。
//!
//! # 顺序
//!
//! 本地的、便宜的检查在前，走网络的 `svn status -u` 放在最后：断网时用户先看到
//! 的应当是「你在 detached HEAD 上」这种能自己修的问题，而不是一条网络错误。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_baseline::{git_dir, TempIndex};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{read_config, BASELINE_BRANCH};
use crate::engine::svn_externals::detect_pinned;
use crate::engine::svn_include_check::{is_w_file, normalize_roots, verify_include_set};
use crate::engine::svn_sync::{check_sync, SyncVerdict};
use crate::engine::svn_xml::parse_update_status;
use crate::engine::{svn_journal, svn_lock};
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};
use std::collections::BTreeMap;

/// 预览：给确认框看的全部事实。执行时原样带回来（`SyncInExpect`）做锁内复核。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInPreview {
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    pub baseline_rev: u64,
    /// 这次会更新到的 revision（`svn status -u` 的 `<against>`）。
    pub target_rev: u64,
    /// 计划 A16：工作副本已被外部**整体** update 到的 revision（比基线新）。
    /// `Some` 时执行会先把基线跟上它，再走 S0–S11；确认框必须说出这件事。
    pub catch_up_rev: Option<u64>,
    /// §7.2 第 5 条：git 不管、本地改过、这次 update 又会碰到的文件。
    /// **非空时必须由用户确认**，确认只授权 update，不授权任何 revert。
    pub outside_changes: Vec<String>,
}

/// 给界面用的入口：锁被拿着、或有未收尾的 journal 时直接拒绝。
pub fn preview_sync_in(repo_root: &str) -> Result<SyncInPreview> {
    svn_lock::assert_not_syncing(repo_root)?;
    if svn_journal::read(repo_root)?.is_some() {
        return Err(unfinished_error());
    }
    check_sync_in(repo_root)
}

/// 预览与执行两处都要说这句话，收在一起免得两份文案说出不同的事。
pub(crate) fn unfinished_error() -> GitError {
    GitError::new(
        "sync_in_unfinished",
        "上一次同步还没有收尾。请先在顶部横幅上选择「继续」或「放弃」，\
         再发起新的更新——两次同步的记录不能叠在一起。",
    )
}

/// 第 1–6 条。**不查锁**：持锁者在锁内复核时调它，查锁会把自己挡住。
pub(crate) fn check_sync_in(repo_root: &str) -> Result<SyncInPreview> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库。",
        ));
    }
    let cfg = read_config(repo_root)?;

    check_git_idle(repo_root)?; // 第 1 条
    let orig_branch = attached_branch(repo_root)?; // 第 3 条
    let orig_head = rev_parse(repo_root, "HEAD")?;
    let baseline_oid = rev_parse(repo_root, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    assert_linear(repo_root, &baseline_oid, &orig_head)?; // 第 6 条前半

    if !detect_pinned(repo_root, &cfg.whitelist)?.is_empty() {
        return Err(GitError::new(
            "sync_in_pinned_unsupported",
            "白名单所在的 external 被 `-r` 钉住了（pinned 模式）。这种形态需要改用\
             「项目根 --ignore-externals + 逐个 external 显式 update」的两步更新，\
             本版本还没有实现，所以不在这里冒险。请先用 TortoiseSVN 更新，\
             再用「重建基线」把基线跟上。",
        ));
    }

    // 第 2 条（完整 nodeRevs）。「整体前进」不拒绝，交给执行时接管（计划 A16）：
    // 在 TortoiseSVN 里对项目根点 update 是每天早上的日常，不是异常
    let status = check_sync(repo_root)?;
    let catch_up_rev = match status.verdict {
        SyncVerdict::InSync => None,
        SyncVerdict::Diverged {
            node_rev,
            baseline_rev,
        } if node_rev > baseline_rev => Some(node_rev),
        _ => {
            return Err(GitError::new(
                "sync_in_out_of_sync",
                format!(
                    "基线已经失同步（白名单 revision {:?}，基线记的是 r{}）。\
                     工作副本处于多个 revision，或被退回到了基线之前——这两种状态\
                     「更新」不能自动接管，否则会把外部的改动当成服务器的内容录进基线。\
                     请先处理顶部的失同步横幅。",
                    status.node_revs, status.baseline_rev
                ),
            ))
        }
    };

    // 第 6 条后半：当前磁盘的纳入集合预检（§6.3.1）。放在 update 之前，是为了
    // 让「工作树里一个 `.gitignore` 挡住了 W-file」这种问题在动手前暴露——
    // 否则要到 S7 才炸，那时 svn update 已经做完，用户卡在 svn/base 上。
    precheck_include_set(repo_root, &cfg.whitelist, &cfg.extensions)?;

    // 第 4、5 条：走网络，放最后
    let xml = run_svn(repo_root, &["status", "-u", "--xml"]).map_err(|e| {
        GitError::new(
            "sync_in_server_unreachable",
            format!(
                "问服务器「这次会更新哪些文件」失败了，没有做任何修改。svn 原话：{}",
                e.message
            ),
        )
    })?;
    let st = parse_update_status(&xml)?;
    let roots = normalize_roots(&cfg.whitelist);
    let mut conflicted = Vec::new();
    let mut outside = Vec::new();
    for e in &st.entries {
        if e.is_conflicted() {
            conflicted.push(e.path.clone());
            continue;
        }
        if is_w_file(&e.path, &roots, &cfg.extensions) {
            continue; // git 管着，S1 会压进 stash
        }
        let local =
            matches!(e.item.as_str(), "modified" | "missing" | "replaced") || e.props == "modified";
        if local && e.is_incoming() {
            outside.push(e.path.clone());
        }
    }
    if !conflicted.is_empty() {
        return Err(GitError::new(
            "sync_in_svn_conflicted",
            format!(
                "工作副本里有 SVN 冲突还没处理，在它上面再 update 只会让情况更乱：\n{}\n\
                 请先用 TortoiseSVN 解决（Resolve）后再更新。",
                list_paths(&conflicted)
            ),
        ));
    }
    let target_rev = st
        .against
        .ok_or_else(|| GitError::new("svn_xml", "svn status -u 没有给出 <against revision>"))?;

    Ok(SyncInPreview {
        orig_branch,
        orig_head,
        baseline_oid,
        baseline_rev: status.baseline_rev,
        target_rev,
        catch_up_rev,
        outside_changes: outside,
    })
}

/// §7.3.2 的 merge 门槛：`target..tip` 必须是线性的。
///
/// **查询失败不能当成空集合**——那等于把「没检查」报成「检查通过」。
pub fn assert_linear(repo_root: &str, target: &str, tip: &str) -> Result<()> {
    // 空 OID 会让区间退化成 `..tip`（= `HEAD..tip`），查的就不是我们以为的那段了
    for oid in [target, tip] {
        if oid.len() != 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GitError::new(
                "sync_in_merge_check_failed",
                format!("merge 门槛拿到的不是一个完整 OID：`{oid}`"),
            ));
        }
    }
    let range = format!("{target}..{tip}");
    let (code, out) = run_git_code(repo_root, &["rev-list", "--merges", &range])?;
    if code != 0 {
        return Err(GitError::new(
            "sync_in_merge_check_failed",
            format!(
                "检查待重放区间 {range} 里有没有 merge 失败了（rev-list 退出 {code}）。\
                 查不成不等于没有 merge，所以这里停下。"
            ),
        ));
    }
    let merges: Vec<&str> = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if !merges.is_empty() {
        return Err(GitError::new(
            "sync_in_merge_in_range",
            format!(
                "你的分支上、基线之后有 merge commit：\n{}\n\
                 更新要把这些提交重放到新基线上，而普通 rebase 会跳过 merge——\
                 只存在于 merge 结果里的改动可能无声地消失。本版本不自动重放 merge 历史。\
                 请保留原分支，自己把历史整理成线性的（比如在新分支上逐个 cherry-pick），\
                 再回来更新。",
                merges.join("\n")
            ),
        ));
    }
    Ok(())
}

/// 第 1 条：git 不在任何未完成的操作里，索引也没有未合并条目。
fn check_git_idle(repo_root: &str) -> Result<()> {
    let dir = git_dir(repo_root)?;
    let busy = [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ];
    for (marker, what) in busy {
        if dir.join(marker).exists() {
            return Err(GitError::new(
                "sync_in_git_busy",
                format!("git 正处在一次没做完的 {what} 里。请先完成或中止它，再更新。"),
            ));
        }
    }
    if !unmerged_paths(repo_root)?.is_empty() {
        return Err(GitError::new(
            "sync_in_git_busy",
            "暂存区里有未解决的冲突。请先解决（或 `git restore --staged`）再更新。",
        ));
    }
    Ok(())
}

/// 第 3 条：HEAD 必须附着在分支上，且不是基线分支。返回分支名。
fn attached_branch(repo_root: &str) -> Result<String> {
    let Some(branch) = current_branch(repo_root)? else {
        let at = rev_parse(repo_root, "HEAD").unwrap_or_default();
        return Err(GitError::new(
            "sync_in_detached_head",
            format!(
                "HEAD 没有附着在分支上（停在 {}）。更新要把一个分支重放到新基线上，\
                 detached HEAD 没有分支可搬。请 `git switch -c <新分支名>` 或切回原来的分支。",
                &at[..at.len().min(10)]
            ),
        ));
    };
    if branch == BASELINE_BRANCH {
        return Err(GitError::new(
            "sync_in_on_baseline",
            format!(
                "当前在基线分支 `{BASELINE_BRANCH}` 上。基线由本应用维护，请先切回你的工作分支再更新。"
            ),
        ));
    }
    Ok(branch)
}

/// 当前分支名；detached 时 `None`。
///
/// 不用 `--short`（PR #49 review #1，实测）：有同名 tag 时它为了消歧义给出
/// `heads/work`，拿这个名字去 `git checkout` 会退出 0 却让 HEAD detached，
/// 拼成 `refs/heads/heads/work` 也对不上 rebase 的 head-name。取完整 ref 自己剥前缀。
pub(crate) fn current_branch(repo_root: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(repo_root, &["symbolic-ref", "--quiet", "HEAD"])?;
    Ok(match out.trim().strip_prefix("refs/heads/") {
        Some(name) if code == 0 && !name.is_empty() => Some(name.to_string()),
        _ => None,
    })
}

/// `git rev-parse --verify <rev>` 的完整 OID。
pub(crate) fn rev_parse(repo_root: &str, rev: &str) -> Result<String> {
    Ok(run_git(repo_root, &["rev-parse", "--verify", rev])?
        .trim()
        .to_string())
}

/// 索引里未合并的路径（冲突态）。
pub(crate) fn unmerged_paths(repo_root: &str) -> Result<Vec<String>> {
    let out = run_git(repo_root, &["diff", "--name-only", "--diff-filter=U", "-z"])?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// 当前磁盘上的纳入集合预检：期望集合（遍历磁盘）与 `git add -A` 实际收进来的相等。
fn precheck_include_set(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
) -> Result<()> {
    let idx = TempIndex::empty(repo_root)?;
    idx.add_all(repo_root, whitelist)?;
    // 空的 fixes：这里比的就是「磁盘现在的样子」，不是服务器的
    verify_include_set(repo_root, &idx, whitelist, extensions, &BTreeMap::new())
}

/// 路径清单给人看：最多 20 条，多了说还有几条。`svn_sync_in` 也用它。
pub(crate) fn list_paths(paths: &[String]) -> String {
    let mut s: Vec<String> = paths.iter().take(20).map(|p| format!("  - {p}")).collect();
    if paths.len() > 20 {
        s.push(format!("  …还有 {} 条", paths.len() - 20));
    }
    s.join("\n")
}
