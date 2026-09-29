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
use crate::engine::git_cli::{run_git, run_git_code, run_git_env};
use crate::engine::svn_baseline::{
    create_baseline_commit, git_dir, move_baseline_ref, parse_trailer, read_baseline,
    swap_index_to_tree, TempIndex, Trailer,
};
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_dirty::read_dirty;
use crate::engine::svn_include_check::{is_w_file, normalize_roots};
use crate::engine::svn_init::{rebuild_baseline, record_server_tree_into};
use crate::engine::svn_journal::{self, Flow, Journal, Phase};
use crate::engine::svn_lock;
use crate::engine::svn_preflight::{check_targets, read_node_rev, run_preflight};
use crate::engine::svn_sync_check::{
    assert_linear, check_sync_in, current_branch, list_paths, rev_parse, unfinished_error,
    unmerged_paths, SyncInPreview,
};
use crate::engine::svn_sync_stash::{self as wip, RestoreOutcome};
use crate::engine::svn_wc::enumerate_wcs;
use crate::engine::svn_xml::parse_update_status;
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

/// 计划 A16：工作副本被外部整体 update 到了比基线新的 rN。
///
/// 先用 Phase 1 的「重建基线」把 `svn/base` 录成服务器 rN 的原版（`svn cat -r BASE`，
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

fn on_update_failed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    stderr: &str,
) -> Result<SyncInOutcome> {
    rolled_back(
        repo,
        cfg,
        j,
        Disk::MaybeUpdated,
        update_failed_error(stderr),
    )
}

fn on_update_conflict(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    conflicts: &[String],
) -> Result<SyncInOutcome> {
    rolled_back(
        repo,
        cfg,
        j,
        Disk::MaybeUpdated,
        update_conflict_error(conflicts),
    )
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
            "切到基线之后，下面这些文件在 SVN 看来有修改——基线记的内容与工作副本对不上\
             （多半有人在外部改过）：\n{}\n基线没有被改动。请先处理失同步，再更新。",
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

/// S6–S7：录新基线并发布（spec §7.3 S7、§7.6 录制三阶段）。
pub(crate) fn capture_and_publish(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    j.phase = Phase::CapturingBaseline;
    j.capture_tree = None;
    j.new_baseline_oid = None;
    svn_journal::write(repo, j)?;

    // S6：P1–P4 与 nodeRevs 单值。S5 若没让 nodeRevs 收敛，在这里停（journal 留在
    // capturing-baseline，继续时从这里重来）
    let pre = run_preflight(repo, &cfg.whitelist)?;
    // §6.3.1：S7 从当前 HEAD 树（此刻就是旧基线）起步
    let idx = TempIndex::from_head(repo)?;
    let (tree, _explained) = record_server_tree_into(repo, cfg, &pre, &idx)?;
    // 与 R3c 同一道：录制期间有人在 TortoiseSVN 里 commit / update
    let now = read_node_rev(repo, &cfg.whitelist)?;
    if now != pre.node_rev {
        return Err(GitError::new(
            "sync_in_revision_moved",
            format!(
                "录新基线的过程中工作副本被外部改动了（r{} → r{now}）。基线没有推进，\
                 记录保留；外部操作结束后在顶部横幅上点「继续」。",
                pre.node_rev
            ),
        ));
    }
    // 创建 commit 前记录 captureTree（§7.6）
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree.clone());
    svn_journal::write(repo, j)?;
    let trailer = Trailer {
        url: pre.repos_root,
        rev: pre.node_rev,
        uuid: pre.uuid,
    };
    let commit = create_baseline_commit(repo, &tree, Some(&old), &trailer)?;
    // 移动 ref 前记录 newBaselineOid（§7.6）
    j.new_baseline_oid = Some(commit);
    svn_journal::write(repo, j)?;
    finish_publish(repo, j)
}

/// 发布 + 索引对齐 + `baseline-ready`。重开时也从这里续（§7.6 publishing-baseline）。
pub(crate) fn finish_publish(repo: &str, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    match read_baseline(repo)?.as_deref() {
        Some(cur) if cur == new => {} // 已发布（崩溃恢复时会走到）
        Some(cur) if cur == old => move_baseline_ref(repo, &new, Some(&old))?,
        other => {
            return Err(GitError::new(
                "svn_baseline_moved",
                format!(
                    "基线分支现在是 {other:?}，既不是更新前的 {old}，也不是本次录好的 {new}。\
                     有别的工具动过它，这里停下，不覆盖。"
                ),
            ))
        }
    }
    align_index(repo, &old, &new)?;
    j.phase = Phase::BaselineReady;
    svn_journal::write(repo, j)
}

/// S3 之后真实索引 == 旧基线树，而 HEAD 跟着 `svn/base` 移到了新 commit。
/// 不对齐的话 S8 切分支时，新旧两棵树的差异会被当成「已暂存的改动」带走。
fn align_index(repo: &str, old: &str, new: &str) -> Result<()> {
    let (code, _) = run_git_code(repo, &["diff-index", "--cached", "--quiet", new])?;
    if code == 0 {
        return Ok(()); // 已对齐
    }
    // CAS：索引必须仍是旧基线树，否则有别的 git 操作动过它，宁可停下也不覆盖
    swap_index_to_tree(repo, old, new)
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
    // 用户配置不能改变 rebase 的语义（2c 独立 review C1，实测）：
    // - `rebase.autoStash=true` 会把工作区里的任何残留收起、重放、再放回，退出 0——
    //   残留若是基线的字节，就悄悄撤掉了用户提交过的内容，而结局报 Done；
    // - `rebase.updateRefs=true` 会顺手移动指向被重放区间的其它分支。
    // 与 `--empty=drop` 同一个原则：不依赖默认值，也不依赖用户配置
    match run_git(
        repo,
        &[
            "-c",
            "rebase.autoStash=false",
            "-c",
            "rebase.updateRefs=false",
            "rebase",
            "--empty=drop",
            &new_base,
        ],
    ) {
        Ok(_) => finish_forward(repo, j),
        Err(e) => conflict_or_error(repo, e),
    }
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
    Ok(match restore_step(repo, j)? {
        RestoreStep::Finished => SyncInOutcome::Done {
            new_rev: baseline_rev(repo)?,
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
    Conflict {
        files: Vec<String>,
        oid: String,
    },
    Blocked {
        collisions: Vec<String>,
        oid: String,
    },
}

/// S10 + S11：取回 WIP、drop、删 journal。往前走与往回走（放弃）共用。
pub(crate) fn restore_step(repo: &str, j: &mut Journal) -> Result<RestoreStep> {
    let Some(oid) = j.stash_oid.clone() else {
        svn_journal::remove(repo)?; // 没有 WIP（计划 A7）
        return Ok(RestoreStep::Finished);
    };
    if !j.stash_applied {
        j.phase = Phase::ApplyingStash;
        svn_journal::write(repo, j)?;
        match wip::restore_wip(repo, &oid)? {
            RestoreOutcome::Applied => {}
            RestoreOutcome::Conflicted { files } => {
                j.stash_applied = true;
                svn_journal::write(repo, j)?;
                return Ok(RestoreStep::Conflict { files, oid });
            }
            RestoreOutcome::Blocked { collisions } => {
                return Ok(RestoreStep::Blocked { collisions, oid })
            }
        }
        // apply 成功之后、drop 之前落盘：崩在这里重开时只 drop，不再 apply（计划 A2）
        j.stash_applied = true;
        svn_journal::write(repo, j)?;
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

/// `git checkout` 之后核对它真的切干净了（计划 A15）。
///
/// 2026-09-25 实测：文件被以 share=Read 打开（Unity 的方式）时，checkout **退出 0**、
/// HEAD 已经切走，而那个文件还是原来的内容——git 只在 stderr 里打一行
/// `unable to unlink old`。所以退出码证明不了任何事，只能事后看。
pub(crate) fn checkout_verified(
    repo: &str,
    cfg: &SvnConfig,
    branch: &str,
    force: bool,
) -> Result<()> {
    let from = rev_parse(repo, "HEAD")?;
    let mut args: Vec<&str> = vec!["checkout"];
    if force {
        args.push("-f");
    }
    args.push(branch);
    let ran = run_git(repo, &args);
    let on_target = current_branch(repo)?.as_deref() == Some(branch);
    let mut stale = tracked_changes(repo, cfg)?;
    // 该删却没删掉的文件退出 0 地留下，成了未跟踪文件（PR #49 review #7）
    stale.extend(untracked_leftovers(repo, cfg, &[from])?);
    if on_target && stale.is_empty() {
        return Ok(());
    }
    let files = if stale.is_empty() {
        String::new()
    } else {
        format!("\n下面这些文件没能写进去或删掉：\n{}", list_paths(&stale))
    };
    let git_said = match ran {
        Err(e) => format!("\ngit 原话：{}", e.message.trim()),
        Ok(_) => String::new(),
    };
    let root = Path::new(repo);
    Err(GitError::new(
        "sync_in_checkout_failed",
        format!(
            "切到分支 `{branch}` 没有成功，多半是有程序占着文件（Unity / IDE）。{files}\n{}{git_said}",
            describe(root, &find_blockers(root, 5))
        ),
    ))
}

/// 白名单内被跟踪文件相对 HEAD 的改动（不含未跟踪）。
fn tracked_changes(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
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
fn untracked_leftovers(repo: &str, cfg: &SvnConfig, from: &[String]) -> Result<Vec<String>> {
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
/// 「base 文件被改了」，而是 SVN 不认识的文件。
pub(crate) fn dirty_w_files(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    let wcs = enumerate_wcs(repo, &cfg.whitelist)?;
    let targets = check_targets(&roots, &wcs);
    Ok(read_dirty(repo, &targets)?
        .into_iter()
        .filter(|e| is_w_file(&e.path, &roots, &cfg.extensions))
        .filter(|e| {
            !matches!(
                e.item.as_str(),
                "normal" | "unversioned" | "ignored" | "external" | "none"
            ) || e.props == "modified"
        })
        .map(|e| e.path)
        .collect())
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

fn baseline_rev(repo: &str) -> Result<u64> {
    let msg = run_git(
        repo,
        &[
            "log",
            "-1",
            "--format=%B",
            &format!("refs/heads/{BASELINE_BRANCH}"),
        ],
    )?;
    parse_trailer(&msg)
        .map(|t| t.rev)
        .ok_or_else(|| GitError::new("svn_baseline", "基线 commit 上读不到 git-svn-id"))
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

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
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
                Ok(back_outcome(restore_step(repo, j)?))
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
    Ok(back_outcome(step))
}

/// 往回走（放弃 / 回退）那一次 S10–S11 的结局。
fn back_outcome(step: RestoreStep) -> SyncInOutcome {
    match step {
        RestoreStep::Finished => SyncInOutcome::Aborted,
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
            // A5：能 -f 的前提是 W-file 在 svn 看来全部干净 = 磁盘上就是服务器内容
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
        }
        checkout_verified(repo, cfg, &orig, true)?;
    }
    // HEAD 仍在原分支上（S3 失败时 git 整体拒绝、一个字节没写——计划 A15）
    // 或刚切回来：都要求分支本身没被动过
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    // 上一次切回原分支可能「退出 0 却没切干净」（A15 / C1）；刚才那次 checkout 已经核对过
    settle_checkout_residue(repo, cfg, j, &orig)?;
    // 随 restore_step 进入 applying-stash 时一起落盘（PR #49 review #3）
    j.rolling_back = true;
    restore_step(repo, j)
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
    let stale = tracked_changes(repo, cfg)?;
    let left = untracked_leftovers(repo, cfg, &bases)?;
    if stale.is_empty() && left.is_empty() {
        return Ok(());
    }
    let mut foreign = Vec::new();
    for p in stale.iter().chain(&left) {
        if !is_baseline_bytes(repo, &bases, p)? {
            foreign.push(p.clone());
        }
    }
    if !foreign.is_empty() {
        return Err(GitError::new(
            "sync_in_unsafe_to_return",
            format!(
                "分支 `{orig}` 上下面这些文件的内容既不是分支里的、也不是基线里的——\
                 不知道是谁改的，不能覆盖：\n{}\n记录已保留。请先确认这些改动\
                 （需要的话自己备份），再点「继续」或「放弃」。",
                list_paths(&foreign)
            ),
        ));
    }
    if !stale.is_empty() {
        // 非空才调：空 pathspec 会变成「全部」（§12.2）
        let mut args: Vec<&str> = vec!["checkout", "HEAD", "--"];
        args.extend(stale.iter().map(String::as_str));
        run_git(repo, &args)?;
    }
    for p in &left {
        // 删不掉（仍被占着）不在这里报：下面的复核会指名它
        let _ = std::fs::remove_file(Path::new(repo).join(p));
    }
    let mut remaining = tracked_changes(repo, cfg)?;
    remaining.extend(untracked_leftovers(repo, cfg, &bases)?);
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
fn is_baseline_bytes(repo: &str, bases: &[String], p: &str) -> Result<bool> {
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
    if first.flow != Flow::SyncIn {
        return Err(sync_out_unsupported());
    }
    let lock = svn_lock::acquire(repo, &first.opid)?;
    // 拿锁之前那份只用来取 opid；两次读之间可能被另一个进程推进过
    let j = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    Ok((lock, j))
}

/// A11：从 `stashed` 接着走之前，确认中断期间没人动过。
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
fn require_our_rebase(repo: &str, j: &Journal) -> Result<()> {
    let want_head = format!("refs/heads/{}", need(&j.orig_branch, "origBranch")?);
    let want_orig = need(&j.orig_head, "origHead")?;
    let want_onto = published_baseline(repo, j)?;
    if rebase_state(repo, "head-name")?.as_deref() != Some(want_head.as_str())
        || rebase_state(repo, "orig-head")?.as_deref() != Some(want_orig.as_str())
        || rebase_state(repo, "onto")?.as_deref() != Some(want_onto.as_str())
    {
        return Err(state_changed("进行中的 rebase 不是本次同步发起的那一个"));
    }
    Ok(())
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

fn is_ancestor(repo: &str, a: &str, b: &str) -> Result<bool> {
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

fn no_unfinished() -> GitError {
    GitError::new(
        "sync_no_unfinished",
        "没有未收尾的同步记录，不需要继续或放弃。",
    )
}

fn sync_out_unsupported() -> GitError {
    GitError::new(
        "sync_out_not_supported",
        "这条记录是「交付」流程留下的，本版本还不能替你收尾（Phase 3 提供）。\
         在那之前请不要手工去动 stash 或分支指针。",
    )
}
