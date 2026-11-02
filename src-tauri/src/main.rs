// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // TortoiseSVN 把同一个 exe 当客户端钩子调起：在 Tauri 启动之前分流，不开窗口
    if let Some(code) =
        app_lib::engine::tsvn_hook_cli::tsvn_hook_main(std::env::args_os().collect())
    {
        std::process::exit(code);
    }
    app_lib::run();
}
