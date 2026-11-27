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

/// 翻页位置：下一次从 `<peg_path>@<peg_rev>` 的 `next_top` 往下查。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Cursor {
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

/// 把查到的 SVN 提交分到各条基线下（设计 §5.3）。
///
/// 提交 X 归「`file_rev ≥ X` 的基线里 `file_rev` 最小的那条」；`file_rev` 相同的几条归最老的
/// （重建基线可能录出与更早那条相同的 revision，内容是更早那条引入的）。`file_rev` 严格递减时，
/// 这就是设计里的半开区间 (R(i+1), Ri]。比所有基线都新的丢掉（服务器上更新的提交不显示），
/// `hidden`（交付录的 revision，已由「已交付 rN」的本地提交代表）不列，同号只留一条。
pub fn bucket(
    groups: &[GroupSpec],
    fetched: &[SvnLogEntry],
    hidden: &BTreeSet<u64>,
) -> BTreeMap<String, Vec<SvnRevEntry>> {
    let mut out: BTreeMap<String, Vec<SvnRevEntry>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for e in fetched {
        let x = e.revision;
        if hidden.contains(&x) || !seen.insert(x) {
            continue;
        }
        let owner = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| g.file_rev >= x)
            // 先比 file_rev（小的优先），相同时下标大（更老）的优先
            .min_by_key(|(i, g)| (g.file_rev, std::cmp::Reverse(*i)));
        if let Some((_, g)) = owner {
            out.entry(g.id.clone()).or_default().push(e.into());
        }
    }
    out
}

/// 这一页之后从哪儿接着查；`None` 表示全部查完（设计 §5.4）。
///
/// - 满页、最老的还不是 r1：同一个 peg，从「最老 − 1」往下
/// - 否则这一段（这个 peg 能追到的历史）到头了。O = 这一段追到的最老 revision；比当前 peg 那组
///   更老、且 `file_rev < O` 的组是这个 peg 够不着的（git 里改名后交付、删除后重建，SVN 那边复制
///   历史断了），取其中最新的一条作新 peg。只往更老的组找，所以不会选回自己、不会来回跳。
pub fn next_cursor(
    groups: &[GroupSpec],
    cur: &Cursor,
    fetched: &[SvnLogEntry],
    page: usize,
) -> Option<Cursor> {
    let oldest = fetched.iter().map(|e| e.revision).min();
    if fetched.len() >= page {
        if let Some(o) = oldest.filter(|o| *o > 1) {
            return Some(Cursor {
                next_top: o - 1,
                ..cur.clone()
            });
        }
    }
    // 这一页什么都没有时，这一段追到的就是上一页的最老那条（= next_top + 1）
    let reached = oldest.unwrap_or(cur.next_top.saturating_add(1));
    let start = groups
        .iter()
        .rposition(|g| g.path == cur.peg_path && g.file_rev == cur.peg_rev)
        .map_or(0, |i| i + 1);
    groups[start..]
        .iter()
        .find(|g| g.file_rev < reached)
        .map(|g| Cursor {
            peg_path: g.path.clone(),
            peg_rev: g.file_rev,
            next_top: g.file_rev,
        })
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
        Err(e) => Ok(SvnHistoryPage {
            groups: BTreeMap::new(),
            cursor,
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
    let Some(first) = groups.first() else {
        return Ok(empty_page());
    };
    let hidden: BTreeSet<u64> = hidden.iter().copied().collect();
    let mut cur = cursor.unwrap_or_else(|| Cursor {
        peg_path: first.path.clone(),
        peg_rev: first.file_rev,
        next_top: first.file_rev,
    });
    let mut collected: Vec<SvnLogEntry> = Vec::new();
    // 一页里最多换这么多次 peg：每次都只往更老的组走，组数就是上限，这里再兜一层
    for _ in 0..=groups.len() {
        let limit = PAGE - collected.len().min(PAGE - 1);
        let fetched = match fetch_segment(repo, units, &cur, limit) {
            Ok(f) => f,
            Err(e) => {
                return Ok(SvnHistoryPage {
                    groups: bucket(groups, &collected, &hidden),
                    cursor: Some(cur),
                    error: Some(e.into()),
                })
            }
        };
        let next = next_cursor(groups, &cur, &fetched, limit);
        collected.extend(fetched);
        match next {
            Some(n) if collected.len() < PAGE => cur = n,
            next => {
                return Ok(SvnHistoryPage {
                    groups: bucket(groups, &collected, &hidden),
                    cursor: next,
                    error: None,
                })
            }
        }
    }
    Ok(SvnHistoryPage {
        groups: bucket(groups, &collected, &hidden),
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

/// 按 `cur` 查一段：`svn log --xml -r <next_top>:1 -l <limit> -- <url>@<peg_rev>`。
///
/// 路径不在任何白名单单元底下（git 里有、SVN 不管的文件），或者服务器上这个 peg 处没有这个路径，
/// 都当这一段什么都没有，交给 [`next_cursor`] 去看更老的组。
fn fetch_segment(
    repo: &str,
    units: &[WcIdentity],
    cur: &Cursor,
    limit: usize,
) -> Result<Vec<SvnLogEntry>> {
    let Some((i, _)) = crate::engine::svn_wc::unit_of(units, &cur.peg_path) else {
        return Ok(Vec::new());
    };
    let Some(url) = crate::engine::svn_wc::server_url(units, &cur.peg_path) else {
        return Ok(Vec::new());
    };
    let key = format!(
        "log\u{1f}{}\u{1f}{url}\u{1f}{}\u{1f}{}\u{1f}{limit}",
        units[i].uuid, cur.peg_rev, cur.next_top
    );
    if let Some(CacheVal::Log(hit)) = cache_get(&key) {
        return Ok(hit);
    }
    let range = format!("{}:1", cur.next_top);
    let limit_s = limit.to_string();
    let target = format!("{url}@{}", cur.peg_rev);
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

// ── 进程内缓存 ─────────────────────────────────────────────────────────────
//
// 键里都带着 peg revision，结果不会再变，可以一直留到进程退出；总数设上限（设计 §7）。

/// 缓存上限（设计 §7 的取值）。
const CACHE_CAP: usize = 200;

#[derive(Clone)]
enum CacheVal {
    Log(Vec<SvnLogEntry>),
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
    fn cur(path: &str, peg: u64, top: u64) -> Cursor {
        Cursor {
            peg_path: path.to_string(),
            peg_rev: peg,
            next_top: top,
        }
    }

    #[test]
    fn revisions_fall_into_half_open_intervals() {
        let m = bucket(&groups(), &log(&[25, 20, 11, 10, 1]), &BTreeSet::new());
        assert_eq!(revs_of(&m, "B1"), vec![25]);
        assert_eq!(revs_of(&m, "B2"), vec![20, 11]);
        assert_eq!(revs_of(&m, "B3"), vec![10, 1]);
    }

    #[test]
    fn revisions_above_the_newest_baseline_are_dropped() {
        let m = bucket(&groups(), &log(&[31, 25]), &BTreeSet::new());
        assert_eq!(revs_of(&m, "B1"), vec![25]);
        assert!(m.values().flatten().all(|e| e.rev != 31));
    }

    #[test]
    fn delivered_revisions_are_hidden() {
        let m = bucket(&groups(), &log(&[25, 22]), &BTreeSet::from([25]));
        assert_eq!(revs_of(&m, "B1"), vec![22]);
    }

    #[test]
    fn duplicate_revisions_are_kept_once() {
        let m = bucket(&groups(), &log(&[25, 25, 12]), &BTreeSet::new());
        assert_eq!(revs_of(&m, "B1"), vec![25]);
        assert_eq!(revs_of(&m, "B2"), vec![12]);
    }

    #[test]
    fn equal_file_revs_go_to_the_oldest_of_them() {
        // 重建基线可能录出与更早那条相同的 revision：内容是更早那条引入的
        let gs = vec![g("N", 20), g("O", 20), g("I", 10)];
        let m = bucket(&gs, &log(&[20, 15, 10]), &BTreeSet::new());
        assert_eq!(revs_of(&m, "N"), Vec::<u64>::new());
        assert_eq!(revs_of(&m, "O"), vec![20, 15]);
        assert_eq!(revs_of(&m, "I"), vec![10]);
    }

    #[test]
    fn a_full_page_continues_from_the_oldest_minus_one() {
        let fetched = log(&(11..=30).rev().collect::<Vec<_>>());
        let next = next_cursor(&groups(), &cur("Code/B1.cs", 30, 30), &fetched, 20);
        assert_eq!(next, Some(cur("Code/B1.cs", 30, 10)));
    }

    #[test]
    fn a_full_page_ending_at_r1_is_exhausted() {
        let fetched = log(&[2, 1]);
        let next = next_cursor(&groups(), &cur("Code/B3.cs", 10, 2), &fetched, 2);
        assert_eq!(next, None);
    }

    #[test]
    fn an_exhausted_segment_switches_peg_to_the_newest_unreachable_group() {
        // 段从 B1 起，只查到 r15 就结束（O=15）：B3(r10) 够不着 → 新 peg；B2(r20) 视为完整
        let fetched = log(&[25, 15]);
        let next = next_cursor(&groups(), &cur("Code/B1.cs", 30, 30), &fetched, 50);
        assert_eq!(next, Some(cur("Code/B3.cs", 10, 10)));
    }

    #[test]
    fn an_exhausted_segment_with_nothing_left_returns_none() {
        let fetched = log(&[25, 15, 3]);
        let next = next_cursor(&groups(), &cur("Code/B1.cs", 30, 30), &fetched, 50);
        assert_eq!(next, None);
    }

    #[test]
    fn an_empty_segment_does_not_requery_its_own_group() {
        // 这条 peg 一条都没查到：不能又选回自己（死循环），只看比它更老的组
        let next = next_cursor(&groups(), &cur("Code/B3.cs", 10, 10), &[], 50);
        assert_eq!(next, None);
        let next = next_cursor(&groups(), &cur("Code/B2.cs", 20, 20), &[], 50);
        assert_eq!(next, Some(cur("Code/B3.cs", 10, 10)));
    }

    #[test]
    fn an_empty_page_after_full_pages_keeps_what_earlier_pages_reached() {
        // 上一页满 50 条、最老 r15，这一页从 r14 起什么都没有：这一段到 r15 为止
        let next = next_cursor(&groups(), &cur("Code/B1.cs", 30, 14), &[], 50);
        assert_eq!(next, Some(cur("Code/B3.cs", 10, 10)));
    }
}
