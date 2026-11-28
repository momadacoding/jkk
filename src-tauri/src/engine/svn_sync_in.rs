//! `sync_in` 编排（spec §7.3 S0–S11）、冲突出口，以及中断后的继续 / 放弃（§7.6、§13）。
//!
//! # 为什么不走 `trait Vcs`
//!
//! `SvnGitBackend` 的写方法都挂着 `assert_not_syncing`，而本流程全程持锁——走那一层
//! 会被自己的锁挡回来（见 `svn_lock::assert_not_syncing` 的文档）。这里一律直接调
//! `run_git` / `run_svn_code`。
//!
//! # 锁
//!
//! 在入口拿、函数返回时由 Drop 释放。**冲突出口也是一次返回**：用户要编辑、暂存
//! 文件来解决冲突，锁必须放开（spec §7.4）。journal 留着，继续 / 放弃时重新拿锁。
//!
//! # watcher
//!
//! 本模块不碰。挂起与恢复要跨过整次调用，由命令层用 `WatchSuspension` 包住，
//! 结束后再补发一次 `repo-changed`（`commands/svn_sync.rs`）。

use crate::engine::file_lock::{describe, find_blockers};
use crate::engine::git_cli::{
    run_git, run_git_bytes, run_git_code, run_git_env, run_git_paths, run_git_stdin,
};
use crate::engine::svn_baseline::{
    create_baseline_commit, git_dir, move_baseline_ref, read_baseline, refresh_index_stat,
    swap_index_to_tree, TempIndex,
};
use crate::engine::svn_basemap::BaseMap;
use crate::engine::svn_changelist;
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_dirty::read_dirty;
use crate::engine::svn_follow_audit;
use crate::engine::svn_follow_mark::renew_mark_after_follow;
use crate::engine::svn_include_check::normalize_roots;
use crate::engine::svn_init::{rebuild_baseline, record_mixed, record_server_tree_into};
use crate::engine::svn_journal::{self, Flow, Journal, KeptFile, Phase};
use crate::engine::svn_lock;
use crate::engine::svn_preflight::{check_targets, read_node_rev, run_preflight, Preflight};
use crate::engine::svn_scope::WScope;
use crate::engine::svn_sync_check::{
    assert_linear, check_sync_in, current_branch, list_paths, rev_parse, unfinished_error,
    unmerged_paths, SyncInPreview,
};
use crate::engine::svn_sync_stash::{self as wip, RestoreOutcome};
use crate::engine::svn_util::{now_secs, pinned_rebase};
use crate::engine::svn_wc::{enumerate_wcs, peg_safe};
use crate::engine::svn_xml::{parse_status_entries, parse_update_status, StatusEntry};
use crate::error::{GitError, Result};
use std::path::Path;

/// 执行时带回来的预览事实（§7.2 第 7 条：锁内复核要与用户看到的一致）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInExpect {
    /// 用户在确认框里看到的分支。只比 `orig_head` 不够：外部切到指向同一提交的
    /// 另一条分支时 OID 不变，重放的却是另一条（PR #49 review 第三轮 #6）。
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    /// 用户在确认框里看过并同意的「白名单外会被碰到的文件」。
    pub confirmed_outside: Vec<String>,
}

/// 一次 `sync_in` / 继续 / 放弃的结局。**冲突不是错误**，是正常出口（spec §7.4）。
///
/// `rename_all_fields` 不能省：只有 `rename_all` 时变体内部字段原样发出
/// （`new_rev`），前端读 `newRev` 拿到 undefined——`svn_sync.rs` 的
/// `SyncVerdict` 上踩过同一个坑。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SyncInOutcome {
    Done {
        new_rev: u64,
    },
    Aborted,
    RebaseConflict {
        files: Vec<String>,
    },
    StashConflict {
        files: Vec<String>,
        stash_oid: String,
    },
    StashBlocked {
        collisions: Vec<String>,
        stash_oid: String,
    },
    /// 在 `applying-stash` 阶段放弃：未提交的改动**没有**（完整）放回——撞名时一个字节
    /// 都没放，冲突时文件里留着冲突标记。备份原样留在 stash 列表里。
    ///
    /// 不能报成 `Aborted`：界面对它说的是「已回到更新前的分支，未提交的改动已放回」，
    /// 用户信了就可能把 stash 列表里那条备份当垃圾清掉（2c 独立 review I1）。
    Released {
        stash_oid: String,
        /// 停下的那次放回属于回退 / 放弃：分支在更新前的位置。为假时分支已在新基线上
        /// （PR #49 review #3）。
        rolled_back: bool,
    },
    /// 放回改动时停下过（冲突，或崩在 apply 之后），用户处理完点了「继续」：流程收尾，
    /// 但**原始备份留在 stash 列表里、不 drop**。
    ///
    /// 此刻磁盘上的 WIP 经过了用户之手（或外部工具），我们证明不了它还完整——实测
    /// 冲突之后外部 `reset --hard` 会让冲突与 WIP 一起消失，而所有复核照样通过。
    /// stash 里那份可能是唯一的备份，删不删交给用户（PR #49 review 第四轮 P1-2）。
    Settled {
        stash_oid: String,
        /// 为真时收尾的是一次回退 / 放弃：分支在更新前的位置。
        rolled_back: bool,
    },
    /// 收尾了，改动内容都放回来了，但「已暂存」的状态没保住（分析 §6.11.1）：手动跟进带 `--index`
    /// 放不回去、退回了普通 apply；或者更新（本来就不带 `--index`）时有暂存版本与磁盘不同。
    /// 不能报成 `Done` / `Aborted`：用户拿暂存区当存档，要知道存档没了。冲突的结局不用这一条：
    /// `StashConflict` 本身就意味着暂存没保住。
    ///
    /// **备份不删、钉子不拆**（Review 3 第 1 条）：`files` 的暂存版本此后只在 stash 里。
    StageLost {
        /// `None`：收尾的是一次放弃，分支回到了流程之前的位置。
        new_rev: Option<u64>,
        /// 留着的 stash。`None` 只在崩溃恢复时出现：stash 已经不在了（被外部删掉）。
        stash_oid: Option<String>,
        /// 暂存版本与磁盘不同的路径（[`wip::staged_only_paths`]）。手动跟进时可以为空：
        /// 暂存的内容磁盘上都有，丢的只是标记。
        files: Vec<String>,
    },
    /// 手动跟进收尾了，但收尾核对（[`svn_follow_audit`]，事故分析 6.9）发现重放结果与跟进前
    /// 不一致，或者核对没跑成。**不能报成 `Done`**：2026-11-05 磁盘上的文件被静默合错，界面却报「完成」，
    /// 正确内容唯一的副本（stash 备份）也随之被删。
    ///
    /// 分支已经在新基线上，什么都不回滚；备份不删（拆掉钉子，同 `Settled`），交给用户核对。
    FollowDrift {
        new_rev: u64,
        /// 跟进前后磁盘内容不同的文件。
        changed: Vec<String>,
        /// 重放后的分支里有、新基线和磁盘上都没有的文件。
        resurrected: Vec<String>,
        /// 代表「跟进前的磁盘」的提交：有未提交改动时是 stash，否则是原分支 HEAD。
        /// 给用户取回原内容用（`git restore --source=<before> --worktree -- <文件>`，不动暂存区）。
        before: String,
        /// 留在 stash 列表里的备份。`None`：这次没有未提交的改动。
        stash_oid: Option<String>,
        /// 暂存状态也没保住时，暂存版本与磁盘不同的路径（同 `StageLost.files`，可以为空）。
        /// `None`：暂存保住了。
        stage_lost: Option<Vec<String>>,
        /// 核对没跑成时的原因（这时 `changed` / `resurrected` 为空、备份照样留着）。流程不因此
        /// 停住：没有备份要保护时停住就再也走不完（独立 review）。
        audit_error: Option<String>,
    },
}

/// 一次完整的更新。
pub fn run_sync_in(repo: &str, expect: &SyncInExpect) -> Result<SyncInOutcome> {
    let opid = new_opid();
    let _lock = svn_lock::acquire(repo, &opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    // §7.2 第 7 条：锁内复核。预览与确认之间可能过了几分钟，什么都可能变。
    // **必须在接管之前**：接管会移动基线指针，复核不过就一个字节都不能动
    let mut p = check_sync_in(repo)?;
    verify_expect(&p, expect)?;
    if p.catch_up_rev.is_some() {
        p = catch_up(repo, &p, expect)?;
    }
    let cfg = read_config(repo)?;

    // S0：任何修改之前落盘；origBranch / origHead 此刻就写（§7.3.1）
    let mut j = Journal {
        opid,
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: now_secs(),
        orig_branch: Some(p.orig_branch),
        orig_head: Some(p.orig_head),
        baseline_rev_before: Some(p.baseline_rev),
        baseline_oid_before: Some(p.baseline_oid),
        ..Default::default()
    };
    svn_journal::write(repo, &j)?;

    // S1 起文件会暂时等于 SVN 原版，IDE 这时刷新会摘掉 changelist：先记下，收尾后补挂（`svn_changelist`）
    svn_changelist::record(repo, &cfg.whitelist, &j.opid);

    // S1（S2 的 origBranch 已在 S0 落盘，这里只推进 phase）
    wip::stash_wip(repo, &mut j, &cfg.whitelist)?;
    from_stashed(repo, &cfg, &mut j, true)
}

fn verify_expect(p: &SyncInPreview, e: &SyncInExpect) -> Result<()> {
    if p.orig_branch != e.orig_branch
        || p.orig_head != e.orig_head
        || p.baseline_oid != e.baseline_oid
    {
        return Err(GitError::new(
            "sync_in_state_changed",
            "从你确认到现在，分支或基线被改动过（可能是别的 git 工具）。\
             这次没有做任何修改，请重新点一次「更新」。",
        ));
    }
    let unseen: Vec<String> = p
        .outside_changes
        .iter()
        .filter(|f| !e.confirmed_outside.contains(f))
        .cloned()
        .collect();
    if !unseen.is_empty() {
        return Err(GitError::new(
            "sync_in_needs_reconfirm",
            format!(
                "从你确认到现在，又多了几个这次更新会碰到、而 git 管不到的文件：\n{}\n\
                 这次没有做任何修改，请重新点一次「更新」看一下完整清单。",
                list_paths(&unseen)
            ),
        ));
    }
    Ok(())
}

/// 计划 A16：SVN 侧有新内容——外部整体 update 到了比基线新的 rN，或者（计划 Task 5 起）
/// 只提交不更新、只更新了部分目录留下的混合版本。
///
/// 先用 Phase 1 的「重建基线」把 `svn/base` 录成各文件在服务器上的原版（`svn cat -r BASE`，
/// 不碰磁盘），再走正常的 S0–S11：S9 把分支重放上去、S10 放回 WIP 时，服务器那部分
/// 改动两边一致，三方合并自动抵消，git 这一侧重新分得清「服务器的」和「我的」
/// （2026-09-25 实测，见计划 Task 7A）。
///
/// # 为什么放在 S0 之前、不写 journal
///
/// `rebuild_baseline` 不碰 HEAD、真实索引和磁盘，只以旧 OID 为条件移动一个 ref（R4）。
/// 它要么没发生、要么完整发生，两种状态都自洽：崩在它之后，重开时基线已经跟上、
/// 没有 journal，下次预览看到的就是普通的 `InSync`。没有需要恢复的中间态，
/// 为它加 phase 只会多一种要处理的崩溃现场。
///
/// # 为什么接管之后还要再复核
///
/// 后面 S0 记的 `baselineOidBefore` 必须是**新**基线；同时确认重建期间分支没被动过。
/// 预览时给用户看过的「会碰到的白名单外文件」照常再比一次。
fn catch_up(repo: &str, before: &SyncInPreview, expect: &SyncInExpect) -> Result<SyncInPreview> {
    let report = rebuild_baseline(repo)?;
    let p = check_sync_in(repo)?;
    if report.old_baseline != before.baseline_oid
        || p.catch_up_rev.is_some()
        || p.orig_branch != before.orig_branch
    {
        return Err(GitError::new(
            "sync_in_state_changed",
            format!(
                "基线已经按服务器 r{} 的原版重录好了，但复核时发现仓库在这期间被别的操作改动过。\
                 你的文件和分支都还没有被收起或改动，请重新点一次「更新」。",
                report.node_rev
            ),
        ));
    }
    verify_expect(
        &p,
        &SyncInExpect {
            baseline_oid: p.baseline_oid.clone(),
            ..expect.clone()
        },
    )?;
    Ok(p)
}

/// S3 起。`fresh` 为真表示本进程一路从 S0 走到这里（Task 7 的回退据此选前提）。
pub(crate) fn from_stashed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    fresh: bool,
) -> Result<SyncInOutcome> {
    // S3。`leftOrig` 先落盘，再 checkout（见字段文档；PR #49 review #2）
    if !j.left_orig {
        j.left_orig = true;
        svn_journal::write(repo, j)?;
    }
    if current_branch(repo)?.as_deref() != Some(BASELINE_BRANCH) {
        if let Err(e) = checkout_verified(repo, cfg, BASELINE_BRANCH, false) {
            return on_checkout_failed(repo, cfg, j, e);
        }
    }
    // 从中断处接着走（「继续」）时，上次 update 可能被打断、留下写锁：没换成功的文件 wc.db 里已经
    // 记成新版，磁盘上还是旧内容，要 Clean up 跑完工作队列才写进去。不先拦下，S4 会把它们当成
    // 「基线记的内容不对」去回退（第三轮 review 第 5 条实测）
    if !fresh && wc_root_locked(repo)? {
        return Err(GitError::new(
            "sync_in_update_incomplete",
            "工作副本上还有上一次 svn update 中断留下的写锁，这次什么都没动，记录已保留。\
             请先在 TortoiseSVN 里对工作副本根目录跑一次 Clean up，再点顶部横幅上的「继续」。",
        ));
    }
    // S4：直接问 SVN——revision 比对抓不住「外部手工改了 base 文件」（§7.4）
    let dirty = dirty_w_files(repo, cfg)?;
    if !dirty.is_empty() {
        return on_stale_baseline(repo, cfg, j, fresh, &dirty);
    }
    // S5：`--accept postpone` 是硬要求（§7.4）。成败**不看退出码**（计划 A4）
    let up = run_svn_code(repo, &["update", "--accept", "postpone"])?;
    if up.code != 0 {
        return on_update_failed(repo, cfg, j, &up.stderr);
    }
    let conflicts = svn_conflicts(repo)?;
    if !conflicts.is_empty() {
        return on_update_conflict(repo, cfg, j, &conflicts);
    }
    // S6–S7
    capture_and_publish(repo, cfg, j)?;
    from_baseline_ready(repo, cfg, j)
}

// —— S3 / S4 / S5 的失败出口：回到原分支并取回 WIP（spec §13 前五行）——

fn on_checkout_failed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    e: GitError,
) -> Result<SyncInOutcome> {
    // S3 之前 S5 必然没跑：磁盘上被跟踪的内容全是 git 自己写的（A12）
    rolled_back(repo, cfg, j, Disk::GitOnly, e)
}

fn on_stale_baseline(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    fresh: bool,
    dirty: &[String],
) -> Result<SyncInOutcome> {
    // 本进程一路走到这里才敢按 GitOnly；从崩溃里恢复时不知道 S5 跑没跑（A12）
    let disk = if fresh {
        Disk::GitOnly
    } else {
        Disk::MaybeUpdated
    };
    rolled_back(repo, cfg, j, disk, stale_error(dirty))
}

/// S5 失败。update 已经换掉了 git 管的文件时**不回退**（见 [`updated_by_svn`]），记录停在
/// `stashed`，「继续」会从 S4 起重跑 update。一个都没换时照旧回原分支。
fn on_update_failed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    stderr: &str,
) -> Result<SyncInOutcome> {
    let cause = update_failed_error(stderr);
    let updated = updated_or_keep(repo, cfg, j, &cause)?;
    if !updated.is_empty() {
        return Err(GitError::new(
            "sync_in_update_incomplete",
            format!(
                "svn update 没跑完就失败了。svn 原话：{}\n\n{}",
                stderr.trim(),
                kept_for_continue(
                    &updated,
                    j,
                    "请先在 TortoiseSVN 里对工作副本根目录跑一次 Clean up（清掉中断留下的锁），\
                     等网络、VPN 正常后，点顶部横幅上的「继续」把这次更新跑完。\
                     先用 TortoiseSVN 的 Update 补齐、再点「继续」也可以。",
                )
            ),
        ));
    }
    rolled_back(repo, cfg, j, Disk::MaybeUpdated, cause)
}

/// [`updated_by_svn`]，失败时**原因在前**（与 [`rolled_back`] 同一条规矩）：核对不成就不回退，
/// 记录留着，用户先要知道 update 为什么没成。
fn updated_or_keep(
    repo: &str,
    cfg: &SvnConfig,
    j: &Journal,
    cause: &GitError,
) -> Result<Vec<String>> {
    updated_by_svn(repo, cfg, j).map_err(|e| {
        // 不用原因自己的错误码：`sync_in_update_failed` 约定的是「已回退」，这里记录是留着的
        GitError::new(
            "sync_in_update_incomplete",
            format!(
                "{}\n\n另外：没能核对 svn update 换掉了哪些 git 管的文件（{}），所以没有回退，\
                 记录已保留。处理好之后在顶部横幅上点「继续」或「放弃」。",
                cause.message, e.message
            ),
        )
    })
}

fn on_update_conflict(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    conflicts: &[String],
) -> Result<SyncInOutcome> {
    // update 跑完了才会走到这里，git 管的文件多半已经换成新版：回退同样会把它们写回旧内容
    let cause = update_conflict_error(conflicts);
    let updated = updated_or_keep(repo, cfg, j, &cause)?;
    if !updated.is_empty() {
        return Err(GitError::new(
            "sync_in_update_conflict",
            format!(
                "svn update 产生了冲突：\n{}\n这些文件不归 git 管（或者是目录级的树冲突），\
                 本应用没法替你解决。\n\n{}",
                list_paths(conflicts),
                kept_for_continue(
                    &updated,
                    j,
                    "请先用 TortoiseSVN 解决（Resolve）上面的冲突，再点顶部横幅上的「继续」。",
                )
            ),
        ));
    }
    rolled_back(repo, cfg, j, Disk::MaybeUpdated, cause)
}

/// 回退并把结果接在原因后面。**原因永远在前**：用户先要知道为什么没更新成。
fn rolled_back(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    disk: Disk,
    cause: GitError,
) -> Result<SyncInOutcome> {
    let orig = j.orig_branch.clone().unwrap_or_default();
    let tail = match back_to_orig(repo, cfg, j, disk) {
        Ok(RestoreStep::Finished) => {
            format!("已回到分支 `{orig}`，未提交的改动已放回；基线没有变。")
        }
        // 没有横幅可看（记录已删）：暂存的那份在哪要在这里说清楚
        Ok(RestoreStep::StageKept { oid, files }) => format!(
            "已回到分支 `{orig}`，未提交的改动已放回；基线没有变。但「已暂存」的状态没保住{}\
             原始备份留在 stash 列表里（{oid}），需要时用 `git show {oid}^2:<文件>` 查看暂存的版本。",
            if files.is_empty() {
                "。".to_string()
            } else {
                format!("，下面这些文件暂存的版本与磁盘不同：\n{}\n", list_paths(&files))
            }
        ),
        Ok(_) => "已回到原分支，但放回未提交的改动时遇到问题——详见顶部横幅。".to_string(),
        Err(re) => format!(
            "另外：回到原分支也没成功（{}）。记录已保留，处理好之后在顶部横幅上点「放弃」。",
            re.message
        ),
    };
    Err(GitError::new(
        &cause.code,
        format!("{}\n\n{tail}", cause.message),
    ))
}

fn stale_error(dirty: &[String]) -> GitError {
    GitError::new(
        "sync_in_baseline_stale",
        format!(
            "这次没有更新：切到基线之后，下面这些文件在 SVN 看来与基线记的内容不一样：\n{}\n\
             基线没有被改动。先在工具栏「SVN ▾」里点「体检基线…」，看是哪里对不上；\
             确认工作副本是对的，再用「重建基线」按服务器原版重录。",
            list_paths(dirty)
        ),
    )
}

fn update_failed_error(stderr: &str) -> GitError {
    GitError::new(
        "sync_in_update_failed",
        format!(
            "svn update 失败了（常见原因：VPN 没连、凭据过期）。svn 原话：{}",
            stderr.trim()
        ),
    )
}

fn update_conflict_error(conflicts: &[String]) -> GitError {
    GitError::new(
        "sync_in_update_conflict",
        format!(
            "svn update 产生了冲突：\n{}\n这些文件不归 git 管（或者是目录级的树冲突），\
             本应用没法替你解决，请用 TortoiseSVN 处理。工作副本已经更新到新 revision，\
             但基线没有推进——处理完冲突后，用「重建基线」把基线跟上。",
            list_paths(conflicts)
        ),
    )
}

/// HEAD 在基线分支上时，`checkout -f` 回原分支会改写的、update 换掉了的 W-file：
///
/// - 与基线 commit 对不上的被跟踪文件（内容变了，或被删了——`-f` 会把它写回来，SVN 却记着已删除）；
/// - 服务器新增、基线里没有、而原分支恰好也有同名文件的路径（`-f` 会用分支的版本盖掉它，终审
///   review 第 1 条实测）。
///
/// 服务器新增、原分支也没有的文件不算：`-f` 不碰它们，回原分支后原样留在磁盘上，与 SVN 记的一致
/// （同名 WIP 被它挡住时停在 `applying-stash`，见对应测试）。
///
/// # 为什么这就是「`svn update` 换掉了的文件」
///
/// S3 切到基线后磁盘 == 基线树，WIP 连同未跟踪文件（`stash -u`）都在 stash 里；S4 又确认过
/// W-file 在 SVN 看来全部干净。之后能改这些文件的只有 S5 的 update。崩溃后重开、不知道 S5 跑没跑时
/// 也成立：中断期间有人手改过的文件 SVN 看来不干净，调用方先用 [`dirty_w_files`] 拦下。
///
/// # 为什么它们在就不能回原分支（2026-11-27 现场）
///
/// 这些文件的磁盘内容与 SVN 记的 revision 一起前进了，而 git 的基线还停在旧版。`checkout -f`
/// 回原分支会把它们写回旧内容，SVN 却仍记着新版本：同事的改动于是成了「你的本地修改」。之后的
/// 跟进、重建基线都把它当未提交改动保留（SVN 看来就是本地修改，没有别的证据能分辨），交付时撤掉
/// 同事的提交。以前的前提「覆盖掉也能从服务器取回」只在有人去取回时成立，而没有任何步骤去取回。
///
/// 基线已经发布（`baseline-ready` 之后）时 HEAD 就是新基线、磁盘与它一致，这里返回空：回原分支
/// 留下的是「分支落后基线」，git 自己记得，下次更新会重放上去。
fn updated_by_svn(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<Vec<String>> {
    if current_branch(repo)?.as_deref() != Some(BASELINE_BRANCH) {
        return Ok(Vec::new());
    }
    let mut out = tracked_changes(repo, cfg)?;
    out.extend(added_over_branch_files(repo, cfg, j)?);
    Ok(out)
}

/// 白名单内的未跟踪文件（此刻只可能是 update 新增的，WIP 的未跟踪文件都在 stash 里）中，
/// 原分支也跟踪着的那些。
fn added_over_branch_files(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(Vec::new()); // 空 pathspec 会变成「全部」（§12.2）
    }
    // 不带 `--exclude-standard`：原分支可能 `add -f` 跟踪着被 .gitignore 排除的路径，`-f` 一样会
    // 盖掉服务器新增的同名文件。多列出来的忽略文件下面按原分支的树筛掉
    let mut args: Vec<&str> = vec!["ls-files", "-z", "--others", "--"];
    args.extend(roots.iter().map(String::as_str));
    let others = run_git(repo, &args)?;
    let others: Vec<&str> = others.split('\0').filter(|p| !p.is_empty()).collect();
    if others.is_empty() {
        return Ok(Vec::new());
    }
    let orig_head = need(&j.orig_head, "origHead")?;
    let mut args: Vec<&str> = vec!["ls-tree", "-r", "-z", "--name-only", &orig_head, "--"];
    args.extend(roots.iter().map(String::as_str));
    let on_branch = run_git(repo, &args)?;
    let on_branch: std::collections::HashSet<&str> =
        on_branch.split('\0').filter(|p| !p.is_empty()).collect();
    Ok(others
        .into_iter()
        .filter(|p| on_branch.contains(p))
        .map(str::to_string)
        .collect())
}

/// 不回退、留给「继续」时接在原因后面的说明。`next` 是用户下一步要做的事。
fn kept_for_continue(updated: &[String], j: &Journal, next: &str) -> String {
    // 没有 WIP 时（计划 A7）stash 里什么都没有，不能让用户去 stash 列表里找
    let wip = if j.stash_oid.is_some() {
        "未提交的改动还在 stash 里，"
    } else {
        ""
    };
    format!(
        "svn update 已经把下面这些 git 管的文件换成了服务器上的新版本：\n{}\n\
         这时直接回到原分支，会把它们写回旧内容，而 SVN 记的已经是新版本——同事的改动会变成你的\
         本地修改，交付时被撤掉。所以这次没有回退：分支、基线都没动，{wip}记录已保留。\n{next}",
        list_paths(updated)
    )
}

/// 「放弃」时 update 已经换掉的文件里，原分支的提交或 WIP 也改过的那些（见 [`back_to_orig`]）。
///
/// 这些文件回原分支后要的是「你的版本 + 服务器的改动」，得做三方合并，只有「继续」做得了。
/// 其余的文件在原分支上与旧基线一字不差，保留服务器内容就只多出「服务器的改动」这一份差异。
///
/// 改过、但改成了与服务器一模一样（两边都删了、改成同样的字节）的不算碰过：回原分支写下的就是
/// 服务器内容，没有要合并的（独立 review：否则 update 一直跑不完时，这类文件也会让记录永远清不掉）。
fn touched_by_branch_or_wip(
    repo: &str,
    cfg: &SvnConfig,
    j: &Journal,
    updated: &[String],
) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(updated.to_vec()); // 空 pathspec 会变成「全部」（§12.2），宁可全算碰过
    }
    let base = need(&j.baseline_oid_before, "baselineOidBefore")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    // (起点, 终点)：终点上的版本就是回原分支、放回 WIP 之后这个路径会变成的样子
    let mut ranges: Vec<(String, String)> = vec![(base, orig_head)];
    if let Some(oid) = &j.stash_oid {
        // WIP 的工作区（`^1` → stash 自己）与暂存（`^1` → `^2`）
        ranges.push((format!("{oid}^1"), oid.clone()));
        ranges.push((format!("{oid}^1"), format!("{oid}^2")));
    }
    // 不用看 `stash -u` 的未跟踪部分（`^3`）：`updated` 里的路径要么在旧基线里被跟踪，要么在原分支
    // 上被跟踪（[`added_over_branch_files`]）。WIP 要在这样的路径上留一个未跟踪文件，原分支就得
    // 不跟踪它，即原分支相对旧基线删了它——第一段 diff 已经把它算进来了
    let mut changed: Vec<(String, std::collections::HashSet<String>)> = Vec::new();
    for (from, to) in &ranges {
        let mut args: Vec<&str> = vec!["diff", "--name-only", "-z", "--no-renames", from, to, "--"];
        args.extend(roots.iter().map(String::as_str));
        let out = run_git(repo, &args)?;
        let set = out
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect();
        changed.push((to.clone(), set));
    }
    let mut touched = Vec::new();
    for p in updated {
        for (to, set) in &changed {
            if set.contains(p) && !same_as_disk(repo, to, p)? {
                touched.push(p.clone());
                break;
            }
        }
    }
    Ok(touched)
}

/// `rev` 上的 `p` 与磁盘上此刻（服务器内容）的 `p` 是否一致：两边都没有，或内容相同。
fn same_as_disk(repo: &str, rev: &str, p: &str) -> Result<bool> {
    let (code, blob) = run_git_code(
        repo,
        &["rev-parse", "-q", "--verify", &format!("{rev}:{p}")],
    )?;
    let on_disk = Path::new(repo)
        .join(p)
        .symlink_metadata()
        .is_ok_and(|m| m.is_file());
    match (code == 0, on_disk) {
        (false, false) => Ok(true),
        (true, true) => {
            // `--path` 让磁盘内容走与 `git add` 相同的过滤器，才能与库里的 blob 比
            let disk = run_git(repo, &["hash-object", &format!("--path={p}"), "--", p])?;
            Ok(disk.trim() == blob.trim())
        }
        _ => Ok(false),
    }
}

/// 把 update 换掉的文件此刻的内容（服务器版本）原样存进对象库，返回要记进 journal 的清单。
///
/// `--no-filters`：存的是磁盘上的原始字节，写回时一个字节都不变，SVN 看来仍是干净的。
/// 走 `--stdin-paths` 一次存完，不把内容读进内存（大的美术文件一次几百 MB）。
fn save_server_content(repo: &str, updated: &[String]) -> Result<Vec<KeptFile>> {
    let mut kept = Vec::new();
    let mut present: Vec<&str> = Vec::new();
    for p in updated {
        match Path::new(repo).join(p).symlink_metadata() {
            Ok(m) if m.is_file() => present.push(p),
            Ok(_) => {
                return Err(GitError::new(
                    "sync_in_update_incomplete",
                    format!(
                        "{p} 在磁盘上不是普通文件（服务器可能把它换成了目录），没法保留它的内容，\
                         这次没法放弃。记录已保留，请点「继续」把更新跑完。"
                    ),
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => kept.push(KeptFile {
                path: p.clone(),
                blob: None,
            }),
            Err(e) => {
                return Err(GitError::new(
                    "sync_in_unsafe_to_return",
                    format!("读不了 {p}（{e}），没有回原分支，记录已保留。"),
                ))
            }
        }
    }
    if !present.is_empty() {
        let input = present.join("\n") + "\n";
        let out = run_git_stdin(
            repo,
            &["hash-object", "-w", "--no-filters", "--stdin-paths"],
            input.as_bytes(),
        )?;
        let oids: Vec<&str> = out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        if oids.len() != present.len() {
            return Err(GitError::new(
                "sync_in_unsafe_to_return",
                "没能把服务器版本的文件存下来，没有回原分支，记录已保留。",
            ));
        }
        for (p, oid) in present.iter().zip(oids) {
            kept.push(KeptFile {
                path: (*p).to_string(),
                blob: Some(oid.to_string()),
            });
        }
    }
    Ok(kept)
}

/// 把 [`save_server_content`] 存下的内容写回磁盘，可以重复执行。写不成的指名报出来，记录留着：
/// 关掉占用的程序后再点一次「放弃」会接着写。
fn put_back_server_content(repo: &str, kept: &[KeptFile]) -> Result<()> {
    let mut failed: Vec<String> = Vec::new();
    for k in kept {
        let path = Path::new(repo).join(&k.path);
        let ok = match &k.blob {
            Some(oid) => run_git_bytes(repo, &["cat-file", "blob", oid]).is_ok_and(|bytes| {
                path.parent()
                    .map_or(Ok(()), std::fs::create_dir_all)
                    .and_then(|()| std::fs::write(&path, bytes))
                    .is_ok()
            }),
            None => match std::fs::remove_file(&path) {
                Err(e) => e.kind() == std::io::ErrorKind::NotFound,
                Ok(()) => true,
            },
        };
        if !ok {
            failed.push(k.path.clone());
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    Err(GitError::new(
        "sync_in_unsafe_to_return",
        format!(
            "下面这些文件没能换回服务器上的版本（多半有程序占着它们）：\n{}\n\
             记录已保留。关掉占用的程序之后，再在顶部横幅上点一次「放弃」，会接着写回。",
            list_paths(&failed)
        ),
    ))
}

/// S6–S7：录新基线并发布（spec §7.3 S7、§7.6 录制三阶段）。
pub(crate) fn capture_and_publish(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    j.phase = Phase::CapturingBaseline;
    j.capture_tree = None;
    j.new_baseline_oid = None;
    svn_journal::write(repo, j)?;

    // §6.3.1：S7 从当前 HEAD 树（此刻就是旧基线）起步
    let idx = TempIndex::from_head(repo)?;
    let (tree, map) = match capture_single(repo, cfg, &idx)? {
        Some(done) => done,
        // S5 之后白名单仍是混合版本（例如 external 没跟着更新到同一个 revision）：
        // 按文件记账录（计划 Task 5），带体检与墓碑。单值时不走这条，行为与改造前相同
        None => {
            let rec = record_mixed(repo, cfg, &idx).map_err(|e| {
                if e.code == "svn_rebuild_revision_moved" {
                    moved_error(&e.message)
                } else {
                    e
                }
            })?;
            (rec.tree, rec.map)
        }
    };
    // 创建 commit 前记录 captureTree（§7.6）
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree.clone());
    svn_journal::write(repo, j)?;
    let commit = create_baseline_commit(repo, &tree, Some(&old), &map)?;
    // 移动 ref 前记录 newBaselineOid（§7.6）
    j.new_baseline_oid = Some(commit);
    svn_journal::write(repo, j)?;
    finish_publish(repo, j)
}

/// S6–S7 的单值录制：P1–P4 与 nodeRevs 单值，与改造前逐字相同。白名单不是单值时返回
/// `None`，交给 [`record_mixed`]；其余失败照旧报错（journal 留在 capturing-baseline，
/// 继续时从这里重来）。
fn capture_single(
    repo: &str,
    cfg: &SvnConfig,
    idx: &TempIndex,
) -> Result<Option<(String, BaseMap)>> {
    let pre = match run_preflight(repo, &cfg.whitelist) {
        Ok(p) => p,
        Err(e) if e.code == "svn_preflight_mixed_revision" => return Ok(None),
        Err(e) => return Err(e),
    };
    match record_single_rev(repo, cfg, pre, idx)? {
        Ok(done) => Ok(Some(done)),
        Err(moved) => Err(moved_error(&moved.to_string())),
    }
}

/// 录制期间 revision 被挪了：录之前与录完之后读到的 `nodeRevs`。
pub(crate) struct RevisionMoved {
    pub before: u64,
    pub now: u64,
}

impl std::fmt::Display for RevisionMoved {
    /// 报错里的写法：`r5 → r6`。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "r{} → r{}", self.before, self.now)
    }
}

/// 按 `pre` 的单值 revision 把服务器内容录进 `idx`，录完再读一次 `nodeRevs` 核对没被挪
/// （与 R3c 同一道：录制期间有人在 TortoiseSVN 里 commit / update）。返回树与单值 base map。
///
/// 更新的 S7 与交付的 D7a 共用（第 77 号 issue 第二批）。revision 被挪了返回内层的 `Err`，
/// 由调用方报各自的错误：两边的错误码与说法不同。
pub(crate) fn record_single_rev(
    repo: &str,
    cfg: &SvnConfig,
    pre: Preflight,
    idx: &TempIndex,
) -> Result<std::result::Result<(String, BaseMap), RevisionMoved>> {
    let (tree, _explained) = record_server_tree_into(repo, cfg, &pre, idx)?;
    let now = read_node_rev(repo, &cfg.whitelist)?;
    if now != pre.node_rev {
        return Ok(Err(RevisionMoved {
            before: pre.node_rev,
            now,
        }));
    }
    let map = BaseMap::single(pre.repos_root, pre.uuid, pre.node_rev);
    Ok(Ok((tree, map)))
}

fn moved_error(detail: &str) -> GitError {
    GitError::new(
        "sync_in_revision_moved",
        format!(
            "录新基线的过程中工作副本被外部改动了（{detail}）。基线没有推进，\
             记录保留；外部操作结束后在顶部横幅上点「继续」。"
        ),
    )
}

/// 发布 + 索引对齐 + `baseline-ready`。重开时也从这里续（§7.6 publishing-baseline）。
pub(crate) fn finish_publish(repo: &str, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    publish_baseline_ref(repo, &old, &new, "更新前", "")?;
    align_index(repo, &old, &new)?;
    j.phase = Phase::BaselineReady;
    svn_journal::write(repo, j)
}

/// 把基线分支从 `old` 挪到本次录好的 `new`（CAS）。已经是 `new` 时什么都不做（崩溃恢复时会走到）；
/// 既不是 `old` 也不是 `new` 就停下、不覆盖。更新（S7）与交付（D8b）共用（第 77 号 issue 第二批）。
///
/// `before` 是报错里旧基线的叫法（「更新前」「交付前」），`tail` 接在报错末尾（交付要说提交已在服务器上）。
pub(crate) fn publish_baseline_ref(
    repo: &str,
    old: &str,
    new: &str,
    before: &str,
    tail: &str,
) -> Result<()> {
    match read_baseline(repo)?.as_deref() {
        Some(cur) if cur == new => Ok(()),
        Some(cur) if cur == old => move_baseline_ref(repo, new, Some(old)),
        other => Err(GitError::new(
            "svn_baseline_moved",
            format!(
                "基线分支现在是 {other:?}，既不是{before}的 {old}，也不是本次录好的 {new}。\
                 有别的工具动过它，这里停下，不覆盖。{tail}"
            ),
        )),
    }
}

/// S3 之后真实索引 == 旧基线树，而 HEAD 跟着 `svn/base` 移到了新 commit。
/// 不对齐的话 S8 切分支时，新旧两棵树的差异会被当成「已暂存的改动」带走。
fn align_index(repo: &str, old: &str, new: &str) -> Result<()> {
    let (code, _) = run_git_code(repo, &["diff-index", "--cached", "--quiet", new])?;
    if code == 0 {
        return Ok(()); // 已对齐
    }
    // CAS：索引必须仍是旧基线树，否则有别的 git 操作动过它，宁可停下也不覆盖
    swap_index_to_tree(repo, old, new)?;
    // 此刻 HEAD 已在新基线上，索引与它一致，可以回填这次变了的条目的 stat
    refresh_index_stat(repo);
    Ok(())
}

/// S8 起：切回原分支、复核待重放区间、rebase。
pub(crate) fn from_baseline_ready(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
) -> Result<SyncInOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    // 先核对本次发布的基线，再动工作区：基线被外部挪过就在切分支、清残留之前停下，
    // 而不是把现场改了一半才拒绝（PR #49 Codex review #4）
    let new_base = published_baseline(repo, j)?;
    // S8
    match current_branch(repo)?.as_deref() {
        Some(b) if b == BASELINE_BRANCH => checkout_verified(repo, cfg, &orig, false)?,
        Some(b) if b == orig => {}
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是基线分支也不是原分支 `{orig}`"
            )))
        }
    }
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    // 上一次 S8 可能「退出 0 却没切干净」，继续时 HEAD 已经在原分支上（A15 / C1）
    settle_checkout_residue(repo, cfg, j, &orig)?;
    // S9：对新基线再复核一次（§7.3.2）
    assert_linear(repo, &new_base, &orig_head)?;
    j.phase = Phase::Rebase;
    svn_journal::write(repo, j)?;
    // 用户配置不能改变 rebase 的语义，见 `PINNED_REBASE_CONFIG`
    let ran = if j.rebase_todo.is_empty() {
        run_git(repo, &pinned_rebase(&["rebase", "--empty=drop", &new_base]))
    } else {
        rebase_with_todo(repo, &new_base, &j.rebase_todo)
    };
    match ran {
        Ok(_) => finish_forward(repo, j),
        // rebase 根本没开始：不是冲突，也不是 git 的报错，「继续」只会拿同一份列表再失败一次
        Err(e) if e.code == REBASE_TODO_LOST => Err(e),
        Err(e) => conflict_or_error(repo, e),
    }
}

/// 手动跟进事先准备的 C' 已被 gc 清掉（见 [`rebase_with_todo`]）。
const REBASE_TODO_LOST: &str = "svn_follow_manual_todo_lost";

/// 手动跟进的 S9：按事先算好的列表逐个 pick（事故分析 6.10，`svn_follow::snapshot_todo`）。
///
/// 用 `rebase -i` 换掉 todo，而不是自己 cherry-pick：继续、放弃、冲突出口都还是 `git rebase` 的
/// 那套状态机，下游一行不用改。todo 经 `GIT_SEQUENCE_EDITOR` 写入，内容放环境变量：git 把编辑器命令
/// 交给 sh，todo 文件路径由 git 自己作为参数传给它，我们不拼任何路径进命令行（仓库路径可能含空格、
/// 中文，Windows 上还有代码页问题）。环境变量的优先级高于 `sequence.editor` 配置。
///
/// 钉住的配置：`missingCommitsCheck=ignore`，原 commit 不在 todo 里是故意的，用户配成 `error` 时
/// rebase 会拒绝；`autoSquash=false`，免得 `fixup!` commit 在我们写 todo 之前被挪动；另外两项同上。
fn rebase_with_todo(repo: &str, new_base: &str, todo: &[String]) -> Result<String> {
    let mut list = String::new();
    let mut oids = String::new();
    for oid in todo {
        // 分支上全是快照内容：什么都不 pick，分支落到新基线上
        if oid == crate::engine::svn_follow::NOOP {
            list.push_str("noop\n");
            continue;
        }
        // 只放 OID：todo 里的说明文字随用户配置变，pick 只认 OID
        if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(GitError::new(
                "sync_in_rebase_failed",
                format!("内部错误：待重放列表里不是一个完整 OID：`{oid}`。基线已经推进，记录保留，可以点「放弃」。"),
            ));
        }
        list.push_str("pick ");
        list.push_str(oid);
        list.push('\n');
        oids.push_str(oid);
        oids.push('\n');
    }
    // C' 没有 ref 指着：记录搁置太久又碰上 gc 时会被清掉。在 rebase 开始之前查，免得它停在半路
    // （独立 review 第 9 条）。这时什么都还没动，HEAD 仍在原分支上
    let check = run_git_stdin(repo, &["cat-file", "--batch-check"], oids.as_bytes())?;
    if check.lines().any(|l| l.ends_with(" missing")) {
        return Err(GitError::new(
            REBASE_TODO_LOST,
            "手动跟进事先准备好的提交已经被 git 清理掉了（记录搁置太久）。分支没动，未提交的改动收在备份里：\
             点「放弃」会把它们放回来，然后再点一次「跟进」重新开始。",
        ));
    }
    run_git_env(
        repo,
        &pinned_rebase(&[
            "-c",
            "rebase.missingCommitsCheck=ignore",
            "-c",
            "rebase.autoSquash=false",
            "rebase",
            "-i",
            "--empty=drop",
            new_base,
        ]),
        &[
            (
                "GIT_SEQUENCE_EDITOR",
                "printf '%s' \"$MYGITCLIENT_REBASE_TODO\" >",
            ),
            ("MYGITCLIENT_REBASE_TODO", &list),
        ],
    )
}

/// rebase 非零退出：真冲突就停在冲突态（正常出口），否则按命令错误处理（§7.3.2 末）。
pub(crate) fn conflict_or_error(repo: &str, e: GitError) -> Result<SyncInOutcome> {
    let files = unmerged_paths(repo)?;
    if rebase_in_progress(repo)? && !files.is_empty() {
        return Ok(SyncInOutcome::RebaseConflict { files });
    }
    Err(GitError::new(
        "sync_in_rebase_failed",
        format!(
            "把分支重放到新基线上失败了，而且不是冲突。基线已经推进，记录保留：按 git 说的\
             处理好之后点「继续」会重新重放，也可以点「放弃」回到更新前。git 原话：{}",
            e.message.trim()
        ),
    ))
}

/// S10–S11，结局记为 `Done`。
pub(crate) fn finish_forward(repo: &str, j: &mut Journal) -> Result<SyncInOutcome> {
    let manual = j.flow == Flow::FollowManual;
    if manual {
        // 分支已经落到基线上，旧快照里的 HEAD 与暂存都过时了。换成这一刻只含 commit 的快照
        // （跟进的 F6 同样换）：放回收起的改动不动 HEAD，暂存本来就不记，放在这里与放在后面一样
        renew_mark_after_follow(repo);
    }
    // 收尾核对只给手动跟进：更新（sync_in）自己跑了 svn update，磁盘本来就该变
    let mut audit = manual.then(Audit::default);
    // 在动手之前读：收尾之后再读失败的话，核对的发现就跟着错误一起丢了（独立 review）
    let new_rev = if manual {
        Some(baseline_rev(repo)?)
    } else {
        None
    };
    let step = restore_step_with(repo, j, audit.as_mut())?;
    if let (Some(a), Some(new_rev)) = (audit.filter(Audit::worth_reporting), new_rev) {
        let kept = match &step {
            RestoreStep::Finished => Some((None, None)),
            RestoreStep::Kept { oid } => Some((Some(oid.clone()), None)),
            RestoreStep::StageKept { oid, files } => Some((Some(oid.clone()), Some(files.clone()))),
            // 冲突 / 撞名时改动还没放回，核对不会跑；万一有，照原来的结局走
            RestoreStep::Conflict { .. } | RestoreStep::Blocked { .. } => None,
        };
        if let Some((stash_oid, stage_lost)) = kept {
            let drift = a.drift.unwrap_or_default();
            return Ok(SyncInOutcome::FollowDrift {
                new_rev,
                changed: drift.changed,
                resurrected: drift.resurrected,
                before: a.before.unwrap_or_default(),
                stash_oid,
                stage_lost,
                audit_error: a.error,
            });
        }
    }
    Ok(match step {
        RestoreStep::Finished if j.stage_lost => SyncInOutcome::StageLost {
            new_rev: Some(baseline_rev(repo)?),
            stash_oid: None,
            files: Vec::new(),
        },
        RestoreStep::Finished => SyncInOutcome::Done {
            new_rev: baseline_rev(repo)?,
        },
        RestoreStep::StageKept { oid, files } => SyncInOutcome::StageLost {
            new_rev: Some(baseline_rev(repo)?),
            stash_oid: Some(oid),
            files,
        },
        RestoreStep::Kept { oid } => SyncInOutcome::Settled {
            stash_oid: oid,
            rolled_back: false,
        },
        RestoreStep::Conflict { files, oid } => SyncInOutcome::StashConflict {
            files,
            stash_oid: oid,
        },
        RestoreStep::Blocked { collisions, oid } => SyncInOutcome::StashBlocked {
            collisions,
            stash_oid: oid,
        },
    })
}

pub(crate) enum RestoreStep {
    Finished,
    /// 收尾了，但备份留在 stash 里（见 [`SyncInOutcome::Settled`]）。
    Kept {
        oid: String,
    },
    /// 干净地放回了，但暂存没保住：stash 与钉子都留着（见 [`SyncInOutcome::StageLost`]）。
    StageKept {
        oid: String,
        files: Vec<String>,
    },
    Conflict {
        files: Vec<String>,
        oid: String,
    },
    Blocked {
        collisions: Vec<String>,
        oid: String,
    },
}

/// 手动跟进收尾核对的输入与结果（[`restore_step_with`]）。
#[derive(Default)]
pub(crate) struct Audit {
    /// 核对跑完了才有。`Some(空)` 是「核对过，干净」。
    drift: Option<svn_follow_audit::Drift>,
    /// 拿来比的「跟进前的磁盘」：stash 或原分支 HEAD。
    before: Option<String>,
    /// 核对没跑成（git 出错）时的原因。这时流程照常收尾、备份留着，但要告诉用户没核对。
    error: Option<String>,
}

impl Audit {
    /// 要不要报 `FollowDrift`：有发现，或者核对没跑成。
    fn worth_reporting(&self) -> bool {
        self.error.is_some() || self.drift.as_ref().is_some_and(|d| !d.is_empty())
    }
}

/// S10 + S11：取回 WIP、drop、删 journal。往前走与往回走（放弃）共用。
pub(crate) fn restore_step(repo: &str, j: &mut Journal) -> Result<RestoreStep> {
    restore_step_with(repo, j, None)
}

/// 跟进前的磁盘与此刻的磁盘比一遍，结果写进 `audit`。返回「是否要留下备份」。
///
/// **不返回错误**：核对是收尾之后的附加检查，它出错（某个文件读不了、过滤器缺失）不能让流程停住
/// ——没有 stash 时 journal 还停在 rebase，「继续」会再核对一次、再出同一个错，永远走不完（独立
/// review）。出错就记下原因、留下备份，交给用户。
fn run_audit(
    j: &Journal,
    repo: &str,
    before: &str,
    untracked: Option<&str>,
    audit: &mut Audit,
) -> bool {
    audit.before = Some(before.to_string());
    let result = (|| {
        let new_base = need(&j.new_baseline_oid, "newBaselineOid")?;
        let orig_head = need(&j.orig_head, "origHead")?;
        let old_base = need(&j.baseline_oid_before, "baselineOidBefore")?;
        svn_follow_audit::audit(repo, before, untracked, &orig_head, &old_base, &new_base)
    })();
    match result {
        Ok(drift) => {
            let keep = !drift.is_empty();
            audit.drift = Some(drift);
            keep
        }
        Err(e) => {
            log::warn!("手动跟进的收尾核对没跑成：{} {}", e.code, e.message);
            audit.error = Some(e.message);
            true
        }
    }
}

/// [`restore_step`] 加上收尾核对：`audit` 为 `Some` 时，改动干净放回之后、drop 之前核对磁盘，
/// 有发现（或核对没跑成）就**不 drop**，按 `Kept` 收尾。
pub(crate) fn restore_step_with(
    repo: &str,
    j: &mut Journal,
    mut audit: Option<&mut Audit>,
) -> Result<RestoreStep> {
    let Some(oid) = j.stash_oid.clone() else {
        // 没有 WIP（计划 A7）：跟进前的磁盘就是原分支 HEAD，白名单里也没有未跟踪文件。
        // 先收尾再核对：这里没有备份要保护，核对怎样都不该让流程停在半路
        svn_journal::remove(repo)?;
        if let Some(a) = audit.as_deref_mut() {
            match need(&j.orig_head, "origHead") {
                Ok(orig_head) => {
                    run_audit(j, repo, &orig_head, None, a);
                }
                Err(e) => a.error = Some(e.message),
            }
        }
        return Ok(RestoreStep::Finished);
    };
    if !j.stash_applied {
        j.phase = Phase::ApplyingStash;
        svn_journal::write(repo, j)?;
        let outcome = if j.flow == Flow::FollowManual {
            let (outcome, stage_lost) = wip::restore_wip_keeping_index(repo, &oid)?;
            j.stage_lost = stage_lost;
            outcome
        } else {
            wip::restore_wip(repo, &oid)?
        };
        match outcome {
            RestoreOutcome::Applied => {}
            RestoreOutcome::Conflicted { files } => {
                // 先落盘：下面的自动解决写了磁盘，崩在中途重开时不能再 apply 一次（计划 A2）
                j.stash_applied = true;
                svn_journal::write(repo, j)?;
                // 手动跟进时，新 HEAD 与新基线相同的冲突文件按跟进前的磁盘解决（使用报告问题 4、5）。
                // 更新流程不用：它的 stash 是在 svn update 之前压的，基准就是对的。
                // 回滚不用：回滚没有收尾核对，checkout 静默没写进去（被占用的文件）时没人发现，
                // 而后面紧跟着 drop（独立 review 第 3 条）
                let files = if j.flow == Flow::FollowManual && !j.rolling_back {
                    let new_base = need(&j.new_baseline_oid, "newBaselineOid")?;
                    wip::settle_untouched_conflicts(repo, &oid, &new_base, &files)?
                } else {
                    files
                };
                if !files.is_empty() {
                    return Ok(RestoreStep::Conflict { files, oid });
                }
                // 全部解决：与干净放回同路（核对、drop）。暂存只在跟进前真有暂存时才算没保住
                wip::unstage_applied(repo)?;
                j.stage_lost = wip::had_staged(repo, &oid)?;
            }
            RestoreOutcome::Blocked { collisions } => {
                return Ok(RestoreStep::Blocked { collisions, oid })
            }
        }
        // 暂存没放回去时，与磁盘不同的那份暂存只在 stash 里（Review 3 第 1 条）。手动跟进带
        // `--index` 放回成功时暂存原样在索引里，不用查
        let staged_only = if j.flow == Flow::FollowManual && !j.stage_lost {
            Vec::new()
        } else {
            wip::staged_only_paths(repo, &oid)?
        };
        if !staged_only.is_empty() {
            j.stage_lost = true;
        }
        // apply 成功之后、drop 之前落盘：崩在这里重开时只 drop，不再 apply（计划 A2）
        j.stash_applied = true;
        svn_journal::write(repo, j)?;
        let keep = match audit {
            Some(a) => match svn_follow_audit::stash_untracked(repo, &oid) {
                Ok(untracked) => run_audit(j, repo, &oid, untracked.as_deref(), a),
                Err(e) => {
                    a.before = Some(oid.clone());
                    a.error = Some(e.message);
                    true
                }
            },
            None => false,
        };
        if j.stage_lost {
            // 不 drop、不拆钉子：拆了之后用户在 stash 列表里删掉那条，暂存的那份就没有任何引用了。
            // 手动跟进退回普通 apply 时即使 `staged_only` 为空也留着：内容磁盘上都有，但用户拿暂存区
            // 当存档，备份交给用户确认后自己删。崩在这里之后的「继续」走下面的 else，按 `Kept`
            // 收尾，备份照样留在 stash 列表里
            svn_journal::remove(repo)?;
            return Ok(RestoreStep::StageKept {
                oid,
                files: staged_only,
            });
        }
        if keep {
            // 核对有发现：stash 里那份可能是正确内容唯一的副本（11-05 就是），不 drop。
            // 只拆钉子、留在 stash 列表里，与 `Kept` 同一个处理（PR #49 review 第四轮 P1-2）
            wip::unpin(repo, &j.opid)?;
            svn_journal::remove(repo)?;
            return Ok(RestoreStep::Kept { oid });
        }
    } else {
        let files = unmerged_paths(repo)?;
        if !files.is_empty() {
            return Err(unresolved(&files));
        }
        // 已经 apply 过、又停下过：磁盘上的 WIP 经过了用户之手，证明不了它还完整，
        // stash 里那份可能是唯一的备份——不 drop，只拆掉我们自己的钉子与记录
        // （PR #49 review 第四轮 P1-2，方案 A）。只有同一进程里刚刚干净 apply 完的才 drop
        //
        // `stashApplied` 也可能是干净 apply 之后、drop 之前落的盘：崩在 drop 之后、删 journal
        // 之前时栈里已经没有它了。「备份还在」必须是查出来的，不是推出来的（PR #49 Codex review #1）
        let kept = wip::stash_position(repo, &oid)?.is_some();
        wip::unpin(repo, &j.opid)?;
        svn_journal::remove(repo)?;
        return Ok(if kept {
            RestoreStep::Kept { oid }
        } else {
            RestoreStep::Finished
        });
    }
    wip::drop_wip(repo, &j.opid, &oid)?;
    svn_journal::remove(repo)?; // S11；锁由调用方的守卫释放，watcher 由命令层恢复
    Ok(RestoreStep::Finished)
}

/// 切到分支 `branch`，事后核对真的切干净了（见 [`run_checkout_verified`]）。
pub(crate) fn checkout_verified(
    repo: &str,
    cfg: &SvnConfig,
    branch: &str,
    force: bool,
) -> Result<()> {
    let mut args: Vec<&str> = vec!["checkout"];
    if force {
        args.push("-f");
    }
    args.push(branch);
    run_checkout_verified(repo, cfg, &args, branch, || {
        Ok(current_branch(repo)?.as_deref() == Some(branch))
    })
}

/// 跑一次 `git checkout`（`args` 是完整参数），之后核对它真的切干净了（计划 A15）。
/// `sync_out` 的 detached checkout 也走这里（第 77 号 issue 去重）。
///
/// 2026-09-25 实测：文件被以 share=Read 打开（Unity 的方式）时，checkout **退出 0**、
/// HEAD 已经切走，而那个文件还是原来的内容——git 只在 stderr 里打一行
/// `unable to unlink old`。所以退出码证明不了任何事，只能事后看。
///
/// `on_target`：HEAD 是不是落在了目标上，checkout 之后、核对文件之前调用。`label` 是报错里的目标名。
pub(crate) fn run_checkout_verified(
    repo: &str,
    cfg: &SvnConfig,
    args: &[&str],
    label: &str,
    on_target: impl FnOnce() -> Result<bool>,
) -> Result<()> {
    let from = rev_parse(repo, "HEAD")?;
    let ran = run_git(repo, args);
    let on_target = on_target()?;
    let mut stale = tracked_changes(repo, cfg)?;
    // 该删却没删掉的文件退出 0 地留下，成了未跟踪文件（PR #49 review #7）
    stale.extend(untracked_leftovers(repo, cfg, &[from])?);
    if on_target && stale.is_empty() {
        return Ok(());
    }
    Err(checkout_failed(repo, label, &stale, ran))
}

/// checkout 没切干净时的报错（计划 A15）。经 [`run_checkout_verified`]，`sync_out` 的 detached checkout 也用它。
pub(crate) fn checkout_failed(
    repo: &str,
    target: &str,
    stale: &[String],
    ran: Result<String>,
) -> GitError {
    let files = if stale.is_empty() {
        String::new()
    } else {
        format!("\n下面这些文件没能写进去或删掉：\n{}", list_paths(stale))
    };
    let git_said = match ran {
        Err(e) => format!("\ngit 原话：{}", e.message.trim()),
        Ok(_) => String::new(),
    };
    let root = Path::new(repo);
    GitError::new(
        "sync_in_checkout_failed",
        format!(
            "切到 `{target}` 没有成功，多半是有程序占着文件（Unity / IDE）。{files}\n{}{git_said}",
            describe(root, &find_blockers(root, 5))
        ),
    )
}

/// 白名单内被跟踪文件相对 HEAD 的改动（不含未跟踪）。
pub(crate) fn tracked_changes(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(Vec::new()); // 空 pathspec 会变成「全部」（§12.2）
    }
    let mut args: Vec<&str> = vec!["diff", "--name-only", "-z", "HEAD", "--"];
    args.extend(roots.iter().map(String::as_str));
    Ok(run_git(repo, &args)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// `from` 里某个 commit 有、当前 HEAD 没有、却还在磁盘上且不在索引里的白名单内文件。
///
/// 这是 checkout 该删却没删掉的形状（PR #49 review #7）：文件被占着时 unlink 失败，
/// checkout 照样退出 0，文件成了新分支上的未跟踪文件。[`tracked_changes`] 看不见它们，
/// 而同名未跟踪文件**即使内容相同**也会挡住之后的 checkout / rebase（2026-09-26 实测）。
///
/// 已进索引的不算：那是被跟踪的改动，归 [`tracked_changes`] 管。
pub(crate) fn untracked_leftovers(
    repo: &str,
    cfg: &SvnConfig,
    from: &[String],
) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(Vec::new()); // 空 pathspec 会变成「全部」（§12.2）
    }
    let mut out: Vec<String> = Vec::new();
    for f in from {
        let mut args: Vec<&str> = vec![
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=A",
            "HEAD",
            f,
            "--",
        ];
        args.extend(roots.iter().map(String::as_str));
        for p in run_git(repo, &args)?.split('\0').filter(|p| !p.is_empty()) {
            if !out.iter().any(|q| q == p) && Path::new(repo).join(p).symlink_metadata().is_ok() {
                out.push(p.to_string());
            }
        }
    }
    if out.is_empty() {
        return Ok(out); // 下面的 ls-files 不能带空 pathspec
    }
    let mut args: Vec<&str> = vec!["ls-files", "-z", "--"];
    args.extend(out.iter().map(String::as_str));
    let indexed = run_git(repo, &args)?;
    let indexed: Vec<&str> = indexed.split('\0').filter(|p| !p.is_empty()).collect();
    out.retain(|p| !indexed.contains(&p.as_str()));
    Ok(out)
}

/// S4：白名单内的 W-file 在 SVN 看来有没有修改。`?` / ignored 不算——它们不是
/// 「base 文件被改了」，而是 SVN 不认识的文件。判定见 [`s4_counts_as_modified`]。
pub(crate) fn dirty_w_files(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let scope = WScope::for_repo(repo, cfg)?;
    let wcs = enumerate_wcs(repo, &cfg.whitelist)?;
    let targets = check_targets(scope.roots(), &wcs);
    let entries = read_dirty(repo, &targets)?;
    let w_files = scope.filter(entries.iter().map(|e| e.path.as_str()))?;
    Ok(entries
        .into_iter()
        .filter(|e| w_files.contains(&e.path))
        .filter(s4_counts_as_modified)
        .map(|e| e.path)
        .collect())
}

/// S4 时（已切到基线）这一条在 SVN 看来算不算「基线记的内容与工作副本对不上」。
///
/// # 已 `svn add`、还没提交的文件不算
///
/// 分支上新建的文件基线里没有，S3 切到基线时 git 把它从磁盘删掉，SVN 看到的是
/// `missing` + `revision="-1"`。它从没上过服务器，与基线对不对得上无关；S5 的
/// `svn update` 不碰它，S9 重放分支时 git 把它写回来，添加标记与 changelist 全程没动
/// （2026-11-26 现场误拦，见 `tests/svn_sync_in.rs` 对应用例）。
///
/// 只认 `schedule == "add"` 这一个正面证据，与 [`crate::engine::svn_dirty::classify`]
/// 摘掉这类条目的判法相同。`replace`（`svn rm X` 后同名再加）服务器上**有**旧内容，
/// 查不到 schedule 的说不清来源，都照旧拦下。受版本控制的文件不见了是 `revision="N"`，
/// 走不到这里。
///
/// 磁盘上还在的 `added` 照旧拦：切到基线之后 git 管的新文件只可能不在磁盘上，
/// 还在说明它不是分支带来的，来源不明。
fn s4_counts_as_modified(e: &StatusEntry) -> bool {
    let added_and_away =
        e.item == "missing" && e.revision == Some(-1) && e.schedule.as_deref() == Some("add");
    if added_and_away {
        return false;
    }
    !matches!(
        e.item.as_str(),
        "normal" | "unversioned" | "ignored" | "external" | "none"
    ) || e.props == "modified"
}

/// 工作副本根目录上有没有没释放的写锁（svn status 的 `L`）。`svn update` 从根目录起整棵树加锁，
/// 被打断后根目录一定还锁着，只看根目录（`--depth=empty`）就够，不用遍历整个工作副本。
fn wc_root_locked(repo: &str) -> Result<bool> {
    let arg = peg_safe(&repo.replace('\\', "/"));
    let xml = run_svn(
        repo,
        &[
            "status",
            "--depth=empty",
            "--ignore-externals",
            "--xml",
            "--",
            &arg,
        ],
    )?;
    Ok(parse_status_entries(&xml)?.iter().any(|e| e.wc_locked))
}

/// S5 之后的冲突（计划 A4：`svn update` 有冲突时仍然退出 0）。
fn svn_conflicts(repo: &str) -> Result<Vec<String>> {
    let xml = run_svn(repo, &["status", "--xml"])?;
    Ok(parse_update_status(&xml)?
        .entries
        .into_iter()
        .filter(|e| e.is_conflicted())
        .map(|e| e.path)
        .collect())
}

pub(crate) fn rebase_in_progress(repo: &str) -> Result<bool> {
    let d = git_dir(repo)?;
    Ok(d.join("rebase-merge").exists() || d.join("rebase-apply").exists())
}

/// 新基线的 floor（混合基线也读得出，计划 Task 5）。
fn baseline_rev(repo: &str) -> Result<u64> {
    crate::engine::svn_sync::read_baseline_rev(repo)
}

pub(crate) fn need(v: &Option<String>, field: &str) -> Result<String> {
    v.clone().ok_or_else(|| {
        GitError::new(
            "svn_journal_corrupt",
            format!("同步记录缺字段 `{field}`，不知道该往哪里走。请联系开发者，不要删记录文件。"),
        )
    })
}

pub(crate) fn state_changed(what: &str) -> GitError {
    GitError::new(
        "sync_in_state_changed",
        format!("{what}。有别的 git 操作动过仓库，这里停下，记录与 stash 原样保留。"),
    )
}

pub(crate) fn unresolved(files: &[String]) -> GitError {
    GitError::new(
        "sync_in_unresolved",
        format!(
            "还有冲突没解决：\n{}\n改好之后把它们暂存（git add），再点「继续」。",
            list_paths(files)
        ),
    )
}

pub(crate) fn new_opid() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // 只含十六进制字符：它会进 stash message 与 ref 名（`refs/mygitclient/sync/<opid>`）
    format!("{nanos:x}{:x}", std::process::id())
}

// —— 继续 / 放弃（spec §7.6：「提供继续 / 放弃两个入口」）——

/// 往回走的两种前提（计划 A5 / A12）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disk {
    /// S5 还没跑、且是本进程一路走到这里：磁盘上被跟踪的内容全是 git 自己写的，
    /// `checkout -f` 只会还原 git 写下的字节。
    GitOnly,
    /// S5 可能跑过，或从崩溃里恢复、不知道跑没跑：磁盘上可能是服务器内容，
    /// `-f` 之前必须先确认那确实是服务器内容（能从服务器取回）。
    MaybeUpdated,
}

/// 从中断处接着走。
pub fn continue_sync(repo: &str) -> Result<SyncInOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    // 一次「放弃」已经存下服务器内容、动了手却没走完：往前走会在分支、磁盘半新半旧的现场上
    // 重跑 update。只能接着放弃（它会把存下的内容写回去）。放回 WIP 那一步之前已经写回了，
    // `applying-stash` 照常
    if !j.kept_server.is_empty() && j.phase != Phase::ApplyingStash {
        return Err(GitError::new(
            "sync_in_abort_unfinished",
            "上一次「放弃」还没走完（有些文件还没换回服务器上的版本）。请再点一次「放弃」，\
             它会接着把这些文件写回去。",
        ));
    }
    match j.phase {
        // A10：`start` 的定义就是「还没动任何东西」
        Phase::Start => {
            svn_journal::remove(repo)?;
            Ok(SyncInOutcome::Aborted)
        }
        Phase::Stashing => {
            wip::settle_stashing(repo, j)?;
            resume_stashed(repo, &cfg, j)
        }
        Phase::Stashed => resume_stashed(repo, &cfg, j),
        Phase::CapturingBaseline => {
            require_on_old_baseline(repo, j)?;
            capture_and_publish(repo, &cfg, j)?;
            from_baseline_ready(repo, &cfg, j)
        }
        Phase::PublishingBaseline => {
            if j.new_baseline_oid.is_some() {
                // commit 已建：只补发布与索引对齐，不重建另一个 commit（§7.6）
                require_on_branch(repo, BASELINE_BRANCH)?;
                finish_publish(repo, j)?;
            } else {
                // 没有新 OID = 尚未发布，重新验证前提、重建候选索引（§7.6）
                require_on_old_baseline(repo, j)?;
                capture_and_publish(repo, &cfg, j)?;
            }
            from_baseline_ready(repo, &cfg, j)
        }
        Phase::BaselineReady => from_baseline_ready(repo, &cfg, j),
        Phase::Rebase => continue_rebase(repo, &cfg, j),
        Phase::ApplyingStash => {
            require_restore_target(repo, j)?;
            if j.rolling_back {
                // 往回走时停下的：收尾的是一次回退，不是更新（PR #49 review #3）
                let step = restore_step(repo, j)?;
                Ok(back_outcome(step, j.stage_lost))
            } else {
                finish_forward(repo, j)
            }
        }
        _ => Err(sync_out_unsupported()),
    }
}

/// 放弃这次同步：回到更新前的分支、放回 WIP。SVN 侧已做的不退回（计划 A5）。
pub fn abort_sync(repo: &str) -> Result<SyncInOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    let step = match j.phase {
        Phase::Start => {
            svn_journal::remove(repo)?;
            return Ok(SyncInOutcome::Aborted);
        }
        Phase::Stashing => {
            wip::settle_stashing(repo, j)?;
            back_to_orig(repo, &cfg, j, Disk::MaybeUpdated)?
        }
        Phase::Stashed
        | Phase::CapturingBaseline
        | Phase::PublishingBaseline
        | Phase::BaselineReady => back_to_orig(repo, &cfg, j, Disk::MaybeUpdated)?,
        Phase::Rebase => {
            if rebase_in_progress(repo)? {
                // A14 同样适用于放弃（PR #49 review #4）：`--abort` 会丢掉那次 rebase
                // 里已经解决的冲突，不是本次同步发起的就不能碰
                require_our_rebase(repo, j)?;
                run_git(repo, &["rebase", "--abort"])?;
            }
            back_to_orig(repo, &cfg, j, Disk::MaybeUpdated)?
        }
        Phase::ApplyingStash => {
            // 放弃接管：stash 条目原样留在列表里（message 带 opid，认得出来），
            // 只删钉住的 ref 与 journal
            let stash_oid = need(&j.stash_oid, "stashOid")?;
            wip::unpin(repo, &j.opid)?;
            svn_journal::remove(repo)?;
            return Ok(SyncInOutcome::Released {
                stash_oid,
                rolled_back: j.rolling_back,
            });
        }
        _ => return Err(sync_out_unsupported()),
    };
    Ok(back_outcome(step, j.stage_lost))
}

/// 往回走（放弃 / 回退）那一次 S10–S11 的结局。
fn back_outcome(step: RestoreStep, stage_lost: bool) -> SyncInOutcome {
    match step {
        RestoreStep::Finished if stage_lost => SyncInOutcome::StageLost {
            new_rev: None,
            stash_oid: None,
            files: Vec::new(),
        },
        RestoreStep::Finished => SyncInOutcome::Aborted,
        RestoreStep::StageKept { oid, files } => SyncInOutcome::StageLost {
            new_rev: None,
            stash_oid: Some(oid),
            files,
        },
        RestoreStep::Kept { oid } => SyncInOutcome::Settled {
            stash_oid: oid,
            rolled_back: true,
        },
        RestoreStep::Conflict { files, oid } => SyncInOutcome::StashConflict {
            files,
            stash_oid: oid,
        },
        RestoreStep::Blocked { collisions, oid } => SyncInOutcome::StashBlocked {
            collisions,
            stash_oid: oid,
        },
    }
}

/// 回到原分支并取回 WIP（spec §13「回滚到」一列；计划 A5 / A12 / A15）。
pub(crate) fn back_to_orig(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    disk: Disk,
) -> Result<RestoreStep> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let branch = current_branch(repo)?;
    if branch.as_deref() != Some(orig.as_str()) {
        if branch.as_deref() != Some(BASELINE_BRANCH) {
            return Err(state_changed(&format!(
                "HEAD 在 {branch:?} 上，既不是原分支 `{orig}` 也不是基线分支"
            )));
        }
        if disk == Disk::MaybeUpdated {
            // 上一次放弃已经存下服务器内容、checkout 却没走完（HEAD 还在基线上，可能写了一部分）：
            // 先写回，磁盘回到放弃之前的样子，下面的判定照常重做
            if !j.kept_server.is_empty() {
                put_back_server_content(repo, &j.kept_server)?;
            }
            // 先认「不知道是谁改的」，再认「update 换掉了的」：手改过的文件两边都会列出，
            // 归到后者会把它说成服务器内容
            let dirty = dirty_w_files(repo, cfg)?;
            if !dirty.is_empty() {
                return Err(GitError::new(
                    "sync_in_unsafe_to_return",
                    format!(
                        "要回到分支 `{orig}` 就得覆盖工作区里的文件，而下面这些文件的内容\
                         既不是 git 写的、也不是服务器上的——不知道是谁改的，不能覆盖：\n{}\n\
                         记录已保留。请先确认这些改动（需要的话自己备份），再点「放弃」。",
                        list_paths(&dirty)
                    ),
                ));
            }
            // 基线还没跟上、update 已经换掉了 git 管的文件（2026-11-27 现场）：`-f` 会把它们写回
            // 旧内容，SVN 却记着新版本。原分支的提交与 WIP 都没碰过的，回去之后保留服务器内容——
            // 它们在原分支上与旧基线一字不差，留下的差异只有「服务器的改动」，是一份如实的未提交
            // 改动，下次更新时基线跟上、三方合并自动抵消。碰过的要三方合并，只能「继续」。
            //
            // 不一律拒绝放弃（独立 review 第 1 条）：update 一直跑不完时（external 失效、没有
            // 写权限……），「继续」次次失败，记录就永远清不掉，跟进、更新、交付全被挡住
            let updated = updated_by_svn(repo, cfg, j)?;
            if !updated.is_empty() {
                let touched = touched_by_branch_or_wip(repo, cfg, j, &updated)?;
                if !touched.is_empty() {
                    return Err(GitError::new(
                        "sync_in_update_incomplete",
                        kept_for_continue(
                            &touched,
                            j,
                            "你的提交或未提交的改动也改过这些文件，回原分支要把两边合起来，\
                             这次没法放弃。请点「继续」把更新跑完；「继续」时 svn update 报错的话，\
                             先在 TortoiseSVN 里对工作副本根目录跑一次 Clean up。一直报同一个错\
                             （比如 external 失效、没有写权限），就先在 TortoiseSVN 里把它解决到 \
                             Update 能跑通，再点「继续」。",
                        ),
                    ));
                }
            }
            // 服务器内容**先落盘再 `-f`**：存进对象库、清单写进 journal。之后不论 checkout、核对
            // 残留、写回哪一步出错或进程被杀，再点「放弃」都能接着写回（第三轮 review）
            j.kept_server = if updated.is_empty() {
                Vec::new()
            } else {
                save_server_content(repo, &updated)?
            };
            svn_journal::write(repo, j)?;
            // 上面已确认 W-file 全部干净（与 `force_back_when_w_clean` 同一道判定，不重跑 svn status）
            checkout_verified(repo, cfg, &orig, true)?;
        } else {
            checkout_verified(repo, cfg, &orig, true)?;
        }
    }
    // HEAD 仍在原分支上（S3 失败时 git 整体拒绝、一个字节没写——计划 A15）
    // 或刚切回来：都要求分支本身没被动过
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    // 上一次切回原分支可能「退出 0 却没切干净」（A15 / C1）；刚才那次 checkout 已经核对过。
    // 存下的服务器内容不算残留：它们既不是分支的也不是基线的，却是我们自己要写回的
    settle_checkout_residue(repo, cfg, j, &orig)?;
    // 写回在放回 WIP **之前**：写不成就停在这里，记录与 stash 都原样留着，再点「放弃」接着写。
    // WIP 没碰过这些文件（`touched_by_branch_or_wip`），放回不会撞上它们。可以重复执行
    put_back_server_content(repo, &j.kept_server)?;
    // 随 restore_step 进入 applying-stash 时一起落盘（PR #49 review #3）
    j.rolling_back = true;
    restore_step(repo, j)
}

/// 用 `-f` 切回原分支（计划 A5）：能 `-f` 的前提是 W-file 在 svn 看来全部干净 = 磁盘上就是服务器内容，
/// 覆盖掉也能从服务器取回。有不干净的就用 `dirty_error` 报错、什么都不动。
///
/// 原本更新的「放弃」与交付的「停止收尾」共用（第 77 号 issue 第二批），现在只剩交付用。更新那边
/// 「能取回」不够：基线还没跟上时写回旧内容，SVN 记的却是新版本，见 [`updated_by_svn`]
/// （2026-11-27 现场）。交付的停止收尾是否有同一形状没有核对过。报错由调用方给。
pub(crate) fn force_back_when_w_clean(
    repo: &str,
    cfg: &SvnConfig,
    orig: &str,
    dirty_error: impl FnOnce(&[String]) -> GitError,
) -> Result<()> {
    let dirty = dirty_w_files(repo, cfg)?;
    if !dirty.is_empty() {
        return Err(dirty_error(&dirty));
    }
    checkout_verified(repo, cfg, orig, true)
}

/// HEAD 已在原分支上，但白名单内被跟踪文件与 HEAD 不一致时怎么办（2c 独立 review C1）。
///
/// 此刻 WIP 全在 stash 里，这类差异只有两个来源：
///
/// 1. **checkout 的残留**：S8 / 回原分支那次 `git checkout` 遇上 share=Read 占用，
///    退出 0、HEAD 已切走，文件却还是基线的字节（A15 实测 L1）。这些字节在基线
///    commit 里原样存着，按 HEAD 写回去不丢任何东西——不写回的话，rebase 与放回
///    WIP 之后磁盘上是「撤掉了你那笔提交」的内容，而结局报成功。
/// 2. **中断期间有人改过**：字节既不是分支的也不是基线的，不知道是谁的，不能覆盖。
///
/// 所以逐个比对：全是新旧基线里的 blob（或文件已不在）才写回，否则指名停下，
/// 记录原样保留。
///
/// 残留有两种形状，处理方式相同：被跟踪文件的内容不对（按 HEAD 写回），以及基线里有、
/// 原分支上没有的文件没删掉、成了未跟踪文件（删掉——PR #49 review #7，不删的话 S9
/// 的 rebase 启动就失败）。
fn settle_checkout_residue(repo: &str, cfg: &SvnConfig, j: &Journal, orig: &str) -> Result<()> {
    // 「WIP 全在 stash 里」的前提只在 S1 确实完成过时成立。没有 stashOid、又停在 S3
    // 之前的 phase、且 HEAD 从没离开过原分支，可能是 push 从未落地（崩在写 journal 与
    // push 之间）：此时 WIP 还原样在磁盘上，没有任何 checkout 残留可言。
    //
    // `leftOrig` 不能省（PR #49 review #2）：没有可压的改动时 phase 一路停在 `stashed`
    // 走过 S3–S5，回退那次 checkout 留下的残留若在这里被跳过，「放弃」会报「已放回」，
    // 磁盘上却是撤掉了你那几笔提交的内容
    if j.stash_oid.is_none()
        && !j.left_orig
        && matches!(j.phase, Phase::Start | Phase::Stashing | Phase::Stashed)
    {
        return Ok(());
    }
    let mut bases = vec![rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?];
    if let Some(old) = &j.baseline_oid_before {
        bases.push(old.clone());
    }
    // 放弃时存下的服务器内容（`keptServer`）由调用方随后写回，不算残留
    let kept: Vec<String> = j.kept_server.iter().map(|k| k.path.clone()).collect();
    settle_residue(
        repo,
        cfg,
        &bases,
        orig,
        "sync_in_unsafe_to_return",
        "「继续」或「放弃」",
        &kept,
    )
}

/// [`settle_checkout_residue`] 的核心，`sync_out` 的「停止收尾」也用（Task 8–10 里程碑
/// review 第 2 条）：差异全是 `bases` 里原样存着的字节才写回，否则指名停下。
///
/// `skip` 里的路径不看：调用方之后会自己写这些文件（更新放弃时存下的服务器内容）。
pub(crate) fn settle_residue(
    repo: &str,
    cfg: &SvnConfig,
    bases: &[String],
    orig: &str,
    unsafe_code: &str,
    // 报错里让用户点的按钮：两个流程的横幅按钮名不同（交付没有「放弃」）
    buttons: &str,
    skip: &[String],
) -> Result<()> {
    let not_skipped =
        |v: Vec<String>| -> Vec<String> { v.into_iter().filter(|p| !skip.contains(p)).collect() };
    let stale = not_skipped(tracked_changes(repo, cfg)?);
    let left = not_skipped(untracked_leftovers(repo, cfg, bases)?);
    if stale.is_empty() && left.is_empty() {
        return Ok(());
    }
    let mut foreign = Vec::new();
    for p in stale.iter().chain(&left) {
        if !is_baseline_bytes(repo, bases, p)? {
            foreign.push(p.clone());
        }
    }
    if !foreign.is_empty() {
        return Err(GitError::new(
            unsafe_code,
            format!(
                "分支 `{orig}` 上下面这些文件的内容既不是分支里的、也不是基线里的——\
                 不知道是谁改的，不能覆盖：\n{}\n记录已保留。请先确认这些改动\
                 （需要的话自己备份），再点{buttons}。",
                list_paths(&foreign)
            ),
        ));
    }
    if !stale.is_empty() {
        // 非空才调：空 pathspec 会变成「全部」（§12.2）。路径走 stdin：大工作副本上
        // 几百个文件拼进命令行会撞上 Windows 的长度上限（`git_cli::run_git_paths`）
        run_git_paths(repo, &["checkout", "HEAD"], &stale)?;
    }
    for p in &left {
        // 删不掉（仍被占着）不在这里报：下面的复核会指名它
        let _ = std::fs::remove_file(Path::new(repo).join(p));
    }
    let mut remaining = not_skipped(tracked_changes(repo, cfg)?);
    remaining.extend(not_skipped(untracked_leftovers(repo, cfg, bases)?));
    if !remaining.is_empty() {
        let root = Path::new(repo);
        return Err(GitError::new(
            "sync_in_checkout_failed",
            format!(
                "分支 `{orig}` 上下面这些文件还没恢复成分支里的样子（没写回或没删掉），\
                 多半仍有程序占着它们：\n{}\n{}\n\
                 记录已保留，关掉占用的程序之后再点「继续」或「放弃」。",
                list_paths(&remaining),
                describe(root, &find_blockers(root, 5))
            ),
        ));
    }
    Ok(())
}

/// 磁盘上的 `p` 是不是某个基线 commit 里原样存着的字节（或者文件已经不在）。
pub(crate) fn is_baseline_bytes(repo: &str, bases: &[String], p: &str) -> Result<bool> {
    let path = Path::new(repo).join(p);
    let Ok(meta) = path.symlink_metadata() else {
        return Ok(true); // 磁盘上没有：按 HEAD 写回不会覆盖任何字节
    };
    if !meta.is_file() {
        return Ok(false);
    }
    // `--path` 让磁盘内容走与 `git add` 相同的过滤器（svn_baseline.rs 记过这个坑）
    let disk = run_git(repo, &["hash-object", &format!("--path={p}"), "--", p])?;
    for b in bases {
        let (code, blob) =
            run_git_code(repo, &["rev-parse", "-q", "--verify", &format!("{b}:{p}")])?;
        if code == 0 && blob.trim() == disk.trim() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// 拿锁并在锁内重读 journal。
fn take_over(repo: &str) -> Result<(svn_lock::SyncLock, Journal)> {
    let first = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    match first.flow {
        // 手动跟进的冲突出口沿用这里的继续 / 放弃（计划 Task 13）；差别只在恢复时不走 S3–S7，
        // 见 `resume_stashed`
        Flow::SyncIn | Flow::FollowManual => {}
        Flow::SyncOut => return Err(sync_out_unsupported()),
        Flow::Follow => return Err(follow_unsupported()),
    }
    let lock = svn_lock::acquire(repo, &first.opid)?;
    // 拿锁之前那份只用来取 opid；两次读之间可能被另一个进程推进过
    let j = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    Ok((lock, j))
}

/// A11：从 `stashed` 接着走之前，确认中断期间没人动过。
///
/// 手动跟进（`Flow::FollowManual`）的基线早就发布了，这里直接接到 S8 之后：套用 S3–S7 等于切基线、
/// 跑 `svn update`，而它恰恰不做这两件事。它的 `baselineOidBefore` 是分支落后的那个旧基线，HEAD
/// 被外部切到新基线上时下面照样报「基线分支的位置变了」，不用单独分支。
fn resume_stashed(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<SyncInOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let base = need(&j.baseline_oid_before, "baselineOidBefore")?;
    match current_branch(repo)?.as_deref() {
        Some(b) if b == orig => {
            if rev_parse(repo, "HEAD")? != orig_head {
                return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
            }
            if !whitelist_clean(repo, cfg)? {
                return Err(state_changed(
                    "中断之后白名单里又有了新的改动（它们不在本次同步的 stash 里）",
                ));
            }
        }
        Some(b) if b == BASELINE_BRANCH => {
            if rev_parse(repo, "HEAD")? != base {
                return Err(state_changed("基线分支的位置变了"));
            }
        }
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是原分支 `{orig}` 也不是基线分支"
            )))
        }
    }
    if j.flow == Flow::FollowManual {
        return from_baseline_ready(repo, cfg, j);
    }
    from_stashed(repo, cfg, j, false)
}

fn continue_rebase(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<SyncInOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    if !rebase_in_progress(repo)?
        && current_branch(repo)?.as_deref() == Some(orig.as_str())
        && rev_parse(repo, "HEAD")? == orig_head
    {
        // S9 没有发生过：rebase 启动就失败（退出 1、不留 rebase 目录、HEAD 不动——
        // PR #49 review #6 实测），或者用户在外面 `rebase --abort` 了。两者现场相同，
        // 都从 S8 之后重来；那条路径自带分支 / 位置 / 残留 / 线性复核
        return from_baseline_ready(repo, cfg, j);
    }
    if rebase_in_progress(repo)? {
        require_our_rebase(repo, j)?;
        let files = unmerged_paths(repo)?;
        if !files.is_empty() {
            return Err(unresolved(&files));
        }
        // GIT_EDITOR=true：非交互地接受 git 给的提交信息（2026-09-25 实测可行；
        // 冲突被解成与新基线相同时，--empty=drop 让这一笔直接丢掉，退出 0）
        if let Err(e) = run_git_env(repo, &["rebase", "--continue"], &[("GIT_EDITOR", "true")]) {
            return conflict_or_error(repo, e);
        }
    }
    // 不论是我们续完的，还是用户在外面自己走完的，都必须落在同一个状态上
    if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
        return Err(state_changed(&format!("HEAD 不在原分支 `{orig}` 上")));
    }
    let base = published_baseline(repo, j)?;
    if !is_ancestor(repo, &base, "HEAD")? {
        return Err(state_changed(
            "原分支还没有落到新基线上（rebase 可能在外面被中止了）；\
             要回到更新之前，请点「放弃」",
        ));
    }
    finish_forward(repo, j)
}

/// `applying-stash` 停下之后锁是放开的：放回之前确认 HEAD 还是当初要放回的那个位置
/// （PR #49 review 第三轮 #1）。
///
/// 撞名挡住时一个字节都没放回、工作区干净，用户可以随手切走；不查的话 WIP 会被合进
/// 别的分支，接着 drop 掉备份、报成功。往前走要求原分支已落在新基线上（中间多了用户
/// 自己的提交不要紧）；往回走要求原分支仍在更新前的位置。
fn require_restore_target(repo: &str, j: &Journal) -> Result<()> {
    let orig = need(&j.orig_branch, "origBranch")?;
    if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
        return Err(state_changed(&format!("HEAD 不在原分支 `{orig}` 上")));
    }
    let ok = if j.rolling_back {
        rev_parse(repo, "HEAD")? == need(&j.orig_head, "origHead")?
    } else {
        let base = published_baseline(repo, j)?;
        is_ancestor(repo, &base, "HEAD")?
    };
    if !ok {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    Ok(())
}

/// 本次 S7 发布的基线 OID，并确认 `svn/base` 仍指着它（PR #49 review 第四轮 P1-1）。
///
/// S8 之后的每一步（重放目标、认 rebase、放回前复核）都要以它为准，而不是「此刻的
/// `svn/base`」：冲突期间锁是放开的，基线被外部挪走后，拿当前引用比什么都能对上。
/// 挪过就停下，不猜。
fn published_baseline(repo: &str, j: &Journal) -> Result<String> {
    let want = need(&j.new_baseline_oid, "newBaselineOid")?;
    let now = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    if now != want {
        return Err(state_changed(&format!(
            "基线分支 `{BASELINE_BRANCH}` 被移动过（本次发布的是 {want}，现在是 {now}）"
        )));
    }
    Ok(want)
}

/// A14：冲突期间锁是放开的，先确认进行中的正是本次同步发起的 rebase。
///
/// 只比分支名不够（PR #49 review 第三轮 #2）：用户中止本次 rebase 后又在同一条分支上
/// rebase 到别处，head-name 一模一样。本次的 rebase 由三样东西确定：重放的分支、
/// 起点（S9 时 HEAD 就是 `origHead`）、目标（`svn/base`）。2026-09-26 实测
/// `rebase-merge/` 下的 `orig-head` 与 `onto` 就是这两个完整 OID。
pub(crate) fn require_our_rebase(repo: &str, j: &Journal) -> Result<()> {
    let onto = published_baseline(repo, j)?;
    if !is_our_rebase(repo, j, &onto)? {
        return Err(state_changed("进行中的 rebase 不是本次同步发起的那一个"));
    }
    Ok(())
}

/// [`require_our_rebase`] 的判定部分。`sync_out` 用自己的基线核对与报错码调它
/// （Task 8–10 里程碑 review 第 5 / 6 条）。
pub(crate) fn is_our_rebase(repo: &str, j: &Journal, onto: &str) -> Result<bool> {
    let want_head = format!("refs/heads/{}", need(&j.orig_branch, "origBranch")?);
    let want_orig = need(&j.orig_head, "origHead")?;
    Ok(
        rebase_state(repo, "head-name")?.as_deref() == Some(want_head.as_str())
            && rebase_state(repo, "orig-head")?.as_deref() == Some(want_orig.as_str())
            && rebase_state(repo, "onto")?.as_deref() == Some(onto),
    )
}

/// `.git/rebase-merge/<name>`（或 `rebase-apply/`）的内容。
fn rebase_state(repo: &str, name: &str) -> Result<Option<String>> {
    let d = git_dir(repo)?;
    for sub in ["rebase-merge", "rebase-apply"] {
        let p = d.join(sub).join(name);
        if p.exists() {
            return Ok(Some(std::fs::read_to_string(p)?.trim().to_string()));
        }
    }
    Ok(None)
}

pub(crate) fn is_ancestor(repo: &str, a: &str, b: &str) -> Result<bool> {
    let (code, _) = run_git_code(repo, &["merge-base", "--is-ancestor", a, b])?;
    match code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(GitError::new(
            "sync_in_state_changed",
            format!("判断 {a} 是否在 {b} 的历史里失败了（merge-base 退出 {code}）"),
        )),
    }
}

fn whitelist_clean(repo: &str, cfg: &SvnConfig) -> Result<bool> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(false); // 空 pathspec 会变成「全部」（§12.2），宁可判不干净
    }
    let mut args: Vec<&str> = vec!["status", "--porcelain", "--"];
    args.extend(roots.iter().map(String::as_str));
    Ok(run_git(repo, &args)?.trim().is_empty())
}

fn require_on_branch(repo: &str, name: &str) -> Result<()> {
    if current_branch(repo)?.as_deref() != Some(name) {
        return Err(state_changed(&format!("HEAD 不在 `{name}` 上")));
    }
    Ok(())
}

fn require_on_old_baseline(repo: &str, j: &Journal) -> Result<()> {
    require_on_branch(repo, BASELINE_BRANCH)?;
    let base = need(&j.baseline_oid_before, "baselineOidBefore")?;
    if rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))? != base {
        return Err(state_changed("基线分支已经不是更新前的位置"));
    }
    Ok(())
}

pub(crate) fn no_unfinished() -> GitError {
    GitError::new(
        "sync_no_unfinished",
        "没有未收尾的同步记录，不需要继续或放弃。",
    )
}

fn follow_unsupported() -> GitError {
    GitError::new(
        "sync_follow_not_supported",
        "这条记录是「跟进」流程留下的。请用顶部横幅上的「继续」把它走完，不要手工去动分支指针。",
    )
}

fn sync_out_unsupported() -> GitError {
    GitError::new(
        "sync_out_not_supported",
        "这条记录是「交付」流程留下的。请用顶部横幅上交付那一组按钮处理，不要手工去动分支指针。",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 更新与交付的「录制期间 revision 被挪了」报错都靠它拼 `r前 → r后`，两个数不能写反。
    #[test]
    fn revision_moved_reads_before_then_now() {
        let moved = RevisionMoved { before: 5, now: 6 };
        assert_eq!(moved.to_string(), "r5 → r6");
    }

    fn entry(item: &str, revision: Option<i64>, schedule: Option<&str>) -> StatusEntry {
        StatusEntry {
            path: "Code/New.cs".to_string(),
            item: item.to_string(),
            props: "none".to_string(),
            wc_locked: false,
            revision,
            copied: false,
            schedule: schedule.map(str::to_string),
        }
    }

    /// 2026-11-26 现场：已 `svn add` 的新文件被切到基线删掉，不是基线失同步。
    #[test]
    fn s4_lets_an_added_file_that_left_the_disk_through() {
        assert!(!s4_counts_as_modified(&entry(
            "missing",
            Some(-1),
            Some("add")
        )));
    }

    /// 放过的只有「add 后不见了」。其余几种 `missing` 都可能是服务器上的内容丢了，照旧拦。
    #[test]
    fn s4_still_stops_on_every_other_kind_of_missing() {
        // 受版本控制的文件被删
        assert!(s4_counts_as_modified(&entry("missing", Some(1), None)));
        // `svn rm X` 后同名再加、又删掉：服务器上有旧内容
        assert!(s4_counts_as_modified(&entry(
            "missing",
            Some(-1),
            Some("replace")
        )));
        // `svn info` 没说清
        assert!(s4_counts_as_modified(&entry("missing", Some(-1), None)));
        // 磁盘上还在的已添加文件：不是分支带来的
        assert!(s4_counts_as_modified(&entry("added", Some(-1), None)));
    }
}
