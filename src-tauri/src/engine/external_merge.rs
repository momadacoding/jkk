//! 外部 merge：冲突文件的三个版本导出成临时文件，连同工作区里的真实文件交给工具（spec §4.2）。
//!
//! 工具退出后不自动暂存，退出码也不用来判断冲突是否已解决——用户回到应用里看结果再手动 Stage。

use crate::engine::blob_export::{export_filtered, side_file_name};
use crate::engine::conflicts::{read_conflicts, ConflictCase, NotTextReason, StageInfo};
use crate::engine::external_diff::{check_rel_path, launcher_error, path_arg, short, workdir_file};
use crate::engine::external_launch::{
    expand_template, spawn_watched, try_lock, ExitReport, TempWorkspace,
};
use crate::engine::external_tools_config::{find_tool, ExternalToolsConfig};
use crate::error::{GitError, Result};
use git2::{BranchType, Oid, Repository, RepositoryState};
use std::path::{Path, PathBuf};

/// 四个位置在工具里显示的标题。各侧写真实名字，不写 ours / theirs。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeTitles {
    pub local: String,
    pub remote: String,
    pub base: String,
    pub merged: String,
}

#[derive(Debug, Clone)]
pub struct PreparedMerge {
    pub local: PathBuf,
    pub remote: PathBuf,
    pub base: PathBuf,
    /// 工作区里的真实文件，工具把合并结果存到这里；任何清理都不碰它
    pub merged: PathBuf,
    pub titles: MergeTitles,
}

/// 指着这个提交的分支名：先找本地分支，再找远程跟踪分支，同类里按名字取第一个。
fn branch_at(repo: &Repository, oid: Oid) -> Option<String> {
    for kind in [BranchType::Local, BranchType::Remote] {
        let mut names: Vec<String> = repo
            .branches(Some(kind))
            .ok()?
            .flatten()
            .filter(|(b, _)| b.get().target() == Some(oid))
            .filter_map(|(b, _)| b.name().ok().flatten().map(str::to_string))
            .collect();
        names.sort();
        if let Some(first) = names.into_iter().next() {
            return Some(first);
        }
    }
    None
}

fn name_or_short(repo: &Repository, oid: Oid) -> String {
    branch_at(repo, oid).unwrap_or_else(|| oid.to_string()[..7].to_string())
}

/// `<当前分支>（HEAD）`；detached 时用短 hash。
fn head_label(repo: &Repository) -> String {
    let name = match repo.head() {
        Ok(h) if h.is_branch() => h.shorthand().unwrap_or("HEAD").to_string(),
        Ok(h) => h
            .target()
            .map(|o| o.to_string()[..7].to_string())
            .unwrap_or_else(|| "HEAD".into()),
        Err(_) => "HEAD".into(),
    };
    format!("{name}（HEAD）")
}

/// `REBASE_HEAD 1a2b3c4 标题` 这种形状；读不到就只写名字，标题不值得让打开失败。
fn pseudo_ref_label(repo: &Repository, name: &str) -> String {
    match repo.revparse_single(name).and_then(|o| o.peel_to_commit()) {
        Ok(c) => format!("{name} {} {}", short(&c), c.summary().unwrap_or("")),
        Err(_) => name.to_string(),
    }
}

/// rebase 的新基线。状态文件在仓库自己的 gitdir 下（linked worktree 是
/// `.git/worktrees/<name>/`），`repo.path()` 已经指向那里。
fn rebase_onto(repo: &Repository) -> Option<Oid> {
    ["rebase-merge", "rebase-apply"].iter().find_map(|dir| {
        let text = std::fs::read_to_string(repo.path().join(dir).join("onto")).ok()?;
        Oid::from_str(text.trim()).ok()
    })
}

/// 按仓库当前停在的操作给各侧取名（spec §4.2「标题」）。rebase 和 merge 必须区分：
/// rebase 时 HEAD 是「上游 + 已经重放成功的提交」，不是用户自己的分支。
pub fn merge_titles(repo_path: &str) -> Result<MergeTitles> {
    // mergehead_foreach 要 &mut
    let mut repo = Repository::open(repo_path)?;
    let (local, remote) = match repo.state() {
        RepositoryState::Merge => {
            let mut heads = Vec::new();
            // 多个 MERGE_HEAD（octopus）时只取第一个
            let _ = repo.mergehead_foreach(|oid| {
                heads.push(*oid);
                false
            });
            let remote = match heads.first() {
                Some(oid) => name_or_short(&repo, *oid),
                None => "MERGE_HEAD".into(),
            };
            (head_label(&repo), remote)
        }
        RepositoryState::Rebase
        | RepositoryState::RebaseInteractive
        | RepositoryState::RebaseMerge
        | RepositoryState::ApplyMailboxOrRebase => {
            let onto = rebase_onto(&repo)
                .map(|oid| name_or_short(&repo, oid))
                .unwrap_or_else(|| "上游".into());
            (
                format!("当前重放结果（HEAD，基于 {onto}）"),
                pseudo_ref_label(&repo, "REBASE_HEAD"),
            )
        }
        RepositoryState::CherryPick | RepositoryState::CherryPickSequence => (
            head_label(&repo),
            pseudo_ref_label(&repo, "CHERRY_PICK_HEAD"),
        ),
        RepositoryState::Revert | RepositoryState::RevertSequence => {
            (head_label(&repo), pseudo_ref_label(&repo, "REVERT_HEAD"))
        }
        // stash apply / pop 的冲突不留状态文件，看不出对方是谁
        _ => (head_label(&repo), "对方".into()),
    };
    Ok(MergeTitles {
        local,
        remote,
        base: "共同祖先".into(),
        merged: "结果（工作区）".into(),
    })
}

fn export_stage(
    repo_path: &str,
    file: &str,
    stage: &Option<StageInfo>,
    tag: &str,
    ws: &TempWorkspace,
) -> Result<PathBuf> {
    let dest = ws.file(&side_file_name(file, tag));
    match stage {
        Some(s) => export_filtered(repo_path, file, &s.oid, &dest)?,
        // AA 没有共同祖先：BASE 用空文件
        None => std::fs::write(&dest, b"")?,
    }
    Ok(dest)
}

/// 导出 stage 2 / 3 / 1 并定位 `$MERGED`。
///
/// 只接 ① / ② / ④a / ④b：③（一边删一边改）在应用内二选一，④c（二进制）和 ⑤（结构冲突）
/// 没有可以逐行合并的东西。每次都重新读冲突信息，不信任前端手里可能已经过期的状态。
pub fn prepare_merge(repo_path: &str, file: &str, ws: &TempWorkspace) -> Result<PreparedMerge> {
    check_rel_path(file)?;
    let info = read_conflicts(repo_path)?
        .into_iter()
        .find(|c| c.path == file)
        .ok_or_else(|| {
            GitError::new(
                "conflict_state_changed",
                format!("{file} 已经不是冲突状态，请刷新"),
            )
        })?;
    let supported = match info.case {
        ConflictCase::Markers | ConflictCase::NoMarkers => true,
        ConflictCase::NotTextMerged => info.reason != Some(NotTextReason::BinaryContent),
        ConflictCase::Structural | ConflictCase::DeleteModify => false,
    };
    if !supported {
        return Err(GitError::new(
            "external_merge_unsupported",
            format!("{file} 的冲突（{}）不能交给外部合并工具", info.code),
        ));
    }
    let repo = Repository::open(repo_path)?;
    // 先确认 `$MERGED` 是工作区里的普通文件，再导出：拒绝时不留下临时文件
    let merged = workdir_file(&repo, file)?.ok_or_else(|| {
        GitError::new(
            "external_tool_not_regular",
            format!("工作区里没有 {file}，不能作为合并结果"),
        )
    })?;
    Ok(PreparedMerge {
        local: export_stage(repo_path, file, &info.ours, "LOCAL", ws)?,
        remote: export_stage(repo_path, file, &info.theirs, "REMOTE", ws)?,
        base: export_stage(repo_path, file, &info.base, "BASE", ws)?,
        merged,
        titles: merge_titles(repo_path)?,
    })
}

/// 用 `tool_id` 指定的工具打开外部 merge，流程与 `open_external_diff` 相同：
/// 锁 → 临时目录 → 导出 → 展开参数 → 启动；退出后删临时目录、放锁，再调用 `on_exit`。
pub fn open_external_merge(
    repo_path: &str,
    cfg: &ExternalToolsConfig,
    config_path: &Path,
    tool_id: &str,
    file: &str,
    on_exit: impl FnOnce(ExitReport) + Send + 'static,
) -> Result<()> {
    let tool = find_tool(cfg, tool_id)?;
    let template = tool.merge_args.as_ref().ok_or_else(|| {
        GitError::new(
            "external_tool_unsupported",
            format!("{} 没有配置 mergeArgs，不能用来合并", tool.label),
        )
    })?;
    let lock = try_lock(repo_path, file, &tool.label)?;
    let ws = TempWorkspace::create()?;
    let m = prepare_merge(repo_path, file, &ws)?;
    let args = expand_template(
        template,
        &[
            ("LOCAL", path_arg(&m.local)?),
            ("REMOTE", path_arg(&m.remote)?),
            ("BASE", path_arg(&m.base)?),
            ("MERGED", path_arg(&m.merged)?),
            ("LOCAL_TITLE", m.titles.local),
            ("REMOTE_TITLE", m.titles.remote),
            ("BASE_TITLE", m.titles.base),
            ("MERGED_TITLE", m.titles.merged),
        ],
    );
    spawn_watched(&tool.program, &args, move |report| {
        // 只删本次的临时子目录；`$MERGED` 在工作区里，不在删除范围内
        let _ = ws.remove();
        drop(lock);
        on_exit(report);
    })
    .map_err(|e| launcher_error(e, config_path))
}
