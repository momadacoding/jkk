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

#[tauri::command(async)]
pub fn read_diff(
    path: String,
    scope: String,
) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    backend_for(&path)?.diff(&scope)
}
