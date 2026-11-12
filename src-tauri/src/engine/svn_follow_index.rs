//! 跟进时拼索引：HEAD 从 OLD 换到 NEW，暂存区要挺过去（分析 §6.11.1；日志 W22–W26、W28）。
//!
//! # 为什么不能只跑一次 `read-tree -i -m OLD NEW`
//!
//! 「已暂存」且「OLD→NEW 也改了」的路径，只要索引与 OLD、NEW 都不同，read-tree 就整体拒绝
//! （W25）。先把最终条目写进去也不行：最终条目（保留的暂存、合并结果）同样与两边都不同。
//! 所以顺序是：这些路径先写成**等于 NEW** → read-tree → 再写最终条目。
//!
//! # 为什么写的是 `.git/index.lock`
//!
//! 全程持有 git 自己的索引锁，外部 git 的写操作被挡回（W23），拼好的索引留在锁文件里，等编排
//! 把分支 CAS 过去之后再一次 rename 换入（日志「偏离计划」的发布顺序）。CAS 失败时只要删锁，
//! 真实索引从未被碰过。`GIT_INDEX_FILE` 指向锁文件时 git 自己锁的是 `index.lock.lock`，互不相干。
//!
//! # 为什么先复制真实索引
//!
//! 没变的条目要带着 stat 走（否则 1.5 万文件的 `git status` 冷缓存 53 秒，见
//! `svn_baseline::build_index_keeping_stat`），i-t-a 与 skip-worktree 也靠它留下。
//!
//! # 不碰工作区
//!
//! 写操作只有 `update-index`（`--index-info`、标志、`--refresh`）与 `read-tree -i`（不带 `-u`）。
//! `--refresh` 读磁盘、只写 stat。
//!
//! # git 版本
//!
//! `merge-file --object-id` 要 git ≥ 2.43，比重放要的 2.40 新（日志 Task 8「计划的错误」）。

use crate::engine::git_cli::{
    run_git, run_git_bytes, run_git_code, run_git_env, run_git_stdin, run_git_stdin_env,
};
use crate::engine::svn_baseline::{copy_with_mtime, parse_entry_flags, IndexLock, TempFile};
use crate::engine::svn_leak::{stage_hunks_in_new, StageTally};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

/// 意外失败（读不懂 git 的输出、路径不是 UTF-8）。
const FAILED: &str = "svn_follow_index_failed";

/// rename 被外部进程占着 `.git/index` 时的退避，合计约 1.5 秒（W24）。
const RENAME_BACKOFF_MS: [u64; 6] = [50, 100, 150, 250, 400, 550];

/// 钉索引的 commit 的身份：写死，这样用户没配 `user.name` 时也不会多一个失败点。
const PIN_IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "mygitclient"),
    ("GIT_AUTHOR_EMAIL", "mygitclient@localhost"),
    ("GIT_COMMITTER_NAME", "mygitclient"),
    ("GIT_COMMITTER_EMAIL", "mygitclient@localhost"),
];

/// 判定一条路径需要的外部事实。`None` 表示不知道，一律按「拿不准」处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PathFacts {
    /// 这个文件最新一次提交的作者是我。
    pub author_is_me: Option<bool>,
    /// 这份暂存在失焦快照里已经有了（早于外部提交）。
    pub staged_before_mark: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StageDecision {
    TakeNew,
    KeepStaged,
    /// 三方合并的结果 blob。
    Merged(String),
    /// 暂存内容没能保留，取 NEW。原内容在钉住的索引里。
    LostToNew,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeReport {
    /// 「已暂存 ∩ OLD→NEW 改过」的每条路径，外加与 NEW 目录 / 文件碰撞的暂存。
    pub decisions: BTreeMap<String, StageDecision>,
    /// 作者是我但先后拿不准，暂存留着，交给用户确认。
    pub uncertain: Vec<String>,
    /// 暂存内容没能保留，而且其中有改动不在 NEW 里（`superseded` 的不在此列）。
    pub lost: Vec<String>,
    /// 暂存内容没能保留，但它相对 OLD 的每处改动 NEW 里都有了：没丢东西，暂存区取 NEW
    /// （2026-11-11 使用报告第 4 节，方案 C；判据见 [`stage_hunks_in_new`]）。原内容照旧钉住。
    #[serde(default)]
    pub superseded: Vec<String>,
    /// `lost` 里一部分改动已在 NEW 里的：路径 → 计数。一处都不在、或判不了（二进制、删除、
    /// mode 变化）的不在这里。
    #[serde(default)]
    pub partial: BTreeMap<String, StageTally>,
    /// 作者是我、确定早于提交，按中间版本丢弃（分析 §6.12 的通知要列出来）。
    /// 与 `TakeNew` 里「索引本来就等于 NEW」的路径不同，那些没有丢东西。
    pub discarded: Vec<String>,
}

/// 拼好、留在 `.git/index.lock` 里的索引。持有它就持有索引锁。
pub struct PreparedIndex {
    lock: IndexLock,
    pinned_ref: String,
    pub report: ComposeReport,
}

impl PreparedIndex {
    /// 钉着原始索引的 ref，放弃或换入之后都保留。
    pub fn pinned_ref(&self) -> &str {
        &self.pinned_ref
    }

    /// 拼好的索引所在的锁文件（绝对路径，给 `GIT_INDEX_FILE`）。
    pub(crate) fn lock_abs_path(&self) -> Result<String> {
        self.lock.abs_path()
    }

    /// rename 成 `.git/index`，成功之后不再持锁。
    ///
    /// 收 `&mut self`：失败时锁必须还在手里。编排这时已经移了分支，要先 CAS 回去，再放开外部
    /// git（日志「偏离计划」的发布顺序），否则外部 `git commit` 会踩进「分支已移、索引是旧的」。
    pub fn commit(&mut self) -> Result<()> {
        if self.lock.is_released() {
            return Err(GitError::new(FAILED, "这份索引已经换入过了。"));
        }
        rename_lock_into_index(self.lock.path())?;
        self.lock.release();
        Ok(())
    }

    /// 放弃：删锁，真实索引不变，钉住的 ref 保留。Drop 同样处理。
    pub fn abandon(self) {}

    /// Drop 不再删锁，`.git/index.lock` 原样留在磁盘上。
    ///
    /// 分支已经移走、又移不回去时用：这时删锁就是「分支已移、索引是旧的、外部 git 可以提交」，
    /// 宁可挡住外部 git，等 journal 补做（Review Focus 5）。
    pub(crate) fn keep_lock_on_drop(&mut self) {
        self.lock.release();
    }
}

/// 把 `lock`（`.git/index.lock`）换成同目录的 `index`。os error 5 / 32 退避重试（W24）。
///
/// 单独公开给崩溃恢复：journal 补做 rename 时手里没有 [`PreparedIndex`]。
pub fn rename_lock_into_index(lock: &Path) -> Result<()> {
    let index = lock.with_file_name("index");
    for wait in RENAME_BACKOFF_MS {
        match std::fs::rename(lock, &index) {
            Ok(()) => return Ok(()),
            Err(e) if is_held_open(&e) => std::thread::sleep(Duration::from_millis(wait)),
            Err(e) => return Err(rename_failed(&index, &e)),
        }
    }
    std::fs::rename(lock, &index).map_err(|e| rename_failed(&index, &e))
}

fn is_held_open(e: &std::io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32))
}

fn rename_failed(index: &Path, e: &std::io::Error) -> GitError {
    GitError::new(
        "svn_follow_index_rename_failed",
        format!(
            "换入新索引失败，可能有别的程序占着 {}：{e}",
            index.display()
        ),
    )
}

/// 按分析 §6.11.1 拼 NEW 的索引，留在 `.git/index.lock` 里，**不动真实索引、HEAD 与磁盘**。
///
/// `old`、`new` 是完整 OID：旧 HEAD 与新 HEAD（分支上有 commit 时是重放前后的 tip，不是基线）。
/// 失败时锁随 RAII 删掉；钉住的 ref 可能已写，无害。
pub fn prepare_index(
    repo: &str,
    opid: &str,
    old: &str,
    new: &str,
    facts: &dyn Fn(&str) -> PathFacts,
) -> Result<PreparedIndex> {
    prepare_index_with(repo, opid, old, new, facts, &mut || Ok(()))
}

/// 同 [`prepare_index`]，建好索引锁之后、动手之前先调 `locked`：编排要在这里把「锁是我方建的」
/// 写进 journal。它失败时锁随 RAII 删掉，什么都没做。
pub fn prepare_index_with(
    repo: &str,
    opid: &str,
    old: &str,
    new: &str,
    facts: &dyn Fn(&str) -> PathFacts,
    locked: &mut dyn FnMut() -> Result<()>,
) -> Result<PreparedIndex> {
    // 1. 建锁，之后 .git/index 就不会再变
    let lock = IndexLock::acquire(repo)?;
    locked()?;
    let index_file = lock.path().with_file_name("index");
    if !run_git(repo, &["ls-files", "-u", "-z"])?.is_empty() {
        return Err(GitError::new(
            "svn_follow_index_conflicted",
            "暂存区里有未解决的冲突，请先解决再跟进。",
        ));
    }
    // git 读不了 0 字节的索引（fatal: index file smaller than expected），而锁文件这时正是
    // 0 字节；没有索引时 git 自己也只是把一切当成「已暂存删除」，这个形状不值得往下走
    let raw = match std::fs::read(&index_file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(GitError::new(
                "svn_follow_index_missing",
                format!("找不到索引文件 {}。", index_file.display()),
            ));
        }
        Err(e) => return Err(e.into()),
    };

    let staged = read_changes(repo, &["diff-index", "--cached", "--no-renames", "-z", old])?;

    // 2. 钉原始字节（W28）：write-tree 还原不了 i-t-a、标志与 stat
    let pinned_ref = pin_raw_index(repo, opid, &raw, &staged)?;

    // 3. 工作副本就是锁文件。mtime 要跟着走（理由见 build_index_keeping_stat）
    copy_with_mtime(&index_file, lock.path())?;
    let work = lock.abs_path()?;
    let env = [("GIT_INDEX_FILE", work.as_str())];
    // 第 4、6 步的 --index-info 与 read-tree 都会丢标志，要在动手之前记
    let flags_before = run_git_env(repo, &["ls-files", "-v", "-z"], &env)?;

    // 4. 求交集、定最终结果，交集全部先写成 NEW（W25）
    let changed = read_changes(repo, &["diff-tree", "-r", "--no-renames", "-z", old, new])?;
    let plan = compose(repo, &index_file, &staged, &changed, facts)?;
    let zero = "0".repeat(old.len());
    write_entries(repo, &env, &zero, &plan.to_new)?;

    // 5. 其余路径（没暂存的跟 NEW，NEW 没碰的留暂存）
    run_git_env(repo, &["read-tree", "-i", "-m", old, new], &env)?;

    // 6. 最终条目
    write_entries(repo, &env, &zero, &plan.finals)?;

    // 7. 补回标志
    restore_flags(repo, &env, &flags_before)?;

    // 8. stat 只关乎快慢，失败不让跟进失败（约定同 refresh_index_stat）
    if let Err(e) = run_git_env(repo, &["update-index", "-q", "--refresh"], &env) {
        log::warn!("刷新索引 stat 失败，之后第一次 status 会慢：{}", e.message);
    }

    Ok(PreparedIndex {
        lock,
        pinned_ref,
        report: plan.report,
    })
}

/// 存成 blob、包进 commit，挂在 `refs/mygitclient/follow/<opid>`：树里 `index` 是原始字节，
/// `blobs/<oid>` 是全部已暂存的 blob。
///
/// 第二部分不能省：索引文件对 git 只是一个 blob，gc 看不到它引用了什么。暂存内容只有索引指着，
/// 不挂进树里的话 `gc --prune=now` 之后钉住的索引还在，要找回的内容没了（Review 2 探针 P6）。
/// 没暂存的条目等于 OLD 的树，靠 OLD 本身可达（分支之外还有 reflog），这里不管。
///
/// 同一 opid 重来时覆盖：上一次既然走到了重来，就没有换入过，真实索引此刻的字节才是该钉的那份。
fn pin_raw_index(
    repo: &str,
    opid: &str,
    raw: &[u8],
    staged: &BTreeMap<String, Change>,
) -> Result<String> {
    let blob = run_git_stdin(repo, &["hash-object", "-w", "--no-filters", "--stdin"], raw)?;
    let mut listing = format!("100644 blob {}\tindex\n", blob.trim());
    let blobs = existing_blobs(repo, staged)?;
    if !blobs.is_empty() {
        let inner: String = blobs
            .iter()
            .map(|b| format!("100644 blob {b}\t{b}\n"))
            .collect();
        let sub = run_git_stdin(repo, &["mktree"], inner.as_bytes())?;
        listing.push_str(&format!("040000 tree {}\tblobs\n", sub.trim()));
    }
    let tree = run_git_stdin(repo, &["mktree"], listing.as_bytes())?;
    let msg = format!("mygitclient: follow {opid} raw index\n");
    let file = TempFile::in_git_dir(repo, "msg", msg.as_bytes())?;
    let path = file.abs_path()?;
    let commit = run_git_env(
        repo,
        &["commit-tree", tree.trim(), "-F", &path],
        &PIN_IDENTITY,
    )?;
    drop(file);
    let name = format!("refs/mygitclient/follow/{opid}");
    run_git(repo, &["update-ref", &name, commit.trim()])?;
    Ok(name)
}

/// `staged` 指着的、对象库里确实有的 blob。
///
/// i-t-a 在 `diff-index` 里是空 blob，对象库里未必有；gitlink 是 commit。mktree 遇到缺的对象
/// 整条失败，写进树里的话 gc 又会报错，所以先问一遍。
fn existing_blobs(repo: &str, staged: &BTreeMap<String, Change>) -> Result<BTreeSet<String>> {
    let wanted: BTreeSet<&str> = staged
        .values()
        .filter_map(|c| c.new.as_ref())
        .map(|e| e.oid.as_str())
        .collect();
    if wanted.is_empty() {
        return Ok(BTreeSet::new());
    }
    let input: String = wanted.iter().map(|o| format!("{o}\n")).collect();
    let out = run_git_stdin(repo, &["cat-file", "--batch-check"], input.as_bytes())?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let mut f = l.split(' ');
            let (oid, kind) = (f.next()?, f.next()?);
            (kind == "blob").then(|| oid.to_string())
        })
        .collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    mode: String,
    oid: String,
}

impl Entry {
    /// 普通文件（可执行也算）。符号链接与 gitlink 的 blob 不是文本内容，不能三方合并。
    fn is_regular_file(&self) -> bool {
        self.mode == "100644" || self.mode == "100755"
    }
}

/// raw diff 的两侧；`None` 表示这一侧没有该路径。
struct Change {
    old: Option<Entry>,
    new: Option<Entry>,
}

/// 解析 `--no-renames -z` 的 raw 输出。
///
/// 路径严格按 UTF-8 解：这些路径接下来要原样写回索引，经 `from_utf8_lossy` 换成 U+FFFD
/// 的路径会变成另一条条目。宁可拒绝。
fn read_changes(repo: &str, args: &[&str]) -> Result<BTreeMap<String, Change>> {
    let out = run_git_bytes(repo, args)?;
    let mut fields = out.split(|b| *b == 0);
    let mut map = BTreeMap::new();
    while let Some(meta) = fields.next() {
        if meta.is_empty() {
            continue;
        }
        let meta = std::str::from_utf8(meta).ok();
        let f: Vec<&str> = meta.unwrap_or("").split(' ').collect();
        let path = fields.next().and_then(|p| std::str::from_utf8(p).ok());
        let (&[old_mode, new_mode, old_oid, new_oid, _], Some(path)) = (f.as_slice(), path) else {
            return Err(GitError::new(
                FAILED,
                format!("读不懂 git 的 raw diff 输出（路径不是 UTF-8？）：{f:?}"),
            ));
        };
        map.insert(
            path.to_string(),
            Change {
                old: entry(old_mode.trim_start_matches(':'), old_oid),
                new: entry(new_mode, new_oid),
            },
        );
    }
    Ok(map)
}

fn entry(mode: &str, oid: &str) -> Option<Entry> {
    (mode != "000000").then(|| Entry {
        mode: mode.to_string(),
        oid: oid.to_string(),
    })
}

/// 两轮写入：`(路径, 条目)`，条目 `None` 表示从索引里摘掉。
type Writes = Vec<(String, Option<Entry>)>;

#[derive(Default)]
struct Plan {
    report: ComposeReport,
    /// 第 4 步：read-tree 之前先写成 NEW。
    to_new: Writes,
    /// 第 6 步：read-tree 之后写最终条目。
    finals: Writes,
}

/// 决策规则（分析 §6.11.1 的表，T0 换成失焦快照）。`staged` 是 OLD→索引，`changed` 是 OLD→NEW。
fn compose(
    repo: &str,
    index_file: &Path,
    staged: &BTreeMap<String, Change>,
    changed: &BTreeMap<String, Change>,
    facts: &dyn Fn(&str) -> PathFacts,
) -> Result<Plan> {
    let mut plan = Plan::default();
    if staged.is_empty() || changed.is_empty() {
        return Ok(plan);
    }
    // read-tree 要带进索引的路径
    let incoming: BTreeSet<&str> = changed
        .iter()
        .filter(|(_, c)| c.new.is_some())
        .map(|(p, _)| p.as_str())
        .collect();
    let intent_to_add = intent_to_add_paths(index_file)?;

    for (path, s) in staged {
        let in_index = s.new.as_ref();
        let collides = in_index.is_some() && collides_with(path, &incoming);
        let Some(c) = changed.get(path) else {
            // NEW 没碰这条路径，正常留给 read-tree。但索引放不下「文件 a」与「a/b」：
            // read-tree 退出 0 却无声吞掉其中一边（Task 8 实测），只能让位并如实报告
            if collides {
                plan.report.lost.push(path.clone());
                plan.report
                    .decisions
                    .insert(path.clone(), StageDecision::LostToNew);
                plan.to_new.push((path.clone(), None));
            }
            continue;
        };
        plan.to_new.push((path.clone(), c.new.clone()));

        if in_index == c.new.as_ref() {
            plan.report
                .decisions
                .insert(path.clone(), StageDecision::TakeNew);
            continue;
        }
        let ita = intent_to_add.contains(path);
        let mut decision = decide(repo, in_index, c, ita, facts(path))?;
        // 写回暂存的文件会把 NEW 带进来的目录顶掉（`--index-info` 允许替换）
        if collides && decision == StageDecision::KeepStaged {
            decision = StageDecision::LostToNew;
        }
        match &decision {
            StageDecision::TakeNew if !ita => plan.report.discarded.push(path.clone()),
            StageDecision::TakeNew => {}
            StageDecision::KeepStaged => {
                plan.report.uncertain.push(path.clone());
                plan.finals.push((path.clone(), s.new.clone()));
            }
            StageDecision::Merged(oid) => {
                // 走到 Merged 时三侧 mode 相同，NEW 一定有这条路径
                let merged = c.new.clone().map(|n| Entry {
                    oid: oid.clone(),
                    ..n
                });
                plan.finals.push((path.clone(), merged));
            }
            StageDecision::LostToNew => {
                match stage_tally(repo, c.old.as_ref(), in_index, c.new.as_ref()) {
                    Some(t) if t.total > 0 && t.present == t.total => {
                        plan.report.superseded.push(path.clone())
                    }
                    Some(t) if t.present > 0 => {
                        plan.report.lost.push(path.clone());
                        plan.report.partial.insert(path.clone(), t);
                    }
                    _ => plan.report.lost.push(path.clone()),
                }
            }
        }
        plan.report.decisions.insert(path.clone(), decision);
    }
    Ok(plan)
}

/// 保不住的暂存（OLD = `base`、暂存 = `staged`、NEW = `new`），每处改动在 NEW 里有没有（方案 C）。
///
/// 只判暂存与 NEW 都是普通文件、OLD 是普通文件或没有的情形；其余（删除、mode 变化、二进制）以及
/// 读对象出错都返回 `None`，照旧算丢失：这是只能往「多报」错的判定。
fn stage_tally(
    repo: &str,
    base: Option<&Entry>,
    staged: Option<&Entry>,
    new: Option<&Entry>,
) -> Option<StageTally> {
    let (staged, new) = (staged?, new?);
    if !staged.is_regular_file() || !new.is_regular_file() || staged.mode != new.mode {
        return None;
    }
    if base.is_some_and(|b| !b.is_regular_file() || b.mode != staged.mode) {
        return None;
    }
    let r = git2::Repository::open(repo).ok()?;
    let text = |e: &Entry| -> Option<Vec<u8>> {
        let blob = r.find_blob(git2::Oid::from_str(&e.oid).ok()?).ok()?;
        (!blob.is_binary()).then(|| blob.content().to_vec())
    };
    let b = match base {
        Some(b) => text(b)?,
        None => Vec::new(),
    };
    match stage_hunks_in_new(&b, &text(staged)?, &text(new)?) {
        Ok(t) => Some(t),
        Err(e) => {
            log::warn!("判断暂存是否已在 SVN 上时出错，按丢失报告：{}", e.message);
            None
        }
    }
}

/// 索引与 NEW 不同的路径（`ours != theirs` 已判过）怎么办。
fn decide(
    repo: &str,
    in_index: Option<&Entry>,
    c: &Change,
    ita: bool,
    f: PathFacts,
) -> Result<StageDecision> {
    if f.author_is_me == Some(true) {
        // i-t-a 没有暂存内容，没什么可保留的；硬留还有害：NEW 有这个文件时，i-t-a 条目不进
        // write-tree，下次提交等于把 NEW 里的文件删掉
        return Ok(if ita || f.staged_before_mark == Some(true) {
            StageDecision::TakeNew
        } else {
            StageDecision::KeepStaged
        });
    }
    // 作者不是我或不知道（D4）：这份暂存可能从没进过 SVN，能合就合
    let inputs = match (c.new.as_ref(), c.old.as_ref(), in_index) {
        (Some(ours), Some(base), Some(theirs))
            if !ita
                && [ours, base, theirs].iter().all(|e| e.is_regular_file())
                && ours.mode == base.mode
                && theirs.mode == base.mode =>
        {
            Some((ours, base, theirs))
        }
        _ => None,
    };
    match inputs {
        Some((ours, base, theirs)) => three_way(repo, ours, base, theirs),
        // 删除 / 新增 / mode / i-t-a：不能 KeepStaged，那会在暂存区留下对 NEW 改动的反向 diff
        None => Ok(StageDecision::LostToNew),
    }
}

/// `git merge-file --object-id`（W22）。参数顺序 `<ours> <base> <theirs>`，退出码 = 冲突块数。
fn three_way(repo: &str, ours: &Entry, base: &Entry, theirs: &Entry) -> Result<StageDecision> {
    let (rc, out) = run_git_code(
        repo,
        &[
            "merge-file",
            "--object-id",
            "-L",
            "ours",
            "-L",
            "base",
            "-L",
            "theirs",
            &ours.oid,
            &base.oid,
            &theirs.oid,
        ],
    )?;
    match rc {
        0 => match out.trim() {
            "" => Err(GitError::new(FAILED, "git merge-file 成功了却没给 blob。")),
            oid => Ok(StageDecision::Merged(oid.to_string())),
        },
        // 冲突块数（最多 127）；255 = 二进制
        1..=127 | 255 => Ok(StageDecision::LostToNew),
        // 这两个不是冲突，不能当成「合不了」把暂存丢掉：129 是不认识 --object-id，
        // 128 是读不到对象（实测）
        129 => Err(GitError::new(
            "svn_follow_git_too_old",
            "跟进需要 git ≥ 2.43（merge-file --object-id），请升级 git。",
        )),
        _ => Err(GitError::new(
            FAILED,
            format!("git merge-file 异常退出（{rc}）。"),
        )),
    }
}

/// i-t-a 在 `diff-index` 里与「暂存了一个空文件」完全同形（都是 e69de29），`ls-files` 也看不出，
/// 只有条目的扩展标志能分开。读取走 libgit2，只读。
fn intent_to_add_paths(index_file: &Path) -> Result<BTreeSet<String>> {
    let index = git2::Index::open(index_file)?;
    let paths = index
        .iter()
        .filter(|e| {
            git2::IndexEntryExtendedFlag::from_bits_truncate(e.flags_extended).is_intent_to_add()
        })
        .map(|e| String::from_utf8_lossy(&e.path).to_string())
        .collect();
    Ok(paths)
}

/// `path` 与 `incoming` 里的某条路径互为文件 / 目录。
fn collides_with(path: &str, incoming: &BTreeSet<&str>) -> bool {
    let below = format!("{path}/");
    let has_below = incoming
        .range(below.as_str()..)
        .next()
        .is_some_and(|p| p.starts_with(&below));
    has_below
        || path
            .rmatch_indices('/')
            .any(|(i, _)| incoming.contains(&path[..i]))
}

/// `--index-info` 一次写完。`entries` 为空时不发命令（AGENTS.md §6.2）。
fn write_entries(repo: &str, env: &[(&str, &str)], zero: &str, entries: &Writes) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut input = Vec::new();
    for (path, e) in entries {
        let line = match e {
            Some(e) => format!("{} {}\t{path}", e.mode, e.oid),
            None => format!("0 {zero}\t{path}"),
        };
        input.extend_from_slice(line.as_bytes());
        input.push(0);
    }
    run_git_stdin_env(repo, &["update-index", "-z", "--index-info"], &input, env)?;
    Ok(())
}

/// 补回 assume-unchanged（read-tree 换掉条目时丢，W25）与 skip-worktree（`--index-info` 丢，
/// Task 8 实测）。只补「之前有、现在没有、条目还在」的路径：条目已不在时补标志是 fatal。
fn restore_flags(repo: &str, env: &[(&str, &str)], before: &str) -> Result<()> {
    let after = run_git_env(repo, &["ls-files", "-v", "-z"], env)?;
    let present: BTreeSet<&str> = after
        .split('\0')
        .filter_map(|r| r.split_once(' '))
        .map(|(_, p)| p)
        .collect();
    let (skip_before, assume_before) = parse_entry_flags(before);
    let (skip_after, assume_after) = parse_entry_flags(&after);
    for (flag, was, is) in [
        ("--skip-worktree", skip_before, skip_after),
        ("--assume-unchanged", assume_before, assume_after),
    ] {
        let is: BTreeSet<&str> = is.into_iter().collect();
        let lost: Vec<&str> = was
            .into_iter()
            .filter(|p| !is.contains(p) && present.contains(p))
            .collect();
        // 空列表不发命令（AGENTS.md §6.2）。两个标志分两次发，理由见 clear_entry_flags
        if lost.is_empty() {
            continue;
        }
        let mut stdin = lost.join("\0").into_bytes();
        stdin.push(0);
        run_git_stdin_env(repo, &["update-index", "-z", flag, "--stdin"], &stdin, env)?;
    }
    Ok(())
}
