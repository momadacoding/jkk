//! 应用级设置（与仓库无关）：关窗是否进托盘、呼出主界面的全局快捷键。
//!
//! 落在 `app_config_dir()/settings.json`，与 `session.json` 同目录。

use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;
use tauri_plugin_global_shortcut::{Modifiers, Shortcut};

/// 默认的呼出快捷键。
///
/// 避开 `Ctrl+Shift+<字母>`：浏览器和 IDE 大量占用（`Ctrl+Shift+J` 是浏览器控制台），
/// 注册成全局快捷键后会把它们的功能整个吞掉。`Ctrl+Alt+J` 撞车的少，J 取自 jkk。
pub const DEFAULT_SHOW_SHORTCUT: &str = "Ctrl+Alt+J";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// 点窗口 × 时隐藏到托盘（`true`）还是直接退出（`false`）。
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
    /// 呼出主界面的全局快捷键；`None` 表示用户关掉了。
    ///
    /// 字段缺失取默认值，显式写 `null` 保持关闭——两者必须区分，
    /// 否则用户关掉的快捷键会在下次启动时被默认值「复活」。
    #[serde(default = "default_show_shortcut")]
    pub show_shortcut: Option<String>,
}

fn default_close_to_tray() -> bool {
    true
}

fn default_show_shortcut() -> Option<String> {
    Some(DEFAULT_SHOW_SHORTCUT.to_string())
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            close_to_tray: default_close_to_tray(),
            show_shortcut: default_show_shortcut(),
        }
    }
}

/// 读取设置；文件不存在或解析失败都退回默认值。
///
/// 和 `session.json` 同一取舍：这份文件只由设置框写，坏了只可能是写到一半被打断，
/// 退回默认值比把解析错误顶到启动路径上更有用。
pub fn load_from(path: &Path) -> AppSettings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

pub fn save_to(path: &Path, settings: &AppSettings) -> Result<()> {
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| GitError::new("settings", e.to_string()))?;
    crate::engine::session::write_atomic(path, &text)
}

/// 解析并校验一个快捷键字符串。
///
/// 解析器本身接受单键（`"J"`、`"F5"`）和只带 Shift 的组合，但这些注册成**全局**
/// 快捷键后，用户在任何程序里都再也打不出那个字符、按不出那个功能键，
/// 所以要求至少带 Ctrl / Alt / Super 之一。
pub fn validate_shortcut(text: &str) -> Result<Shortcut> {
    let shortcut = Shortcut::from_str(text)
        .map_err(|e| GitError::new("settings", format!("快捷键「{text}」无法识别：{e}")))?;
    if !shortcut
        .mods
        .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(GitError::new(
            "settings",
            format!("快捷键「{text}」至少要带 Ctrl、Alt 或 Win 中的一个"),
        ));
    }
    Ok(shortcut)
}
