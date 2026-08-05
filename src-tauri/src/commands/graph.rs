use crate::engine;

#[tauri::command]
pub fn load_graph(path: String) -> Result<crate::engine::graph::GraphData, crate::error::GitError> {
    engine::graph::load(&path)
}
