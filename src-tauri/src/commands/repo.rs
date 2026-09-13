use crate::engine;
use crate::vcs::backend_for;

#[tauri::command(async)]
pub fn open_repo(path: String) -> Result<crate::engine::repo::RepoInfo, crate::error::GitError> {
    backend_for(&path)?.open()
}

#[tauri::command(async)]
pub fn read_upstream(
    path: String,
) -> Result<crate::engine::upstream::UpstreamInfo, crate::error::GitError> {
    engine::upstream::read(&path)
}

#[tauri::command(async)]
pub fn repo_state(path: String) -> Result<String, crate::error::GitError> {
    let git_dir = std::path::Path::new(&path).join(".git");
    if git_dir.join("MERGE_HEAD").exists() {
        return Ok("merge".to_string());
    }
    if git_dir.join("rebase-apply").exists() || git_dir.join("rebase-merge").exists() {
        return Ok("rebase".to_string());
    }
    if git_dir.join("CHERRY_PICK_HEAD").exists() {
        return Ok("cherry-pick".to_string());
    }
    Ok("normal".to_string())
}

/// 读取当前仓库的能力集合，前端据此决定渲染哪些入口。
#[tauri::command(async)]
pub fn read_capabilities(path: String) -> Result<crate::vcs::Capabilities, crate::error::GitError> {
    Ok(backend_for(&path)?.capabilities())
}

/// 探测 svn 是否可用。返回 `"available"` / `"stale_path"` / `"not_installed"`。
///
/// 后两者必须分开：安装器写的是**持久** PATH，已经在跑的进程持有启动时的环境
/// 快照，于是「装完之后应用仍然看不见」是一个真实且常见的状态，正确引导是
/// **重启应用**而不是重装（spec §17.1）。
#[tauri::command(async)]
pub fn probe_svn_environment() -> Result<String, crate::error::GitError> {
    use crate::engine::svn_cli::{probe_svn, SvnAvailability};
    Ok(match probe_svn() {
        SvnAvailability::Available { .. } => "available",
        SvnAvailability::StalePath => "stale_path",
        SvnAvailability::NotInstalled => "not_installed",
    }
    .to_string())
}
