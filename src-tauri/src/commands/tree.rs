use crate::engine;

#[tauri::command(async)]
pub fn list_tree(
    path: String,
    rev: String,
) -> Result<crate::engine::tree::TreeListing, crate::error::GitError> {
    engine::tree::list_tree(&path, &rev)
}

#[tauri::command(async)]
pub fn read_blob(
    path: String,
    rev: String,
    file: String,
) -> Result<crate::engine::tree::BlobContent, crate::error::GitError> {
    engine::tree::read_blob(&path, &rev, &file)
}
