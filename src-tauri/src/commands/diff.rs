use crate::engine;

#[tauri::command]
pub fn read_status(
    path: String,
) -> Result<crate::engine::diff::StatusData, crate::error::GitError> {
    engine::diff::read_status(&path)
}

#[tauri::command]
pub fn read_diff(
    path: String,
    scope: String,
) -> Result<crate::engine::diff::DiffData, crate::error::GitError> {
    engine::diff::read_diff(&path, scope)
}
