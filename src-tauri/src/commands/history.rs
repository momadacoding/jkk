//! 文件历史 / Blame / 单文件 diff 的命令入口，逻辑见 `engine::file_history`。
//!
//! 三个都只读，不经过 `trait Vcs`：SVN 混合仓库看的同样是 git 这边的历史（含基线提交），
//! 不需要后端分派。

use crate::engine::diff::{DiffData, DiffViewOptions};
use crate::engine::file_history::{self, BlameData, FileHistory};
use crate::engine::svn_file_history::{
    Cursor, GroupSpec, OverlayResult, OverlayTarget, SvnHistoryPage,
};
use crate::error::GitError;

#[tauri::command(async)]
pub fn file_history(
    path: String,
    file: String,
    revs: Vec<String>,
) -> Result<FileHistory, GitError> {
    file_history::file_history(&path, &file, &revs)
}

/// `rev` 为 null 时 blame 工作区里的文件。
#[tauri::command(async)]
pub fn blame_file(path: String, file: String, rev: Option<String>) -> Result<BlameData, GitError> {
    file_history::blame_file(&path, &file, rev.as_deref())
}

/// 混合仓库：把历史里的基线提交展开成真实 SVN 提交，一次一页。
///
/// JS 侧参数名：`{ path, groups, hidden, cursor }`，`groups` 元素 `{ id, path, fileRev }`，
/// `cursor` 第一页传 null。SVN 出错放在结果的 `error` 里，不当成命令失败。
#[tauri::command(async)]
pub fn svn_file_history(
    path: String,
    groups: Vec<GroupSpec>,
    hidden: Vec<u64>,
    cursor: Option<Cursor>,
) -> Result<SvnHistoryPage, GitError> {
    crate::engine::svn_file_history::svn_file_history(&path, &groups, &hidden, cursor)
}

/// 混合仓库：一条 SVN 提交里这个文件的改动。
///
/// JS 侧参数名：`{ path, pegPath, pegRev, rev, options }`。`pegPath` / `pegRev` 取这条提交所在那组
/// 基线的路径和 `fileRev`。`options` 可以不传。
#[tauri::command(async)]
pub fn svn_rev_file_diff(
    path: String,
    peg_path: String,
    peg_rev: u64,
    rev: u64,
    options: Option<DiffViewOptions>,
) -> Result<DiffData, GitError> {
    crate::engine::svn_file_history::svn_rev_file_diff(
        &path,
        &peg_path,
        peg_rev,
        rev,
        &options.unwrap_or_default(),
    )
}

/// 混合仓库：Blame 里归到基线的行，去 `svn blame` 查真实作者和版本。
///
/// JS 侧参数名：`{ path, targets }`，`targets` 元素 `{ baselineId, origPath, fileRev }`。结果与
/// `targets` 同序，每项带 `kind`：`mapped` / `mismatch` / `failed` / `skipped`。
#[tauri::command(async)]
pub fn svn_blame_overlay(
    path: String,
    targets: Vec<OverlayTarget>,
) -> Result<Vec<OverlayResult>, GitError> {
    crate::engine::svn_file_history::svn_blame_overlay(&path, &targets)
}

/// JS 侧参数名：`{ path, rev, file, oldFile, options }`。`options` 可以不传。
#[tauri::command(async)]
pub fn commit_file_diff(
    path: String,
    rev: String,
    file: String,
    old_file: Option<String>,
    options: Option<DiffViewOptions>,
) -> Result<DiffData, GitError> {
    file_history::commit_file_diff_with(
        &path,
        &rev,
        &file,
        old_file.as_deref(),
        &options.unwrap_or_default(),
    )
}
