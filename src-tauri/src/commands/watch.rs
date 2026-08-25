use crate::engine::path::normalize;
use crate::error::{GitError, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
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
                // 带上路径，前端各标签据此只刷新自己那个仓库
                let _ = handle.emit("repo-changed", &path);
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
