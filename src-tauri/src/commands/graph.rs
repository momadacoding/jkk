use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn load_graph(path: String) -> Result<crate::engine::graph::GraphData, crate::error::GitError> {
    backend_for(&path)?.graph()
}
