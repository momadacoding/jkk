use crate::engine::git_cli::run_git_bytes;
use crate::error::Result;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub name: String,
    pub path: String,
    pub head: String,
    pub branch: String,
    pub is_main: bool,
}

/// 走 `run_git_bytes` 而不是自己拼 `Command`：这样才能一并拿到 `GIT_TERMINAL_PROMPT=0`
/// 和 `CREATE_NO_WINDOW`。之前这里是全仓唯一绕开 `git_cli` 的 git 调用，装机版上
/// 每次刷新 worktree 列表都会闪一个 console 黑框。
pub fn read_worktrees(path: &str) -> Result<Vec<Worktree>> {
    let stdout = run_git_bytes(path, &["worktree", "list", "--porcelain", "-z"])?;
    Ok(parse_worktrees(&String::from_utf8_lossy(&stdout)))
}

/// 解析 `git worktree list --porcelain -z` 的输出。
///
/// `-z` 的分隔规则是：每个属性行以 `\0` 结尾，块与块之间再多一个 `\0`。
/// 因此要先按 `\0\0` 切块，块内再按 `\0` 切行。
/// 主工作区固定是第一个块（git 的输出顺序保证）。
fn parse_worktrees(stdout: &str) -> Vec<Worktree> {
    let mut worktrees = Vec::new();
    for block in stdout.split("\0\0") {
        let mut name = String::new();
        let mut path_str = String::new();
        let mut head = String::new();
        let mut branch = String::new();
        for line in block.split('\0') {
            let line = line.trim_end_matches('\n');
            if let Some(p) = line.strip_prefix("worktree ") {
                path_str = p.to_string();
                name = std::path::Path::new(p)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
            } else if let Some(h) = line.strip_prefix("HEAD ") {
                head = h.to_string();
            } else if let Some(b) = line.strip_prefix("branch ") {
                branch = b.replace("refs/heads/", "").to_string();
            } else if line == "detached" {
                branch = "detached".to_string();
            }
        }
        if !path_str.is_empty() {
            worktrees.push(Worktree {
                name,
                path: path_str,
                head,
                branch,
                is_main: worktrees.is_empty(),
            });
        }
    }
    worktrees
}

#[cfg(test)]
mod tests {
    use super::parse_worktrees;

    /// 真实的 `-z` 输出：每个属性行以 \0 结尾，块与块之间再多一个 \0。
    const RAW: &str = "worktree C:/repo\0HEAD abc123\0branch refs/heads/master\0\
                       \0worktree C:/repo-wt\0HEAD def456\0branch refs/heads/feat\0\0";

    #[test]
    fn parses_nul_framed_blocks() {
        let wts = parse_worktrees(RAW);
        assert_eq!(wts.len(), 2);
        assert_eq!(wts[0].path, "C:/repo");
        assert_eq!(wts[0].name, "repo");
        assert_eq!(wts[1].name, "repo-wt");
    }

    /// 曾经按单个 \0 分块，导致每个片段只含一个属性，head/branch 恒为空串。
    #[test]
    fn keeps_head_and_branch_with_their_worktree() {
        let wts = parse_worktrees(RAW);
        assert_eq!(wts[0].head, "abc123");
        assert_eq!(wts[0].branch, "master");
        assert_eq!(wts[1].head, "def456");
        assert_eq!(wts[1].branch, "feat");
    }

    /// git 的输出顺序保证主工作区排第一，不靠路径字符串比较判断。
    #[test]
    fn first_block_is_the_main_worktree() {
        let wts = parse_worktrees(RAW);
        assert!(wts[0].is_main);
        assert!(!wts[1].is_main);
    }

    #[test]
    fn marks_detached_head() {
        let wts = parse_worktrees("worktree C:/repo\0HEAD abc123\0detached\0\0");
        assert_eq!(wts.len(), 1);
        assert_eq!(wts[0].branch, "detached");
    }

    #[test]
    fn ignores_trailing_empty_block() {
        assert_eq!(parse_worktrees("").len(), 0);
        assert_eq!(parse_worktrees("worktree C:/repo\0HEAD abc\0\0").len(), 1);
    }
}
