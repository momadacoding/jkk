use crate::engine::{custom_config, launcher};
use crate::error::{GitError, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 配置文件路径：`app_config_dir()/customCommands.json`。
///
/// `app_config_dir()` 解析成 `config_dir()/${identifier}`，
/// Windows 上即 `%APPDATA%\com.momadacoding.jkk\`。
fn config_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| GitError::new("custom_config", format!("无法定位配置目录：{e}")))?;
    Ok(dir.join("customCommands.json"))
}

#[tauri::command]
pub fn read_custom_commands(app: AppHandle) -> Result<Vec<custom_config::CustomCommand>> {
    custom_config::load_from(&config_path(&app)?)
}

/// 按前端算好的 argv 启动外部程序。
///
/// 后端不认识占位符：展开在 `src/engine/customCommands.ts` 里做完了。
#[tauri::command]
pub fn run_custom_command(program: String, args: Vec<String>, cwd: Option<String>) -> Result<()> {
    launcher::spawn_detached(&program, &args, cwd.as_deref())
}

/// 用系统默认程序打开配置文件。
///
/// 故意不接受路径参数：先自己解析位置、确保文件存在（缺失时写默认预设），
/// 前端因此没有能力让系统去打开一个它自己拼出来的路径。
#[tauri::command]
pub fn open_custom_commands_config(app: AppHandle) -> Result<()> {
    let path = config_path(&app)?;
    custom_config::load_from(&path)?;
    let text = path
        .to_str()
        .ok_or_else(|| GitError::new("custom_config", "配置路径不是合法的 UTF-8"))?;
    launcher::open_in_default_app(text)
}
