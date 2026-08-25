use std::path::{Path, PathBuf};

/// 一个仓库需要监听的目录集合：工作区根 + git 元数据目录。
///
/// linked worktree 的 `.git` 是指向主仓库 gitdir 的指针文件，真正的
/// refs / index / HEAD 全在主仓库的 commondir（及其下的 worktrees/<name>）里，
/// 都在工作区根目录之外。只盯工作区根时，外部 commit 在被监听的目录树里
/// 一个文件事件都不产生，`repo-changed` 永远不发，界面停在旧状态。
///
/// 顺序固定为「根目录优先」；已被已含目录覆盖的路径跳过——普通仓库的
/// gitdir 就是 <root>/.git，递归监听根目录时不需要再重复挂。
pub fn watch_targets(root: &Path, repo: Option<&git2::Repository>) -> Vec<PathBuf> {
    let mut targets = vec![root.to_path_buf()];
    if let Some(r) = repo {
        // commondir 在前：linked worktree 的 per-worktree gitdir 位于其下，
        // 会被 starts_with 检查一并跳过
        for dir in [r.commondir(), r.path()] {
            if targets.iter().any(|t| dir.starts_with(t)) {
                continue;
            }
            targets.push(dir.to_path_buf());
        }
    }
    targets
}
