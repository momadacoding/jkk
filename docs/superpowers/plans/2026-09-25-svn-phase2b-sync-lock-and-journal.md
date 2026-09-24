# Phase 2b：同步基础设施（跨进程锁 + 写前 journal + watcher 挂起）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `sync_in` / `sync_out` 都要用的三件基础设施做出来并单独验证：跨进程仓库锁、写前 operation journal、同步期间的 watcher 挂起；外加「上次同步未完成」的检测与展示。

**Architecture:** 锁是一个**长期持有的独占文件句柄**（不是 pid 探测，理由见「与 spec 的偏离」），崩溃后由操作系统自动释放，因此「文件还在」与「真的有人在跑」被干净地区分开。journal 是写前日志：任何一步落地之前它必须已经在磁盘上，写入走「临时文件 → fsync → rename」。watcher 挂起是一个按归一化路径的集合，去抖线程发事件前查一次。

**Tech Stack:** Rust（`std::os::windows::fs::OpenOptionsExt::share_mode`、`serde` / `serde_json`）、React 18 + TypeScript、Vitest、cargo test + fixture 仓库。**不新增任何依赖。**

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（本计划实现 §5.1、§5.1.1、§5.2、§7.6、§7.6.1 的落库部分，以及 §13 中与「拿不到跨进程锁」「客户端崩溃」两行相关的展示）

**前置：** Phase 2a（PR #45）已合并到 `master`（`b1473b9`）。

---

## Global Constraints

逐条抄自 spec 与 `AGENTS.md`，每个 Task 的要求都隐含包含本节。

- 锁文件：`.git/mygitclient-sync.lock`；journal：`.git/mygitclient-sync.json`（spec §5.1.1、§7.6）
- 获取锁的语义是「**原子创建且必须不存在**」（POSIX `O_EXCL` / Windows `CREATE_NEW`）（§5.1.1）
- **拒绝而不是排队**：持锁期间所有写操作直接返回 `GitError::new("sync_in_progress", …)`（§5.1）
- **读操作（status / diff / graph）不阻塞**，但要能返回「同步进行中」状态供 UI 显示（§5.1）
- journal **原子写入**：写临时文件 → `fsync` → rename。「半个 JSON 比没有 JSON 更糟」（§7.6）
- **每进入一个 phase 前先更新 `phase` 字段**，而不是事后补记（§7.6）
- stash 一律按**不可变 OID** 记，禁止记 `stash@{N}`（§7.6.1）
- rebase 中途状态**不重复记**，复用现有 `repo_state` 命令（§7.6）
- journal 的所有权由锁界定：**拿到锁才允许读写 journal**（§5.1.1）
- 陈旧锁**只有在确认持有者已不存在时**才允许接管，接管必须写日志并告知用户（§5.1.1）
- ⚠️ **不能只看时间戳**——一次 `svn commit` 卡在网络上几分钟是正常的（§5.1.1）
- 所有 `#[tauri::command]` 必须写成 `#[tauri::command(async)]`（`AGENTS.md` §7.3，`tests/commands_are_async.rs` 守着）
- 所有 git CLI 调用走现有 `run_git` 封装，错误映射为 `crate::error::GitError`（§7.3）
- 涉及 git 写操作的测试必须自建临时 fixture 仓库（`AGENTS.md` §8）
- 注释用中文，解释「为什么」而不是「做了什么」（§7.3）
- 本仓库**没有 PR 门禁 CI**，也**没有 lint 工具**；前端唯一静态检查是 `pnpm build` 里的 `tsc`，且 `tsc` 不覆盖 `tests/vitest/`（§5）

---

## 与 spec 的一处**有意偏离**（执行者必读，不要"改回去"）

spec §5.1.1 写的是：

```text
陈旧锁：只有当记录的 pid 已不存在时才允许接管
```

**本计划不用 pid 探测。** 2026-09-25 在 Windows 上实测（三组探针，见下），改用「尝试独占打开锁文件」作为存活判据：

| 观察 | 结果 |
|---|---|
| 持锁进程活着时，另一方尝试以 `share_mode(0)` 打开 | `BLOCKED`（`ERROR_SHARING_VIOLATION`，`raw_os_error() == 32`） |
| 持锁进程**被 `Stop-Process -Force` 杀掉**后，锁文件 | **仍然存在** |
| 同上，另一方再尝试独占打开 | `ACQUIRED` —— 操作系统已自动释放句柄 |
| 持锁期间，第三方以 `share_mode(FILE_SHARE_READ\|FILE_SHARE_WRITE)` 只读打开 | **读得到** `{pid:…}`，满足 §13「显示持锁进程的 pid 与开始时间」 |

为什么这比 pid 探测好，三条都是实打实的：

1. **崩溃安全由操作系统保证**，不依赖我们的清理代码跑到——而崩溃恰恰是清理代码跑不到的那种情况
2. **没有 pid 复用问题**。pid 探测会把「一个无关新进程恰好复用了死进程的 pid」误判成「持锁者还活着」，于是那把锁**永远**接管不了，用户被永久卡死
3. **不需要给 `windows` crate 开 `Win32_System_Threading` feature**（`OpenProcess`），零依赖变动

代价：非 Windows 平台上 `share_mode` 不存在。按 `engine/file_lock.rs` 已有的做法拆 `#[cfg(windows)]` / `#[cfg(not(windows))]`，非 Windows 分支返回「无法判定存活」并要求用户显式接管。本项目主力平台是 Windows（`AGENTS.md` §10），与 `file_lock.rs` 的处理一致。

`pid` / `startedAt` **仍然要写进锁文件**——§13 要求把它们显示给用户，只是它们不再是判据。

> 执行完本计划后，把这条偏离补进 spec §5.1.1 与 §18 修订记录。**这属于本计划的验收项**，见 Task 6 的收尾步骤。

---

## File Structure

### 新建

| 文件 | 职责 |
|---|---|
| `src-tauri/src/engine/svn_lock.rs` | 跨进程仓库锁：获取（RAII）、存活探测、持有者信息、`assert_not_syncing` |
| `src-tauri/src/engine/svn_journal.rs` | 写前 journal：`Phase` 枚举、原子写、读取与校验、删除 |
| `src-tauri/tests/svn_lock.rs` | 锁的集成测试（含一条真正跨进程的崩溃测试） |
| `src-tauri/tests/svn_journal.rs` | journal 的集成测试 |
| `src-tauri/tests/svn_write_guard.rs` | 源码扫描测试：每个写方法都必须先过锁检查 |
| `src/engine/svnJournal.ts` | 前端纯逻辑：`phase` → 横幅文案与可用出口 |
| `tests/vitest/svnJournal.test.ts` | 上者的单测 |

### 修改

| 文件 | 改动 |
|---|---|
| `src-tauri/src/engine/mod.rs` | 注册 `svn_lock`、`svn_journal` |
| `src-tauri/src/vcs/svn.rs` | 12 个写方法前置 `assert_not_syncing` |
| `src-tauri/src/commands/watch.rs` | 挂起 / 恢复 API + 去抖线程查挂起状态 |
| `src-tauri/src/commands/svn.rs` | 新增 `svn_sync_state` 命令 |
| `src-tauri/src/lib.rs` | 注册新命令 |
| `src/store/repoStore.ts` | `svnSyncState()` + 状态字段 + 刷新 |
| `src/components/BaselineBanner.tsx` | 「上次同步未完成」横幅 |
| `src/components/RepoTab.tsx` | 接线 |
| `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md` | §5.1.1 的偏离 + §18 修订记录 |

### 为什么锁和 journal 分两个文件

它们的**失效模式完全不同**，混在一起会让人以为二者同生共死。锁没拿到 ⇒ 什么都别做；journal 存在 ⇒ 上次没收尾，要给用户出口。更实际的理由：Phase 3 的 `sync_out` 会给 journal 加一大批字段（`deliverRef` / `svnDelta` / `tempWorktree`…），锁则一个字段都不会变。

---

## 开工前：必须先建决策日志

`AGENTS.md` §2.1 硬性要求，且要求**在动第一个 Task 之前建好**。

- [ ] **Step 0: 建 `docs/superpowers/plans/2026-09-25-svn-phase2b-sync-lock-and-journal-decisions.md`**

内容两部分：进度总账表（每个 Task 的状态 / 产出 / 验证，**每完成一个 Task 立即更新**），以及按 Task 分节的决策条目，每节至少覆盖：偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的。

- [ ] **Step 0b: 建 worktree 并切进去**

`AGENTS.md` §4.1：动第一行代码之前必须用 `EnterWorktree` 切入，且 `Get-Location` 的实测值要在该 worktree 内。建议名字 `svn-phase2b-lock`。

---

## Task 1: 跨进程锁的获取与释放

**Files:**
- Create: `src-tauri/src/engine/svn_lock.rs`
- Create: `src-tauri/tests/svn_lock.rs`
- Modify: `src-tauri/src/engine/mod.rs`

**Interfaces:**
- Consumes: `crate::error::{GitError, Result}`
- Produces:
  ```rust
  pub struct SyncLock { /* 私有 */ }
  #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "camelCase")]
  pub struct LockHolder { pub pid: u32, pub started_at: u64, pub opid: String }

  pub fn lock_path(repo_root: &str) -> std::path::PathBuf;
  pub fn acquire(repo_root: &str, opid: &str) -> Result<SyncLock>;
  pub fn holder(repo_root: &str) -> Option<LockHolder>;
  pub fn is_held(repo_root: &str) -> bool;
  impl Drop for SyncLock;  // 释放句柄并删除文件
  ```

- [ ] **Step 1: 写失败测试 —— 活锁挡住第二次获取，Drop 之后又能拿**

建 `src-tauri/tests/svn_lock.rs`：

```rust
//! 跨进程仓库锁（spec §5.1.1）。
//!
//! 这把锁的全部价值在**崩溃之后**：清理代码跑不到的时候，锁还能不能被正确
//! 判定成"没人拿着"。所以本文件里最重要的不是 happy path，是那条真正杀掉
//! 一个外部进程的测试（见 `a_lock_whose_holder_was_killed_is_not_held`）。

use app_lib::engine::svn_lock::{acquire, holder, is_held, lock_path};

/// 锁只要求 `<repo>/.git` 存在，不需要是完整的混合仓库——
/// 它在 `init_hybrid` 之外也会被用到。
fn git_dir_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".git")).unwrap();
    d
}

#[test]
fn a_live_lock_blocks_a_second_acquisition() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();

    let guard = acquire(&root, "opid-A").unwrap();
    assert!(is_held(&root), "拿到锁之后必须报「有人持有」");

    let err = acquire(&root, "opid-B").expect_err("第二次获取必须失败，不能排队");
    assert_eq!(err.code, "sync_in_progress", "实际：{}", err.message);

    drop(guard);
    assert!(!is_held(&root), "Drop 之后必须释放");
    acquire(&root, "opid-C").expect("释放之后必须能重新获取");
}

#[test]
fn the_holder_is_readable_while_the_lock_is_held() {
    // §13 要求"显示持锁进程的 pid 与开始时间"。持锁期间读不到这些信息的话，
    // 用户看到的就只是一句"有人在同步"，无从判断是不是自己那个卡死的窗口。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let _guard = acquire(&root, "opid-XYZ").unwrap();

    let h = holder(&root).expect("持锁期间必须读得到持有者信息");
    assert_eq!(h.pid, std::process::id());
    assert_eq!(h.opid, "opid-XYZ");
    assert!(h.started_at > 0, "startedAt 必须落盘");
}

#[test]
fn a_lock_file_left_behind_by_a_crash_is_not_held() {
    // **文件存在 ≠ 有人拿着**（2026-09-25 实测：杀掉持锁进程后文件仍在）。
    // 把"文件存在"当判据的话，一次崩溃就让这个仓库永远同步不了。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    std::fs::write(lock_path(&root), br#"{"pid":999999,"startedAt":1,"opid":"dead"}"#).unwrap();

    assert!(!is_held(&root), "没有任何进程持有句柄时，残留文件不算被持有");
    acquire(&root, "opid-new").expect("残锁必须可以接管");
}
```

- [ ] **Step 2: 跑测试，确认因为模块不存在而编译失败**

```bash
cd src-tauri
cargo test --test svn_lock
```

Expected: 编译失败，`unresolved import app_lib::engine::svn_lock`。

- [ ] **Step 3: 实现 `svn_lock.rs`**

```rust
//! 仓库级跨进程同步锁（spec §5.1、§5.1.1）。
//!
//! # 为什么不能用 `std::sync::Mutex`
//!
//! 它的作用域只到进程边界。本仓库**没有** `tauri-plugin-single-instance`，
//! 同一个工作副本上完全可能开着两个 mygitclient 进程；两个进程各拿各的进程内
//! 锁之后，会同时写 `.git/mygitclient-sync.json`，后写的把前一个的 journal
//! 整个覆盖——被覆盖那一方的 stash OID 全部丢失，等于失去恢复依据。
//!
//! # 判据是"能不能独占打开"，不是"pid 还在不在"
//!
//! spec §5.1.1 原写的是 pid 探测，本实现**有意偏离**。2026-09-25 在 Windows 上
//! 实测：
//!
//! ```text
//! 持锁进程活着     → 独占打开 BLOCKED（raw_os_error() == 32）
//! 持锁进程被杀     → 锁文件仍在，但独占打开 ACQUIRED —— 句柄由操作系统释放了
//! ```
//!
//! 这比 pid 探测强在两处：崩溃安全由操作系统保证（而崩溃正是我们的清理代码跑
//! 不到的情形）；且没有 **pid 复用**问题——一个无关新进程恰好复用了死进程的
//! pid，会让 pid 探测把残锁永远判成"活着"，用户被永久卡死。
//!
//! `pid` / `startedAt` 仍然落盘：§13 要求显示给用户，只是不再用作判据。

use crate::error::{GitError, Result};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

/// 锁文件名。与 journal（`mygitclient-sync.json`）同目录、同前缀，便于人工排查。
const LOCK_FILE: &str = "mygitclient-sync.lock";

/// 持锁者信息。**只用于显示**（§13），不参与存活判定。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockHolder {
    pub pid: u32,
    /// Unix 秒。用于"从什么时候开始"的展示——**不能**拿它当超时判据：
    /// 一次 svn commit 卡在网络上几分钟是正常的（spec §5.1.1）。
    pub started_at: u64,
    pub opid: String,
}

/// `<repo_root>/.git/mygitclient-sync.lock`。
///
/// 直接拼 `.git` 而不是问 `git rev-parse --git-dir`：与邻居 `svn_attributes.rs`
/// 的 `info_dir` 同一约定，且混合仓库的 `.git` 一定是工作副本根下的真实目录
/// （`init_hybrid` 在那里跑的 `git init`，§6.1 的识别也据此）。省掉一次子进程
/// 调用在这里是要紧的：`assert_not_syncing` 会挂在每一个写操作前面。
pub fn lock_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join(LOCK_FILE)
}

/// 持有中的锁。**靠 `Drop` 释放**，不靠调用点记得删。
pub struct SyncLock {
    /// 这个句柄本身就是锁：它一直开着，别人就独占打不开；进程没了，
    /// 操作系统替我们关掉。**不要提前 drop 它**。
    _file: std::fs::File,
    path: PathBuf,
}

/// 以"必须能独占"的方式打开一个**已存在**的文件。
///
/// `Ok(_)` ⇒ 没有别的进程持有 ⇒ 是残锁。
/// `Err(32/33)` ⇒ 有人正拿着（`ERROR_SHARING_VIOLATION` / `ERROR_LOCK_VIOLATION`，
/// 与 `engine/file_lock.rs::is_occupied` 同一套判据）。
#[cfg(windows)]
fn exclusive_open(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0) // 不允许任何人同时持有
        .open(path)
}

#[cfg(not(windows))]
fn exclusive_open(path: &Path) -> std::io::Result<std::fs::File> {
    // 类 Unix 上 std 没有跨进程劝告锁，这里拿不到等价语义。
    // 于是**永远报"打得开"**，也就是永远把残锁当成可接管——
    // 由 `is_held` 那边的注释说明这个降级，不在这里假装安全。
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}

/// 现在是不是真的有进程持有这把锁？
///
/// 注意与"锁文件存不存在"的区别：崩溃之后文件还在，但没人持有（实测）。
pub fn is_held(repo_root: &str) -> bool {
    let p = lock_path(repo_root);
    if !p.exists() {
        return false;
    }
    match exclusive_open(&p) {
        Ok(_) => false, // 独占拿得到 ⇒ 没人持有 ⇒ 残锁
        Err(e) => matches!(e.raw_os_error(), Some(32) | Some(33)),
    }
}

/// 读持锁者信息。持锁期间也读得到——读的时候**允许**已有的写者存在。
pub fn holder(repo_root: &str) -> Option<LockHolder> {
    let mut f = permissive_open(&lock_path(repo_root)).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(&s).ok()
}

/// 只读打开，且**允许**别的进程同时读写。
///
/// 少了"允许写者"这一半就读不到：持锁者是以读写方式开着的，而一个只允许
/// 共享读的打开请求会与它冲突（2026-09-25 实测，`File.ReadAllText` 正是这样
/// 失败的）。
#[cfg(windows)]
fn permissive_open(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)
}

#[cfg(not(windows))]
fn permissive_open(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).open(path)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 拿锁。拿不到**立刻失败**，不排队（spec §5.1：排队会让用户点了没反应、
/// 几秒后突然生效，比明确报错更糟）。
pub fn acquire(repo_root: &str, opid: &str) -> Result<SyncLock> {
    let path = lock_path(repo_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    // 先按"必须不存在"创建（§5.1.1 的 O_EXCL / CREATE_NEW 语义）。
    let mut file = match create_new_exclusive(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // 文件在。真有人拿着，还是上次崩溃留下的？
            if is_held(repo_root) {
                return Err(busy_error(repo_root));
            }
            // 残锁：接管。§5.1.1 要求接管必须写日志并告知用户。
            log::warn!(
                "接管了一把无人持有的同步锁（上次同步没有正常收尾）：{}，原持有者 {:?}",
                path.display(),
                holder(repo_root)
            );
            std::fs::remove_file(&path)?;
            create_new_exclusive(&path)?
        }
        Err(e) => return Err(e.into()),
    };

    let meta = LockHolder {
        pid: std::process::id(),
        started_at: now_secs(),
        opid: opid.to_string(),
    };
    // 元数据写进句柄本身，不另开一次——另开就要考虑"开不开得了"的分支，
    // 而我们此刻正独占着它。
    // ⚠️ `GitError` **没有** `From<serde_json::Error>`（`error.rs` 只实现了
    // git2 与 io 两种），所以这里必须 `map_err`，不能用 `?`。
    // 写法与 `svn_config::write_config` 保持一致。
    let body = serde_json::to_string(&meta)
        .map_err(|e| GitError::new("svn_lock", format!("锁元数据序列化失败: {e}")))?;
    file.rewind()?;
    file.write_all(body.as_bytes())?;
    file.flush()?;

    Ok(SyncLock { _file: file, path })
}

/// 「原子创建且必须不存在」+ 独占共享模式。两件事必须在**同一次**打开里完成：
/// 分成"先 create_new 再重开成独占"的话，中间那条缝足以让另一个进程插进来。
#[cfg(windows)]
fn create_new_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x1;
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        // 允许别人**只读**地看一眼 pid / startedAt（§13 要显示），
        // 但不允许任何人拿到写权限。
        .share_mode(FILE_SHARE_READ)
        .open(path)
}

#[cfg(not(windows))]
fn create_new_exclusive(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

fn busy_error(repo_root: &str) -> GitError {
    let who = match holder(repo_root) {
        Some(h) => format!("持有者：进程 {}，开始于 {}（opid {}）。", h.pid, h.started_at, h.opid),
        None => "读不到持有者信息（它可能刚刚开始）。".to_string(),
    };
    GitError::new(
        "sync_in_progress",
        format!("这个工作副本上已经有一次同步正在进行。{who}"),
    )
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        // Drop 里不能 panic（展开途中 panic 会直接 abort）。
        // 删不掉也不致命：句柄一关，下一次 `is_held` 就会判成残锁并接管。
        let _ = std::fs::remove_file(&self.path);
    }
}
```

在 `src-tauri/src/engine/mod.rs` 里按字母序加一行：

```rust
pub mod svn_lock;
```

- [ ] **Step 4: 跑测试，三条必须全绿**

```bash
cd src-tauri
cargo test --test svn_lock
```

Expected: `3 passed`。

- [ ] **Step 5: 变异验证 —— 把判据换成"文件存不存在"**

临时把 `is_held` 改成 `lock_path(repo_root).exists()`，重跑。

Expected: 只有 `a_lock_file_left_behind_by_a_crash_is_not_held` 变红，另外两条仍绿。确认后改回去。

> 这条变异是本 Task 的核心：它证明测试真的在区分"文件在"和"有人拿着"，
> 而不是碰巧通过。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/svn_lock.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_lock.rs
git commit -m "feat(svn): 跨进程同步锁的获取与释放"
```

---

## Task 2: 崩溃之后锁必须真的松开（跨进程实测）

Task 1 的三条测试全在**同一个进程**里。那证明不了最要紧的那件事：**进程没了，操作系统到底有没有替我们松手。** 这条要用一个真正的外部进程来验。

**Files:**
- Modify: `src-tauri/tests/svn_lock.rs`

**Interfaces:**
- Consumes: Task 1 的 `acquire` / `is_held` / `lock_path`
- Produces: 无新公开接口

- [ ] **Step 1: 写失败测试 —— 杀掉持锁进程之后锁必须可接管**

追加到 `src-tauri/tests/svn_lock.rs`：

```rust
/// 用一个**真的外部进程**持锁，然后杀掉它。
///
/// 为什么非要跨进程：同进程内 drop 走的是我们自己的 `Drop`，验证的是"代码有没有
/// 写对"；崩溃走的是操作系统回收句柄，验证的是"这个机制本身成不成立"。后者才是
/// 这把锁敢用的理由，也是它区别于 pid 探测的地方。
///
/// 只在 Windows 上跑：非 Windows 分支按注释所述是降级实现，没有等价语义可验。
#[cfg(windows)]
#[test]
fn a_lock_whose_holder_was_killed_is_not_held() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let path = lock_path(&root);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    // 外部进程：以"允许别人只读"的共享模式开着锁文件不放，正是 acquire 的形态
    let script = format!(
        "$f=[System.IO.File]::Open('{}','OpenOrCreate','ReadWrite','Read'); Start-Sleep 120",
        path.to_string_lossy().replace('\\', "/")
    );
    let mut child = app_lib::engine::proc::hidden_command("powershell")
        .args(["-NoProfile", "-Command", &script])
        .spawn()
        .expect("起不来 powershell");

    // 等它真的把文件打开；轮询而不是固定 sleep，避免在慢机器上假失败
    let mut opened = false;
    for _ in 0..50 {
        if is_held(&root) {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(opened, "外部进程没能在 5 秒内拿到锁，测试前提不成立");

    assert!(
        acquire(&root, "mine").is_err(),
        "外部进程正持有时，本进程绝不能拿到锁"
    );

    child.kill().expect("杀不掉子进程");
    child.wait().ok();

    // 关键断言：没有任何清理代码跑过，但锁必须已经松开
    let mut released = false;
    for _ in 0..50 {
        if !is_held(&root) {
            released = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        released,
        "持锁进程已被杀，锁却仍报「有人持有」——崩溃之后这个仓库就再也同步不了了"
    );
    assert!(path.exists(), "锁文件本身应当还在（残留），只是被判成无人持有");
    acquire(&root, "mine").expect("崩溃留下的残锁必须可以接管");
}
```

- [ ] **Step 2: 跑测试**

`engine/proc.rs::hidden_command` 已经是 `pub fn`，`engine/mod.rs` 里也已经有
`pub mod proc;`（两者都已核对，不需要改）。用它而不是裸 `std::process::Command`：
后者会弹黑窗，`proc.rs` 存在的全部理由就是这个。

```bash
cd src-tauri
cargo test --test svn_lock -- a_lock_whose_holder_was_killed
```

Expected: 先因 `proc` 不可见或断言不成立而失败；修正可见性后应当**通过**（Task 1 的实现已经正确）。

> 若这条测试在 Task 1 的实现下就直接绿了，那是对的——它不是驱动新实现的测试，
> 是给那条"操作系统会替我们松手"的假设**上锁**的测试。把这一点写进决策日志。

- [ ] **Step 3: 变异验证 —— 把存活判据改成 pid 探测**

临时把 `is_held` 改成「读 journal 里的 pid，用 `tasklist /FI "PID eq <pid>"` 判断是否存在」。

Expected: 本条测试仍绿（pid 确实没了），但**用一个仍存活的无关 pid 手工写进锁文件**时会变红——把这个变体也留成一条测试：

```rust
#[test]
fn a_stale_lock_naming_a_live_unrelated_pid_is_still_takeable() {
    // pid 复用的现实后果：pid 探测会把这把残锁永远判成"活着"，仓库被永久卡死。
    // 用当前测试进程自己的 pid 冒充——它当然活着，但它并没有持有这个句柄。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let meta = format!(
        r#"{{"pid":{},"startedAt":1,"opid":"recycled"}}"#,
        std::process::id()
    );
    std::fs::write(lock_path(&root), meta).unwrap();

    assert!(!is_held(&root), "pid 活着不等于这把锁有人拿着");
    acquire(&root, "mine").expect("必须可以接管");
}
```

- [ ] **Step 4: 两条测试都跑通**

```bash
cd src-tauri
cargo test --test svn_lock
```

Expected: `5 passed`。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/tests/svn_lock.rs
git commit -m "test(svn): 跨进程验证崩溃后同步锁确实被操作系统释放"
```

---

## Task 3: 持锁期间拒绝所有写操作

**Files:**
- Modify: `src-tauri/src/engine/svn_lock.rs`
- Modify: `src-tauri/src/vcs/svn.rs:84-121`
- Create: `src-tauri/tests/svn_write_guard.rs`

**Interfaces:**
- Consumes: Task 1 的 `is_held`
- Produces: `pub fn assert_not_syncing(repo_root: &str) -> Result<()>`

- [ ] **Step 1: 写失败测试 —— 源码扫描，每个写方法都必须先过检查**

`src-tauri/tests/svn_write_guard.rs`：

```rust
//! 守住「同步进行中，所有写操作必须拒绝」这条约束（spec §5.1）。
//!
//! 扫源码而不是靠 review 盯着——理由与 `commands_are_async.rs` 完全一样：
//! 新增一个写方法时忘记加守卫是**无声的**，编译器和 clippy 都不会报，而后果是
//! 同步跑到一半被一次 stage / checkout 从中间插进来。

use std::fs;
use std::path::Path;

/// `trait Vcs` 里属于"写"的方法。读方法（open / graph / status / diff /
/// list_stashes / capabilities）**不得**加守卫：spec §5.1 明确要求读不阻塞。
const WRITE_METHODS: &[&str] = &[
    "stage",
    "unstage",
    "discard",
    "commit",
    "create_branch",
    "delete_branch",
    "rename_branch",
    "checkout",
    "stash_push",
    "stash_apply",
    "stash_pop",
    "stash_drop",
];

#[test]
fn every_write_method_checks_the_sync_lock() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vcs/svn.rs");
    let src = fs::read_to_string(&path).expect("读不到 src/vcs/svn.rs");

    let mut missing = Vec::new();
    for m in WRITE_METHODS {
        // 取该方法从签名到下一个 `    fn ` 之间的方法体
        let sig = format!("    fn {m}(");
        let start = src
            .find(&sig)
            .unwrap_or_else(|| panic!("`{m}` 在 src/vcs/svn.rs 里找不到——方法改名了？"));
        let rest = &src[start + sig.len()..];
        let end = rest.find("\n    fn ").unwrap_or(rest.len());
        if !rest[..end].contains("assert_not_syncing") {
            missing.push(*m);
        }
    }

    assert!(
        missing.is_empty(),
        "以下写方法没有检查同步锁，同步跑到一半会被它们从中间插进来：\n  {}\n\
         在方法体第一行加 `assert_not_syncing(self.root())?;`。",
        missing.join("\n  ")
    );
}

#[test]
fn read_methods_are_not_guarded() {
    // 反向约束，同样重要：spec §5.1 要求"读操作不阻塞"。给读方法也加上守卫的话，
    // 同步期间界面会整个瞎掉——而那恰恰是最需要显示状态的时候。
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vcs/svn.rs");
    let src = fs::read_to_string(&path).expect("读不到 src/vcs/svn.rs");

    for m in ["open", "graph", "status", "diff", "list_stashes"] {
        let sig = format!("    fn {m}(");
        let start = src.find(&sig).unwrap_or_else(|| panic!("`{m}` 找不到"));
        let rest = &src[start + sig.len()..];
        let end = rest.find("\n    fn ").unwrap_or(rest.len());
        assert!(
            !rest[..end].contains("assert_not_syncing"),
            "读方法 `{m}` 不该有同步锁守卫：同步期间读必须照常可用（spec §5.1）"
        );
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri
cargo test --test svn_write_guard
```

Expected: `every_write_method_checks_the_sync_lock` FAIL，列出全部 12 个方法；`read_methods_are_not_guarded` PASS。

- [ ] **Step 3: 加 `assert_not_syncing`**

在 `svn_lock.rs` 末尾：

```rust
/// 写操作的守卫。**拒绝而不是排队**（spec §5.1）。
///
/// 读操作不得调用它：同步期间界面还要照常显示状态，把读也挡掉等于在最需要看清
/// 状况的时候把灯关了。`tests/svn_write_guard.rs` 两个方向都守着。
pub fn assert_not_syncing(repo_root: &str) -> Result<()> {
    if is_held(repo_root) {
        return Err(busy_error(repo_root));
    }
    Ok(())
}
```

在 `src-tauri/src/vcs/svn.rs` 里给 `SvnGitBackend` 补一个取根路径的访问器（`GitBackend` 已持有路径，若它没有公开访问器，就在 `SvnGitBackend` 上自己存一份 `root: String`，在 `open()` 里填），然后每个写方法首行加守卫。以 `stage` 为例：

```rust
    fn stage(&self, paths: &[String]) -> Result<()> {
        crate::engine::svn_lock::assert_not_syncing(&self.root)?;
        self.git.stage(paths)
    }
```

其余 11 个照此办理：`unstage`、`discard`、`commit`、`create_branch`、`delete_branch`、`rename_branch`、`checkout`、`stash_push`、`stash_apply`、`stash_pop`、`stash_drop`。

> ⚠️ **不要**给 `capabilities` / `open` / `graph` / `status` / `diff` /
> `list_stashes` 加。第二条测试会把它们挡回来。

- [ ] **Step 4: 跑测试确认通过**

```bash
cd src-tauri
cargo test --test svn_write_guard
```

Expected: `2 passed`。

- [ ] **Step 5: 补一条真行为测试（源码扫描不等于行为正确）**

源码里出现这个字符串，不代表它真的挡住了。追加到 `src-tauri/tests/svn_lock.rs`：

```rust
#[test]
fn staging_is_refused_while_a_sync_holds_the_lock() {
    // 扫源码能发现"忘了加"，发现不了"加了但没生效"。这条走真实后端。
    let fx = svn_fixtures::svn_repo_with_native_eol();
    app_lib::engine::svn_init::init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".into()],
        extensions: vec!["cs".into()],
    })
    .unwrap();
    let root = fx.wc_str();

    let backend = app_lib::vcs::backend_for(&root).unwrap();
    backend.stage(&["Code/Nat.cs".into()]).expect("没持锁时 stage 必须正常");

    let _guard = acquire(&root, "opid-sync").unwrap();
    let err = backend
        .stage(&["Code/Nat.cs".into()])
        .expect_err("持锁期间 stage 必须被拒");
    assert_eq!(err.code, "sync_in_progress", "实际：{}", err.message);

    // 读必须照常可用
    backend.status().expect("持锁期间读操作不得被阻塞（spec §5.1）");
}
```

需要在 `tests/svn_lock.rs` 顶部加：

```rust
#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;
use app_lib::engine::svn_init::InitRequest;
```

- [ ] **Step 6: 全跑一遍**

```bash
cd src-tauri
cargo test --test svn_lock --test svn_write_guard
cargo fmt --check && cargo clippy --all-targets
```

Expected: 全绿，clippy 0 告警。

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/engine/svn_lock.rs src-tauri/src/vcs/svn.rs src-tauri/tests/svn_write_guard.rs src-tauri/tests/svn_lock.rs
git commit -m "feat(svn): 同步进行中拒绝全部写操作，读操作照常"
```

---

## Task 4: 写前 journal

**Files:**
- Create: `src-tauri/src/engine/svn_journal.rs`
- Create: `src-tauri/tests/svn_journal.rs`
- Modify: `src-tauri/src/engine/mod.rs`

**Interfaces:**
- Consumes: `crate::error::{GitError, Result}`
- Produces:
  ```rust
  pub enum Flow { SyncIn, SyncOut }
  pub enum Phase { Start, Stashing, Stashed, CapturingBaseline, PublishingBaseline,
                   BaselineReady, Rebase, Prepare, BuildingDeliver, DeliverBuilt,
                   CheckedOut, SvnStaged, Committing, PostCommit, RestoringBranch }
  pub struct Journal { /* 见下 */ }
  pub fn journal_path(repo_root: &str) -> std::path::PathBuf;
  pub fn write(repo_root: &str, j: &Journal) -> Result<()>;
  pub fn read(repo_root: &str) -> Result<Option<Journal>>;
  pub fn remove(repo_root: &str) -> Result<()>;
  ```

- [ ] **Step 1: 写失败测试**

`src-tauri/tests/svn_journal.rs`：

```rust
//! 写前 operation journal（spec §7.6）。
//!
//! 它不是"冲突时才写的状态文件"，而是**写前日志**：任何一步修改落地之前，
//! journal 必须已经在磁盘上。否则存在无法恢复的窗口——S1 完成 stash 之后、
//! 状态写盘之前崩溃，用户的 WIP 就只剩一个匿名 stash 条目，客户端重开后对它
//! 一无所知（§7.3.1）。

use app_lib::engine::svn_journal::{journal_path, read, remove, write, Flow, Journal, Phase};

fn git_dir_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".git")).unwrap();
    d
}

fn minimal(opid: &str) -> Journal {
    Journal {
        opid: opid.to_string(),
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: 1_757_000_000,
        orig_branch: Some("work".into()),
        orig_head: Some("e5d3f77".into()),
        ..Default::default()
    }
}

#[test]
fn a_journal_round_trips() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let j = minimal("opid-1");
    write(&root, &j).unwrap();
    assert_eq!(read(&root).unwrap().as_ref(), Some(&j));
}

#[test]
fn no_journal_is_none_not_an_error() {
    // "上次干净收尾了"与"读失败了"必须分得开：前者什么都不做，后者要停下来报错。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    assert_eq!(read(&root).unwrap(), None);
}

#[test]
fn a_corrupt_journal_is_an_error_not_a_default() {
    // **最要紧的一条。** 坏 JSON 静默退化成默认值，等于把用户 stash 的 OID
    // 悄悄丢掉——恢复代码会以为"上次什么都没做"，而磁盘上工作区已经被清空了。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    std::fs::write(journal_path(&root), b"{ this is not json").unwrap();

    let err = read(&root).expect_err("坏 journal 必须报错，不能当成没有");
    assert_eq!(err.code, "svn_journal_corrupt", "实际：{}", err.message);
    assert!(
        journal_path(&root).exists(),
        "报错之后必须把文件留在原地——它是唯一的现场"
    );
}

#[test]
fn writing_leaves_no_temp_file_behind() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    write(&root, &minimal("opid-2")).unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(fx.path().join(".git"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "留下了临时文件：{leftovers:?}");
}

#[test]
fn overwriting_never_leaves_a_half_written_file() {
    // 原子写的可观察后果：任何时刻读到的要么是旧的完整 JSON，要么是新的完整
    // JSON。用"写一个大 journal 覆盖小 journal，中间反复读"来逼近——读到的
    // 每一份都必须能解析。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    write(&root, &minimal("small")).unwrap();

    let mut big = minimal("big");
    big.svn_added = (0..2000).map(|i| format!("Assets/Generated/F{i}.cs")).collect();

    let r2 = root.clone();
    let t = std::thread::spawn(move || {
        for _ in 0..200 {
            // 每一次读都必须成功——读到半个文件就会在这里炸
            read(&r2).expect("并发读到了一个解析不了的 journal，说明写不是原子的");
        }
    });
    for _ in 0..50 {
        write(&root, &big).unwrap();
        write(&root, &minimal("small")).unwrap();
    }
    t.join().unwrap();
}

#[test]
fn every_phase_uses_the_exact_string_from_the_spec() {
    // 线序格式写错是**看不见的**：本地全绿，崩溃恢复时才发现读不回来。
    // 这些字符串逐字抄自 spec §7.6 的两张 phase 表。
    let cases = [
        (Phase::Start, "start"),
        (Phase::Stashing, "stashing"),
        (Phase::Stashed, "stashed"),
        (Phase::CapturingBaseline, "capturing-baseline"),
        (Phase::PublishingBaseline, "publishing-baseline"),
        (Phase::BaselineReady, "baseline-ready"),
        (Phase::Rebase, "rebase"),
        (Phase::Prepare, "prepare"),
        (Phase::BuildingDeliver, "building-deliver"),
        (Phase::DeliverBuilt, "deliver-built"),
        (Phase::CheckedOut, "checked-out"),
        (Phase::SvnStaged, "svn-staged"),
        (Phase::Committing, "committing"),
        (Phase::PostCommit, "post-commit"),
        (Phase::RestoringBranch, "restoring-branch"),
    ];
    for (phase, text) in cases {
        let encoded = serde_json::to_string(&phase).unwrap();
        assert_eq!(encoded, format!("\"{text}\""), "{phase:?} 的线序名不对");
    }
}

#[test]
fn removing_a_missing_journal_is_not_an_error() {
    // S11 / D10 收尾会无条件调它；失败路径上可能已经删过一次。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    remove(&root).expect("删一个不存在的 journal 不该报错");
}
```

- [ ] **Step 2: 跑测试确认编译失败**

```bash
cd src-tauri
cargo test --test svn_journal
```

Expected: `unresolved import app_lib::engine::svn_journal`。

- [ ] **Step 3: 实现 `svn_journal.rs`**

```rust
//! 写前 operation journal（spec §7.6）。`sync_in` 与 `sync_out` 共用。
//!
//! # 「写前」是字面意思
//!
//! 任何一步修改落地**之前**，journal 必须已经在磁盘上。spec §7.3.1 记了反例：
//! 原来的步骤顺序把 `git stash push` 排在写 journal 之前，崩在那条缝里，用户的
//! WIP 就变成一个匿名 stash 条目躺在 reflog 里，客户端重开后对它一无所知。
//!
//! # 所有权由锁界定
//!
//! 拿到 `svn_lock` 才允许读写本文件（§5.1.1）。没有这条，两个进程会互相覆盖
//! journal，被覆盖那一方的 stash OID 全部丢失。本模块**不自己去拿锁**——
//! 那会让"谁持有锁"这件事散到两处；调用方（Task 5 之后的 `sync_in`）负责。

use crate::error::{GitError, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

const JOURNAL_FILE: &str = "mygitclient-sync.json";

/// 哪条流程留下的 journal。恢复入口据此分流（§7.6：`publishing-baseline` 是两个
/// 流程共用的阶段名，但父提交与返回入口完全不同，不能套用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    SyncIn,
    SyncOut,
}

/// 流程停在哪一步。字符串**逐字**取自 spec §7.6 的两张表，不要"优化"拼写：
/// 它是落在用户磁盘上的线序格式，改一个字符就读不回上一版写的 journal。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    // —— sync_in ——
    Start,
    Stashing,
    Stashed,
    Rebase,
    // —— 两个流程共用的录制阶段 ——
    CapturingBaseline,
    PublishingBaseline,
    BaselineReady,
    // —— sync_out ——
    Prepare,
    BuildingDeliver,
    DeliverBuilt,
    CheckedOut,
    SvnStaged,
    Committing,
    PostCommit,
    RestoringBranch,
}

impl Default for Phase {
    fn default() -> Self {
        Phase::Start
    }
}

impl Default for Flow {
    fn default() -> Self {
        Flow::SyncIn
    }
}

/// journal 的全部字段（spec §7.6 的 JSON 样例）。
///
/// `sync_out` 专用字段现在就定义：本模块拥有这个文件格式，等 Phase 3 再加会让
/// 两期写出的 journal 互相读不了。**空值一律 `skip_serializing_if`**，
/// 让 `sync_in` 写出来的文件保持干净可读。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Journal {
    pub opid: String,
    pub flow: Flow,
    pub phase: Phase,
    pub started_at: u64,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orig_head: Option<String>,
    /// push **之前**的 `refs/stash`。负责区分"push 没落地"和"push 落地了但
    /// reflog 被外部 `stash clear` 抹掉了"——后者要报错停下，不能当成没事
    /// 发生（§7.3.1）。`None` 表示当时还没有 stash。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash_ref_before: Option<String>,
    /// **不可变 OID**，绝不是 `stash@{N}`（§7.6.1：那是栈位置，不是身份）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stash_oid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_rev_before: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_oid_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capture_tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_baseline_oid: Option<String>,

    // —— 以下只有 sync_out 用（§7.6 末尾三段）——
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub svn_added: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub svn_deleted: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub new_parent_dirs: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub local_only_commits: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deliver_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_worktree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_start_rev: Option<u64>,
}

pub fn journal_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join(JOURNAL_FILE)
}

/// 原子写入：临时文件 → `sync_all` → rename（§7.6：「半个 JSON 比没有 JSON 更糟」）。
///
/// 临时文件名带 pid + 纳秒，理由同 `svn_attributes::write_attributes`：固定名字
/// 在并发下会互相截断对方写到一半的内容。
pub fn write(repo_root: &str, j: &Journal) -> Result<()> {
    let target = journal_path(repo_root);
    let dir = target.parent().expect("journal 路径一定有父目录");
    std::fs::create_dir_all(dir)?;

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(
        "{JOURNAL_FILE}.{}.{nanos}.tmp",
        std::process::id()
    ));

    let body = serde_json::to_vec_pretty(j)
        .map_err(|e| GitError::new("svn_journal_encode", format!("journal 序列化失败: {e}")))?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        // rename 是原子的，但「新内容已经落盘」不是它保证的。
        // 少了这一步，断电后可能 rename 生效了而内容还是空的。
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换 journal 失败: {e}"))
    })?;
    Ok(())
}

/// 读 journal。
///
/// **三种结果必须分开**：没有（上次干净收尾）、有且能解析、有但坏了。
/// 第三种绝不能退化成第一种——那等于把用户 stash 的 OID 悄悄丢掉，而恢复代码
/// 会以为"上次什么都没做"，磁盘上工作区却已经被清空了。
pub fn read(repo_root: &str) -> Result<Option<Journal>> {
    let p = journal_path(repo_root);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        GitError::new(
            "svn_journal_corrupt",
            format!(
                "上次同步留下的记录（{}）读不出来：{e}。\
                 文件已原样保留——它是判断上次停在哪里的唯一现场，请勿删除，\
                 先把它交给开发者。",
                p.display()
            ),
        )
    })
}

/// 删除 journal。不存在也算成功——收尾路径会无条件调它。
pub fn remove(repo_root: &str) -> Result<()> {
    match std::fs::remove_file(journal_path(repo_root)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
```

在 `engine/mod.rs` 按字母序加：

```rust
pub mod svn_journal;
```

- [ ] **Step 4: 跑测试**

```bash
cd src-tauri
cargo test --test svn_journal
```

Expected: `7 passed`。

- [ ] **Step 5: 变异验证 —— 让坏 journal 静默退化**

把 `read` 的错误分支临时改成 `Ok(None)`。

Expected: 只有 `a_corrupt_journal_is_an_error_not_a_default` 变红。改回去。

- [ ] **Step 6: 变异验证 —— 去掉原子写**

把 `write` 临时改成直接 `std::fs::write(&target, &body)`。

Expected: `overwriting_never_leaves_a_half_written_file` 变红（并发读会撞上被截断的文件）。改回去。

> 若它没变红，**不要**就此放过：说明并发窗口没撞上。把 `big` 的条目数从 2000
> 提到 20000、循环次数加倍再试，并把实测的"多少轮里红几次"记进决策日志——
> `tests/svn_init.rs::two_concurrent_inits_…` 已有这个先例（轮数是量出来的，
> 不是拍的）。若仍然撞不上，就把这条测试删掉并在决策日志里如实说明它测不到，
> **不要留一条永远绿的测试冒充覆盖**（`AGENTS.md` §13 rubric #9）。

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/engine/svn_journal.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_journal.rs
git commit -m "feat(svn): 写前 operation journal 的读写与原子落盘"
```

---

## Task 5: 同步期间挂起文件监听

**Files:**
- Modify: `src-tauri/src/commands/watch.rs`
- Create: `src-tauri/tests/svn_watch_suspend.rs`

**Interfaces:**
- Consumes: 无
- Produces:
  ```rust
  // commands/watch.rs
  pub fn suspend_watch(path: &str);
  pub fn resume_watch(path: &str);
  pub fn is_watch_suspended(path: &str) -> bool;
  ```

**背景（spec §5.2）：** `svn update` 一次改动成百上千个文件。现有 watcher 是 300ms 尾沿去抖，流程跑到一半就会发 `repo-changed`，前端刷新后读到「已切到基线分支但还没落新基线」的中间态，界面上会闪出「改动全没了」的假象。

- [ ] **Step 1: 写失败测试**

`src-tauri/tests/svn_watch_suspend.rs`：

```rust
//! 同步期间挂起 watcher（spec §5.2）。
//!
//! 挂起状态本身是纯逻辑，单独测得了；"去抖线程真的查了它"由 Task 5 Step 4 的
//! 源码断言守着——起一个真 watcher 线程需要 `AppHandle`，那在集成测试里拿不到。

use app_lib::commands::watch::{is_watch_suspended, resume_watch, suspend_watch};

#[test]
fn suspending_and_resuming_one_repo_does_not_touch_another() {
    let a = "C:/repos/alpha";
    let b = "C:/repos/beta";
    assert!(!is_watch_suspended(a));

    suspend_watch(a);
    assert!(is_watch_suspended(a));
    assert!(!is_watch_suspended(b), "挂起一个仓库不该影响另一个");

    resume_watch(a);
    assert!(!is_watch_suspended(a));
}

#[test]
fn suspension_is_path_normalized() {
    // watcher 自己按归一化路径登记（`watch_repo` 里的 `normalize`）。挂起这边
    // 若不归一化，Windows 上 `C:\repo` 与 `C:/repo` 会变成两个键，挂起就静默
    // 失效——而失效的表现只是"界面偶尔闪一下"，没人会去查。
    suspend_watch("C:\\repos\\gamma");
    assert!(is_watch_suspended("C:/repos/gamma"));
    resume_watch("C:/repos/gamma");
    assert!(!is_watch_suspended("C:\\repos\\gamma"));
}

#[test]
fn resuming_something_never_suspended_is_harmless() {
    // 失败路径会无条件调 resume，可能 suspend 根本没跑到。
    resume_watch("C:/repos/never-suspended");
    assert!(!is_watch_suspended("C:/repos/never-suspended"));
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd src-tauri
cargo test --test svn_watch_suspend
```

Expected: 编译失败（函数不存在）。`lib.rs` 里 `pub mod commands;` 已经有了，不需要改。

- [ ] **Step 3: 实现挂起集合**

在 `src-tauri/src/commands/watch.rs` 的 `WATCHERS` 旁边加：

```rust
/// 同步期间被挂起的仓库（归一化路径）。
///
/// spec §5.2：`svn update` 一次改动成百上千个文件，300ms 去抖压不住；流程跑到
/// 一半发出的 `repo-changed` 会让前端读到「已切到基线分支但还没落新基线」的
/// 中间态，界面上闪出「改动全没了」的假象。
///
/// 用"挂起"而不是"停掉再重建"：停掉会丢掉引用计数（多个标签页共用一条线程），
/// 重建还要重新遍历目录树。
static SUSPENDED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn suspended() -> &'static Mutex<HashSet<String>> {
    SUSPENDED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 挂起某个仓库的刷新事件。**同步流程开始时调**（spec §5.2）。
pub fn suspend_watch(path: &str) {
    if let Ok(mut s) = suspended().lock() {
        s.insert(normalize(path));
    }
}

/// 恢复刷新事件。调用方在恢复之后**必须主动发一次 `repo-changed`**
/// （§5.2），否则挂起期间攒下的那一大批改动没有任何事件去告诉前端。
pub fn resume_watch(path: &str) {
    if let Ok(mut s) = suspended().lock() {
        s.remove(&normalize(path));
    }
}

pub fn is_watch_suspended(path: &str) -> bool {
    suspended()
        .lock()
        .map(|s| s.contains(&normalize(path)))
        .unwrap_or(false)
}
```

顶部 `use` 补 `HashSet`：

```rust
use std::collections::{HashMap, HashSet};
```

去抖线程里，把发事件那一段（现有代码 `if pending && last_change.elapsed() >= DEBOUNCE {`）改成：

```rust
            if pending && last_change.elapsed() >= DEBOUNCE {
                pending = false;
                // 挂起期间**丢弃**事件而不是攒着：同步结束时调用方会主动发一次，
                // 攒着只会在恢复的瞬间连发几百条。
                if !is_watch_suspended(&path) {
                    // 带上路径，前端各标签据此只刷新自己那个仓库
                    let _ = handle.emit("repo-changed", &path);
                }
            }
```

- [ ] **Step 4: 加一条源码断言，守住"去抖线程真的查了它"**

追加到 `src-tauri/tests/svn_watch_suspend.rs`：

```rust
#[test]
fn the_debounce_loop_actually_checks_the_suspension() {
    // 上面三条测的是集合本身。集合正确但没人查它，是这个改动最可能的失效方式，
    // 而它在运行时完全无声——只表现为"同步时界面偶尔闪一下"。
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/watch.rs"),
    )
    .unwrap();
    let emit = src
        .find("emit(\"repo-changed\"")
        .expect("找不到发事件那一句——改名了？");
    let window = &src[emit.saturating_sub(400)..emit];
    assert!(
        window.contains("is_watch_suspended"),
        "发 repo-changed 之前没有查挂起状态（spec §5.2）"
    );
}
```

- [ ] **Step 5: 跑测试**

```bash
cd src-tauri
cargo test --test svn_watch_suspend
cargo fmt --check && cargo clippy --all-targets
```

Expected: `4 passed`，clippy 0 告警。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/commands/watch.rs src-tauri/tests/svn_watch_suspend.rs
git commit -m "feat(svn): 同步期间挂起文件监听，避免中间态刷新"
```

---

## Task 6: 「上次同步未完成」的检测与展示

**Files:**
- Modify: `src-tauri/src/commands/svn.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src/engine/svnJournal.ts`
- Create: `tests/vitest/svnJournal.test.ts`
- Modify: `src/store/repoStore.ts`
- Modify: `src/components/BaselineBanner.tsx`
- Modify: `src/components/RepoTab.tsx`
- Modify: `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`

**Interfaces:**
- Consumes: Task 1 的 `holder` / `is_held`，Task 4 的 `read`
- Produces:
  ```rust
  #[derive(serde::Serialize)] #[serde(rename_all = "camelCase")]
  pub struct SyncState {
      pub locked: bool,
      pub holder: Option<LockHolder>,
      pub unfinished: Option<UnfinishedSync>,
  }
  #[derive(serde::Serialize)] #[serde(rename_all = "camelCase")]
  pub struct UnfinishedSync { pub flow: Flow, pub phase: Phase, pub opid: String, pub started_at: u64 }
  #[tauri::command(async)] pub fn svn_sync_state(path: String) -> Result<SyncState, GitError>
  ```
  ```ts
  export interface SyncState { locked: boolean; holder: LockHolder | null; unfinished: UnfinishedSync | null }
  export function describeUnfinishedSync(s: SyncState | null): BannerModel | null
  ```

- [ ] **Step 1: 写前端失败测试**

`tests/vitest/svnJournal.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import {
  describeUnfinishedSync,
  otherBannersAreSuppressed,
  type SyncState,
} from '@/engine/svnJournal'

const clean: SyncState = { locked: false, holder: null, unfinished: null }

describe('describeUnfinishedSync', () => {
  it('还没读到状态时不挂任何东西', () => {
    // 打开仓库那一瞬间它是 null。挂一条横幅等于每次打开都闪一下。
    expect(describeUnfinishedSync(null)).toBeNull()
  })

  it('干净时不挂', () => {
    expect(describeUnfinishedSync(clean)).toBeNull()
  })

  it('正在同步时说的是"进行中"，不是"未完成"', () => {
    // 这两件事的出口完全相反：进行中要等，未完成要用户决定继续还是放弃。
    // 说错了会引导用户去"放弃"一次正在正常跑的同步。
    const b = describeUnfinishedSync({
      locked: true,
      holder: { pid: 4242, startedAt: 1757000000, opid: 'abc' },
      unfinished: { flow: 'sync_in', phase: 'rebase', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b).not.toBeNull()
    expect(b.title).toContain('正在进行')
    expect(b.actions).toEqual([])
    expect(b.detail).toContain('4242')
  })

  it('锁没人拿着但 journal 还在 ⇒ 上次没收尾', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.tone).toBe('danger')
    expect(b.title).toContain('未完成')
  })

  it('stashed 阶段必须告诉用户改动被压进了 stash', () => {
    // spec §7.3.1：这正是写前日志存在的理由。用户看到工作区空了，
    // 界面必须立刻说清东西在哪，而不是让他以为改动没了。
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.detail).toContain('stash')
  })

  it('本期还没有继续/放弃按钮，但必须说清这一点', () => {
    // 半个功能比没有功能更糟的地方在于让用户干等。这一期只能检测和展示，
    // 那就明说，别挂两颗点不动的按钮。
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'start', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.actions).toEqual([])
    expect(b.note).toBeTruthy()
  })

  it('未收尾或进行中时，其余横幅一律让位', () => {
    // 那两种状态下仓库停在中间态，别的横幅的结论可能是错的，而它们的出口
    // （重建基线）此刻是危险的——会移动正被依赖的基线指针。
    expect(otherBannersAreSuppressed(null)).toBe(false)
    expect(otherBannersAreSuppressed(clean)).toBe(false)
    expect(
      otherBannersAreSuppressed({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'x', startedAt: 1 },
      }),
    ).toBe(true)
    expect(
      otherBannersAreSuppressed({ locked: true, holder: null, unfinished: null }),
    ).toBe(true)
  })

  it('每个 phase 都有自己的说明，不能落到一句通用话上', () => {
    // 通用文案等于没说。用户要知道的恰恰是"我的东西现在在哪"。
    const phases = [
      'start', 'stashing', 'stashed', 'rebase',
      'capturing-baseline', 'publishing-baseline', 'baseline-ready',
    ] as const
    const seen = new Set<string>()
    for (const phase of phases) {
      const b = describeUnfinishedSync({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in', phase, opid: 'x', startedAt: 1 },
      })!
      expect(b.detail, `phase ${phase} 没有说明`).toBeTruthy()
      seen.add(b.detail)
    }
    expect(seen.size, '有 phase 共用了同一句说明').toBe(phases.length)
  })
})
```

- [ ] **Step 2: 跑测试确认失败**

```bash
pnpm test
```

Expected: `Cannot find module '@/engine/svnJournal'`。

- [ ] **Step 3: 实现后端命令**

`src-tauri/src/commands/svn.rs` 末尾：

```rust
/// 同步状态：现在有没有人在跑，以及上次有没有留下没收尾的 journal。
///
/// **纯读**，且不需要拿锁——它恰恰是用来观察锁的。这也是 spec §5.1
/// 「读操作不阻塞，但要能返回『同步进行中』状态供 UI 显示」那一条的落点。
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
```

外加 `SyncState` / `UnfinishedSync` 两个结构体（签名见本 Task 的 Interfaces），放在同文件里。在 `src-tauri/src/lib.rs` 的 `invoke_handler!` 里按字母序注册 `svn_sync_state`。

> ⚠️ `svn_journal::read` 返回 `Err` 时**照常往上抛**。坏 journal 必须让用户看见，
> 不能在这里吞成 `None`——那正是 Task 4 那条测试守着的事。

- [ ] **Step 4: 实现前端纯逻辑**

`src/engine/svnJournal.ts`：

```ts
/**
 * 同步状态 → 横幅（spec §7.6 末尾「打开仓库时发现它存在 → 界面顶部常驻
 * 『上次同步未完成』」）。
 *
 * 纯逻辑，无 React 依赖（`AGENTS.md` §7.2）。
 */
import type { BannerModel } from './svnSync'

/** 后端 `svn_lock::LockHolder`。**只用于显示**，不是存活判据。 */
export interface LockHolder {
  pid: number
  startedAt: number
  opid: string
}

/** 后端 `svn_journal::Phase` 的线序名，逐字对应 spec §7.6 的两张表。 */
export type SyncPhase =
  | 'start'
  | 'stashing'
  | 'stashed'
  | 'rebase'
  | 'capturing-baseline'
  | 'publishing-baseline'
  | 'baseline-ready'

export interface UnfinishedSync {
  flow: 'sync_in' | 'sync_out'
  phase: SyncPhase
  opid: string
  startedAt: number
}

export interface SyncState {
  locked: boolean
  holder: LockHolder | null
  unfinished: UnfinishedSync | null
}

/**
 * 每个 phase 回答同一个问题：**我的东西现在在哪。**
 *
 * 通用文案等于没说。用户在这一刻看到的是一个工作区可能空了的仓库，
 * 他要知道的不是「出错了」，是「改动还在不在」。
 */
const PHASE_DETAIL: Record<SyncPhase, string> = {
  start:
    '流程刚开始，还没有动过工作目录里的任何东西——可以直接当作没发生过。',
  stashing:
    '正要把未提交的改动压进 stash，但压没压成还不确定。' +
    '**不要自己去 apply 任何 stash 条目**：挑错条目会把别人的 WIP 覆盖到你的工作区上。',
  stashed:
    '未提交的改动已经压进 stash，它的 OID 已经记下来了，东西没丢。' +
    '工作目录看起来是空的，那是正常的。',
  'capturing-baseline':
    '正在录新基线。旧基线还在原处，磁盘上的文件一个都没动。',
  'publishing-baseline':
    '新基线可能已经发布，但工作分支还没恢复到原来那条。',
  'baseline-ready':
    '新基线已经就位，只差切回你原来的分支。',
  rebase:
    '停在 git 自己的 rebase 冲突态上。冲突文件可以在「本地改动」里看到，' +
    '这部分状态由 git 自己记着，没有丢失风险。',
}

const NOTE_NEXT_PHASE =
  '本期只能告诉你停在哪里；「继续」与「放弃」两个入口随更新流程一起到来。' +
  '在那之前请不要手工去动 stash 或分支指针——记录还在，等入口到了就能接着走。'

/**
 * `null`（还没读到）与「干净」都不挂横幅。
 *
 * 打开仓库那一瞬间状态是 `null`，那时挂任何东西都等于每次打开都闪一下。
 */
export function describeUnfinishedSync(s: SyncState | null): BannerModel | null {
  if (!s) return null

  // 「正在进行」与「未完成」的出口完全相反：前者要等，后者要用户决定。
  // 说反了会引导用户去「放弃」一次正在正常跑的同步。
  if (s.locked) {
    const who = s.holder
      ? `持有者是进程 ${s.holder.pid}。`
      : '还读不到持有它的进程信息（它可能刚刚开始）。'
    return {
      tone: 'warn',
      title: '这个工作副本正在进行一次同步',
      detail:
        `${who}同步期间暂存、提交、切换分支等写操作会被拒绝，` +
        '查看状态和 diff 不受影响。',
      actions: [],
      note: '如果你确定没有任何同步在跑，那是上次异常退出留下的痕迹，重开应用即可。',
    }
  }

  if (!s.unfinished) return null

  return {
    tone: 'danger',
    title: '上次同步没有正常结束',
    detail: PHASE_DETAIL[s.unfinished.phase],
    actions: [],
    note: NOTE_NEXT_PHASE,
  }
}
```

- [ ] **Step 5: 跑前端测试**

```bash
pnpm test
```

Expected: 8 条全绿。

- [ ] **Step 6: 接线 store 与横幅**

`src/store/repoStore.ts` —— 与既有的 `svnEolStatus` / `refreshSvnEol`（约 243 行、
462 行）逐行同形，照抄那一份的结构：

```ts
export function svnSyncState(path: string): Promise<SyncState> {
  return invoke('svn_sync_state', { path })
}
```

状态字段（与 `svnEol: EolStatus | null` 并列，约 126 行）与初始值（约 168 行）：

```ts
  svnSyncState: SyncState | null
```
```ts
  svnSyncState: null,
```

刷新函数（与 `refreshSvnEol` 并列）：

```ts
  const refreshSvnSyncState = async (path: string) => {
    try {
      const st = await svnSyncState(path)
      // `samePath` 守卫：用户可能在 await 期间切走了标签页，
      // 把结果写进另一个仓库的状态里会让横幅挂在错的仓库上。
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnSyncState: st } : s))
    } catch {
      // 读不到就当没有。**不要**在这里把错误吞成"干净"以外的样子——
      // 坏 journal 的报错由后端抛上来，交给通用错误提示。
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnSyncState: null } : s))
    }
  }
```

在打开仓库的路径里（约 361 行 `await refreshSvnEol(path)` 旁边）加
`await refreshSvnSyncState(path)`；非 SVN 分支（约 374 行）加
`svnSyncState: null`；并在 return 的方法表（约 648 行）里导出
`refreshSvnSyncState`。

`src/components/BaselineBanner.tsx` —— 加 prop 并**排在所有其它横幅之前**渲染：

```tsx
interface BaselineBannerProps {
  isSvn: boolean
  sync: SyncStatus | null
  audit: ServerAudit | null
  eol: EolStatus | null
  /** 同步锁与上次未收尾的 journal（spec §5.1、§7.6）。`null` = 还没读到。 */
  syncState: SyncState | null
  onRebuild: () => void
  onCheck: () => void
  onMigrateEol: () => void
}
```

组件里**没有**通用的 `<Banner>` 子组件，每条横幅是一段内联 JSX；让位也不是
靠包一层 `{cond && …}`，而是**把别的 model 算成 `null`**（现有
`syncBannerIsSuppressed` 就是这个写法）。照这个来：

先在 `svnJournal.ts` 里加一个与 `syncBannerIsSuppressed` 同形的判据：

```ts
/**
 * 未收尾 / 同步进行中时，其余横幅一律让位。
 *
 * 那两种状态下仓库停在中间态：失同步横幅会拿一个半推进的基线去比 revision，
 * 行尾横幅会对着一个还没录完的基线报「需要迁移」。结论都可能是错的，而它们
 * 的出口（重建基线）在这种状态下是**危险**的——重建会移动基线指针，而此刻
 * 正有另一套流程（或它的残骸）依赖着那个指针。
 */
export function otherBannersAreSuppressed(s: SyncState | null): boolean {
  return describeUnfinishedSync(s) !== null
}
```

`BaselineBanner.tsx` 的计算段（现有约 69–73 行）改成：

```tsx
  const unfinished = describeUnfinishedSync(syncState)
  const hushed = otherBannersAreSuppressed(syncState)
  const eolModel = hushed ? null : describeEolMigration(eol, sync)
  const model = hushed || syncBannerIsSuppressed(eol, sync) ? null : describeSync(sync)
  const auditModel = hushed ? null : describeAudit(audit)
  const standalone = !hushed && needsStandaloneCheck(isSvn, sync, eol)

  if (!unfinished && !eolModel && !model && !auditModel && !standalone) return null
```

再在 `return (<>` 之后、`{eolModel && (` **之前**插入这一段（形状照抄 `eolModel`
那一块，去掉按钮——本期 `actions` 恒为空）：

```tsx
      {unfinished && (
        <div className={`banner-baseline ${unfinished.tone}`}>
          <div className="banner-baseline-head">
            <strong>{unfinished.title}</strong>
          </div>
          <p className="banner-baseline-detail">{unfinished.detail}</p>
          {unfinished.note && (
            <p className="banner-baseline-note">{unfinished.note}</p>
          )}
        </div>
      )}
```

`src/components/RepoTab.tsx` —— 传 `syncState={state.svnSyncState}`。

- [ ] **Step 7: 前后端一起验**

```bash
pnpm build && pnpm test
cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test
```

- [ ] **Step 8: 在真实窗口里验 invoke 真的通**

`AGENTS.md` §7.3 硬性要求：新增 Tauri 命令的 `snake_case` ↔ `camelCase` 转换没有编译期检查，必须实跑。

```bash
pnpm tauri dev
```

手工造现场（在一个 fixture 混合仓库里）：

```powershell
# 1. 造一个"上次没收尾"的 journal
@'
{"opid":"manual-test","flow":"sync_in","phase":"stashed","startedAt":1757000000,"origBranch":"work"}
'@ | Set-Content -Encoding utf8 <FIX>\wc1\.git\mygitclient-sync.json
```

打开该仓库，应当看到红色横幅「上次同步未完成」，说明里提到 stash。删掉该文件重开，横幅消失。

- [ ] **Step 9: 把「与 spec 的偏离」写回 spec**

本计划头部那条偏离（存活判据从 pid 探测改成独占打开）必须落进 spec，否则下一个人会照着 §5.1.1 再写一遍 pid 探测：

- 改 §5.1.1 的「陈旧锁」那一段，把判据换掉，并保留 pid / startedAt 作为**展示**字段
- 在 §18 修订记录加一条，附本计划头部那张实测表

- [ ] **Step 10: 提交**

```bash
git add -A
git commit -m "feat(svn): 同步状态命令与「上次同步未完成」横幅"
```

---

## 收尾

- [ ] 跑全量门槛：`cargo fmt --check` / `cargo clippy --all-targets` / `cargo test` / `pnpm build` / `pnpm test`
- [ ] 补 E2E 清单 `docs/superpowers/e2e/2026-09-25-svn-phase2b-e2e.md`。**本期验收不是「能更新」**——`sync_in` 在 2c。本期的真实窗口场景只有三个：手写 journal 出横幅、删掉 journal 横幅消失、手工造一把活锁后 stage 被拒且 status 仍可用
- [ ] 决策日志四类条目补齐（偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的）
- [ ] 按 `AGENTS.md` §4.1 开 **Draft** PR，并跑一次**独立于实现者**的 review

---

## 自审记录

**Spec 覆盖：** §5.1（拒绝而非排队、读不阻塞）→ Task 3；§5.1.1（跨进程、O_EXCL、陈旧接管、journal 所有权）→ Task 1/2/4；§5.2（watcher 挂起 + 恢复后主动发事件）→ Task 5；§7.6（原子写、phase 先行、正常收尾删除、打开时常驻提示）→ Task 4/6；§7.6.1（按不可变 OID 记 stash）→ Task 4 的 `stash_oid` 字段与注释；§13「拿不到跨进程锁」「任意步骤客户端崩溃」两行 → Task 6。

**本期有意不做**（属 2c / 2d，不是遗漏）：S0–S11 编排、前置检查七条、§7.3.2 merge 门槛、冲突解决 UI、继续 / 放弃两个入口的**执行**（本期只检测与展示）、§9.4 恢复向导。

**类型一致性：** `LockHolder` 在 Rust 侧 `camelCase` 序列化（`startedAt`），前端 `svnJournal.ts` 的 interface 与之对齐；`Phase` 的线序字符串由 Task 4 的 `every_phase_uses_the_exact_string_from_the_spec` 钉死，前端 `svnJournal.ts` 的 phase 字面量类型必须用同一批字符串。
