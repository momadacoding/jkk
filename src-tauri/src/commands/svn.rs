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
    check_current_baseline, eol_status, init_hybrid, migrate_eol, pinned_externals,
    rebuild_baseline, EolStatus, InitReport, InitRequest, RebuildReport,
};
use crate::engine::svn_journal::{Flow, Phase};
use crate::engine::svn_lock::LockHolder;
use crate::engine::svn_log::{fetch_log, read_log, LogPage};
use crate::engine::svn_preflight::{run_preflight, Preflight};
use crate::engine::svn_server_set::{audit_server_set_at_baseline, ServerAudit};
use crate::engine::svn_sync::{check_sync, read_root_revs, RootRev, SyncStatus};
use crate::engine::{svn_journal, svn_lock};
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

/// 行尾规则的安装状态（spec §7.3.0）。打开仓库时读一次。
///
/// **纯读。** 规则缺失意味着这个仓库的基线是在旧的行尾设置下录的，要走一次
/// `svn_migrate_eol`；但那必须由用户确认后触发，侦测本身一个字节都不写——
/// 只装规则不重建会把仓库留在潜伏故障状态。
#[tauri::command(async)]
pub fn svn_eol_status(path: String) -> Result<EolStatus, GitError> {
    eol_status(&path)
}

/// 走一次行尾迁移：装规则 + 重录基线（spec §7.3.0）。
///
/// **前端必须先过确认框**：这会移动 `svn/base` 指针，与 `svn_rebuild_baseline`
/// 同等分量。重建失败时规则会被还原，仓库回到原状。
#[tauri::command(async)]
pub fn svn_migrate_eol(path: String) -> Result<RebuildReport, GitError> {
    migrate_eol(&path)
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
pub fn svn_read_log(path: String) -> Result<LogPage, GitError> {
    read_log(&path)
}

/// 拉取 SVN 历史并写回缓存。**走网络**，调用方不要 await。
///
/// `older == false` 是增量拉新；`true` 是往下翻一页。
///
/// # 翻页游标**不由前端传**
///
/// 曾经的签名是 `before_rev: Option<u64>`，前端传序列里最旧的那条 revision。
/// 那个序列含孤儿锚点、也混了多个 target，不代表任何一段连续覆盖，会让中间
/// 整段历史被永久跳过（详见 [`fetch_log`] 的文档）。游标改由后端按各 target
/// 自己的覆盖区间算，前端只说「再往下翻一页」。
#[tauri::command(async)]
pub fn svn_fetch_log(path: String, older: bool, limit: u32) -> Result<LogPage, GitError> {
    fetch_log(&path, older, limit)
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

/// 上次同步留下的痕迹：`phase` 与 `opid`。`flow` 决定恢复入口走哪条分支
/// （spec §7.6：`publishing-baseline` 是两个流程共用的阶段名，父提交与返回
/// 入口却完全不同，不能套用）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnfinishedSync {
    pub flow: Flow,
    pub phase: Phase,
    pub opid: String,
    pub started_at: u64,
}

/// 同步状态。**`locked` 与 `unfinished` 是两件事，不能合并。**
///
/// `locked` 说的是「现在有人在跑」，`unfinished` 说的是「上次留下了记录」。
/// 两者的出口完全相反：前者要等，后者要用户决定继续还是放弃。合成一个布尔值
/// 会让界面在正常同步期间劝用户去「放弃」。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub locked: bool,
    pub holder: Option<LockHolder>,
    pub unfinished: Option<UnfinishedSync>,
}

/// 同步状态：现在有没有人在跑，以及上次有没有留下没收尾的 journal。
///
/// **纯读**，且**不拿锁**——它恰恰是用来观察锁的。这也是 spec §5.1
/// 「读操作不阻塞，但要能返回『同步进行中』状态供 UI 显示」那一条的落点。
///
/// ⚠️ `read` 返回 `Err`（journal 坏了）时**照常往上抛**，不在这里吞成 `None`。
/// 吞掉等于把用户 stash 的 OID 悄悄丢掉，而界面会显示「一切正常」——
/// 磁盘上工作区却已经被清空了。`tests/svn_journal.rs` 有一条测试守着这个语义。
#[tauri::command(async)]
pub fn svn_sync_state(path: String) -> Result<SyncState, GitError> {
    Ok(SyncState {
        locked: svn_lock::is_held(&path),
        holder: svn_lock::holder(&path),
        unfinished: svn_journal::read(&path)?.map(|j| UnfinishedSync {
            flow: j.flow,
            phase: j.phase,
            opid: j.opid,
            started_at: j.started_at,
        }),
    })
}
