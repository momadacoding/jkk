use crate::engine::git_cli::run_git;
use crate::engine::svn_sync_stash::{untracked_clashes, with_same_files_lifted};
use crate::error::{GitError, Result};
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
/// `stash push` 在未跟踪文件、pathspec 上有出入，而工具栏 Stash 要的正是 CLI 的 `-u`。
pub fn push(path: &str, message: Option<&str>, include_untracked: bool) -> Result<()> {
    let mut args: Vec<&str> = vec!["stash", "push"];
    // 工具栏的普通 Stash 带 `-u`，按住 Ctrl 的「Stash tracked」不带（第 99 号 issue）
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
    guard_untracked(path, index, || {
        run_git(path, &["stash", "apply", &format!("stash@{{{index}}}")]).map(|_| ())
    })
}

/// 应用第 index 条 stash 并删除它。
pub fn pop(path: &str, index: usize) -> Result<()> {
    guard_untracked(path, index, || {
        run_git(path, &["stash", "pop", &format!("stash@{{{index}}}")]).map(|_| ())
    })
}

/// 报错里最多列几个撞名的路径，多出来的只给个数。
const MAX_LISTED_COLLISIONS: usize = 5;

/// stash 里的新增文件在磁盘上已经有同名的了，就**在 apply 之前**拦下，一个字节都不动。
///
/// 不能交给 git 自己报错：撞名时它先把已跟踪的改动写上去，再在未跟踪的文件上失败、退出 1、
/// 条目留着（2026-11-16 实测 git 2.51，与 `svn_sync_stash` 2026-09-25 的记录一致）。半套 apply 之后
/// 移走同名文件再 pop 一次，又会撞上自己刚写下的已跟踪改动（`Your local changes ... would be
/// overwritten`）。工具栏 Stash 默认带 `-u` 之后这种 stash 成了常态，所以每次 apply / pop 都先查。
///
/// 撞名判定与同步流程共用（`svn_sync_stash::untracked_clashes`）：stash、HEAD、磁盘三处是同一个
/// blob 的不算撞名（混合仓库里同事新增的文件经跟进回到磁盘，计划 A17），先拿掉再让 apply 写回。
///
/// 检查与执行之间别的进程又压了一条 stash 时，`stash@{index}` 会指向另一条：git 的 pop 只认
/// reflog 里的条目，不能改用 OID 钉死。窗口只有一次 git 调用那么长，且 git 自己撞名时照样报错。
fn guard_untracked(path: &str, index: usize, run: impl FnOnce() -> Result<()>) -> Result<()> {
    let Some(untracked) = untracked_parent(path, index)? else {
        return run();
    };
    let clashes = untracked_clashes(path, &untracked)?;
    if clashes.collisions.is_empty() {
        return with_same_files_lifted(path, &clashes.same_file, run);
    }
    let collisions = clashes.collisions;
    let mut listed = collisions
        .iter()
        .take(MAX_LISTED_COLLISIONS)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if collisions.len() > MAX_LISTED_COLLISIONS {
        listed.push_str(&format!(" 等 {} 个", collisions.len()));
    }
    Err(GitError::new(
        "stash_untracked_exists",
        format!(
            "这条 stash 里的新增文件，工作区里已经有同名的了：{listed}。\
             git 碰到这种情况会只恢复一半，所以这次什么都没动，stash 也还在。\
             把这些文件改名、移走或删掉之后再试一次。"
        ),
    ))
}

/// 第 index 条 stash 的未跟踪树（第三个父提交）的 OID。只有带 `-u` 的 stash 才有，没有就是 `None`。
fn untracked_parent(path: &str, index: usize) -> Result<Option<String>> {
    let repo = Repository::open(path)?;
    let stash = repo
        .revparse_single(&format!("stash@{{{index}}}"))?
        .peel_to_commit()?;
    if stash.parent_count() < 3 {
        return Ok(None);
    }
    Ok(Some(stash.parent_id(2)?.to_string()))
}

/// 删除第 index 条 stash，不应用。
///
/// 函数名 `drop_entry` 而不是 `drop`：后者与 `Drop::drop` 同名，clippy 的
/// `should_implement_trait` 会报，而 `AGENTS.md` §7.3 禁止用 `#[allow]` 掩盖。
pub fn drop_entry(path: &str, index: usize) -> Result<()> {
    run_git(path, &["stash", "drop", &format!("stash@{{{index}}}")])?;
    Ok(())
}
