//! 只读查询 svn 工作副本的 `wc.db`。
//!
//! # 为什么不用 svn 命令
//!
//! 跟进的体检要找出被 `--set-depth=exclude` 掉的节点，原来用 `svn info -R --xml`：它把白名单
//! 底下每个节点的信息都打一遍，2 万文件的 fixture 上要 0.75–1.9 s，而且与体检的 `svn diff`
//! 抢同一个 `wc.db` 的锁（svn 的只读命令在同一工作副本上基本串行，见
//! `docs/superpowers/plans/2026-11-14-follow-overlap-decisions.md`）。被排除的节点在 `wc.db`
//! 里就是 `NODES.presence = 'excluded'` 的那几行，查一次几十毫秒。
//!
//! # 依据
//!
//! Subversion 没有对外的表结构文档。这里用到的表、列与取值只来自它源码里的
//! `subversion/libsvn_wc/wc-metadata.sql`（格式 31 的建表语句与列注释）与 `wc.h`（格式号
//! 对照：1.8.x–1.14.x 是 31；1.15.x 引入 32，默认仍是 31）。结果是否与 svn 相同由
//! `tests/svn_wcdb.rs` 逐个场景与 `svn info -R --xml` 比对。
//!
//! # 认不出来就不读
//!
//! 下面任何一条不满足都返回 `Err`（说明原因），调用方照旧跑 svn：
//!
//! - 格式号（`PRAGMA user_version`）不是 31；
//! - `WORK_QUEUE` 里有没做完的活（操作被打断，此刻 `wc.db` 与磁盘可能不一致）。只读的
//!   `svn info` 并不因此拒绝，这一条是我们自己保守：那时交给 svn，由它照原来的方式给结果；
//! - 单元没有 `wcroot-abspath`、不在它的工作副本根底下、或它的根在 `NODES` 里查不到；
//! - 出现了 `file` / `dir` 以外的 kind；
//! - 打不开、被锁住超时、或任何一条查询失败。
//!
//! 打开方式是只读（`SQLITE_OPEN_READ_ONLY`），不建 journal，不改 `wc.db` 的任何字节。

use crate::engine::perf;
use crate::engine::svn_tombstone::fault::{self, Point};
use crate::engine::svn_wc::WcIdentity;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

/// 认得的唯一格式号（`wc.h`：1.8.x 到 1.14.x，以及 1.15.x 的默认格式）。
const FORMAT: i64 = 31;

/// 与 svn 自己的 busy timeout 相同（libsvn_subr/sqlite.c 的 `BUSY_TIMEOUT`，10 秒）：svn 正在
/// 写的时候，`svn info -R` 也是等这么久。
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// 每个单元底下被排除的节点：仓库相对路径（正斜杠）→ kind（`file` / `dir`）。
///
/// 与 `svn info -R --xml -- <各单元>@` 再挑 `<depth>exclude</depth>` 的结果相同
/// （`svn_xml::parse_excluded_nodes`）。`repo_root` 是单元路径的基准（svn 命令的 cwd）。
pub fn excluded_nodes(
    repo_root: &str,
    units: &[WcIdentity],
) -> std::result::Result<BTreeMap<String, String>, String> {
    if fault::hit(Point::WcdbRefused).is_err() {
        return Err("测试注入：认不出这个工作副本".to_string());
    }
    if let Some(earlier) = take_adopted(repo_root, units) {
        return earlier;
    }
    query_all(repo_root, units)
}

/// 提前查好的一份（[`probe`]），由 [`Probe::adopt`] 交给同一线程上随后的 [`excluded_nodes`]。
pub struct Probe {
    repo_root: String,
    units: Vec<WcIdentity>,
    result: std::result::Result<BTreeMap<String, String>, String>,
}

/// [`Probe::adopt`] 的守卫：drop 时没被取走的那份作废。
#[must_use = "守卫一 drop，提前查好的结果就作废"]
pub struct Adopted(());

thread_local! {
    static ADOPTED: std::cell::RefCell<Option<Probe>> = const { std::cell::RefCell::new(None) };
}

impl Probe {
    /// 结果能不能用（`Ok`）。
    pub fn is_ok(&self) -> bool {
        self.result.is_ok()
    }

    /// 交给当前线程上随后的 [`excluded_nodes`]：同一个 `repo_root` 与单元列表时直接拿这份
    /// （成功的结果，或「认不出来」的结论），只拿一次。
    pub fn adopt(self) -> Adopted {
        ADOPTED.with(|a| *a.borrow_mut() = Some(self));
        Adopted(())
    }
}

impl Drop for Adopted {
    fn drop(&mut self) {
        ADOPTED.with(|a| a.borrow_mut().take());
    }
}

fn take_adopted(
    repo_root: &str,
    units: &[WcIdentity],
) -> Option<std::result::Result<BTreeMap<String, String>, String>> {
    ADOPTED.with(|a| {
        let mut slot = a.borrow_mut();
        let fits = slot
            .as_ref()
            .is_some_and(|p| p.repo_root == repo_root && p.units == units);
        fits.then(|| slot.take().map(|p| p.result)).flatten()
    })
}

/// 现在就查一次，结果留着交给随后的体检（[`Probe::adopt`]）。
///
/// 跟进在磁盘录完（`git add -A`）之后调用它，与原来预跑 `svn info -R` 是同一个时刻：两者
/// 之间发生的排除，体检原来也看不到，由纳入集合核对与下一次检测兜住。查一次就够，不让
/// 体检再查（再查一次也可能撞上同一把锁、再等一轮 busy timeout）。
pub fn probe(repo_root: &str, units: &[WcIdentity]) -> Probe {
    let result = if fault::hit(Point::WcdbRefused).is_err() {
        Err("测试注入：认不出这个工作副本".to_string())
    } else {
        query_all(repo_root, units)
    };
    Probe {
        repo_root: repo_root.to_string(),
        units: units.to_vec(),
        result,
    }
}

fn query_all(
    repo_root: &str,
    units: &[WcIdentity],
) -> std::result::Result<BTreeMap<String, String>, String> {
    let started = perf::enabled().then(perf::now_ms);
    let mut out = BTreeMap::new();
    for u in units {
        for (p, kind) in excluded_in_unit(repo_root, u)? {
            out.insert(p, kind);
        }
    }
    if let Some(ts) = started {
        let mut rec = perf::Record::rust("query", "wc.db excluded nodes");
        rec.ts = ts;
        rec.dur = perf::now_ms() - ts;
        perf::record(&rec);
    }
    Ok(out)
}

fn excluded_in_unit(
    repo_root: &str,
    u: &WcIdentity,
) -> std::result::Result<Vec<(String, String)>, String> {
    let wc_root = u
        .wc_root_abspath
        .as_deref()
        .ok_or_else(|| format!("`{}` 没有工作副本根", u.wc_root))?;
    let unit_abs = format!("{}/{}", norm(repo_root), norm(&u.wc_root));
    let sub = within(&unit_abs, &norm(wc_root))
        .ok_or_else(|| format!("`{}` 不在工作副本根 `{wc_root}` 底下", u.wc_root))?;

    let db = open(&Path::new(wc_root).join(".svn").join("wc.db"))?;
    let rows = query(&db, &sub).map_err(|e| format!("查询 wc.db 失败：{e}"))?;
    let rows = rows?;
    Ok(rows
        .into_iter()
        .map(|(relpath, kind)| (repo_relative(&u.wc_root, &sub, &relpath), kind))
        .collect())
}

fn open(path: &Path) -> std::result::Result<Connection, String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let db = Connection::open_with_flags(path, flags)
        .map_err(|e| format!("打不开 `{}`：{e}", path.display()))?;
    db.busy_timeout(BUSY_TIMEOUT)
        .map_err(|e| format!("设不了 busy timeout：{e}"))?;
    Ok(db)
}

/// 外层 `Err` 是 SQLite 报的错，内层 `Err` 是「认不出来」。
type Rows = std::result::Result<Vec<(String, String)>, String>;

fn query(db: &Connection, sub: &str) -> rusqlite::Result<Rows> {
    // 几条查询在同一个读事务里，看到的是同一时刻的 wc.db
    let tx = db.unchecked_transaction()?;

    let format: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if format != FORMAT {
        return Ok(Err(format!("工作副本格式是 {format}，只认 {FORMAT}")));
    }
    let pending: Option<i64> = tx
        .query_row("SELECT 1 FROM work_queue LIMIT 1", [], |r| r.get(0))
        .optional()?;
    if pending.is_some() {
        return Ok(Err("有没做完的操作（WORK_QUEUE 非空）".to_string()));
    }
    // 工作副本根那一行的 local_abspath 是 NULL（wc-metadata.sql 的 WCROOT 注释）
    let ids: Vec<i64> = tx
        .prepare("SELECT id FROM wcroot WHERE local_abspath IS NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let [wc_id] = ids[..] else {
        return Ok(Err(format!("WCROOT 里有 {} 个根", ids.len())));
    };

    // 每个路径取 op_depth 最大的那一层：那是这个节点此刻的样子（BASE 是 0，WORKING 层更大，
    // wc-metadata.sql 的 NODES 注释）。svn info 报的也是这一层
    const TOP: &str = "op_depth = (SELECT MAX(op_depth) FROM nodes m \
                       WHERE m.wc_id = n.wc_id AND m.local_relpath = n.local_relpath)";

    let root: Option<String> = tx
        .query_row(
            &format!(
                "SELECT presence FROM nodes n WHERE wc_id = ?1 AND local_relpath = ?2 AND {TOP}"
            ),
            rusqlite::params![wc_id, sub],
            |r| r.get(0),
        )
        .optional()?;
    if root.as_deref() != Some("normal") {
        return Ok(Err(format!(
            "单元的根 `{sub}` 在 wc.db 里是 {}",
            root.as_deref().unwrap_or("（查不到）")
        )));
    }

    // 子树用区间而不是 LIKE：LIKE 不区分大小写，还要转义 `_` `%`。`/` 是 0x2F、`0` 是 0x30，
    // `sub/` 到 `sub0` 之间正好是 `sub/` 开头的全部路径（local_relpath 按字节比较）
    let sql = format!(
        "SELECT local_relpath, kind FROM nodes n \
         WHERE wc_id = ?1 AND presence = 'excluded' AND {TOP} \
         AND (?2 = '' OR local_relpath = ?2 OR (local_relpath >= ?2 || '/' AND local_relpath < ?2 || '0')) \
         ORDER BY local_relpath"
    );
    let found: Vec<(String, String)> = tx
        .prepare(&sql)?
        .query_map(rusqlite::params![wc_id, sub], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    if let Some((p, k)) = found.iter().find(|(_, k)| k != "file" && k != "dir") {
        return Ok(Err(format!("`{p}` 的 kind 是 {k}")));
    }
    Ok(Ok(found))
}

/// 一个单元在 wc.db 里的形状，决定体检的 floor 比对能不能由工作副本自己回答
/// （`svn_health` 的本地 floor，日志 `2026-11-17-floor-check-by-status-decisions.md`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FloorShape {
    /// 每个节点都满足 `changed_revision ≤ F ≤ revision`（`stale` 里的文件除外），没有 switch、
    /// 稀疏检出、not-present、本地 add/delete/copy/move、冲突、file external、未完成的操作。
    /// 这时 `svn diff --old=<url>@F --new=<单元>` 的结论只取决于本地修改。
    Regular {
        /// 单元底下的目录（仓库相对）。status 不给 kind，体检要剔掉目录条目时用。
        dirs: BTreeSet<String>,
        /// 不满足 `c ≤ F ≤ n` 的**文件**（仓库相对）。F 上的内容可能与 pristine 不同，diff 会
        /// 向服务器要它们。调用方确认它们都另有归属（映射单列，或不是 W-file）才能用本地结论。
        stale: Vec<String>,
    },
    /// 必须问服务器。字符串说明为什么（写进日志与性能打点）。
    Irregular(String),
}

/// 每个单元的 [`FloorShape`]，键是 `WcIdentity::wc_root`。外层 `Err` 是 wc.db 读不了或认不出
/// （格式、未完成的操作……，同 [`excluded_nodes`]），调用方全部照旧跑 diff。
pub fn floor_shapes(
    repo_root: &str,
    units: &[WcIdentity],
    floor: u64,
) -> std::result::Result<BTreeMap<String, FloorShape>, String> {
    if fault::hit(Point::FloorShapeRefused).is_err() {
        return Err("测试注入：不用本地 floor".to_string());
    }
    let started = perf::enabled().then(perf::now_ms);
    let mut out = BTreeMap::new();
    for u in units {
        out.insert(u.wc_root.clone(), shape_of_unit(repo_root, u, floor)?);
    }
    if let Some(ts) = started {
        let mut rec = perf::Record::rust("query", "wc.db floor shape");
        rec.ts = ts;
        rec.dur = perf::now_ms() - ts;
        perf::record(&rec);
    }
    Ok(out)
}

fn shape_of_unit(
    repo_root: &str,
    u: &WcIdentity,
    floor: u64,
) -> std::result::Result<FloorShape, String> {
    let wc_root = u
        .wc_root_abspath
        .as_deref()
        .ok_or_else(|| format!("`{}` 没有工作副本根", u.wc_root))?;
    let unit_abs = format!("{}/{}", norm(repo_root), norm(&u.wc_root));
    let sub = within(&unit_abs, &norm(wc_root))
        .ok_or_else(|| format!("`{}` 不在工作副本根 `{wc_root}` 底下", u.wc_root))?;
    let db = open(&Path::new(wc_root).join(".svn").join("wc.db"))?;
    let shape = query_shape(&db, &sub, floor).map_err(|e| format!("查询 wc.db 失败：{e}"))??;
    Ok(match shape {
        FloorShape::Regular { dirs, stale } => FloorShape::Regular {
            dirs: dirs
                .iter()
                .map(|p| repo_relative(&u.wc_root, &sub, p))
                .collect(),
            stale: stale
                .iter()
                .map(|p| repo_relative(&u.wc_root, &sub, p))
                .collect(),
        },
        irregular => irregular,
    })
}

/// 子树条件：`local_relpath` 是 `?2` 本身或在它底下（`?2` 为空串时是整个工作副本）。区间写法的
/// 理由见 [`query`]。
const IN_SUBTREE: &str = "(?2 = '' OR local_relpath = ?2 OR \
                          (local_relpath >= ?2 || '/' AND local_relpath < ?2 || '0'))";

/// [`FloorShape`] 的查询本体。路径是 wc.db 的 relpath，调用方换算。外层 `Err` 是 SQLite 报的错，
/// 中层 `Err` 是认不出来（照旧跑 diff）。
fn query_shape(
    db: &Connection,
    sub: &str,
    floor: u64,
) -> rusqlite::Result<std::result::Result<FloorShape, String>> {
    let tx = db.unchecked_transaction()?;
    let format: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if format != FORMAT {
        return Ok(Err(format!("工作副本格式是 {format}，只认 {FORMAT}")));
    }
    let pending: Option<i64> = tx
        .query_row("SELECT 1 FROM work_queue LIMIT 1", [], |r| r.get(0))
        .optional()?;
    if pending.is_some() {
        return Ok(Err("有没做完的操作（WORK_QUEUE 非空）".to_string()));
    }
    let ids: Vec<i64> = tx
        .prepare("SELECT id FROM wcroot WHERE local_abspath IS NULL")?
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let [wc_id] = ids[..] else {
        return Ok(Err(format!("WCROOT 里有 {} 个根", ids.len())));
    };
    let irregular = |why: String| Ok(Ok(FloorShape::Irregular(why)));

    // 有人拿着工作副本锁：svn 操作正在进行或中途被杀（status 的 wc-locked）。不分子树：锁记在
    // 目录上、可以向下覆盖整棵树，判断它管不管到这个单元不值得
    let locked: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM wc_lock WHERE wc_id = ?1 LIMIT 1",
            [wc_id],
            |r| r.get(0),
        )
        .optional()?;
    if locked.is_some() {
        return irregular("工作副本被锁着（有 svn 操作正在进行或中途被杀）".to_string());
    }

    // WORKING 层（op_depth > 0）= 本地 add / delete / copy / move。diff 对它们的报法（added、
    // deleted、带历史的复制）status 给不全，交给 diff
    let working: Option<String> = tx
        .query_row(
            &format!(
                "SELECT local_relpath FROM nodes WHERE wc_id = ?1 AND op_depth > 0 \
                 AND {IN_SUBTREE} LIMIT 1"
            ),
            rusqlite::params![wc_id, sub],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(p) = working {
        return irregular(format!(
            "`{p}` 有本地的增删或复制（svn add / delete / copy）"
        ));
    }

    // 冲突（文本、属性、树）。格式 31 全部记在 conflict_data 里（wc-metadata.sql 的 ACTUAL_NODE
    // 注释：老的几列在升级到格式 30 时并进来了）
    let conflict: Option<String> = tx
        .query_row(
            &format!(
                "SELECT local_relpath FROM actual_node WHERE wc_id = ?1 \
                 AND conflict_data IS NOT NULL AND {IN_SUBTREE} LIMIT 1"
            ),
            rusqlite::params![wc_id, sub],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(p) = conflict {
        return irregular(format!("`{p}` 有冲突"));
    }

    struct Row {
        relpath: String,
        kind: String,
        presence: String,
        depth: Option<String>,
        revision: Option<i64>,
        changed: Option<i64>,
        repos_id: Option<i64>,
        repos_path: Option<String>,
        file_external: Option<i64>,
    }
    let rows: Vec<Row> = tx
        .prepare(&format!(
            "SELECT local_relpath, kind, presence, depth, revision, changed_revision, \
             repos_id, repos_path, file_external FROM nodes \
             WHERE wc_id = ?1 AND op_depth = 0 AND {IN_SUBTREE} ORDER BY local_relpath"
        ))?
        .query_map(rusqlite::params![wc_id, sub], |r| {
            Ok(Row {
                relpath: r.get(0)?,
                kind: r.get(1)?,
                presence: r.get(2)?,
                depth: r.get(3)?,
                revision: r.get(4)?,
                changed: r.get(5)?,
                repos_id: r.get(6)?,
                repos_path: r.get(7)?,
                file_external: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let Some(root) = rows.iter().find(|r| r.relpath == sub) else {
        return Ok(Err(format!("单元的根 `{sub}` 在 wc.db 的 BASE 里查不到")));
    };
    if root.presence != "normal" {
        return Ok(Err(format!(
            "单元的根 `{sub}` 在 wc.db 里是 {}",
            root.presence
        )));
    }
    let (Some(root_repos), Some(root_path)) = (root.repos_id, root.repos_path.clone()) else {
        return Ok(Err(format!("单元的根 `{sub}` 没有仓库位置")));
    };

    let floor = i64::try_from(floor).unwrap_or(i64::MAX);
    let mut dirs = BTreeSet::new();
    let mut stale = Vec::new();
    for r in &rows {
        // 被排除的节点整单元 diff 本来就不报，体检另外点名比（`svn_health` 的 excluded）
        if r.presence == "excluded" {
            continue;
        }
        if r.presence != "normal" {
            // not-present（提交了删除、父目录还没更新）、server-excluded、incomplete：F 上可能
            // 有工作副本里没有的东西
            return irregular(format!("`{}` 在 wc.db 里是 {}", r.relpath, r.presence));
        }
        if r.file_external.is_some() {
            return irregular(format!("`{}` 是 file external", r.relpath));
        }
        if r.kind != "file" && r.kind != "dir" {
            return irregular(format!("`{}` 的 kind 是 {}", r.relpath, r.kind));
        }
        // switch 过的子树：diff 拿它自己的 URL 去比，F 上的内容另有来处
        let rel = if sub.is_empty() {
            r.relpath.as_str()
        } else {
            r.relpath[sub.len()..].trim_start_matches('/')
        };
        let expect = match (root_path.is_empty(), rel.is_empty()) {
            (_, true) => root_path.clone(),
            (true, false) => rel.to_string(),
            (false, false) => format!("{root_path}/{rel}"),
        };
        if r.repos_id != Some(root_repos) || r.repos_path.as_deref() != Some(expect.as_str()) {
            return irregular(format!("`{}` 被 switch 到了别处", r.relpath));
        }
        if r.kind == "dir" {
            if r.depth.as_deref() != Some("infinity") {
                return irregular(format!(
                    "`{}` 是稀疏检出（depth={}）",
                    r.relpath,
                    r.depth.as_deref().unwrap_or("?")
                ));
            }
            dirs.insert(r.relpath.clone());
        }
        let (Some(n), Some(c)) = (r.revision, r.changed) else {
            return irregular(format!("`{}` 没有 revision", r.relpath));
        };
        if c <= floor && floor <= n {
            continue;
        }
        if r.kind == "dir" {
            // 目录的子项清单在 F 上可能不同：子项增删会让目录的 committed-rev 前进
            return irregular(format!(
                "目录 `{}` 在 r{n}（最后修改 r{c}），F 上的子项可能不同",
                r.relpath
            ));
        }
        stale.push(r.relpath.clone());
    }
    Ok(Ok(FloorShape::Regular { dirs, stale }))
}

/// wc.db 里的 `relpath`（相对工作副本根）换成仓库相对：单元那一段用调用方给的写法（svn info
/// 的 entry path 也是照抄 target），往下的部分用 wc.db 的。
fn repo_relative(unit: &str, sub: &str, relpath: &str) -> String {
    let unit = norm(unit);
    let rest = if sub.is_empty() {
        relpath
    } else {
        relpath[sub.len()..].trim_start_matches('/')
    };
    if rest.is_empty() {
        unit
    } else {
        format!("{unit}/{rest}")
    }
}

fn norm(p: &str) -> String {
    p.replace('\\', "/").trim_end_matches('/').to_string()
}

/// `path` 相对 `root` 的部分（`root` 本身是空串）；不在底下返回 `None`。Windows 上不区分
/// 大小写（盘符、用户打开仓库时的写法都可能与 svn 记的不同）。
fn within(path: &str, root: &str) -> Option<String> {
    let mut p = path.chars();
    for r in root.chars() {
        let c = p.next()?;
        if !same_char(c, r) {
            return None;
        }
    }
    let rest: String = p.collect();
    if rest.is_empty() {
        Some(String::new())
    } else {
        rest.strip_prefix('/').map(str::to_string)
    }
}

fn same_char(a: char, b: char) -> bool {
    if cfg!(windows) {
        a == b || a.to_lowercase().eq(b.to_lowercase())
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_root() {
        assert_eq!(within("C:/w/Code", "C:/w").as_deref(), Some("Code"));
        assert_eq!(within("C:/w", "C:/w").as_deref(), Some(""));
        assert_eq!(within("C:/w2/Code", "C:/w"), None, "前缀不是一整段");
        assert_eq!(within("C:/", "C:/w"), None);
        if cfg!(windows) {
            assert_eq!(within("c:/W/Code", "C:/w").as_deref(), Some("Code"));
        }
    }

    #[test]
    fn repo_relative_paths() {
        assert_eq!(repo_relative("Code", "Code", "Code/Deep"), "Code/Deep");
        assert_eq!(
            repo_relative("Code/Deep", "Code/Deep", "Code/Deep/D.cs"),
            "Code/Deep/D.cs"
        );
        assert_eq!(repo_relative("Code/Vendor", "", "V.cs"), "Code/Vendor/V.cs");
        assert_eq!(repo_relative("Code", "Code", "Code"), "Code");
    }
}
