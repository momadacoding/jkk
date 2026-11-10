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
