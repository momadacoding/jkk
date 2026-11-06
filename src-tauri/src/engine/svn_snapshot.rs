//! 重放时快照内容按基线走（事故分析 6.10；方案 `docs/superpowers/specs/2026-11-06-svn-snapshot-replay-design.md`）。
//!
//! 「`svn up` 之后把整个工作区提交进 git」的快照提交，把别人的 SVN 改动记成了自己的 commit。分支停在
//! 旧基线上、SVN 之后又改了同一片代码时，重放就会冲突，或者干净地合错（2026-11-05：37 个冲突里
//! 35 个是这种，另有两处静默合错）。
//!
//! 规则：重放 commit C 时，C 改过的路径 p 同时满足下面两条，就当成 SVN 的内容，放进 F2 ① 已有的
//! 「换成 SVN 版本」集合（`svn_replay` 的 `superseded`），重放时取新基线的版本：
//!
//! 1. C 里 p 的条目等于某个**比这条分支自己的基线更新**的基线里 p 的条目，且不等于分支自己基线里的；
//! 2. C^ 里 p 的条目也是纯 SVN 的：等于分支自己基线里的，或者也出现在某个更新的基线里。
//!
//! 第 2 条（独立 review 第 1 条）：换的手段是把 p 写回 C^ 的条目，C^ 里要是有我自己的内容（前面的
//! commit 改过、后来在 SVN 侧撤掉再整包提交），写回去就把撤掉的改动复活了。这种情形不命中，退回原来
//! 的重放（冲突就交给人）。只比 git 对象，不联网。
//!
//! 判定出错时这条 commit 什么都不加，退回原来的行为：宁可多一个冲突交给人，也不猜。

use crate::engine::git_cli::run_git;
use crate::engine::svn_replay::ReplayCommit;
use crate::error::Result;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// 一个路径的条目：(mode, oid)；`None` = 不存在（删除）。
type Entry = Option<(String, String)>;

/// 一条分支基线之后、到 NEW 为止的基线里的条目。
struct Versions {
    /// 出现过的条目，已去掉与分支基线相同的。
    seen: BTreeMap<String, BTreeSet<Entry>>,
    /// `seen` 里各路径在分支基线里的条目。
    at_scope: BTreeMap<String, Entry>,
    /// `seen` 里各路径在 NEW 里的条目（链上最后一次改动）。
    at_new: BTreeMap<String, Entry>,
}

/// commit 里命中的路径及其在 C 里的条目。
type Hits = BTreeMap<String, Entry>;

/// 按分支基线缓存的判定。当前分支、其他分支、手动跟进共用一个，同一个基线只读一遍。
pub struct SnapshotRule<'a> {
    repo: &'a str,
    new: &'a str,
    /// 分支基线 → 各基线的条目。`None`：读的时候出错了，这个基线下一律不命中。
    versions: RefCell<BTreeMap<String, Option<Versions>>>,
    /// (分支基线, commit) → 命中的路径。
    hits: RefCell<BTreeMap<(String, String), Hits>>,
}

impl<'a> SnapshotRule<'a> {
    /// `new`：这次重放的目标基线（NEW）。
    pub fn new(repo: &'a str, new: &'a str) -> Self {
        Self {
            repo,
            new,
            versions: RefCell::new(BTreeMap::new()),
            hits: RefCell::new(BTreeMap::new()),
        }
    }

    /// commit `c` 里按本规则要换成 SVN 版本的路径。`scope` 是 `c` 所在分支自己的基线。
    ///
    /// Local-Only commit 一律不命中：它的内容等于基线说明补丁被带上了服务器，那是 F3 泄漏检查要
    /// 报的，换成 SVN 版本等于把它静默收掉（与 F2 ① 同一个理由）。
    pub fn paths(&self, scope: &str, c: &ReplayCommit) -> BTreeSet<String> {
        self.hits(scope, c).into_keys().collect()
    }

    /// 要在报告里告诉用户的命中路径：去掉 C 里已经与 NEW 完全相同的（独立 review 第 7 条）。那种
    /// 换不换结果都一样，不存在「把有意停住的旧版本换掉」的风险，报出来只是噪音。
    pub fn reported_paths(&self, scope: &str, c: &ReplayCommit) -> BTreeSet<String> {
        let hits = self.hits(scope, c);
        let versions = self.versions.borrow();
        let at_new = match versions.get(scope) {
            Some(Some(v)) => &v.at_new,
            _ => return BTreeSet::new(),
        };
        hits.into_iter()
            .filter(|(p, entry)| at_new.get(p) != Some(entry))
            .map(|(p, _)| p)
            .collect()
    }

    fn hits(&self, scope: &str, c: &ReplayCommit) -> Hits {
        if c.local_only {
            return Hits::new();
        }
        let key = (scope.to_string(), c.oid.clone());
        if let Some(hit) = self.hits.borrow().get(&key) {
            return hit.clone();
        }
        let hit = match self.compute(scope, c) {
            Ok(hit) => hit,
            Err(e) => {
                log::warn!(
                    "判定 {} 里的快照内容时出错，按没有命中处理：{}",
                    short(&c.oid),
                    e.message
                );
                Hits::new()
            }
        };
        self.hits.borrow_mut().insert(key, hit.clone());
        hit
    }

    fn compute(&self, scope: &str, c: &ReplayCommit) -> Result<Hits> {
        let [parent] = c.parents.as_slice() else {
            return Ok(Hits::new());
        };
        if !self.versions.borrow().contains_key(scope) {
            let v = match read_versions(self.repo, scope, self.new) {
                Ok(v) => Some(v),
                Err(e) => {
                    log::warn!(
                        "读不出 {} 之后的基线，快照内容不按基线走：{}",
                        short(scope),
                        e.message
                    );
                    None
                }
            };
            self.versions.borrow_mut().insert(scope.to_string(), v);
        }
        let versions = self.versions.borrow();
        let Some(Some(v)) = versions.get(scope) else {
            return Ok(Hits::new());
        };
        if v.seen.is_empty() {
            return Ok(Hits::new());
        }
        let mut hit = Hits::new();
        for (path, before, after) in raw_diff(self.repo, parent, &c.oid)? {
            let Some(seen) = v.seen.get(&path) else {
                continue;
            };
            // 第 1 条：C 的内容是某个更新的基线里的
            if !seen.contains(&after) {
                continue;
            }
            // 第 2 条：C^ 的内容也是纯 SVN 的，写回它不会复活我的改动
            let parent_is_svn = v.at_scope.get(&path) == Some(&before) || seen.contains(&before);
            if parent_is_svn {
                hit.insert(path, after);
            }
        }
        Ok(hit)
    }
}

/// `scope..new` 上每个基线提交改过的路径的新条目，去掉与 `scope` 相同的（用户故意退回分支自己
/// 基线的版本时不算 SVN 内容）。基线提交是一条链，各自对上一项的 diff 合起来就是全部出现过的条目，
/// 链上最后一次改动就是 NEW 里的条目。
fn read_versions(repo: &str, scope: &str, new: &str) -> Result<Versions> {
    let mut v = Versions {
        seen: BTreeMap::new(),
        at_scope: BTreeMap::new(),
        at_new: BTreeMap::new(),
    };
    if scope == new {
        return Ok(v);
    }
    let list = run_git(
        repo,
        &[
            "rev-list",
            "--first-parent",
            "--reverse",
            &format!("{scope}..{new}"),
        ],
    )?;
    let mut prev = scope.to_string();
    for b in list.lines().map(str::trim).filter(|l| !l.is_empty()) {
        for (path, _, entry) in raw_diff(repo, &prev, b)? {
            v.seen
                .entry(path.clone())
                .or_default()
                .insert(entry.clone());
            v.at_new.insert(path, entry);
        }
        prev = b.to_string();
    }
    if v.seen.is_empty() {
        return Ok(v);
    }
    let paths: Vec<&str> = v.seen.keys().map(String::as_str).collect();
    let found = entries_for(repo, scope, &paths)?;
    for path in v.seen.keys() {
        v.at_scope.insert(path.clone(), found.get(path).cloned());
    }
    for (path, seen) in v.seen.iter_mut() {
        if let Some(at) = v.at_scope.get(path) {
            seen.remove(at);
        }
    }
    v.seen.retain(|_, seen| !seen.is_empty());
    Ok(v)
}

/// `diff-tree -r a b` 的每个路径与它在 `a`、`b` 里的条目（不存在时为 `None`）。
fn raw_diff(repo: &str, a: &str, b: &str) -> Result<Vec<(String, Entry, Entry)>> {
    // raw 格式：`:<旧 mode> <新 mode> <旧 oid> <新 oid> <状态>` NUL `<路径>` NUL
    let out = run_git(repo, &["diff-tree", "-r", "-z", "--no-renames", a, b])?;
    let mut list = Vec::new();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (Some(old_mode), Some(new_mode), Some(old_oid), Some(new_oid), Some(status)) = (
            meta.first(),
            meta.get(1),
            meta.get(2),
            meta.get(3),
            meta.get(4),
        ) else {
            continue;
        };
        let before = if status.starts_with('A') {
            None
        } else {
            Some((old_mode.to_string(), old_oid.to_string()))
        };
        let after = if status.starts_with('D') {
            None
        } else {
            Some((new_mode.to_string(), new_oid.to_string()))
        };
        list.push((path.to_string(), before, after));
    }
    Ok(list)
}

/// `rev` 树里 `paths` 各自的条目；不存在的不出现在结果里。只查用得到的路径（独立 review 第 10 条：
/// 大仓库整棵 `ls-tree -r` 的输出很长）。
fn entries_for(
    repo: &str,
    rev: &str,
    paths: &[&str],
) -> Result<BTreeMap<String, (String, String)>> {
    let wanted: BTreeSet<&str> = paths.iter().copied().collect();
    let mut found = BTreeMap::new();
    let mut start = 0;
    while start < paths.len() {
        // 分批：Windows 命令行上限约 32K 字符
        let mut end = start;
        let mut len = 0;
        while end < paths.len() && (end == start || len + paths[end].len() < 16_000) {
            len += paths[end].len() + 1;
            end += 1;
        }
        // `--literal-pathspecs`：`[`、`*` 不当 glob；`--full-tree`：不受 cwd 影响
        let mut args = vec![
            "--literal-pathspecs",
            "ls-tree",
            "--full-tree",
            "-z",
            rev,
            "--",
        ];
        args.extend_from_slice(&paths[start..end]);
        let out = run_git(repo, &args)?;
        for rec in out.split('\0').filter(|r| !r.is_empty()) {
            let Some((meta, path)) = rec.split_once('\t') else {
                continue;
            };
            if !wanted.contains(path) {
                continue;
            }
            let mut m = meta.split(' ');
            if let (Some(mode), Some(_), Some(oid)) = (m.next(), m.next(), m.next()) {
                found.insert(path.to_string(), (mode.to_string(), oid.to_string()));
            }
        }
        start = end;
    }
    Ok(found)
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}
