use crate::desktop::{AppSettingsView, DesktopState};
use crate::error::Result;
use tauri::{AppHandle, State};

#[tauri::command(async)]
pub fn read_app_settings(state: State<'_, DesktopState>) -> AppSettingsView {
    state.view()
}

/// 保存应用设置。`show_shortcut` 传 `null` 表示关掉快捷键。
///
/// 用 async 命令：换快捷键要派到主线程执行注册，放在工作线程上等，不占住主线程的事件循环。
/// （同步命令跑在主线程上时，插件会直接就地执行，不会自锁；选 async 只是不阻塞 UI。）
#[tauri::command(async)]
pub fn save_app_settings(
    app: AppHandle,
    state: State<'_, DesktopState>,
    close_to_tray: bool,
    show_shortcut: Option<String>,
) -> Result<AppSettingsView> {
    state.apply(&app, close_to_tray, show_shortcut)
}
