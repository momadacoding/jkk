//! Local-Only 泄漏按 hunk 判定（分析 §6.5 F3、§6.11.2；日志 W21）。
//!
//! # 问题
//!
//! 用 TortoiseSVN 直接提交时，Local-Only commit 里的本地补丁（例如注释掉的 log）可能被
//! 一起带上服务器。同一个文件里往往既有 Local-Only 的 hunk、也有正常要提交的功能 hunk
//! （U6），所以按路径交集判断必然误报；判据必须按 hunk。
//!
//! # 为什么不用三方合并或 `git apply`（W21 实测）
//!
//! - 把整个 L 三方重放到 NEW 上：L 改第 5 行、NEW 改第 6 行这种相邻改动会合成一个冲突块，
//!   判不出来。
//! - `git apply --unidiff-zero --check`：零上下文时 git 会按偏移在全文里找匹配，第 5 行与
//!   第 30 行内容相同时互相顶替，给出假阳性。
//!
//! # 做法（M4）
//!
//! 对 Local-Only commit L（父提交 B）改过、又在本次外部变更里的路径 f：
//!
//! 1. 用 `git2::Patch`（context_lines = 0）求 B→NEW 的 hunk，作为 B 行号到 NEW 行号的映射。
//! 2. 对 L 的每个 hunk（B→L，同样零上下文）：
//!    - 与 NEW 的任何 hunk 都不重叠：NEW 没碰这几行，不可能泄漏（不输出）；
//!    - 重叠，且有一个起点更早、改变了行数的 NEW hunk：映射位置拿不准 → [`LeakVerdict::Unknown`]；
//!    - 映射位置上 NEW 的内容逐字等于 L 的 `+` 行 → [`LeakVerdict::Present`]；
//!    - 其余在重叠的 NEW hunk 的新内容里逐行查 L 的 `+` 行：全部查到 → [`LeakVerdict::Present`]，
//!      查到一部分 → [`LeakVerdict::Unknown`]，一条都没有才算没泄漏。
//!
//! 只比映射位置不够（Review 2）：`git commit` 时功能改动和本地补丁落在同一个 hunk 里，L 的行
//! 在 NEW 里往往不在映射位置上（同一点先插了功能行、一个 hunk 只上去一半）。
//!
//! 全程只读对象（读取走 libgit2 是本项目的既定分工），不写任何东西。
//!
//! # 误判方向
//!
//! 漏报（把泄漏判成没泄漏）是危险的方向：本地补丁会被一次静默跟进带过去。拿不准时一律给
//! `Unknown`，宁可多一条告警。

use crate::engine::svn_replay::ReplayCommit;
use crate::error::Result;
use git2::{DiffOptions, ObjectType, Oid, Patch, Repository, Tree};
use std::collections::BTreeSet;

/// 某条分支上一个 Local-Only commit 在某个文件里被带上服务器（或拿不准）的 hunk。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Leak {
    pub branch: String,
    /// Local-Only commit 的 OID。
    pub commit: String,
    pub subject: String,
    /// 仓库相对路径（`/` 分隔）。
    pub path: String,
    /// 至少一条；只列 Present / Unknown。
    pub hunks: Vec<HunkLeak>,
}

/// L 的一个 hunk，坐标与 `git diff -U0 B L` 的 hunk 头 `@@ -old_start,lines` 一致
/// （纯插入时 `old_start` 是插入点之前那一行、`lines` 为 0）。
///
/// 整文件的判定（二进制、L 删了文件且 NEW 也删了）没有行号可给，记为 `old_start = 0`、
/// `lines = 0`，与「从空文件新增」的 hunk 头 `@@ -0,0` 同形。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkLeak {
    pub old_start: u32,
    pub lines: u32,
    pub verdict: LeakVerdict,
}

/// Absent（没泄漏）不输出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LeakVerdict {
    /// L 的这个 hunk 原样出现在 NEW 的对应位置：已经上了服务器。
    Present,
    /// NEW 动过这一片，但对不上位置或比不了内容（二进制）：交给用户看。
    Unknown,
}

/// 对 `commits` 里每个 Local-Only commit，找出它改过、又在 `changed_paths` 里的路径上
/// 已经进了 `new_base` 的 hunk。
///
/// - 只看 `local_only` 的 commit；B 取它的第一个父提交（没有父提交时是空树）。
/// - `changed_paths` 是本次跟进的外部变更集合（changed ∪ added ∪ removed），仓库相对路径。
/// - 结果按 `commits` 的顺序、同一 commit 内按路径排序。
pub fn local_only_leaks(
    repo: &str,
    branch: &str,
    commits: &[ReplayCommit],
    changed_paths: &BTreeSet<String>,
    new_base: &str,
) -> Result<Vec<Leak>> {
    let mut out = Vec::new();
    if changed_paths.is_empty() || !commits.iter().any(|c| c.local_only) {
        return Ok(out);
    }
    let repo = Repository::open(repo)?;
    let new_tree = repo.find_commit(Oid::from_str(new_base)?)?.tree()?;
    for c in commits.iter().filter(|c| c.local_only) {
        let commit = repo.find_commit(Oid::from_str(&c.oid)?)?;
        let l_tree = commit.tree()?;
        let b_tree = match commit.parent_ids().next() {
            Some(p) => Some(repo.find_commit(p)?.tree()?),
            None => None,
        };
        for path in touched_paths(&repo, b_tree.as_ref(), &l_tree)? {
            if !changed_paths.contains(&path) {
                continue;
            }
            let b = entry(b_tree.as_ref(), &path);
            let l = entry(Some(&l_tree), &path);
            let n = entry(Some(&new_tree), &path);
            let hunks = judge_path(&repo, b, l, n)?;
            if !hunks.is_empty() {
                out.push(Leak {
                    branch: branch.to_string(),
                    commit: c.oid.clone(),
                    subject: c.subject.clone(),
                    path,
                    hunks,
                });
            }
        }
    }
    Ok(out)
}

/// 树里某个路径的条目：(OID, 对象类型)。`filemode` 不参与判定：只改 mode 的泄漏
/// 不在 F3 的范围内（见日志 Task 7「遗留风险」）。
type Entry = Option<(Oid, Option<ObjectType>)>;

fn entry(tree: Option<&Tree>, path: &str) -> Entry {
    let e = tree?.get_path(std::path::Path::new(path)).ok()?;
    Some((e.id(), e.kind()))
}

/// L 相对 B 改过的文件路径（增、删、改；不做改名检测——改名在这里就是一删一增）。
fn touched_paths(repo: &Repository, b: Option<&Tree>, l: &Tree) -> Result<Vec<String>> {
    let diff = repo.diff_tree_to_tree(b, Some(l), None)?;
    let mut paths = Vec::new();
    for d in diff.deltas() {
        // 删除只有 old_file，其余以 new_file 为准
        let f = if d.new_file().exists() {
            d.new_file()
        } else {
            d.old_file()
        };
        if let Some(p) = f.path_bytes() {
            paths.push(String::from_utf8_lossy(p).into_owned());
        }
    }
    Ok(paths)
}

/// 整文件的判定结果（见 [`HunkLeak`] 的约定）。
fn whole_file(verdict: LeakVerdict) -> Vec<HunkLeak> {
    vec![HunkLeak {
        old_start: 0,
        lines: 0,
        verdict,
    }]
}

fn judge_path(repo: &Repository, b: Entry, l: Entry, n: Entry) -> Result<Vec<HunkLeak>> {
    // NEW 里这个路径与 B 完全相同：NEW 没碰它，什么都没带上去
    if n == b {
        return Ok(Vec::new());
    }
    let (Some(l), Some(n)) = (l, n) else {
        // L 删了文件：NEW 也删了才是「删除」泄漏；NEW 只是改了内容，删除没上去。
        // NEW 删了文件而 L 是改 / 增：L 的内容不可能在 NEW 里
        return Ok(if l.is_none() && n.is_none() {
            whole_file(LeakVerdict::Present)
        } else {
            Vec::new()
        });
    };
    // 两侧都有、NEW 也改过。非 blob（gitlink 等）或二进制：按行比不了
    let blob_bytes = |e: (Oid, Option<ObjectType>)| -> Result<Option<Vec<u8>>> {
        if e.1 != Some(ObjectType::Blob) {
            return Ok(None);
        }
        let blob = repo.find_blob(e.0)?;
        Ok((!blob.is_binary()).then(|| blob.content().to_vec()))
    };
    let b_bytes = match b {
        // L 新增的文件：B 当空文件
        None => Some(Vec::new()),
        Some(e) => blob_bytes(e)?,
    };
    let (Some(b_bytes), Some(l_bytes), Some(n_bytes)) = (b_bytes, blob_bytes(l)?, blob_bytes(n)?)
    else {
        return Ok(whole_file(LeakVerdict::Unknown));
    };
    let l_hunks = spans(&b_bytes, &l_bytes)?;
    let n_hunks = spans(&b_bytes, &n_bytes)?;
    let l_lines: Vec<&[u8]> = l_bytes.split_inclusive(|c| *c == b'\n').collect();
    let n_lines: Vec<&[u8]> = n_bytes.split_inclusive(|c| *c == b'\n').collect();
    let mut out = Vec::new();
    for h in &l_hunks {
        // L 的 `+` 行：L 中从 new_start 起的 new_lines 行（new_lines = 0 时 new_start 无意义）
        let plus = if h.nc == 0 {
            &[][..]
        } else {
            slice(&l_lines, h.new_start as i64, h.nc as usize).unwrap_or(&[])
        };
        if let Some(verdict) = judge_hunk(h, &n_hunks, &n_lines, plus) {
            out.push(HunkLeak {
                old_start: h.old_start,
                lines: h.oc,
                verdict,
            });
        }
    }
    Ok(out)
}

/// 一个零上下文 hunk 在 B 中的位置。
struct Span {
    /// git 的 hunk 头原值（纯插入时是插入点之前那一行，可能是 0）。
    old_start: u32,
    /// B 中受影响的第一行（1 基）；B 侧区间是半开的 `[start, start + oc)`。
    /// 纯插入时区间为空，`start` 是插入点之后那一行。
    start: u32,
    oc: u32,
    nc: u32,
    new_start: u32,
}

impl Span {
    fn end(&self) -> u32 {
        self.start + self.oc
    }
}

fn spans(old: &[u8], new: &[u8]) -> Result<Vec<Span>> {
    let mut opts = DiffOptions::new();
    // 上下文为 0、相邻 hunk 不合并：每个 hunk 只含真正改动的行，才能逐个比。
    // force_text：二进制在调用前已经排除，这里不让 libgit2 再按内容猜一次
    opts.context_lines(0).interhunk_lines(0).force_text(true);
    let patch = Patch::from_buffers(old, None, new, None, Some(&mut opts))?;
    let mut out = Vec::with_capacity(patch.num_hunks());
    for i in 0..patch.num_hunks() {
        let (h, _) = patch.hunk(i)?;
        let (os, oc) = (h.old_start(), h.old_lines());
        out.push(Span {
            old_start: os,
            start: if oc > 0 { os } else { os + 1 },
            oc,
            nc: h.new_lines(),
            new_start: h.new_start(),
        });
    }
    Ok(out)
}

/// 1 基起点、长度 `len` 的一段行；越界返回 `None`。
fn slice<'a, 'b>(lines: &'b [&'a [u8]], start: i64, len: usize) -> Option<&'b [&'a [u8]]> {
    let from = usize::try_from(start.checked_sub(1)?).ok()?;
    lines.get(from..from.checked_add(len)?)
}

/// 判定 L 的一个 hunk `h`；`None` 表示没泄漏。规则与 W21 的原型（hunkcheck.pl）一致，
/// 只在纯删除上收紧（见下）。
fn judge_hunk(h: &Span, news: &[Span], n_lines: &[&[u8]], plus: &[&[u8]]) -> Option<LeakVerdict> {
    let (s, e) = (h.start, h.end());
    // B→NEW 在 h 之前累计的行数变化：把 h 的起点映射到 NEW
    let mut delta: i64 = 0;
    let mut overlap = false;
    let mut unknown = false;
    // 重叠的 NEW hunk 是否都落在 h 的区间里，以及它们的净行数变化（只给纯删除用）
    let mut contained = true;
    let mut net: i64 = 0;
    // 重叠的 NEW hunk 带进来的全部行
    let mut carried: Vec<&[u8]> = Vec::new();
    for k in news {
        let (ks, ke) = (k.start, k.end());
        let change = i64::from(k.nc) - i64::from(k.oc);
        // 同一插入点的两个纯插入不算「之前」：要拿来比内容
        let same_insert_point = k.oc == 0 && h.oc == 0 && ks == s;
        // 整个在 h 之前：算进偏移。它可能同时贴着 h 的上边界，所以不能就此跳过下面的重叠判断
        let before = ke <= s && !same_insert_point;
        if before {
            delta += change;
        }
        // 有一侧是纯插入（区间为空）时，碰到边界也算重叠
        let ov = if h.oc == 0 || k.oc == 0 {
            ks <= e && s <= ke
        } else {
            ks < e && s < ke
        };
        // L 纯删除、NEW 只在边界上插行：这几行还在 NEW 里，没有什么可比的
        let beside_deletion = plus.is_empty() && k.oc == 0 && (ks == s || ks == e);
        if !ov || beside_deletion {
            continue;
        }
        overlap = true;
        if !before && ks < s && k.nc != k.oc {
            // NEW 从更早的地方开始改，并改变了行数：h 的起点落在 NEW 里哪一行说不准
            unknown = true;
        }
        if ks < s || ke > e {
            contained = false;
        }
        net += change;
        if k.nc > 0 {
            carried.extend(slice(n_lines, i64::from(k.new_start), k.nc as usize).unwrap_or(&[]));
        }
    }
    if !overlap {
        return None;
    }
    if unknown {
        return Some(LeakVerdict::Unknown);
    }
    if plus.is_empty() {
        // 纯删除：原型拿空的 `+` 行去比，永远相等，NEW 把这几行改成别的也会判成泄漏。
        // 这里要求 NEW 的改动都在 h 的区间内、且净删掉的正好是这几行；其余拿不准
        return Some(if contained && i64::from(h.oc) + net == 0 {
            LeakVerdict::Present
        } else {
            LeakVerdict::Unknown
        });
    }
    if slice(n_lines, i64::from(s) + delta, plus.len()) == Some(plus) {
        return Some(LeakVerdict::Present);
    }
    // 映射位置对不上不等于没泄漏。按条消费：`+` 里有两条相同的行、NEW 只带上去一条时是半个 hunk
    let hits = plus.iter().filter(|l| take(&mut carried, l)).count();
    if hits == 0 {
        None
    } else if hits == plus.len() {
        Some(LeakVerdict::Present)
    } else {
        Some(LeakVerdict::Unknown)
    }
}

/// 从 `pool` 里取走一条与 `line` 相同的行。行尾不算内容：文件末行没有换行、同一行换了行尾
/// 都是常见的形状，这里漏掉就是漏报。
fn take(pool: &mut Vec<&[u8]>, line: &[u8]) -> bool {
    match pool.iter().position(|l| body(l) == body(line)) {
        Some(i) => {
            pool.swap_remove(i);
            true
        }
        None => false,
    }
}

fn body(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}
