use crate::engine;

#[tauri::command]
pub fn open_repo(path: String) -> Result<crate::engine::repo::RepoInfo, crate::error::GitError> {
    engine::repo::open(&path)
}
