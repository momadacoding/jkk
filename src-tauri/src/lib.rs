pub mod commands;
pub mod engine;
pub mod error;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
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
      commands::repo::open_repo,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
