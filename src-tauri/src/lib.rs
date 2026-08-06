pub mod commands;
pub mod engine;
pub mod error;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .setup(|app| {
      if cfg!(debug_assertions) {
        app.handle().plugin(
          tauri_plugin_log::Builder::default()
            .level(log::LevelFilter::Info)
            .build(),
        )?;
      }
      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      commands::diff::read_diff,
      commands::diff::read_status,
      commands::graph::load_graph,
      commands::ops::checkout,
      commands::ops::commit,
      commands::ops::create_branch,
      commands::ops::delete_branch,
      commands::ops::rename_branch,
      commands::ops::stage,
      commands::ops::unstage,
      commands::remote::fetch,
      commands::remote::pull,
      commands::remote::push,
      commands::repo::open_repo,
      commands::repo::read_upstream,
      commands::repo::repo_state,
      commands::watch::unwatch_repo,
      commands::watch::watch_repo,
      commands::worktree::create_worktree,
      commands::worktree::delete_worktree,
      commands::worktree::read_worktrees_cmd,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
