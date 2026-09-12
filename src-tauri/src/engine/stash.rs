use crate::engine::git_cli::run_git;
use crate::error::Result;
use git2::Repository;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StashEntry {
    /// stash@{N} 里的 N。0 是最新的一条。
    pub index: usize,
    pub message: String,
    /// stash 本身就是一个提交，前端拿它复用现成的 `commit:<hash>` diff scope。
    pub hash: String,
    pub author: String,
    /// 提交时间，Unix 秒。
    pub time: i64,
    /// 父提交。stash 是个 merge 提交，第一父是 stash 时所在的那个提交。
    pub parents: Vec<String>,
}

/// 列出所有 stash，最新的在前。
///
/// 除了索引和信息，还要带上作者/时间/父提交：stash 提交不被任何分支引用，
/// 不会出现在 `load_graph` 的输出里，详情面板只能靠这里给的元数据渲染。
pub fn list(path: &str) -> Result<Vec<StashEntry>> {
    // stash_foreach 需要可变借用，所以先把 oid 收集出来，再逐个查提交
    let mut repo = Repository::open(path)?;
    let mut raw = Vec::new();
    repo.stash_foreach(|index, message, oid| {
        raw.push((index, message.to_string(), *oid));
        true
    })?;

    let mut entries = Vec::with_capacity(raw.len());
    for (index, message, oid) in raw {
        let commit = repo.find_commit(oid)?;
        entries.push(StashEntry {
            index,
            message,
            hash: oid.to_string(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            parents: commit.parent_ids().map(|p| p.to_string()).collect(),
        });
    }
    Ok(entries)
}

/// 把当前改动压进 stash。
///
/// 写入走 git CLI 而不是 git2：`Repository::stash_save` 的语义与 CLI 的
/// `stash push` 在未跟踪文件、pathspec 上有出入，而界面上那个「按住 Ctrl」
/// 的变体对应的正是 CLI 的 `-u`。
pub fn push(path: &str, message: Option<&str>, include_untracked: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["stash", "push"];
    // include_untracked 对应界面上按住 Ctrl 的变体
    if include_untracked {
        args.push("-u");
    }
    if let Some(msg) = message {
        args.push("-m");
        args.push(msg);
    }
    run_git(path, &args)?;
    Ok(())
}

/// 应用第 index 条 stash，保留该条目。
pub fn apply(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "apply", &format!("stash@{{{index}}}")])?;
    Ok(())
}

/// 应用第 index 条 stash 并删除它。
pub fn pop(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "pop", &format!("stash@{{{index}}}")])?;
    Ok(())
}

/// 删除第 index 条 stash，不应用。
///
/// 函数名 `drop_entry` 而不是 `drop`：后者与 `Drop::drop` 同名，clippy 的
/// `should_implement_trait` 会报，而 `AGENTS.md` §7.3 禁止用 `#[allow]` 掩盖。
pub fn drop_entry(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "drop", &format!("stash@{{{index}}}")])?;
    Ok(())
}
