//! 外部 diff：把两侧准备成文件，交给配置里的工具打开。

use crate::engine::blob_export::{ensure_regular_file, export_filtered, side_file_name};
use crate::engine::external_launch::{
    expand_template, spawn_watched, try_lock, ExitReport, TempWorkspace,
};
use crate::engine::external_tools_config::{find_tool, ExternalToolsConfig};
use crate::error::{GitError, Result};
use git2::{Commit, ObjectType, Oid, Repository, Tree};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 要对比的是哪两侧（spec §4.3 的表）。
///
/// 前端发来的是 `{ kind: 'commit', oid, file, oldFile }` 这种形状：嵌套在参数里的字段
/// 不经过 Tauri 的 camelCase 转换，所以字段名要靠 `rename_all_fields` 自己转。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DiffTarget {
    /// index 对比工作区，同 `git difftool`。
    Unstaged { file: String },
    /// HEAD 对比 index，同 `git difftool --cached`。
    Staged { file: String },
    /// 第一父提交对比该提交；改名时 `old_file` 是父提交里的路径。
    Commit {
        oid: String,
        file: String,
        #[serde(default)]
        old_file: Option<String>,
    },
    /// 两个任意提交之间（SVN 节点）。
    Range {
        from: String,
        to: String,
        file: String,
        #[serde(default)]
        old_file: Option<String>,
    },
}

impl DiffTarget {
    /// 界面上标识这个文件用的路径（改名时是新路径），也是锁的 key。
    pub fn file(&self) -> &str {
        match self {
            DiffTarget::Unstaged { file }
            | DiffTarget::Staged { file }
            | DiffTarget::Commit { file, .. }
            | DiffTarget::Range { file, .. } => file,
        }
    }
}

/// 一侧的内容从哪来。
enum Side {
    /// git 里的 blob，连同它所在的路径（过滤器按路径匹配 `.gitattributes`）。
    Blob(Oid, String),
    /// 这一侧不存在（新增、删除、根提交），用空文件。
    Missing,
    /// 工作区里的真实文件，直接交给工具。
    Workdir(PathBuf),
}

fn blob_in_tree(tree: Option<&Tree>, rel: &str) -> Side {
    let entry = tree.and_then(|t| t.get_path(Path::new(rel)).ok());
    match entry {
        // 子模块之类不是 blob 的条目当作不存在：cat-file 导不出来
        Some(e) if e.kind() == Some(ObjectType::Blob) => Side::Blob(e.id(), rel.to_string()),
        _ => Side::Missing,
    }
}

fn index_side(repo: &Repository, rel: &str) -> Result<Side> {
    let index = repo.index()?;
    Ok(match index.get_path(Path::new(rel), 0) {
        Some(entry) => Side::Blob(entry.id, rel.to_string()),
        None => Side::Missing,
    })
}

/// 仓库内的相对路径必须每一段都是普通名字：不能为空、不能是绝对路径或盘符、不能有 `..`。
///
/// 在任何 git2 查找之前调用：`Index::get_path` 拿到绝对路径时会直接 panic，不是返回错误。
pub(crate) fn check_rel_path(rel: &str) -> Result<()> {
    let bad = || {
        GitError::new(
            "external_tool_bad_path",
            format!("不是仓库内的相对路径：{rel}"),
        )
    };
    let path = Path::new(rel);
    if rel.is_empty() || path.has_root() {
        return Err(bad());
    }
    if path
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        Ok(())
    } else {
        Err(bad())
    }
}

/// 工作区那一侧：文件不存在时用空临时文件；存在就必须是普通文件。
fn workdir_side(repo: &Repository, rel: &str) -> Result<Side> {
    Ok(match workdir_file(repo, rel)? {
        Some(abs) => Side::Workdir(abs),
        None => Side::Missing,
    })
}

/// 工作区里的真实文件，交给外部工具直接读写；文件或它所在的目录不存在时返回 None。
///
/// 从 worktree 根往下逐段用 `symlink_metadata` 检查，**每一级目录**都不能是链接（junction 也算）：
/// 只查末级的话，父目录被换成指向仓库外的链接时，工具会打开、保存仓库外的同名文件。
/// 最后再核对规范化后的位置仍在 worktree 内，作为第二道防线。
pub(crate) fn workdir_file(repo: &Repository, rel: &str) -> Result<Option<PathBuf>> {
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::new("external_tool_export", "裸仓库没有工作区"))?;
    let mut abs: PathBuf = workdir.components().collect();
    let parts: Vec<_> = Path::new(rel).components().collect();
    for (i, part) in parts.iter().enumerate() {
        abs.push(part);
        let meta = match std::fs::symlink_metadata(&abs) {
            Ok(m) => m,
            // 文件或它所在的目录不存在：这一侧是空的
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let last = i + 1 == parts.len();
        if !last && meta.file_type().is_symlink() {
            return Err(GitError::new(
                "external_tool_not_regular",
                format!("{} 是链接，不能把它下面的文件交给外部工具", abs.display()),
            ));
        }
    }
    ensure_regular_file(&abs)?;
    let root = std::fs::canonicalize(workdir)?;
    if !std::fs::canonicalize(&abs)?.starts_with(&root) {
        return Err(GitError::new(
            "external_tool_bad_path",
            format!("{} 不在当前 worktree 内", abs.display()),
        ));
    }
    Ok(Some(abs))
}

fn materialize(
    repo_path: &str,
    side: Side,
    name_rel: &str,
    tag: &str,
    ws: &TempWorkspace,
) -> Result<PathBuf> {
    match side {
        Side::Workdir(abs) => Ok(abs),
        Side::Missing => {
            let dest = ws.file(&side_file_name(name_rel, tag));
            std::fs::write(&dest, b"")?;
            Ok(dest)
        }
        Side::Blob(oid, rel) => {
            let dest = ws.file(&side_file_name(name_rel, tag));
            export_filtered(repo_path, &rel, &oid.to_string(), &dest)?;
            Ok(dest)
        }
    }
}

fn find_commit<'r>(repo: &'r Repository, spec: &str) -> Result<Commit<'r>> {
    Ok(repo.revparse_single(spec)?.peel_to_commit()?)
}

pub(crate) fn short(commit: &Commit) -> String {
    commit.id().to_string()[..7].to_string()
}

/// 准备两侧文件，返回 (左路径, 右路径, 左标题, 右标题)。
///
/// 临时文件都写进 `ws`；工作区那一侧直接返回真实文件路径，用户在工具里改了就是改了。
pub fn prepare_diff(
    repo_path: &str,
    target: &DiffTarget,
    ws: &TempWorkspace,
) -> Result<(PathBuf, PathBuf, String, String)> {
    check_rel_path(target.file())?;
    if let DiffTarget::Commit {
        old_file: Some(old),
        ..
    }
    | DiffTarget::Range {
        old_file: Some(old),
        ..
    } = target
    {
        check_rel_path(old)?;
    }
    let repo = Repository::open(repo_path)?;
    let (left, left_name, right, right_name, lt, rt) = match target {
        DiffTarget::Unstaged { file } => (
            index_side(&repo, file)?,
            file.as_str(),
            workdir_side(&repo, file)?,
            file.as_str(),
            format!("{file}（暂存区）"),
            format!("{file}（工作区）"),
        ),
        DiffTarget::Staged { file } => {
            let head = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
            (
                blob_in_tree(head.as_ref(), file),
                file.as_str(),
                index_side(&repo, file)?,
                file.as_str(),
                format!("{file}（HEAD）"),
                format!("{file}（暂存区）"),
            )
        }
        DiffTarget::Commit {
            oid,
            file,
            old_file,
        } => {
            let commit = find_commit(&repo, oid)?;
            let parent = if commit.parent_count() > 0 {
                Some(commit.parent(0)?)
            } else {
                None
            };
            let old = old_file.as_deref().unwrap_or(file);
            let parent_tree = parent.as_ref().map(|c| c.tree()).transpose()?;
            let left_title = match &parent {
                Some(p) => format!("{old}（{}）", short(p)),
                None => format!("{old}（无父提交）"),
            };
            let summary = commit.summary().unwrap_or("");
            (
                blob_in_tree(parent_tree.as_ref(), old),
                old,
                blob_in_tree(Some(&commit.tree()?), file),
                file.as_str(),
                left_title,
                format!("{file}（{} {summary}）", short(&commit)),
            )
        }
        DiffTarget::Range {
            from,
            to,
            file,
            old_file,
        } => {
            let from = find_commit(&repo, from)?;
            let to = find_commit(&repo, to)?;
            let old = old_file.as_deref().unwrap_or(file);
            (
                blob_in_tree(Some(&from.tree()?), old),
                old,
                blob_in_tree(Some(&to.tree()?), file),
                file.as_str(),
                format!("{old}（{}）", short(&from)),
                format!("{file}（{}）", short(&to)),
            )
        }
    };
    let left = materialize(repo_path, left, left_name, "LOCAL", ws)?;
    let right = materialize(repo_path, right, right_name, "REMOTE", ws)?;
    Ok((left, right, lt, rt))
}

pub(crate) fn path_arg(path: &Path) -> Result<String> {
    path.to_str().map(str::to_string).ok_or_else(|| {
        GitError::new(
            "external_tool_export",
            format!("路径不是合法的 UTF-8：{}", path.display()),
        )
    })
}

/// 用 `tool_id` 指定的工具打开外部 diff。
///
/// 顺序是锁 → 临时目录 → 导出 → 展开参数 → 启动；进程退出后先删临时目录、放锁，
/// 再调用 `on_exit`（命令层在这里发事件，前端收到时文件已经可以再次打开）。
/// 任何一步失败都当场返回：锁随 guard 释放，临时目录随 `TempWorkspace` 的 Drop 删除。
pub fn open_external_diff(
    repo_path: &str,
    cfg: &ExternalToolsConfig,
    config_path: &Path,
    tool_id: &str,
    target: &DiffTarget,
    on_exit: impl FnOnce(ExitReport) + Send + 'static,
) -> Result<()> {
    let tool = find_tool(cfg, tool_id)?;
    let template = tool.diff_args.as_ref().ok_or_else(|| {
        GitError::new(
            "external_tool_unsupported",
            format!("{} 没有配置 diffArgs，不能用来对比", tool.label),
        )
    })?;
    let lock = try_lock(repo_path, target.file(), &tool.label)?;
    let ws = TempWorkspace::create()?;
    let (left, right, left_title, right_title) = prepare_diff(repo_path, target, &ws)?;
    let args = expand_template(
        template,
        &[
            ("LOCAL", path_arg(&left)?),
            ("REMOTE", path_arg(&right)?),
            ("LOCAL_TITLE", left_title),
            ("REMOTE_TITLE", right_title),
        ],
    );
    spawn_watched(&tool.program, &args, move |report| {
        // 清理失败（例如工具还占着文件）只留下临时目录，不影响结果上报
        let _ = ws.remove();
        drop(lock);
        on_exit(report);
    })
    .map_err(|e| launcher_error(e, config_path))
}

/// `launcher` 错误补上配置文件路径：用户要去 externalTools.json 改 program，不是 customCommands.json。
pub fn launcher_error(e: GitError, config_path: &Path) -> GitError {
    if e.code == "launcher" {
        GitError::new(
            "launcher",
            format!(
                "{}，请检查外部工具配置 {}",
                e.message,
                config_path.display()
            ),
        )
    } else {
        e
    }
}
