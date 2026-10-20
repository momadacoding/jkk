pub mod build_info;
pub mod commands;
pub mod engine;
pub mod error;
pub mod vcs;

use tauri::Manager;

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
            // tauri.conf.json 里的 "jkk" 只是兜底，带版本的标题只能在运行期设。
            // 标题只是外观，设失败不能让应用起不来，所以不用 `?`
            if let Some(window) = app.get_webview_window("main") {
                if let Err(e) = window.set_title(&build_info::window_title()) {
                    log::warn!("设置窗口标题失败: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::diff::read_diff,
            commands::conflict::read_conflicts,
            commands::conflict::resolve_delete_modify,
            commands::custom::open_custom_commands_config,
            commands::custom::read_custom_commands,
            commands::custom::run_custom_command,
            commands::external::open_external_diff,
            commands::external::open_external_merge,
            commands::external::open_external_tools_config,
            commands::external::read_external_tools,
            commands::diff::read_status,
            commands::graph::load_graph,
            commands::ops::branch_delete_info,
            commands::ops::checkout,
            commands::ops::checkout_remote_branch,
            commands::ops::cherry_pick,
            commands::ops::commit,
            commands::ops::create_branch,
            commands::ops::create_tag,
            commands::ops::delete_branch,
            commands::ops::delete_remote_branch,
            commands::ops::delete_remote_ref,
            commands::ops::discard,
            commands::ops::rename_branch,
            commands::ops::revert,
            commands::ops::save_patch,
            commands::ops::stage,
            commands::ops::unstage,
            commands::pr::list_pull_requests,
            commands::remote::fetch,
            commands::remote::pull,
            commands::remote::push,
            commands::remote::push_tag,
            commands::repo::open_repo,
            commands::repo::probe_svn_environment,
            commands::svn::svn_check_baseline,
            commands::svn::svn_eol_status,
            commands::svn::svn_fetch_log,
            commands::svn::svn_init_hybrid,
            commands::svn::svn_migrate_eol,
            commands::svn::svn_pinned_externals,
            commands::svn::svn_preflight_check,
            commands::svn::svn_read_log,
            commands::svn::svn_rebuild_baseline,
            commands::svn::svn_root_revs,
            commands::svn::svn_server_audit,
            commands::svn::svn_sync_state,
            commands::svn_sync::svn_sync_in_preview,
            commands::svn_sync::svn_sync_in,
            commands::svn_sync::svn_sync_continue,
            commands::svn_sync::svn_sync_abort,
            commands::svn_sync::svn_sync_out_preview,
            commands::svn_sync::svn_sync_out,
            commands::svn_sync::svn_sync_out_continue,
            commands::svn_sync::svn_sync_out_abort,
            commands::svn::svn_sync_status,
            commands::repo::read_capabilities,
            commands::repo::read_upstream,
            commands::repo::repo_state,
            commands::session::load_session,
            commands::session::save_session,
            commands::stash::read_stashes,
            commands::stash::stash_apply,
            commands::stash::stash_drop,
            commands::stash::stash_pop,
            commands::stash::stash_push,
            commands::tree::list_tree,
            commands::tree::read_blob,
            commands::watch::unwatch_repo,
            commands::watch::watch_repo,
            commands::worktree::create_worktree,
            commands::worktree::delete_worktree,
            commands::worktree::read_worktrees_cmd,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
