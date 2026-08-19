use crate::engine::git_cli::run_git;
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use crate::engine::worktree::{read_worktrees, Worktree};

/// 回收站里最多留几个条目。超出的按时间从旧到新清掉。
///
/// 这里存的是完整的工作树，装了 node_modules 的能到几个 GB，不能无限攒。
const TRASH_KEEP: usize = 5;

/// 槽位的状态标记文件。同一时刻一个槽位里**有且只有一个**标记存在，状态转换
/// 就是在槽位内把这个小文件改名——原子、跨进程互斥。
///
/// # 为什么状态不编码在目录名上
///
/// 原先的做法是把状态写成目录名后缀（`.partial` / `.pruning`），转换靠重命名
/// 整个槽位目录。这在 Windows 上是概率性失败：**重命名目录要求整棵子树没有任何
/// 打开的句柄**（连带 `FILE_SHARE_DELETE` 的规矩开法也挡），而槽位里装的正是刚
/// 搬进来的整棵工作树。任何扫描器、编辑器、watcher 在那几毫秒里碰一下里面的
/// 文件，转换就报 `Access is denied. (os error 5)`。POSIX 的 rename 不看子树句柄，
/// 所以这个缺陷只在 Windows 上暴露，Linux/macOS 上永远测不出来。
///
/// 改成标记文件之后，转换只重命名一个几十字节的小文件，与槽位里装了什么无关。
/// 目录名一经创建永不改变。
///
/// 用 `meta.json` 的扩展名变体而不是独立文件名：这几个名字都在槽位内部，
/// 撞不上用户数据（槽位里只有 `worktree/`、`admin/` 两个子目录）。
const MARK_COMMITTED: &str = "meta.json";
/// 删除搬移进行中：槽位已建、两次搬移尚未全部完成。
const MARK_DELETING: &str = "meta.json.partial";
/// 清理认领中：已被某个清理线程判了死刑，只有认领者自己删。
const MARK_PRUNING: &str = "meta.json.pruning";

/// 槽位状态。判据是槽位里存在哪个标记文件。
#[derive(Debug, PartialEq, Eq)]
enum SlotState {
    /// 正式条目：删除已完成，参与清理选举。
    Committed,
    /// 删除搬移进行中。
    Deleting,
    /// 清理认领中。
    Pruning,
    /// 一个标记都没有。既可能是 `create_dir` 刚返回、标记还没落盘的微秒级窗口，
    /// 也可能是异常残留。一律按残留里最保守的一档处理，绝不当正式条目清掉。
    Unknown,
}

/// 0.2.1 及更早版本把状态编码在目录名后缀上时留下的残留。只用来识别，不再产生。
const LEGACY_PARTIAL_SUFFIX: &str = ".partial";
const LEGACY_PRUNING_SUFFIX: &str = ".pruning";

/// 一个槽位现在是什么状态。
///
/// 探测出错（权限、盘在抖）会让判据滑向 [`SlotState::Unknown`]，那一档比
/// `Committed` 更保守——错的方向必须是不删。
///
/// # 旧版本残留：后缀的优先级高于标记文件
///
/// 升级前留下的槽位里只有一个 `meta.json`，状态全靠目录名后缀区分。**带后缀的
/// 半截槽位里同样有 `meta.json`**，只看标记文件会把它们判成正式条目——那意味着
/// 一个「删除崩在搬移中途」的槽位会参与清理选举，连同里面未提交内容的唯一副本
/// 一起被 `remove_dir_all` 掉。所以必须先看后缀。
///
/// 旧的**正式**条目（无后缀 + `meta.json`）本来就该判成 `Committed`，天然兼容：
/// 照常参与清理选举，被认领时 `meta.json` 改名成新的标记，自动完成迁移。
fn slot_state(slot: &Path) -> SlotState {
    if let Some(name) = slot.file_name().and_then(|s| s.to_str()) {
        if name.ends_with(LEGACY_PARTIAL_SUFFIX) {
            return SlotState::Deleting;
        }
        if name.ends_with(LEGACY_PRUNING_SUFFIX) {
            return SlotState::Pruning;
        }
    }
    for (mark, state) in [
        (MARK_COMMITTED, SlotState::Committed),
        (MARK_PRUNING, SlotState::Pruning),
        (MARK_DELETING, SlotState::Deleting),
    ] {
        if slot.join(mark).is_file() {
            return state;
        }
    }
    SlotState::Unknown
}

/// 首发失败之后最多再重试这么多次，每次之前等 [`RENAME_BACKOFF`]。
/// 合计 1 + [`RENAME_RETRIES`] 次尝试、[`RENAME_RETRIES`] 段退避（共 1 秒）。
///
/// 兜的是**不可避免要搬移带内容目录**的那几处（工作树和管理目录本身的搬进搬出）：
/// 标记文件改名已经不受子树句柄影响了，但数据搬移本身绕不开，只能等对方松手。
/// 实测挡路的句柄是毫秒级瞬时的，1 秒的总窗口足够，又不至于让真正被占用的
/// 情况卡住界面太久。
const RENAME_RETRIES: usize = 10;
const RENAME_BACKOFF: std::time::Duration = std::time::Duration::from_millis(100);

/// 带退避重试的 `rename`。只对 `PermissionDenied` 重试——那正是 Windows 上
/// 「子树里有人开着文件」的表现（`ERROR_ACCESS_DENIED`）。
///
/// `NotFound` 之类的错误重试没有意义，而且认领互斥依赖它**立刻**返回：
/// 慢一秒会把「别人先拿到了」拖成假死。
fn rename_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    for _ in 0..RENAME_RETRIES {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                std::thread::sleep(RENAME_BACKOFF);
            }
            Err(e) => return Err(e),
        }
    }
    std::fs::rename(from, to)
}

/// 进行中残留的自动回收时限。只适用于**能证明没有用户数据**的残留：
/// 空的或只写了 meta 的进行中槽位（[`SlotState::Deleting`] / [`SlotState::Unknown`]），
/// 以及已被判定为垃圾的 [`SlotState::Pruning`]。
/// 含 `worktree/` 或 `admin/` 的进行中槽位是未提交内容的唯一副本
/// （删除崩在搬移中途、或回滚失败），永久保留等用户处理，绝不自动删。
///
/// 取 24 小时：正常进行中窗口是毫秒级，这个余量同时兜住「两个应用实例
/// 开同一个仓库」的场景；年龄读不出时一律不清，宁可漏清不可误删。
const RESIDUE_TTL_SECS: u64 = 24 * 3600;

/// 主仓库的根目录。
///
/// `path` 可能是主仓库，也可能是任意一个 worktree。`--git-common-dir` 在两种情况下
/// 都指向主仓库的 .git，它的父目录就是主仓库根。新 worktree 统一建在那下面的
/// `.worktrees/`，否则从一个 worktree 里新建会建到仓库外面去。
fn main_repo(path: &str) -> Result<PathBuf> {
    let git_dir = common_git_dir(path)?;
    Ok(git_dir.parent().unwrap_or(&git_dir).to_path_buf())
}

/// 整个仓库共用的那个 `.git`，从主工作区还是从某个 worktree 问都指向同一处。
fn common_git_dir(path: &str) -> Result<PathBuf> {
    let out = run_git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    Ok(PathBuf::from(out.trim()))
}

#[tauri::command(async)]
pub fn read_worktrees_cmd(path: String) -> Result<Vec<Worktree>> {
    read_worktrees(&path)
}

#[tauri::command(async)]
pub fn create_worktree(path: String, name: String, branch: Option<String>) -> Result<()> {
    let worktree_path = main_repo(&path)?.join(".worktrees").join(&name);
    let target = worktree_path.to_string_lossy().to_string();

    let mut args: Vec<&str> = vec!["worktree", "add", &target];
    // 不带 -b 的话 git 会把它当成 commit-ish 检出成游离 HEAD
    if let Some(b) = &branch {
        args.push("-b");
        args.push(b);
    }
    run_git(&path, &args)?;
    Ok(())
}

/// 一次删除的事实记录：删掉了谁、东西落在回收站的哪个槽位里。
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeletedWorktree {
    pub name: String,
    /// 回收站槽位的绝对路径。没有撤销入口之后它不再是「凭据」，而是让用户
    /// （和集成测试）能定位到东西搬去哪了的唯一线索。
    pub trash_path: String,
}

/// 回收站槽位里的 meta：这个槽位是从哪两个目录搬来的。
///
/// 没有撤销功能之后它不再被程序消费，但**必须继续写**：手工把 worktree 捞回去
/// 需要知道原路径，尤其是 `admin`（`.git/worktrees/<name>`）——光看槽位内容
/// 猜不出来。`deleted_at` 仍然是清理选举的排序依据。
///
/// 路径存绝对路径而不是相对：手工恢复的人不需要再推导一遍。
#[derive(Serialize, Deserialize, Debug)]
struct TrashMeta {
    /// 原工作树目录
    worktree: String,
    /// 原管理目录（`.git/worktrees/<name>`）
    admin: String,
    deleted_at: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 这个 worktree 的管理目录（`.git/worktrees/<name>`）。
///
/// **不能拿工作树的目录名去拼**。同名冲突时 git 会给管理目录加后缀，名字对不上；
/// 问 git 要是唯一可靠的办法。
fn admin_dir(worktree_path: &str) -> Result<PathBuf> {
    let out = run_git(worktree_path, &["rev-parse", "--absolute-git-dir"])?;
    Ok(PathBuf::from(out.trim()))
}

/// 登记反查专用的拼写比较：**全平台精确匹配，不折叠大小写**。
///
/// 不能复用 `same_path`：它无条件 `to_lowercase`，那是给 Windows 的锁/主工作区
/// 否决准备的，误判方向是「当成同一个」→ 拒绝，安全；而登记反查的误判方向是
/// 「搬错管理目录」→ 弄坏另一个还活着的 worktree。Windows 可以按目录开大小写
/// 敏感（`SetCaseSensitiveInformation`），那种目录里 `Foo` 和 `foo` 是两条不同
/// 的登记；目标目录已丢时 canonicalize 兜底也用不上，折叠后按 read_dir 顺序
/// 撞上谁就搬谁。唯一零误命中的做法就是精确匹配：Windows 只归一分隔符
/// （反斜杠在 Windows 上不可能是文件名字符），非 Windows 连分隔符都不动
/// （反斜杠在 Linux 上是合法文件名字符），只允许尾部斜杠差异。
///
/// 代价：大小写拼写漂移（8.3 短名、junction 别名、macOS 大小写不敏感卷上
/// 拼写变体）匹配不上，按「找不到登记」拒删——over-refusal 是安全方向；
/// gitdir 文件和前端路径同出 git 一族写法，正常情形拼写一致。
fn spelling_equal(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.replace('\\', "/").trim_end_matches('/') == b.replace('\\', "/").trim_end_matches('/')
    }
    #[cfg(not(windows))]
    {
        a.trim_end_matches('/') == b.trim_end_matches('/')
    }
}

/// 目录已经从盘上消失时，从登记里反查管理目录。
///
/// 每个 linked worktree 的登记目录（`.git/worktrees/<name>/`）里都有一个 `gitdir`
/// 文件，写着那个工作树的 `.git` 文件的位置，它的父目录就是工作树本身。扫一遍按
/// [`spelling_equal`] 精确匹配——不靠目录名拼，同名被加过后缀的照样对得上。
///
/// 多重精确命中说明登记损坏（两条登记指向同一路径），拒绝搬动：分不清哪条是
/// 真的时，唯一安全的动作是什么都不动。
fn admin_from_registry(common: &Path, worktree_path: &str) -> Result<Option<PathBuf>> {
    let entries = match std::fs::read_dir(common.join("worktrees")) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut hits: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let admin = entry.path();
        let Ok(gitdir) = std::fs::read_to_string(admin.join("gitdir")) else {
            continue;
        };
        let Some(dir) = Path::new(gitdir.trim()).parent() else {
            continue;
        };
        if spelling_equal(&dir.to_string_lossy(), worktree_path) {
            hits.push(admin);
        }
    }
    match hits.len() {
        0 => Ok(None),
        1 => Ok(hits.into_iter().next()),
        _ => Err(GitError::new(
            "worktree_ambiguous",
            format!(
                "有多条登记指向同一个工作树路径：\n{worktree_path}\n\n\
                 登记已损坏，分不清哪条是真的，拒绝搬动。\
                 先 git worktree repair 或手工清理 .git/worktrees 再来。"
            ),
        )),
    }
}

/// 这个 worktree 被 `git worktree lock` 锁了吗？锁了就返回理由。
///
/// 单独解析而不是扩展 `read_worktrees`：那个函数是喂给界面列表的，加字段要连着
/// 前端类型一起动；这里只关心「这一个锁没锁」，就地问一次更省事。
///
/// 必须**在搬移之前**问。锁的判据在登记里而不在目录里，先搬走再解除登记的话，
/// git 照样会因为锁拒绝，那时候目录已经进回收站了，状态就不一致了。
fn lock_reason(path: &str, worktree_path: &str) -> Result<Option<String>> {
    let out = run_git(path, &["worktree", "list", "--porcelain", "-z"])?;
    for block in out.split("\0\0") {
        let mut hit = false;
        let mut reason: Option<String> = None;
        for line in block.split('\0') {
            let line = line.trim_end_matches('\n');
            if let Some(p) = line.strip_prefix("worktree ") {
                hit = same_path(p, worktree_path);
            } else if line == "locked" {
                reason = Some(String::new());
            } else if let Some(r) = line.strip_prefix("locked ") {
                reason = Some(r.to_string());
            }
        }
        if hit {
            return Ok(reason);
        }
    }
    Ok(None)
}

/// 工作区里有没有未提交的东西（含未跟踪文件）。
fn is_dirty(worktree_path: &str) -> Result<bool> {
    let out = run_git(worktree_path, &["status", "--porcelain"])?;
    Ok(!out.trim().is_empty())
}

/// 在回收站里开一个不重名的槽位。目录名一经创建就固定，状态由里面的标记文件表示。
///
/// 名字带秒级时间戳，同一秒删两个同名 worktree 会撞上，所以再加序号兜底。
/// 用 `create_dir` 的 `AlreadyExists` 来避让而不是先 `exists()` 再建：后者中间有
/// TOCTOU 窗口，两个并发删除会拿到同一个槽位路径。
fn new_trash_slot(trash_root: &Path, name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(trash_root)?;
    let stamp = now_secs();
    for n in 0..1000 {
        let slot = if n == 0 {
            trash_root.join(format!("{name}-{stamp}"))
        } else {
            trash_root.join(format!("{name}-{stamp}-{n}"))
        };
        match std::fs::create_dir(&slot) {
            Ok(()) => return Ok(slot),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(GitError::new(
        "trash_slot",
        "回收站里同名条目太多，建不出新槽位".to_string(),
    ))
}

/// 落名：把 [`MARK_DELETING`] 改成 [`MARK_COMMITTED`]，槽位从这一刻起是正式条目。
///
/// 只重命名一个小文件，与槽位里已经装了多大的工作树无关——这正是这个设计存在的
/// 理由，见 [`MARK_COMMITTED`] 的说明。同目录内 rename 原子：清理线程要么看到
/// 进行中标记（跳过），要么看到正式标记（meta 已完整），不存在中间态。
fn commit_slot(slot: &Path) -> Result<()> {
    rename_retry(&slot.join(MARK_DELETING), &slot.join(MARK_COMMITTED))?;
    Ok(())
}

/// 目录在不在？探测**出错**（盘没挂上、权限不够、网络盘断连）不是「不在」：
/// `Path::exists` 会把这些错误折叠成 false，顺着「目录已丢」的分支走下去会
/// 跳过脏检查、只搬登记，让仍真实存在的工作树变成无主目录。只有明确的
/// NotFound 才算丢了，其他错误一律拒绝操作。
fn dir_exists(path: &Path) -> Result<bool> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(GitError::new(
            "dir_probe",
            format!(
                "读不出这个目录的状态（盘没挂上或权限不够？）：{e}\n\n{}",
                path.display()
            ),
        )),
    }
}

/// 目录的年龄。`created` 在部分平台/文件系统上问不出来，退回 `modified`；
/// 都问不出来（盘在抖、权限不够）返回 `None`，调用方按「宁可漏清不可误删」处理。
fn dir_age(path: &Path) -> Option<std::time::Duration> {
    let md = std::fs::metadata(path).ok()?;
    let t = md.created().or_else(|_| md.modified()).ok()?;
    t.elapsed().ok()
}

/// 进行中残留什么时候可以清：超过宽限期、且**能证明里面没有用户数据**
/// （既没有 `worktree/` 也没有 `admin/`）。含内容的进行中槽位是未提交内容的
/// 唯一副本——删除崩在搬移中途、或回滚失败——永久保留等用户处理，绝不自动删。
fn should_reap_partial(age: Option<std::time::Duration>, has_content: bool) -> bool {
    !has_content && matches!(age, Some(a) if a >= std::time::Duration::from_secs(RESIDUE_TTL_SECS))
}

/// 已被认领为清理对象（[`SlotState::Pruning`]）的残留：内容本来就是按策略判了
/// 死刑的旧条目，超过宽限期（认领它的线程崩了/被抢占）就接着清。
fn should_reap_pruning(age: Option<std::time::Duration>) -> bool {
    matches!(age, Some(a) if a >= std::time::Duration::from_secs(RESIDUE_TTL_SECS))
}

/// 目录里有没有这个子项？探测出错一律按「有」处理——这个判断决定要不要删
/// 目录，错的方向必须是不删。
fn has_entry(dir: &Path, name: &str) -> bool {
    !matches!(
        std::fs::metadata(dir.join(name)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound
    )
}

/// 删除一个 worktree：搬进仓库内的回收站，而不是抹掉。
///
/// 收的是 worktree 的**完整路径**而不是名字：名字得靠 `parent(当前仓库)/名字` 拼，
/// 只有当前打开的恰好是它的兄弟 worktree 时才碰巧对；从主仓库删算出来的是仓库外面
/// 的路径。`Worktree` 结构体本来就带 path，直接用。
///
/// # 为什么不让 git 删
///
/// 原先是 `git worktree remove` 删文件、我们兜底删剩下的。那条路上有两个已经验证过
/// 的数据丢失通道：
///
/// 1. `git worktree remove` **会跟进 junction 把目标里的东西删掉**（见
///    `tests/worktree_ops.rs` 里 `our_cleanup_unlinks_junctions_instead_of_following_them`
///    的注释，单独验过）。指向仓库外的链接会连累外面的数据。
/// 2. pnpm 的 junction 布局会让 git 递归删除栽在死链接上、删一半放弃，留下半个目录。
///
/// 改成由我们搬移之后，git 不再碰任何文件，这两条通道一起消失。
///
/// # 顺序
///
/// 工作树和**管理目录**（`.git/worktrees/<name>`）必须成对搬走。index 存在管理目录
/// 里而不在工作树里，只搬工作树的话暂存状态会丢。两个都搬走之后 git 就彻底不认这个
/// worktree 了，`git worktree list` 干净、同名分支能立刻重建，不需要再跑 `remove`。
///
/// 搬移绕过了 git 自带的脏检查和锁检查，所以这两道必须我们**先**做完再动手。
#[tauri::command(async)]
pub fn delete_worktree(
    path: String,
    worktree_path: String,
    force: bool,
) -> Result<DeletedWorktree> {
    // 目录在不在必须用严格探测：`Path::exists` 把 I/O 错误（网络盘断连、未挂载、
    // 无权限）折叠成 false，那种情况下走「目录已丢」分支会跳过脏检查、只搬登记，
    // 仍真实存在的工作树和里面的未提交内容就成了无主数据。
    let worktree_exists = dir_exists(Path::new(&worktree_path))?;

    // 主工作区一票否决，而且要排在所有检查最前面。
    //
    // 这道保护原先是 git 给的（`fatal: '.' is a main working tree`）。现在 git 不再
    // 参与删除，就必须自己拦：主工作区的管理目录**就是仓库的 `.git` 本身**，照着
    // 后面的流程走会把整个 `.git` 搬进回收站。界面上虽然禁着按钮，但那拦不住
    // 直接调命令的路径。
    //
    // 判据：主工作区的 `--absolute-git-dir` 和 `--git-common-dir` 相等，linked
    // worktree 的前者是 `.git/worktrees/<name>`，不相等。
    let admin = if worktree_exists {
        let admin = admin_dir(&worktree_path)?;
        if same_path(
            &admin.to_string_lossy(),
            &common_git_dir(&worktree_path)?.to_string_lossy(),
        ) {
            return Err(GitError::new(
                "worktree_is_main",
                "这是仓库的主工作区，不能删。".to_string(),
            ));
        }
        admin
    } else {
        // 目录已丢（prunable）：git 进程没法在不存在的目录里启动（Windows 上 io
        // error 267），`admin_dir` 走不通，改从登记里反查。主工作区不在
        // `.git/worktrees/` 下登记，扫不到只会换来「找不到登记」的拒绝——
        // 一票否决仍然成立，只是换个说法。
        let common = common_git_dir(&path)?;
        admin_from_registry(&common, &worktree_path)?.ok_or_else(|| {
            GitError::new(
                "worktree_not_registered",
                format!(
                    "git 的登记里找不到这个 worktree：\n{worktree_path}\n\n\
                     目录已经不在盘上，登记也对不上，没什么可删的。"
                ),
            )
        })?
    };

    if let Some(reason) = lock_reason(&path, &worktree_path)? {
        let tail = if reason.is_empty() {
            String::new()
        } else {
            format!("：{reason}")
        };
        return Err(GitError::new(
            "worktree_locked",
            format!("这个 worktree 被锁了{tail}\n\n先 git worktree unlock 再删。"),
        ));
    }

    // 脏就拦下来。跑不动 status 说明 worktree 本身已经坏了，那种情况下拦着反而
    // 让人没法收拾，交给 force 放行。
    //
    // 目录已丢时不查：未提交内容随目录一起没了，没有可保护的数据，git 进程也起不来。
    if !force && worktree_exists {
        match is_dirty(&worktree_path) {
            Ok(true) => {
                return Err(GitError::new(
                    "worktree_dirty",
                    "这个 worktree 里有未提交的改动或未跟踪的文件。\n\n\
                     确认要删的话可以继续，目录会被移到仓库内的回收站，不是直接抹掉。"
                        .to_string(),
                ))
            }
            Ok(false) => {}
            Err(err) => {
                return Err(GitError::new(
                    "worktree_broken",
                    format!(
                        "读不出这个 worktree 的状态，它可能已经坏了：{}\n\n\
                         确认要删的话可以继续。",
                        err.message
                    ),
                ))
            }
        }
    }

    let name = Path::new(&worktree_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let trash_root = main_repo(&path)?.join(".worktrees").join(".trash");
    let slot = new_trash_slot(&trash_root, &name)?;

    // meta 在任何搬移之前落盘。进行中槽位靠 MARK_DELETING 标记对清理不可见，先写 meta
    // 是第二层防护：换成正式标记的那一刻，槽位内容（meta/worktree/admin）必须完整。
    // 写不成时槽位还是空的，直接当作没删过。
    let meta = TrashMeta {
        worktree: worktree_path.clone(),
        admin: admin.to_string_lossy().to_string(),
        deleted_at: now_secs(),
    };
    if let Err(err) = write_meta(&slot, &meta) {
        remove_slot(&slot);
        return Err(err);
    }

    // 先搬工作树。这一步失败什么都没动，直接把原因带回去。目录已丢时无事可搬。
    let slot_worktree = slot.join("worktree");
    if worktree_exists {
        if let Err(err) = rename_retry(Path::new(&worktree_path), &slot_worktree) {
            remove_slot(&slot);
            // 系统只会说 "used by another process"，不说是哪个。挨个探测一遍，
            // 把话说到能动手为止——探测是只读的，此时目录一动未动。
            let root = Path::new(&worktree_path);
            let blockers = crate::engine::file_lock::find_blockers(root, 3);
            return Err(GitError::new(
                "worktree_busy",
                format!(
                    "移不动这个 worktree：{err}\n\n{}",
                    crate::engine::file_lock::describe(root, &blockers)
                ),
            ));
        }
    }

    // 再搬管理目录。失败就把工作树搬回原位——半搬状态下 git 看到的是个坏 worktree。
    let slot_admin = slot.join("admin");
    if let Err(err) = rename_retry(&admin, &slot_admin) {
        let rolled_back =
            !worktree_exists || rename_retry(&slot_worktree, Path::new(&worktree_path)).is_ok();
        if rolled_back {
            remove_slot(&slot);
        }
        // 工作树搬不回去时槽位连 meta 一起留着：meta 里有原路径，是手工捞回去的线索。
        return Err(GitError::new(
            "worktree_admin_move_failed",
            if rolled_back {
                format!("搬不动这个 worktree 的管理目录：{err}\n\n已经原样放回，什么都没改。")
            } else {
                format!(
                    "搬不动这个 worktree 的管理目录：{err}\n\n\
                     而且工作树也没能放回原位，现在在：\n{}\n\n\
                     手工搬回 {worktree_path} 即可恢复。",
                    slot_worktree.display()
                )
            },
        ));
    }

    // 落名提交：把进行中标记换成正式标记，清理线程从这一刻起才把它当回收站条目。
    // 失败就把两道工序都搬回去，当作没删过。
    if let Err(err) = commit_slot(&slot) {
        let worktree_back =
            !worktree_exists || rename_retry(&slot_worktree, Path::new(&worktree_path)).is_ok();
        let admin_back = rename_retry(&slot_admin, &admin).is_ok();
        if worktree_back && admin_back {
            remove_slot(&slot);
        }
        return Err(GitError::new(
            "trash_commit",
            if worktree_back && admin_back {
                format!(
                    "回收站条目落不了名：{}\n\n已经原样放回，什么都没改。",
                    err.message
                )
            } else {
                format!(
                    "回收站条目落不了名：{}\n\n\
                     也没能完整放回原位，剩下的东西在：\n{}\n\n\
                     这个槽位没落名，不会被自动清理，手工搬回即可。",
                    err.message,
                    slot.display()
                )
            },
        ));
    }

    spawn_prune(trash_root);

    Ok(DeletedWorktree {
        name,
        trash_path: slot.to_string_lossy().to_string(),
    })
}

/// 落 meta，槽位进入「删除进行中」状态（[`MARK_DELETING`]）。
///
/// 写的是进行中标记而不是正式标记：正式标记的出现就是提交点，必须留给
/// [`commit_slot`]，在两次搬移都完成之后才允许出现。
fn write_meta(slot: &Path, meta: &TrashMeta) -> Result<()> {
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| GitError::new("trash_meta", format!("写不出回收站元数据：{e}")))?;
    std::fs::write(slot.join(MARK_DELETING), json)?;
    Ok(())
}

/// 清掉一个只写了 meta、还没搬进去东西的槽位（删除中途失败的收尾）。
///
/// 四个标记都试着删一遍：收尾可能发生在任何一个状态上，而 `remove_dir` 要求
/// 目录是空的。删不掉不追究——残留会被清理器按「无内容且超期」回收。
fn remove_slot(slot: &Path) {
    for mark in [MARK_DELETING, MARK_COMMITTED, MARK_PRUNING] {
        let _ = std::fs::remove_file(slot.join(mark));
    }
    let _ = std::fs::remove_dir(slot);
}

/// 从指定标记文件里读元数据。清理认领之后标记已经改名，调用方必须传对应的那个。
fn read_meta(slot: &Path, mark: &str) -> Result<TrashMeta> {
    let raw = std::fs::read_to_string(slot.join(mark))
        .map_err(|e| GitError::new("trash_meta", format!("这个回收站条目读不出元数据：{e}")))?;
    serde_json::from_str(&raw)
        .map_err(|e| GitError::new("trash_meta", format!("回收站元数据坏了：{e}")))
}

fn sort_slots(slots: &mut [(u64, PathBuf)]) {
    slots.sort_by(|(left_ts, left_path), (right_ts, right_path)| {
        left_ts
            .cmp(right_ts)
            .then_with(|| left_path.cmp(right_path))
    });
}

/// 把最旧的条目清掉，只留 [`TRASH_KEEP`] 个。
///
/// 清不掉不算错误——回收站塞不下顶多占地方，不该让删除操作本身失败。
///
/// 回收站里的槽位按 [`slot_state`] 分治：
/// - [`SlotState::Committed`]：正式条目，参与 victim 选举，超限后最旧的被清。
/// - [`SlotState::Deleting`]：正在搬移的槽位，**一律跳过、不计入条数**。只有超过
///   宽限期、且能证明为空（无 worktree/ admin 子项）的残留才清——含内容的是
///   未提交内容的唯一副本，永久保留。
/// - [`SlotState::Pruning`]：已被某个清理线程认领的 victim。跳过；超过宽限期的
///   说明认领者崩了，内容本就判了死刑，接着清。
/// - [`SlotState::Unknown`]：既可能是 `create_dir` 刚返回、标记还没落盘的微秒级
///   窗口，也可能是异常残留。按与进行中同样最保守的一档处理：**绝不当正式条目
///   清掉**，否则会误删一个正在进行的删除。
///
/// 删除 victim 前先原子改名认领：两个清理线程撞同一个 victim 时谁先改名成功谁
/// 拿到，另一方 rename 必 NotFound——「两个清理者同时删同一个槽位」这条竞态
/// 从结构上不存在，跨进程也成立。
///
/// 这里用 `remove_dir_all`，它**解链接而不跟进目标**，这一条至关重要：槽位里存的是
/// 原样搬进来的工作树，node_modules 里几千个 junction 要是被跟进去删，会连累链接
/// 指向的地方。有测试钉着这个不变量。
fn prune_trash(trash_root: &Path) {
    let Ok(entries) = std::fs::read_dir(trash_root) else {
        return;
    };
    let mut slots: Vec<(u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = entry.path();
        match slot_state(&path) {
            SlotState::Deleting | SlotState::Unknown => {
                let has_content = has_entry(&path, "worktree") || has_entry(&path, "admin");
                if should_reap_partial(dir_age(&path), has_content) {
                    let _ = std::fs::remove_dir_all(&path);
                }
            }
            SlotState::Pruning => {
                if should_reap_pruning(dir_age(&path)) {
                    let _ = std::fs::remove_dir_all(&path);
                }
            }
            SlotState::Committed => {
                // 读不出元数据的正式条目当成最旧的，优先清掉
                let ts = read_meta(&path, MARK_COMMITTED)
                    .map(|m| m.deleted_at)
                    .unwrap_or(0);
                slots.push((ts, path));
            }
        }
    }
    if slots.len() <= TRASH_KEEP {
        return;
    }
    // read_dir 顺序未定义；相同秒级时间戳必须再按路径排序，让并发清理者
    // 尝试认领同一组 victim，避免各自成功删除不同条目而跌破保留数量。
    sort_slots(&mut slots);
    for (_, dir) in slots.iter().take(slots.len() - TRASH_KEEP) {
        // 认领失败 = 另一个清理线程先拿到了，放弃这个 victim
        let Some(claimed) = claim_for_pruning(dir) else {
            continue;
        };
        let _ = std::fs::remove_dir_all(&claimed);
    }
}

/// 清理线程认领 victim：把 [`MARK_COMMITTED`] 原子改名成 [`MARK_PRUNING`]。
/// 认领失败（被别的清理线程认领、已不在）就放弃这个 victim，等下一轮清理。
///
/// 返回的路径就是 `dir` 本身：目录名不随状态变化，保留返回值只为让调用方
/// 明确拿到「已认领」这个事实。
fn claim_for_pruning(dir: &Path) -> Option<PathBuf> {
    std::fs::rename(dir.join(MARK_COMMITTED), dir.join(MARK_PRUNING))
        .ok()
        .map(|_| dir.to_path_buf())
}

/// 把清理放到后台线程跑。
///
/// 清掉一个装了 node_modules 的旧槽位是分钟级操作（实测 10 万文件两分多钟）。
/// 同步跑的话，用户的「删除」点击会替之前删除攒下的回收站买单；搬移本身只要
/// 毫秒级，后台化之后删除的耗时就和回收站里躺了什么无关。清理快慢只影响磁盘
/// 空间回收速度，用户感知不到。
///
/// 并发清理由认领机制保证：victim 先把标记原子改名成 [`MARK_PRUNING`] 再删，
/// 两个线程撞同一个 victim 时慢者 rename 失败直接放弃。应用中途退出留下的残留
/// 分两类：空的进行中槽位和半截 [`SlotState::Pruning`] 由 24 小时后的清理回收；
/// 含工作树/管理目录的进行中槽位是用户数据的唯一副本，永久保留。
fn spawn_prune(trash_root: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("trash-prune".into())
        .spawn(move || prune_trash(&trash_root));
}

/// 启动时在后台清理这些仓库的回收站。
///
/// 兜住「上次退出时后台清理没跑完」的残留：用户不再删除的话，残留靠下次启动
/// 收回。路径可能是主仓库也可能是 worktree，`main_repo` 各自归位；问不出来的
/// 路径（盘没挂上之类）跳过，清理永远不算错误。
pub fn prune_trash_background(paths: Vec<String>) {
    let _ = std::thread::Builder::new()
        .name("trash-prune-startup".into())
        .spawn(move || {
            for path in paths {
                let Ok(main) = main_repo(&path) else {
                    continue;
                };
                prune_trash(&main.join(".worktrees").join(".trash"));
            }
        });
}

/// 比较两个路径是否指向同一个地方。
///
/// `git worktree list` 给的是正斜杠，界面传回来的可能是反斜杠，直接比字符串
/// 在 Windows 上会误判。
///
/// 但归一化分隔符和大小写还不够：**两个写法完全不同的字符串照样可能是同一个
/// 目录**——junction / symlink、8.3 短名（`C:\Users\RUNNER~1\...`）、`subst` 出来的
/// 盘符都会造成这种情况。字符串比不出来时必须问文件系统，它才是权威。
///
/// 这一条不是洁癖：`same_path` 同时撑着「锁住的 worktree 不许删」和「主工作区不许
/// 删」两道保护，判错的方向是**放行**——保护会往不安全的方向失效。
fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    if norm(a) == norm(b) {
        return true;
    }
    // 只在字符串已经不等时才走：canonicalize 要碰盘，而且路径不存在就会失败。
    // 任一侧解析不出来就维持「不是同一个」，与原来的行为一致。
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::same_path;

    /// 造一个「已经装进整棵工作树、并且里面有文件正被打开」的槽位。
    ///
    /// 这是删除流程走到落名那一刻的**真实形态**：两次搬移刚做完，槽位里是完整的
    /// 工作树，而扫描器 / 编辑器 / watcher 随时可能正打开着其中某个文件。
    /// 返回的句柄必须由调用方持有到断言之后，提前 drop 就测不到东西了。
    fn slot_with_open_handle(
        trash: &std::path::Path,
        name: &str,
    ) -> (std::path::PathBuf, std::fs::File) {
        let slot = super::new_trash_slot(trash, name).unwrap();
        super::write_meta(
            &slot,
            &super::TrashMeta {
                worktree: format!("/wt/{name}"),
                admin: format!("/admin/{name}"),
                deleted_at: 100,
            },
        )
        .unwrap();
        let inner = slot.join("worktree").join("node_modules").join("pkg");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir_all(slot.join("admin")).unwrap();
        let file = inner.join("index.js");
        std::fs::write(&file, "被别人打开着").unwrap();
        let handle = std::fs::File::open(&file).unwrap();
        (slot, handle)
    }

    /// 落名绝不能因为槽位子树里有打开的句柄而失败。
    ///
    /// Windows 上重命名目录要求**整棵子树没有任何打开的句柄**（连带
    /// `FILE_SHARE_DELETE` 的规矩开法也挡）。把状态编码在目录名后缀上、靠重命名
    /// 槽位目录来提交的做法，因此会在「刚把整棵工作树搬进来」的那一刻被随便哪个
    /// 扫描器撞掉，报 `Access is denied. (os error 5)`。
    ///
    /// 这个用例只在 Windows 上有鉴别力：POSIX 的 rename 不看子树句柄，Unix 上恒过。
    #[test]
    fn commit_survives_open_handles_inside_the_slot() {
        let dir = tempfile::TempDir::new().unwrap();
        let (slot, _handle) = slot_with_open_handle(dir.path(), "feature");

        super::commit_slot(&slot).expect("落名不能因为子树里有打开的句柄而失败");

        assert_eq!(
            super::slot_state(&slot),
            super::SlotState::Committed,
            "落名之后必须是正式条目"
        );
        assert!(
            super::read_meta(&slot, super::MARK_COMMITTED).is_ok(),
            "落名之后必须能按正式条目读出元数据"
        );
    }

    /// 清理认领同样不能被子树里的句柄挡住：认领失败会被 `let _ =` 吞掉，
    /// 表现为回收站悄悄超限、磁盘不回收。
    #[test]
    fn prune_claim_survives_open_handles_inside_the_slot() {
        let dir = tempfile::TempDir::new().unwrap();
        let (slot, _handle) = slot_with_open_handle(dir.path(), "feature");
        super::commit_slot(&slot).unwrap();

        assert!(
            super::claim_for_pruning(&slot).is_some(),
            "清理认领不能因为子树里有打开的句柄而失败"
        );
    }

    /// 进行中槽位对清理不可见：回收站超限时清理只动正式条目，
    /// 带进行中 / 清理认领标记且年龄新的必须原样保留。
    #[test]
    fn prune_never_touches_in_progress_slots() {
        let dir = tempfile::TempDir::new().unwrap();
        let trash = dir.path();
        // 6 个正式条目，超出 TRASH_KEEP（5）一个
        for i in 0..6u64 {
            let slot = trash.join(format!("w{i}-100"));
            std::fs::create_dir(&slot).unwrap();
            super::write_meta(
                &slot,
                &super::TrashMeta {
                    worktree: format!("w{i}"),
                    admin: format!("a{i}"),
                    deleted_at: i, // 越旧越该先清
                },
            )
            .unwrap();
            super::commit_slot(&slot).unwrap();
        }
        // 删除搬到一半：只有进行中标记
        let deleting = trash.join("wip-100");
        std::fs::create_dir(&deleting).unwrap();
        std::fs::write(deleting.join(super::MARK_DELETING), "{}").unwrap();
        std::fs::write(deleting.join("fragile.txt"), "搬了一半").unwrap();
        // 别的清理线程认领中
        let pruning = trash.join("old-99");
        std::fs::create_dir(&pruning).unwrap();
        std::fs::write(pruning.join(super::MARK_PRUNING), "{}").unwrap();
        std::fs::write(pruning.join("half.txt"), "删了一半").unwrap();

        super::prune_trash(trash);

        assert!(
            deleting.join("fragile.txt").exists(),
            "进行中槽位绝不能被清"
        );
        assert!(pruning.join("half.txt").exists(), "别人认领中的槽位不能碰");
        assert!(!trash.join("w0-100").exists(), "清掉的该是最旧的正式条目");
        assert_eq!(
            std::fs::read_dir(trash).unwrap().count(),
            5 + 2,
            "正式条目收敛到 5 个，进行中/认领中的原样保留且不计入条数"
        );
    }

    /// 一个标记都没有的槽位既可能是 `create_dir` 刚返回、标记还没落盘的窗口，
    /// 也可能是异常残留。含内容时永远不清——那是用户数据的唯一副本，
    /// 而且当成正式条目清掉等于误删一个**正在进行的删除**。
    #[test]
    fn slots_without_a_marker_are_never_treated_as_committed() {
        let dir = tempfile::TempDir::new().unwrap();
        let trash = dir.path();
        for i in 0..6u64 {
            let slot = trash.join(format!("w{i}-100"));
            std::fs::create_dir(&slot).unwrap();
            super::write_meta(
                &slot,
                &super::TrashMeta {
                    worktree: format!("w{i}"),
                    admin: format!("a{i}"),
                    deleted_at: 100 + i,
                },
            )
            .unwrap();
            super::commit_slot(&slot).unwrap();
        }
        let bare = trash.join("bare-1");
        std::fs::create_dir_all(bare.join("worktree")).unwrap();
        std::fs::write(bare.join("worktree").join("data.txt"), "唯一副本").unwrap();

        assert_eq!(super::slot_state(&bare), super::SlotState::Unknown);
        super::prune_trash(trash);

        assert!(
            bare.join("worktree").join("data.txt").exists(),
            "无标记且含内容的槽位绝不能被清"
        );
        assert!(
            !trash.join("w0-100").exists(),
            "该被清的仍是最旧的正式条目，无标记槽位不参与选举"
        );
    }

    /// 升级前留下的半截槽位（状态编码在目录名后缀上）里同样有 `meta.json`，
    /// 光看标记文件会把它判成正式条目、参与清理选举，把未提交内容的唯一副本删掉。
    /// 后缀的优先级必须高于标记文件。
    #[test]
    fn legacy_suffixed_residue_is_never_treated_as_committed() {
        let dir = tempfile::TempDir::new().unwrap();
        let trash = dir.path();
        // 5 个正式条目，已经顶满 TRASH_KEEP
        for i in 0..5u64 {
            let slot = trash.join(format!("w{i}-200"));
            std::fs::create_dir(&slot).unwrap();
            super::write_meta(
                &slot,
                &super::TrashMeta {
                    worktree: format!("w{i}"),
                    admin: format!("a{i}"),
                    deleted_at: 200 + i,
                },
            )
            .unwrap();
            super::commit_slot(&slot).unwrap();
        }
        // 旧版本崩在搬移中途留下的残留：时间戳最旧，会第一个被选中
        let legacy = trash.join(format!("half-1{}", super::LEGACY_PARTIAL_SUFFIX));
        std::fs::create_dir_all(legacy.join("worktree")).unwrap();
        std::fs::write(legacy.join("worktree").join("data.txt"), "唯一副本").unwrap();
        super::write_meta(
            &legacy,
            &super::TrashMeta {
                worktree: "half".into(),
                admin: "half-admin".into(),
                deleted_at: 1,
            },
        )
        .unwrap();
        // 旧格式的 meta 就叫 meta.json，和新的正式标记同名——这正是误判的来源
        std::fs::rename(
            legacy.join(super::MARK_DELETING),
            legacy.join(super::MARK_COMMITTED),
        )
        .unwrap();

        assert_eq!(
            super::slot_state(&legacy),
            super::SlotState::Deleting,
            "带旧 .partial 后缀的残留必须按进行中处理，哪怕里面躺着 meta.json"
        );
        super::prune_trash(trash);

        assert!(
            legacy.join("worktree").join("data.txt").exists(),
            "旧格式的半截槽位含用户数据，绝不能被清"
        );

        // 旧的正式条目（无后缀 + meta.json）则天然兼容
        let legacy_committed = trash.join("old-150");
        std::fs::create_dir(&legacy_committed).unwrap();
        std::fs::write(legacy_committed.join(super::MARK_COMMITTED), "{}").unwrap();
        assert_eq!(
            super::slot_state(&legacy_committed),
            super::SlotState::Committed
        );
    }

    /// 数据搬移本身绕不开重命名带内容的目录，只能等对方松手：Windows 上句柄没
    /// 放开时 rename 报 `PermissionDenied`，重试窗口内应当熬过去。
    /// 只在 Windows 上有鉴别力，Unix 上首次就成功。
    #[test]
    fn rename_retry_waits_out_a_transient_lock() {
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        let file = src.join("sub").join("held.txt");
        std::fs::write(&file, "x").unwrap();

        let handle = std::fs::File::open(&file).unwrap();
        let holder = std::thread::spawn(move || {
            // 明显短于 RENAME_RETRIES * RENAME_BACKOFF（1 秒）的占用窗口
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(handle);
        });

        let dst = dir.path().join("dst");
        super::rename_retry(&src, &dst).expect("瞬时占用应当被重试熬过去");

        holder.join().unwrap();
        assert!(dst.join("sub").join("held.txt").exists());
    }

    /// 并发 prune 各自拿到的 read_dir 顺序可能不同；时间戳相同时还必须按路径打破
    /// 平局，保证所有清理者尝试认领同一组最旧 victim，而不是各删一组。
    #[test]
    fn equal_timestamp_victims_have_a_stable_path_order() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = dir.path().join("a-100");
        let z = dir.path().join("z-100");
        let mut first = vec![(100, z.clone()), (100, a.clone())];
        let mut second = vec![(100, a.clone()), (100, z.clone())];

        super::sort_slots(&mut first);
        super::sort_slots(&mut second);

        assert_eq!(first, second);
        assert_eq!(first, vec![(100, a), (100, z)]);
    }

    /// 进行中残留什么时候可以清：旧的、且能证明为空的才清；含用户数据的
    /// （worktree/ admin 子项）是唯一副本，永久保留；年龄读不出的宁可漏清。
    #[test]
    fn partial_slots_are_reaped_only_when_provably_empty_and_old() {
        use std::time::Duration;
        let young = Duration::from_secs(60);
        let old = Duration::from_secs(25 * 3600);
        assert!(!super::should_reap_partial(None, false));
        assert!(!super::should_reap_partial(Some(young), false));
        assert!(!super::should_reap_partial(Some(old), true));
        assert!(super::should_reap_partial(Some(old), false));
        // .pruning 残留的内容本就判了死刑，只看年龄
        assert!(super::should_reap_pruning(Some(old)));
        assert!(!super::should_reap_pruning(Some(young)));
        assert!(!super::should_reap_pruning(None));
    }

    /// 清理认领是原子的：同一个 victim 只会被一个清理线程拿到。
    #[test]
    fn a_victim_can_be_claimed_only_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let victim = dir.path().join("y-100");
        std::fs::create_dir(&victim).unwrap();
        std::fs::write(victim.join(super::MARK_COMMITTED), "{}").unwrap();

        let claimed = super::claim_for_pruning(&victim).expect("第一个认领者必须拿到");
        assert_eq!(claimed, victim, "目录名不随认领变化");
        assert_eq!(super::slot_state(&victim), super::SlotState::Pruning);

        assert!(
            super::claim_for_pruning(&victim).is_none(),
            "已被认领的 victim 不能被二次认领"
        );
        assert!(
            super::claim_for_pruning(&dir.path().join("z-100")).is_none(),
            "不存在的条目认领不到"
        );
    }

    /// 「目录已丢」只认明确的 NotFound：在的报在，不在的报不在。
    #[test]
    fn dir_probe_distinguishes_present_from_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(super::dir_exists(dir.path()).unwrap());
        assert!(!super::dir_exists(&dir.path().join("missing")).unwrap());
    }

    /// 探测出错（而不是 NotFound）必须报错——折叠成「不存在」会跳过脏检查、
    /// 只搬登记，让真实存在的工作树变成无主数据。
    ///
    /// Windows 上没有不提权、不依赖环境的便携造错法（文件当中间组件报的是
    /// NotFound），错误分支只在 Unix 上测；root 下权限拦截不生效，这个用例会
    /// 假红，属已知限制。
    #[cfg(unix)]
    #[test]
    fn dir_probe_errors_instead_of_assuming_missing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let err = super::dir_exists(&locked.join("child"))
            .expect_err("探测失败必须报错，不能当成目录已丢");
        assert_eq!(err.code, "dir_probe");
        // 恢复权限，TempDir 收尾才删得掉
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    /// 登记反查全平台精确匹配：大小写不同就是不同路径（Windows 可以按目录
    /// 开大小写敏感），Windows 上只额外归一分隔符。
    #[test]
    fn registry_spelling_match_is_exact_everywhere() {
        assert!(super::spelling_equal("/wt/foo", "/wt/foo"));
        assert!(super::spelling_equal("/wt/foo/", "/wt/foo"));
        assert!(!super::spelling_equal("/wt/foo", "/wt/foo2"));
        // 任何平台都不折叠大小写：Foo 和 foo 可能是两条不同的登记
        assert!(!super::spelling_equal("/wt/Foo", "/wt/foo"));
        #[cfg(windows)]
        assert!(super::spelling_equal("C:/wt/foo", "C:\\wt\\foo"));
        // 反斜杠在非 Windows 上是合法文件名字符，不能归一
        #[cfg(not(windows))]
        assert!(!super::spelling_equal("/wt/a\\b", "/wt/a/b"));
    }

    /// 拼写对不上（哪怕只差大小写）：拒删，不猜。这一条是「绝不折叠误命中」
    /// 的直接钉子，任何平台都能造。
    #[test]
    fn registry_lookup_refuses_case_mismatch() {
        let dir = tempfile::TempDir::new().unwrap();
        let common = dir.path();
        let admin = common.join("worktrees").join("foo");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(admin.join("gitdir"), "D:/wt/foo/.git\n").unwrap();

        let found = super::admin_from_registry(common, "D:/wt/foo")
            .unwrap()
            .unwrap();
        assert!(found.ends_with("foo"));
        assert!(
            super::admin_from_registry(common, "D:/wt/FOO")
                .unwrap()
                .is_none(),
            "大小写对不上时必须找不到，而不是折叠命中"
        );
    }

    /// 双大小写登记（大小写敏感的目录里 Foo 和 foo 共存）必须各查各的。
    /// Windows 默认卷造不出两个只差大小写的兄弟目录（create_dir_all 会落进
    /// 同一个目录），这个形态只在大小写敏感的文件系统上测。
    #[cfg(not(windows))]
    #[test]
    fn registry_lookup_distinguishes_case_variants() {
        let dir = tempfile::TempDir::new().unwrap();
        let common = dir.path();
        for (entry, target) in [("Foo", "/wt/Foo"), ("foo", "/wt/foo")] {
            let admin = common.join("worktrees").join(entry);
            std::fs::create_dir_all(&admin).unwrap();
            std::fs::write(admin.join("gitdir"), format!("{target}/.git\n")).unwrap();
        }
        let found = super::admin_from_registry(common, "/wt/foo")
            .unwrap()
            .unwrap();
        assert!(found.ends_with("foo"), "命中错了：{found:?}");
        let found = super::admin_from_registry(common, "/wt/Foo")
            .unwrap()
            .unwrap();
        assert!(found.ends_with("Foo"), "命中错了：{found:?}");
    }

    /// 两条登记指向同一路径 = 登记损坏。分不清哪条是真的时，
    /// 唯一安全的动作是什么都不动。
    #[test]
    fn duplicate_registrations_are_refused() {
        let dir = tempfile::TempDir::new().unwrap();
        let common = dir.path();
        for entry in ["x", "x.1"] {
            let admin = common.join("worktrees").join(entry);
            std::fs::create_dir_all(&admin).unwrap();
            std::fs::write(admin.join("gitdir"), "D:/wt/x/.git\n").unwrap();
        }
        let err = super::admin_from_registry(common, "D:/wt/x").expect_err("重复登记必须拒绝");
        assert_eq!(err.code, "worktree_ambiguous");
    }

    /// `git worktree list` 吐正斜杠，界面传回来的可能是反斜杠。直接比字符串会
    /// 判成两个不同的 worktree，锁检查就会漏过去。
    #[test]
    fn separators_and_case_do_not_change_the_path() {
        assert!(same_path(
            "C:/repo/.worktrees/feat",
            "C:\\repo\\.worktrees\\feat"
        ));
        assert!(same_path("C:/Repo/WT", "c:/repo/wt"));
        assert!(same_path("C:/repo/wt/", "C:/repo/wt"));
    }

    #[test]
    fn different_worktrees_stay_different() {
        assert!(!same_path("C:/repo/.worktrees/a", "C:/repo/.worktrees/b"));
        // 前缀相同也不能算同一个
        assert!(!same_path("C:/repo/wt", "C:/repo/wt2"));
    }

    /// 归一化解决不了的那一类：两个写法差得很远，却是同一个目录。
    ///
    /// junction 只是最好复现的代表，8.3 短名和 `subst` 盘符走的是同一条失效路径。
    #[cfg(windows)]
    #[test]
    fn different_spellings_of_one_directory_are_the_same_path() {
        use std::os::windows::process::CommandExt;

        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("inner")).unwrap();
        let link = dir.path().join("link");
        let out = std::process::Command::new("cmd")
            .arg("/c")
            .raw_arg(format!(
                "mklink /J \"{}\" \"{}\"",
                link.display(),
                real.display()
            ))
            .output()
            .expect("cmd 起不来");
        assert!(
            out.status.success(),
            "mklink 失败: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        // 字符串上毫无共同点，落到的是同一个目录
        assert!(same_path(
            real.join("inner").to_str().unwrap(),
            link.join("inner").to_str().unwrap()
        ));
        // 但不能因此放宽成「沾边就算同一个」
        assert!(!same_path(
            real.join("inner").to_str().unwrap(),
            link.join("elsewhere").to_str().unwrap()
        ));
    }
}
