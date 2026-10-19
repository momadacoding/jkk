use crate::engine::conflicts::{self, ConflictInfo};
use crate::error::Result;

/// 冲突文件的 stage 信息与五种情况的分类（spec §4.1）。没有冲突时返回空列表。
#[tauri::command(async)]
pub fn read_conflicts(path: String) -> Result<Vec<ConflictInfo>> {
    conflicts::read_conflicts(&path)
}
