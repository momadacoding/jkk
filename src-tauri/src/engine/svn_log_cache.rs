//! SVN 历史缓存：`<仓库根>/.git/mygitclient-svnlog.json`。
//!
//! 放在 `.git/` 内部而不是工作目录，理由同 [`crate::engine::svn_config`]：
//! 工作目录**只多出 `.git` 一个条目**（spec §3），多一个文件就多一个被
//! TortoiseSVN 误提交上去的东西。
//!
//! # 为什么可以永久缓存
//!
//! SVN 历史不可变（除非有人改 revprop，罕见到可忽略，spec §10.2）。已拉取的
//! revision 只增不改，增量只拉新的。`svn log` 每次都走网络（工作副本里没有任何
//! 过往 revision 的记录，spec §10.1），所以这份缓存是「打开仓库先把图画出来」
//! 的唯一依据。
//!
//! # 缓存是加速手段，不是事实来源
//!
//! 任何一种读失败——文件不存在、JSON 坏了、版本号不认识、uuid 对不上——都
//! **降级成空缓存**，绝不向上抛错。让一个加速用的 JSON 把仓库打不开，是本模块
//! 最坏的失败形态。

use crate::engine::svn_xml::SvnLogEntry;
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 缓存格式版本。改变字段语义时必须 +1——旧文件会被整份作废而不是误读。
const CACHE_VERSION: u32 = 1;

/// 缓存里的一条 revision 记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedRev {
    pub rev: u64,
    pub author: String,
    /// ISO8601 原样，与 [`SvnLogEntry::date`] 一致，归一化留给前端。
    pub date: String,
    pub message: String,
}

/// 一个 `svn log` target 已缓存的历史。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetLog {
    /// 已**连续**覆盖区间的下界（含）。往下翻页时朝这个方向扩。
    pub covered_min: u64,
    /// 已**连续**覆盖区间的上界（含）。增量拉取只拉 `(covered_max, HEAD]`。
    ///
    /// `0` 表示「还什么都没覆盖」——revision 从 1 起算，0 不是合法值。
    pub covered_max: u64,
    /// 按 revision **降序**。
    pub entries: Vec<CachedRev>,
}

impl TargetLog {
    pub fn new() -> Self {
        Self {
            covered_min: 0,
            covered_max: 0,
            entries: Vec::new(),
        }
    }

    /// 还没覆盖过任何区间。
    fn is_fresh(&self) -> bool {
        self.covered_max == 0
    }
}

impl Default for TargetLog {
    fn default() -> Self {
        Self::new()
    }
}

/// 整份缓存。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnLogCache {
    pub version: u32,
    /// 仓库身份。与当前 `svn info` 的 UUID 不符时**整份作废**。
    ///
    /// 同一个磁盘路径重新 checkout 到别的仓库之后，旧缓存的 revision 号在新仓库里
    /// 指向完全不同的提交。混用不会报任何错，只会画出一张**看起来完全正常**的
    /// 假历史——这是本模块唯一一个会产生错误信息而不是缺失信息的失败形态。
    pub uuid: String,
    /// key = target URL（[`crate::engine::svn_log::log_targets`] 的产物）。
    pub targets: BTreeMap<String, TargetLog>,
    /// 孤儿 baseline 的锚点，key = revision。
    ///
    /// # 为什么必须与 `targets` 分开存
    ///
    /// 锚点是**逐个 revision 单点补拉**的（`-r N:N` 打给 Repository Root），
    /// 彼此之间不连续。把它们记进某个 target 的 `covered_min/max` 会谎称中间那
    /// 一大段也拉过了，下一次增量拉取直接跳过——那段历史永久缺失，而且界面上
    /// 看不出来（图上只是少了些节点，不会有任何报错）。
    pub anchors: BTreeMap<u64, CachedRev>,
}

impl SvnLogCache {
    pub fn new(uuid: &str) -> Self {
        Self {
            version: CACHE_VERSION,
            uuid: uuid.to_string(),
            targets: BTreeMap::new(),
            anchors: BTreeMap::new(),
        }
    }
}

pub fn cache_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root)
        .join(".git")
        .join("mygitclient-svnlog.json")
}

/// 读缓存。**任何失败都返回一份干净的空缓存**，永不报错。
///
/// UUID 比较是大小写不敏感的：svn 各处对 UUID 的写法并不一致，大小写敏感比较
/// 会让缓存每次都作废，表现为「每次打开仓库都要重拉全部历史」。
pub fn read_cache(repo_root: &str, uuid: &str) -> SvnLogCache {
    let fallback = SvnLogCache::new(uuid);
    let Ok(raw) = std::fs::read_to_string(cache_path(repo_root)) else {
        return fallback;
    };
    let Ok(parsed) = serde_json::from_str::<SvnLogCache>(&raw) else {
        return fallback;
    };
    if parsed.version != CACHE_VERSION || !parsed.uuid.eq_ignore_ascii_case(uuid) {
        return fallback;
    }
    parsed
}

/// 原子写：先写同目录临时文件再 rename，形状照抄 `session.rs`。
///
/// 直接覆写的话，写到一半掉电会留下半截 JSON。虽然 [`read_cache`] 会把它降级成
/// 空缓存（不致命），但用户会看到历史莫名其妙整个消失一次，而且没有任何提示。
pub fn write_cache(repo_root: &str, cache: &SvnLogCache) -> Result<()> {
    let path = cache_path(repo_root);
    let dir = path
        .parent()
        .ok_or_else(|| GitError::new("svn_log_cache", "缓存路径没有父目录"))?;
    let tmp = dir.join(format!(
        "mygitclient-svnlog.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let bytes = serde_json::to_vec_pretty(cache)
        .map_err(|e| GitError::new("svn_log_cache_encode", e.to_string()))?;
    std::fs::write(&tmp, bytes)?;
    // rename 失败时别把临时文件留在 .git 里——那会让 `.git` 里慢慢堆出一地垃圾，
    // 而每一个都长得像一份合法缓存。
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// 把一次拉取的结果并进某个 target。
///
/// `from` / `to` 是**这次实际查询的区间**，方向随意（`fetch_range` 会用降序区间
/// 拉「最近 N 条」）。
///
/// # 覆盖区间只记**连续**的那一段
///
/// `covered_min/max` 的唯一用途是「下次从哪儿接着拉」，所以它必须是一段**真的
/// 没有洞**的区间。新区间与旧区间不相接时（中间隔着没拉过的 revision），
/// **整段换成新区间**而不是取并集的最值——后者会谎称中间那段也拉过了，
/// 下一次增量拉取跳过它，那段历史就永久缺失，界面上还看不出来。
///
/// 已经拉到的记录不会因此被丢掉，只是不再声称那段区间被覆盖过。
///
/// # 重复 revision 保留**先到的那条**
///
/// SVN 历史不可变（spec §10.2），同一个 revision 两次拉到的内容应当相同。
/// 万一不同（有人改了 revprop），保留先到的那条比来回翻覆更可预测。
pub fn merge_into(target: &mut TargetLog, fetched: &[SvnLogEntry], from: u64, to: u64) {
    let (lo, hi) = if from <= to { (from, to) } else { (to, from) };

    for e in fetched {
        if !target.entries.iter().any(|c| c.rev == e.revision) {
            target.entries.push(CachedRev {
                rev: e.revision,
                author: e.author.clone(),
                date: e.date.clone(),
                message: e.message.clone(),
            });
        }
    }
    // 降序：新的 revision 在前，与 svn log 的默认顺序和图上的从上到下一致
    target.entries.sort_by_key(|e| std::cmp::Reverse(e.rev));

    if target.is_fresh() {
        target.covered_min = lo;
        target.covered_max = hi;
        return;
    }
    // 相接判定用 `+1`：覆盖到 8 之后拉 9..10，中间没有洞，可以并起来。
    let adjacent_or_overlapping =
        lo <= target.covered_max.saturating_add(1) && hi.saturating_add(1) >= target.covered_min;
    if adjacent_or_overlapping {
        target.covered_min = target.covered_min.min(lo);
        target.covered_max = target.covered_max.max(hi);
    } else {
        target.covered_min = lo;
        target.covered_max = hi;
    }
}

/// 所有 target 与锚点的并集，按 revision **降序**去重。
///
/// 这就是画图用的那条「SVN 序列」（spec §10.3 步骤 ①）。
pub fn merged_sequence(cache: &SvnLogCache) -> Vec<CachedRev> {
    let mut by_rev: BTreeMap<u64, CachedRev> = BTreeMap::new();
    for t in cache.targets.values() {
        for e in &t.entries {
            by_rev.entry(e.rev).or_insert_with(|| e.clone());
        }
    }
    for (rev, e) in &cache.anchors {
        by_rev.entry(*rev).or_insert_with(|| e.clone());
    }
    by_rev.into_values().rev().collect()
}
