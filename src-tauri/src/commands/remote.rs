use crate::engine::git_cli::run_git;
use crate::error::Result;

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
