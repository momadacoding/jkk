//! 混合仓库的文件历史 / Blame 补全真实 SVN 提交（2026-11-27 设计；计划同名）。
//!
//! git 那一层（`file_history`）照旧先出；这里把其中的基线提交按记账展开成 SVN 提交。
//! 本文件上半是纯函数（分组、翻页、换 peg），下半是去服务器查的编排。

use crate::engine::svn_cli::{run_svn_deadline, SVN_QUERY_TIMEOUT};
use crate::engine::svn_wc::WcIdentity;
use crate::engine::svn_xml::SvnLogEntry;
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

/// 一页最多多少条 SVN 条目，与 TortoiseSVN Show Log 的默认一致（设计 §12）。
pub const PAGE: usize = 50;

/// 历史里的一条基线提交：前端从 `file_history` 的结果里挑出来传回，从新到旧。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GroupSpec {
    /// 基线提交的 git id。
    pub id: String,
    /// 这个文件在该基线里的路径（仓库相对、正斜杠）。
    pub path: String,
    /// 这个文件在该基线里的 revision（`BaseMap::rev_of`）。
    pub file_rev: u64,
}

/// 翻页位置：下一次查第 `group` 组，从 `<peg_path>@<peg_rev>` 的 `next_top` 往下。
///
/// `peg_path` / `peg_rev` 就是那一组的 `path` / `file_rev`，带着是为了日志与核对，查询只认 `group`。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
    pub group: usize,
    pub peg_path: String,
    pub peg_rev: u64,
    pub next_top: u64,
}

/// 一条真实的 SVN 提交。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SvnRevEntry {
    pub rev: u64,
    pub author: String,
    /// ISO8601 原样，前端转时间（与 `SvnLogEntry::date` 同一口径）。
    pub date: String,
    pub message: String,
}

impl From<&SvnLogEntry> for SvnRevEntry {
    fn from(e: &SvnLogEntry) -> Self {
        Self {
            rev: e.revision,
            author: e.author.clone(),
            date: e.date.clone(),
            message: e.message.clone(),
        }
    }
}

// ── 每组用自己的 peg 查自己的区间（设计 §5.3；独立 review 第 C2 条）──────────
//
// 第 i 组的区间是 (R(i+1), Ri]，最老一组到 r1；查询是 `svn log <path_i>@<R_i> -r <top>:<R(i+1)+1>`。
// 不能「沿一个 peg 查下去，再按 revision 号分给各组」：路径被删掉、又用复制补回同名路径时，新 peg 沿
// 复制历史查到的是**另一个文件**的提交，号码却落在旧基线的区间里，会被算到旧文件头上（2026-11-28
// 实测，`a_path_replaced_by_a_copy_keeps_each_baseline_on_its_own_lineage`）。每组只认自己 peg 的
// 那条血缘，按构造就不会串；改名后交付、删除重建也不再需要猜什么时候换 peg。

/// 第 `i` 组区间的下界（不含）：更老一组的 `file_rev`，最老一组为 0。
fn lower_of(groups: &[GroupSpec], i: usize) -> u64 {
    groups.get(i + 1).map_or(0, |g| g.file_rev)
}

/// 从第 `i` 组开始查的游标。
fn group_cursor(groups: &[GroupSpec], i: usize) -> Option<Cursor> {
    groups.get(i).map(|g| Cursor {
        group: i,
        peg_path: g.path.clone(),
        peg_rev: g.file_rev,
        next_top: g.file_rev,
    })
}

/// 第一页的游标：从最新那组开始。
pub fn start_cursor(groups: &[GroupSpec]) -> Option<Cursor> {
    group_cursor(groups, 0)
}

/// 这一次要查的 revision 区间 `(top, low)`，两端都含；区间为空（相邻两组 `file_rev` 相同、
/// 或更老一组反而更大）时为 `None`，这一组什么都不归它。
pub fn segment_range(groups: &[GroupSpec], cur: &Cursor) -> Option<(u64, u64)> {
    let g = groups.get(cur.group)?;
    let top = cur.next_top.min(g.file_rev);
    let low = lower_of(groups, cur.group) + 1;
    (top >= low).then_some((top, low))
}

/// 这一页之后从哪儿接着查；`None` 表示全部查完（设计 §5.4）。
///
/// 查满了 `limit` 条、且这一组的区间还没到底：同一组，从「最老 − 1」往下。否则这一组查完了，
/// 换下一组（更老的那条基线）。
pub fn next_cursor(
    groups: &[GroupSpec],
    cur: &Cursor,
    fetched: &[SvnLogEntry],
    limit: usize,
) -> Option<Cursor> {
    let low = lower_of(groups, cur.group) + 1;
    if fetched.len() >= limit {
        if let Some(o) = fetched
            .iter()
            .map(|e| e.revision)
            .min()
            .filter(|o| *o > low)
        {
            return Some(Cursor {
                next_top: o - 1,
                ..cur.clone()
            });
        }
    }
    group_cursor(groups, cur.group + 1)
}

/// 把一页查到的提交装进各组：`hidden`（交付录的 revision，已由「已交付 rN」的本地提交代表）不列，
/// 同号只留一条。
pub fn assemble(
    groups: &[GroupSpec],
    collected: &[(usize, SvnLogEntry)],
    hidden: &BTreeSet<u64>,
) -> BTreeMap<String, Vec<SvnRevEntry>> {
    let mut out: BTreeMap<String, Vec<SvnRevEntry>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for (i, e) in collected {
        if hidden.contains(&e.revision) || !seen.insert(e.revision) {
            continue;
        }
        if let Some(g) = groups.get(*i) {
            out.entry(g.id.clone()).or_default().push(e.into());
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// 去服务器查
// ─────────────────────────────────────────────────────────────────────────

/// SVN 那边没查成：不当成命令失败（git 那层照常显示），放进结果里让前端说明原因。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SvnFetchError {
    /// `svn_unreachable` / `svn_auth` / `svn_path_missing` / `svn_cli`（`svn_cli::classify_svn_error`）。
    pub code: String,
    pub message: String,
}

impl From<GitError> for SvnFetchError {
    fn from(e: GitError) -> Self {
        Self {
            code: e.code,
            message: e.message,
        }
    }
}

/// 一页 SVN 补全。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SvnHistoryPage {
    /// 基线提交 id → 这一页里归它的 SVN 提交（从新到旧）。这一页没分到的组不出现。
    pub groups: BTreeMap<String, Vec<SvnRevEntry>>,
    /// 下一页从哪儿查；`None` 是全部查完。出错时保留出错那一刻的位置，前端可以重试。
    pub cursor: Option<Cursor>,
    pub error: Option<SvnFetchError>,
}

/// 取一页（设计 §5.4）。`groups` 从新到旧；`hidden` 是已由本地提交代表的交付 revision。
///
/// 没有基线组时直接返回空页，一条 svn 命令都不发（这个文件从没进过基线）。SVN 出错不返回
/// `Err`：已经拿到的分组照常给，原因放进 `error`。
pub fn svn_file_history(
    repo: &str,
    groups: &[GroupSpec],
    hidden: &[u64],
    cursor: Option<Cursor>,
) -> Result<SvnHistoryPage> {
    check_groups(groups)?;
    if groups.is_empty() {
        return Ok(empty_page());
    }
    match crate::engine::svn_wc::units_cached(repo) {
        Ok(units) => page_with_units(repo, &units, groups, hidden, cursor),
        // 第一页就失败时也给一个起点游标：前端据此显示「没取到」和重试，而不是「0 次」（review 第 I1 条）
        Err(e) => Ok(SvnHistoryPage {
            groups: BTreeMap::new(),
            cursor: cursor.or_else(|| start_cursor(groups)),
            error: Some(e.into()),
        }),
    }
}

/// [`svn_file_history`] 的本体，单元由调用方给（测试用它造一个连不上的单元）。
pub fn page_with_units(
    repo: &str,
    units: &[WcIdentity],
    groups: &[GroupSpec],
    hidden: &[u64],
    cursor: Option<Cursor>,
) -> Result<SvnHistoryPage> {
    check_groups(groups)?;
    let Some(start) = start_cursor(groups) else {
        return Ok(empty_page());
    };
    let mut cur = cursor.unwrap_or(start);
    if cur.group >= groups.len() {
        return Err(GitError::new(
            "bad_history_args",
            format!("翻页位置第 {} 组超出了 {} 组", cur.group, groups.len()),
        ));
    }
    let hidden: BTreeSet<u64> = hidden.iter().copied().collect();
    let mut collected: Vec<(usize, SvnLogEntry)> = Vec::new();
    // 每组最多查一次就换下一组（满页才停），组数加一就是一页里循环的上限
    for _ in 0..=groups.len() {
        let limit = PAGE - collected.len().min(PAGE - 1);
        let fetched = match fetch_segment(repo, units, groups, &cur, limit) {
            Ok(f) => f,
            Err(e) => {
                return Ok(SvnHistoryPage {
                    groups: assemble(groups, &collected, &hidden),
                    cursor: Some(cur),
                    error: Some(e.into()),
                })
            }
        };
        let next = next_cursor(groups, &cur, &fetched, limit);
        collected.extend(fetched.into_iter().map(|e| (cur.group, e)));
        match next {
            Some(n) if collected.len() < PAGE => cur = n,
            next => {
                return Ok(SvnHistoryPage {
                    groups: assemble(groups, &collected, &hidden),
                    cursor: next,
                    error: None,
                })
            }
        }
    }
    Ok(SvnHistoryPage {
        groups: assemble(groups, &collected, &hidden),
        cursor: None,
        error: None,
    })
}

fn empty_page() -> SvnHistoryPage {
    SvnHistoryPage {
        groups: BTreeMap::new(),
        cursor: None,
        error: None,
    }
}

fn check_groups(groups: &[GroupSpec]) -> Result<()> {
    for g in groups {
        // 空路径在 URL 里会变成单元根：查出整个目录的历史，而不是这个文件的
        if g.id.is_empty() || g.path.is_empty() || g.file_rev == 0 {
            return Err(GitError::new(
                "bad_history_args",
                format!("基线组参数不完整：{g:?}"),
            ));
        }
    }
    Ok(())
}

/// 按 `cur` 查第 `cur.group` 组的一段：`svn log --xml -r <top>:<low> -l <limit> -- <url>@<peg>`，
/// peg 与路径都取那一组自己的（[`segment_range`]）。
///
/// 区间为空、路径不在任何白名单单元底下（git 里有、SVN 不管的文件），或者区间落在这个节点创建之前
/// （`svn_path_missing`，含 E195012），都当这一组这一段什么都没有，交给 [`next_cursor`] 换下一组。
fn fetch_segment(
    repo: &str,
    units: &[WcIdentity],
    groups: &[GroupSpec],
    cur: &Cursor,
    limit: usize,
) -> Result<Vec<SvnLogEntry>> {
    let Some((top, low)) = segment_range(groups, cur) else {
        return Ok(Vec::new());
    };
    let g = &groups[cur.group];
    let Some((i, _)) = crate::engine::svn_wc::unit_of(units, &g.path) else {
        return Ok(Vec::new());
    };
    let Some(url) = crate::engine::svn_wc::server_url(units, &g.path) else {
        return Ok(Vec::new());
    };
    let key = format!(
        "log\u{1f}{}\u{1f}{url}\u{1f}{}\u{1f}{top}\u{1f}{low}\u{1f}{limit}",
        units[i].uuid, g.file_rev
    );
    if let Some(CacheVal::Log(hit)) = cache_get(&key) {
        return Ok(hit);
    }
    let range = format!("{top}:{low}");
    let limit_s = limit.to_string();
    let target = format!("{url}@{}", g.file_rev);
    let xml = match run_svn_deadline(
        repo,
        &["log", "--xml", "-r", &range, "-l", &limit_s, "--", &target],
        SVN_QUERY_TIMEOUT,
    ) {
        Ok(x) => x,
        Err(e) if e.code == "svn_path_missing" => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let entries = crate::engine::svn_xml::parse_log(&xml)?;
    cache_put(key, CacheVal::Log(entries.clone()));
    Ok(entries)
}

/// 一条 SVN 提交里这个文件的改动（设计 §5.6）。
///
/// `peg_path@peg_rev` 是这条提交所在那组的 peg（前端从组里取）：`svn cat -r <rev> <url>@<peg>`
/// 的操作版本从 peg 往回追，跟得上复制 / 改名。`rev - 1` 处还没有这个文件（新增）时旧侧为空。
///
/// diff 用 libgit2 算，输出与 git 提交的单文件 diff 同形、同一套 `DiffViewOptions`。两份内容只写进
/// 一个挂在这次打开的仓库对象上的内存对象库（mempack），**不落 `.git/objects`**（设计 §2 反目标，
/// `no_object_is_written_to_the_users_git` 实测）。
pub fn svn_rev_file_diff(
    repo: &str,
    peg_path: &str,
    peg_rev: u64,
    rev: u64,
    view: &crate::engine::diff::DiffViewOptions,
) -> Result<crate::engine::diff::DiffData> {
    if peg_path.is_empty() || rev == 0 || peg_rev < rev {
        return Err(GitError::new(
            "bad_history_args",
            format!("SVN diff 参数不对：{peg_path}@{peg_rev} -c {rev}"),
        ));
    }
    let units = crate::engine::svn_wc::units_cached(repo)?;
    let url = crate::engine::svn_wc::server_url(&units, peg_path).ok_or_else(|| {
        GitError::new(
            "svn_path_missing",
            format!("{peg_path} 不在任何 SVN 白名单目录底下"),
        )
    })?;
    let target = format!("{url}@{peg_rev}");
    let uuid = unit_uuid(&units, peg_path);
    let new = svn_cat(repo, uuid, &target, rev)?;
    let old = if rev == 1 {
        None
    } else {
        match svn_cat(repo, uuid, &target, rev - 1) {
            Ok(b) => Some(b),
            // 上一个版本还没有这个文件：这一条是新增
            Err(e) if e.code == "svn_path_missing" => None,
            Err(e) => return Err(e),
        }
    };
    let r = git2::Repository::open(repo)?;
    // 优先级高于默认的 loose（1）/ packed（2）后端：写入都落到内存里
    r.odb()?.add_new_mempack_backend(1000)?;
    let new_tree = tree_with_file(&r, peg_path, &new)?;
    let old_tree = old
        .as_deref()
        .map(|b| tree_with_file(&r, peg_path, b))
        .transpose()?;
    let old_tree = old_tree.map(|o| r.find_tree(o)).transpose()?;
    let new_tree = r.find_tree(new_tree)?;
    let mut opts = view.options();
    opts.pathspec(peg_path);
    opts.disable_pathspec_match(true);
    let diff = r.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), Some(&mut opts))?;
    crate::engine::diff::diff_to_data_only(diff, Some(peg_path))
}

/// 路径所在单元的仓库 UUID，缓存键用：同一个 URL 在进程存活期间被重建成另一个仓库时不串。
fn unit_uuid<'a>(units: &'a [WcIdentity], path: &str) -> &'a str {
    crate::engine::svn_wc::unit_of(units, path).map_or("", |(i, _)| units[i].uuid.as_str())
}

fn svn_cat(repo: &str, uuid: &str, target: &str, rev: u64) -> Result<Vec<u8>> {
    let key = format!("cat\u{1f}{uuid}\u{1f}{target}\u{1f}{rev}");
    if let Some(CacheVal::Bytes(hit)) = cache_get(&key) {
        return Ok(hit);
    }
    let r = rev.to_string();
    let out = crate::engine::svn_cli::run_svn_deadline_bytes(
        repo,
        &["cat", "-r", &r, "--", target],
        SVN_QUERY_TIMEOUT,
    )?;
    cache_put(key, CacheVal::Bytes(out.clone()));
    Ok(out)
}

/// 建一棵只含 `path` 这一个文件的树（逐层建目录），返回根树的 id。
fn tree_with_file(r: &git2::Repository, path: &str, bytes: &[u8]) -> Result<git2::Oid> {
    let blob = r.blob(bytes)?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let Some((name, dirs)) = parts.split_last() else {
        return Err(GitError::new("bad_history_args", "空路径"));
    };
    let mut b = r.treebuilder(None)?;
    b.insert(name, blob, 0o100644)?;
    let mut child = b.write()?;
    for d in dirs.iter().rev() {
        let mut b = r.treebuilder(None)?;
        b.insert(d, child, 0o040000)?;
        child = b.write()?;
    }
    Ok(child)
}

/// 一次 Blame 最多映射几个 (基线, 路径)（设计 §6.4 的取值）。
pub const MAX_OVERLAY_TARGETS: usize = 8;

/// Blame 里归到某条基线、某个路径的那些行，要去 SVN 查出处。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OverlayTarget {
    pub baseline_id: String,
    /// 这些行在那条基线里所在文件的路径（`BlameLine::orig_path`）。
    pub orig_path: String,
    /// 该路径在那条基线里的 revision（`BlameCommit::baseline.file_rev`）。
    pub file_rev: u64,
}

/// 一个 target 的结果，与传入的 targets 同序。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum OverlayResult {
    /// 第 i 项是那条基线里第 i+1 行的出处（`BlameLine::orig_line` 从 1 起）。
    Mapped {
        lines: Vec<Option<crate::engine::svn_xml::BlameRevAuthor>>,
    },
    /// 行数对不上：不映射，这些行保持「基线 rN」，免得把行归给错的人（设计 §2 反目标、§6.4）。
    #[serde(rename_all = "camelCase")]
    Mismatch {
        svn_lines: usize,
        git_lines: usize,
    },
    Failed {
        error: SvnFetchError,
    },
    /// 超过 [`MAX_OVERLAY_TARGETS`]，没有查。
    Skipped,
}

/// Blame 归到基线的行 → `svn blame` 的作者和版本（设计 §6.2）。
pub fn svn_blame_overlay(repo: &str, targets: &[OverlayTarget]) -> Result<Vec<OverlayResult>> {
    check_targets(targets)?;
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    match crate::engine::svn_wc::units_cached(repo) {
        Ok(units) => overlay_with_units(repo, &units, targets),
        Err(e) => {
            let err: SvnFetchError = e.into();
            Ok(targets
                .iter()
                .map(|_| OverlayResult::Failed { error: err.clone() })
                .collect())
        }
    }
}

/// [`svn_blame_overlay`] 的本体，单元由调用方给（测试用它造一个连不上的单元）。
pub fn overlay_with_units(
    repo: &str,
    units: &[WcIdentity],
    targets: &[OverlayTarget],
) -> Result<Vec<OverlayResult>> {
    check_targets(targets)?;
    let r = git2::Repository::open(repo)?;
    let mut out = Vec::with_capacity(targets.len());
    for (i, t) in targets.iter().enumerate() {
        if i >= MAX_OVERLAY_TARGETS {
            out.push(OverlayResult::Skipped);
            continue;
        }
        out.push(match overlay_one(repo, &r, units, t) {
            Ok(x) => x,
            Err(e) => OverlayResult::Failed { error: e.into() },
        });
    }
    Ok(out)
}

fn check_targets(targets: &[OverlayTarget]) -> Result<()> {
    for t in targets {
        if t.baseline_id.is_empty() || t.orig_path.is_empty() || t.file_rev == 0 {
            return Err(GitError::new(
                "bad_history_args",
                format!("Blame 映射参数不完整：{t:?}"),
            ));
        }
    }
    Ok(())
}

fn overlay_one(
    repo: &str,
    r: &git2::Repository,
    units: &[WcIdentity],
    t: &OverlayTarget,
) -> Result<OverlayResult> {
    let url = crate::engine::svn_wc::server_url(units, &t.orig_path).ok_or_else(|| {
        GitError::new(
            "svn_path_missing",
            format!("{} 不在任何 SVN 白名单目录底下", t.orig_path),
        )
    })?;
    let target = format!("{url}@{}", t.file_rev);
    let key = format!(
        "blame\u{1f}{}\u{1f}{target}",
        unit_uuid(units, &t.orig_path)
    );
    let lines = match cache_get(&key) {
        Some(CacheVal::Blame(hit)) => hit,
        _ => {
            let xml =
                run_svn_deadline(repo, &["blame", "--xml", "--", &target], SVN_QUERY_TIMEOUT)?;
            let lines = crate::engine::svn_xml::parse_blame(&xml)?;
            cache_put(key, CacheVal::Blame(lines.clone()));
            lines
        }
    };
    let git_lines = baseline_line_count(r, &t.baseline_id, &t.orig_path)?;
    if lines.len() != git_lines {
        return Ok(OverlayResult::Mismatch {
            svn_lines: lines.len(),
            git_lines,
        });
    }
    Ok(OverlayResult::Mapped { lines })
}

/// 基线提交里这个文件的行数，口径与 `git blame` 一致：最后一行没有换行符也算一行。
fn baseline_line_count(r: &git2::Repository, id: &str, path: &str) -> Result<usize> {
    let commit = r.find_commit(git2::Oid::from_str(id)?)?;
    let entry = commit.tree()?.get_path(std::path::Path::new(path))?;
    let blob = r.find_blob(entry.id())?;
    let bytes = blob.content();
    let n = bytes.iter().filter(|b| **b == b'\n').count();
    Ok(if bytes.last().is_some_and(|b| *b != b'\n') {
        n + 1
    } else {
        n
    })
}

// ── 进程内缓存 ─────────────────────────────────────────────────────────────
//
// 键里都带着 peg revision，结果不会再变，可以一直留到进程退出；总数设上限（设计 §7）。

/// 缓存上限（设计 §7 的取值）。
const CACHE_CAP: usize = 200;

#[derive(Clone)]
enum CacheVal {
    Log(Vec<SvnLogEntry>),
    Bytes(Vec<u8>),
    Blame(Vec<Option<crate::engine::svn_xml::BlameRevAuthor>>),
}

#[derive(Default)]
struct Lru {
    map: HashMap<String, (u64, CacheVal)>,
    tick: u64,
}

fn cache() -> &'static Mutex<Lru> {
    static C: OnceLock<Mutex<Lru>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn cache_get(key: &str) -> Option<CacheVal> {
    let mut c = cache().lock().ok()?;
    c.tick += 1;
    let t = c.tick;
    let hit = c.map.get_mut(key)?;
    hit.0 = t;
    Some(hit.1.clone())
}

fn cache_put(key: String, val: CacheVal) {
    let Ok(mut c) = cache().lock() else {
        return;
    };
    c.tick += 1;
    let t = c.tick;
    if c.map.len() >= CACHE_CAP && !c.map.contains_key(&key) {
        if let Some(old) = c
            .map
            .iter()
            .min_by_key(|(_, v)| v.0)
            .map(|(k, _)| k.clone())
        {
            c.map.remove(&old);
        }
    }
    c.map.insert(key, (t, val));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_evicts_the_least_recently_used() {
        for i in 0..CACHE_CAP {
            cache_put(format!("lru-test-{i}"), CacheVal::Log(Vec::new()));
        }
        // 碰一下第 0 条，它就不是最久没用的了
        assert!(cache_get("lru-test-0").is_some());
        cache_put("lru-test-new".to_string(), CacheVal::Log(Vec::new()));
        assert!(cache_get("lru-test-0").is_some());
        assert!(cache().lock().unwrap().map.len() <= CACHE_CAP);
    }

    fn g(id: &str, rev: u64) -> GroupSpec {
        GroupSpec {
            id: id.to_string(),
            path: format!("Code/{id}.cs"),
            file_rev: rev,
        }
    }
    /// B1(r30) B2(r20) B3(r10，初始化)
    fn groups() -> Vec<GroupSpec> {
        vec![g("B1", 30), g("B2", 20), g("B3", 10)]
    }
    fn log(revs: &[u64]) -> Vec<SvnLogEntry> {
        revs.iter()
            .map(|r| SvnLogEntry {
                revision: *r,
                author: format!("u{r}"),
                date: "2026-11-20T00:00:00.000000Z".to_string(),
                message: format!("m{r}"),
            })
            .collect()
    }
    fn revs_of(m: &BTreeMap<String, Vec<SvnRevEntry>>, id: &str) -> Vec<u64> {
        m.get(id)
            .map(|v| v.iter().map(|e| e.rev).collect())
            .unwrap_or_default()
    }
    /// 第 `group` 组、从 `top` 往下的游标（peg 取那一组自己的）。
    fn cur(gs: &[GroupSpec], group: usize, top: u64) -> Cursor {
        Cursor {
            group,
            peg_path: gs[group].path.clone(),
            peg_rev: gs[group].file_rev,
            next_top: top,
        }
    }

    #[test]
    fn each_group_queries_its_own_half_open_interval() {
        let gs = groups();
        assert_eq!(segment_range(&gs, &cur(&gs, 0, 30)), Some((30, 21)));
        assert_eq!(segment_range(&gs, &cur(&gs, 1, 20)), Some((20, 11)));
        // 最老一组一直查到 r1
        assert_eq!(segment_range(&gs, &cur(&gs, 2, 10)), Some((10, 1)));
        assert_eq!(start_cursor(&gs), Some(cur(&gs, 0, 30)));
    }

    #[test]
    fn equal_file_revs_give_the_newer_group_an_empty_range() {
        // 重建基线可能录出与更早那条相同的 revision：内容是更早那条引入的
        let gs = vec![g("N", 20), g("O", 20), g("I", 10)];
        assert_eq!(segment_range(&gs, &cur(&gs, 0, 20)), None);
        assert_eq!(segment_range(&gs, &cur(&gs, 1, 20)), Some((20, 11)));
        // 更老一组反而更大（非单调）：同样为空，不发查询
        let gs = vec![g("A", 10), g("B", 20)];
        assert_eq!(segment_range(&gs, &cur(&gs, 0, 10)), None);
    }

    #[test]
    fn a_full_page_continues_in_the_same_group_from_the_oldest_minus_one() {
        let gs = groups();
        let fetched = log(&(25..=30).rev().collect::<Vec<_>>());
        let next = next_cursor(&gs, &cur(&gs, 0, 30), &fetched, 6);
        assert_eq!(next, Some(cur(&gs, 0, 24)));
    }

    #[test]
    fn a_full_page_reaching_the_lower_bound_moves_to_the_next_group() {
        let gs = groups();
        let fetched = log(&(21..=30).rev().collect::<Vec<_>>());
        let next = next_cursor(&gs, &cur(&gs, 0, 30), &fetched, 10);
        assert_eq!(next, Some(cur(&gs, 1, 20)));
    }

    #[test]
    fn a_short_page_moves_to_the_next_group() {
        let gs = groups();
        assert_eq!(
            next_cursor(&gs, &cur(&gs, 0, 30), &log(&[25]), 50),
            Some(cur(&gs, 1, 20))
        );
        // 这一组一条都没有（区间落在创建之前、路径不在单元里）也一样往下走，不会选回自己
        assert_eq!(
            next_cursor(&gs, &cur(&gs, 1, 20), &[], 50),
            Some(cur(&gs, 2, 10))
        );
    }

    #[test]
    fn the_last_group_exhausted_returns_none() {
        let gs = groups();
        assert_eq!(next_cursor(&gs, &cur(&gs, 2, 10), &log(&[5, 1]), 50), None);
        // 满页但已经到 r1，也不再查 `-r 0:1`
        assert_eq!(next_cursor(&gs, &cur(&gs, 2, 2), &log(&[2, 1]), 2), None);
    }

    #[test]
    fn delivered_revisions_are_hidden_and_duplicates_kept_once() {
        let gs = groups();
        let collected: Vec<(usize, SvnLogEntry)> = log(&[25, 25, 22])
            .into_iter()
            .map(|e| (0, e))
            .chain(log(&[12]).into_iter().map(|e| (1, e)))
            .collect();
        let m = assemble(&gs, &collected, &BTreeSet::from([22]));
        assert_eq!(revs_of(&m, "B1"), vec![25]);
        assert_eq!(revs_of(&m, "B2"), vec![12]);
    }
}
