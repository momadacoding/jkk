use crate::engine::path::normalize;
use crate::engine::svn_lock;
use crate::error::{GitError, Result};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
///
/// # 必须计数，不能是集合
///
/// 同一仓库可以开在两个标签页里。A 的更新正在跑时 B 点了「继续」，B 的命令也会
/// 挂起、然后因拿不到锁很快返回——集合的话 B 的恢复会把 A 的挂起一起撤掉
/// （2c 独立 review I2）。计数到 0 才算恢复。
static SUSPENDED: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn suspended() -> &'static Mutex<HashMap<String, usize>> {
    SUSPENDED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 挂起某个仓库的刷新事件。**同步流程开始时调**（spec §5.2）。
///
/// 锁中毒时静默放弃：挂起失败的后果只是同步期间多刷新几次界面，而在这里 panic
/// 会把整个同步流程带走。
pub fn suspend_watch(path: &str) {
    if let Ok(mut s) = suspended().lock() {
        *s.entry(normalize(path)).or_insert(0) += 1;
    }
}

/// 恢复刷新事件。调用方在恢复之后**必须主动发一次 `repo-changed`**（§5.2），
/// 否则挂起期间攒下的那一大批改动没有任何事件去告诉前端。
pub fn resume_watch(path: &str) {
    if let Ok(mut s) = suspended().lock() {
        let key = normalize(path);
        // 没挂起过的恢复是无害的空操作：失败路径可能在挂起之前就走到这里
        if let Some(n) = s.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                s.remove(&key);
            }
        }
    }
}

pub fn is_watch_suspended(path: &str) -> bool {
    suspended()
        .lock()
        .map(|s| s.contains_key(&normalize(path)))
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

/// 本应用自己写 index（Stage / Unstage）的登记：归一化路径 → 在写的次数、最后一次写完时 index 的指纹。
///
/// 2026-11-05 排查报告 6.13 方案 B：Stage 之后前端自己会就地刷新「本地改动」；监听再看到
/// `.git/index` 被改，300ms 后又发一次 `repo-changed`，前端随之跑一整轮（状态全量扫描、提交图、
/// stash、merge driver、svn info……），和前端自己那轮抢磁盘和 CPU。这一轮什么新信息都没有：
/// Stage / Unstage 只改 index（`stage_and_unstage_only_touch_the_index` 实测）。
///
/// # 为什么核对指纹而不是开一个时间窗口
///
/// 第一版是「写的过程中加之后 1 秒内的 index 事件都算自己的」。窗口内外部进程也写了 index
/// （终端里 `git add`、mergetool 标记已解决）会被一起吞掉。现在写完记下 index 的大小和修改时间，
/// 去抖到点要发事件时再看一次：还是那份就是自己写的；不是就说明之后又有人写过，照常刷新。
/// 迟到的事件也认得出来，不怕超过某个余量。剩下的缝只有「git 退出」到「记下指纹」之间那几微秒。
static OWN_INDEX_WRITES: OnceLock<Mutex<HashMap<String, OwnWrites>>> = OnceLock::new();

struct OwnWrites {
    running: usize,
    /// 最后一次写完时 index 的指纹。读不到（index 不存在、仓库打不开）时为 None：不豁免任何东西
    stamp: Option<IndexStamp>,
}

/// index 的指纹：大小 + 修改时间。git 写 index 是先写 `index.lock` 再改名，修改时间每次都变
/// （NTFS 精度 100ns）；只比修改时间也够，加上大小多一道保险。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct IndexStamp {
    len: u64,
    modified: std::time::SystemTime,
}

fn index_stamp(index: &Path) -> Option<IndexStamp> {
    let meta = std::fs::metadata(index).ok()?;
    Some(IndexStamp {
        len: meta.len(),
        modified: meta.modified().ok()?,
    })
}

fn own_index_writes() -> &'static Mutex<HashMap<String, OwnWrites>> {
    OWN_INDEX_WRITES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 本应用自己写 index 的 RAII 登记：构造即开始，Drop 即结束并记下 index 的指纹。
///
/// 用守卫而不是成对调用，理由同 `WatchSuspension`：中间任何一个 `?` 早退都会让登记永远停在
/// 「在写」，之后这个仓库的 index 事件一直等下去。锁中毒时静默放弃：后果只是多刷新一轮。
/// 命令失败也照样记：git 可能写了一部分，前端在失败时自己全量刷新（`RepoTab` 的 `writeIndex`）。
pub struct OwnIndexWrite {
    key: String,
    path: String,
}

impl OwnIndexWrite {
    pub fn begin(path: &str) -> Self {
        let key = normalize(path);
        if let Ok(mut m) = own_index_writes().lock() {
            m.entry(key.clone())
                .or_insert(OwnWrites {
                    running: 0,
                    stamp: None,
                })
                .running += 1;
        }
        Self {
            key,
            path: path.to_string(),
        }
    }
}

impl Drop for OwnIndexWrite {
    fn drop(&mut self) {
        // 先取指纹再减计数：反过来的话，watcher 可能在「不在写了、指纹还是上一次的」那一刻来核对
        let stamp = git2::Repository::open(&self.path)
            .ok()
            .and_then(|r| index_stamp(&r.path().join("index")));
        if let Ok(mut m) = own_index_writes().lock() {
            if let Some(w) = m.get_mut(&self.key) {
                w.running = w.running.saturating_sub(1);
                w.stamp = stamp;
            }
        }
    }
}

/// 只看到「自己写 index 会碰到的路径」时，到点核对的结论。
#[derive(Debug, PartialEq, Eq)]
enum OwnVerdict {
    /// index 还是我们最后写完的那份：不发事件
    Own,
    /// 没登记过、读不到指纹、或者指纹变了（之后有人又写过）：照常发
    NotOwn,
    /// 本应用的写还没结束：先不判，下一个 tick 再看
    Pending,
}

/// `key` 是 `normalize` 过的路径（watcher 线程里已经算好）；`index` 是这个仓库 gitdir 下的 index。
fn own_write_verdict(key: &str, index: &Path) -> OwnVerdict {
    let recorded = {
        let Ok(m) = own_index_writes().lock() else {
            return OwnVerdict::NotOwn;
        };
        match m.get(key) {
            None => return OwnVerdict::NotOwn,
            Some(w) if w.running > 0 => return OwnVerdict::Pending,
            Some(w) => w.stamp,
        }
    };
    // stat 放在锁外：Stage 命令的 Drop 也要拿这把锁
    match recorded {
        Some(stamp) if index_stamp(index) == Some(stamp) => OwnVerdict::Own,
        _ => OwnVerdict::NotOwn,
    }
}

/// Stage / Unstage 在 watcher 眼里会碰到的路径（已规范化）：gitdir 下的 `index`，以及两个只是
/// 「目录项变了」的目录事件——gitdir 本身（`index.lock` 建了又改名）和 commondir 下的 `objects`
/// （新对象落进新的两位前缀子目录）。后两个是 `stage_and_unstage_only_touch_the_index` 实测出来的；
/// 它们本身不带信息，外部真有改动时总有自己那条子路径的事件（index、refs……）。linked worktree 的
/// index 在 per-worktree gitdir 里，对象库在 commondir 里，所以两个目录分开取。第一项必须是 index。
fn own_write_paths(repo: &git2::Repository) -> Vec<PathBuf> {
    let gitdir = crate::engine::watch::canonical(repo.path());
    vec![
        gitdir.join("index"),
        gitdir,
        crate::engine::watch::canonical(repo.commondir()).join("objects"),
    ]
}

/// 事件路径规范化之后在不在 `own` 里。
///
/// 事件路径是按监听目标的原始写法给的，而 `own` 来自 libgit2，两边写法（8.3 短名、大小写、分隔符）
/// 不同就会失配，见 `engine::watch::watch_targets`。只规范化父目录：`index` 改名的瞬间文件本身
/// 可能还不在。
fn is_own_write_path(own: &[PathBuf], target: &Path) -> bool {
    let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else {
        return false;
    };
    // 先比文件名，绝大多数事件到这里就结束，不付 `canonicalize` 的代价。Windows 上不区分大小写
    let same_name = |p: &PathBuf| {
        p.file_name().is_some_and(|n| {
            if cfg!(windows) {
                n.eq_ignore_ascii_case(name)
            } else {
                n == name
            }
        })
    };
    if !own.iter().any(same_name) {
        return false;
    }
    let normalized = crate::engine::watch::canonical(parent).join(name);
    own.contains(&normalized)
}

/// 一条事件路径对刷新意味着什么。
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Wake {
    /// 不相关（被忽略的文件、对象库、锁文件……）
    No,
    /// 自己写 index 也会碰到的路径：到点核对指纹再决定（`own_write_verdict`）
    IfNotOwn,
    /// 一定要刷新
    Yes,
}

fn classify(
    repo: Option<&git2::Repository>,
    root: &Path,
    own_paths: &[PathBuf],
    target: &Path,
) -> Wake {
    if !is_relevant(repo, root, target) {
        Wake::No
    } else if is_own_write_path(own_paths, target) {
        Wake::IfNotOwn
    } else {
        Wake::Yes
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
        // 失焦快照（`svn_follow_mark`）是客户端自己写的线索，每次切窗口写一次，放过去
        // 就是每次切窗口多一轮全量刷新。里面的内容不影响任何面板
        if s.contains("/.git/mygitclient/") || s.ends_with("/.git/mygitclient") {
            return false;
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

        // 认「自己写的 index」用，见 `own_write_paths`。打不开仓库时为空：什么都不豁免
        let own_paths = repo.as_ref().map(own_write_paths).unwrap_or_default();
        // 第一项是 index（`own_write_paths`）；打不开仓库时没有，`own_only` 也就不会被置上
        let index_path = own_paths.first().cloned().unwrap_or_default();
        // 攒着的变化里有一定要刷新的
        let mut pending = false;
        // 攒着的变化只有「自己写 index 也会碰到的路径」：到点核对指纹再决定
        let mut own_only = false;
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
                    for p in &event.paths {
                        match classify(repo.as_ref(), &root, &own_paths, p) {
                            Wake::No => continue,
                            Wake::IfNotOwn => own_only = true,
                            Wake::Yes => pending = true,
                        }
                        last_change = Instant::now();
                    }
                }
                Ok(Err(e)) => {
                    let _ = handle.emit("watch-error", e.to_string());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if (pending || own_only) && last_change.elapsed() >= DEBOUNCE {
                // 只有 index 一类的变化时到这时才核对：去抖之后 Stage 命令多半已经结束、指纹已经记下。
                // 还没结束就留着，下一个 tick 再看
                if !pending {
                    match own_write_verdict(&key, &index_path) {
                        OwnVerdict::Pending => continue,
                        OwnVerdict::Own => {
                            own_only = false;
                            continue;
                        }
                        OwnVerdict::NotOwn => {}
                    }
                }
                pending = false;
                own_only = false;
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
    use super::{
        classify, is_relevant, normalize, own_write_verdict, OwnIndexWrite, OwnVerdict, Wake,
    };
    use crate::engine::svn_lock;
    use std::path::PathBuf;

    /// 带一个空 `.git/` 的临时工作副本。`is_relevant` 只看路径形状，不需要真仓库。
    fn wc() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        (dir, root)
    }

    /// 一个真的仓库（带一个空 index）：`own_write_paths` 要从 libgit2 拿 gitdir / commondir，
    /// 指纹要从真的 index 文件取。
    fn real_repo() -> (tempfile::TempDir, PathBuf, git2::Repository) {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let repo = git2::Repository::init(&root).unwrap();
        repo.index().unwrap().write().unwrap();
        (dir, root, repo)
    }

    #[test]
    fn what_our_own_index_write_touches_is_checked_not_refreshed_outright() {
        // 2026-11-05 排查报告 6.13 方案 B：Stage 之后前端自己会就地刷新；监听再看到 `.git/index`
        // 被改，300ms 后又触发一整轮刷新（状态全量扫描、提交图、stash、svn info……），和前端那轮
        // 抢磁盘和 CPU。这三条路径要先核对是不是自己写的
        let (_d, root, repo) = real_repo();
        let own = super::own_write_paths(&repo);
        let git = root.join(".git");
        for p in [git.join("index"), git.clone(), git.join("objects")] {
            assert_eq!(
                classify(Some(&repo), &root, &own, &p),
                Wake::IfNotOwn,
                "{}",
                p.display()
            );
        }
    }

    #[test]
    fn everything_else_still_wakes_a_refresh() {
        // 同一时刻工作区里的保存、别的 git 元数据变化都不能被一起吞掉
        let (_d, root, repo) = real_repo();
        let own = super::own_write_paths(&repo);
        let git = root.join(".git");
        for p in [
            root.join("a.txt"),
            // 工作区里恰好叫 index / objects 的文件
            root.join("index"),
            root.join("sub").join("index"),
            root.join("objects"),
            git.join("HEAD"),
            git.join("refs").join("heads").join("main"),
            git.join("MERGE_HEAD"),
        ] {
            assert_eq!(
                classify(Some(&repo), &root, &own, &p),
                Wake::Yes,
                "{}",
                p.display()
            );
        }
        // 打不开仓库（没有清单）时不核对任何东西
        assert_eq!(classify(None, &root, &[], &git.join("index")), Wake::Yes);
    }

    #[test]
    fn the_check_survives_a_different_spelling_of_the_repo_path() {
        // 事件路径按监听目标的原始写法给，清单来自 libgit2（正斜杠、可能是长名）。
        // 写法不同也要认得出来，否则永远走不到指纹核对
        let (_d, root, repo) = real_repo();
        let own = super::own_write_paths(&repo);
        let spelled = PathBuf::from(root.to_string_lossy().replace('\\', "/")).join(".git/index");
        assert_eq!(classify(Some(&repo), &root, &own, &spelled), Wake::IfNotOwn);
        #[cfg(windows)]
        {
            let upper = PathBuf::from(root.to_string_lossy().to_uppercase()).join(".git\\index");
            assert_eq!(classify(Some(&repo), &root, &own, &upper), Wake::IfNotOwn);
        }
    }

    #[test]
    fn the_verdict_follows_the_index_fingerprint() {
        let (_d, root, _repo) = real_repo();
        let raw = root.to_str().unwrap();
        let key = &normalize(raw);
        let index = root.join(".git").join("index");
        // 没登记过：不是自己写的
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::NotOwn);

        // Stage 命令收到的路径写法（分隔符、结尾斜杠、大小写）和 watcher 的不同，也是同一个仓库
        let first = OwnIndexWrite::begin(&format!("{}/", raw.replace('\\', "/")));
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::Pending);
        // 两次 Stage 重叠：先结束的那次不能让另一次还在写的被当成写完了
        let second = OwnIndexWrite::begin(raw);
        drop(first);
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::Pending);
        drop(second);
        // 写完了、index 还是那份：自己写的。过多久都一样，不靠时间窗口
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::Own);
        let other = &normalize(root.join("elsewhere").to_str().unwrap());
        assert_eq!(own_write_verdict(other, &index), OwnVerdict::NotOwn);

        // 之后有人又写了 index（大小变了）：不是自己写的，照常刷新
        let original = std::fs::read(&index).unwrap();
        std::fs::write(&index, [original.as_slice(), b"x"].concat()).unwrap();
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::NotOwn);
        // 大小一样、只有修改时间变了：同样认出来
        drop(OwnIndexWrite::begin(raw));
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::Own);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&index)
            .unwrap();
        f.set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        drop(f);
        assert_eq!(own_write_verdict(key, &index), OwnVerdict::NotOwn);
    }

    /// 实测：本应用的 Stage / Unstage（`GitBackend` 原样的参数、`OwnIndexWrite` 原样的守卫）在 watcher
    /// 眼里只碰了那三条路径，而且到点核对时认得出是自己写的；之后外部终端里的 `git add` 认得出不是。
    ///
    /// 方案 B 的前提就是这一条。若 git 某天在 `add` / `reset` 时还写了别的相关文件，这里先红，
    /// 而不是界面上悄悄多出一轮刷新。
    #[cfg(windows)]
    #[test]
    fn stage_and_unstage_only_touch_the_index() {
        use crate::vcs::Vcs;
        use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
        use std::process::Command;
        use std::time::{Duration, Instant};

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let git = |args: &[&str]| {
            assert!(Command::new("git")
                .current_dir(&root)
                .args(args)
                .status()
                .unwrap()
                .success());
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "t@t.com"]);
        git(&["config", "user.name", "T"]);
        std::fs::write(root.join("a.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(root.join("a.txt"), "1\n2\n").unwrap();
        std::fs::write(root.join("new.txt"), "n\n").unwrap();

        let repo = git2::Repository::open(&root).unwrap();
        let own = super::own_write_paths(&repo);
        let key = normalize(root.to_str().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        let mut w = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                let _ = tx.send(res);
            },
            Config::default(),
        )
        .unwrap();
        w.watch(&root, RecursiveMode::Recursive).unwrap();
        // 一段时间内每条事件路径的分类
        let collect = |d: Duration| -> Vec<(PathBuf, Wake)> {
            let end = Instant::now() + d;
            let mut seen = Vec::new();
            while let Some(left) = end.checked_duration_since(Instant::now()) {
                match rx.recv_timeout(left) {
                    Ok(Ok(ev)) => {
                        for p in ev.paths {
                            let wake = classify(Some(&repo), &root, &own, &p);
                            if wake != Wake::No {
                                seen.push((p, wake));
                            }
                        }
                    }
                    Ok(Err(e)) => panic!("watcher 报错：{e}"),
                    Err(_) => break,
                }
            }
            seen
        };
        let _ = collect(Duration::from_millis(500));

        let backend = crate::vcs::git::GitBackend::new(root.to_str().unwrap());
        let paths = vec!["a.txt".to_string(), "new.txt".to_string()];
        for (what, run) in [
            (
                "stage",
                &(|| backend.stage(&paths)) as &dyn Fn() -> crate::error::Result<()>,
            ),
            ("unstage", &|| backend.unstage(&paths)),
        ] {
            // 与 `commands::ops::stage` 相同：命令跑完守卫就 drop，记下指纹
            {
                let _own = OwnIndexWrite::begin(root.to_str().unwrap());
                run().unwrap();
            }
            let seen = collect(Duration::from_millis(1000));
            // 先证明确实看到了 index 的变化，下面的结论才有意义
            assert!(
                seen.iter().any(|(p, _)| p.ends_with(".git/index")),
                "{what}：没收到 index 的事件，watcher 没工作？{seen:?}"
            );
            let outside: Vec<_> = seen.iter().filter(|(_, w)| *w == Wake::Yes).collect();
            assert!(
                outside.is_empty(),
                "{what} 碰到了清单以外的相关路径：{outside:?}"
            );
            assert_eq!(
                own_write_verdict(&key, &own[0]),
                OwnVerdict::Own,
                "{what}：到点核对没认出是自己写的"
            );
        }

        // 外部终端里的 `git add`：也只碰这三条路径，但核对时认得出不是自己写的，照常刷新
        git(&["add", "--", "a.txt"]);
        let seen = collect(Duration::from_millis(1000));
        assert!(
            seen.iter().any(|(p, _)| p.ends_with(".git/index")),
            "{seen:?}"
        );
        assert_eq!(own_write_verdict(&key, &own[0]), OwnVerdict::NotOwn);

        // 正对照：工作区的保存一定唤醒
        std::fs::write(root.join("a.txt"), "1\n2\n3\n").unwrap();
        let seen = collect(Duration::from_millis(1000));
        assert!(
            seen.iter()
                .any(|(p, w)| p.ends_with("a.txt") && *w == Wake::Yes),
            "正对照失败：工作区的保存没有唤醒刷新：{seen:?}"
        );
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

    #[test]
    fn the_follow_mark_written_on_blur_is_noise() {
        // 失焦时写快照。放它过去，每次切窗口都会多一轮全量刷新。目录本身的事件也要挡：
        // 第一次写快照时 `.git/mygitclient` 是新建的
        let (_d, root) = wc();
        let repo = root.to_str().unwrap();
        let mark = crate::engine::svn_follow_mark::mark_path(repo);
        for p in [
            mark.clone(),
            mark.with_extension("json.123.0.tmp"),
            mark.parent().unwrap().to_path_buf(),
        ] {
            assert!(!is_relevant(None, &root, &p), "{}", p.display());
        }
        // 只挡这个子目录：同名前缀的 journal 仍然相关
        let journal = root.join(".git").join("mygitclient-sync.json");
        assert!(is_relevant(None, &root, &journal));
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
