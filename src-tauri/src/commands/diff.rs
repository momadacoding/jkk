use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn read_status(
    path: String,
) -> Result<crate::engine::diff::StatusData, crate::error::GitError> {
    backend_for(&path)?.status()
}

/// Stage / Unstage 之后的局部重读，见 `engine::diff::read_status_paths`。
#[tauri::command(async)]
pub fn read_status_paths(
    path: String,
    paths: Vec<String>,
) -> Result<crate::engine::diff::StatusPatch, crate::error::GitError> {
    backend_for(&path)?.status_paths(&paths)
}

/// JS 侧参数名：`{ path, scope, options }`。`options` 可以不传，就是原来的默认行为。
#[tauri::command(async)]
pub fn read_diff(
    path: String,
    scope: String,
    options: Option<crate::engine::diff::DiffViewOptions>,
) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    backend_for(&path)?.diff(&scope, &options.unwrap_or_default())
}

/// 提交 / range 里一个文件的 diff，`read_diff` 只列了文件（`summaryOnly`）时用。
/// JS 侧参数名：`{ path, scope, file, oldFile, options }`；`oldFile`、`options` 可以不传。
/// 只读对象库，不碰工作区，git 与 svn 混合仓库同一条路，所以不经过 `Vcs`（同 `commit_file_diff`）。
#[tauri::command(async)]
pub fn read_diff_file(
    path: String,
    scope: String,
    file: String,
    old_file: Option<String>,
    options: Option<crate::engine::diff::DiffViewOptions>,
) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    crate::engine::diff::read_diff_file(
        &path,
        &scope,
        &file,
        old_file.as_deref(),
        &options.unwrap_or_default(),
    )
}
