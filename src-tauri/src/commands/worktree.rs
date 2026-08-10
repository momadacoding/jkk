use crate::engine::git_cli::run_git;
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub use crate::engine::worktree::{read_worktrees, Worktree};

/// 回收站里最多留几个条目。超出的按时间从旧到新清掉。
///
/// 这里存的是完整的工作树，装了 node_modules 的能到几个 GB，不能无限攒。
const TRASH_KEEP: usize = 5;

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

/// 在回收站里开一个不重名的槽位。
///
/// 名字带秒级时间戳，同一秒删两个同名 worktree 会撞上，所以再加序号兜底。
fn new_trash_slot(trash_root: &Path, name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(trash_root)?;
    let stamp = now_secs();
    for n in 0..1000 {
        let slot = if n == 0 {
            trash_root.join(format!("{name}-{stamp}"))
        } else {
            trash_root.join(format!("{name}-{stamp}-{n}"))
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
    // 主工作区一票否决，而且要排在所有检查最前面。
    //
    // 这道保护原先是 git 给的（`fatal: '.' is a main working tree`）。现在 git 不再
    // 参与删除，就必须自己拦：主工作区的管理目录**就是仓库的 `.git` 本身**，照着
    // 后面的流程走会把整个 `.git` 搬进回收站。界面上虽然禁着按钮，但那拦不住
    // 直接调命令的路径。
    //
    // 判据：主工作区的 `--absolute-git-dir` 和 `--git-common-dir` 相等，linked
    // worktree 的前者是 `.git/worktrees/<name>`，不相等。
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
    if !force {
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

    // 先搬工作树。这一步失败什么都没动，直接把原因带回去。
    let slot_worktree = slot.join("worktree");
    if let Err(err) = std::fs::rename(&worktree_path, &slot_worktree) {
        let _ = std::fs::remove_dir(&slot);
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

    // 再搬管理目录。失败就把工作树搬回原位——半搬状态下 git 看到的是个坏 worktree。
    let slot_admin = slot.join("admin");
    if let Err(err) = std::fs::rename(&admin, &slot_admin) {
        let rolled_back = std::fs::rename(&slot_worktree, &worktree_path).is_ok();
        let _ = std::fs::remove_dir(&slot);
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

    let meta = TrashMeta {
        worktree: worktree_path.clone(),
        admin: admin.to_string_lossy().to_string(),
        deleted_at: now_secs(),
    };
    // meta 写不成就撤销整个删除：没有它撤不回来，不如当作没删过。
    if let Err(err) = write_meta(&slot, &meta) {
        let _ = std::fs::rename(&slot_admin, &admin);
        let _ = std::fs::rename(&slot_worktree, &worktree_path);
        let _ = std::fs::remove_dir(&slot);
        return Err(err);
    }

    prune_trash(&trash_root);

    Ok(DeletedWorktree {
        name,
        trash_path: slot.to_string_lossy().to_string(),
    })
}

fn write_meta(slot: &Path, meta: &TrashMeta) -> Result<()> {
    let json = serde_json::to_string_pretty(meta)
        .map_err(|e| GitError::new("trash_meta", format!("写不出回收站元数据：{e}")))?;
    std::fs::write(slot.join("meta.json"), json)?;
    Ok(())
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

/// 把最旧的条目清掉，只留 [`TRASH_KEEP`] 个。
///
/// 清不掉不算错误——回收站塞不下顶多占地方，不该让删除操作本身失败。
///
/// 这里用 `remove_dir_all`，它**解链接而不跟进目标**，这一条至关重要：槽位里存的是
/// 原样搬进来的工作树，node_modules 里几千个 junction 要是被跟进去删，会连累链接
/// 指向的地方。有测试钉着这个不变量。
fn prune_trash(trash_root: &Path) {
    let Ok(entries) = std::fs::read_dir(trash_root) else {
        return;
    };
    let mut slots: Vec<(u64, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| {
            let p = e.path();
            // 读不出元数据的当成最旧的，优先清掉
            let ts = read_meta(&p).map(|m| m.deleted_at).unwrap_or(0);
            (ts, p)
        })
        .collect();
    if slots.len() <= TRASH_KEEP {
        return;
    }
    slots.sort_by_key(|(ts, _)| *ts);
    for (_, dir) in slots.iter().take(slots.len() - TRASH_KEEP) {
        let _ = std::fs::remove_dir_all(dir);
    }
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

    let meta = read_meta(&slot)?;
    let worktree = PathBuf::from(&meta.worktree);
    let admin = PathBuf::from(&meta.admin);
    let slot_worktree = slot.join("worktree");
    let slot_admin = slot.join("admin");

    if worktree.exists() {
        return Err(GitError::new(
            "worktree_occupied",
            format!(
                "原来的位置已经被占了，撤销会覆盖它：\n{}\n\n\
                 先把那里挪开再撤销。",
                worktree.display()
            ),
        ));
    }
    if admin.exists() {
        return Err(GitError::new(
            "worktree_occupied",
            format!(
                "这个 worktree 的登记已经重新存在了，撤销会覆盖它：\n{}",
                admin.display()
            ),
        ));
    }

    // 父目录可能已经因为空了被 git 收走，直接 rename 会 NotFound
    if let Some(parent) = worktree.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = admin.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::rename(&slot_worktree, &worktree).map_err(|e| {
        GitError::new(
            "restore_failed",
            format!("搬不回工作树：{e}\n\n回收站里的东西没动，可以再试一次。"),
        )
    })?;
    if let Err(e) = std::fs::rename(&slot_admin, &admin) {
        // 工作树已经回去了但登记没回去，那是个坏 worktree。搬回槽位保持原状。
        let _ = std::fs::rename(&worktree, &slot_worktree);
        return Err(GitError::new(
            "restore_failed",
            format!("搬不回登记信息：{e}\n\n已经放回回收站，可以再试一次。"),
        ));
    }

    let _ = std::fs::remove_file(slot.join("meta.json"));
    let _ = std::fs::remove_dir(&slot);
    Ok(())
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
