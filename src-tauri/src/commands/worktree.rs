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
    let common = run_git(path, &["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
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
/// 目录被别的进程占着……），而且它**不是原子的**——先删文件、最后删目录，最后一步
/// 失败时前面删掉的东西和管理记录都回不来了。这种时候用户必须看到 git 到底说了什么。
#[tauri::command]
pub fn delete_worktree(path: String, worktree_path: String) -> Result<()> {
    match run_git(&path, &["worktree", "remove", &worktree_path]) {
        Ok(_) => Ok(()),
        Err(err) => Err(explain_failure(&path, &worktree_path, err)),
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

/// 把 `git worktree remove` 的失败补充成「到底走到哪一步了」。
///
/// **这个命令不是原子的**：先解除登记，再递归删目录。第二步在 Windows 上很容易
/// 栽——目录里的 node_modules 满是 junction 链接和超长路径，还常被编辑器或文件
/// 监听占着，于是 git 报一句 `Directory not empty` 就走了，留下一个**不再受 git
/// 管理的孤儿目录**。光把这句原话显示出来，用户完全看不出登记已经没了。
///
/// 所以失败后回头查一次登记还在不在，两种情形分开说：还在就是什么都没动，可以
/// 直接重试；不在就得讲明白目录成了孤儿、要手工删，以及分支和远程都还没动过
/// （删除顺序把 worktree 排在最前，正是为了失败时后面两样保得住）。
fn explain_failure(path: &str, worktree_path: &str, err: crate::error::GitError) -> crate::error::GitError {
    let still_registered = read_worktrees(path)
        .map(|list| list.iter().any(|w| same_path(&w.path, worktree_path)))
        .unwrap_or(true); // 查不出来就别乱下结论，按「什么都没动」说

    if still_registered {
        return err;
    }

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
            "{}\n\ngit 已经解除了这个 worktree 的登记，但目录没能删干净（还剩 {remaining} 项），\
             它现在不受 git 管理了，需要手工删除：\n{worktree_path}\n\n\
             分支和远程分支都还没有动过。目录清掉之后再删一次分支即可。",
            err.message
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
