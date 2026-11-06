//! 手动跟进的收尾核对（2026-11-05 事故分析 6.9；分析文档 R19 / W14）。
//!
//! # 为什么「磁盘应该不变」
//!
//! 混合仓库里磁盘就是 SVN 工作副本。SVN 带来的新内容、提交进 git 的改动、没提交的改动，
//! 跟进**之前**都已经在磁盘上了；手动跟进只是让 git 的历史追上磁盘，正确的结果是磁盘一个
//! 字节都不变。它却要先按新 tip 写一遍磁盘（rebase）、再放回改动（stash apply），两步都是
//! 三方合并，合错了不会报冲突——2026-11-05 `MetaLeagueDbsApp.cpp` 就这样被合出两份同名函数，
//! 界面照样报「完成」，而唯一的正确副本（stash 备份）随即被删掉。
//!
//! 例外是用户在 rebase 冲突时亲手解决出来的内容，它可以有意与跟进前不同。所以这里只**报告**，
//! 由调用方决定不报「完成」、留下备份，不回滚任何东西。
//!
//! # 磁盘上本来就是旧内容的那些文件不算
//!
//! 分支落后时（「仅切换」或外部工具切到含旧基线的分支），git 会把**旧基线**的内容写回磁盘，
//! 这时磁盘不是 SVN 的真实内容（`stale_disk`，Review 3 第 2 条）。手动跟进正是为它而存在：
//! rebase 把新基线的内容写回去。所以一个文件「跟进前等于原分支 HEAD（不是你的改动）、跟进后
//! 等于新基线（服务器上的版本）」是被正确挪到了新版本，不报。合错的结果两头都不等，照样报。
//!
//! # 只读
//!
//! 全部是 `git diff` / `ls-tree` / `hash-object`，不写索引、不写磁盘、不联网。

use crate::engine::git_cli::{run_git, run_git_code, run_git_stdin};
use crate::error::{GitError, Result};
use std::collections::BTreeMap;
use std::path::Path;

/// 收尾核对的发现。两项都为空才算「跟进干净」。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drift {
    /// 跟进前后磁盘内容不同的文件：内容变了、被删了、或者多出来了。
    pub changed: Vec<String>,
    /// 重放后的分支里有、新基线里没有、磁盘上也没有的文件。典型来源是 SVN 删掉或改名了它，
    /// 旧的快照提交又把它带了回来（2026-11-05 的 `sim_lowmem.lua`）。
    pub resurrected: Vec<String>,
}

impl Drift {
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.resurrected.is_empty()
    }
}

/// 拿跟进后的磁盘与「跟进前的磁盘」比。
///
/// - `before`：代表跟进前磁盘的提交。有 stash 时是 stash 提交本身（它的树就是当时的工作区，
///   含已跟踪文件的改动与删除）；白名单里什么改动都没有时是原分支 HEAD。
/// - `untracked`：stash 的第三个父提交（`-u` 收走的未跟踪文件）。跟进前它们不是已跟踪文件，
///   跟进后可能变成了（同事新加的文件进了新基线），内容没变就不算。
/// - `orig_head`：原分支 HEAD。跟进前磁盘与它相同的文件不是你的改动（见模块文档）。
/// - `old_base`：分支自己的旧基线（跟进前它落在哪个基线上）。
/// - `new_base`：这次重放到的基线提交。
///
/// 「被带回的文件」是重放后的分支里有、新基线里没有的文件中，满足其一的：
///
/// - 磁盘上没有：「本地改动」里会显示成一条你没做过的删除（11-05 `sim_lowmem.lua` 的形状）
/// - 旧基线里有：SVN 在这段时间里删掉或改名了它，分支却还留着。磁盘本来就是旧内容（分支落后、
///   或者白名单里没有改动不用 stash）时它跟进前后都在磁盘上，只看上一条会漏（独立 review）
///
/// 两条都不满足的是你自己在 git 里新建的文件（旧基线里没有、磁盘上也有），不报。
pub fn audit(
    repo: &str,
    before: &str,
    untracked: Option<&str>,
    orig_head: &str,
    old_base: &str,
    new_base: &str,
) -> Result<Drift> {
    let changed = changed_on_disk(repo, before, untracked, orig_head, new_base)?;
    let added = added_over(repo, new_base, "HEAD")?;
    let old_tree = if added.is_empty() {
        BTreeMap::new()
    } else {
        tree_blobs(repo, old_base)?.regular
    };
    let resurrected: Vec<String> = added
        .into_iter()
        .filter(|p| !changed.contains(p) && (!on_disk(repo, p) || old_tree.contains_key(p)))
        .collect();
    Ok(Drift {
        changed,
        resurrected,
    })
}

/// 跟进前后磁盘内容不同的路径。
///
/// 候选取 `git diff <before>`（提交的树对比**工作区**，路径取提交与索引的并集）：只看得到
/// 已跟踪路径，所以对每个候选再按 blob 精确比一遍——
///
/// - 跟进前：`before` 的树里有就取它，否则取未跟踪部分 `untracked`，都没有就是「不在磁盘上」
/// - 跟进后：磁盘文件按 `git add` 的过滤器算出来的 blob，不存在就是「不在磁盘上」
///
/// 两者相同不算（典型是跟进前未跟踪、跟进后进了新基线的同事新文件）；跟进前等于原分支 HEAD、
/// 跟进后等于新基线的也不算（见模块文档）。
///
/// 符号链接与子模块不判（与 `svn_follow::stale_disk_paths` 同一口径）：它们在树里不是普通
/// 文件的 blob，拿磁盘去 hash 比不出有意义的结果，只会误报。
fn changed_on_disk(
    repo: &str,
    before: &str,
    untracked: Option<&str>,
    orig_head: &str,
    new_base: &str,
) -> Result<Vec<String>> {
    // `--no-renames`：改名会把一删一增合成一条，路径就对不上了
    let out = run_git(
        repo,
        &[
            "diff",
            "--no-renames",
            "--no-ext-diff",
            "--name-only",
            "-z",
            before,
        ],
    )?;
    let candidates: Vec<String> = out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let before_tree = tree_blobs(repo, before)?;
    let untracked_tree = match untracked {
        Some(u) => tree_blobs(repo, u)?,
        None => Tree::default(),
    };
    let head_tree = tree_blobs(repo, orig_head)?;
    let base_tree = tree_blobs(repo, new_base)?;
    let candidates: Vec<String> = candidates
        .into_iter()
        .filter(|p| {
            let special = [&before_tree, &untracked_tree, &head_tree, &base_tree]
                .iter()
                .any(|t| t.special.contains(p));
            !special && !is_symlink(repo, p)
        })
        .collect();
    let disk = hash_on_disk(repo, &candidates)?;
    let (before_tree, untracked_tree, head_tree, base_tree) = (
        before_tree.regular,
        untracked_tree.regular,
        head_tree.regular,
        base_tree.regular,
    );

    let mut changed: Vec<String> = candidates
        .into_iter()
        .filter(|p| {
            let was = before_tree.get(p).or_else(|| untracked_tree.get(p));
            let now = disk.get(p);
            if was == now {
                return false;
            }
            // 跟进前不是你的改动、跟进后是服务器上的版本：被正确地挪到了新基线
            let moved_to_base = was == head_tree.get(p) && now == base_tree.get(p);
            !moved_to_base
        })
        .collect();
    changed.sort();
    changed.dedup();
    Ok(changed)
}

/// `to` 相对 `from` 新增的路径。
fn added_over(repo: &str, from: &str, to: &str) -> Result<Vec<String>> {
    let out = run_git(
        repo,
        &[
            "diff",
            "--no-renames",
            "--name-only",
            "--diff-filter=A",
            "-z",
            from,
            to,
        ],
    )?;
    Ok(out
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// 一棵树（提交）的内容：普通文件（`100644` / `100755`）的 blob，加上符号链接、子模块这类
/// 不按 blob 比的路径。
#[derive(Default)]
struct Tree {
    regular: BTreeMap<String, String>,
    special: std::collections::BTreeSet<String>,
}

fn tree_blobs(repo: &str, tree: &str) -> Result<Tree> {
    let out = run_git(repo, &["ls-tree", "-r", "-z", tree])?;
    let mut t = Tree::default();
    for line in out.split('\0').filter(|l| !l.is_empty()) {
        // `<mode> SP <type> SP <object> TAB <path>`
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let meta: Vec<&str> = meta.split(' ').collect();
        match (meta.first(), meta.get(2)) {
            (Some(&("100644" | "100755")), Some(oid)) => {
                t.regular.insert(path.to_string(), oid.to_string());
            }
            _ => {
                t.special.insert(path.to_string());
            }
        }
    }
    Ok(t)
}

/// 磁盘上这些文件的 blob。`--stdin-paths` 逐个按路径走与 `git add` 相同的过滤器（换行符规则，
/// 2026-11-01 实测 git 2.51，见 `svn_follow::stale_disk_paths`），否则 CRLF 的工作区会和存进去的
/// LF 永远对不上。走 stdin 也就不受命令行长度限制。不是普通文件的（目录、不存在）不出现在结果里。
fn hash_on_disk(repo: &str, paths: &[String]) -> Result<BTreeMap<String, String>> {
    let files: Vec<&String> = paths
        .iter()
        .filter(|p| Path::new(repo).join(p.as_str()).is_file())
        .collect();
    // 空输入不起进程
    if files.is_empty() {
        return Ok(BTreeMap::new());
    }
    let input: String = files.iter().map(|p| format!("{p}\n")).collect();
    let out = run_git_stdin(repo, &["hash-object", "--stdin-paths"], input.as_bytes())?;
    let hashes: Vec<&str> = out.lines().map(str::trim).collect();
    // 行数对不上时 zip 会静默截断，少掉的文件会被当成「删了」报出来
    if hashes.len() != files.len() {
        return Err(GitError::new(
            "svn_follow_audit_hash_failed",
            format!(
                "核对磁盘内容时 git hash-object 的输出行数对不上（{} 个文件、{} 行）。",
                files.len(),
                hashes.len()
            ),
        ));
    }
    Ok(files
        .into_iter()
        .zip(hashes)
        .map(|(p, h)| (p.clone(), h.to_string()))
        .collect())
}

fn on_disk(repo: &str, rel: &str) -> bool {
    Path::new(repo).join(rel).symlink_metadata().is_ok()
}

fn is_symlink(repo: &str, rel: &str) -> bool {
    Path::new(repo)
        .join(rel)
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// stash 的未跟踪部分（第三个父提交）。没有 `-u` 收到东西时 git 不建它。
pub fn stash_untracked(repo: &str, stash: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(
        repo,
        &["rev-parse", "-q", "--verify", &format!("{stash}^3")],
    )?;
    Ok((code == 0).then(|| out.trim().to_string()))
}
