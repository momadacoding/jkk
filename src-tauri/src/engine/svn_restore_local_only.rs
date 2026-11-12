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
//! 跟进在 journal 存续期间不写磁盘（`svn_follow` 模块头的不变量），所以分三步：
//!
//! - [`restorable_paths`]：跟进时只读地挑出能放回的文件，记进报告；
//! - [`restore_locked`]：跟进发布完成、journal 删掉之后，还拿着同步锁时，把其中「SVN 上最后
//!   一次改它的是我」的文件放回（计划 `2026-11-12-auto-restore-local-only.md` 的 A）。**每条
//!   条件现场重验**，再用 `git checkout-index` 把分支版本写回磁盘；
//! - 没放回的记进待放回清单（[`record_pending`]），横幅上按 [`pending_restores`] 现场重验后
//!   给「放回」按钮（[`restore_local_only`]）。
//!
//! 只自动放回「我最后提交的」：用户有意去掉补丁、还没提交时，`svn update` 带来别人对这个文件
//! 的改动，跟进后它同样满足下面的条件。自动写回会撤销用户的准备，接着整包提交就把补丁带上 SVN。
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
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// [`restore_local_only`] 的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreOutcome {
    /// 已写回磁盘的文件，按请求的顺序。
    pub restored: Vec<String>,
    /// 没写的文件和原因。
    pub skipped: Vec<RestoreSkip>,
    /// 路径 → 改过它的 Local-Only 提交（`短 hash 说明`，从新到旧）。只有过了树这几关的文件才有，
    /// 报告与横幅用它写「文件 ← 提交」（使用报告 2026-11-12 缺陷 6）。
    #[serde(default)]
    pub commits: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreSkip {
    pub path: String,
    /// 给人看的原因。
    pub reason: String,
    /// 过一会儿可能就满足了（文件被占用、暂存了别的版本、分支落后、还没自动放回），留在待放回
    /// 清单里；为假的是再等也不会变的（普通 commit 改过、磁盘有用户的编辑、已经放回过），从清单
    /// 里删掉。只在后端用。
    #[serde(skip)]
    pub retry: bool,
}

/// 横幅上的一个待放回文件（[`pending_restores`]）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingRestore {
    pub path: String,
    /// 改过它的 Local-Only 提交，同 [`RestoreOutcome::commits`]。
    pub commits: Vec<String>,
}

/// 跟进时用：`candidates`（报告的 `drift`）里哪些能放回。`base` 是新基线，`head` 是重放后的
/// tip，两者都还没发布成 ref，所以按 OID 给。只读；不看索引（发布时才拼出新索引），写之前
/// [`restore_locked`] 会再查。
pub fn restorable_paths(
    repo: &str,
    base: &str,
    head: &str,
    candidates: &[String],
) -> Result<Vec<String>> {
    // 没做换行迁移时放回会拒绝，就别列出来
    if candidates.is_empty() || !has_eol_rule(repo) {
        return Ok(Vec::new());
    }
    let (ok, _) = check_paths(repo, base, head, candidates, false)?;
    Ok(ok.into_iter().map(|r| r.path).collect())
}

/// 按钮：把 `paths` 里满足条件的文件写回成分支（HEAD）的版本。放回成功的同时从待放回清单里删掉。
///
/// 持同步锁；有没做完的 SVN 同步、git 正忙、索引被占用、HEAD 不在工作分支上时报错，什么都不写。
/// 单个文件不满足条件只是跳过，写进 [`RestoreOutcome::skipped`]。
pub fn restore_local_only(repo: &str, opid: &str, paths: &[String]) -> Result<RestoreOutcome> {
    if paths.is_empty() {
        return Ok(RestoreOutcome::default());
    }
    let _lock = svn_lock::acquire(repo, opid)?;
    let out = restore_locked(repo, paths)?;
    let gone: Vec<String> = out
        .restored
        .iter()
        .cloned()
        .chain(
            out.skipped
                .iter()
                .filter(|s| !s.retry)
                .map(|s| s.path.clone()),
        )
        .collect();
    if let Err(e) = update_pending(repo, &[], &gone) {
        log::warn!("更新待放回清单失败：{}", e.message);
    }
    Ok(out)
}

/// [`restore_local_only`] 去掉拿锁那一步：**调用方持同步锁**。跟进发布完成、journal 删掉之后在
/// 同一把锁里调用，中间不给别的写操作留空档（使用报告 2026-11-12 的 A）。
///
/// 前置条件不满足时返回 `Err`，什么都不写；不碰待放回清单，由调用方决定怎么记。
pub fn restore_locked(repo: &str, paths: &[String]) -> Result<RestoreOutcome> {
    if paths.is_empty() {
        return Ok(RestoreOutcome::default());
    }
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
    let mut commits: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in &ok {
        commits.insert(r.path.clone(), r.commits.clone());
    }
    if ok.is_empty() {
        return Ok(RestoreOutcome {
            restored: Vec::new(),
            skipped,
            commits,
        });
    }
    // `--stdin -z`：路径不经命令行，空格、中文、上万个文件都不用拆参。`ok` 非空，不会出现
    // 「空列表 = 全部」（AGENTS.md §6.2）；`checkout-index` 本来也不把空输入当成全部
    let input: Vec<u8> = ok.iter().flat_map(|r| r.path.bytes().chain([0])).collect();
    // 一个文件写不进去（被编辑器独占打开之类），`checkout-index` 照样写完其余的、最后退出 1。
    // 所以失败不能直接 `?`：那会把已经写成的也报成失败，下次再点还会给出错的原因（PR 159
    // review 第 1 条）。不管成败都逐个核对磁盘，按实际结果分类
    let write_error = run_git_stdin(repo, &["checkout-index", "-f", "-z", "--stdin"], &input)
        .err()
        .map(|e| e.message);
    if let Some(msg) = &write_error {
        log::warn!("放回 Local-Only 补丁时 checkout-index 报错：{msg}");
    }
    let names: Vec<String> = ok.iter().map(|r| r.path.clone()).collect();
    let after = disk_hashes(repo, &names);
    let mut restored = Vec::with_capacity(ok.len());
    for (r, disk) in ok.into_iter().zip(after) {
        let Some(disk) = disk else {
            // 写没写成说不准：不能报「没写」，也不能说写成了
            skipped.push(RestoreSkip {
                path: r.path,
                reason: "写回之后读不了磁盘上的这个文件，请在「本地改动」里看一眼。".into(),
                retry: true,
            });
            continue;
        };
        if disk == r.head_oid {
            restored.push(r.path);
        } else if disk == r.svn_oid {
            let why = write_error.as_deref().unwrap_or("git 没有给出原因");
            skipped.push(RestoreSkip {
                path: r.path,
                reason: format!("没写进去，磁盘上还是 SVN 的版本：{why}"),
                retry: true,
            });
        } else {
            skipped.push(RestoreSkip {
                path: r.path,
                reason: "写回之后磁盘内容与分支、SVN 都不一样（可能被别的程序同时改了）。\
                         请在「本地改动」里核对。"
                    .into(),
                retry: false,
            });
        }
    }
    Ok(RestoreOutcome {
        restored,
        skipped,
        commits,
    })
}

// —— 待放回清单（计划 `2026-11-12-auto-restore-local-only.md` 的 C） ——
//
// 横幅不按「当前状态」推断哪些文件缺补丁：用户提交 SVN 前有意去掉补丁时，磁盘 = SVN、分支多出
// Local-Only，正好满足放回条件，横幅会在他准备提交时让他放回。所以只认跟进记下的文件。
//
// 文件在 `.git/mygitclient/` 下，watcher 不把那里的变化当成刷新信号（`commands/watch.rs`）。

const PENDING_DIR: &str = "mygitclient";
const PENDING_FILE: &str = "restore-pending.json";

fn pending_path(repo: &str) -> Result<PathBuf> {
    Ok(git_dir(repo)?.join(PENDING_DIR).join(PENDING_FILE))
}

/// 读清单。没有文件、读不了、内容坏了都按空清单：它只是提醒，不能挡住别的操作。
fn read_pending(repo: &str) -> Vec<String> {
    let Ok(path) = pending_path(repo) else {
        return Vec::new();
    };
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            log::warn!("待放回清单读不了，按空清单处理：{e}");
            return Vec::new();
        }
    };
    serde_json::from_slice::<Vec<String>>(&raw).unwrap_or_else(|e| {
        log::warn!("待放回清单坏了，按空清单处理：{e}");
        Vec::new()
    })
}

fn write_pending(repo: &str, paths: &[String]) -> Result<()> {
    let path = pending_path(repo)?;
    if paths.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        };
    }
    let dir = path.parent().expect("清单路径一定有父目录");
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec(paths)
        .map_err(|e| GitError::new("svn_restore_pending", format!("清单序列化失败: {e}")))?;
    // 写临时文件再换名：写一半崩溃留下的是旧清单，不是半个 JSON
    let tmp = dir.join(format!("{PENDING_FILE}.{}.tmp", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, &body) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换待放回清单失败: {e}"))
    })
}

/// 清单 = (旧清单 ∪ `add`) − `remove`，保持先来的在前。**调用方持同步锁**：跟进、放回、
/// 「不放回」都在锁里改它，读-改-写不会互相盖掉。
fn update_pending(repo: &str, add: &[String], remove: &[String]) -> Result<()> {
    let old = read_pending(repo);
    let gone: BTreeSet<&String> = remove.iter().collect();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let next: Vec<String> = old
        .iter()
        .chain(add)
        .filter(|p| !gone.contains(p) && seen.insert((*p).clone()))
        .cloned()
        .collect();
    if next == old {
        return Ok(());
    }
    write_pending(repo, &next)
}

/// 跟进结束时调用（持同步锁）：这次可放回却没放回的记进清单，放回了的删掉；旧条目顺手按放回
/// 条件现场重验，再等也不会满足的（普通 commit 改过、磁盘有用户的编辑、已经是分支的版本）删掉。
///
/// 修剪只在这里与 [`restore_local_only`] 做，都持同步锁。横幅的 [`pending_restores`] 只读：它跟着
/// 状态刷新跑，拿同步锁会让 watcher 看到锁文件变化、再刷一轮，形成回路。
pub fn record_pending(repo: &str, add: &[String], remove: &[String]) -> Result<()> {
    let gone: BTreeSet<&String> = remove.iter().collect();
    let fresh: BTreeSet<&String> = add.iter().collect();
    let old: Vec<String> = read_pending(repo)
        .into_iter()
        .filter(|p| !gone.contains(p) && !fresh.contains(p))
        .collect();
    // 修剪失败只是少删几条：这次要加、要删的照样写进去（review L1）
    let stale = if old.is_empty() {
        Vec::new()
    } else {
        stale_entries(repo, &old).unwrap_or_else(|e| {
            log::warn!("修剪待放回清单时核对失败，这次不修剪：{}", e.message);
            Vec::new()
        })
    };
    let remove: Vec<String> = remove.iter().cloned().chain(stale).collect();
    update_pending(repo, add, &remove)
}

/// `paths` 里再等也不会满足放回条件的（[`RestoreSkip::retry`] 为假）。
fn stale_entries(repo: &str, paths: &[String]) -> Result<Vec<String>> {
    let Some((base, head)) = live_refs(repo)? else {
        return Ok(Vec::new());
    };
    let (_, skipped) = check_paths(repo, &base, &head, paths, true)?;
    Ok(skipped
        .into_iter()
        .filter(|s| !s.retry)
        .map(|s| s.path)
        .collect())
}

/// 待放回清单的原样内容，按记下的顺序。给测试核对修剪用；界面走 [`pending_restores`]。
pub fn pending_list(repo: &str) -> Vec<String> {
    read_pending(repo)
}

/// 现场重验要的 (`svn/base`, HEAD)。HEAD 不在工作分支上时 `None`：先不下结论，切回来再看。
fn live_refs(repo: &str) -> Result<Option<(String, String)>> {
    match current_branch(repo)? {
        Some(b) if b != BASELINE_BRANCH => {}
        _ => return Ok(None),
    }
    let base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    let head = rev_parse(repo, "HEAD")?;
    Ok(Some((base, head)))
}

/// 横幅：清单里此刻能放回的文件，逐个按放回条件现场重验（含索引）。只读，不拿同步锁（见
/// [`record_pending`]）；正在同步、有没做完的同步、没装换行规则、HEAD 不在工作分支上时返回空。
/// 清单为空时不碰 git：它跟着每次状态刷新跑。
pub fn pending_restores(repo: &str) -> Result<Vec<PendingRestore>> {
    let paths = read_pending(repo);
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    // 仓库停在中间态时现场重验的结论不可靠，等同步做完
    if svn_lock::is_held(repo) || svn_journal::read(repo)?.is_some() || !has_eol_rule(repo) {
        return Ok(Vec::new());
    }
    let Some((base, head)) = live_refs(repo)? else {
        return Ok(Vec::new());
    };
    let (ok, _) = check_paths(repo, &base, &head, &paths, true)?;
    Ok(ok
        .into_iter()
        .map(|r| PendingRestore {
            path: r.path,
            commits: r.commits,
        })
        .collect())
}

/// `svn/base..HEAD` 里改过这些文件的 Local-Only 提交（`短 hash 说明`，从新到旧），给报告写
/// 「文件 ← 提交」。只读；不判断能不能放回。HEAD 不在工作分支上、或者一个都没有时不出现在结果里。
pub fn local_only_commits(repo: &str, paths: &[String]) -> Result<BTreeMap<String, Vec<String>>> {
    let mut out = BTreeMap::new();
    if paths.is_empty() {
        return Ok(out);
    }
    let Some((base, head)) = live_refs(repo)? else {
        return Ok(out);
    };
    let git = Repository::open(repo)?;
    let base = git.find_commit(Oid::from_str(&base)?)?;
    let head = git.find_commit(Oid::from_str(&head)?)?;
    let commits = range(&git, &base, &head)?;
    for p in paths {
        let labels: Vec<String> = commits
            .iter()
            .filter(|c| touches(c, p))
            .filter(|c| parse_local_only(&String::from_utf8_lossy(c.message_bytes())))
            .map(label)
            .collect();
        if !labels.is_empty() {
            out.insert(p.clone(), labels);
        }
    }
    Ok(out)
}

/// 横幅上的「不放回」：从清单里删掉，不碰磁盘。
pub fn dismiss_pending_restores(repo: &str, opid: &str, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let _lock = svn_lock::acquire(repo, opid)?;
    update_pending(repo, &[], paths)
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

/// 能放回的文件。
struct Restorable {
    path: String,
    /// 分支（HEAD）版本的 OID。
    head_oid: String,
    /// SVN（`svn/base`）版本的 OID。
    svn_oid: String,
    /// 改过它的 Local-Only 提交，`短 hash 说明`，从新到旧。
    commits: Vec<String>,
}

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
                // 跟进之后就不落后了
                retry: true,
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
    // 再等也不会变的原因
    let skip = |skipped: &mut Vec<RestoreSkip>, p: &str, why: &str| {
        skipped.push(RestoreSkip {
            path: p.to_string(),
            reason: why.to_string(),
            retry: false,
        })
    };
    // 过一会儿可能就满足了的原因（见 [`RestoreSkip::retry`]）
    let skip_for_now = |skipped: &mut Vec<RestoreSkip>, p: &str, why: &str| {
        skipped.push(RestoreSkip {
            path: p.to_string(),
            reason: why.to_string(),
            retry: true,
        })
    };
    // 过了树与索引这几关、还要看磁盘的
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
                skip_for_now(
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
        pending.push(Restorable {
            path: p.clone(),
            head_oid: head_oid.to_string(),
            svn_oid: base_oid.to_string(),
            commits: touching.iter().map(|c| label(c)).collect(),
        });
    }

    // 磁盘：普通文件，且按 `git add` 的规则算出的 hash 等于 SVN 版本
    let on_disk: Vec<Restorable> = pending
        .into_iter()
        .filter(|r| {
            let regular = Path::new(repo)
                .join(&r.path)
                .symlink_metadata()
                .is_ok_and(|m| m.is_file());
            if !regular {
                skip(&mut skipped, &r.path, "磁盘上没有这个文件。");
            }
            regular
        })
        .collect();
    let names: Vec<String> = on_disk.iter().map(|r| r.path.clone()).collect();
    let hashes = disk_hashes(repo, &names);
    let mut ok = Vec::new();
    for (r, disk) in on_disk.into_iter().zip(hashes) {
        let Some(disk) = disk else {
            skip_for_now(
                &mut skipped,
                &r.path,
                "读不了磁盘上的这个文件（可能被别的程序占用），没动它。",
            );
            continue;
        };
        if disk == r.svn_oid {
            ok.push(r);
        } else if disk == r.head_oid {
            // 已经放回过（重复点、或用户自己丢弃过本地改动）：不是用户的编辑，别吓人
            skip(&mut skipped, &r.path, "磁盘上已经是分支的版本，不用放回。");
        } else {
            skip(
                &mut skipped,
                &r.path,
                "磁盘上的内容不是 SVN 的版本（可能有你自己的编辑），不覆盖。",
            );
        }
    }
    // 按请求顺序排好，免得跳过的原因东一条西一条
    let order = |p: &str| paths.iter().position(|q| q == p).unwrap_or(usize::MAX);
    skipped.sort_by_key(|s| order(&s.path));
    Ok((ok, skipped))
}

/// 报告里提交的写法：`短 hash 说明`，与跟进报告的 `overlap` 同一口径（`git log --format=%h %s`）。
fn label(c: &Commit) -> String {
    let id = c
        .as_object()
        .short_id()
        .ok()
        .and_then(|b| b.as_str().map(str::to_string))
        .unwrap_or_else(|| c.id().to_string()[..7].to_string());
    format!("{id} {}", c.summary().unwrap_or_default())
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
