use crate::engine::external_diff::{self, DiffTarget};
use crate::engine::external_launch::exit_event;
use crate::engine::{external_merge, external_tools_config, launcher};
use crate::error::{GitError, Result};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

const CODE: &str = "external_tools_config";

/// 配置文件路径：`app_config_dir()/externalTools.json`，
/// Windows 上即 `%APPDATA%\com.momadacoding.jkk\externalTools.json`。
fn config_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| GitError::new(CODE, format!("无法定位配置目录：{e}")))?;
    Ok(dir.join("externalTools.json"))
}

#[tauri::command(async)]
pub fn read_external_tools(app: AppHandle) -> Result<external_tools_config::ExternalToolsConfig> {
    external_tools_config::load_from(&config_path(&app)?)
}

/// 用系统默认程序打开配置文件。
///
/// 故意不接受路径参数（同 `open_custom_commands_config`）：前端没有能力
/// 让系统去打开一个它自己拼出来的路径。
///
/// 配置坏了也照样打开——用户正是要去修它，所以这里只确保文件存在，不做解析。
#[tauri::command(async)]
pub fn open_external_tools_config(app: AppHandle) -> Result<()> {
    let path = config_path(&app)?;
    if !path.exists() {
        external_tools_config::load_from(&path)?;
    }
    let text = path
        .to_str()
        .ok_or_else(|| GitError::new(CODE, "配置路径不是合法的 UTF-8"))?;
    launcher::open_in_default_app(text)
}

/// 用外部工具对比一个文件。立即返回；工具退出后发 `external-tool-exited`。
///
/// 每次都重新读配置：用户改完 externalTools.json 不用重启应用。
#[tauri::command(async)]
pub fn open_external_diff(
    app: AppHandle,
    path: String,
    tool_id: String,
    target: DiffTarget,
) -> Result<()> {
    let config_path = config_path(&app)?;
    let cfg = external_tools_config::load_from(&config_path)?;
    let tool = external_tools_config::find_tool(&cfg, &tool_id)?.clone();
    let file = target.file().to_string();
    let worktree = path.clone();
    external_diff::open_external_diff(
        &path,
        &cfg,
        &config_path,
        &tool_id,
        &target,
        move |report| {
            let _ = app.emit(
                "external-tool-exited",
                exit_event(&worktree, &file, &tool, report),
            );
        },
    )
}

/// 用外部工具解决一个冲突文件。立即返回；工具退出后发 `external-tool-exited`。
///
/// 工具退出后不暂存：冲突文件由用户回到应用里检查后手动 Stage（spec §2 反目标）。
#[tauri::command(async)]
pub fn open_external_merge(
    app: AppHandle,
    path: String,
    tool_id: String,
    file: String,
) -> Result<()> {
    let config_path = config_path(&app)?;
    let cfg = external_tools_config::load_from(&config_path)?;
    let tool = external_tools_config::find_tool(&cfg, &tool_id)?.clone();
    let worktree = path.clone();
    let exited_file = file.clone();
    external_merge::open_external_merge(&path, &cfg, &config_path, &tool_id, &file, move |report| {
        let _ = app.emit(
            "external-tool-exited",
            exit_event(&worktree, &exited_file, &tool, report),
        );
    })
}
