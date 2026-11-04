//! 桌面外壳：托盘、关窗进托盘、全局快捷键呼出主界面。
//!
//! 只做窗口层面的事，不碰任何 git 状态。

use crate::engine::app_settings::{self, AppSettings};
use crate::error::{GitError, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Runtime, Window, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const MAIN_WINDOW: &str = "main";
const MENU_SHOW: &str = "tray-show";
const MENU_QUIT: &str = "tray-quit";

/// 运行期的设置状态。
///
/// `close_to_tray` 单独放在原子量里，**不能**和其余字段共用 `inner` 的锁：
/// 快捷键插件的 register/unregister 会把活儿派到主线程并阻塞等结果，
/// 而关窗事件就跑在主线程上。保存设置时若持锁注册快捷键，恰好撞上用户点 ×，
/// 主线程等锁、保存线程等主线程，两边一起卡死。
pub struct DesktopState {
    close_to_tray: AtomicBool,
    inner: Mutex<Inner>,
    path: PathBuf,
}

struct Inner {
    settings: AppSettings,
    /// 当前真正注册在系统里的快捷键。启动时注册失败（被别的程序占了）则为 `None`。
    registered: Option<Shortcut>,
    /// 最近一次注册失败的原因，交给设置框显示。
    shortcut_error: Option<String>,
}

/// 交给前端的设置视图。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsView {
    pub close_to_tray: bool,
    /// 托盘图标没建出来时为 `false`：这时关窗只能直接退出，设置框要把那一项置灰，
    /// 否则用户勾上并保存后什么都不会发生。
    pub tray_available: bool,
    pub show_shortcut: Option<String>,
    /// 已保存的快捷键此刻没注册上的原因；`None` 表示正常（或快捷键已关闭）。
    pub shortcut_error: Option<String>,
    pub default_shortcut: String,
}

impl DesktopState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        // 锁中毒只说明另一次保存 panic 了，设置本身没有跨字段不变量，拿回内层值继续用
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn view(&self) -> AppSettingsView {
        let inner = self.lock();
        view_of(&inner)
    }

    /// 应用新设置：先换快捷键，再落盘；任何一步失败都回到改动前的状态并报错。
    ///
    /// 新快捷键注册失败（多半是被别的程序占了）或写盘失败时，把旧的装回去，
    /// 磁盘上的设置保持不变——否则下次启动会去注册一个注定失败的快捷键，
    /// 或者设置框显示的是旧快捷键、实际生效的却是新的。
    pub fn apply<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        close_to_tray: bool,
        show_shortcut: Option<String>,
    ) -> Result<AppSettingsView> {
        let show_shortcut = show_shortcut
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let wanted = show_shortcut
            .as_deref()
            .map(app_settings::validate_shortcut)
            .transpose()?;

        let mut inner = self.lock();
        let old = inner.registered;
        let changed = wanted != old;
        if changed {
            let gs = app.global_shortcut();
            if let Some(o) = old {
                // 注销失败就别往下换：旧的还挂在系统里，再注册新的会出现两个组合都能呼出，
                // 之后想换回旧的又会撞上 AlreadyRegistered
                gs.unregister(o)
                    .map_err(|e| GitError::new("settings", format!("注销原来的快捷键失败：{e}")))?;
                inner.registered = None;
            }
            if let Some(new) = wanted {
                if let Err(e) = gs.register(new) {
                    restore_old(app, &mut inner, old);
                    return Err(register_error(show_shortcut.as_deref().unwrap_or(""), e));
                }
                inner.registered = Some(new);
            }
        }

        let next = AppSettings {
            close_to_tray,
            show_shortcut,
        };
        if let Err(e) = app_settings::save_to(&self.path, &next) {
            if changed {
                if let Some(new) = wanted {
                    if let Err(e) = app.global_shortcut().unregister(new) {
                        log::warn!("回滚时注销新快捷键失败: {e}");
                    }
                }
                restore_old(app, &mut inner, old);
            }
            return Err(e);
        }
        inner.shortcut_error = None;
        inner.settings = next;
        self.close_to_tray.store(close_to_tray, Ordering::SeqCst);
        Ok(view_of(&inner))
    }
}

/// 把改动前**真正注册着**的快捷键装回去。
///
/// 启动时就没注册上的（`old` 为 None）不重试，原来的启动错误原样保留；
/// 否则会拿同一个注定失败的组合再试一次，误报「原来的也没能恢复」。
fn restore_old<R: Runtime>(app: &AppHandle<R>, inner: &mut Inner, old: Option<Shortcut>) {
    let restored = old.filter(|o| app.global_shortcut().register(*o).is_ok());
    inner.registered = restored;
    if old.is_some() && restored.is_none() {
        inner.shortcut_error = Some("原来的快捷键也没能恢复，当前没有生效的快捷键".to_string());
    }
}

fn view_of(inner: &Inner) -> AppSettingsView {
    AppSettingsView {
        close_to_tray: inner.settings.close_to_tray,
        tray_available: !TRAY_MISSING.load(Ordering::SeqCst),
        show_shortcut: inner.settings.show_shortcut.clone(),
        shortcut_error: inner.shortcut_error.clone(),
        default_shortcut: app_settings::DEFAULT_SHOW_SHORTCUT.to_string(),
    }
}

fn register_error(text: &str, e: impl std::fmt::Display) -> GitError {
    GitError::new(
        "settings",
        format!("快捷键「{text}」注册失败，可能已被其他程序占用：{e}"),
    )
}

/// 在 `setup` 里调用：读设置、装快捷键插件、注册快捷键、建托盘。
///
/// 这里的失败都不该让应用起不来：快捷键被占、托盘建不出来，主窗口照样能用。
pub fn setup(app: &mut App) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let path = app.path().app_config_dir()?.join("settings.json");
    let settings = app_settings::load_from(&path);

    app.handle().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(|app, _shortcut, event| {
                // 只注册了一个快捷键，不用比对是哪个；只认按下，松开不再触发一次。
                //
                // 只呼出、不做「已在前台就隐藏」的切换：用户在设置框里重录同一个组合键时，
                // 系统会先把这次按键当成全局快捷键派过来，切换的话窗口当场消失
                if event.state() == ShortcutState::Pressed {
                    show_main_window(app);
                }
            })
            .build(),
    )?;

    let mut registered = None;
    let mut shortcut_error = None;
    if let Some(text) = settings.show_shortcut.as_deref() {
        match app_settings::validate_shortcut(text) {
            Ok(sc) => match app.global_shortcut().register(sc) {
                Ok(()) => registered = Some(sc),
                Err(e) => shortcut_error = Some(register_error(text, e).message),
            },
            Err(e) => shortcut_error = Some(e.message),
        }
    }
    if let Some(e) = &shortcut_error {
        log::warn!("{e}");
    }

    app.manage(DesktopState {
        close_to_tray: AtomicBool::new(settings.close_to_tray),
        inner: Mutex::new(Inner {
            settings,
            registered,
            shortcut_error,
        }),
        path,
    });

    if let Err(e) = build_tray(app) {
        // 没有托盘时关窗还藏起来，用户就只能靠快捷键找回窗口了；干脆退回「关窗即退出」
        log::warn!("创建托盘失败，关窗将直接退出: {e}");
        app.state::<DesktopState>()
            .close_to_tray
            .store(false, Ordering::SeqCst);
        TRAY_MISSING.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// 托盘没建出来。之后用户在设置里重新打开「关窗进托盘」也不能生效，见 `on_window_event`。
static TRAY_MISSING: AtomicBool = AtomicBool::new(false);

fn build_tray(app: &App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, MENU_SHOW, "显示主界面", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(crate::build_info::window_title())
        .menu(&menu)
        // 左键直接回到主界面，菜单留给右键
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            MENU_SHOW => show_main_window(app),
            MENU_QUIT => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// 窗口事件：关主窗口时按设置决定藏到托盘还是照常退出。
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != MAIN_WINDOW || TRAY_MISSING.load(Ordering::SeqCst) {
        return;
    }
    let Some(state) = window.try_state::<DesktopState>() else {
        return;
    };
    if state.close_to_tray.load(Ordering::SeqCst) {
        api.prevent_close();
        if let Err(e) = window.hide() {
            log::warn!("隐藏主窗口失败: {e}");
        }
    }
}

pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_focus();
}
