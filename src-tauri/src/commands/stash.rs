use crate::engine::stash::StashEntry;
use crate::error::Result;
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn read_stashes(path: String) -> Result<Vec<StashEntry>> {
    backend_for(&path)?.list_stashes()
}

#[tauri::command(async)]
pub fn stash_push(path: String, message: Option<String>, include_untracked: bool) -> Result<()> {
    backend_for(&path)?.stash_push(message.as_deref(), include_untracked)
}

#[tauri::command(async)]
pub fn stash_apply(path: String, index: usize) -> Result<()> {
    backend_for(&path)?.stash_apply(index)
}

#[tauri::command(async)]
pub fn stash_pop(path: String, index: usize) -> Result<()> {
    backend_for(&path)?.stash_pop(index)
}

#[tauri::command(async)]
pub fn stash_drop(path: String, index: usize) -> Result<()> {
    backend_for(&path)?.stash_drop(index)
}
