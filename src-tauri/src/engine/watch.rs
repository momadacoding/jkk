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
///
/// 返回的是**调用方给的原始写法**，只有包含判断走规范化形式，见 [`canonical`]。
pub fn watch_targets(root: &Path, repo: Option<&git2::Repository>) -> Vec<PathBuf> {
    let mut targets = vec![root.to_path_buf()];
    // 判断包含关系时**不能**直接拿这两边的路径比：`root` 是调用方给的原始写法，
    // 而 commondir()/path() 是 libgit2 解析过的。两者只要写法不同，
    // `starts_with` 就失配，.git/ 被当成独立目标多挂一次——而根目录本来就是
    // 递归监听的，结果是每次 git 元数据变更触发两遍刷新。
    //
    // 真实案例：CI 上 TempDir 落在 C:\Users\RUNNER~1\...（8.3 短名），
    // commondir() 却返回 C:/Users/runneradmin/...，v0.4.0 的发版构建栽在这里。
    // 同样的失配也会由大小写、`..` 段、正反斜杠造成。
    let mut probes = vec![canonical(root)];
    if let Some(r) = repo {
        // commondir 在前：linked worktree 的 per-worktree gitdir 位于其下，
        // 会被下面的包含检查一并跳过
        for dir in [r.commondir(), r.path()] {
            let probe = canonical(dir);
            if probes.iter().any(|t| probe.starts_with(t)) {
                continue;
            }
            targets.push(dir.to_path_buf());
            probes.push(probe);
        }
    }
    targets
}

/// 仅用于比较的规范化形式。
///
/// `fs::canonicalize` 是这里唯一能解开 8.3 短名的手段——大小写折叠、分隔符
/// 替换都做不到 `RUNNER~1` → `runneradmin`。Windows 上它返回 `\\?\C:\...`
/// 前缀，两边都过一遍就仍然可比。
///
/// 解析失败（路径已被删、权限不足）时退回原值：那种情况下多挂一次监听远好过
/// 少挂——少挂意味着界面收不到刷新事件。
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
