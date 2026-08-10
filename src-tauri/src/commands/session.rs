use crate::engine::session::{self, RecentRepo, Session, SessionView};
use crate::error::{GitError, Result};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// 串行化「读 → 合并 → 写」这一段。
///
/// `#[tauri::command(async)]` 让每次调用落在各自的线程上，快速连点标签时
/// 两次保存是真的并发：两边都读到更新前的 `recent`，后写的那个会把前一个的
/// 更新整个盖掉。锁只护这一小段文件操作，不涉及任何 git 调用。
static SAVE_LOCK: Mutex<()> = Mutex::new(());

/// 会话文件路径：`app_config_dir()/session.json`，
/// 与 `customCommands.json` 同一个目录（Windows 上是 `%APPDATA%\com.momadacoding.jkk\`）。
fn session_path(app: &AppHandle) -> Result<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| GitError::new("session", format!("无法定位配置目录：{e}")))?;
    Ok(dir.join("session.json"))
}

#[tauri::command(async)]
pub fn load_session(app: AppHandle) -> Result<SessionView> {
    Ok(session::to_view(session::load_from(&session_path(&app)?)))
}

/// 保存当前标签与活动标签，返回更新后的最近列表。
///
/// 最近列表的维护整个放在后端：前端只知道「现在开着哪些标签」，
/// 让它自己算合并顺序和去重，规则迟早会和 Rust 侧漂移。
/// 返回值让前端不必重读文件就能刷新空态页上的最近列表。
#[tauri::command(async)]
pub fn save_session(
    app: AppHandle,
    tabs: Vec<String>,
    active_tab: Option<String>,
) -> Result<Vec<RecentRepo>> {
    let path = session_path(&app)?;
    // 锁中毒只说明另一个保存 panic 了；会话状态本身没有不变量被破坏，
    // 拿回内层值继续用，别让整个功能从此报错
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let previous = session::load_from(&path);
    let recent = session::merge_recent(&previous.recent, &tabs);
    session::save_to(&path, &Session::new(tabs, active_tab, recent.clone()))?;
    // 只把 recent 标上 exists 就够了。走 to_view 会顺带对每个标签跑一次
    // Repository::open——保存是每次切标签都发生的事，而一个指向断线 SMB 路径的
    // 标签会让每次 open 卡到共享超时，白白把保存拖成秒级操作。
    Ok(session::to_recent(recent))
}
