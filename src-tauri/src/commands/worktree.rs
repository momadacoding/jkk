use crate::engine::git_cli::run_git;
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use crate::engine::worktree::{read_worktrees, Worktree};

/// 回收站里最多留几个条目。超出的按时间从旧到新清掉。
///
/// 这里存的是完整的工作树，装了 node_modules 的能到几个 GB，不能无限攒。
const TRASH_KEEP: usize = 5;

/// 进行中槽位的后缀：两次搬移和 meta 落盘全部完成之前，槽位名一直带着它；
/// 撤销开始时也会把正式槽位改名带上它来认领。
///
/// 清理线程看到带后缀的一律跳过——「清理绝不删掉正在搬移/正在撤销的槽位」
/// 靠这个结构保证，不靠时序余量。改名是原子操作，认领互斥跨进程也成立。
///
/// 用后缀而不用前缀：worktree 名字是用户输入，名字本身可能以前缀开头，
/// 而后缀是我们在时间戳之后追加的，任何用户名字都撞不上。
const PARTIAL_SUFFIX: &str = ".partial";

/// 清理线程认领 victim 时改名的后缀。认领后的目录只有认领者自己删；
/// 别的清理线程和撤销都当它是别人的，跳过。
const PRUNING_SUFFIX: &str = ".pruning";

/// 带后缀残留的自动回收时限。只适用于**能证明没有用户数据**的残留：
/// 空的或只写了 meta 的 `.partial`、以及已被判定为垃圾的 `.pruning`。
/// 含 `worktree/` 或 `admin/` 的 `.partial` 是未提交内容的唯一副本
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

/// 删除成功后交给前端的凭据，用来撤销。
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeletedWorktree {
    pub name: String,
    /// 回收站槽位的绝对路径，`restore_worktree` 的唯一入参。
    pub trash_path: String,
}

/// 回收站槽位里的 `meta.json`。撤销时靠它知道两个目录该搬回哪。
///
/// 路径存绝对路径而不是相对：撤销时不需要再推导一遍，少一处能算错的地方。
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

/// 在回收站里开一个不重名的进行中槽位（带 [`PARTIAL_SUFFIX`] 后缀）。
///
/// 名字带秒级时间戳，同一秒删两个同名 worktree 会撞上，所以再加序号兜底。
fn new_trash_slot(trash_root: &Path, name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(trash_root)?;
    let stamp = now_secs();
    for n in 0..1000 {
        let slot = if n == 0 {
            trash_root.join(format!("{name}-{stamp}{PARTIAL_SUFFIX}"))
        } else {
            trash_root.join(format!("{name}-{stamp}-{n}{PARTIAL_SUFFIX}"))
        };
        if !slot.exists() {
            std::fs::create_dir(&slot)?;
            return Ok(slot);
        }
    }
    Err(GitError::new(
        "trash_slot",
        "回收站里同名条目太多，建不出新槽位".to_string(),
    ))
}

/// 把搬完了的槽位摘掉 `.partial` 后缀，变成正式的回收站条目。
///
/// 同卷 rename 原子，清理线程要么看到带后缀的旧名（跳过），要么看到摘了后缀的
/// 新名（meta 已在里面），不存在中间态。创建时避过让的是带后缀的名字，去后缀后
/// 的名字可能被同秒同名的前一次删除占掉，所以这里再兜一轮序号（`-c{n}`，
/// 和创建时的 `-{n}` 区分开）。
fn commit_slot(slot: &Path) -> Result<PathBuf> {
    let Some(file) = slot.file_name().and_then(|s| s.to_str()) else {
        return Err(GitError::new(
            "trash_slot",
            "回收站槽位名不是合法 UTF-8".to_string(),
        ));
    };
    let Some(base) = file.strip_suffix(PARTIAL_SUFFIX) else {
        return Err(GitError::new(
            "trash_slot",
            format!("回收站槽位缺少进行中后缀：{file}"),
        ));
    };
    let parent = slot.parent().unwrap_or(Path::new("."));
    for n in 0..1000 {
        let target = if n == 0 {
            parent.join(base)
        } else {
            parent.join(format!("{base}-c{n}"))
        };
        if !target.exists() {
            std::fs::rename(slot, &target)?;
            return Ok(target);
        }
    }
    Err(GitError::new(
        "trash_slot",
        "回收站里同名条目太多，落不了名".to_string(),
    ))
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
/// （既没有 `worktree/` 也没有 `admin/`）。含内容的 `.partial` 是未提交内容的
/// 唯一副本——删除崩在搬移中途、或回滚失败——永久保留等用户处理，绝不自动删。
fn should_reap_partial(age: Option<std::time::Duration>, has_content: bool) -> bool {
    !has_content && matches!(age, Some(a) if a >= std::time::Duration::from_secs(RESIDUE_TTL_SECS))
}

/// 已被认领为清理对象的 `.pruning` 残留：内容本来就是按策略判了死刑的旧条目，
/// 超过宽限期（认领它的线程崩了/被抢占）就接着清。
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

/// 认领一个正式回收站槽位：原子改名加上 [`PARTIAL_SUFFIX`]，清理器从这一刻起
/// 结构性跳过它。与清理器的 victim 认领（改名 [`PRUNING_SUFFIX`]）互斥——
/// 谁先改名成功谁拿到，另一方的 rename 必然 NotFound，跨进程也成立。
///
/// 撤销结束后槽位被删空；撤销失败时尽量改回原名，让前端持有的路径还能重试。
fn claim_slot(slot: &Path) -> Result<PathBuf> {
    let Some(name) = slot.file_name().and_then(|s| s.to_str()) else {
        return Err(GitError::new(
            "trash_slot",
            "回收站槽位名不是合法 UTF-8".to_string(),
        ));
    };
    if name.ends_with(PARTIAL_SUFFIX) || name.ends_with(PRUNING_SUFFIX) {
        return Err(GitError::new(
            "trash_state",
            "这个回收站条目正被别的操作占用，稍后再试。".to_string(),
        ));
    }
    // 认领养名可能撞上正在进行的同名删除（同秒同名），换序号再试
    for n in 0..1000 {
        let claimed = if n == 0 {
            slot.with_file_name(format!("{name}{PARTIAL_SUFFIX}"))
        } else {
            slot.with_file_name(format!("{name}-r{n}{PARTIAL_SUFFIX}"))
        };
        if claimed.exists() {
            continue;
        }
        match std::fs::rename(slot, &claimed) {
            Ok(()) => return Ok(claimed),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(GitError::new(
                    "trash_gone",
                    "这个回收站条目已经不在了（可能刚被自动清理）。".to_string(),
                ))
            }
            Err(e) => {
                return Err(GitError::new(
                    "restore_failed",
                    format!("认领回收站条目失败：{e}"),
                ))
            }
        }
    }
    Err(GitError::new(
        "trash_slot",
        "回收站里同名条目太多，认领不了".to_string(),
    ))
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
                     确认要删的话可以继续，删掉之后还能从回收站撤销。"
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

    // meta 在任何搬移之前落盘。进行中槽位靠 .partial 后缀对清理不可见，先写 meta
    // 是第二层防护：摘后缀落名的那一刻，槽位内容（meta/worktree/admin）必须完整。
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
        if let Err(err) = std::fs::rename(&worktree_path, &slot_worktree) {
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
    if let Err(err) = std::fs::rename(&admin, &slot_admin) {
        let rolled_back =
            !worktree_exists || std::fs::rename(&slot_worktree, &worktree_path).is_ok();
        if rolled_back {
            remove_slot(&slot);
        }
        // 工作树搬不回去时槽位连 meta 一起留着，它仍是一个可撤销的回收站条目。
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

    // 落名提交：摘掉 .partial 后缀，清理线程从这一刻起才把它当回收站条目。
    // 失败就把两道工序都搬回去，当作没删过。
    let committed = match commit_slot(&slot) {
        Ok(committed) => committed,
        Err(err) => {
            let worktree_back =
                !worktree_exists || std::fs::rename(&slot_worktree, &worktree_path).is_ok();
            let admin_back = std::fs::rename(&slot_admin, &admin).is_ok();
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
                         这个目录带 .partial 后缀，不会被自动清理，手工搬回即可。",
                        err.message,
                        slot.display()
                    )
                },
            ));
        }
    };

    spawn_prune(trash_root);

    Ok(DeletedWorktree {
        name,
        trash_path: committed.to_string_lossy().to_string(),
    })
}

fn write_meta(slot: &Path, meta: &TrashMeta) -> Result<()> {
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| GitError::new("trash_meta", format!("写不出回收站元数据：{e}")))?;
    std::fs::write(slot.join("meta.json"), json)?;
    Ok(())
}

/// 清掉一个只写了 meta、还没搬进去东西的槽位（删除中途失败的收尾）。
fn remove_slot(slot: &Path) {
    let _ = std::fs::remove_file(slot.join("meta.json"));
    let _ = std::fs::remove_dir(slot);
}

fn read_meta(slot: &Path) -> Result<TrashMeta> {
    let raw = std::fs::read_to_string(slot.join("meta.json")).map_err(|e| {
        GitError::new(
            "trash_meta",
            format!("这个回收站条目读不出元数据，撤销不了：{e}"),
        )
    })?;
    serde_json::from_str(&raw)
        .map_err(|e| GitError::new("trash_meta", format!("回收站元数据坏了，撤销不了：{e}")))
}

fn begin_restore(slot: &Path) -> Result<(PathBuf, TrashMeta)> {
    let claimed = claim_slot(slot)?;
    match read_meta(&claimed) {
        Ok(meta) => Ok((claimed, meta)),
        Err(error) => Err(restore_bail_with_error(&claimed, slot, error, true)),
    }
}

fn rollback_restored_worktree(worktree: &Path, slot_worktree: &Path, has_worktree: bool) -> bool {
    !has_worktree || std::fs::rename(worktree, slot_worktree).is_ok()
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
/// 回收站里有三种目录，分治：
/// - 正式条目：参与 victim 选举，超限后最旧的被清。
/// - 带 [`PARTIAL_SUFFIX`] 的：正在搬移（删除）或正在搬回（撤销）的槽位，
///   **一律跳过、不计入条数**。只有超过宽限期、且能证明为空（无 worktree/
///   admin 子项）的残留才清——含内容的是未提交内容的唯一副本，永久保留。
/// - 带 [`PRUNING_SUFFIX`] 的：已被某个清理线程认领的 victim。跳过；
///   超过宽限期的说明认领者崩了，内容本就判了死刑，接着清。
///
/// 删除 victim 前先原子改名认领：与撤销的认领互斥，谁先改名成功谁拿到，
/// 另一方 rename 必 NotFound——「清理删掉别人正在用的槽位」这条竞态
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
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with(PARTIAL_SUFFIX) {
            let has_content = has_entry(&path, "worktree") || has_entry(&path, "admin");
            if should_reap_partial(dir_age(&path), has_content) {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        if name.ends_with(PRUNING_SUFFIX) {
            if should_reap_pruning(dir_age(&path)) {
                let _ = std::fs::remove_dir_all(&path);
            }
            continue;
        }
        // 读不出元数据的正式条目当成最旧的，优先清掉
        let ts = read_meta(&path).map(|m| m.deleted_at).unwrap_or(0);
        slots.push((ts, path));
    }
    if slots.len() <= TRASH_KEEP {
        return;
    }
    // read_dir 顺序未定义；相同秒级时间戳必须再按路径排序，让并发清理者
    // 尝试认领同一组 victim，避免各自成功删除不同条目而跌破保留数量。
    sort_slots(&mut slots);
    for (_, dir) in slots.iter().take(slots.len() - TRASH_KEEP) {
        // 认领失败 = 别人（撤销或另一个清理线程）先拿到了，放弃这个 victim
        let Some(claimed) = claim_for_pruning(dir) else {
            continue;
        };
        let _ = std::fs::remove_dir_all(&claimed);
    }
}

/// 清理线程认领 victim：原子改名加 [`PRUNING_SUFFIX`]。认领失败（被撤销拿走、
/// 被别的清理线程认领、已不在）就放弃这个 victim，等下一轮清理。
fn claim_for_pruning(dir: &Path) -> Option<PathBuf> {
    let name = dir.file_name()?.to_str()?;
    let claimed = dir.with_file_name(format!("{name}{PRUNING_SUFFIX}"));
    if claimed.exists() {
        return None;
    }
    std::fs::rename(dir, &claimed).ok().map(|_| claimed)
}

/// 把清理放到后台线程跑。
///
/// 清掉一个装了 node_modules 的旧槽位是分钟级操作（实测 10 万文件两分多钟）。
/// 同步跑的话，用户的「删除」点击会替之前删除攒下的回收站买单；搬移本身只要
/// 毫秒级，后台化之后删除的耗时就和回收站里躺了什么无关。清理快慢只影响磁盘
/// 空间回收速度，用户感知不到。
///
/// 并发清理由认领机制保证：victim 先原子改名（.pruning）再删，两个线程撞同一个
/// victim 时慢者 rename 失败直接放弃。应用中途退出留下的残留分两类：空的
/// `.partial` 和半截 `.pruning` 由 24 小时后的清理回收；含工作树/管理目录的
/// `.partial` 是用户数据的唯一副本，永久保留。
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

/// 撤销一次删除：把工作树和管理目录原样搬回去。
///
/// 只收回收站槽位的路径，原位置一律从槽位里的 `meta.json` 读——前端不该有机会
/// 指定往哪写。
///
/// 搬回去之后 git 的登记自然就完好了（登记本来就是那个管理目录），不需要
/// `worktree add` 或 `repair`：前者对已存在的目录一律拒绝，后者在管理目录丢失后
/// 也救不回来，都验证过。
#[tauri::command(async)]
pub fn restore_worktree(path: String, trash_path: String) -> Result<()> {
    let slot = PathBuf::from(&trash_path);

    // 只认本仓库回收站底下的槽位，避免被指使去搬别处的目录
    let trash_root = main_repo(&path)?.join(".worktrees").join(".trash");
    if !slot.starts_with(&trash_root) {
        return Err(GitError::new(
            "trash_foreign",
            "这个路径不在本仓库的回收站里，不能撤销。".to_string(),
        ));
    }

    // 必须先认领再读取元数据；否则清理器能在两者之间删掉并重建同名槽位，
    // 让本次恢复拿旧 meta 操作一个新的槽位（ABA）。
    let (claimed, meta) = begin_restore(&slot)?;
    let worktree = PathBuf::from(&meta.worktree);
    let admin = PathBuf::from(&meta.admin);

    match dir_exists(&worktree) {
        Ok(true) => {
            return Err(restore_bail_with_error(
                &claimed,
                &slot,
                GitError::new(
                    "worktree_occupied",
                    format!(
                        "原来的位置已经被占了，撤销会覆盖它：\n{}\n\n\
                         先把那里挪开再撤销。",
                        worktree.display()
                    ),
                ),
                true,
            ));
        }
        Ok(false) => {}
        Err(error) => return Err(restore_bail_with_error(&claimed, &slot, error, true)),
    }
    match dir_exists(&admin) {
        Ok(true) => {
            return Err(restore_bail_with_error(
                &claimed,
                &slot,
                GitError::new(
                    "worktree_occupied",
                    format!(
                        "这个 worktree 的登记已经重新存在了，撤销会覆盖它：\n{}",
                        admin.display()
                    ),
                ),
                true,
            ));
        }
        Ok(false) => {}
        Err(error) => return Err(restore_bail_with_error(&claimed, &slot, error, true)),
    }

    let slot_worktree = claimed.join("worktree");
    let slot_admin = claimed.join("admin");
    // 「目录已丢」的删除只搬了管理目录，槽位里没有工作树可还。
    // 必须用严格探测：I/O 错误折叠成 false 的话，会丢 meta 走人，残留槽位
    // 就再也对不上前端持有的路径了。
    let has_worktree = match dir_exists(&slot_worktree) {
        Ok(v) => v,
        Err(e) => {
            return Err(restore_bail(
                &claimed,
                &slot,
                format!("读不出回收站条目的状态：{}", e.message),
            ))
        }
    };

    // 父目录可能已经因为空了被 git 收走，直接 rename 会 NotFound
    if let Some(parent) = admin.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Err(restore_bail(
                &claimed,
                &slot,
                format!("建不回登记目录的父目录：{e}"),
            ));
        }
    }
    if has_worktree {
        if let Some(parent) = worktree.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return Err(restore_bail(
                    &claimed,
                    &slot,
                    format!("建不回工作树的父目录：{e}"),
                ));
            }
        }
        if let Err(e) = std::fs::rename(&slot_worktree, &worktree) {
            return Err(restore_bail(&claimed, &slot, format!("搬不回工作树：{e}")));
        }
    }
    if let Err(e) = std::fs::rename(&slot_admin, &admin) {
        // 工作树已经回去了但登记没回去，那是个坏 worktree。搬回槽位保持原状。
        let rollback_succeeded =
            rollback_restored_worktree(&worktree, &slot_worktree, has_worktree);
        let rollback_status = if rollback_succeeded {
            "工作树已经放回回收站。".to_string()
        } else {
            format!("工作树也没能放回回收站，仍在：\n{}", worktree.display())
        };
        return Err(restore_bail_with_error(
            &claimed,
            &slot,
            GitError::new(
                "restore_failed",
                format!("搬不回登记信息：{e}\n\n{rollback_status}"),
            ),
            rollback_succeeded,
        ));
    }

    let _ = std::fs::remove_file(claimed.join("meta.json"));
    let _ = std::fs::remove_dir(&claimed);
    Ok(())
}

/// 撤销中途失败的收尾：把认领来的槽位改回正式名，前端持有的 trash_path 才能
/// 原样重试；改不回去就在消息里给出新路径——带 .partial 后缀不会被清理碰。
fn restore_bail(claimed: &Path, original: &Path, what: String) -> GitError {
    restore_bail_with_error(
        claimed,
        original,
        GitError::new("restore_failed", what),
        true,
    )
}

fn restore_bail_with_error(
    claimed: &Path,
    original: &Path,
    error: GitError,
    safe_to_unclaim: bool,
) -> GitError {
    let hint = if safe_to_unclaim && std::fs::rename(claimed, original).is_ok() {
        "可以再试一次。".to_string()
    } else {
        format!(
            "回收站条目现在在：\n{}\n\n\
             带 .partial 后缀，不会被自动清理，改回原名或手工处理。",
            claimed.display()
        )
    };
    GitError::new(&error.code, format!("{}\n\n{hint}", error.message))
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

    /// 进行中槽位对清理不可见：回收站超限时清理只动正式条目，
    /// 带 .partial / .pruning 后缀且年龄新的必须原样保留。
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
        }
        let partial = trash.join(format!("wip-100{}", super::PARTIAL_SUFFIX));
        std::fs::create_dir(&partial).unwrap();
        std::fs::write(partial.join("fragile.txt"), "搬了一半").unwrap();
        let pruning = trash.join(format!("old-99{}", super::PRUNING_SUFFIX));
        std::fs::create_dir(&pruning).unwrap();
        std::fs::write(pruning.join("half.txt"), "删了一半").unwrap();

        super::prune_trash(trash);

        assert!(partial.join("fragile.txt").exists(), "进行中槽位绝不能被清");
        assert!(pruning.join("half.txt").exists(), "别人认领中的槽位不能碰");
        assert!(!trash.join("w0-100").exists(), "清掉的该是最旧的正式条目");
        assert_eq!(
            std::fs::read_dir(trash).unwrap().count(),
            5 + 2,
            "正式条目收敛到 5 个，带后缀的原样保留且不计入条数"
        );
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

    /// 认领是原子的：同一槽位只能被认领一次；原名被拿走后后来者得到
    /// trash_gone，已认领/已被清理认领的目录不能被二次认领。
    #[test]
    fn a_slot_can_be_claimed_only_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let slot = dir.path().join("x-100");
        std::fs::create_dir(&slot).unwrap();

        let claimed = super::claim_slot(&slot).unwrap();
        assert!(claimed.ends_with(format!("x-100{}", super::PARTIAL_SUFFIX)));

        let err = super::claim_slot(&slot).expect_err("原名已被拿走，后来者必须失败");
        assert_eq!(err.code, "trash_gone");

        let err = super::claim_slot(&claimed).expect_err("已认领的槽位不能被二次认领");
        assert_eq!(err.code, "trash_state");

        // 清理线程的认领同样互斥
        let victim = dir.path().join("y-100");
        std::fs::create_dir(&victim).unwrap();
        let claimed = super::claim_for_pruning(&victim).unwrap();
        assert!(claimed.ends_with(format!("y-100{}", super::PRUNING_SUFFIX)));
        assert!(super::claim_for_pruning(&victim).is_none());
        let err = super::claim_slot(&claimed).expect_err("清理认领中的槽位不能被撤销认领");
        assert_eq!(err.code, "trash_state");
    }

    /// restore 必须先原子认领槽位，再从被认领的目录读取 meta。返回值把目录身份
    /// 和对应元数据绑在一起，调用方不能拿旧路径读出的 meta 去操作后来重建的槽位。
    #[test]
    fn restore_claim_binds_metadata_to_the_claimed_directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let slot = dir.path().join("x-100");
        std::fs::create_dir(&slot).unwrap();
        super::write_meta(
            &slot,
            &super::TrashMeta {
                worktree: "old-worktree".into(),
                admin: "old-admin".into(),
                deleted_at: 100,
            },
        )
        .unwrap();

        let (claimed, meta) = super::begin_restore(&slot).unwrap();

        assert!(!slot.exists(), "返回 meta 时原路径必须已经被认领走");
        assert!(claimed.exists());
        assert_eq!(meta.worktree, "old-worktree");
        assert_eq!(
            super::read_meta(&claimed).unwrap().worktree,
            meta.worktree,
            "返回的 meta 必须来自被认领的目录"
        );
    }

    /// worktree 已搬出槽位、admin 搬回失败且 worktree 回滚也失败时，claimed 目录
    /// 是唯一能描述残缺状态的位置，绝不能改回会被 prune 选中的正式槽位名。
    #[test]
    fn failed_worktree_rollback_keeps_incomplete_slot_claimed() {
        let dir = tempfile::TempDir::new().unwrap();
        let original = dir.path().join("x-100");
        let claimed = dir.path().join(format!("x-100{}", super::PARTIAL_SUFFIX));
        let slot_worktree = claimed.join("worktree");
        let restored_worktree = dir.path().join("restored-worktree");
        std::fs::create_dir_all(&slot_worktree).unwrap();
        std::fs::write(slot_worktree.join("collision"), "占住回滚目标").unwrap();
        std::fs::create_dir(&restored_worktree).unwrap();
        std::fs::write(restored_worktree.join("uncommitted"), "用户数据").unwrap();

        let rollback_succeeded =
            super::rollback_restored_worktree(&restored_worktree, &slot_worktree, true);
        assert!(!rollback_succeeded, "被占用的回滚目标必须使 rename 失败");
        assert!(restored_worktree.join("uncommitted").exists());

        let error = super::restore_bail_with_error(
            &claimed,
            &original,
            super::GitError::new("restore_failed", "工作树回滚失败"),
            rollback_succeeded,
        );

        assert!(claimed.exists(), "残缺槽位必须保持为清理器不可见的 partial");
        assert!(!original.exists(), "回滚失败时不能重新提交为正式槽位");
        assert!(error.message.contains(&claimed.display().to_string()));
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
