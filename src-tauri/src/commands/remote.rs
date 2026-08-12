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
    // 默认不带参数，尊重用户自己的 pull.rebase 配置——
    // 这是 git 客户端最不意外的行为
    if rebase {
        run_git(&path, &["pull", "--rebase"])?;
    } else {
        run_git(&path, &["pull"])?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn push(path: String, force: bool) -> Result<()> {
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
