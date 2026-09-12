use crate::engine;
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn open_repo(path: String) -> Result<crate::engine::repo::RepoInfo, crate::error::GitError> {
    backend_for(&path)?.open()
}

#[tauri::command(async)]
pub fn read_upstream(
    path: String,
) -> Result<crate::engine::upstream::UpstreamInfo, crate::error::GitError> {
    engine::upstream::read(&path)
}

#[tauri::command(async)]
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
