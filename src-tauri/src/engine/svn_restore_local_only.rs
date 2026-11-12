//! 跟进后把 Local-Only 补丁放回磁盘（使用报告 9.1；计划 `2026-11-12-restore-local-only-patches.md`）。
//!
//! # 问题
//!
//! 普通 commit 在 git 里删掉了 Local-Only commit 加的一行，这个文件随 TortoiseSVN 提交上了
//! SVN。跟进时普通 commit 改过的路径换成 SVN 版本，它变空被丢；Local-Only commit 照常重放，
//! 把这一行加回分支。按「Local-Only 永远留在本地」的语义分支是对的，缺的是磁盘：报告里列在
//! 「分支与磁盘对不上」，用户只能自己去「本地改动」里丢弃这个文件。
//!
//! # 做法
//!
//! 跟进不写磁盘（`svn_follow` 模块头的不变量），所以分两步：
//!
//! - [`restorable_paths`]：跟进时只读地挑出能放回的文件，记进报告；
//! - [`restore_local_only`]：用户点按钮时持同步锁、**每条条件现场重验**，再用
//!   `git checkout-index` 把分支版本写回磁盘。
//!
//! # 为什么安全
//!
//! 一个文件同时满足下面几条才写（[`check_paths`]）：
//!
//! 1. 磁盘按 `git add` 的过滤规则算出的 hash 等于 `svn/base` 里的版本：磁盘就是 SVN 的内容，
//!    没有用户自己的编辑；
//! 2. `svn/base..HEAD` 里改过它的 commit 全是 Local-Only：分支比 SVN 多出来的只有本地补丁；
//! 3. （写的时候）索引里只有 stage 0、且等于 HEAD：用户没暂存别的版本，写完磁盘 = 索引 = HEAD。
//!
//! 写回去的内容在 git 里，原来的内容在 SVN 里（`svn revert` 就能拿回来），两边都丢不了东西。

use crate::engine::git_cli::run_git_stdin;
use crate::engine::graph::parse_local_only;
use crate::engine::svn_baseline::git_dir;
use crate::engine::svn_config::BASELINE_BRANCH;
use crate::engine::svn_journal;
use crate::engine::svn_lock;
use crate::engine::svn_sync_check::{check_git_idle, current_branch, rev_parse};
use crate::error::{GitError, Result};
use git2::{Commit, ObjectType, Oid, Repository, Tree};
use std::path::Path;

/// [`restore_local_only`] 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    /// 已写回磁盘的文件，按请求的顺序。
    pub restored: Vec<String>,
    /// 没写的文件和原因。
    pub skipped: Vec<RestoreSkip>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSkip {
    pub path: String,
    /// 给人看的原因。
    pub reason: String,
}

/// 跟进时用：`candidates`（报告的 `drift`）里哪些能放回。`base` 是新基线，`head` 是重放后的
/// tip，两者都还没发布成 ref，所以按 OID 给。只读；不看索引（发布时才拼出新索引），按钮按下时
/// [`restore_local_only`] 会再查。
pub fn restorable_paths(
    repo: &str,
    base: &str,
    head: &str,
    candidates: &[String],
) -> Result<Vec<String>> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let (ok, _) = check_paths(repo, base, head, candidates, false)?;
    Ok(ok)
}

/// 按钮：把 `paths` 里满足条件的文件写回成分支（HEAD）的版本。
///
/// 持同步锁；有没做完的 SVN 同步、git 正忙、索引被占用、HEAD 不在工作分支上时报错，什么都不写。
/// 单个文件不满足条件只是跳过，写进 [`RestoreOutcome::skipped`]。
pub fn restore_local_only(repo: &str, opid: &str, paths: &[String]) -> Result<RestoreOutcome> {
    if paths.is_empty() {
        return Ok(RestoreOutcome::default());
    }
    let _lock = svn_lock::acquire(repo, opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(GitError::new(
            "svn_restore_unfinished_sync",
            "有一次 SVN 同步还没做完。请先在顶部横幅上处理，再放回 Local-Only 补丁。",
        ));
    }
    check_git_idle(repo, "svn_restore_git_busy", "放回 Local-Only 补丁")?;
    let lock = git_dir(repo)?.join("index.lock");
    if lock.exists() {
        return Err(GitError::new(
            "svn_index_busy",
            format!(
                "另一个 git 进程正在占用这个仓库的索引（{}）。等它结束再放回；如果确认没有 git 在跑，\
                 那是上一次崩溃留下的残锁，手工删掉该文件即可。",
                lock.display()
            ),
        ));
    }
    match current_branch(repo)? {
        Some(b) if b != BASELINE_BRANCH => {}
        _ => {
            return Err(GitError::new(
                "svn_restore_not_on_branch",
                "HEAD 不在工作分支上。请先切回跟进过的那条分支。",
            ))
        }
    }
    let base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    let head = rev_parse(repo, "HEAD")?;
    let (ok, mut skipped) = check_paths(repo, &base, &head, paths, true)?;
    if ok.is_empty() {
        return Ok(RestoreOutcome {
            restored: Vec::new(),
            skipped,
        });
    }
    // `--stdin -z`：路径不经命令行，空格、中文、上万个文件都不用拆参。`ok` 非空，不会出现
    // 「空列表 = 全部」（AGENTS.md §6.2）；`checkout-index` 本来也不把空输入当成全部
    let input: Vec<u8> = ok.iter().flat_map(|p| p.bytes().chain([0])).collect();
    run_git_stdin(repo, &["checkout-index", "-f", "-z", "--stdin"], &input)?;
    // 写完核对一遍：过滤规则（eol / autocrlf）不对称时磁盘可能不等于分支。核对本身失败时
    // 已经写过了，不能报「什么都没写」的错，按写成了算
    let after = match disk_hashes(repo, &ok) {
        Ok(h) => h,
        Err(e) => {
            log::warn!(
                "放回 Local-Only 补丁后核对磁盘出错，跳过核对：{}",
                e.message
            );
            return Ok(RestoreOutcome {
                restored: ok,
                skipped,
            });
        }
    };
    let git = Repository::open(repo)?;
    let head_tree = git.find_commit(Oid::from_str(&head)?)?.tree()?;
    let mut restored = Vec::with_capacity(ok.len());
    for (p, disk) in ok.into_iter().zip(after) {
        let want = entry(&head_tree, &p).map(|(oid, _)| oid.to_string());
        if want.as_deref() == Some(disk.as_str()) {
            restored.push(p);
        } else {
            skipped.push(RestoreSkip {
                path: p,
                reason: "已经写回磁盘，但内容与分支里的版本不同（可能是换行或过滤规则）。\
                         请在「本地改动」里核对。"
                    .into(),
            });
        }
    }
    Ok(RestoreOutcome { restored, skipped })
}

/// 树里某个路径的 (OID, filemode)；不存在为 `None`。
fn entry(tree: &Tree, path: &str) -> Option<(Oid, i32)> {
    let e = tree.get_path(Path::new(path)).ok()?;
    (e.kind() == Some(ObjectType::Blob)).then(|| (e.id(), e.filemode()))
}

/// 普通文件（100644 / 100755）。符号链接、子模块不处理。
fn is_regular(mode: i32) -> bool {
    mode == 0o100644 || mode == 0o100755
}

/// 逐个文件判定，返回 (能放回的, 跳过的及原因)，各自按 `paths` 的顺序。见模块头「为什么安全」。
fn check_paths(
    repo: &str,
    base: &str,
    head: &str,
    paths: &[String],
    with_index: bool,
) -> Result<(Vec<String>, Vec<RestoreSkip>)> {
    let git = Repository::open(repo)?;
    let base = git.find_commit(Oid::from_str(base)?)?;
    let head = git.find_commit(Oid::from_str(head)?)?;
    let (base_tree, head_tree) = (base.tree()?, head.tree()?);
    let commits = range(&git, &base, &head)?;
    let index = if with_index { Some(git.index()?) } else { None };

    let mut skipped = Vec::new();
    let skip = |skipped: &mut Vec<RestoreSkip>, p: &str, why: &str| {
        skipped.push(RestoreSkip {
            path: p.to_string(),
            reason: why.to_string(),
        })
    };
    // 过了树与索引这几关、还要看磁盘的：(路径, SVN 版本的 OID)
    let mut pending: Vec<(String, String)> = Vec::new();
    for p in paths {
        let Some((head_oid, head_mode)) = entry(&head_tree, p).filter(|(_, m)| is_regular(*m))
        else {
            skip(
                &mut skipped,
                p,
                "分支里没有这个普通文件，没有可放回的内容。",
            );
            continue;
        };
        let Some((base_oid, _)) = entry(&base_tree, p) else {
            skip(
                &mut skipped,
                p,
                "SVN 里没有这个文件，只处理改动，不新建文件。",
            );
            continue;
        };
        if base_oid == head_oid {
            skip(
                &mut skipped,
                p,
                "分支里的版本与 SVN 相同，没有可放回的内容。",
            );
            continue;
        }
        let touching: Vec<&Commit> = commits.iter().filter(|c| touches(c, p)).collect();
        if touching.is_empty() {
            skip(
                &mut skipped,
                p,
                "基线之后没有 commit 改过它，判断不了多出来的内容从哪来。",
            );
            continue;
        }
        if let Some(c) = touching
            .iter()
            .find(|c| !parse_local_only(&String::from_utf8_lossy(c.message_bytes())))
        {
            let id = c.id().to_string();
            skip(
                &mut skipped,
                p,
                &format!(
                    "普通 commit {}「{}」也改过它：放回去会把没交给 SVN 的普通改动一起写进磁盘。",
                    &id[..10],
                    c.summary().unwrap_or_default()
                ),
            );
            continue;
        }
        if let Some(index) = &index {
            let conflicted = (1..=3).any(|stage| index.get_path(Path::new(p), stage).is_some());
            let staged = index.get_path(Path::new(p), 0);
            let same = staged
                .as_ref()
                .is_some_and(|e| e.id == head_oid && e.mode as i32 == head_mode);
            if conflicted || !same {
                skip(
                    &mut skipped,
                    p,
                    "暂存区里是别的版本：先处理暂存，免得放回去把它盖掉。",
                );
                continue;
            }
        }
        pending.push((p.clone(), base_oid.to_string()));
    }

    // 磁盘：普通文件，且按 `git add` 的规则算出的 hash 等于 SVN 版本
    let on_disk: Vec<(String, String)> = pending
        .into_iter()
        .filter(|(p, _)| {
            let regular = Path::new(repo)
                .join(p)
                .symlink_metadata()
                .is_ok_and(|m| m.is_file());
            if !regular {
                skip(&mut skipped, p, "磁盘上没有这个文件。");
            }
            regular
        })
        .collect();
    let names: Vec<String> = on_disk.iter().map(|(p, _)| p.clone()).collect();
    let hashes = disk_hashes(repo, &names)?;
    let mut ok = Vec::new();
    for ((p, svn), disk) in on_disk.into_iter().zip(hashes) {
        if disk == svn {
            ok.push(p);
        } else {
            skip(
                &mut skipped,
                &p,
                "磁盘上的内容不是 SVN 的版本（可能有你自己的编辑），不覆盖。",
            );
        }
    }
    // 按请求顺序排好，免得跳过的原因东一条西一条
    let order = |p: &str| paths.iter().position(|q| q == p).unwrap_or(usize::MAX);
    skipped.sort_by_key(|s| order(&s.path));
    Ok((ok, skipped))
}

/// `base..head` 的 commit（不含 base）。
fn range<'r>(git: &'r Repository, base: &Commit, head: &Commit) -> Result<Vec<Commit<'r>>> {
    let mut walk = git.revwalk()?;
    walk.push(head.id())?;
    walk.hide(base.id())?;
    let mut out = Vec::new();
    for oid in walk {
        out.push(git.find_commit(oid?)?);
    }
    Ok(out)
}

/// 这个 commit 相对第一个父提交改过 `path`（内容或 mode）。没有父提交时，树里有它就算。
/// merge commit 只看第一个父提交：它的消息不是 Local-Only，结果总是「不能放回」，偏保守。
fn touches(c: &Commit, path: &str) -> bool {
    let mine = c.tree().ok().and_then(|t| entry(&t, path));
    let parent = c
        .parent(0)
        .ok()
        .and_then(|p| p.tree().ok())
        .and_then(|t| entry(&t, path));
    mine != parent
}

/// 磁盘上这些文件按 `git add` 的过滤规则算出的 blob OID（不写对象库），与 `paths` 同序。
/// 与 `svn_follow::drift_paths` 同一口径：`--stdin-paths` 逐个按路径套 attributes。
fn disk_hashes(repo: &str, paths: &[String]) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = paths.iter().map(|p| format!("{p}\n")).collect();
    let out = run_git_stdin(repo, &["hash-object", "--stdin-paths"], input.as_bytes())?;
    let hashes: Vec<String> = out.lines().map(|l| l.trim().to_string()).collect();
    if hashes.len() != paths.len() {
        return Err(GitError::new(
            "svn_restore_hash_failed",
            "核对磁盘内容时 git hash-object 的输出行数对不上。这次什么都没写，请再试一次。",
        ));
    }
    Ok(hashes)
}
