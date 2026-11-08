pub mod build_info;
pub mod commands;
pub mod desktop;
pub mod dev_profile;
pub mod engine;
pub mod error;
pub mod vcs;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // debug 构建用 `<identifier>.dev`：配置目录、WebView2 数据都与安装版分开，见 dev_profile
    let mut context = tauri::generate_context!();
    let installed_identifier = context.config().identifier.clone();
    context.config_mut().identifier =
        dev_profile::runtime_identifier(&installed_identifier, cfg!(debug_assertions));

    let builder = tauri::Builder::default();
    // 全局单例：再次启动时唤起已有窗口（关到托盘的也拿出来），自己直接退出。
    // 托盘、全局快捷键、session.json、settings.json 都按「一台机器一个进程」设计，多开会互相覆盖。
    //
    // 必须是第一个插件：第二个实例要在装托盘、注册快捷键之前就退出（官方文档的要求）。
    // 只在 release 构建注册：插件按 identifier 认实例，几个 worktree 同时跑 dev 时它们共用
    // `.dev` 这一个 identifier，后起的会悄悄唤起先起的那个，看到的不是自己的代码。
    // TortoiseSVN 钩子不受影响：main.rs 在进 Tauri 之前就分流了。
    // 这几条由 tests/single_instance.rs 守着。
    //
    // 用运行期的 `cfg!` 而不是 `#[cfg(not(debug_assertions))]`：后者让 debug 下的
    // clippy / cargo test 根本不编译这段，插件升级改了回调签名也要等打 tag 才炸
    #[cfg(desktop)]
    let builder = if cfg!(debug_assertions) {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            desktop::show_main_window(app);
        }))
    };
    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            // 必须在 desktop::setup 读 settings.json 之前
            dev_profile::seed_on_first_dev_launch(app, &installed_identifier);
            // tauri.conf.json 里的 "jkk" 只是兜底，带版本的标题只能在运行期设。
            // 标题只是外观，设失败不能让应用起不来，所以不用 `?`
            if let Some(window) = app.get_webview_window("main") {
                if let Err(e) = window.set_title(&build_info::window_title()) {
                    log::warn!("设置窗口标题失败: {e}");
                }
            }
            // 托盘和快捷键是附加能力，装不上也不能让应用起不来；
            // 这时设置框读不到状态会报错，关窗退回默认的直接退出
            if let Err(e) = desktop::setup(app) {
                log::warn!("初始化托盘 / 全局快捷键失败: {e}");
            }
            Ok(())
        })
        .on_window_event(desktop::on_window_event)
        .on_page_load(desktop::on_page_load)
        .invoke_handler(tauri::generate_handler![
            commands::app_settings::read_app_settings,
            commands::app_settings::save_app_settings,
            commands::app_settings::set_svn_auto_follow,
            commands::app_settings::set_shortcut_recording,
            commands::diff::read_diff,
            commands::conflict::check_merge_drivers,
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
            commands::diff::read_status_paths,
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
            commands::ops::prefetch_remote_default,
            commands::ops::prefetch_remote_default_for_ref,
            commands::ops::check_remote_deletable,
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
            commands::svn::svn_branches_behind,
            commands::svn::svn_check_baseline,
            commands::svn::svn_eol_status,
            commands::svn::svn_fetch_log,
            commands::svn::svn_follow_mark,
            commands::svn::svn_get_user,
            commands::svn::svn_init_hybrid,
            commands::svn::svn_migrate_eol,
            commands::svn::svn_pinned_externals,
            commands::svn::svn_preflight_check,
            commands::svn::svn_read_log,
            commands::svn::svn_rebuild_baseline,
            commands::svn::svn_root_revs,
            commands::svn::svn_server_audit,
            commands::svn::svn_set_user,
            commands::svn::tsvn_hooks_status,
            commands::svn::tsvn_hooks_register,
            commands::svn::tsvn_hooks_unregister,
            commands::svn::svn_sync_state,
            commands::svn::svn_wc_db_stamps,
            commands::svn_sync::svn_sync_in_preview,
            commands::svn_sync::svn_sync_in,
            commands::svn_sync::svn_sync_continue,
            commands::svn_sync::svn_sync_abort,
            commands::svn_sync::svn_sync_out_preview,
            commands::svn_sync::svn_sync_out,
            commands::svn_sync::svn_sync_out_continue,
            commands::svn_sync::svn_sync_out_abort,
            commands::svn_sync::svn_follow,
            commands::svn_sync::svn_follow_continue,
            commands::svn_sync::svn_follow_auto,
            commands::svn_sync::svn_follow_manual,
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
        .run(context)
        .expect("error while running tauri application");
}
