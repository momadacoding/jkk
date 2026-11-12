pub mod build_info;
pub mod commands;
pub mod desktop;
pub mod dev_profile;
pub mod engine;
pub mod error;
pub mod vcs;

use tauri::Manager;

/// 在 `app_log_dir()/perf/` 新建这次启动的记录文件，写文件头（设计 §4.1）。
///
/// 建不出来不能让应用起不来，但要让用户知道：这时日志插件还没装（release 根本不装），
/// 光 `log::warn!` 没人看得见（review 第 3 条），所以弹一个不阻塞的提示框。标题不带 `[perf]`，
/// 前端第一次 `perf_append` 拿到 `false` 后关掉自己。
fn open_perf_log<R: tauri::Runtime>(app: &tauri::App<R>, extra_args: usize) {
    let opened = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("取不到日志目录：{e}"))
        .and_then(|dir| {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let path = dir
                .join("perf")
                .join(engine::perf::file_name(secs, std::process::id()));
            engine::perf::PerfLog::create(path.clone())
                .map_err(|e| format!("建不出 {}：{e}", path.display()))
        });
    match opened {
        Ok(log) => {
            engine::perf::install(log);
            engine::perf::record(&engine::perf::meta_record(extra_args, tauri::is_dev()));
        }
        Err(reason) => {
            log::warn!("性能记录没有打开：{reason}");
            use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
            app.dialog()
                .message(format!(
                    "这次启动带了 --perf，但记录文件建不出来，性能记录没有打开。\n\n{reason}"
                ))
                .title("性能记录没有打开")
                .kind(MessageDialogKind::Warning)
                .show(|_| {});
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `pnpm tauri dev` 用 `<identifier>.dev`：配置目录、WebView2 数据都与安装版分开，见 dev_profile。
    // 「dev」按 tauri::is_dev() 认（有没有 custom-protocol），不按 debug_assertions
    let mut context = tauri::generate_context!();
    let configured_identifier = context.config().identifier.clone();
    let identifier = dev_profile::runtime_identifier(&configured_identifier, tauri::is_dev());
    let seed_from_installed = identifier != configured_identifier;
    context.config_mut().identifier = identifier;

    // 性能记录模式只认启动参数（设计 §4.1）。main.rs 已经先分流了 TortoiseSVN 钩子调用
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let perf_on = engine::perf::flag_in(&args);
    let perf_extra_args = args
        .iter()
        .skip(1)
        .filter(|a| a.as_os_str() != engine::perf::FLAG)
        .count();

    let builder = tauri::Builder::default();
    // 全局单例：再次启动时唤起已有窗口（关到托盘的也拿出来），自己直接退出。
    // 托盘、全局快捷键、session.json、settings.json 都按「一台机器一个进程」设计，多开会互相覆盖。
    //
    // 必须是第一个插件：第二个实例要在装托盘、注册快捷键之前就退出（官方文档的要求）。
    // dev 不注册：插件按 identifier 认实例，几个 worktree 同时跑 dev 时它们共用
    // `.dev` 这一个 identifier，后起的会悄悄唤起先起的那个，看到的不是自己的代码。
    // TortoiseSVN 钩子不受影响：main.rs 在进 Tauri 之前就分流了。
    // 这几条由 tests/single_instance.rs 守着。
    //
    // 用运行期判断而不是 `#[cfg(...)]` 属性：后者让一种构建下的 clippy / cargo test 根本不编译
    // 这段，插件升级改了回调签名也要等打 tag 才炸
    #[cfg(desktop)]
    let builder = if tauri::is_dev() {
        builder
    } else {
        builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            desktop::show_main_window(app);
            // 第二个实例带着 --perf 来、本进程没开：它马上就退出了，记录模式不会打开。
            // 不在运行中途打开（设计 §4.1），只告诉用户怎么做
            if engine::perf::flag_in(&args) && !engine::perf::enabled() {
                log::warn!("已有 jkk 在运行，--perf 没有生效");
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                app.dialog()
                    .message(
                        "jkk 已经在运行，这次的 --perf 没有生效。\n\
                         要打开性能记录，先完全退出 jkk（包括托盘图标），再带 --perf 启动。",
                    )
                    .title("性能记录没有打开")
                    .kind(MessageDialogKind::Warning)
                    .show(|_| {});
            }
        }))
    };
    // 前端在页面脚本执行前就知道开没开，关闭时 invoke 从第一条起就原样转发（决策日志 D1）
    let builder = if perf_on {
        builder.plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("jkk-perf")
                .js_init_script(engine::perf::INIT_SCRIPT)
                .build(),
        )
    } else {
        builder
    };
    // dev 首次启动从安装版复制种子。必须是插件：要赶在建窗口、前端读配置之前，见 dev_profile::plugin
    let builder = if seed_from_installed {
        builder.plugin(dev_profile::plugin(configured_identifier))
    } else {
        builder
    };
    builder
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            if perf_on {
                open_perf_log(app, perf_extra_args);
            }
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            dev_profile::log_seed_outcome(app);
            // tauri.conf.json 里的 "jkk" 只是兜底，带版本的标题只能在运行期设。
            // 标题只是外观，设失败不能让应用起不来，所以不用 `?`
            if let Some(window) = app.get_webview_window("main") {
                let mut title = build_info::window_title();
                if engine::perf::enabled() {
                    title.push_str(" [perf]");
                }
                if let Err(e) = window.set_title(&title) {
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
            commands::ops::probe_remote_branch,
            commands::ops::forget_gone_remote_branch,
            commands::ops::check_remote_deletable,
            commands::ops::discard,
            commands::ops::rename_branch,
            commands::ops::revert,
            commands::ops::save_patch,
            commands::ops::stage,
            commands::ops::unstage,
            commands::perf::perf_append,
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
            commands::history::file_history,
            commands::history::blame_file,
            commands::history::commit_file_diff,
            commands::watch::unwatch_repo,
            commands::watch::watch_repo,
            commands::worktree::create_worktree,
            commands::worktree::delete_worktree,
            commands::worktree::read_worktrees_cmd,
        ])
        .run(context)
        .expect("error while running tauri application");
}
