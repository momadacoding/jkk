use crate::engine::perf;

/// 前端批量送来的记录（`src/store/perf.ts`）。逐行校验，坏行丢弃，见 [`perf::web_line`]。
///
/// 返回记录器在不在。带了 `--perf` 但记录文件没建成时是 `false`，前端据此关掉自己。
#[tauri::command(async)]
pub fn perf_append(lines: Vec<String>) -> bool {
    perf::append_web(&lines)
}
