//! SVN 混合仓库的初始化、重建与体检入口（Phase 1b）。
//!
//! # ⚠️ 三个会改磁盘的命令
//!
//! `svn_init_hybrid` 会在用户的 SVN 工作副本里创建 `.git`，`svn_rebuild_baseline`
//! 会移动基线指针。`AGENTS.md` §7.3 要求签名改动必须实际跑一次 `pnpm tauri dev`
//! 验证——对这三个来说那不是形式。
//!
//! # 参数名的 snake_case ↔ camelCase 转换没有编译期检查
//!
//! Tauri 把 Rust 侧的 `snake_case` 参数名转成 JS 侧的 `camelCase`，写错只会在运行时
//! 静默失败或报「参数缺失」。返回类型同理，全部结构体都带
//! `#[serde(rename_all = "camelCase")]`，并在 `tests/svn_backend.rs` 里用断言钉住
//! 序列化后的字段名。

use crate::engine::svn_externals::PinnedExternal;
use crate::engine::svn_health::Health;
use crate::engine::svn_init::{
    check_current_baseline, init_hybrid, pinned_externals, rebuild_baseline, InitReport,
    InitRequest, RebuildReport,
};
use crate::engine::svn_log::{fetch_log, read_log};
use crate::engine::svn_log_cache::CachedRev;
use crate::engine::svn_preflight::{run_preflight, Preflight};
use crate::engine::svn_server_set::{audit_server_set_at_baseline, ServerAudit};
use crate::engine::svn_sync::{check_sync, read_root_revs, RootRev, SyncStatus};
use crate::error::GitError;

/// 只跑前置校验 P1–P4，**什么都不创建**。
///
/// 向导在用户确认白名单之后、真正动手之前调它，把「能不能建」这件事先告诉用户。
#[tauri::command(async)]
pub fn svn_preflight_check(path: String, whitelist: Vec<String>) -> Result<Preflight, GitError> {
    run_preflight(&path, &whitelist)
}

/// 走完整个初始化向导。任何一步失败都会把本次创建的 `.git` 删干净。
#[tauri::command(async)]
pub fn svn_init_hybrid(
    path: String,
    whitelist: Vec<String>,
    extensions: Vec<String>,
) -> Result<InitReport, GitError> {
    init_hybrid(&InitRequest {
        repo_root: path,
        whitelist,
        extensions,
    })
}

/// 重建基线 R1–R5。全程不动 HEAD、不动真实索引、不动磁盘。
#[tauri::command(async)]
pub fn svn_rebuild_baseline(path: String) -> Result<RebuildReport, GitError> {
    rebuild_baseline(&path)
}

/// 体检当前基线。rev 取自基线自己的 trailer，所以这里**能**报出 `Lying`。
///
/// 三种结果都通过 `Ok` 返回，不是 `Err`：「没能验证」（断网、认证、URL 不可达）
/// 与「验证不通过」必须分开，把前者当成后者会让离线场景寸步难行（spec §6.8.6）。
#[tauri::command(async)]
pub fn svn_check_baseline(path: String) -> Result<Health, GitError> {
    check_current_baseline(&path)
}

/// 当前仓库里被 `-r` 钉死、且与白名单有交集的 external。
///
/// spec §6.5.3 第 3 条要求 pinned 模式的残余风险**UI 常驻说明**——常驻就意味着
/// 界面每次打开这个仓库都得能拿到这份清单，光在初始化向导里显示一次不算。
///
/// 不是混合仓库时返回空列表而不是报错：调用方是「打开仓库」那条热路径，
/// 普通 git 仓库走到这里是正常的，不该在那儿制造一条错误。
///
/// ⚠️ 它要跑 `svn propget -R`，在几万文件的工作副本上不便宜。调用方**不要 await**
/// （对齐 `refreshPullRequests` 的做法）。
#[tauri::command(async)]
pub fn svn_pinned_externals(path: String) -> Result<Vec<PinnedExternal>, GitError> {
    pinned_externals(&path)
}

// ═══════════════════════════════════════════════════════════════════════════
// Phase 1c：失同步状态、SVN 历史、服务器侧核对
//
// 这一组**全部只读**，一条 SVN 写命令都不发。
// ═══════════════════════════════════════════════════════════════════════════

/// 完整的失同步校验（spec §6.6.3 的**第二档**）。
///
/// 纯本地读（`svn status` / `svn info`），不走网络。时机：打开仓库、以及第一档
/// 的 [`svn_root_revs`] 报告「有人动过」之后。
#[tauri::command(async)]
pub fn svn_sync_status(path: String) -> Result<SyncStatus, GitError> {
    check_sync(&path)
}

/// 每个工作副本根的 revision（spec §6.6.3 的**第一档**，O(1)）。
///
/// 挂在高频的 `repo-changed` 刷新上。**与上次观测值比**，变了才跑第二档；
/// 拿它跟 `baselineRev` 比是错的——交付之后根节点会长期停在旧 revision，
/// 那样每次刷新都会判不等，分层的意义直接归零（spec §6.6.3 有实测）。
///
/// 上次观测值由前端 store 持有，后端保持无状态。
#[tauri::command(async)]
pub fn svn_root_revs(path: String) -> Result<Vec<RootRev>, GitError> {
    read_root_revs(&path)
}

/// 读**缓存里**的 SVN 历史，**不走网络**。
///
/// 打开仓库时先用它把图画出来（spec §10.2），随后由 [`svn_fetch_log`] 异步拉新的。
/// 缓存不存在或读坏了都返回空列表而不是报错——那只是「还没拉过」。
#[tauri::command(async)]
pub fn svn_read_log(path: String) -> Result<Vec<CachedRev>, GitError> {
    read_log(&path)
}

/// 拉取 SVN 历史并写回缓存。**走网络**，调用方不要 await。
///
/// `before_rev`（JS 侧 `beforeRev`）为 `None` 时拉「比已缓存的更新的那一段」；
/// 给了值就是往下翻页，拉比它更旧的 `limit` 条。
#[tauri::command(async)]
pub fn svn_fetch_log(
    path: String,
    before_rev: Option<u64>,
    limit: u32,
) -> Result<Vec<CachedRev>, GitError> {
    fetch_log(&path, before_rev, limit)
}

/// 服务器侧清单核对：白名单内的代码文件有没有在本地漏收（§6.3.1 / NB-2）。
///
/// **走网络**（`svn ls -R`），在大工作副本上可能不便宜，调用方不要 await。
/// 三种结论都通过 `Ok` 返回；`NotChecked` 与 `Matched` 必须分开——把「没验成」
/// 当成「没漏收」，就是用修这个缺陷的代码把它重新造一遍。
#[tauri::command(async)]
pub fn svn_server_audit(path: String) -> Result<ServerAudit, GitError> {
    audit_server_set_at_baseline(&path)
}
