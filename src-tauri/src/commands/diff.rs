use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn read_status(
    path: String,
) -> Result<crate::engine::diff::StatusData, crate::error::GitError> {
    backend_for(&path)?.status()
}

#[tauri::command(async)]
pub fn read_diff(
    path: String,
    scope: String,
) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    backend_for(&path)?.diff(&scope)
}
