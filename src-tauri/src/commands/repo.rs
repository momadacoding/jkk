use crate::engine;

#[tauri::command]
pub fn open_repo(path: String) -> Result<crate::engine::repo::RepoInfo, crate::error::GitError> {
    engine::repo::open(&path)
}

#[tauri::command]
pub fn repo_state(path: String) -> Result<String, crate::error::GitError> {
    let git_dir = std::path::Path::new(&path).join(".git");
    if git_dir.join("MERGE_HEAD").exists() {
        return Ok("merge".to_string());
    }
    if git_dir.join("rebase-apply").exists() || git_dir.join("rebase-merge").exists() {
        return Ok("rebase".to_string());
    }
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Ok("cherry-pick".to_string());
    }
    Ok("normal".to_string())
}
