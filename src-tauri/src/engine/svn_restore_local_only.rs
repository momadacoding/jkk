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
//! 3. （写的时候）索引里只有 stage 0、且等于 HEAD，没标 skip-worktree / assume-unchanged：用户
//!    没暂存别的版本，写完磁盘 = 索引 = HEAD；
//! 4. HEAD 是 `svn/base` 的后代：分支没落后，写回 HEAD 的版本不会撤掉 SVN 后来的改动；
//! 5. 仓库装了换行规则（`* -text`）：写出来的字节就是 blob，SVN 里不会整个文件变换行。
//!
//! 写回去的内容在 git 里，原来的内容在 SVN 里（`svn revert` 就能拿回来），两边都丢不了东西。

use crate::engine::git_cli::run_git_stdin;
use crate::engine::graph::parse_local_only;
use crate::engine::svn_attributes::has_eol_rule;
use crate::engine::svn_baseline::git_dir;
use crate::engine::svn_config::BASELINE_BRANCH;
use crate::engine::svn_journal;
use crate::engine::svn_lock;
use crate::engine::svn_sync_check::{check_git_idle, current_branch, rev_parse};
use crate::error::{GitError, Result};
use git2::{Commit, IndexEntryExtendedFlag, IndexEntryFlag, ObjectType, Oid, Repository, Tree};
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
    // 没做换行迁移时按钮会拒绝，就别给按钮
    if candidates.is_empty() || !has_eol_rule(repo) {
        return Ok(Vec::new());
    }
    let (ok, _) = check_paths(repo, base, head, candidates, false)?;
    Ok(ok.into_iter().map(|(p, _, _)| p).collect())
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
    // 没装换行规则的旧仓库：`checkout-index` 按 autocrlf 写出 CRLF，SVN 里整个文件每一行都变了
    // （`svn_attributes` 模块头的「最坏的下游后果」；PR 159 review 第 2 条）
    if !has_eol_rule(repo) {
        return Err(GitError::new(
            "svn_restore_needs_eol_migration",
            "这个仓库还没做换行迁移，写回的文件换行可能和 SVN 里不一样。请先按顶部的提示做换行迁移，\
             再放回 Local-Only 补丁。",
        ));
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
    let input: Vec<u8> = ok
        .iter()
        .flat_map(|(p, _, _)| p.bytes().chain([0]))
        .collect();
    // 一个文件写不进去（被编辑器独占打开之类），`checkout-index` 照样写完其余的、最后退出 1。
    // 所以失败不能直接 `?`：那会把已经写成的也报成失败，下次再点还会给出错的原因（PR 159
    // review 第 1 条）。不管成败都逐个核对磁盘，按实际结果分类
    let write_error = run_git_stdin(repo, &["checkout-index", "-f", "-z", "--stdin"], &input)
        .err()
        .map(|e| e.message);
    if let Some(msg) = &write_error {
        log::warn!("放回 Local-Only 补丁时 checkout-index 报错：{msg}");
    }
    let names: Vec<String> = ok.iter().map(|(p, _, _)| p.clone()).collect();
    let after = disk_hashes(repo, &names);
    let mut restored = Vec::with_capacity(ok.len());
    for ((p, head_oid, svn_oid), disk) in ok.into_iter().zip(after) {
        let Some(disk) = disk else {
            // 写没写成说不准：不能报「没写」，也不能说写成了
            skipped.push(RestoreSkip {
                path: p,
                reason: "写回之后读不了磁盘上的这个文件，请在「本地改动」里看一眼。".into(),
            });
            continue;
        };
        if disk == head_oid {
            restored.push(p);
        } else if disk == svn_oid {
            let why = write_error.as_deref().unwrap_or("git 没有给出原因");
            skipped.push(RestoreSkip {
                path: p,
                reason: format!("没写进去，磁盘上还是 SVN 的版本：{why}"),
            });
        } else {
            skipped.push(RestoreSkip {
                path: p,
                reason: "写回之后磁盘内容与分支、SVN 都不一样（可能被别的程序同时改了）。\
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

/// 能放回的文件：(路径, 分支版本的 OID, SVN 版本的 OID)。
type Restorable = (String, String, String);

/// 逐个文件判定，返回 (能放回的, 跳过的及原因)，各自按 `paths` 的顺序。见模块头「为什么安全」。
fn check_paths(
    repo: &str,
    base: &str,
    head: &str,
    paths: &[String],
    with_index: bool,
) -> Result<(Vec<Restorable>, Vec<RestoreSkip>)> {
    let git = Repository::open(repo)?;
    let base = git.find_commit(Oid::from_str(base)?)?;
    let head = git.find_commit(Oid::from_str(head)?)?;
    let skip_all = |why: &str| {
        let skipped = paths
            .iter()
            .map(|p| RestoreSkip {
                path: p.clone(),
                reason: why.to_string(),
            })
            .collect();
        Ok((Vec::new(), skipped))
    };
    // 分支落后基线：`base..HEAD` 只剩 Local-Only commit，但 HEAD 缺着基线后来的 SVN 改动，
    // 写回 HEAD 的版本等于在磁盘上撤销它们（PR 159 review 第 3 条）
    if base.id() != head.id() && !git.graph_descendant_of(head.id(), base.id())? {
        return skip_all(
            "当前分支落后于基线，写回分支的版本会撤掉 SVN 后来的改动。请先跟进这条分支。",
        );
    }
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
    // 过了树与索引这几关、还要看磁盘的：(路径, 分支版本的 OID, SVN 版本的 OID)
    let mut pending: Vec<Restorable> = Vec::new();
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
            // skip-worktree 的条目 `checkout-index` 拒绝写；assume-unchanged 写了 git 也看不见。
            // 都是用户有意设的，不替他改（PR 159 review 第 1 条）
            let hidden = staged.as_ref().is_some_and(|e| {
                e.flags_extended & IndexEntryExtendedFlag::SKIP_WORKTREE.bits() != 0
                    || e.flags & IndexEntryFlag::VALID.bits() != 0
            });
            if hidden {
                skip(
                    &mut skipped,
                    p,
                    "暂存区里这个文件标了 skip-worktree 或 assume-unchanged，不替你改。",
                );
                continue;
            }
        }
        pending.push((p.clone(), head_oid.to_string(), base_oid.to_string()));
    }

    // 磁盘：普通文件，且按 `git add` 的规则算出的 hash 等于 SVN 版本
    let on_disk: Vec<Restorable> = pending
        .into_iter()
        .filter(|(p, _, _)| {
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
    let names: Vec<String> = on_disk.iter().map(|(p, _, _)| p.clone()).collect();
    let hashes = disk_hashes(repo, &names);
    let mut ok = Vec::new();
    for ((p, head_oid, svn), disk) in on_disk.into_iter().zip(hashes) {
        let Some(disk) = disk else {
            skip(
                &mut skipped,
                &p,
                "读不了磁盘上的这个文件（可能被别的程序占用），没动它。",
            );
            continue;
        };
        if disk == svn {
            ok.push((p, head_oid, svn));
        } else if disk == head_oid {
            // 已经放回过（重复点、或用户自己丢弃过本地改动）：不是用户的编辑，别吓人
            skip(&mut skipped, &p, "磁盘上已经是分支的版本，不用放回。");
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

/// 磁盘上这些文件按 `git add` 的过滤规则算出的 blob OID（不写对象库），与 `paths` 同序；读不了的
/// （被别的程序独占打开之类）为 `None`。与 `svn_follow::drift_paths` 同一口径：`--stdin-paths`
/// 逐个按路径套 attributes。
///
/// 先整批算；整批失败时 git 只说第一个读不了的文件，所以逐个重算，免得一个被锁住的文件把
/// 其余的都拖成「核对不了」。
fn disk_hashes(repo: &str, paths: &[String]) -> Vec<Option<String>> {
    let batch = |ps: &[String]| -> Option<Vec<String>> {
        // `--stdin-paths` 按行读，以 `"` 开头的行会被当成 C 风格引号去掉；Windows 文件名里不能有
        // `"`，换行也进不了文件名，所以按行喂是安全的（`checkout-index` 那边用的是 NUL 分隔）
        let input: String = ps.iter().map(|p| format!("{p}\n")).collect();
        let out = run_git_stdin(repo, &["hash-object", "--stdin-paths"], input.as_bytes()).ok()?;
        let hashes: Vec<String> = out.lines().map(|l| l.trim().to_string()).collect();
        (hashes.len() == ps.len()).then_some(hashes)
    };
    if paths.is_empty() {
        return Vec::new();
    }
    if let Some(all) = batch(paths) {
        return all.into_iter().map(Some).collect();
    }
    paths
        .iter()
        .map(|p| batch(std::slice::from_ref(p)).and_then(|mut h| h.pop()))
        .collect()
}
