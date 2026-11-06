//! 重放时快照内容按基线走（事故分析 6.10；方案 `docs/superpowers/specs/2026-11-06-svn-snapshot-replay-design.md`）。
//!
//! 「`svn up` 之后把整个工作区提交进 git」的快照提交，把别人的 SVN 改动记成了自己的 commit。分支停在
//! 旧基线上、SVN 之后又改了同一片代码时，重放就会冲突，或者干净地合错（2026-11-05：37 个冲突里
//! 35 个是这种，另有两处静默合错）。
//!
//! 规则：重放 commit C 时，C 改过的路径 p，如果 C 里 p 的条目等于某个**比这条分支自己的基线更新**
//! 的基线里 p 的条目，且不等于分支自己基线里的条目，就当成 SVN 的内容，放进 F2 ① 已有的「换成
//! SVN 版本」集合（`svn_replay` 的 `superseded`），重放时取新基线的版本。只比 git 对象，不联网。
//!
//! 判定出错时这条 commit 什么都不加，退回原来的行为：宁可多一个冲突交给人，也不猜。

use crate::engine::git_cli::run_git;
use crate::engine::svn_replay::ReplayCommit;
use crate::error::Result;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

/// 一个路径的条目：(mode, oid)；`None` = 不存在（删除）。
type Entry = Option<(String, String)>;

/// 一条分支基线之后、到 NEW 为止各个基线里出现过的条目，已去掉与分支基线相同的。
type Versions = BTreeMap<String, BTreeSet<Entry>>;

/// 按分支基线缓存的判定。当前分支、其他分支、手动跟进共用一个，同一个基线只读一遍。
pub struct SnapshotRule<'a> {
    repo: &'a str,
    new: &'a str,
    /// 分支基线 → 各基线的条目。`None`：读的时候出错了，这个基线下一律不命中。
    versions: RefCell<BTreeMap<String, Option<Versions>>>,
    /// (分支基线, commit) → 命中的路径。
    hits: RefCell<BTreeMap<(String, String), BTreeSet<String>>>,
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
        if c.local_only {
            return BTreeSet::new();
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
                BTreeSet::new()
            }
        };
        self.hits.borrow_mut().insert(key, hit.clone());
        hit
    }

    fn compute(&self, scope: &str, c: &ReplayCommit) -> Result<BTreeSet<String>> {
        let [parent] = c.parents.as_slice() else {
            return Ok(BTreeSet::new());
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
        let Some(Some(versions)) = versions.get(scope) else {
            return Ok(BTreeSet::new());
        };
        if versions.is_empty() {
            return Ok(BTreeSet::new());
        }
        let mut hit = BTreeSet::new();
        for (path, entry) in raw_diff(self.repo, parent, &c.oid)? {
            if versions
                .get(&path)
                .is_some_and(|seen| seen.contains(&entry))
            {
                hit.insert(path);
            }
        }
        Ok(hit)
    }
}

/// `scope..new` 上每个基线提交改过的路径的新条目，去掉与 `scope` 相同的（用户故意退回分支自己
/// 基线的版本时不算 SVN 内容）。基线提交是一条链，各自对父提交的 diff 合起来就是全部出现过的条目。
fn read_versions(repo: &str, scope: &str, new: &str) -> Result<Versions> {
    let mut versions = Versions::new();
    if scope == new {
        return Ok(versions);
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
        for (path, entry) in raw_diff(repo, &prev, b)? {
            versions.entry(path).or_default().insert(entry);
        }
        prev = b.to_string();
    }
    if versions.is_empty() {
        return Ok(versions);
    }
    let at_scope = tree_entries(repo, scope)?;
    for (path, seen) in versions.iter_mut() {
        seen.remove(&at_scope.get(path).cloned());
    }
    versions.retain(|_, seen| !seen.is_empty());
    Ok(versions)
}

/// `diff-tree -r` 的每个路径与它在 `b` 里的条目（`b` 里没有时为 `None`）。
fn raw_diff(repo: &str, a: &str, b: &str) -> Result<Vec<(String, Entry)>> {
    // raw 格式：`:<旧 mode> <新 mode> <旧 oid> <新 oid> <状态>` NUL `<路径>` NUL
    let out = run_git(repo, &["diff-tree", "-r", "-z", "--no-renames", a, b])?;
    let mut list = Vec::new();
    let mut fields = out.split('\0').filter(|f| !f.is_empty());
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let meta: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (Some(mode), Some(oid), Some(status)) = (meta.get(1), meta.get(3), meta.get(4)) else {
            continue;
        };
        let entry = if status.starts_with('D') {
            None
        } else {
            Some((mode.to_string(), oid.to_string()))
        };
        list.push((path.to_string(), entry));
    }
    Ok(list)
}

/// `rev` 树里全部文件的条目。一个进程读完，比逐路径 `ls-tree` 省事。
fn tree_entries(repo: &str, rev: &str) -> Result<BTreeMap<String, (String, String)>> {
    let out = run_git(repo, &["ls-tree", "-r", "-z", "--full-tree", rev])?;
    let mut map = BTreeMap::new();
    for rec in out.split('\0').filter(|r| !r.is_empty()) {
        let Some((meta, path)) = rec.split_once('\t') else {
            continue;
        };
        let mut m = meta.split(' ');
        if let (Some(mode), Some(_), Some(oid)) = (m.next(), m.next(), m.next()) {
            map.insert(path.to_string(), (mode.to_string(), oid.to_string()));
        }
    }
    Ok(map)
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}
