//! `sync_in` 的四个入口（spec §7、§13）与 `sync_out` 的四个入口（spec §8、Phase 3a）。
//!
//! # 参数名的 snake_case ↔ camelCase 转换没有编译期检查
//!
//! JS 侧是 `invoke('svn_sync_in', { path, expect })`，`expect` 里是
//! `{ origBranch, origHead, baselineOid, confirmedOutside }`；
//! `invoke('svn_sync_out', { path, expect })` 的 `expect` 是
//! `{ origBranch, origHead, baselineOid, deliverTree, changes, confirmedOutside, message }`。
//! `tests/svn_backend.rs` 钉着序列化形状；签名改动必须实跑一次 `pnpm tauri dev`
//! （`AGENTS.md` §7.3）。

use crate::commands::watch::WatchSuspension;
use crate::engine::svn_sync_check::{preview_sync_in, SyncInPreview};
use crate::engine::svn_sync_in::{
    abort_sync, continue_sync, run_sync_in, SyncInExpect, SyncInOutcome,
};
use crate::engine::svn_sync_out::{
    abort_sync_out, continue_sync_out, run_sync_out, SyncOutExpect, SyncOutOutcome,
};
use crate::engine::svn_sync_out_check::{preview_sync_out, SyncOutPreview};
use crate::error::GitError;
use tauri::{AppHandle, Emitter};

/// 预览：前置检查 1–6，**什么都不改**。确认框里的全部事实都来自这里。
#[tauri::command(async)]
pub fn svn_sync_in_preview(path: String) -> Result<SyncInPreview, GitError> {
    preview_sync_in(&path)
}

/// 执行一次更新。`expect` 是用户在确认框里看到的预览，锁内复核要与它一致。
#[tauri::command(async)]
pub fn svn_sync_in(
    handle: AppHandle,
    path: String,
    expect: SyncInExpect,
) -> Result<SyncInOutcome, GitError> {
    with_watch_suspended(&handle, &path, || run_sync_in(&path, &expect))
}

/// 从中断处接着走（冲突解决之后，或崩溃重开之后）。
#[tauri::command(async)]
pub fn svn_sync_continue(handle: AppHandle, path: String) -> Result<SyncInOutcome, GitError> {
    with_watch_suspended(&handle, &path, || continue_sync(&path))
}

/// 放弃这次更新，回到更新前的分支并放回未提交的改动。
#[tauri::command(async)]
pub fn svn_sync_abort(handle: AppHandle, path: String) -> Result<SyncInOutcome, GitError> {
    with_watch_suspended(&handle, &path, || abort_sync(&path))
}

/// 交付预览：前置检查与净变更清单，**什么都不改**（Phase 3a 计划 B1）。
#[tauri::command(async)]
pub fn svn_sync_out_preview(path: String) -> Result<SyncOutPreview, GitError> {
    preview_sync_out(&path)
}

/// 执行一次交付。`expect` 是用户在确认框里看到并同意的预览与提交信息。
#[tauri::command(async)]
pub fn svn_sync_out(
    handle: AppHandle,
    path: String,
    expect: SyncOutExpect,
) -> Result<SyncOutOutcome, GitError> {
    with_watch_suspended(&handle, &path, || run_sync_out(&path, &expect))
}

/// 从中断处接着走（D5 之前一律是清理，计划 B8）。
#[tauri::command(async)]
pub fn svn_sync_out_continue(handle: AppHandle, path: String) -> Result<SyncOutOutcome, GitError> {
    with_watch_suspended(&handle, &path, || continue_sync_out(&path))
}

/// 放弃这次交付（D5 之后是「停止收尾」，计划 B7）。
#[tauri::command(async)]
pub fn svn_sync_out_abort(handle: AppHandle, path: String) -> Result<SyncOutOutcome, GitError> {
    with_watch_suspended(&handle, &path, || abort_sync_out(&path))
}

/// 挂起 watcher 跑完一段同步，然后**无论成败**补发一次 `repo-changed`。
///
/// spec §5.2：挂起期间事件被丢弃而不是攒着，所以结束时必须主动告诉前端
/// 「仓库变了」，否则 update 带来的那一大批改动界面看不见。失败也要发：回退
/// 本身就动过分支和工作区。
fn with_watch_suspended<T>(
    handle: &AppHandle,
    path: &str,
    f: impl FnOnce() -> Result<T, GitError>,
) -> Result<T, GitError> {
    let r = {
        let _w = WatchSuspension::new(path);
        f()
    };
    let _ = handle.emit("repo-changed", path);
    r
}
