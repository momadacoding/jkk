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
use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_baseline::{
    create_baseline_commit, git_dir, move_baseline_ref, parse_trailer, read_baseline,
    swap_index_to_tree, TempIndex, Trailer,
};
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_dirty::read_dirty;
use crate::engine::svn_include_check::{is_w_file, normalize_roots};
use crate::engine::svn_init::record_server_tree_into;
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
}

/// 一次完整的更新。
pub fn run_sync_in(repo: &str, expect: &SyncInExpect) -> Result<SyncInOutcome> {
    let opid = new_opid();
    let _lock = svn_lock::acquire(repo, &opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    // §7.2 第 7 条：锁内复核。预览与确认之间可能过了几分钟，什么都可能变
    let p = check_sync_in(repo)?;
    verify_expect(&p, expect)?;
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
    if p.orig_head != e.orig_head || p.baseline_oid != e.baseline_oid {
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

/// S3 起。`fresh` 为真表示本进程一路从 S0 走到这里（Task 7 的回退据此选前提）。
pub(crate) fn from_stashed(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    fresh: bool,
) -> Result<SyncInOutcome> {
    // S3
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

// —— S3 / S4 / S5 的失败出口。本 Task 只如实报错、journal 原样保留；
//    Task 7 把它们换成「回到原分支并取回 WIP」。——

fn on_checkout_failed(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    e: GitError,
) -> Result<SyncInOutcome> {
    Err(e)
}

fn on_stale_baseline(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    _fresh: bool,
    dirty: &[String],
) -> Result<SyncInOutcome> {
    Err(stale_error(dirty))
}

fn on_update_failed(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    stderr: &str,
) -> Result<SyncInOutcome> {
    Err(update_failed_error(stderr))
}

fn on_update_conflict(
    _repo: &str,
    _cfg: &SvnConfig,
    _j: &mut Journal,
    conflicts: &[String],
) -> Result<SyncInOutcome> {
    Err(update_conflict_error(conflicts))
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
    // S9：对新基线再复核一次（§7.3.2）
    let new_base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    assert_linear(repo, &new_base, &orig_head)?;
    j.phase = Phase::Rebase;
    svn_journal::write(repo, j)?;
    match run_git(repo, &["rebase", "--empty=drop", BASELINE_BRANCH]) {
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
            "把分支重放到新基线上失败了，而且不是冲突。基线已经推进，记录保留。git 原话：{}",
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
    let mut args: Vec<&str> = vec!["checkout"];
    if force {
        args.push("-f");
    }
    args.push(branch);
    let ran = run_git(repo, &args);
    let on_target = current_branch(repo)?.as_deref() == Some(branch);
    let stale = tracked_changes(repo, cfg)?;
    if on_target && stale.is_empty() {
        return Ok(());
    }
    let files = if stale.is_empty() {
        String::new()
    } else {
        format!("\n下面这些文件没能写进去：\n{}", list_paths(&stale))
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
        .filter(|e| e.item == "conflicted" || e.tree_conflicted)
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

fn new_opid() -> String {
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
