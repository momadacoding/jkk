//! 仓库级跨进程同步锁（spec §5.1、§5.1.1）。
//!
//! # 为什么不能用 `std::sync::Mutex`
//!
//! 它的作用域只到进程边界。本仓库**没有** `tauri-plugin-single-instance`，
//! 同一个工作副本上完全可能开着两个 mygitclient 进程；两个进程各拿各的进程内
//! 锁之后，会同时写 `.git/mygitclient-sync.json`，后写的把前一个的 journal
//! 整个覆盖——被覆盖那一方的 stash OID 全部丢失，等于失去恢复依据。
//!
//! # 判据是「能不能独占打开」，不是「pid 还在不在」
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
//! pid，会让 pid 探测把残锁永远判成「活着」，用户被永久卡死。
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
    /// Unix 秒。用于「从什么时候开始」的展示——**不能**拿它当超时判据：
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
///
/// `Debug` 是测试要的：`acquire(...).expect_err(...)` 要求 `Ok` 那一侧可打印。
#[derive(Debug)]
pub struct SyncLock {
    /// 这个句柄本身就是锁：它一直开着，别人就独占打不开；进程没了，
    /// 操作系统替我们关掉。**不要提前 drop 它**。
    ///
    /// 包成 `Option` 只为了 `Drop` 里能 `take()` 出来先关掉——理由见 `Drop` 的注释。
    file: Option<std::fs::File>,
    path: PathBuf,
}

/// 以「必须能独占」的方式打开一个**已存在**的文件。
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
    // 于是**永远报「打得开」**，也就是永远把残锁当成可接管——
    // 由 `is_held` 那边的注释说明这个降级，不在这里假装安全。
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}

/// 探测结果三态。**「判不出来」必须与「没人持有」分开**（2026-09-25 独立 review）。
///
/// 合成布尔的话只能二选一，而两个方向都错：当成「没人持有」会在一次真实的同步
/// 进行中放行写操作；当成「有人持有」会让接管路径抢走一把可能真有人在用的锁。
/// 分开之后，**守卫**可以保守地关门，**接管**可以保守地不动手。
pub enum Liveness {
    /// 确有进程持有（共享冲突）。
    Held,
    /// 确实没人持有：文件不存在，或独占打得开（= 残锁）。
    Free,
    /// 判不出来。权限、网络盘、ACL 等——不知道就是不知道，不许猜。
    Unknown(std::io::Error),
}

/// 现在是不是真的有进程持有这把锁？
///
/// 注意与「锁文件存不存在」的区别：崩溃之后文件还在，但没人持有（实测）。
///
/// ⚠️ 非 Windows 上拿不到跨进程语义，`exclusive_open` 恒能打开，于是这里恒为
/// `Free`（即「谁都能接管」）。本项目主力平台是 Windows（`AGENTS.md` §10），
/// 与 `file_lock.rs` 的降级方式一致。
pub fn probe(repo_root: &str) -> Liveness {
    let p = lock_path(repo_root);
    if !p.exists() {
        return Liveness::Free;
    }
    match exclusive_open(&p) {
        Ok(_) => Liveness::Free, // 独占拿得到 ⇒ 没人持有 ⇒ 残锁
        Err(e) => match e.raw_os_error() {
            // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION，
            // 与 `engine/file_lock.rs::is_occupied` 同一套判据。
            Some(32) | Some(33) => Liveness::Held,
            _ => Liveness::Unknown(e),
        },
    }
}

/// 「有人持有吗」的**保守**答案：判不出来时算有人持有。
///
/// 守卫（[`assert_not_syncing`]）和界面显示都用它。拿不准时关门的代价是用户多
/// 等一次、看到一条可能多余的提示；开门的代价是一次真实同步跑到一半被写操作
/// 从中间插进来。两者不对等。
pub fn is_held(repo_root: &str) -> bool {
    !matches!(probe(repo_root), Liveness::Free)
}

/// 只读打开，且**允许**别的进程同时读写。
///
/// 少了「允许写者」这一半就读不到：持锁者是以读写方式开着的，而一个只允许
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

/// 读持锁者信息。持锁期间也读得到——读的时候**允许**已有的写者存在。
pub fn holder(repo_root: &str) -> Option<LockHolder> {
    let mut f = permissive_open(&lock_path(repo_root)).ok()?;
    let mut s = String::new();
    f.read_to_string(&mut s).ok()?;
    serde_json::from_str(&s).ok()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 「原子创建且必须不存在」+ 独占共享模式。两件事必须在**同一次**打开里完成：
/// 分成「先 create_new 再重开成独占」的话，中间那条缝足以让另一个进程插进来。
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

/// 拿锁。拿不到**立刻失败**，不排队（spec §5.1：排队会让用户点了没反应、
/// 几秒后突然生效，比明确报错更糟）。
pub fn acquire(repo_root: &str, opid: &str) -> Result<SyncLock> {
    let path = lock_path(repo_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }

    // 先按「必须不存在」创建（§5.1.1 的 O_EXCL / CREATE_NEW 语义）。
    let file = match create_new_exclusive(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // 文件在。真有人拿着，还是上次崩溃留下的？
            match probe(repo_root) {
                Liveness::Held => return Err(busy_error(repo_root)),
                // 判不出来就**不接管**。抢走一把可能真有人在用的锁，比多报一次
                // 「正忙」危险得多（2026-09-25 独立 review N2）。
                Liveness::Unknown(io) => {
                    return Err(GitError::new(
                        "sync_lock_unreadable",
                        format!(
                            "这个工作副本上有一个同步锁文件（{}），但判断不出它是不是\
                             还有人在用（{io}）。为安全起见这次不接管它。\
                             常见成因：文件被设成只读、权限不足、或它在网络盘上。",
                            path.display()
                        ),
                    ))
                }
                Liveness::Free => {}
            }
            // 残锁：接管。§5.1.1 要求接管必须写日志并告知用户。
            log::warn!(
                "接管了一把无人持有的同步锁（上次同步没有正常收尾）：{}，原持有者 {:?}",
                path.display(),
                holder(repo_root)
            );
            std::fs::remove_file(&path)?;
            match create_new_exclusive(&path) {
                Ok(f) => f,
                // 另一个进程在我们删完之后抢先建了——它赢了，如实报「正忙」而不是
                // 把裸 io 错误抛给用户（2026-09-25 独立 review N11）。
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(busy_error(repo_root))
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(e) => return Err(e.into()),
    };

    // ⚠️ **先把 guard 建起来，再写元数据。** 顺序反过来的话，写元数据这几步里
    // 任何一个 `?` 早退都会让**文件留在磁盘上而没有 guard 去删它**——下一次
    // `acquire` 于是打出「上次同步没有正常收尾」，又是一条永远为假的告警
    //（2026-09-25 独立 review N1；与 Task 1 修的那个 Drop bug 同一类后果）。
    let mut guard = SyncLock {
        file: Some(file),
        path,
    };

    let meta = LockHolder {
        pid: std::process::id(),
        started_at: now_secs(),
        opid: opid.to_string(),
    };
    // `GitError` **没有** `From<serde_json::Error>`（`error.rs` 只实现了 git2 与
    // io 两种），所以这里必须 `map_err`，不能用 `?`。与 `svn_config::write_config`
    // 同一写法。
    let body = serde_json::to_string(&meta)
        .map_err(|e| GitError::new("svn_lock", format!("锁元数据序列化失败: {e}")))?;
    // 元数据写进句柄本身，不另开一次——另开就要考虑「开不开得了」的分支，
    // 而我们此刻正独占着它。
    let f = guard.file.as_mut().expect("刚建好，一定是 Some");
    f.rewind()?;
    f.write_all(body.as_bytes())?;
    f.flush()?;

    Ok(guard)
}

/// 写操作的守卫。**拒绝而不是排队**（spec §5.1）。
///
/// 读操作不得调用它：同步期间界面还要照常显示状态，把读也挡掉等于在最需要看清
/// 状况的时候把灯关了。`tests/svn_write_guard.rs` 两个方向都守着。
///
/// # ⚠️ 它也会挡住**持锁者自己**
///
/// 判据是「这把锁在不在」，不是「我是不是持有者」。所以 2c 的 `sync_in` **不能**
/// 拿 `backend_for(..).stash_push(..)` / `.checkout(..)` 去干自己的活——那会被自己
/// 的锁挡回来。它应当直接调 `engine` 层（`run_git`），那一层本来就不经过守卫。
/// 这是有意的：守卫的职责是挡住**界面**发起的写，不是给编排层当便利函数。
pub fn assert_not_syncing(repo_root: &str) -> Result<()> {
    if is_held(repo_root) {
        return Err(busy_error(repo_root));
    }
    Ok(())
}

fn busy_error(repo_root: &str) -> GitError {
    let who = match holder(repo_root) {
        Some(h) => format!(
            "持有者：进程 {}，开始于 {}（opid {}）。",
            h.pid, h.started_at, h.opid
        ),
        None => "读不到持有者信息（它可能刚刚开始）。".to_string(),
    };
    GitError::new(
        "sync_in_progress",
        format!("这个工作副本上已经有一次同步正在进行。{who}"),
    )
}

impl Drop for SyncLock {
    fn drop(&mut self) {
        // ⚠️ **必须先关句柄再删文件，顺序不能反。**
        //
        // `Drop::drop` 在字段被释放**之前**运行，所以此刻 `file` 还开着；而我们
        // 拿它时没给 `FILE_SHARE_DELETE`（给了就等于允许别的进程删掉我们正持有
        // 的锁），于是 Windows 上 `remove_file` 会以 ERROR_SHARING_VIOLATION 失败。
        //
        // 2026-09-25 由变异测试抓到：失败被 `let _ =` 吞掉，锁文件于是**永远**
        // 留在磁盘上。功能上不致命（`is_held` 看的是句柄不是文件），但每一次
        // 正常同步结束后都会留下一个残锁，导致下一次 `acquire` 打出
        // 「上次同步没有正常收尾」的警告——一条永远为假的告警，比没有告警更糟。
        drop(self.file.take());
        // 到这里句柄已经关了，删得掉。仍然不 unwrap：Drop 里 panic 会在展开
        // 途中直接 abort，而删不掉的后果只是留个残锁，下次照常接管。
        let _ = std::fs::remove_file(&self.path);
    }
}
