//! 文件历史 / Blame / 单文件 diff 的命令入口，逻辑见 `engine::file_history`。
//!
//! 三个都只读，不经过 `trait Vcs`：SVN 混合仓库看的同样是 git 这边的历史（含基线提交），
//! 不需要后端分派。

use crate::engine::diff::{DiffData, DiffViewOptions};
use crate::engine::file_history::{self, BlameData, FileHistory};
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
