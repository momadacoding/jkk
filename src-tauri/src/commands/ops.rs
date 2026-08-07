use crate::engine::git_cli::run_git;
use crate::error::Result;
use std::collections::HashSet;

/// 跑一条会输出 NUL 分隔路径列表的 git 命令，收成集合。
///
/// 命令失败当作空集：`ls-tree HEAD` 在还没有任何提交的仓库里就会失败，
/// 那正是「HEAD 里什么都没有」，不是错误。
fn path_set(path: &str, args: &[&str]) -> HashSet<String> {
    match run_git(path, args) {
        Ok(out) => out.split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect(),
        Err(_) => HashSet::new(),
    }
}

/// 丢弃改动。
///
/// 三类文件走三条完全不同的路，混在一次多选里是常态：
///
/// - **未跟踪**：git 里没有任何副本，只能直接删文件。这一步不可逆，界面必须先确认。
/// - **HEAD 里没有、但已进暂存区**（新加的文件）：`git rm -f` 同时从索引和磁盘去掉。
/// - **其余**：`git checkout` 恢复。
///
/// `staged` 决定恢复到哪一层：Unstaged 区的 Discard 只回退「工作区相对暂存区」的
/// 部分（`git checkout --`），把已暂存的改动一起抹掉是 Staged 区的语义
/// （`git checkout HEAD --`）。
#[tauri::command]
pub fn discard(path: String, paths: Vec<String>, staged: bool) -> Result<()> {
    // 空 pathspec 会被 git 当成「全部」，那就成了一次误伤整个仓库的 discard
    if paths.is_empty() {
        return Ok(());
    }

    let mut probe: Vec<&str> = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    probe.extend(paths.iter().map(String::as_str));
    let untracked = path_set(&path, &probe);

    let mut head_probe: Vec<&str> = vec!["ls-tree", "-r", "HEAD", "--name-only", "-z", "--"];
    head_probe.extend(paths.iter().map(String::as_str));
    let in_head = path_set(&path, &head_probe);

    let mut restore: Vec<&str> = Vec::new();
    let mut remove: Vec<&str> = Vec::new();
    let mut unlink: Vec<&str> = Vec::new();
    for p in &paths {
        if untracked.contains(p) {
            unlink.push(p);
        } else if staged && !in_head.contains(p) {
            remove.push(p);
        } else {
            restore.push(p);
        }
    }

    if !restore.is_empty() {
        let mut args: Vec<&str> = vec!["checkout"];
        if staged {
            args.push("HEAD");
        }
        args.push("--");
        args.extend(restore);
        run_git(&path, &args)?;
    }

    if !remove.is_empty() {
        let mut args: Vec<&str> = vec!["rm", "-f", "--"];
        args.extend(remove);
        run_git(&path, &args)?;
    }

    // 直接删而不是 `git clean -f`：pathspec 稍有偏差就会波及没被选中的文件，
    // 而这一步不可逆
    let root = std::path::Path::new(&path);
    for rel in unlink {
        let target = root.join(rel);
        let result = if target.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        // 已经不在了就当成功——用户要的就是它消失
        if let Err(err) = result {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(err.into());
            }
        }
    }

    Ok(())
}

#[tauri::command]
pub fn create_branch(path: String, name: String, start_point: Option<String>) -> Result<()> {
    let mut args = vec!["branch", &name];
    if let Some(start) = &start_point {
        args.push(start);
    }
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn delete_branch(path: String, name: String) -> Result<()> {
    run_git(&path, &["branch", "-D", &name])?;
    Ok(())
}

/// 删除分支前的确认框要显示的信息：对应的远程分支、还有多少提交没合并进来。
#[tauri::command]
pub fn branch_delete_info(
    path: String,
    name: String,
) -> Result<crate::engine::branch::DeleteInfo> {
    crate::engine::branch::delete_info(&path, &name)
}

#[tauri::command]
pub fn delete_remote_branch(path: String, remote: String, branch: String) -> Result<()> {
    run_git(&path, &["push", &remote, "--delete", &branch])?;
    Ok(())
}

/// 按 `origin/feat/x` 这种完整简称删除远程分支。
///
/// 侧边栏的远程分支那一栏拿到的就是这个形式，拆分放在后端做——远程名可以带
/// 斜杠，前端按第一个 `/` 去切会切错。
#[tauri::command]
pub fn delete_remote_ref(path: String, remote_branch: String) -> Result<()> {
    let (remote, branch) = crate::engine::branch::split_remote_ref(&path, &remote_branch)?;
    delete_remote_branch(path, remote, branch)
}

/// 检出一个远程分支，返回落地的本地分支名。
///
/// **不能直接 `git checkout origin/x`**——那会进游离 HEAD，界面上看着像切过去了，
/// 实际上此后的提交不属于任何分支。要建一条跟踪它的本地分支。
///
/// 同名本地分支已经存在时直接切过去，不去动它指向哪：那多半就是用户上次检出
/// 留下的分支，静默把它重置到远程会丢掉本地提交。
#[tauri::command]
pub fn checkout_remote_branch(path: String, remote_branch: String) -> Result<String> {
    let (local, exists) = crate::engine::branch::local_target_for(&path, &remote_branch)?;
    if exists {
        run_git(&path, &["checkout", &local])?;
    } else {
        run_git(&path, &["checkout", "-b", &local, "--track", &remote_branch])?;
    }
    Ok(local)
}

#[tauri::command]
pub fn rename_branch(path: String, old: String, new: String) -> Result<()> {
    run_git(&path, &["branch", "-m", &old, &new])?;
    Ok(())
}

#[tauri::command]
pub fn checkout(path: String, rev: String) -> Result<()> {
    run_git(&path, &["checkout", &rev])?;
    Ok(())
}

#[tauri::command]
pub fn stage(path: String, paths: Vec<String>) -> Result<()> {
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut args: Vec<&str> = vec!["add"];
    args.extend(path_refs);
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn unstage(path: String, paths: Vec<String>) -> Result<()> {
    let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
    let mut args: Vec<&str> = vec!["reset", "HEAD"];
    args.extend(path_refs);
    run_git(&path, &args)?;
    Ok(())
}

#[tauri::command]
pub fn commit(path: String, message: String, amend: bool) -> Result<()> {
    if amend {
        run_git(&path, &["commit", "--amend", "-m", &message])?;
    } else {
        run_git(&path, &["commit", "-m", &message])?;
    }
    Ok(())
}
