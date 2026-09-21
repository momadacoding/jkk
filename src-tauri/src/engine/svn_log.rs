//! `svn log` 的拉取编排。
//!
//! 本模块只负责**打给谁、打什么区间、失败算谁的**；解析归 [`crate::engine::svn_xml`]，
//! 缓存归 [`crate::engine::svn_log_cache`]。
//!
//! # 为什么是「每个工作副本一次调用」
//!
//! 2026-09-15 实测（临时 fixture，SlikSvn 1.14.2）：
//!
//! | 写法 | 结果 |
//! |---|---|
//! | `svn log <url1> <url2>` | ❌ `E205000: Only relative paths can be specified after a URL` |
//! | `svn log <url> Code Other` | ✅ 成功，返回**并集**，降序去重 |
//! | `svn log . Code` | ❌ `E200007: When specifying working copy paths, only one target may be given` |
//! | `svn log <url> Code NoSuch` | ❌ `E160013: File not found` —— **一条坏路径炸掉整次调用** |
//!
//! 第二行那个形态看起来能把 N 次调用压成 1 次，**但没有采用**，理由是第四行：
//! 白名单里的目录被上游删掉、或在早期 revision 上还不存在，都是正常情况，
//! 而本设计的白名单**天然可能分布在不同的 external / 工作副本**里
//! （spec §6.6.5），URL 前缀本来就不止一个。
//! 每个 WC 一次调用的话，一个失败只损失那一个 WC 的历史。
//!
//! # ⚠️ 这里的 target 绝不能过 `svn_wc::peg_safe`
//!
//! 实测 R5：`svn log <URL> "Code@"` → `E160013: File not found: path '/Code@'`。
//! 与 `svn info` / `svn status`（**必须**加空 peg 后缀）正好相反。这是本仓库
//! 第三处 peg 规则互相矛盾的地方，另两处见 `svn_wc::peg_safe` 的文档。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::{parse_log, SvnLogEntry};
use crate::error::{GitError, Result};

/// 读某个 URL 当前的 HEAD revision。
///
/// 走 `svn info --show-item revision`：实测（P7）stdout 就是裸的十进制数字，
/// 不用过 XML 解析。
///
/// ⚠️ **`svn info` 绝不能带 `--ignore-externals`**（实测 exit 1，它拒绝这个参数）。
pub fn head_revision(repo_root: &str, url: &str) -> Result<u64> {
    let out = run_svn(repo_root, &["info", "--show-item", "revision", "--", url])?;
    let text = out.trim();
    text.parse::<u64>().map_err(|_| {
        GitError::new(
            "svn_log_bad_head",
            format!("svn info --show-item revision 返回了不是数字的内容：`{text}`"),
        )
    })
}

/// 拉一段区间的 log。**区间两端都会先被钳到 `[1, HEAD]`**，方向保持不变。
///
/// # 为什么必须钳制，而不是把错误抛上去
///
/// 实测 P4/P5：`-r` 的**任一端**越界，整条命令 `exit 1` 并报
/// `E160006: No such revision N`。而增量拉取的自然写法就是
/// 「上次覆盖到 N，这次拉 `N+1 : HEAD`」——没有新提交时 `N+1 > HEAD`，
/// 于是**每一次「没有新东西」都会变成一次错误**，界面上就会挂出一个
/// 「SVN 历史拉取失败」而其实什么事都没有。判成「拉到 0 条」才是它真实的含义。
///
/// `limit == 0` 同理：实测 P6 `--limit 0` 直接 `E200004`，而 limit 是调用方算出来
/// 的剩余配额，算到 0 是正常的。
pub fn fetch_range(
    repo_root: &str,
    url: &str,
    from: u64,
    to: u64,
    limit: u32,
) -> Result<Vec<SvnLogEntry>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let head = head_revision(repo_root, url)?;
    if head == 0 {
        // 空仓库。实测 P9：svn log 对 HEAD=0 返回 exit 0 + 空 <log>，
        // 但我们连命令都不用发。
        return Ok(Vec::new());
    }
    // **整段**都在 HEAD 之上时直接返回空。
    // 少了这一条，下面的钳制会把 `5:9`（HEAD=4）压成 `4:4`，于是「没有新东西」
    // 被拉回一条早就有的记录 —— 比报错更糟，因为它看起来是成功的。
    if from > head && to > head {
        return Ok(Vec::new());
    }
    let clamp = |v: u64| v.clamp(1, head);
    // 不排序、不交换：区间方向决定 svn 的输出顺序（实测 M3/M4），
    // 调用方按用途选方向，这里原样转发。
    let range = format!("{}:{}", clamp(from), clamp(to));
    let limit_s = limit.to_string();
    let xml = run_svn(
        repo_root,
        &["log", "--xml", "-r", &range, "--limit", &limit_s, "--", url],
    )?;
    parse_log(&xml)
}

/// 补拉一个**孤儿 baseline** 的锚点 commit。
///
/// # 这不是边界情况，是常态（2026-09-15 实测，C 组）
///
/// spec §10.3 把孤儿 baseline 描述成「若某个 baseline 的 revision **太老**」。
/// 成因与新旧无关：`svn update` 会把工作副本内**所有**节点的 `wc-status revision`
/// 一律推到当时的 HEAD，**不管那个 revision 有没有动过它们**。所以
/// `baselineRev`（取自 `nodeRevs`）通常就是一个根本没碰过白名单的 revision。
///
/// 实测对照：
///
/// ```text
/// svn log -r 4:4 <url>/Code            → 空          （Q1）
/// svn log -r 4:4 <Repository Root>     → 拿到 r4     （Q2）
/// ```
///
/// **所以锚点必须打给 Repository Root。** 打给白名单 URL 会拿到空结果，
/// 图上那个 `svn:N` 节点就永远建不出来，工作分支挂不上主干。
///
/// revision 高于 HEAD 时返回 `Ok(None)` 而不是 `Err`：缓存里记着的 baseline
/// 可能指向本仓库没有的 revision（换了远端、或仓库被重建过），那该跳过这一个，
/// 不该让整次拉取失败。
pub fn fetch_anchor(repo_root: &str, repos_root: &str, rev: u64) -> Result<Option<SvnLogEntry>> {
    let entries = fetch_range(repo_root, repos_root, rev, rev, 1)?;
    Ok(entries.into_iter().next())
}

/// 首屏与每次「加载更多」拉多少条（spec §10.2：首屏最近 200 条，不做无限滚动）。
pub const PAGE_SIZE: u32 = 200;

/// 补拉孤儿 baseline 锚点的条数上限。
///
/// spec §10.3 说 baseline commit 的 revision 列表「已知且有限（几十个）」。
/// 但那是对一个刚起步的仓库说的——跑了三年的仓库可能有几百个基线 commit，
/// 而每个锚点是**一次独立的网络往返**。没有上限的话，打开一次仓库要发几百次请求。
///
/// 超出时只补**最近的** N 个：图上越老的部分越少有人往下翻，而孤儿锚点缺失的
/// 后果只是那一段的工作分支挂不上主干，不影响正确性。
const MAX_ANCHORS: usize = 200;

/// 一页历史，外加「还能不能再往下翻」。
///
/// # 完整性判定必须由后端给
///
/// 它的依据是每个 target 的**连续覆盖区间**，而那个信息只存在于缓存里，
/// 前端手上只有一个去重合并后的 revision 列表。2026-09-16 独立 review 之前，
/// 前端拿「这一页有没有带回新东西」当判据，而翻页游标又取自含孤儿锚点的并集，
/// 两个凑在一起会在中间还差一大段时就把「到底了」点亮（见 [`fetch_log`]）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogPage {
    /// 合并后的完整序列（不只是这一页新增的），按 revision 降序。
    pub revs: Vec<crate::engine::svn_log_cache::CachedRev>,
    /// 全部 target 的连续覆盖区间都已经到 r1——再翻也没有了。
    pub complete: bool,
}

/// 读**缓存里**的 SVN 历史，不走网络。
///
/// 打开仓库时先用它把图画出来（spec §10.2）。读不到就是空列表——那只是
/// 「还没拉过」，不是错误。
pub fn read_log(repo_root: &str) -> Result<LogPage> {
    let uuid = repo_uuid(repo_root)?;
    let cache = crate::engine::svn_log_cache::read_cache(repo_root, &uuid);
    let targets = log_targets(repo_root)?;
    Ok(LogPage {
        revs: crate::engine::svn_log_cache::merged_sequence(&cache),
        complete: is_complete(&cache, &targets),
    })
}

/// 每个 target 的连续覆盖区间都到 r1 了吗。
///
/// **锚点不参与。** 锚点是逐个 revision 单点补拉的，彼此之间不连续，
/// 序列里有一个 r1 完全不代表 r2..r300 也拉过了。
fn is_complete(cache: &crate::engine::svn_log_cache::SvnLogCache, targets: &[String]) -> bool {
    !targets.is_empty()
        && targets.iter().all(|t| {
            cache
                .targets
                .get(t)
                .is_some_and(|e| e.covered_min == 1 && e.covered_max > 0)
        })
}

/// 拉取 SVN 历史、并进缓存、返回合并后的序列。**走网络。**
///
/// `older == false` 是增量拉新（比已缓存的更新的那一段）；`true` 是往下翻页。
///
/// # 翻页游标由这里算，不由调用方传
///
/// 2026-09-16 独立 review 的阻断项：原先的签名是 `before_rev: Option<u64>`，
/// 前端传「序列里最旧的那条 revision」。但那个序列是
/// **所有 target 的缓存 ∪ 孤儿锚点**，不代表任何一段连续覆盖：
///
/// ```text
/// 工作副本停在 r1、服务器到 r900：
///   首屏      → 目标覆盖 [701, 900]
///   孤儿锚点  → 基线 r1 单点补拉进来
///   序列      → [900..701, 1]        ← r2..r700 是空的
///   前端游标  → 1  ⇒ from = 0 ⇒ 一条命令都不发，还点亮「到底了」
/// ```
///
/// 多个 target 活跃程度不同时同样会发生：一个小目录早就翻到了 r1，
/// 整份并集的最小值就永远是 1。
///
/// 正确的游标是**每个 target 自己**连续覆盖区间的下界，各算各的——这个信息
/// 只在缓存里，所以只能由后端算。
///
/// # 单个 target 失败不中断其余的
///
/// 白名单可能分布在多个工作副本里（spec §6.6.5）。一条 external 的 URL 不可达
/// 不该让整条历史都拉不到——那会让图直接空掉。**全部 target 都失败才算整次失败。**
pub fn fetch_log(repo_root: &str, older: bool, limit: u32) -> Result<LogPage> {
    use crate::engine::svn_log_cache as cache_mod;

    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let wcs = crate::engine::svn_wc::enumerate_wcs(repo_root, &cfg.whitelist)?;
    let Some(first) = wcs.first() else {
        return Err(GitError::new(
            "svn_log_no_working_copy",
            "白名单没有覆盖到任何工作副本，拉不了历史。",
        ));
    };
    let uuid = first.uuid.clone();
    let repos_root = first.repos_root.trim_end_matches('/').to_string();
    let mut cache = cache_mod::read_cache(repo_root, &uuid);

    let targets = log_targets(repo_root)?;
    let mut failures: Vec<String> = Vec::new();
    for t in &targets {
        // 显式取一次 HEAD：下面要用它当区间上界，也要用它判断「有没有新东西」。
        // 交给 fetch_range 去钳制哨兵值的写法看着省事，但那样这里就不知道
        // 真实上界是多少，覆盖区间只能靠猜。
        let head = match head_revision(repo_root, t) {
            Ok(h) => h,
            Err(e) => {
                failures.push(format!("{t}: {}", e.message));
                continue;
            }
        };
        if head == 0 {
            continue; // 空仓库
        }
        let entry = cache.targets.entry(t.clone()).or_default();

        // ⚠️ **区间一律降序（from >= to）。**
        //
        // 2026-09-16 实测：`--limit` 是从**区间的起始方向**取的——
        //   `svn log -r 1:8 --limit 3` → r1,r2,r3（最旧的）
        //   `svn log -r 8:1 --limit 3` → r8,r7,r6（最新的）
        //
        // 用升序区间的话，revision 数超过一页的仓库首屏拿回来的是**最古老**的
        // 那一页，而 baseline 的 revision 在 HEAD 附近、不在里面，于是工作分支
        // 挂在一个没有链的孤零零锚点上。这是独立 review 抓到的阻断项。
        let (from, to) = if !older {
            // 增量拉新：HEAD 往下，拉到已覆盖上界的下一个。
            (head, entry.covered_max.saturating_add(1))
        } else if entry.covered_max == 0 {
            // 这个 target 还一次都没拉过（比如新加的 external）。
            // 「往下翻」对它而言就是首屏，从 HEAD 开始。
            (head, 1)
        } else {
            // 往下翻页：从**这个 target 自己**连续覆盖区间的下界再往下一个开始。
            // 下界给 1，实际能拉多少由 limit 决定。
            (entry.covered_min.saturating_sub(1), 1)
        };
        if from < to || from == 0 {
            // 增量：covered_max 已经到 HEAD，没有新东西。
            // 翻页：这个 target 已经翻到 r1 了。两种都不用发命令。
            continue;
        }

        match fetch_range(repo_root, t, from, to, limit) {
            Ok(got) => {
                // 覆盖区间的上界就是 `from`：即使 got 里最新的一条小于它，
                // 中间那些 revision 也只是没碰过这个 target，同样算查过了。
                //
                // 下界看这一页**有没有被 `--limit` 截断**：
                // - 返回条数少于 limit → 没截断，整段 `[to, from]` 都查过了，
                //   如实记成覆盖到 `to`。got 为空（这段没有属于该 target 的
                //   revision）也落在这一支；少了它，「这段没有提交」的 target
                //   会在每次刷新时重查同一段越来越大的区间。
                // - 返回条数正好等于 limit → **可能**被截断了，只能保守地取
                //   `min(got)`。比它更旧的那些根本没返回，声称覆盖到 `to` 会让
                //   下一次翻页直接跳过它们，那段历史永久缺失且无声。
                //   恰好不多不少时会多付一次往返，比永久丢历史便宜得多。
                let lo = if (got.len() as u32) < limit {
                    to
                } else {
                    got.iter().map(|e| e.revision).min().unwrap_or(to)
                };
                cache_mod::merge_into(entry, &got, lo, from);
            }
            Err(e) => failures.push(format!("{t}: {}", e.message)),
        }
    }
    if !failures.is_empty() && failures.len() == targets.len() {
        return Err(GitError::new(
            "svn_log_fetch_failed",
            format!("全部 target 的历史都没拉到：\n{}", failures.join("\n")),
        ));
    }

    fetch_missing_anchors(repo_root, &repos_root, &mut cache)?;
    cache_mod::write_cache(repo_root, &cache)?;
    Ok(LogPage {
        revs: cache_mod::merged_sequence(&cache),
        complete: is_complete(&cache, &targets),
    })
}

/// 把基线分支上那些**不在序列里**的 revision 逐个补拉成锚点。
///
/// 见 [`fetch_anchor`]：这不是边界情况，是常态。锚点缺失的表现是工作分支在图上
/// 挂不到 SVN 主干，而不是报错，所以这里的失败**只跳过、不上抛**。
fn fetch_missing_anchors(
    repo_root: &str,
    repos_root: &str,
    cache: &mut crate::engine::svn_log_cache::SvnLogCache,
) -> Result<()> {
    use crate::engine::svn_log_cache as cache_mod;

    let have: std::collections::BTreeSet<u64> = cache_mod::merged_sequence(cache)
        .iter()
        .map(|e| e.rev)
        .collect();

    // 基线分支不存在（还没建库）时这里拿不到输出，不是错误——直接没有锚点要补。
    let Ok(log) = crate::engine::git_cli::run_git(
        repo_root,
        &["log", "--format=%B%x00", "refs/heads/svn/base"],
    ) else {
        return Ok(());
    };
    let mut wanted: Vec<u64> = log
        .split('\0')
        .filter_map(crate::engine::svn_baseline::parse_trailer)
        .map(|t| t.rev)
        .filter(|r| !have.contains(r) && !cache.anchors.contains_key(r))
        .collect();
    wanted.sort_unstable_by(|a, b| b.cmp(a)); // 新的在前
    wanted.dedup();
    wanted.truncate(MAX_ANCHORS);

    for rev in wanted {
        // 失败只跳过：一个拿不到的锚点只损失图上一个节点。
        if let Ok(Some(entry)) = fetch_anchor(repo_root, repos_root, rev) {
            cache.anchors.insert(
                rev,
                cache_mod::CachedRev {
                    rev: entry.revision,
                    author: entry.author,
                    date: entry.date,
                    message: entry.message,
                },
            );
        }
    }
    Ok(())
}

/// 仓库 UUID，缓存的身份键。
fn repo_uuid(repo_root: &str) -> Result<String> {
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let wcs = crate::engine::svn_wc::enumerate_wcs(repo_root, &cfg.whitelist)?;
    wcs.first()
        .map(|w| w.uuid.clone())
        .ok_or_else(|| GitError::new("svn_log_no_working_copy", "白名单没有覆盖到任何工作副本。"))
}

/// 本仓库要拉哪些 target：每个白名单所属工作副本的 URL，各一条，已去重。
///
/// 去重用大小写不敏感比较——同一个 URL 的大小写写法在 Windows 上会有出入，
/// 重复的 target 会让同一段历史被拉两遍并在缓存里占两份。
pub fn log_targets(repo_root: &str) -> Result<Vec<String>> {
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let wcs = crate::engine::svn_wc::enumerate_wcs(repo_root, &cfg.whitelist)?;
    let mut out: Vec<String> = Vec::new();
    for wc in wcs {
        let url = wc.url.trim_end_matches('/').to_string();
        if !out.iter().any(|u| u.eq_ignore_ascii_case(&url)) {
            out.push(url);
        }
    }
    Ok(out)
}
