use crate::engine::git_cli::run_git;
use crate::error::{GitError, Result};

/// 当前分支名。push 需要它来补 -u。
fn current_branch(path: &str) -> Result<String> {
    Ok(run_git(path, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string())
}

/// 该分支是否已经设过上游。
fn has_upstream(path: &str) -> bool {
    run_git(path, &["rev-parse", "--abbrev-ref", "@{upstream}"]).is_ok()
}

#[tauri::command(async)]
pub fn fetch(path: String, all: bool) -> Result<()> {
    // 本文件的命令都不走 trait Vcs：SVN 混合仓库由这一行拦（第 69 条）
    crate::vcs::require_capability(&path, "git 远程", |c| c.remotes)?;
    // all 对应界面上按住 Ctrl 的变体
    if all {
        run_git(&path, &["fetch", "--all", "--prune", "--tags"])?;
    } else {
        run_git(&path, &["fetch", "--prune"])?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn pull(path: String, rebase: bool) -> Result<()> {
    crate::vcs::require_capability(&path, "git 远程", |c| c.remotes)?;
    // 默认不带参数，尊重用户自己的 pull.rebase 配置——
    // 这是 git 客户端最不意外的行为
    let args: &[&str] = if rebase {
        &["pull", "--rebase"]
    } else {
        &["pull"]
    };
    let Some((branch, before)) = head_position(&path) else {
        return run_git(&path, args).map(|_| ());
    };
    let err = match run_git(&path, args) {
        Ok(_) => return Ok(()),
        Err(e) => e,
    };
    let Some(after) = moved_by_someone_else(&path, &branch, &before) else {
        return Err(err);
    };

    // 走到这里：pull 失败了，当前分支却在这期间被另一个进程（另一个 git 工具、另一个
    // 会话）挪到了 `after`。git 以为是自己的 fetch 改了分支，拿旧提交去快进工作区，
    // 而工作区早被那个进程更新好了，于是新提交的内容全被当成「本地改动」拒绝，
    // 还建议 `git reset --hard`。这次快进被 git 整体中止，工作区一个文件都没动。
    //
    // 两次 HEAD 之间变了的路径在工作区里都已是新内容（那个进程做的是正常的 merge /
    // checkout）时，从现在的状态再拉一次就是用户要的那次拉取。没跟上（别的工具只挪了
    // ref）时绝不能重拉：重拉会因为「已是最新」报成功，把反向改动留在工作区里。
    if changed_paths_caught_up(&path, &before, &after) {
        run_git(&path, args)?;
        return Ok(());
    }
    Err(GitError::new(
        "pull_head_moved",
        format!(
            "拉取期间当前分支被另一个程序移动了，工作区没有跟上新的位置，已停止拉取，\
             没有改动工作区里的文件。请先确认工作区里的改动是不是你自己的，再决定怎么处理。\n\n\
             git 原始输出：\n{}",
            err.message
        ),
    ))
}

/// HEAD 停在哪：(所在分支的全名, 指向的提交)。detached 或还没有提交时为 `None`。
fn head_position(path: &str) -> Option<(String, String)> {
    let branch = run_git(path, &["symbolic-ref", "-q", "HEAD"]).ok()?;
    let oid = run_git(path, &["rev-parse", "--verify", "-q", "HEAD"]).ok()?;
    Some((branch.trim().to_string(), oid.trim().to_string()))
}

/// pull 失败后，当前分支是不是被**别的进程**从 `before` 挪走了；是的话返回新位置。
///
/// 不看 git 的报错文案（git 装了语言包会说别的话），看事实：
/// - 还在同一条分支上，提交变了。分支被切走时不重拉——重拉会去拉另一条分支的上游
/// - 没停在 merge / rebase 等操作中途。rebase 撞冲突时 HEAD 也会挪，那是 pull 自己的失败
/// - 这期间分支 reflog 里新增的条目没有一条是 `pull` 开头的。pull 自己快进完又在后面
///   失败（比如 `submodule.recurse` 下子模块检出被挡）时，挪分支的正是它自己。
///   另一个进程恰好也是 `git pull` 时同样会被排除——那时退回原始报错，宁可不认
fn moved_by_someone_else(path: &str, branch: &str, before: &str) -> Option<String> {
    let (now_branch, after) = head_position(path)?;
    if now_branch != branch || after == before {
        return None;
    }
    if !matches!(crate::engine::repo::state(path), Ok("normal")) {
        return None;
    }
    let repo = git2::Repository::open(path).ok()?;
    let log = repo.reflog(branch).ok()?;
    let before_oid = git2::Oid::from_str(before).ok()?;
    for entry in log.iter() {
        if entry.id_new() == before_oid {
            return Some(after);
        }
        // pull 写 reflog 一定带消息；没有消息的条目（裸 `update-ref`）只可能是别人写的
        // 按字节比：消息不是 UTF-8 时 `message()` 返回 None，会被误当成「没有消息」
        if entry
            .message_bytes()
            .is_some_and(|m| m.starts_with(b"pull"))
        {
            return None;
        }
    }
    // reflog 里找不到起点（被关掉了或被截断），无从判断是谁挪的
    None
}

/// `before` 与 `after` 之间变了的路径，在暂存区和工作区里都已与 `after` 一致。
///
/// 只看这些路径：用户手里和这次更新无关的改动不该挡住重拉。未跟踪文件不算——
/// 它们只可能是新提交删掉的文件留下的，重拉时 git 自己会拒绝覆盖未跟踪文件。
/// 不把路径当 pathspec 传给 git：路径多了会撞 Windows 命令行长度上限，空列表又会被
/// git 当成「全部」（`AGENTS.md` §6.2），在这里按集合过滤更稳。
fn changed_paths_caught_up(path: &str, before: &str, after: &str) -> bool {
    // `--no-relative`：用户配了 `diff.relative` 时路径基准要和 status 的「相对仓库根」对齐
    let Ok(changed) = run_git(
        path,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-relative",
            "--no-renames",
            before,
            after,
        ],
    ) else {
        return false;
    };
    let changed: std::collections::HashSet<&str> =
        changed.split('\0').filter(|p| !p.is_empty()).collect();
    if changed.is_empty() {
        return true;
    }
    let Ok(status) = run_git(
        path,
        &[
            "status",
            "--porcelain",
            "-z",
            "--untracked-files=no",
            "--no-renames",
        ],
    ) else {
        return false;
    };
    // 每条是 `XY <路径>`，-z 下不加引号；关掉改名检测后一条只有一个路径
    !status
        .split('\0')
        .filter_map(|entry| entry.get(3..))
        .any(|p| changed.contains(p))
}

#[tauri::command(async)]
pub fn push(path: String, force: bool) -> Result<()> {
    crate::vcs::require_capability(&path, "git 远程", |c| c.remotes)?;
    let branch = current_branch(&path)?;
    // 新分支没有上游，裸 push 会直接失败，自动补 -u 省掉一次无谓的报错
    let set_upstream = !has_upstream(&path);

    let mut args: Vec<&str> = vec!["push"];
    // --force-with-lease 而不是 --force：远程有别人的新提交时会拒绝，
    // 不会把同事的工作覆盖掉
    if force {
        args.push("--force-with-lease");
    }
    if set_upstream {
        args.push("-u");
        args.push("origin");
        args.push(&branch);
    }
    run_git(&path, &args)?;
    Ok(())
}

/// 推 tag 用哪个远端。
///
/// tag 不属于任何分支，没有「上游」可问，只能从仓库配置的远端里挑。
/// 硬写 origin 会让远端叫别的名字（`upstream`、`github`）的仓库永远推不出去，
/// 而且界面上没有任何地方能改。
fn tag_remote(path: &str) -> Result<String> {
    let listed = run_git(path, &["remote"])?;
    let remotes: Vec<&str> = listed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    // 有 origin 就用 origin：绝大多数仓库的约定，多远端时也是最不意外的选择
    if remotes.contains(&"origin") {
        return Ok("origin".to_string());
    }
    match remotes.as_slice() {
        [] => Err(GitError::new(
            "no_remote",
            "this repository has no remote configured",
        )),
        [only] => Ok((*only).to_string()),
        // 多个远端又没有 origin，猜哪个都可能推到错的地方，不如说清楚
        _ => Err(GitError::new(
            "ambiguous_remote",
            format!(
                "multiple remotes and none named origin: {}",
                remotes.join(", ")
            ),
        )),
    }
}

/// 把一条本地 tag 推到远端。
#[tauri::command(async)]
pub fn push_tag(path: String, name: String) -> Result<()> {
    crate::vcs::require_capability(&path, "tag", |c| c.tags)?;
    // 空名必须在这里就挡掉。拼进 refspec 会变成 `refs/tags/:refs/tags/`——
    // 冒号左边为空正是 git 删除远端引用的写法，一个空串就能把远端引用删掉。
    let name = name.trim();
    if name.is_empty() {
        return Err(GitError::new("bad_arg", "tag name is empty"));
    }

    // 用全限定的双向 refspec，不用短名。同时存在分支 v1 和 tag v1 时
    // `git push origin v1` 是有歧义的，可能推错东西。
    //
    // 不带 --force：静默改写别人已经拉走的 tag 是不可接受的，
    // 和 create_tag 不带 -f 是同一个理由。
    let refspec = format!("refs/tags/{name}:refs/tags/{name}");
    let remote = tag_remote(&path)?;
    run_git(&path, &["push", &remote, &refspec])?;
    Ok(())
}
