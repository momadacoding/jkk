use crate::attention;
use tauri::AppHandle;

/// 挂上或撤掉「有事要你看」的提醒（使用报告 9.5）：任务栏角标、托盘图标上的点、托盘提示里的原因。
///
/// `reason` 传 `null` 或空白撤掉。`flash` 为真时任务栏按钮闪到窗口拿到焦点为止；窗口这时在前台的话
/// 等它失焦再闪。JS 侧：`invoke('set_attention', { reason, flash })`。前端一次只发一个
/// （`attentionStore`），这里不再排序。
///
/// 不返回错误：提醒挂不上只记日志（`attention::set`）。用 async 命令：窗口与托盘的操作要派到
/// 主线程，不在主线程上等。
#[tauri::command(async)]
pub fn set_attention(app: AppHandle, reason: Option<String>, flash: bool) {
    attention::set(&app, attention::normalize_reason(reason), flash);
}
