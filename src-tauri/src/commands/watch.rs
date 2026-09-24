use crate::engine::path::normalize;
use crate::error::{GitError, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// 归一化路径 → (代号, 引用计数)。
///
/// 每个标签页监听自己的仓库；多个标签打开同一个路径时共用一条 watcher 线程，
/// 靠引用计数决定何时真正停掉。代号用来让被替换/停掉的旧线程自行退出。
static WATCHERS: OnceLock<Mutex<HashMap<String, (u64, usize)>>> = OnceLock::new();
static NEXT_GEN: AtomicU64 = AtomicU64::new(0);

fn watchers() -> &'static Mutex<HashMap<String, (u64, usize)>> {
    WATCHERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_err() -> GitError {
    GitError::new("mutex", "lock failed")
}

/// 同步期间被挂起的仓库（归一化路径）。
///
/// spec §5.2：`svn update` 一次改动成百上千个文件，300ms 去抖压不住；流程跑到
/// 一半发出的 `repo-changed` 会让前端读到「已切到基线分支但还没落新基线」的
/// 中间态，界面上闪出「改动全没了」的假象。
///
/// 用「挂起」而不是「停掉再重建」：停掉会丢掉引用计数（多个标签页共用一条线程），
/// 重建还要重新遍历目录树。
static SUSPENDED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn suspended() -> &'static Mutex<HashSet<String>> {
    SUSPENDED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 挂起某个仓库的刷新事件。**同步流程开始时调**（spec §5.2）。
///
/// 锁中毒时静默放弃：挂起失败的后果只是同步期间多刷新几次界面，而在这里 panic
/// 会把整个同步流程带走。
pub fn suspend_watch(path: &str) {
    if let Ok(mut s) = suspended().lock() {
        s.insert(normalize(path));
    }
}

/// 恢复刷新事件。调用方在恢复之后**必须主动发一次 `repo-changed`**（§5.2），
/// 否则挂起期间攒下的那一大批改动没有任何事件去告诉前端。
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

/// 变更静默这么久之后才发一次 `repo-changed`（尾沿去抖）。
const DEBOUNCE: Duration = Duration::from_millis(300);
/// 轮询 channel 的间隔，需要明显小于 DEBOUNCE 才能及时判定静默。
const TICK: Duration = Duration::from_millis(100);

/// 判断一个文件事件是否值得触发刷新。
///
/// 注意：路径要先归一化成正斜杠——Windows 上 notify 给出的是
/// `C:\repo\.git\index`，直接匹配 `/.git/` 永远不成立。
fn is_relevant(repo: Option<&git2::Repository>, root: &Path, target: &Path) -> bool {
    let s = target.to_string_lossy().replace('\\', "/");
    if s.contains("/.git/") || s.ends_with("/.git") {
        // git 内部：锁文件和对象库写入是高频噪声，refs / HEAD / index 才有意义
        return !(s.ends_with(".lock") || s.contains("/.git/objects/"));
    }
    // 工作区：跳过被 .gitignore 忽略的路径（node_modules、target、dist 等）
    match repo {
        Some(r) => target
            .strip_prefix(root)
            .map(|rel| !r.is_path_ignored(rel).unwrap_or(false))
            .unwrap_or(true),
        None => true,
    }
}

#[tauri::command(async)]
pub fn watch_repo(handle: AppHandle, path: String) -> Result<()> {
    let key = normalize(&path);
    let my_gen = {
        let mut map = watchers().lock().map_err(|_| lock_err())?;
        if let Some((_, refs)) = map.get_mut(&key) {
            *refs += 1; // 已有线程在看这个路径，挂个引用就够了
            return Ok(());
        }
        let gen = NEXT_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        map.insert(key.clone(), (gen, 1));
        gen
    };

    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<std::result::Result<Event, notify::Error>>();
        let mut watcher: RecommendedWatcher = match RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default().with_poll_interval(Duration::from_millis(500)),
        ) {
            Ok(w) => w,
            Err(e) => {
                let _ = handle.emit("watch-error", e.to_string());
                return;
            }
        };

        let root = std::path::PathBuf::from(&path);
        // 只在两处用：监听目标展开（linked worktree 的 git 元数据在工作区根之外，
        // 必须一并挂监听，否则外部 commit 收不到事件）和 ignore 规则判断。
        // 打不开就退化成「只盯根目录、所有事件都相关」。
        let repo = git2::Repository::open(&root).ok();
        for target in crate::engine::watch::watch_targets(&root, repo.as_ref()) {
            if let Err(e) = watcher.watch(&target, RecursiveMode::Recursive) {
                let _ = handle.emit("watch-error", e.to_string());
                // 根目录挂不上就没有继续的意义；额外的 git 目录挂不上时
                // 保留其余监听，部分覆盖好过没有。
                if target == root {
                    return;
                }
            }
        }

        let mut pending = false;
        let mut last_change = Instant::now();
        loop {
            let still_mine = watchers()
                .lock()
                .map(|m| m.get(&key).map(|(g, _)| *g) == Some(my_gen))
                .unwrap_or(false);
            if !still_mine {
                break;
            }
            match rx.recv_timeout(TICK) {
                Ok(Ok(event)) => {
                    if event
                        .paths
                        .iter()
                        .any(|p| is_relevant(repo.as_ref(), &root, p))
                    {
                        pending = true;
                        last_change = Instant::now();
                    }
                }
                Ok(Err(e)) => {
                    let _ = handle.emit("watch-error", e.to_string());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if pending && last_change.elapsed() >= DEBOUNCE {
                pending = false;
                // 挂起期间**丢弃**事件而不是攒着：同步结束时调用方会主动发一次，
                // 攒着只会在恢复的瞬间连发几百条。
                if !is_watch_suspended(&path) {
                    // 带上路径，前端各标签据此只刷新自己那个仓库
                    let _ = handle.emit("repo-changed", &path);
                }
            }
        }
    });

    Ok(())
}

#[tauri::command(async)]
pub fn unwatch_repo(path: String) -> Result<()> {
    let key = normalize(&path);
    let mut map = watchers().lock().map_err(|_| lock_err())?;
    if let Some((_, refs)) = map.get_mut(&key) {
        *refs -= 1;
        if *refs == 0 {
            map.remove(&key); // 线程下一轮发现代号没了就退出
        }
    }
    Ok(())
}
