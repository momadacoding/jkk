//! 「jkk 与 TortoiseSVN 看法不一致」的文件：基线可能记错了的症状（设计
//! `docs/superpowers/specs/2026-11-25-svn-baseline-mismatch-design.md` §6）。
//!
//! # 为什么需要它
//!
//! 体检（`svn_init::check_current_baseline`）是发现「基线记的内容与服务器不符」的唯一入口，
//! 但它原来是一条常驻横幅，等用户自己想起来去点。用户没有理由怀疑，也就不会去点。这里找的是
//! 用户**看得见**的那个症状：jkk 的「本地改动」里有一个文件，TortoiseSVN 却说它没改过。
//!
//! # 判据（设计 §6.1 第 2–4 条）
//!
//! ```text
//! git 说改了   磁盘 ≠ HEAD                      git diff --diff-filter=M HEAD
//! 分支没碰过   HEAD:p == svn/base:p              git diff-tree svn/base HEAD 里没有 p
//! SVN 说干净   item=normal、props=none、非 copied  此刻现跑的 svn status -v
//! ```
//!
//! 三条同时成立时：磁盘等于 pristine（SVN 说的），HEAD 等于基线（分支没碰），而磁盘 ≠ HEAD，
//! 所以基线 ≠ pristine——基线记的不是 SVN 手上的内容。
//!
//! 第 1 条前提（同步正常、分支不落后基线、没有未收尾的同步）由前端判定后才来调这里，
//! 见 `svnSync.ts::mismatchCheckApplies`。前提不成立时这三条会误报：例如 TortoiseSVN 刚
//! update 下来、jkk 还没跟进的文件同样满足三条。前端手上的失同步结论可能还没跟上（见
//! [`same_revision`]），所以这里另加一条：SVN 手上的 revision 必须与基线记的对得上。
//!
//! # 为什么 svn status 必须现跑
//!
//! `svn status` 的 modified 是查询那一刻拿磁盘和 pristine 比出来的；用户改文件不写 wc.db，
//! 失同步检测那遍扫描（按 wc.db 戳才重跑）此时已经过时。复用它，用户刚改的每个文件都会被
//! 报成「基线记错了」。所以只对候选文件现跑一次，不进扫描缓存。

use crate::engine::git_cli::run_git;
use crate::engine::svn_basemap::read_base_map;
use crate::engine::svn_cli::{chunk_targets, run_svn_fresh, MAX_TARGETS_CMDLINE};
use crate::engine::svn_config::read_config;
use crate::engine::svn_scope::WScope;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_node_status, parse_status_entries, NodeStatus};
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// 一次最多现查多少个候选文件。
///
/// 候选是「git 有改动、分支提交没碰过」的 W-file，正常就是用户手上正在改的那几个。上千个
/// 只会出现在批量替换之类的时候，那时每次刷新都对几千个文件跑 `svn status` 不划算；
/// 只查前一部分，查到的照样报，没查到的由手动体检兜底。
pub const MAX_MISMATCH_CANDIDATES: usize = 1000;

/// 返回给横幅的文件数上限。
pub const MAX_MISMATCH_FILES: usize = 50;

/// 判定结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineMismatch {
    /// 命中的文件（仓库相对、正斜杠、升序），最多 [`MAX_MISMATCH_FILES`] 个。
    pub files: Vec<String>,
    /// 命中的总数（截断前）。
    pub total: usize,
    /// 候选超过 [`MAX_MISMATCH_CANDIDATES`]、只查了一部分。
    pub truncated: bool,
}

impl BaselineMismatch {
    fn none() -> Self {
        Self {
            files: Vec::new(),
            total: 0,
            truncated: false,
        }
    }
}

/// 找出「git 说改了、SVN 说没改、分支提交没碰过」的 W-file。纯读，不碰索引、HEAD 与磁盘。
///
/// 候选为空时不起 svn 进程：用户没改东西、或改的都已经提交在分支上，是最常见的情形。
pub fn find_baseline_mismatch(repo_root: &str) -> Result<BaselineMismatch> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库，没有基线可比。",
        ));
    }
    let cfg = read_config(repo_root)?;
    let base = crate::engine::svn_sync::baseline_oid(repo_root)?;

    // 磁盘 ≠ HEAD。只要 M：新增（A）不在 SVN 里、删除（D）在 SVN 那边是 missing，都不可能
    // 「SVN 说干净」；改名关掉，两边都按原路径比
    let dirty = nul_list(&run_git(
        repo_root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            "--diff-filter=M",
            "HEAD",
            "--",
        ],
    )?);
    if dirty.is_empty() {
        return Ok(BaselineMismatch::none());
    }
    // 只看路径（白名单 ∧ 扩展名），不问 .gitignore：这些文件已经被 git 跟踪，.gitignore
    // 管不到它们。白名单外的文件 SVN 侧本来就不归基线管
    let scope = WScope::plain(&cfg.whitelist, &cfg.extensions);
    let w_dirty: Vec<&str> = scope.w_paths(dirty.iter().map(String::as_str));
    if w_dirty.is_empty() {
        return Ok(BaselineMismatch::none());
    }

    // 分支自己的提交碰过的文件。前端只在当前分支包含 svn/base 时调这里（前提第 1 条），
    // 所以 svn/base..HEAD 就是分支自己的提交，直接比两棵树
    let touched: BTreeSet<String> = nul_list(&run_git(
        repo_root,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--name-only",
            &base,
            "HEAD",
        ],
    )?)
    .into_iter()
    .collect();

    let mut candidates: Vec<String> = w_dirty
        .into_iter()
        .filter(|p| !touched.contains(*p))
        .map(str::to_string)
        .collect();
    if candidates.is_empty() {
        return Ok(BaselineMismatch::none());
    }
    candidates.sort();
    let truncated = candidates.len() > MAX_MISMATCH_CANDIDATES;
    candidates.truncate(MAX_MISMATCH_CANDIDATES);
    let wanted: BTreeSet<&str> = candidates.iter().map(String::as_str).collect();

    // 记账与树取自同一个 OID（同 `check_current_baseline`）
    let map = read_base_map(repo_root, &base)?;

    // `-v`：不带它干净文件不输出，「没输出」与「不在工作副本里」分不开。不带 `--ignore-externals`：
    // target 是文件本身，落在 external 里的文件也要按它自己的工作副本报
    let targets: Vec<String> = candidates.iter().map(|p| peg_safe(p)).collect();
    let mut hits: BTreeSet<String> = BTreeSet::new();
    for batch in chunk_targets(STATUS_HEAD, &targets, MAX_TARGETS_CMDLINE) {
        let mut args: Vec<&str> = STATUS_HEAD.to_vec();
        args.extend(batch.iter().map(String::as_str));
        // 现跑，不读扫描缓存、不进缓存（模块头「为什么 svn status 必须现跑」）。这条命令本来
        // 只在自己的 Tauri 线程上跑、不在任何缓存范围里，但不靠这个隐含前提
        let xml = run_svn_fresh(repo_root, &args)?;
        // 节点解析有两道为全量扫描设的守卫：一个带 revision 的节点都没有就报错（防漏了 `-v`）。
        // 这里的 target 是逐个文件，整批都是 SVN 不认识或刚 add 的文件时同样触发，那只是「这批
        // 没有命中」：它们本来就不是「SVN 说完全干净」（复核 review 的建议，用例
        // `a_candidate_that_svn_does_not_know_does_not_fail_the_query`）
        let nodes = match parse_node_status(&xml) {
            Ok(nodes) => nodes,
            Err(e)
                if matches!(
                    e.code.as_str(),
                    "svn_xml_empty_node_revs" | "svn_xml_all_nodes_uncommitted"
                ) =>
            {
                continue
            }
            Err(e) => return Err(e),
        };
        let revs: BTreeMap<String, NodeStatus> = nodes
            .into_iter()
            .map(|n| (n.path.trim_end_matches('@').to_string(), n))
            .collect();
        for e in parse_status_entries(&xml)? {
            let path = e.path.trim_end_matches('@');
            // 完全干净才算「SVN 说没改」：只改了属性的是 normal + props=modified，复制来的
            // 是 normal + copied（服务器上根本没有这个路径），都不等于服务器内容
            if !(e.item == "normal" && e.props == "none" && !e.copied && wanted.contains(path)) {
                continue;
            }
            if revs
                .get(path)
                .is_some_and(|n| same_revision(n, map.rev_of(path)))
            {
                hits.insert(path.to_string());
            }
        }
    }
    let total = hits.len();
    Ok(BaselineMismatch {
        files: hits.into_iter().take(MAX_MISMATCH_FILES).collect(),
        total,
        truncated,
    })
}

/// 现查候选用的 `svn status`，target 接在后面。
const STATUS_HEAD: &[&str] = &["status", "-v", "--xml", "--"];

/// SVN 手上的这个文件与基线记的是不是同一个 revision 的内容（判据与失同步判定 `svn_sync::judge`
/// 相同：`c(f) > R(f)` 是前进，`n(f) < R(f)` 是回退）。
///
/// # 为什么判定自己也要查，不能只靠前端的前提门槛（2026-11-25 独立 review 阻断项）
///
/// TortoiseSVN update 之后，文件事件同时触发「本地改动」刷新和失同步检测；前者很快，后者要对
/// 整个白名单跑 `svn status -v`，公司机器上几秒，wc.db 的最后一笔写入还要等尾随复查。这段时间
/// 前端手上的失同步结论还是旧的 InSync，前提门槛放行，刚 update 下来的文件就会被报成「基线记
/// 错了」，出口还是重建基线。用户每天早上都 update，这是每天必现的误报。
///
/// 刚 update 下来的文件 `c(f) > R(f)`，在这里被丢掉；基线真的记错时 revision 是对得上的
/// （这正是这条症状存在的理由：revision 对得上，内容不对）。
fn same_revision(n: &NodeStatus, r: u64) -> bool {
    match (n.node_rev, n.commit_rev) {
        (Some(node), Some(commit)) => node >= r && commit <= r,
        // 没有服务器 revision（本地新增、复制来的）不会走到这里（上面已经要求 normal 且非 copied），
        // 万一走到，按「对不上」处理：宁可漏报，不误报
        _ => false,
    }
}

/// `-z` 输出切成路径，去掉空项。
fn nul_list(out: &str) -> Vec<String> {
    out.split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
