use crate::engine::path::normalize;
use crate::engine::svn_lock;
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

/// 挂起监听的 RAII 守卫（spec §5.2，2b 独立 review N3）。
///
/// 构造即挂起，Drop 即恢复。**不要**在同步流程里手写成对的
/// `suspend_watch` / `resume_watch`：中间任何一个 `?` 早退或 panic 都会让
/// watcher 永远停着——之后外部的每一次改动界面都看不见，而且没有任何报错。
///
/// 恢复之后「主动发一次 `repo-changed`」不在这里做：那要 `AppHandle`，而
/// engine 层与测试都拿不到它。由命令层在守卫 drop 之后补发，见
/// `commands/svn_sync.rs`。
pub struct WatchSuspension {
    path: String,
}

impl WatchSuspension {
    pub fn new(path: &str) -> Self {
        suspend_watch(path);
        Self {
            path: path.to_string(),
        }
    }
}

impl Drop for WatchSuspension {
    fn drop(&mut self) {
        resume_watch(&self.path);
    }
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
        // 同步锁是例外：另一个进程开始 / 结束同步时，界面要跟着挂上 / 撤掉横幅
        // （spec §5.1）。混在下面的 `*.lock` 噪声里一并吞掉的话，横幅要等工作区
        // 碰巧有别的变动才会更新。界面自己探测它不产生事件，见
        // `probing_the_sync_lock_does_not_trigger_a_refresh`，所以不会形成刷新回路。
        if target.file_name() == Some(std::ffi::OsStr::new(svn_lock::LOCK_FILE)) {
            return true;
        }
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

#[cfg(test)]
mod tests {
    use super::is_relevant;
    use crate::engine::svn_lock;
    use std::path::PathBuf;

    /// 带一个空 `.git/` 的临时工作副本。`is_relevant` 只看路径形状，不需要真仓库。
    fn wc() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        (dir, root)
    }

    #[test]
    fn the_sync_lock_file_is_relevant() {
        // 另一个进程拿锁（建文件）/ 正常释放（删文件）必须能让界面刷新横幅。
        // 被 `*.lock` 的噪声过滤一并吞掉的话，黄条要等到工作区碰巧有别的变动才更新。
        let (_d, root) = wc();
        let p = svn_lock::lock_path(root.to_str().unwrap());
        assert!(is_relevant(None, &root, &p), "{}", p.display());
    }

    #[test]
    fn git_own_lock_files_are_still_noise() {
        // 例外只开给同步锁一个文件名，git 自己的锁文件照旧过滤。
        let (_d, root) = wc();
        for rel in [
            &[".git", "index.lock"][..],
            &[".git", "HEAD.lock"],
            &[".git", "refs", "heads", "main.lock"],
        ] {
            let p = rel.iter().fold(root.clone(), |acc, c| acc.join(c));
            assert!(!is_relevant(None, &root, &p), "{}", p.display());
        }
    }

    /// 界面探测锁（`probe` 独占打开、`holder` 宽松读）**本身**不能产生相关事件。
    ///
    /// 否则放开过滤之后会形成回路：刷新 → 探测 → 事件 → 再刷新，每 300ms 一轮，
    /// 把整个仓库的面板永远刷下去。
    ///
    /// 末尾的正对照（往锁文件里真写一笔）证明 watcher 确实在工作、且锁文件的
    /// 变动确实会被当成相关——少了它，「没收到事件」可能只是 watcher 没挂上。
    #[cfg(windows)]
    #[test]
    fn probing_the_sync_lock_does_not_trigger_a_refresh() {
        use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
        use std::io::Write;
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::{Duration, Instant};

        let (_d, root) = wc();
        let root_s = root.to_str().unwrap().to_string();
        let lock = svn_lock::lock_path(&root_s);
        std::fs::write(&lock, br#"{"pid":1,"startedAt":1,"opid":"x"}"#).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .unwrap();
        w.watch(&root, RecursiveMode::Recursive).unwrap();

        // 收集一段时间内的相关事件路径
        let relevant_within = |d: Duration| -> Vec<PathBuf> {
            let end = Instant::now() + d;
            let mut hits = Vec::new();
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(Ok(ev)) => {
                        hits.extend(ev.paths.into_iter().filter(|p| is_relevant(None, &root, p)))
                    }
                    Ok(Err(e)) => panic!("watcher 报错：{e}"),
                    Err(_) => break,
                }
            }
            hits
        };
        // 丢掉建文件那一刻可能迟到的事件
        let _ = relevant_within(Duration::from_millis(500));

        // 无人持有时探测（独占打开成功那条路）
        for _ in 0..5 {
            let _ = svn_lock::probe(&root_s);
            let _ = svn_lock::holder(&root_s);
        }
        // 有人持有时探测（共享冲突那条路 + 宽松读）
        {
            const FILE_SHARE_READ: u32 = 0x1;
            let _held = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(FILE_SHARE_READ)
                .open(&lock)
                .unwrap();
            for _ in 0..5 {
                assert!(svn_lock::is_held(&root_s), "测试前提：锁此刻应被持有");
                let _ = svn_lock::holder(&root_s);
            }
        }
        let noise = relevant_within(Duration::from_millis(1000));
        assert!(noise.is_empty(), "探测本身产生了相关事件：{noise:?}");

        // 正对照
        std::fs::OpenOptions::new()
            .append(true)
            .open(&lock)
            .unwrap()
            .write_all(b" ")
            .unwrap();
        let hits = relevant_within(Duration::from_millis(3000));
        assert!(
            hits.iter().any(|p| p.file_name() == lock.file_name()),
            "正对照失败：真写锁文件也没收到相关事件（watcher 没工作，或锁文件仍被过滤）"
        );
    }
}
