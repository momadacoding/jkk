use crate::engine::git_cli::run_git;
use crate::error::Result;

pub use crate::engine::worktree::{read_worktrees, Worktree};

#[tauri::command]
pub fn read_worktrees_cmd(path: String) -> Result<Vec<Worktree>> {
    read_worktrees(&path)
}

/// 主仓库的根目录。
///
/// `path` 可能是主仓库，也可能是任意一个 worktree。`--git-common-dir` 在两种情况下
/// 都指向主仓库的 .git，它的父目录就是主仓库根。新 worktree 统一建在那下面的
/// `.worktrees/`，否则从一个 worktree 里新建会建到仓库外面去。
fn main_repo(path: &str) -> Result<std::path::PathBuf> {
    let common = run_git(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let git_dir = std::path::PathBuf::from(common.trim());
    Ok(git_dir.parent().unwrap_or(&git_dir).to_path_buf())
}

#[tauri::command]
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

/// 删除一个 worktree。
///
/// 收的是 worktree 的**完整路径**而不是名字：名字得靠 `parent(当前仓库)/名字` 拼，
/// 只有当前打开的恰好是它的兄弟 worktree 时才碰巧对；从主仓库删算出来的是仓库外面
/// 的路径。`Worktree` 结构体本来就带 path，直接用。
///
/// 失败时把 git 的原话带回去。`git worktree remove` 拒绝的理由很多（有未跟踪文件、
/// 目录被别的进程占着……），这种时候用户必须看到 git 到底说了什么。
///
/// 它**不是原子的**，失败后可能只删了一半，见 [`finish_removal`]。
#[tauri::command]
pub fn delete_worktree(path: String, worktree_path: String) -> Result<()> {
    match run_git(&path, &["worktree", "remove", &worktree_path]) {
        Ok(_) => Ok(()),
        Err(err) => finish_removal(&path, &worktree_path, err),
    }
}

/// git 删到一半失败时，由我们把剩下的目录删完。
///
/// # 为什么 git 会失败
///
/// 装了 pnpm 依赖的 worktree 基本必然踩到。pnpm 在 Windows 上用 junction 把
/// `node_modules/<包名>` 指向 `node_modules/.pnpm/<包名>@<版本>/...`。而 `.pnpm`
/// 带个点，排序在所有包名前面，于是 git 递归删除时**先把链接的目标删掉**，等走到
/// 那些 junction 时目标已经不存在，它处理不了这种死链接就放弃了。
///
/// 最小复现（`nm/.store/pkg` 是目标，`nm/zlib` 是指向它的 junction）：
///
/// ```text
/// $ git worktree remove wt
/// error: failed to delete '...\wt': Directory not empty
/// 剩余: nm, nm\zlib
/// ```
///
/// 加 `--force` 没用——那只跳过「工作区是否干净」的检查，删除的代码路径一模一样。
///
/// 留下的现场很像「按目录枚举顺序一刀切」：实测那次，排在 `node_modules` 之前的
/// 条目全没了，之后的全留着（1508 个已跟踪文件删掉 464 个）。但别把这当成规律——
/// 另一次实验里 git 跳过了删不掉的死链接、继续处理了后面的条目。枚举顺序和放弃
/// 时机都不受我们控制，所以判断「删到哪一步」只看结果，不靠推演。
///
/// # 为什么由我们接手是合理的
///
/// git 删目录失败也照样解除登记（源码原话：*continue on even if ret is non-zero,
/// there's no going back from here*）。也就是说这时候目录**已经不归 git 管了**，
/// 留着只会是个孤儿。Rust 的 `remove_dir_all` 能处理死链接，而且是**解链接而不是
/// 跟进目标**——这一条至关重要，node_modules 里几千个 junction 要是被跟进去删，
/// 后果不堪设想，有测试钉着。
///
/// 只在「登记已经没了」时才接手。git 在解除登记之前就拒绝（有未提交改动）的话，
/// worktree 还是完好的，那种情形轮不到我们动手，原样把 git 的话带回去。
fn finish_removal(path: &str, worktree_path: &str, err: crate::error::GitError) -> Result<()> {
    let still_registered = read_worktrees(path)
        .map(|list| list.iter().any(|w| same_path(&w.path, worktree_path)))
        .unwrap_or(true); // 查不出来就别乱下结论，按「什么都没动」说

    if still_registered {
        return Err(err);
    }

    let dir = std::path::Path::new(worktree_path);
    if !dir.exists() {
        return Ok(()); // 目录其实已经没了，那就算删成功
    }
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(cleanup_err) => Err(explain_failure(worktree_path, err, cleanup_err)),
    }
}

/// 数一个目录下还剩多少东西，最多数到 `cap` 就停。
///
/// 只是为了在错误里给个量级。失败的现场常常是个装着 node_modules 的目录，
/// 真数下去是十几万个文件，为一行提示语走那么多 IO 不值得。
fn count_remaining(dir: &std::path::Path, cap: usize) -> usize {
    fn walk(dir: &std::path::Path, cap: usize, n: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            if *n >= cap {
                return;
            }
            *n += 1;
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&e.path(), cap, n);
            }
        }
    }
    let mut n = 0;
    walk(dir, cap, &mut n);
    n
}

/// 连我们自己也删不掉时，把现场说清楚。
///
/// 走到这里说明两件事都失败了：git 删到一半放弃、登记已经解除，我们接手重删也不行。
/// 剩下的只能手工处理，那就把「登记没了、目录成了孤儿、剩多少、分支还在」一次讲全，
/// 光甩一句 `Directory not empty` 用户看不出登记已经消失。
fn explain_failure(
    worktree_path: &str,
    git_err: crate::error::GitError,
    cleanup_err: std::io::Error,
) -> crate::error::GitError {
    let dir = std::path::Path::new(worktree_path);
    let remaining = if dir.exists() {
        let n = count_remaining(dir, 10_000);
        if n >= 10_000 {
            "上万".to_string()
        } else {
            n.to_string()
        }
    } else {
        "0".to_string()
    };

    crate::error::GitError::new(
        "worktree_partial",
        format!(
            "{}\n\ngit 已经解除了这个 worktree 的登记，接手重删也失败了：{cleanup_err}\n\n\
             目录现在不受 git 管理，还剩 {remaining} 项，需要手工删除：\n{worktree_path}\n\n\
             分支和远程分支都还没有动过。目录清掉之后再删一次分支即可。",
            git_err.message
        ),
    )
}

/// 比较两个路径是否指向同一个地方。
///
/// `git worktree list` 给的是正斜杠，界面传回来的可能是反斜杠，直接比字符串
/// 在 Windows 上会误判成「登记已消失」。
fn same_path(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").trim_end_matches('/').to_lowercase();
    norm(a) == norm(b)
}

#[cfg(test)]
mod tests {
    use super::same_path;

    /// `git worktree list` 吐正斜杠，界面传回来的可能是反斜杠。直接比字符串会
    /// 判成「登记已消失」，于是每次失败都被说成孤儿目录。
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
}
