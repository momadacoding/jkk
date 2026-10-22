use crate::engine::conflicts::{self, ConflictInfo};
use crate::error::Result;

/// 冲突文件的 stage 信息与五种情况的分类（spec §4.1）。没有冲突时返回空列表。
#[tauri::command(async)]
pub fn read_conflicts(path: String) -> Result<Vec<ConflictInfo>> {
    conflicts::read_conflicts(&path)
}

/// ③ 一边删一边改的二选一：`keep` 为真保留修改后的文件，否则删除。立即暂存。
#[tauri::command(async)]
pub fn resolve_delete_modify(path: String, file: String, keep: bool) -> Result<()> {
    conflicts::resolve_delete_modify(&path, &file, keep)
}

/// 本机 merge driver 配置检查（spec §4.6）。不报错：检查失败写在 `incomplete` 里。
#[tauri::command(async)]
pub fn check_merge_drivers(path: String) -> crate::engine::merge_drivers::DriverReport {
    crate::engine::merge_drivers::check_merge_drivers(&path)
}
