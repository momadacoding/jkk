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

use crate::engine::svn_health::Health;
use crate::engine::svn_init::{
    check_current_baseline, init_hybrid, rebuild_baseline, InitReport, InitRequest, RebuildReport,
};
use crate::engine::svn_preflight::{run_preflight, Preflight};
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
