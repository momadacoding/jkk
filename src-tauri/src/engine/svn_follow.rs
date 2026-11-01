//! 跟进外部提交（分析 §6.5、§6.11.1、§6.11.3；计划 Task 10；日志「偏离计划」的发布顺序）。
//!
//! 用户在 TortoiseSVN 里只提交、不更新，或者只更新某个子目录之后，把基线推进到各文件的真实
//! revision，把分支重放上去，暂存区拼回来。**全程不写磁盘，不发 SVN 写命令。**
//!
//! 唯一的例外是冲突兜底 [`manual_follow`]：它走 stash + rebase，会写磁盘，只由用户显式发起。
//!
//! # 两段
//!
//! ```text
//! 准备（不拿锁，只建对象，不动任何 ref）
//!   预检 → F1 录新基线（含联网体检）→ F2 重放当前分支 → 其他分支 → F3 泄漏
//! 发布（持同步锁，只有几次 ref 与索引写入）
//!   F4 写 journal、锁内复核
//!   F5 CAS svn/base → prepare_index → 核对 HEAD → CAS 当前分支 → 再核对 HEAD
//!      → rename 换入索引 → CAS 其他分支
//!   F6 删快照、修剪钉住的 ref、删 journal
//! ```
//!
//! 联网和重放放在锁外：同步锁拿着的时候用户在 jkk 里的暂存、提交都会被挡回来，而准备阶段只建
//! 对象，什么都没动，失败了什么都不用恢复。这也是 [`FollowOutcome::Refused`] 的含义：**一个
//! ref 都没动**。动过之后的失败走 `Err`，文案说明此刻的状态。
//!
//! # 为什么分支先于索引
//!
//! 原设计是「基线 → 换索引 → 移分支」，日志「偏离计划」改成了现在的顺序。拼好的索引留在
//! `.git/index.lock` 里，从拼索引到 rename 一直持锁，外部 `git add` / `commit` 进不来（W23），
//! 「分支已移、索引是旧的」的窗口对外部 git 不存在。而分支 CAS 失败时什么都还没动，删锁即可。
//!
//! 唯一绕得过 index.lock 的是 `checkout -b` / `symbolic-ref`（W23），所以 HEAD 前后各核对一次。
//! `checkout -b` 落在 CAS 之后时新分支建在 NEW_TIP 上：拼好的索引正好对得上它，往前走；这时把
//! 分支移回去反而造出反向暂存（Review 2 第 3 条）。只有 HEAD 真的不在 NEW_TIP 上才回滚。
//!
//! # 崩溃恢复
//!
//! 每一步之前把步骤写进 journal（[`FollowStep`]）。恢复时的不变量：**绝不留下「分支已移、索引是
//! 旧的、index.lock 不在」**（Review Focus 5）：外部工具这时提交，就是一个真实的回退 commit。
//!
//! - 走到 [`FollowStep::PrepareIndex`] 为止：只有基线可能动过，这是一致状态。journal 记过
//!   [`FollowSteps::lock_created`] 时残留的 index.lock 是我们的半成品，只能删；没记过就是别人的，
//!   不碰，报 `svn_index_busy`。清掉记录重新跟进，基线已经发布的话走「分支跟进」入口
//! - [`FollowStep::MoveBranch`] / [`FollowStep::SwapIndex`]：index.lock 的内容与 journal 记的
//!   blob 对得上，就是拼好的那一份，往前补做；对不上就看索引内容是不是拼好的那份，判断 rename
//!   发生过没有：发生过往前走，没发生过把分支移回去再重新跟进。HEAD 已经被 `checkout -b` 带到
//!   NEW_TIP 上时移不回去，只能往前，拼好的索引没了就照 OLD → NEW_TIP 重拼一份
//! - [`FollowStep::MoveOthers`]：每条分支都是 CAS，补做是幂等的
//!
//! 恢复没有「放弃」这条路（C1a 没有这个入口），所以任何分支都必须能走到删掉 journal：卡住的
//! journal 会挡死更新与交付。
//!
//! # 写操作的路径
//!
//! 本模块全程持锁或者不写，不走 `trait Vcs`（理由同 `svn_sync_in` 模块头）。watcher 由命令层
//! 用 `with_watch_suspended` 包住。

use crate::engine::git_cli::{run_git, run_git_bytes, run_git_code, run_git_env, run_git_stdin};
use crate::engine::svn_baseline::{
    create_baseline_commit_with, git_dir, move_baseline_ref, TempIndex,
};
use crate::engine::svn_basemap::{read_base_map, BaseMap};
use crate::engine::svn_cli::with_scan_cache;
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_follow_index::{
    prepare_index, prepare_index_with, rename_lock_into_index, ComposeReport, PathFacts,
    PreparedIndex,
};
use crate::engine::svn_follow_mark::{clear_mark, my_svn_user, read_mark, FollowMark};
use crate::engine::svn_health::Health;
use crate::engine::svn_include_check::normalize_roots;
use crate::engine::svn_init::{record_mixed, Recorded};
use crate::engine::svn_journal::{self, BranchMove, Flow, FollowStep, FollowSteps, Journal, Phase};
use crate::engine::svn_leak::{local_only_leaks, Leak};
use crate::engine::svn_lock;
use crate::engine::svn_nodes::{compute_map, read_node_status, NodeStatus};
use crate::engine::svn_preflight::run_preflight_with;
use crate::engine::svn_replay::{
    all_committed, commits_between, replay_onto, ReplayCodes, ReplayCommit, ReplayOpts,
    ReplayOutcome,
};
use crate::engine::svn_sync::{
    check_sync_with_nodes, read_baseline_map, tree_files, ExternalChange, SyncVerdict,
};
use crate::engine::svn_sync_check::{
    assert_linear, check_git_idle, current_branch, list_paths, rev_parse,
};
use crate::engine::svn_sync_in::{
    from_baseline_ready, is_ancestor, need, new_opid, no_unfinished, SyncInOutcome,
};
use crate::engine::svn_sync_stash::stash_wip;
use crate::engine::svn_wc::enumerate_wcs;
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};
use git2::{BranchType, Repository};
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 重放给跟进的错误码。`too_old` 是兜底：预检已经挡在 F1 之前。
const FOLLOW_CODES: ReplayCodes = ReplayCodes {
    conflict: "svn_follow_conflict",
    too_old: "svn_follow_git_too_old",
    too_old_message: "跟进需要 git ≥ 2.43，请升级 git。",
    failed: "svn_follow_replay_failed",
};

/// 跟进要的最低 git：`merge-file --object-id` 要 2.43（比 `merge-tree --merge-base` 的 2.40
/// 新，日志 D9）。必须在 F1 之前报：`prepare_index` 走到这里时基线已经发布了。
const MIN_GIT: (u32, u32) = (2, 43);

/// 准备期间状态变了，回到 F1 重来的总次数（分析 §6.5 F4：最多重来一次）。
const MAX_ATTEMPTS: usize = 2;

/// 钉住旧索引的 ref 保留多少【待决 D3】。
const KEEP_PINS: usize = 20;

/// 基线 commit 说明里列多少个 revision、点名多少人。说明要给人看，映射才是记账。
const SUMMARY_REVISIONS: usize = 10;
const SUMMARY_NAMES: usize = 5;

/// 索引内容的指纹用的命令：只含模式、OID、stage、路径，stat 不进来。
const LS_STAGE: &[&str] = &["ls-files", "--stage", "-z"];

/// `svn_follow` / `svn_follow_continue` 的结局。
///
/// `report` 装箱：它比另两个变体大得多（clippy `large_enum_variant`），序列化时看不出来。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FollowOutcome {
    Done {
        report: Box<FollowReport>,
    },
    /// 当前分支重放冲突，要人来判断，由用户发起 [`manual_follow`]。基线已发布，分支与索引没动。
    NeedsManual {
        /// 冲突的原 commit。`stale_disk` 时是当前 HEAD。
        at: String,
        /// 冲突的文件；`stale_disk` 时是磁盘上还是旧分支内容的文件。
        files: Vec<String>,
        baseline_published: bool,
        /// F3：分支没有重放，但 Local-Only 补丁可能已经被带上了服务器，用户要在动手之前知道。
        leaks: Vec<Leak>,
        /// 重放没冲突，但分支落后、磁盘上还是旧 HEAD 的内容（见 [`stale_disk_paths`]）：对象层跟进
        /// 只移 HEAD，「本地改动」会变成把 SVN 的内容改回去（Review 3 第 2 条）。
        stale_disk: bool,
    },
    /// 什么都没动：前置条件不满足，或者准备阶段失败。
    Refused {
        code: String,
        message: String,
    },
}

/// 跟进做了什么（分析 §6.10、§6.12 的通知内容）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowReport {
    pub new_floor: u64,
    /// 这次录的基线经过了服务器体检。断网时是 `false`：基线照样发布，但要告诉用户。
    pub verified: bool,
    pub changes: Vec<ExternalChange>,
    /// 外部删除的文件。没有 revision 与作者（status 里已经没有它们了），不进 `changes`。
    pub removed: Vec<String>,
    /// 变空丢弃的原 commit（「已全部提交」捷径收掉的也在内）。
    pub dropped: Vec<String>,
    /// (原 commit, 换成 SVN 版本的路径)。
    pub rewritten: Vec<(String, Vec<String>)>,
    pub stage: ComposeReport,
    /// 移动过的其他分支。当前分支不在内。
    pub moved_branches: Vec<String>,
    /// (分支, 原因)：不移动，分支列表标「落后基线」。
    pub behind_branches: Vec<(String, String)>,
    pub leaks: Vec<Leak>,
    /// 钉着换入之前的原始索引与暂存 blob 的 ref。没拼过索引时（只发布了基线）是 `None`。
    #[serde(default)]
    pub pinned_ref: Option<String>,
}

/// 外部 git 的版本够不够。解析不了按不够：宁可让用户看一眼版本，也不要发布到一半才失败。
pub fn git_version_ok(text: &str) -> bool {
    let Some(rest) = text.trim().strip_prefix("git version ") else {
        return false;
    };
    let mut parts = rest.split('.').map(|p| p.parse::<u32>());
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= MIN_GIT,
        _ => false,
    }
}

/// 跟进一次：判定为 ExternallyAdvanced 时走 F1–F6，InSync 而分支落后时走「分支跟进」入口。
pub fn follow(repo: &str, opid: &str) -> Result<FollowOutcome> {
    for _ in 0..MAX_ATTEMPTS {
        // 准备阶段里同一条 svn 扫描只跑一次（性能 P2）；发布前的复核在范围外，照常读最新的
        let plan = match with_scan_cache(|| prepare(repo)) {
            Ok(p) => p,
            Err(e) => return Ok(refused(e)),
        };
        fault::reach(fault::Point::Prepared, None);
        // 分支跟进入口上重放冲突：基线本来就在那里，没有要发布的东西
        if let (
            true,
            Target::Conflict {
                at,
                files,
                leaks,
                stale_disk,
            },
        ) = (plan.old_base == plan.new_base, &plan.target)
        {
            return Ok(needs_manual(at, files, leaks, *stale_disk));
        }
        // 同步锁活到 publish 返回：冲突出口这种提前返回也在 Drop 里放掉
        let _lock = match svn_lock::acquire(repo, opid) {
            Ok(l) => l,
            Err(e) => return Ok(refused(e)),
        };
        match still_valid(repo, &plan) {
            Ok(true) => return publish(repo, opid, plan),
            Ok(false) => continue,
            Err(e) => return Ok(refused(e)),
        }
    }
    Ok(FollowOutcome::Refused {
        code: "svn_follow_state_changed".to_string(),
        message: "准备跟进期间仓库一直在变（有别的工具在提交或切分支？）。这次什么都没动，\
                  等它们做完再点一次「跟进」。"
            .to_string(),
    })
}

/// 把崩溃留下的跟进走完，或者回到一致状态之后重新跟进。
pub fn resume_follow(repo: &str) -> Result<FollowOutcome> {
    let first = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    if first.flow != Flow::Follow {
        return Err(wrong_flow());
    }
    let next = {
        let _lock = svn_lock::acquire(repo, &first.opid)?;
        let mut j = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
        if j.opid != first.opid || j.flow != Flow::Follow {
            return Err(wrong_flow());
        }
        resume_locked(repo, &mut j)?
    };
    match next {
        Resumed::Finished(o) => Ok(o),
        // follow 自己要拿同步锁，上面那把已经放了
        Resumed::Again => follow(repo, &new_opid()),
    }
}

fn wrong_flow() -> GitError {
    GitError::new(
        "svn_follow_wrong_flow",
        "这条记录不是「跟进」留下的，请用顶部横幅上对应的按钮处理。",
    )
}

fn not_hybrid() -> GitError {
    GitError::new("svn_not_hybrid", "这个目录不是已初始化的混合仓库。")
}

fn unfinished() -> GitError {
    GitError::new(
        "svn_follow_unfinished",
        "上一次同步还没有收尾。请先在顶部横幅上处理，再跟进：两次同步的记录不能叠在一起。",
    )
}

fn refused(e: GitError) -> FollowOutcome {
    FollowOutcome::Refused {
        code: e.code,
        message: e.message,
    }
}

fn needs_manual(at: &str, files: &[String], leaks: &[Leak], stale_disk: bool) -> FollowOutcome {
    FollowOutcome::NeedsManual {
        at: at.to_string(),
        files: files.to_vec(),
        baseline_published: true,
        leaks: leaks.to_vec(),
        stale_disk,
    }
}

/// 手动跟进（分析 §6.5；计划 Task 13）：对象层重放冲突时的兜底，需要人来判断，所以**只由用户显式
/// 发起，永远不自动执行**。
///
/// 前提：`svn/base` 已发布，当前分支落后它（[`FollowOutcome::NeedsManual`] 的现场）。流程是
/// `sync_in` 去掉基线那一段：
///
/// ```text
/// S1 stash → 不切基线、不跑 svn update → S9 rebase --empty=drop <svn/base> → S10 apply → S11
/// ```
///
/// 基线不动，它是真实的：「放弃」只把分支和改动放回原处。冲突、撞名、继续与放弃都走 `sync_in`
/// 的状态机，journal 的 `flow` 是 [`Flow::FollowManual`]：`continue_sync` / `abort_sync` 接手，
/// `resume_follow` 与 `sync_out` 拒绝（日志 Task 13「偏离计划」）。
pub fn manual_follow(repo: &str, opid: &str) -> Result<SyncInOutcome> {
    if detect_repo_kind(repo) != RepoKind::SvnHybrid {
        return Err(not_hybrid());
    }
    let _lock = svn_lock::acquire(repo, opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished());
    }
    check_git_idle(repo, "svn_follow_git_busy", "手动跟进")?;
    require_index_free(repo)?;
    let branch = attached_branch(repo)?;
    let head = rev_parse(repo, "HEAD")?;
    let base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    if is_ancestor(repo, &base, &head)? {
        return Err(GitError::new(
            "svn_follow_manual_not_needed",
            "当前分支已经在基线上，不需要手动跟进。",
        ));
    }
    let scope = branch_baseline(repo, &head, &base)?;
    require_linear(repo, &scope, &head)?;
    let cfg = read_config(repo)?;
    if !matches!(check_sync_with_nodes(repo)?.0.verdict, SyncVerdict::InSync) {
        return Err(GitError::new(
            "svn_follow_manual_stale",
            "基线和工作副本又对不上了（可能又有新的外部提交，或者 SVN 操作还没做完）。手动跟进只把\
             分支重放到已发布的基线上，请先点一次「跟进」。什么都没有改动。",
        ));
    }
    // 这个入口不录制，分析 §6.5 的前置条件没有别处把关（同分支跟进入口）
    run_preflight_with(repo, &cfg.whitelist, true)?;
    require_changes_in_whitelist(repo, &cfg.whitelist)?;

    let floor = read_base_map(repo, &scope)?.floor;
    let mut j = Journal {
        opid: opid.to_string(),
        flow: Flow::FollowManual,
        phase: Phase::Start,
        started_at: now_secs(),
        orig_branch: Some(branch),
        orig_head: Some(head),
        baseline_rev_before: Some(floor),
        baseline_oid_before: Some(scope),
        new_baseline_oid: Some(base),
        ..Default::default()
    };
    svn_journal::write(repo, &j)?;
    stash_wip(repo, &mut j, &cfg.whitelist)?;
    fault::reach(fault::Point::ManualStashed, None);
    // S8 在这里不切分支（HEAD 一直在原分支上），只做复核；S9 重放到 `newBaselineOid`
    from_baseline_ready(repo, &cfg, &mut j)
}

/// S1 的 stash 只收白名单，rebase 却要求整个工作区干净，白名单外的已跟踪改动放过去就是 stash 完才
/// 失败（`rebase.autoStash=false` 时 rebase 拒绝启动）。在动手之前挡住。
fn require_changes_in_whitelist(repo: &str, whitelist: &[String]) -> Result<()> {
    let names = |args: &[&str]| -> Result<BTreeSet<String>> {
        Ok(run_git(repo, args)?
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect())
    };
    let roots = normalize_roots(whitelist);
    if roots.is_empty() {
        return Ok(()); // 空 pathspec 会变成「全部」（§12.2）；空白名单由 `stash_wip` 拒绝
    }
    // 圈内圈外让 git 判，与 `stash push -- <roots>` 同一口径（大小写、路径规范化都一致）
    let mut inside: Vec<&str> = vec!["diff-index", "--name-only", "-z", "HEAD", "--"];
    inside.extend(roots.iter().map(String::as_str));
    let inside = names(&inside)?;
    let all = names(&["diff-index", "--name-only", "-z", "HEAD"])?;
    let outside: Vec<String> = all.difference(&inside).cloned().collect();
    if outside.is_empty() {
        return Ok(());
    }
    Err(GitError::new(
        "svn_follow_manual_outside_changes",
        format!(
            "白名单之外有已跟踪文件的改动，手动跟进收不走它们，rebase 也就启动不了：\n{}\n\
             请先提交或撤销这些改动。什么都没有改动。",
            list_paths(&outside)
        ),
    ))
}

// —— 准备阶段 ——

enum Target {
    /// 当前分支要移到的 NEW_TIP。
    Tip(String),
    /// 当前分支重放冲突（或磁盘上还是旧内容，`stale_disk`），只发布基线。
    Conflict {
        at: String,
        files: Vec<String>,
        leaks: Vec<Leak>,
        stale_disk: bool,
    },
}

struct Plan {
    branch: String,
    old_head: String,
    /// `svn/base` 的 CAS 旧值。等于 `new_base` 时是分支跟进入口，不动基线。
    old_base: String,
    new_base: String,
    target: Target,
    others: Vec<BranchMove>,
    report: FollowReport,
    /// 这次录的映射。锁内要再核对一遍（R3c）；分支跟进没有录制。
    recorded: Option<BaseMap>,
    who: Who,
}

/// 判定「是不是我提交的」「早于外部提交吗」的线索（计划 Task 9）。全部拿不准时走保守分支。
struct Who {
    me: Option<String>,
    mark: Option<FollowMark>,
    /// 路径 → 最后一次提交的作者（`svn status -v` 的 `<commit author>`）。
    authors: BTreeMap<String, Option<String>>,
    /// 快照之后才进基线的路径（见 [`paths_after_mark`]）。`None`：分不清快照与外部提交的先后。
    after_mark: Option<BTreeSet<String>>,
}

impl Who {
    fn gather(repo: &str, repos_root: &str, nodes: &[NodeStatus]) -> Result<Self> {
        Ok(Self {
            me: my_svn_user(repo, repos_root),
            mark: read_mark(repo)?,
            authors: nodes
                .iter()
                .map(|n| (n.path.clone(), n.author.clone()))
                .collect(),
            after_mark: None,
        })
    }

    /// 匿名提交不可能是我：`Some(false)`。用户名取不到、节点不在：`None`（D4）。
    fn is_me(&self, path: &str) -> Option<bool> {
        let me = self.me.as_deref()?;
        let author = self.authors.get(path)?;
        Some(author.as_deref() == Some(me))
    }

    /// 拼索引要的事实。`staged` 是当前索引的暂存 blob，只在持索引锁时读一次（见 [`staged_blobs`]）。
    fn facts(&self, path: &str, staged: Option<&BTreeMap<String, String>>) -> PathFacts {
        let after_mark = self.after_mark.as_ref().is_some_and(|s| s.contains(path));
        PathFacts {
            author_is_me: self.is_me(path),
            // 这条路径的 SVN 提交早于快照时，快照里的暂存可能是提交之后的新内容，推不出先后
            staged_before_mark: self.mark.as_ref().filter(|_| after_mark).and_then(|m| {
                // 快照把暂存的删除记成全 0；索引里没有这条路径就是暂存了删除
                let zero = "0".repeat(m.head_oid.len());
                let now = staged?.get(path).unwrap_or(&zero);
                Some(m.staged.get(path) == Some(now))
            }),
        }
    }
}

/// 快照之后才进基线的路径：`diff-tree <mark.base_oid> NEW`。
///
/// 「早于快照」只有在快照早于这批 SVN 提交时才说明「早于 SVN 提交」。判定 InSync 时写的快照可以
/// 比 SVN 提交新（基线已发布、分支落后），这时快照里的 commit 与暂存完全可能是提交之后的新工作
/// （Review 2 第 1 条）。旧格式没有 `base_oid`、读不到、不是 NEW 的祖先：`None`，一律算拿不准。
fn paths_after_mark(repo: &str, mark: Option<&FollowMark>, new: &str) -> Option<BTreeSet<String>> {
    let base = mark?.base_oid.as_deref()?;
    match is_ancestor(repo, base, new) {
        Ok(true) => {}
        Ok(false) => return None,
        Err(e) => {
            log::warn!("失焦快照里的基线读不出来，按拿不准处理：{}", e.message);
            return None;
        }
    }
    let args = [
        "diff-tree",
        "-r",
        "-z",
        "--no-renames",
        "--name-only",
        base,
        new,
    ];
    match run_git(repo, &args) {
        Ok(out) => Some(
            out.split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
        ),
        Err(e) => {
            log::warn!("比不出快照之后的基线变化，按拿不准处理：{}", e.message);
            None
        }
    }
}

/// 当前索引里 stage 0 的 `路径 → blob`。读不出来时 `None`，走「拿不准」。
fn staged_blobs(repo: &str) -> Option<BTreeMap<String, String>> {
    let raw = run_git_bytes(repo, &["ls-files", "--stage", "-z"]).ok()?;
    let mut map = BTreeMap::new();
    for rec in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let rec = String::from_utf8_lossy(rec);
        let (meta, path) = rec.split_once('\t')?;
        let mut f = meta.split(' ');
        let (_, oid, stage) = (f.next()?, f.next()?, f.next()?);
        if stage == "0" {
            map.insert(path.to_string(), oid.to_string());
        }
    }
    Some(map)
}

/// 从 `from` 到 `to` 真实变化的文件。作者取节点状态，不看判定：判定里 c(f) 变了不等于内容变了
/// （只改属性、提交了相同内容），而 P 与 F3 问的都是内容（计划 Task 10：只有真实内容变了的才算）。
struct External {
    changes: Vec<ExternalChange>,
    removed: Vec<String>,
}

impl External {
    fn read(repo: &str, from: &str, to: &str, nodes: &[NodeStatus]) -> Result<Self> {
        let by_path: BTreeMap<&str, &NodeStatus> =
            nodes.iter().map(|n| (n.path.as_str(), n)).collect();
        let out = run_git(
            repo,
            &[
                "diff-tree",
                "-r",
                "-z",
                "--no-renames",
                "--name-status",
                from,
                to,
            ],
        )?;
        let mut changes = Vec::new();
        let mut removed = Vec::new();
        let mut fields = out.split('\0').filter(|f| !f.is_empty());
        while let (Some(status), Some(path)) = (fields.next(), fields.next()) {
            if status == "D" {
                removed.push(path.to_string());
                continue;
            }
            let node = by_path.get(path);
            changes.push(ExternalChange {
                path: path.to_string(),
                rev: node
                    .and_then(|n| n.commit_rev.or(n.node_rev))
                    .unwrap_or_default(),
                author: node.and_then(|n| n.author.clone()),
            });
        }
        Ok(Self { changes, removed })
    }

    /// F3 看的变更集合（changed ∪ added ∪ removed）。
    fn paths(&self) -> BTreeSet<String> {
        self.changes
            .iter()
            .map(|c| c.path.clone())
            .chain(self.removed.iter().cloned())
            .collect()
    }

    /// F2 ①：作者是我的路径。取不到用户名时为空。
    fn mine(&self, who: &Who) -> BTreeSet<String> {
        self.changes
            .iter()
            .filter(|c| who.is_me(&c.path) == Some(true))
            .map(|c| c.path.clone())
            .collect()
    }
}

/// 快照 HEAD 的祖先（或者就是它），`new` 的历史除外。集合比逐个 `is-ancestor` 省进程。
///
/// 没有快照、快照里的 commit 已经没了：空集，一律按拿不准处理（正常重放）。
fn commits_before_mark(repo: &str, who: &Who, new: &str) -> BTreeSet<String> {
    let Some(mark) = &who.mark else {
        return BTreeSet::new();
    };
    match run_git_code(repo, &["rev-list", &mark.head_oid, "--not", new]) {
        Ok((0, out)) => out.lines().map(str::to_string).collect(),
        Ok((code, _)) => {
            log::warn!("失焦快照里的 HEAD 读不出来（rev-list 退出 {code}），按没有快照处理");
            BTreeSet::new()
        }
        Err(e) => {
            log::warn!("失焦快照里的 HEAD 读不出来，按没有快照处理：{}", e.message);
            BTreeSet::new()
        }
    }
}

fn check_git_version(repo: &str) -> Result<()> {
    /// 进程内只问一次「够新」：PATH 上的 git 不会在应用运行中变旧。不够新的结果不记，用户
    /// 升级 git 之后不用重启应用（性能 P3：每次跟进省一个子进程）。
    static NEW_ENOUGH: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    let text = match fault::pretended_git_version() {
        Some(v) => v,
        None if NEW_ENOUGH.get().is_some() => return Ok(()),
        None => run_git(repo, &["--version"])?,
    };
    if git_version_ok(&text) {
        if fault::pretended_git_version().is_none() {
            let _ = NEW_ENOUGH.set(());
        }
        return Ok(());
    }
    Err(GitError::new(
        "svn_follow_git_too_old",
        format!(
            "跟进需要 git ≥ {}.{}（重放用 merge-tree --merge-base，合并暂存用 merge-file \
             --object-id），当前是「{}」。请升级 git。什么都没有改动。",
            MIN_GIT.0,
            MIN_GIT.1,
            text.trim()
        ),
    ))
}

/// HEAD 附着在非基线分支上。
fn attached_branch(repo: &str) -> Result<String> {
    match current_branch(repo)? {
        None => Err(GitError::new(
            "svn_follow_detached_head",
            "HEAD 没有附着在分支上。跟进要移动当前分支，请先切回一个分支。",
        )),
        Some(b) if b == BASELINE_BRANCH => Err(GitError::new(
            "svn_follow_on_baseline",
            format!("当前在基线分支 `{BASELINE_BRANCH}` 上。基线由本应用维护，请先切回工作分支。"),
        )),
        Some(b) => Ok(b),
    }
}

/// 外部 git 正拿着索引锁：这时动手，基线发布之后拼索引才失败。
fn require_index_free(repo: &str) -> Result<()> {
    let lock = git_dir(repo)?.join("index.lock");
    if lock.exists() {
        return Err(GitError::new(
            "svn_index_busy",
            format!(
                "另一个 git 进程正在占用这个仓库的索引（{}）。等它结束再跟进；如果确认没有 git 在跑，\
                 那是上一次崩溃留下的残锁，手工删掉该文件即可。",
                lock.display()
            ),
        ));
    }
    Ok(())
}

/// 这条分支是从哪个基线分出来的：`merge-base`，且必须是一条基线 commit。
///
/// 通常就是 `svn/base`。分支落后（上次只发布了基线、用户切到了旧分支）时更早。commit 清单
/// 两种写法相同（`A..B` 本来就排除 A 的全部祖先），差别在「外部变了什么」：只看 `svn/base..NEW`
/// 会漏掉上一个基线带进来的路径，F2 ① 与 F3 都看不到它们。
fn branch_baseline(repo: &str, head: &str, base: &str) -> Result<String> {
    let unrelated = || {
        GitError::new(
            "svn_follow_branch_unrelated",
            "当前分支和基线的分叉点不是一条基线 commit，分不清哪些提交是这条分支自己的。\
             请把历史整理成从基线分出来的样子，再跟进。",
        )
    };
    let Some(scope) = crate::engine::git_read::merge_base(repo, head, base)? else {
        return Err(unrelated());
    };
    read_base_map(repo, &scope).map_err(|_| unrelated())?;
    Ok(scope)
}

/// 预检与 F1–F3。**一个 ref 都不动**：基线 commit 与重放结果只是对象。
fn prepare(repo: &str) -> Result<Plan> {
    if detect_repo_kind(repo) != RepoKind::SvnHybrid {
        return Err(not_hybrid());
    }
    check_git_version(repo)?;
    svn_lock::assert_not_syncing(repo)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished());
    }
    check_git_idle(repo, "svn_follow_git_busy", "跟进")?;
    require_index_free(repo)?;
    let branch = attached_branch(repo)?;
    let old_head = rev_parse(repo, "HEAD")?;
    let old_base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    let cfg = read_config(repo)?;

    let (status, nodes) = check_sync_with_nodes(repo)?;
    let scope = branch_baseline(repo, &old_head, &old_base)?;

    // F1：只有 ExternallyAdvanced 要录新基线
    let mut recorded = None;
    let (new_base, nodes, changes, removed, verified, mut who) = match status.verdict {
        SyncVerdict::WcLocked { dirs } => {
            return Err(GitError::new(
                "svn_follow_wc_locked",
                format!(
                    "工作副本被 SVN 锁着（{}），多半是更新还在进行，或者中途被杀掉了。等它结束；\
                     如果已经没有 SVN 操作在跑，用 TortoiseSVN 的 Clean up 清掉，再跟进。",
                    dirs.join("、")
                ),
            ))
        }
        SyncVerdict::Diverged { node_rev, .. } | SyncVerdict::PinnedReset { node_rev, .. } => {
            return Err(GitError::new(
                "svn_follow_diverged",
                format!(
                    "工作副本有文件退回到了基线之前（r{node_rev}）。跟进只处理 SVN 侧前进的情形，\
                     请先处理顶部的失同步横幅。"
                ),
            ))
        }
        SyncVerdict::ExternallyAdvanced {
            changed,
            added,
            removed,
        } => {
            let rec = record_from_old_base(repo, &cfg, &old_base)?;
            let who = Who::gather(repo, &rec.map.repos_root, &rec.nodes)?;
            let mut changes: Vec<ExternalChange> = changed.into_iter().chain(added).collect();
            changes.sort_by(|a, b| (a.rev, &a.path).cmp(&(b.rev, &b.path)));
            let summary = summary_lines(&changes, removed.len(), who.me.as_deref());
            let new_base = create_baseline_commit_with(
                repo,
                &rec.tree,
                Some(&old_base),
                &rec.map,
                &summary,
                None,
            )?;
            let verified = matches!(rec.health, Health::Verified { .. });
            recorded = Some(rec.map);
            (new_base, rec.nodes, changes, removed, verified, who)
        }
        // 判定说没有新内容，分支也已在基线上：真的没事可做
        SyncVerdict::InSync if scope == old_base => {
            return Err(GitError::new(
                "svn_follow_nothing_to_do",
                "基线已经和工作副本一致，分支也在基线上，没有要跟进的。",
            ))
        }
        // 分支跟进入口：以现有的 svn/base 为 NEW。不录制，也就没有 R1 那道检查，这里补上
        SyncVerdict::InSync => {
            run_preflight_with(repo, &cfg.whitelist, true)?;
            let map = read_baseline_map(repo)?;
            let who = Who::gather(repo, &map.repos_root, &nodes)?;
            let ext = External::read(repo, &scope, &old_base, &nodes)?;
            (old_base.clone(), nodes, ext.changes, ext.removed, true, who)
        }
    };

    // 分支落后时要看从它自己的基线起的全部变化，不只是这一次的判定
    let ext = External::read(repo, &scope, &new_base, &nodes)?;
    let paths = ext.paths();
    who.after_mark = paths_after_mark(repo, who.mark.as_ref(), &new_base);
    let before_mark = commits_before_mark(repo, &who, &new_base);
    // F2 ①：早于失焦快照的 commit 里，被我在快照之后用 TortoiseSVN 提交过的路径取 SVN 版本
    let replaceable: BTreeSet<String> = match &who.after_mark {
        Some(after) => ext.mine(&who).intersection(after).cloned().collect(),
        None => BTreeSet::new(),
    };
    let superseded = |c: &ReplayCommit| -> BTreeSet<String> {
        // Local-Only 不是中间版本：换成 SVN 版本等于把本地补丁静默扔掉（Review 2 第 2 条）。
        // 真的被带上服务器时重放会变空或冲突，由 F3 与冲突出口接手
        if !c.local_only && before_mark.contains(&c.oid) {
            replaceable.clone()
        } else {
            BTreeSet::new()
        }
    };

    let map_floor = read_base_map(repo, &new_base)?.floor;
    let mut report = FollowReport {
        new_floor: map_floor,
        verified,
        changes,
        removed,
        ..Default::default()
    };

    // F2 当前分支。memo 跨分支共享：共享的 commit 只重放一次（R20）
    let mut memo = BTreeMap::new();
    let commits = commits_between(repo, &scope, &old_head, &FOLLOW_CODES)?;
    let replayed = replay_branch(
        repo,
        &scope,
        &old_head,
        &new_base,
        &commits,
        &superseded,
        &mut memo,
    )?;
    let stale = match &replayed {
        Replayed::Moved { tip, .. } => stale_disk_paths(repo, &scope, &old_base, &old_head, tip)?,
        Replayed::Conflict { .. } => Vec::new(),
    };
    let (target, others) = match replayed {
        Replayed::Conflict { at, files } => {
            // F3 也要算：分支没重放，但补丁可能已经上了服务器，用户要在手动跟进之前知道
            let leaks = local_only_leaks(repo, &branch, &commits, &paths, &new_base)?;
            (
                Target::Conflict {
                    at,
                    files,
                    leaks,
                    stale_disk: false,
                },
                Vec::new(),
            )
        }
        // 重放得上，但只移 HEAD 会在「本地改动」里造出反向修改：交给会写磁盘的手动跟进
        Replayed::Moved { .. } if !stale.is_empty() => {
            let leaks = local_only_leaks(repo, &branch, &commits, &paths, &new_base)?;
            (
                Target::Conflict {
                    at: old_head.clone(),
                    files: stale,
                    leaks,
                    stale_disk: true,
                },
                Vec::new(),
            )
        }
        Replayed::Moved {
            tip,
            dropped,
            rewritten,
        } => {
            report.dropped = dropped;
            report.rewritten = rewritten;
            // F3：按原始 commit 算，不受 F2 ① 换路径影响。手动跟进不拦，只告警（§6.12 条件 8 管自动）
            report.leaks = local_only_leaks(repo, &branch, &commits, &paths, &new_base)?;
            let others = plan_others(
                repo,
                &OthersCtx {
                    current: &branch,
                    prev_base: previous_baseline(repo, &old_base, &new_base)?.as_deref(),
                    new_base: &new_base,
                    paths: &paths,
                    superseded: &superseded,
                },
                &mut memo,
                &mut report,
            )?;
            (Target::Tip(tip), others)
        }
    };

    Ok(Plan {
        branch,
        old_head,
        old_base,
        new_base,
        target,
        others,
        report,
        recorded,
        who,
    })
}

/// 录新基线（分析 §6.3）。候选索引从旧基线树起步、沿用真实索引的 stat，免得每次跟进都把白名单
/// 全量哈希一遍（[`TempIndex::from_tree_keeping_stat`]）。
///
/// 原来从空索引起步，为的是甩掉服务器上已删除的文件；从旧基线起步时它们磁盘上已经不在，
/// `add -A` 照样摘掉。不等价的只有一种：旧基线里有现在已不算 W 的文件（白名单或扩展名收窄过），
/// 它是已跟踪文件，`add -A` 会留下它，R2c 的纳入集合核对随即报 `include_set_mismatch`。这时退回
/// 空索引按原来的口径再录一次，结果与改动前完全相同。
fn record_from_old_base(repo: &str, cfg: &SvnConfig, old_base: &str) -> Result<Recorded> {
    let idx = TempIndex::from_tree_keeping_stat(repo, old_base)?;
    match record_mixed(repo, cfg, &idx) {
        Err(e) if e.code == "include_set_mismatch" => {
            log::info!(
                "从旧基线起步的候选索引多收了文件，退回空索引重录：{}",
                e.message
            );
            record_mixed(repo, cfg, &TempIndex::empty(repo)?)
        }
        r => r,
    }
}

/// 分支落后时，磁盘上仍是旧 HEAD 内容、而 HEAD 一移到 `tip` 就会显示成反向修改的路径
/// （Review 3 第 2 条）。
///
/// 场景：「仅切换」或外部工具切到含旧基线的分支，git 把旧内容写回磁盘，SVN 看着已修改，git 看着
/// 干净；判定仍是 InSync、`headBehindBase`。对象层跟进只移 HEAD 不写磁盘，「本地改动」就成了把
/// SVN 的内容撤回去，`commit -a` 会造出回退 commit。手动跟进的 rebase 会把新内容写回磁盘。
///
/// 只看 `scope..old_base` 改过的路径（分支落后的那一段，`scope == old_base` 时为空）里，HEAD 与
/// `tip` 不同的那些：不扫整个工作区。磁盘内容用 `hash-object --stdin-paths` 算，与 `git add`
/// 一样按 attributes 做 autocrlf / eol 转换（2026-11-01 实测 git 2.51：`--stdin-paths` 逐个按路径
/// 过滤，与 `--path` 结果相同）。HEAD 里没有的路径，磁盘上也没有才算旧内容。符号链接、子模块不判，
/// 按不是旧内容处理（与改动前相同）。
fn stale_disk_paths(
    repo: &str,
    scope: &str,
    old_base: &str,
    old_head: &str,
    tip: &str,
) -> Result<Vec<String>> {
    if scope == old_base {
        return Ok(Vec::new());
    }
    let lagging: BTreeSet<String> = run_git(
        repo,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--name-only",
            scope,
            old_base,
        ],
    )?
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_string)
    .collect();
    // raw 格式：`:<旧 mode> <新 mode> <旧 oid> <新 oid> <状态>` NUL `<路径>` NUL
    let raw = run_git(
        repo,
        &["diff-tree", "-r", "-z", "--no-renames", old_head, tip],
    )?;
    let mut stale = Vec::new();
    let mut to_hash: Vec<(String, String)> = Vec::new();
    let mut fields = raw.split('\0').filter(|f| !f.is_empty());
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        if !lagging.contains(path) {
            continue;
        }
        let meta: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (Some(mode), Some(head_oid)) = (meta.first(), meta.get(2)) else {
            continue;
        };
        let on_disk = Path::new(repo).join(path).symlink_metadata();
        if head_oid.bytes().all(|b| b == b'0') {
            if on_disk.is_err() {
                stale.push(path.to_string());
            }
        } else if (*mode == "100644" || *mode == "100755")
            && on_disk.map(|m| m.is_file()).unwrap_or(false)
        {
            to_hash.push((path.to_string(), head_oid.to_string()));
        }
    }
    if !to_hash.is_empty() {
        let input: String = to_hash.iter().map(|(p, _)| format!("{p}\n")).collect();
        let out = run_git_stdin(repo, &["hash-object", "--stdin-paths"], input.as_bytes())?;
        let disk: Vec<&str> = out.lines().map(str::trim).collect();
        if disk.len() != to_hash.len() {
            return Err(GitError::new(
                "svn_follow_hash_failed",
                "核对磁盘内容时 git hash-object 的输出行数对不上。这次什么都没动，请再点一次「跟进」。",
            ));
        }
        for ((p, head_oid), d) in to_hash.iter().zip(disk) {
            if d == head_oid {
                stale.push(p.clone());
            }
        }
    }
    stale.sort();
    Ok(stale)
}

enum Replayed {
    Moved {
        tip: String,
        dropped: Vec<String>,
        rewritten: Vec<(String, Vec<String>)>,
    },
    Conflict {
        at: String,
        files: Vec<String>,
    },
}

fn require_linear(repo: &str, scope: &str, head: &str) -> Result<()> {
    assert_linear(repo, scope, head).map_err(|e| {
        if e.code == "sync_in_merge_in_range" {
            GitError::new(
                "svn_follow_merge_in_range",
                "这条分支基线之后有 merge commit，而且内容还没有全部提交到 SVN。跟进要把提交重放到\
                 新基线上，merge 重放不了。请把历史整理成线性的，或者提交完之后再跟进。",
            )
        } else {
            e
        }
    })
}

/// F2：先试「已全部提交」捷径（不要求线性），否则线性检查后逐个重放。
fn replay_branch(
    repo: &str,
    scope: &str,
    head: &str,
    new: &str,
    commits: &[ReplayCommit],
    superseded: &dyn Fn(&ReplayCommit) -> BTreeSet<String>,
    memo: &mut BTreeMap<String, String>,
) -> Result<Replayed> {
    if all_committed(repo, scope, head, new)? {
        return Ok(Replayed::Moved {
            tip: new.to_string(),
            dropped: commits.iter().map(|c| c.oid.clone()).collect(),
            rewritten: Vec::new(),
        });
    }
    require_linear(repo, scope, head)?;
    let opts = ReplayOpts {
        drop_empty: true,
        superseded,
        codes: FOLLOW_CODES,
    };
    Ok(match replay_onto(repo, new, commits, &opts, memo)? {
        ReplayOutcome::Done {
            tip,
            dropped,
            rewritten,
            ..
        } => Replayed::Moved {
            tip,
            dropped,
            rewritten,
        },
        ReplayOutcome::Conflict { at, files } => Replayed::Conflict { at, files },
    })
}

struct LocalBranch {
    name: String,
    oid: String,
    upstream: bool,
}

fn local_branches(repo: &str) -> Result<Vec<LocalBranch>> {
    let out = run_git(
        repo,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(upstream)",
            "refs/heads/",
        ],
    )?;
    let mut list = Vec::new();
    for line in out.lines() {
        // ref 名不能含空格；没有 upstream 时第三列是空串
        let mut f = line.split(' ');
        let (Some(name), Some(oid)) = (f.next(), f.next()) else {
            continue;
        };
        let Some(name) = name.strip_prefix("refs/heads/") else {
            continue;
        };
        list.push(LocalBranch {
            name: name.to_string(),
            oid: oid.to_string(),
            upstream: f.next().is_some_and(|u| !u.is_empty()),
        });
    }
    Ok(list)
}

/// 落后基线的本地分支（分析 §6.11.3）：含更早的基线、不含当前 `svn/base`。分支列表据此打标记，
/// 切过去时确认。没有基线时是空列表。
///
/// 判据是「与 `svn/base` 有共同祖先，且分叉点不是它自己」：基线是一条链，共同祖先之下一定有更早
/// 的基线。**比 `head_behind_base` 宽：** 分叉点可以是交付 commit（它不是基线 commit，但底下有），
/// 这种分支确实落后，手动跟进会以 `svn_follow_branch_unrelated` 拒绝，用户至少在切过去之前
/// 知道它不在新内容上。历史无关的分支不含任何基线，不算。
///
/// 走 libgit2：每次 `repo-changed` 都要跑，分支一多，逐条起 `git merge-base` 进程扛不住。
pub fn branches_behind(repo: &str) -> Result<Vec<String>> {
    let git = Repository::open(repo)?;
    let spec = format!("refs/heads/{BASELINE_BRANCH}^{{commit}}");
    let Ok(base) = git.revparse_single(&spec) else {
        return Ok(Vec::new());
    };
    let base = base.id();
    let mut behind = Vec::new();
    for entry in git.branches(Some(BranchType::Local))? {
        let (branch, _) = entry?;
        // 名字不是 UTF-8 的分支传不到前端，也就标不上
        let Some(name) = branch.name()?.map(str::to_string) else {
            continue;
        };
        let tip = branch.get().peel_to_commit()?.id();
        if matches!(git.merge_base(tip, base), Ok(fork) if fork != base) {
            behind.push(name);
        }
    }
    Ok(behind)
}

/// 所有 worktree 的 HEAD 指着的分支（含主工作区）。
fn worktree_heads(repo: &str) -> Result<BTreeSet<String>> {
    Ok(run_git(repo, &["worktree", "list", "--porcelain"])?
        .lines()
        .filter_map(|l| l.strip_prefix("branch refs/heads/"))
        .map(str::to_string)
        .collect())
}

/// §6.11.3 的「旧基线」：这次发布之前的 `svn/base`。
///
/// 不能拿当前分支的 merge-base 代替：当前分支落后时它比旧基线更早，范围就把只含更早基线的分支
/// 也圈进来了。分支跟进入口上基线早就发布了，被中断的那次跟进的旧基线是它的父提交（交付录的
/// 基线父提交是交付 commit，这时只会少移动，多留下的分支照样标「落后基线」）。
fn previous_baseline(repo: &str, old_base: &str, new_base: &str) -> Result<Option<String>> {
    if old_base != new_base {
        return Ok(Some(old_base.to_string()));
    }
    crate::engine::git_read::resolve(repo, &format!("{new_base}^1^{{commit}}"))
}

struct OthersCtx<'a> {
    current: &'a str,
    /// `None`：基线没有父提交，也就没有「旧基线」，没有要动的分支。
    prev_base: Option<&'a str>,
    new_base: &'a str,
    paths: &'a BTreeSet<String>,
    superseded: &'a dyn Fn(&ReplayCommit) -> BTreeSet<String>,
}

/// 其他分支（分析 §6.11.3）。范围是包含旧基线的本地分支；跳过当前分支、任何 worktree 里是
/// HEAD 的、配置了 upstream 的。动不了的进 `behind_branches`，**不阻断当前分支**：一条有冲突
/// 的旧实验分支不该让跟进永远失败。
fn plan_others(
    repo: &str,
    ctx: &OthersCtx,
    memo: &mut BTreeMap<String, String>,
    report: &mut FollowReport,
) -> Result<Vec<BranchMove>> {
    let Some(prev_base) = ctx.prev_base else {
        return Ok(Vec::new());
    };
    let busy = worktree_heads(repo)?;
    let mut moves = Vec::new();
    for b in local_branches(repo)? {
        if b.name == ctx.current
            || b.name == BASELINE_BRANCH
            || b.upstream
            || busy.contains(&b.name)
            // 基于更早基线的本来就落后，切换时再处理（§6.11.3）
            || !is_ancestor(repo, prev_base, &b.oid)?
            // 分支跟进入口上它可能已经在 NEW 上了
            || is_ancestor(repo, ctx.new_base, &b.oid)?
        {
            continue;
        }
        match plan_other(repo, ctx, &b, memo, report) {
            Ok(Ok(new)) => moves.push(BranchMove {
                branch: b.name,
                old: b.oid,
                new,
            }),
            Ok(Err(reason)) => report.behind_branches.push((b.name, reason)),
            // 这条分支的事，不是这次跟进的事
            Err(e) => report.behind_branches.push((b.name, e.message)),
        }
    }
    Ok(moves)
}

/// 一条其他分支：`Ok(新 tip)` 或者 `Err(不移动的原因)`。
fn plan_other(
    repo: &str,
    ctx: &OthersCtx,
    b: &LocalBranch,
    memo: &mut BTreeMap<String, String>,
    report: &mut FollowReport,
) -> Result<std::result::Result<String, String>> {
    let own = branch_baseline(repo, &b.oid, ctx.new_base)?;
    let commits = commits_between(repo, &own, &b.oid, &FOLLOW_CODES)?;
    let leaks = local_only_leaks(repo, &b.name, &commits, ctx.paths, ctx.new_base)?;
    if !leaks.is_empty() {
        report.leaks.extend(leaks);
        return Ok(Err(
            "分支上的 Local-Only 补丁可能被一起提交到了 SVN，不自动移动".to_string(),
        ));
    }
    let replayed = replay_branch(
        repo,
        &own,
        &b.oid,
        ctx.new_base,
        &commits,
        ctx.superseded,
        memo,
    );
    match replayed {
        Ok(Replayed::Moved {
            tip,
            dropped,
            rewritten,
        }) => {
            for d in dropped {
                if !report.dropped.contains(&d) {
                    report.dropped.push(d);
                }
            }
            for r in rewritten {
                if !report.rewritten.contains(&r) {
                    report.rewritten.push(r);
                }
            }
            Ok(Ok(tip))
        }
        Ok(Replayed::Conflict { at, files }) => Ok(Err(format!(
            "重放 {} 时冲突：{}",
            &at[..at.len().min(10)],
            files.join("、")
        ))),
        Err(e) if e.code == "svn_follow_merge_in_range" => Ok(Err(
            "分支上有 merge commit，而且内容还没有全部提交到 SVN".to_string(),
        )),
        Err(e) => Err(e),
    }
}

/// 基线 commit 的说明（分析 §6.10）：哪些 revision、谁的。在 git 历史里也分得清别人的改动。
///
/// ```text
/// 跟进 r1245–r1250：A、B；自己的提交 r1248
/// r1245 A：3 个文件
/// r1248 me：1 个文件
/// ```
fn summary_lines(changes: &[ExternalChange], removed: usize, me: Option<&str>) -> Vec<String> {
    let mut by_rev: BTreeMap<u64, (BTreeSet<&str>, usize)> = BTreeMap::new();
    for c in changes {
        let who = match c.author.as_deref() {
            Some(a) if !a.is_empty() => a,
            _ => "（匿名）",
        };
        let e = by_rev.entry(c.rev).or_default();
        e.0.insert(who);
        e.1 += 1;
    }
    let mut lines = Vec::new();
    if let (Some(first), Some(last)) = (by_rev.keys().next(), by_rev.keys().next_back()) {
        let span = if first == last {
            format!("r{first}")
        } else {
            format!("r{first}–r{last}")
        };
        let others: BTreeSet<&str> = by_rev
            .values()
            .flat_map(|(who, _)| who.iter().copied())
            .filter(|w| Some(*w) != me)
            .collect();
        let mine: Vec<String> = by_rev
            .iter()
            .filter(|(_, (who, _))| me.is_some_and(|m| who.contains(m)))
            .map(|(r, _)| format!("r{r}"))
            .collect();
        let mut parts = Vec::new();
        if !others.is_empty() {
            parts.push(capped(others.iter().map(|s| s.to_string()).collect(), "人"));
        }
        if !mine.is_empty() {
            parts.push(format!("自己的提交 {}", capped(mine, "个")));
        }
        lines.push(format!("跟进 {span}：{}", parts.join("；")));
        for (rev, (who, n)) in by_rev.iter().take(SUMMARY_REVISIONS) {
            let who: Vec<&str> = who.iter().copied().collect();
            lines.push(format!("r{rev} {}：{n} 个文件", who.join("、")));
        }
        if by_rev.len() > SUMMARY_REVISIONS {
            lines.push(format!(
                "（另有 {} 个 revision 未列出）",
                by_rev.len() - SUMMARY_REVISIONS
            ));
        }
    }
    if removed > 0 {
        lines.push(format!("{removed} 个文件已从 SVN 删除"));
    }
    lines
}

fn capped(items: Vec<String>, unit: &str) -> String {
    let total = items.len();
    let shown = items
        .into_iter()
        .take(SUMMARY_NAMES)
        .collect::<Vec<_>>()
        .join("、");
    if total > SUMMARY_NAMES {
        format!("{shown} 等 {total} {unit}")
    } else {
        shown
    }
}

// —— 发布阶段 ——

/// F4 锁内复核。变了返回 `false`，调用方释放锁回到 F1（最多一次）。
fn still_valid(repo: &str, plan: &Plan) -> Result<bool> {
    if svn_journal::read(repo)?.is_some() {
        return Err(GitError::new(
            "svn_follow_unfinished",
            "准备跟进期间有另一次同步留下了记录。请先在顶部横幅上处理，再跟进。",
        ));
    }
    check_git_idle(repo, "svn_follow_git_busy", "跟进")?;
    require_index_free(repo)?;
    if current_branch(repo)?.as_deref() != Some(plan.branch.as_str())
        || rev_parse(repo, "HEAD")? != plan.old_head
        || rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))? != plan.old_base
    {
        return Ok(false);
    }
    for m in &plan.others {
        if ref_oid(repo, &m.branch)?.as_deref() != Some(m.old.as_str()) {
            return Ok(false);
        }
    }
    let Some(map) = &plan.recorded else {
        return Ok(true);
    };
    // R3c 的第三道：录完到现在又隔了一段重放。按同一规则重算，算不出来也是被改了
    let cfg = read_config(repo)?;
    let wcs = enumerate_wcs(repo, &cfg.whitelist)?;
    let nodes = read_node_status(repo, &cfg.whitelist, &wcs)?;
    let files = tree_files(repo, &plan.new_base)?;
    Ok(
        match compute_map(&map.repos_root, &map.uuid, &files, &nodes, map.gone.clone()) {
            Ok(again) => again == *map,
            Err(_) => false,
        },
    )
}

/// F4（写 journal）与 F5、F6。调用方持同步锁。走到这里之后失败都是 `Err`，除非基线还没动。
fn publish(repo: &str, opid: &str, plan: Plan) -> Result<FollowOutcome> {
    let Plan {
        branch,
        old_head,
        old_base,
        new_base,
        target,
        others,
        report,
        who,
        ..
    } = plan;
    let tip = match &target {
        Target::Tip(t) => Some(t.clone()),
        Target::Conflict { .. } => None,
    };
    let mut j = Journal {
        opid: opid.to_string(),
        flow: Flow::Follow,
        phase: Phase::FollowPublishing,
        started_at: now_secs(),
        orig_branch: Some(branch.clone()),
        orig_head: Some(old_head.clone()),
        baseline_oid_before: Some(old_base.clone()),
        new_baseline_oid: Some(new_base.clone()),
        follow_steps: Some(FollowSteps {
            step: if old_base == new_base {
                FollowStep::PrepareIndex
            } else {
                FollowStep::Baseline
            },
            new_tip: tip.clone(),
            others,
            index_blob: None,
            index_stages: None,
            lock_created: false,
            report,
        }),
        ..Default::default()
    };
    if let Err(e) = svn_journal::write(repo, &j) {
        return Ok(refused(e));
    }
    fault::reach(fault::Point::Journaled, None);

    // ① 基线。失败时什么都没动，它是最先做的一步
    if old_base != new_base {
        if let Err(e) = move_baseline_ref(repo, &new_base, Some(&old_base)) {
            svn_journal::remove(repo)?;
            return Ok(refused(e));
        }
    }
    fault::reach(fault::Point::BaseMoved, None);

    let Some(tip) = tip else {
        // 当前分支冲突：只发布基线，等用户手动跟进。快照留着，这条分支还没跟上
        svn_journal::remove(repo)?;
        return Ok(match target {
            Target::Conflict {
                at,
                files,
                leaks,
                stale_disk,
            } => needs_manual(&at, &files, &leaks, stale_disk),
            Target::Tip(_) => unreachable!("tip 取自 target"),
        });
    };

    // ② 拼索引
    advance(repo, &mut j, FollowStep::PrepareIndex)?;
    fault::reach(fault::Point::BeforePrepare, None);
    // 暂存 blob 要在持索引锁之后读（facts 只在 prepare_index 持锁时调用），否则外部一次 add
    // 能让这里读到的与拼索引看到的不是同一份。交集为空时一次都不用读
    let staged = OnceCell::new();
    let facts = |p: &str| who.facts(p, staged.get_or_init(|| staged_blobs(repo)).as_ref());
    // 建好锁立刻记下来：恢复时只有这个记录能证明路径上的锁是我们的（Review 2 第 7 条）
    let mut on_locked = || -> Result<()> {
        steps_mut(&mut j)?.lock_created = true;
        svn_journal::write(repo, &j)?;
        fault::reach(fault::Point::IndexLocked, None);
        Ok(())
    };
    let mut prepared = match prepare_index_with(repo, opid, &old_head, &tip, &facts, &mut on_locked)
    {
        Ok(p) => p,
        Err(e) => {
            svn_journal::remove(repo)?;
            return Err(baseline_kept(e));
        }
    };
    if let Err(e) = record_prepared(repo, &mut j, &prepared) {
        prepared.abandon();
        svn_journal::remove(repo)?;
        return Err(baseline_kept(e));
    }
    fault::reach(fault::Point::IndexPrepared, Some(&mut prepared));

    // ③ 核对 HEAD → CAS 当前分支 → 再核对 HEAD
    if !head_on(repo, &branch) {
        return Err(back_out(repo, &j, prepared, false, head_moved(&branch)));
    }
    fault::reach(fault::Point::HeadChecked, Some(&mut prepared));
    if let Err(e) = move_branch(repo, &branch, &tip, &old_head) {
        return Err(back_out(
            repo,
            &j,
            prepared,
            false,
            branch_moved(&branch, &e),
        ));
    }
    fault::reach(fault::Point::BranchMoved, Some(&mut prepared));
    // checkout -b 绕得过 index.lock（W23）。落在这里时新分支建在 NEW_TIP 上，拼好的索引对得上它，
    // 往前走；只有 HEAD 不在 NEW_TIP 上（CAS 之前切走的、symbolic-ref 指到别处的），换入才是错的
    if !head_at(repo, &tip) {
        return Err(back_out(repo, &j, prepared, true, head_moved(&branch)));
    }

    // ④ 换入索引
    if let Err(e) = advance(repo, &mut j, FollowStep::SwapIndex) {
        return Err(back_out(repo, &j, prepared, true, e));
    }
    fault::reach(fault::Point::BeforeSwap, Some(&mut prepared));
    if let Err(e) = prepared.commit() {
        return Err(back_out(repo, &j, prepared, true, e));
    }
    fault::reach(fault::Point::IndexSwapped, None);

    // ⑤ 其他分支 + F6
    advance(repo, &mut j, FollowStep::MoveOthers)?;
    finish(repo, &mut j)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn steps(j: &Journal) -> Result<&FollowSteps> {
    j.follow_steps.as_ref().ok_or_else(corrupt_steps)
}

fn steps_mut(j: &mut Journal) -> Result<&mut FollowSteps> {
    j.follow_steps.as_mut().ok_or_else(corrupt_steps)
}

fn corrupt_steps() -> GitError {
    GitError::new(
        "svn_journal_corrupt",
        "跟进记录缺 `followSteps`，不知道走到了哪一步。请联系开发者，不要删记录文件。",
    )
}

fn advance(repo: &str, j: &mut Journal, step: FollowStep) -> Result<()> {
    steps_mut(j)?.step = step;
    svn_journal::write(repo, j)
}

/// 记下拼好的索引的指纹与暂存报告。之后崩溃时靠它认锁文件，报告也不会丢。
fn record_prepared(repo: &str, j: &mut Journal, prepared: &PreparedIndex) -> Result<()> {
    let lock = git_dir(repo)?.join("index.lock");
    let blob = blob_of(repo, &std::fs::read(lock)?)?;
    let work = prepared.lock_abs_path()?;
    let stages = run_git_env(repo, LS_STAGE, &[("GIT_INDEX_FILE", work.as_str())])?;
    let stages = blob_of(repo, stages.as_bytes())?;
    let s = steps_mut(j)?;
    s.index_blob = Some(blob);
    s.index_stages = Some(stages);
    s.report.stage = prepared.report.clone();
    s.report.pinned_ref = Some(prepared.pinned_ref().to_string());
    s.step = FollowStep::MoveBranch;
    svn_journal::write(repo, j)
}

/// 这段字节的 blob OID（不写对象库）。`--no-filters`：比的是字节，不能过 autocrlf。
fn blob_of(repo: &str, bytes: &[u8]) -> Result<String> {
    Ok(
        run_git_stdin(repo, &["hash-object", "--no-filters", "--stdin"], bytes)?
            .trim()
            .to_string(),
    )
}

fn head_on(repo: &str, branch: &str) -> bool {
    matches!(current_branch(repo), Ok(Some(b)) if b == branch)
}

fn head_at(repo: &str, tip: &str) -> bool {
    rev_parse(repo, "HEAD").is_ok_and(|h| h == tip)
}

/// `checkout -b` 在 CAS 之后把 HEAD 带走了：HEAD 在 NEW_TIP 上，却不在原来的分支上。新分支不归
/// 我们管，移不回去，所以这时不能回滚：放开索引锁就是 HEAD=新分支@NEW_TIP、索引是 OLD 内容。
fn head_left(repo: &str, branch: &str, tip: &str) -> bool {
    !head_on(repo, branch) && head_at(repo, tip)
}

/// CAS 移动一条分支。
fn move_branch(repo: &str, branch: &str, to: &str, from: &str) -> Result<()> {
    run_git(
        repo,
        &[
            "update-ref",
            "-m",
            "mygitclient: follow",
            &format!("refs/heads/{branch}"),
            to,
            from,
        ],
    )?;
    Ok(())
}

/// 分支现在的 OID；不存在时 `None`。
fn ref_oid(repo: &str, branch: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(
        repo,
        &[
            "rev-parse",
            "-q",
            "--verify",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
    )?;
    Ok((code == 0).then(|| out.trim().to_string()))
}

fn head_moved(branch: &str) -> GitError {
    GitError::new(
        "svn_follow_head_moved",
        format!("跟进期间 HEAD 被别的工具移走了，已经不在分支 `{branch}` 的新位置上。"),
    )
}

fn branch_moved(branch: &str, e: &GitError) -> GitError {
    GitError::new(
        "svn_follow_branch_moved",
        format!(
            "跟进期间分支 `{branch}` 被别的工具移动了，这里不覆盖。git 原话：{}",
            e.message.trim()
        ),
    )
}

fn baseline_kept(e: GitError) -> GitError {
    GitError::new(
        &e.code,
        format!(
            "{}\n\n基线保持在新的位置，分支和暂存区都没动。再点一次「跟进」会重新把分支跟上。",
            e.message
        ),
    )
}

/// 发布中途停下：**先把分支移回去，再放开索引锁**。顺序反过来就是外部 git 能提交的回退窗口。
/// `svn/base` 保持前进（它是真实的），由「分支跟进」入口接手。
fn back_out(
    repo: &str,
    j: &Journal,
    mut prepared: PreparedIndex,
    moved: bool,
    cause: GitError,
) -> GitError {
    if moved {
        if let (Some(branch), Some(tip)) = (
            j.orig_branch.as_deref(),
            j.follow_steps.as_ref().and_then(|s| s.new_tip.as_deref()),
        ) {
            if head_left(repo, branch, tip) {
                prepared.keep_lock_on_drop();
                return keep_for_continue(&cause);
            }
        }
        if let Some(stuck) = move_back_failed(repo, j) {
            // 分支还在新位置上又移不回去：宁可留着锁挡住外部 git，等「继续」补做
            prepared.keep_lock_on_drop();
            return GitError::new(
                &cause.code,
                format!(
                    "{}\n\n另外：把分支移回去也失败了（{stuck}）。索引锁和记录都留着，\
                     在顶部横幅上点「继续」补做。",
                    cause.message
                ),
            );
        }
    }
    prepared.abandon();
    if let Err(e) = svn_journal::remove(repo) {
        log::warn!("跟进记录删不掉：{}", e.message);
    }
    baseline_kept(cause)
}

/// HEAD 已经被带到 NEW_TIP 上的新分支、索引又没换成：只能留着锁和记录，等「继续」补做。
fn keep_for_continue(cause: &GitError) -> GitError {
    GitError::new(
        &cause.code,
        format!(
            "{}\n\n另外：HEAD 已被别的工具带到了新分支上，移不回去。索引锁和记录都留着，\
             等占用结束后在顶部横幅上点「继续」补做。",
            cause.message
        ),
    )
}

/// 把当前分支从 NEW_TIP CAS 回 OLD。移回去了、或者它已经不在 NEW_TIP 上了（外部移走的，不归
/// 我们管）返回 `None`；还在 NEW_TIP 上移不回去返回原因。
fn move_back_failed(repo: &str, j: &Journal) -> Option<String> {
    let fields = (|| {
        Ok::<_, GitError>((
            need(&j.orig_branch, "origBranch")?,
            need(&j.orig_head, "origHead")?,
            need(&steps(j)?.new_tip, "newTip")?,
        ))
    })();
    let (branch, old_head, new_tip) = match fields {
        Ok(f) => f,
        Err(e) => return Some(e.message),
    };
    let e = match move_branch(repo, &branch, &old_head, &new_tip) {
        Ok(()) => return None,
        Err(e) => e,
    };
    match ref_oid(repo, &branch) {
        Ok(Some(at)) if at == new_tip => Some(e.message),
        Err(re) => Some(re.message),
        Ok(_) => None,
    }
}

/// CAS 其他分支与 F6。步骤全部幂等，重开时从头跑一遍。
fn finish(repo: &str, j: &mut Journal) -> Result<FollowOutcome> {
    let s = steps(j)?.clone();
    let mut report = s.report;
    // 分析 §6.5 想把这一步也放在 index.lock 里，发布顺序改了之后它在 rename 之后：这时外部可能
    // 刚检出某条分支，再查一遍，把窗口缩到一次 update-ref（日志 Task 10「遗留风险」）
    let busy = worktree_heads(repo)?;
    for m in &s.others {
        let at = ref_oid(repo, &m.branch)?;
        if at.as_deref() == Some(m.new.as_str()) {
            report.moved_branches.push(m.branch.clone());
        } else if at.as_deref() != Some(m.old.as_str()) {
            report
                .behind_branches
                .push((m.branch.clone(), "跟进期间这条分支被移动了".to_string()));
        } else if busy.contains(&m.branch) {
            report
                .behind_branches
                .push((m.branch.clone(), "跟进期间这条分支被检出了".to_string()));
        } else {
            match move_branch(repo, &m.branch, &m.new, &m.old) {
                Ok(()) => report.moved_branches.push(m.branch.clone()),
                Err(e) => report.behind_branches.push((m.branch.clone(), e.message)),
            }
        }
        fault::reach(fault::Point::OtherMoved, None);
    }
    clear_mark(repo);
    trim_pins(repo, &j.opid);
    fault::reach(fault::Point::BeforeFinish, None);
    svn_journal::remove(repo)?;
    Ok(FollowOutcome::Done {
        report: Box::new(report),
    })
}

/// 钉住旧索引的 ref 只留最近 [`KEEP_PINS`] 个【待决 D3】。失败不让跟进失败：ref 多几个无害。
fn trim_pins(repo: &str, opid: &str) {
    let out = run_git(
        repo,
        &[
            "for-each-ref",
            // 后写的键优先。同一秒的按名字：opid 以纳秒开头，新的排前面
            "--sort=-refname",
            "--sort=-committerdate",
            "--format=%(refname)",
            "refs/mygitclient/follow/",
        ],
    );
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            log::warn!("列钉住的索引失败：{}", e.message);
            return;
        }
    };
    let current = format!("refs/mygitclient/follow/{opid}");
    for name in out.lines().skip(KEEP_PINS).filter(|n| *n != current) {
        if let Err(e) = run_git(repo, &["update-ref", "-d", name]) {
            log::warn!("删除钉住的索引 {name} 失败：{}", e.message);
        }
    }
}

// —— 崩溃恢复 ——

enum Resumed {
    Finished(FollowOutcome),
    /// 已回到一致状态、journal 已删，重新跟进一次。
    Again,
}

fn resume_locked(repo: &str, j: &mut Journal) -> Result<Resumed> {
    if j.phase != Phase::FollowPublishing {
        return Err(GitError::new(
            "svn_follow_unknown_phase",
            format!(
                "跟进记录停在 {:?}，这个版本不认识。请不要手工去动分支指针，把记录交给开发者。",
                j.phase
            ),
        ));
    }
    match steps(j)?.step {
        // 基线要么动了要么没动，两种都是一致状态。重新跟进时分别走 F1 与分支跟进入口
        FollowStep::Baseline => {
            svn_journal::remove(repo)?;
            Ok(Resumed::Again)
        }
        // 走到这里 CAS 分支还没开始，残留的 index.lock 就算是我们的也只是半成品，rename 进去就毁了
        // 索引，只能删。没记过「锁是我方建的」的不是我们的：活着的 git 等编辑器时 index.lock 在、
        // 句柄已关，删除照样成功（Review 2 探针 P7）。报忙，记录留着，对方结束后再点「继续」
        FollowStep::PrepareIndex => {
            if steps(j)?.lock_created {
                remove_lock(repo)?;
            } else {
                require_index_free(repo)?;
            }
            svn_journal::remove(repo)?;
            Ok(Resumed::Again)
        }
        FollowStep::MoveBranch | FollowStep::SwapIndex => settle_branch_and_index(repo, j),
        FollowStep::MoveOthers => Ok(Resumed::Finished(finish(repo, j)?)),
    }
}

/// 删掉**确认是我方建的** index.lock。别的进程开着它时删不掉（Windows），报错、记录留着。
fn remove_lock(repo: &str) -> Result<()> {
    let lock = git_dir(repo)?.join("index.lock");
    match std::fs::remove_file(&lock) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(GitError::new(
            "svn_index_busy",
            format!(
                "另一个 git 进程正在占用这个仓库的索引（{}：{e}）。记录保留，等它结束后再点「继续」。",
                lock.display()
            ),
        )),
    }
}

/// `.git/index.lock` 是不是我们拼好的那一份。读不了（被占着）也算不是。
fn lock_is_ours(repo: &str, lock: &Path, blob: Option<&str>) -> Result<bool> {
    let (Some(want), Ok(bytes)) = (blob, std::fs::read(lock)) else {
        return Ok(false);
    };
    Ok(blob_of(repo, &bytes)? == want)
}

/// 真实索引的内容就是拼好的那一份：rename 发生过。
///
/// 比 `ls-files --stage` 的指纹，不比字节：一次 `git status` 刷新 stat 就会改写索引，而内容没变
/// （Review 2 第 10 条）。读不出来、对不上（比如 rename 之后外部又 add 过）时按没发生：往回走
/// 总能恢复（见 [`settle_branch_and_index`] 的文档），往前走不一定；报错则会把 journal 卡死。
fn index_was_swapped(repo: &str, want: Option<&str>) -> bool {
    let (Some(want), Ok(now)) = (want, run_git(repo, LS_STAGE)) else {
        return false;
    };
    blob_of(repo, now.as_bytes()).is_ok_and(|have| have == want)
}

/// 把分支移回 OLD。不在 NEW_TIP 上时什么都不做：要么没移过，要么外部移走了，都不归我们管。
fn restore_branch(repo: &str, branch: &str, old_head: &str, new_tip: &str) -> Result<()> {
    if ref_oid(repo, branch)?.as_deref() == Some(new_tip) {
        move_branch(repo, branch, old_head, new_tip)?;
    }
    Ok(())
}

/// 拼好的索引被手工删了，HEAD 又被带到了 NEW_TIP 上（[`head_left`]）：往回走不了，照 OLD →
/// NEW_TIP 重拼一份。手里没有快照与作者了，每条路径都按拿不准处理（保留暂存或三方合并）。
fn recompose_index(repo: &str, j: &mut Journal, old_head: &str, new_tip: &str) -> Result<Resumed> {
    let opid = j.opid.clone();
    let mut prepared = prepare_index(repo, &opid, old_head, new_tip, &|_| PathFacts::default())?;
    record_prepared(repo, j, &prepared)?;
    prepared.keep_lock_on_drop();
    settle_branch_and_index(repo, j)
}

/// 崩在 index.lock 拼好之后、rename 完成之前（或者刚 rename 完）。
///
/// 往回走（移回分支、重新跟进）总是安全的：就算 rename 其实发生过，索引 == NEW_TIP、分支 ==
/// OLD，重新跟进时这些路径都走「索引正好等于新 HEAD」那一行，结果相同。**例外是 [`head_left`]**：
/// 这时 HEAD 不在我们能移回去的分支上，只能往前。
fn settle_branch_and_index(repo: &str, j: &mut Journal) -> Result<Resumed> {
    let branch = need(&j.orig_branch, "origBranch")?;
    let old_head = need(&j.orig_head, "origHead")?;
    let s = steps(j)?;
    let new_tip = need(&s.new_tip, "newTip")?;
    let step = s.step;
    let lock = git_dir(repo)?.join("index.lock");
    let ours = lock_is_ours(repo, &lock, s.index_blob.as_deref())?;
    let swapped =
        step == FollowStep::SwapIndex && index_was_swapped(repo, s.index_stages.as_deref());
    let left = head_left(repo, &branch, &new_tip);

    if !ours {
        // 我们的锁不在了。走到 SwapIndex 且索引内容已是拼好的那份：rename 发生过（路径上若有锁也
        // 是别人的，不碰）。否则是有人手工删了锁：分支已移、索引是旧的，必须移回去或者重拼
        if swapped {
            advance(repo, j, FollowStep::MoveOthers)?;
            return Ok(Resumed::Finished(finish(repo, j)?));
        }
        if left {
            return recompose_index(repo, j, &old_head, &new_tip);
        }
        restore_branch(repo, &branch, &old_head, &new_tip)?;
        svn_journal::remove(repo)?;
        return Ok(Resumed::Again);
    }

    // 锁是我们的：外部 git 这段时间一直进不来，能往前就往前
    let give_up = |repo: &str, cause: Option<GitError>| -> Result<Resumed> {
        restore_branch(repo, &branch, &old_head, &new_tip)?;
        remove_lock(repo)?;
        svn_journal::remove(repo)?;
        match cause {
            Some(e) => Err(baseline_kept(e)),
            None => Ok(Resumed::Again),
        }
    };
    let at = ref_oid(repo, &branch)?;
    let at_old = at.as_deref() == Some(old_head.as_str());
    let at_new = at.as_deref() == Some(new_tip.as_str());
    if !left && (!head_on(repo, &branch) || !(at_old || at_new)) {
        return give_up(repo, None);
    }
    if !left && at_old {
        steps_mut(j)?.step = FollowStep::MoveBranch;
        svn_journal::write(repo, j)?;
        if move_branch(repo, &branch, &new_tip, &old_head).is_err() {
            return give_up(repo, None);
        }
    }
    fault::reach(fault::Point::BranchMoved, None);
    if !head_at(repo, &new_tip) {
        return give_up(repo, None);
    }
    advance(repo, j, FollowStep::SwapIndex)?;
    fault::reach(fault::Point::BeforeSwap, None);
    if let Err(e) = rename_lock_into_index(&lock) {
        if head_left(repo, &branch, &new_tip) {
            return Err(keep_for_continue(&e));
        }
        return give_up(repo, Some(e));
    }
    fault::reach(fault::Point::IndexSwapped, None);
    advance(repo, j, FollowStep::MoveOthers)?;
    Ok(Resumed::Finished(finish(repo, j)?))
}

/// 测试注入点：在发布的固定位置模拟进程死掉，或者插进一段外部操作。
///
/// # 为什么需要它（Review Focus 4、5）
///
/// 「`checkout -b` 恰好落在两次核对之间」「崩在 CAS 与 rename 之间」都是时序，fixture 造不出来。
/// 做法同 `svn_tombstone::fault`：线程局部，发布全程在调用线程上同步执行，`cargo test` 并行跑的
/// 其他测试碰不到；没有 arm 时每个注入点只是几次线程局部读。生产代码里没有地方 arm 它。
///
/// [`crash_at`] 用 `resume_unwind` 退出：journal 与 `.git/index.lock` 留在磁盘上，同步锁随 unwind
/// 释放，对应操作系统回收崩溃进程的句柄。`held` 让 [`PreparedIndex`] 的 Drop 不删锁：真实崩溃时
/// Drop 不会跑。
///
/// 只给集成测试用，`#[doc(hidden)]`。
#[doc(hidden)]
pub mod fault {
    use crate::engine::svn_follow_index::PreparedIndex;
    use std::cell::{Cell, RefCell};

    /// 发布途中可注入的位置，按执行顺序。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Point {
        /// 准备阶段做完，还没拿同步锁
        Prepared,
        /// journal 写好，还没动 `svn/base`
        Journaled,
        /// `svn/base` 已移动
        BaseMoved,
        /// journal 已记 `prepare-index`，还没拿索引锁
        BeforePrepare,
        /// 索引锁已建，journal 已记下锁是我方的，索引还没开始拼
        IndexLocked,
        /// 索引拼好并记进 journal，还没核对 HEAD
        IndexPrepared,
        /// 第一次核对 HEAD 通过，还没 CAS 当前分支
        HeadChecked,
        /// 当前分支已 CAS，还没第二次核对
        BranchMoved,
        /// journal 已记 `swap-index`，还没 rename
        BeforeSwap,
        /// rename 已完成，journal 还没记 `move-others`
        IndexSwapped,
        /// 每处理完一条其他分支
        OtherMoved,
        /// 快照与钉住的 ref 都收拾好了，还没删 journal
        BeforeFinish,
        /// 手动跟进的 S1 做完，还没 rebase
        ManualStashed,
    }

    /// [`crash_at`] 的 panic payload。
    pub struct Crashed;

    /// [`call_at`] 登记的回调。
    type Hook = (Point, Box<dyn FnMut()>);

    thread_local! {
        static CRASH: Cell<Option<Point>> = const { Cell::new(None) };
        static CALL: RefCell<Option<Hook>> = const { RefCell::new(None) };
        static GIT_VERSION: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// 守卫一 drop，本线程上全部注入都撤销。
    #[must_use = "守卫一 drop，注入就撤销"]
    pub struct Armed(());

    /// 本线程上走到 `p` 时进程「死掉」。
    pub fn crash_at(p: Point) -> Armed {
        CRASH.with(|c| c.set(Some(p)));
        Armed(())
    }

    /// 本线程上每次走到 `p` 时先跑 `f`。
    pub fn call_at(p: Point, f: impl FnMut() + 'static) -> Armed {
        CALL.with(|c| *c.borrow_mut() = Some((p, Box::new(f))));
        Armed(())
    }

    /// 本线程上预检读到的 `git --version` 换成 `v`。开发机的 git 够新，走不到拒绝那一支。
    pub fn pretend_git_version(v: &str) -> Armed {
        GIT_VERSION.with(|g| *g.borrow_mut() = Some(v.to_string()));
        Armed(())
    }

    impl Drop for Armed {
        fn drop(&mut self) {
            CRASH.with(|c| c.set(None));
            CALL.with(|c| *c.borrow_mut() = None);
            GIT_VERSION.with(|g| *g.borrow_mut() = None);
        }
    }

    pub(crate) fn pretended_git_version() -> Option<String> {
        GIT_VERSION.with(|g| g.borrow().clone())
    }

    /// 注入点本身。`held` 是此刻手里的 [`PreparedIndex`]。
    pub(crate) fn reach(p: Point, held: Option<&mut PreparedIndex>) {
        // 取出来再调：回调里可能再走到注入点（git 命令不会，但别让 RefCell 成为前提）
        if let Some((at, mut f)) = CALL.with(|c| c.borrow_mut().take()) {
            if at == p {
                f();
            }
            CALL.with(|c| *c.borrow_mut() = Some((at, f)));
        }
        if CRASH.with(Cell::get) == Some(p) {
            if let Some(h) = held {
                h.keep_lock_on_drop();
            }
            std::panic::resume_unwind(Box::new(Crashed));
        }
    }
}
