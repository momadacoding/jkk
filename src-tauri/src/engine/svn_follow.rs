//! 跟进外部提交（分析 §6.5、§6.11.1、§6.11.3；计划 Task 10；日志「偏离计划」的发布顺序）。
//!
//! 用户在 TortoiseSVN 里只提交、不更新，或者只更新某个子目录之后，把基线推进到各文件的真实
//! revision，把分支重放上去，暂存区拼回来。**journal 存续期间不写磁盘，全程不发 SVN 写命令。**
//!
//! 例外有两个：
//!
//! - 冲突兜底 [`manual_follow`]：它走 stash + rebase，会写磁盘，只由用户显式发起；
//! - 发布完成、journal 删掉之后，还拿着同步锁时放回 Local-Only 补丁（[`with_restore`]；计划
//!   `2026-11-12-auto-restore-local-only.md`）。崩溃恢复只看 journal，这一步落在它外面：崩在
//!   放回中途，看到的是一次已经做完的跟进加上「本地改动」里缺补丁的文件，横幅的待放回清单兜底。
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
    all_committed, commit_without, commits_between, replay_onto, ReplayCodes, ReplayCommit,
    ReplayOpts, ReplayOutcome,
};
use crate::engine::svn_restore_local_only::{
    local_only_commits, record_pending, restorable_paths, restore_locked, RestoreOutcome,
    RestoreSkip,
};
use crate::engine::svn_snapshot::SnapshotRule;
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
use crate::engine::tsvn_hooks::{quick_health, HookHealth};
use crate::engine::tsvn_inbox::{prune_inbox, read_evidence, CommitEvidence, HookEvidence};
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};
use git2::{BranchType, Repository};
use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

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
        /// 手动跟进时会直接采用新基线版本的快照内容（事故分析 6.10，独立 review 第 2 条）：在用户点
        /// 「手动跟进」之前告诉他，有意停住的旧版本会被换掉。
        snapshot: PathsByCommit,
        /// 每个冲突文件的上下文（事故分析 6.5）：SVN 上是谁、在哪个 revision 改的，冲突那几行的预览。
        /// 与 `files` 同序；`stale_disk` 时为空。
        details: Vec<ConflictDetail>,
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
    /// (原 commit, 换成 SVN 版本的路径)：F2 ①，我自己提交到 SVN 的。
    pub rewritten: Vec<(String, Vec<String>)>,
    /// (原 commit, 换成 SVN 版本的路径)：内容与比分支基线更新的某个基线逐字节相同，是快照进来的
    /// SVN 内容（事故分析 6.10，[`SnapshotRule`]）。与 `rewritten` 不重复。
    #[serde(default)]
    pub snapshot: Vec<(String, Vec<String>)>,
    /// 重放后与磁盘对不上的文件：磁盘是 SVN 的版本（= 新基线），分支 tip 却是别的内容，或者把 SVN
    /// 已删的文件带了回来。多半是重放干净却合错了（W14；PR 119 遗留）。只告警，不拦。
    #[serde(default)]
    pub drift: Vec<String>,
    /// `drift` 里分支比磁盘多出来的只有 Local-Only 改动、磁盘就是 SVN 版本的文件：可以用
    /// 「放回 Local-Only 补丁」写回磁盘（使用报告 9.1，[`crate::engine::svn_restore_local_only`]）。
    /// 跟进本身不写；点按钮时每条条件现场重验。
    #[serde(default)]
    pub restorable: Vec<String>,
    /// 跟进完成后自动放回的结果（[`with_restore`]）。`restorable` 为空、或者跟进没走到完成时为
    /// `None`。`restorable` 里没放回的文件都在 `skipped` 里，写着原因，也进了待放回清单。
    #[serde(default)]
    pub restore: Option<RestoreOutcome>,
    /// 分支上保留下来的 commit 改过、SVN 侧也有新内容的文件（分析 §6.12 条件 11），逐个标出重放
    /// 结果与磁盘是否逐字节相同。核对出错时为空（自动跟进因此不做，见 [`auto_blocker`]）。
    #[serde(default)]
    pub overlap: Vec<OverlapFile>,
    pub stage: ComposeReport,
    /// 移动过的其他分支。当前分支不在内。
    pub moved_branches: Vec<String>,
    /// (分支, 原因)：不移动，分支列表标「落后基线」。
    pub behind_branches: Vec<(String, String)>,
    pub leaks: Vec<Leak>,
    /// 钉着换入之前的原始索引与暂存 blob 的 ref。没拼过索引时（只发布了基线）是 `None`。
    #[serde(default)]
    pub pinned_ref: Option<String>,
    /// 判定依据（C1h）：钩子健康时按本机事件，否则按作者 + 失焦快照。
    #[serde(default)]
    pub evidence: Evidence,
    /// 读收件箱的时刻（unix 毫秒）。跟进完成后，早于它的事件都已反映在新基线里，可以清掉。
    /// 不按钩子判定时为 `None`，收件箱不动。
    #[serde(default)]
    pub evidence_cutoff_ms: Option<i64>,
}

/// 条件 11 的一个重叠文件（2026-11-11 使用报告第 2 节，方案 B）。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlapFile {
    pub path: String,
    /// 改过它、重放后仍保留的 commit：`短 hash 提交说明`，按提交顺序从新到旧。
    pub commits: Vec<String>,
    /// 重放后的 tip 与磁盘逐字节相同（磁盘按 `git add` 的过滤规则算；两边都没有这个文件也算）。
    pub matches_disk: bool,
}

/// 跟进判定「暂存是不是中间版本」的依据（C1h；分析 §6.13.3）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Evidence {
    /// TortoiseSVN 钩子写的本机事件。
    Hooks,
    /// 作者 + 失焦快照（C1a）。钩子没注册或失效时用它。
    #[default]
    Heuristic,
}

/// 外部 git 的版本够不够。解析不了按不够：宁可让用户看一眼版本，也不要发布到一半才失败。
/// 解析与全局那道检查共用 `git_cli::parse_git_version`，两边对同一段输出不会得出不同结论。
pub fn git_version_ok(text: &str) -> bool {
    crate::engine::git_cli::parse_git_version(text).is_some_and(|v| (v.0, v.1) >= MIN_GIT)
}

/// 跟进一次：判定为 ExternallyAdvanced 时走 F1–F6，InSync 而分支落后时走「分支跟进」入口。
pub fn follow(repo: &str, opid: &str) -> Result<FollowOutcome> {
    for _ in 0..MAX_ATTEMPTS {
        // 准备阶段里同一条 svn 扫描只跑一次（性能 P2）；发布前的复核在范围外，照常读最新的
        let plan = match with_scan_cache(|| prepare(repo, false)) {
            Ok(p) => p,
            Err(e) => return Ok(refused(e)),
        };
        fault::reach(fault::Point::Prepared, None);
        // 分支跟进入口上重放冲突：基线本来就在那里，没有要发布的东西
        if let (true, Target::Conflict(m)) = (plan.old_base == plan.new_base, &plan.target) {
            return Ok(needs_manual(m));
        }
        // 同步锁活到 publish 返回：冲突出口这种提前返回也在 Drop 里放掉
        let _lock = match svn_lock::acquire(repo, opid) {
            Ok(l) => l,
            Err(e) => return Ok(refused(e)),
        };
        match still_valid(repo, &plan) {
            Ok(true) => {
                let auto = auto_restorable(&plan);
                let out = publish(repo, opid, plan)?;
                // 同步锁还拿着
                return Ok(match out {
                    FollowOutcome::Done { mut report } => {
                        with_restore(repo, &mut report, &auto, NOT_MINE);
                        FollowOutcome::Done { report }
                    }
                    other => other,
                });
            }
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
        // 崩溃前那次的作者信息没有记进 journal，分不清哪些是我最后提交的：一个都不自动写，
        // 只记进待放回清单
        match resume_locked(repo, &mut j)? {
            Resumed::Finished(FollowOutcome::Done { mut report }) => {
                with_restore(repo, &mut report, &[], RESUMED);
                Resumed::Finished(FollowOutcome::Done { report })
            }
            other => other,
        }
    };
    match next {
        Resumed::Finished(o) => Ok(o),
        // follow 自己要拿同步锁，上面那把已经放了
        Resumed::Again => follow(repo, &new_opid()),
    }
}

// —— 自动跟进（分析 §6.12；计划 2026-11-06-svn-auto-follow） ——

/// `svn_follow_auto` 的结局。只有两种：做完了，或者**一个 ref 都没动**、留给横幅上的「跟进」。
///
/// 没有 `NeedsManual`：冲突要人判断（条件 7），自动跟进连基线都不发布。动过 ref 之后的失败与
/// [`follow`] 一样走 `Err`。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AutoFollowOutcome {
    Done {
        report: Box<FollowReport>,
    },
    /// 没自动跟进。`code` 区分是哪一条条件，`message` 直接给横幅，说清为什么、点「跟进」会怎样。
    Skipped {
        code: String,
        message: String,
    },
}

/// 判定写回 ExternallyAdvanced 之后由前端发起（设置里关掉时命令层直接返回 `Skipped`）。
///
/// 与 [`follow`] 走同一套准备与发布，多出分析 §6.12 的条件：
///
/// | # | 落点 |
/// |---|---|
/// | 1–4、10 | [`prepare`] 与 [`still_valid`] 的前置检查，原样拒绝 |
/// | 5 | 录制拒绝没确认成的删除（`HealthMap::Unconfirmed`），断网时由条件 6 挡住 |
/// | 6、7、8、11 | [`auto_blocker`]，在拿同步锁之前判 |
/// | 9 | 录制拒绝超过 `MAX_MAP_ENTRIES` 的映射 |
/// | 12 | [`publish_auto`]：先拼索引、看过决策再发布基线 |
/// | 13 | 没有撤销功能，恒真 |
pub fn follow_auto(repo: &str, opid: &str) -> Result<AutoFollowOutcome> {
    for _ in 0..MAX_ATTEMPTS {
        let plan = match with_scan_cache(|| prepare(repo, true)) {
            Ok(p) => p,
            Err(e) => return Ok(skipped(e)),
        };
        fault::reach(fault::Point::Prepared, None);
        if let Some(e) = auto_blocker(&plan) {
            return Ok(skipped(e));
        }
        let _lock = match svn_lock::acquire(repo, opid) {
            Ok(l) => l,
            Err(e) => return Ok(skipped(e)),
        };
        match still_valid(repo, &plan) {
            Ok(true) => {
                let auto = auto_restorable(&plan);
                let out = publish_auto(repo, opid, plan)?;
                // 同步锁还拿着
                return Ok(match out {
                    AutoFollowOutcome::Done { mut report } => {
                        with_restore(repo, &mut report, &auto, NOT_MINE);
                        AutoFollowOutcome::Done { report }
                    }
                    other => other,
                });
            }
            Ok(false) => continue,
            Err(e) => return Ok(skipped(e)),
        }
    }
    Ok(skipped(GitError::new(
        "svn_follow_state_changed",
        "准备跟进期间仓库一直在变（有别的工具在提交或切分支？），这次没有自动跟进，什么都没动。",
    )))
}

fn skipped(e: GitError) -> AutoFollowOutcome {
    AutoFollowOutcome::Skipped {
        code: e.code,
        message: e.message,
    }
}

/// 点「跟进」的结果与自动跟进不同的那几条，横幅上要说清楚：按钮仍然可以完成。
const TAIL_FOLLOW: &str = "点「跟进」照样可以完成。";

fn branch_behind_not_auto() -> GitError {
    GitError::new(
        "svn_auto_follow_branch_behind",
        format!(
            "当前分支落后基线，但 SVN 侧没有新内容：这是切到了旧分支，或者上次跟进遇到了冲突。\
             自动跟进只处理 SVN 侧的新内容，分支要不要跟上由你决定。{TAIL_FOLLOW}"
        ),
    )
}

/// 条件 6、7、8、11。返回第一条不满足的，按表里的顺序。
fn auto_blocker(plan: &Plan) -> Option<GitError> {
    // prepare(auto) 已经挡在前面，这里只是兜底：分支跟进入口没有新基线可发布
    if plan.old_base == plan.new_base {
        return Some(branch_behind_not_auto());
    }
    if !plan.report.verified {
        return Some(GitError::new(
            "svn_auto_follow_not_verified",
            format!(
                "这次没能向服务器核对新基线（多半是连不上 SVN 服务器）。没核对过的基线不自动发布。\
                 {TAIL_FOLLOW}报告里会标出「未经体检」。"
            ),
        ));
    }
    match &plan.target {
        Target::Tip(_) => {}
        Target::Conflict(m) if m.stale_disk => {
            return Some(GitError::new(
                "svn_auto_follow_stale_disk",
                format!(
                    "磁盘上还是这条分支的旧内容（{}），只移分支的话「本地改动」会变成把 SVN 的内容改回去。\
                     这要用会写磁盘的手动跟进，不能自动做。点「跟进」会给出手动跟进的入口。",
                    capped(m.files.clone(), "个文件")
                ),
            ))
        }
        Target::Conflict(m) => {
            return Some(GitError::new(
                "svn_auto_follow_conflict",
                format!(
                    "当前分支的提交重放到新基线上有冲突（{}），要你来判断。点「跟进」会弹出冲突的详情。",
                    capped(m.files.clone(), "个文件")
                ),
            ))
        }
    };
    if !plan.report.leaks.is_empty() {
        let paths: BTreeSet<String> = plan.report.leaks.iter().map(|l| l.path.clone()).collect();
        return Some(GitError::new(
            "svn_auto_follow_leak",
            format!(
                "当前分支的 Local-Only 补丁可能已经被带上了 SVN 服务器（{}）。这要你第一时间知道，\
                 不能被一次自动跟进带过去。{TAIL_FOLLOW}报告里会列出每一处。",
                capped(paths.into_iter().collect(), "个文件")
            ),
        ));
    }
    if let Some(e) = &plan.overlap_err {
        return Some(GitError::new(
            "svn_auto_follow_overlap_unknown",
            format!(
                "没能核对分支上的提交与 SVN 新内容有没有碰到同一批文件（{}），这次不自动跟进。\
                 {TAIL_FOLLOW}",
                e.message
            ),
        ));
    }
    // 方案 B：重叠但重放结果与磁盘逐字节相同的放行。「本地改动」= 磁盘 − tip，两者相同就不会有
    // W14 那种反向改动；drift 一定是 tip ≠ 磁盘，照样拦。例外是发布后会自动放回的文件
    // （[`auto_restorable`]）：放回之后磁盘 = tip（使用报告 2026-11-12 的 B）
    let auto: BTreeSet<String> = auto_restorable(plan).into_iter().collect();
    let bad: Vec<&OverlapFile> = plan
        .report
        .overlap
        .iter()
        .filter(|o| !o.matches_disk && !auto.contains(&o.path))
        .collect();
    if !bad.is_empty() {
        let mut commits: Vec<String> = Vec::new();
        for c in bad.iter().flat_map(|o| o.commits.iter()) {
            if !commits.contains(c) {
                commits.push(c.clone());
            }
        }
        let by = if commits.is_empty() {
            String::new()
        } else {
            format!("（{}）", capped(commits, "个提交"))
        };
        // 只比磁盘多 Local-Only 补丁、但不是我最后提交的：多半是我有意去掉了补丁（缺陷 7）
        let held: Vec<String> = bad
            .iter()
            .filter(|o| plan.report.restorable.contains(&o.path))
            .map(|o| o.path.clone())
            .collect();
        let why = if held.len() == bad.len() {
            format!(
                "分支只比磁盘多 Local-Only 补丁，但 SVN 上最后改它的不是你（或认不出是谁），可能是你\
                 有意去掉的补丁，不自动放回。{TAIL_FOLLOW}要放回的话，报告里有按钮。"
            )
        } else if !held.is_empty() {
            format!(
                "其中 {} 只比磁盘多 Local-Only 补丁，但 SVN 上最后改它的不是你，不自动放回；其余的可能是\
                 文件里还有没提交的改动，也可能是重放合错了，要你看一眼。{TAIL_FOLLOW}\
                 报告会逐个标出这些文件是否与磁盘一致。",
                capped(held, "个文件")
            )
        } else {
            format!(
                "可能是文件里还有没提交的改动，也可能是重放合错了，要你看一眼。{TAIL_FOLLOW}\
                 报告会逐个标出这些文件是否与磁盘一致。"
            )
        };
        return Some(GitError::new(
            "svn_auto_follow_overlap",
            format!(
                "当前分支上的提交{by}改过的文件，SVN 侧也有新内容，重放出来的结果和磁盘对不上（{}）。{why}",
                capped(bad.iter().map(|o| o.path.clone()).collect(), "个文件")
            ),
        ));
    }
    None
}

/// 发布后会自动放回的文件：能放回（`restorable`），而且 SVN 上最后一次改它的是我。
///
/// 不是我的不自动写：用户有意去掉补丁、还没提交时，`svn update` 带来别人对这个文件的改动，跟进后
/// 它同样能放回，自动写回会撤销用户的准备（计划 `2026-11-12-auto-restore-local-only.md` 的 A）。
fn auto_restorable(plan: &Plan) -> Vec<String> {
    plan.report
        .restorable
        .iter()
        .filter(|p| plan.who.is_me(p) == Some(true))
        .cloned()
        .collect()
}

const NOT_MINE: &str =
    "SVN 上最后改它的不是你（或认不出是谁），可能是你有意去掉的补丁，没有自动放回。";

/// 崩溃恢复走完的跟进：作者信息没进 journal，分不清是谁最后提交的（review M3：不能说成「不是你」）。
const RESUMED: &str =
    "这次跟进是崩溃后续上的，分不清 SVN 上最后改它的是不是你，没有自动放回。请确认后再放回。";

/// 跟进完成之后、**同步锁还拿着**时调用：放回 `auto`，结果写进 `report.restore`，`restorable`
/// 里没放回的记进待放回清单（横幅兜底）。`held` 是不在 `auto` 里的那些文件写给用户看的原因。
/// 没有可放回的文件时只修剪清单。
///
/// 跟进已经做完了，这里出什么错都不能让它变成失败：放回的前置条件不满足时，`auto` 全部记成没放回、
/// 写上原因；清单写不进去只记日志。
fn with_restore(repo: &str, report: &mut FollowReport, auto: &[String], held: &str) {
    if report.restorable.is_empty() {
        // 没有新的可放回文件也要修剪旧条目：否则失效的条目要等下一次碰巧有可放回文件的跟进才删得掉，
        // 期间每次状态刷新都白跑一遍核对（review M2）
        if let Err(e) = record_pending(repo, &[], &[]) {
            log::warn!("修剪待放回清单失败：{}", e.message);
        }
        return;
    }
    let mut out = restore_locked(repo, auto).unwrap_or_else(|e| RestoreOutcome {
        skipped: auto
            .iter()
            .map(|p| RestoreSkip {
                path: p.clone(),
                reason: format!("没能自动放回：{}", e.message),
                retry: true,
            })
            .collect(),
        ..RestoreOutcome::default()
    });
    for p in &report.restorable {
        if !auto.contains(p) {
            out.skipped.push(RestoreSkip {
                path: p.clone(),
                reason: held.to_string(),
                retry: true,
            });
        }
    }
    // 没过树这几关的（放回前置条件不满足、不是我的）也要写清是哪个补丁
    let missing: Vec<String> = report
        .restorable
        .iter()
        .filter(|p| !out.commits.contains_key(*p))
        .cloned()
        .collect();
    match local_only_commits(repo, &missing) {
        Ok(more) => out.commits.extend(more),
        Err(e) => log::warn!("读改过待放回文件的 Local-Only 提交失败：{}", e.message),
    }
    let later: Vec<String> = out
        .skipped
        .iter()
        .filter(|s| s.retry)
        .map(|s| s.path.clone())
        .collect();
    if let Err(e) = record_pending(repo, &later, &out.restored) {
        log::warn!("更新待放回清单失败：{}", e.message);
    }
    report.restore = Some(out);
}

/// 条件 11 的重叠文件：重放后留下的 commit（`new..tip`）相对各自新父提交仍改动的路径与外部变更的
/// 交集，再并上 `drift`（是它的子集，兜底），逐个核对 tip 与磁盘是否逐字节相同（方案 B）。
///
/// 按重放之后算：F2 ① 换成 SVN 版本的路径、6.10 的快照内容已经不在 commit 里了，「C1 改了 a 和
/// e、只有 a 被提交」这种情形 C1' 只剩 e，不受影响（分析 §6.12 条件 11）。
///
/// 磁盘内容按 `git add` 的过滤规则算（同 [`drift_paths`]）。tip 与磁盘都没有这个文件算相同；只差
/// 可执行位不算不同（同 [`drift_paths`] 的 review 第 5 条）；符号链接、子模块、目录一律算不同。
fn overlap_files(
    repo: &str,
    new: &str,
    tip: &str,
    external: &BTreeSet<String>,
    drift: &[String],
) -> Result<Vec<OverlapFile>> {
    let mut by_path = retained_by_path(repo, new, tip, external)?;
    for p in drift {
        by_path.entry(p.clone()).or_default();
    }
    if by_path.is_empty() {
        return Ok(Vec::new());
    }
    let r = Repository::open(repo)?;
    let tree = r.revparse_single(tip)?.peel_to_tree()?;
    let mut out: Vec<OverlapFile> = Vec::new();
    // (out 的下标, tip 的 blob)
    let mut to_hash: Vec<(usize, String)> = Vec::new();
    for (path, commits) in by_path {
        let entry = tree.get_path(Path::new(&path)).ok();
        let on_disk = Path::new(repo).join(&path).symlink_metadata().ok();
        let matches_disk = match (&entry, &on_disk) {
            (None, None) => true,
            (Some(e), Some(m)) if matches!(e.filemode(), 0o100644 | 0o100755) && m.is_file() => {
                to_hash.push((out.len(), e.id().to_string()));
                false
            }
            _ => false,
        };
        out.push(OverlapFile {
            path,
            commits,
            matches_disk,
        });
    }
    if !to_hash.is_empty() {
        let input: String = to_hash
            .iter()
            .map(|(i, _)| format!("{}\n", out[*i].path))
            .collect();
        let hashed = run_git_stdin(repo, &["hash-object", "--stdin-paths"], input.as_bytes())?;
        let disk: Vec<&str> = hashed.lines().map(str::trim).collect();
        if disk.len() != to_hash.len() {
            return Err(GitError::new(
                "svn_follow_hash_failed",
                "核对磁盘内容时 git hash-object 的输出行数对不上。这次什么都没动，请再点一次「跟进」。",
            ));
        }
        for ((i, blob), d) in to_hash.iter().zip(disk) {
            out[*i].matches_disk = d == blob;
        }
    }
    Ok(out)
}

/// 重放后留下的 commit 改过、又在外部变更里的路径 → 改过它的 commit（`短 hash 提交说明`，从新到旧）。
fn retained_by_path(
    repo: &str,
    new: &str,
    tip: &str,
    external: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut hit: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if new == tip || external.is_empty() {
        return Ok(hit);
    }
    // 每个 commit 是 `\x01<短 hash>\x02<说明>` NUL，再跟 `\n<路径>` NUL、`<路径>` NUL ……；没改任何
    // 路径的 commit 只有头（2026-11-11 实测 git 2.51）
    let out = run_git_bytes(
        repo,
        &[
            "log",
            "--format=%x01%h%x02%s",
            "--name-only",
            "--no-renames",
            "-z",
            &format!("{new}..{tip}"),
        ],
    )?;
    let mut current = String::new();
    for raw in out.split(|b| *b == 0) {
        if let Some(head) = raw.strip_prefix(b"\x01") {
            current = String::from_utf8_lossy(head).replacen('\x02', " ", 1);
            continue;
        }
        let raw = raw.strip_prefix(b"\n").unwrap_or(raw);
        if raw.is_empty() {
            continue;
        }
        // 外部路径都是 UTF-8（来自 svn 的 XML）。读不懂的路径不可能命中，但宁可不自动
        let path = std::str::from_utf8(raw).map_err(|_| {
            GitError::new(
                "svn_follow_path_not_utf8",
                "分支上的提交里有不是 UTF-8 的路径。",
            )
        })?;
        if external.contains(path) {
            let commits = hit.entry(path.to_string()).or_default();
            if !commits.contains(&current) {
                commits.push(current.clone());
            }
        }
    }
    Ok(hit)
}

/// 条件 12：拼索引时有暂存内容没能保留（三方合并冲突、二进制、删除 / mode 变化、文件与目录
/// 撞名，即 `lost`），或「作者是我但先后拿不准」（`uncertain`）。
///
/// 放行的两种，通知里都会列出来（分析 §6.12「通知」）：
/// - 三方合并干净（`Merged`）：暂存内容与 SVN 的改动都在，是分析 §6.11.1 表里确定的一行。计划
///   原写「无三方合并」，比分析的「无三方合并冲突」严，按分析（日志 Task 1「计划的错误」）
/// - `discarded`：作者是我、确定早于提交的中间版本。原内容钉在 `refs/mygitclient/follow/<opid>`
fn stage_blocker(report: &ComposeReport) -> Option<GitError> {
    let mut paths: BTreeSet<String> = report.lost.iter().cloned().collect();
    paths.extend(report.uncertain.iter().cloned());
    if paths.is_empty() {
        return None;
    }
    Some(GitError::new(
        "svn_auto_follow_stage",
        format!(
            "暂存区里有文件保不住暂存的内容（和 SVN 的改动合不到一起），或者拿不准是不是比 SVN 上的\
             更新（{}）。自动跟进不替你决定暂存内容的去留。{TAIL_FOLLOW}报告会写明每个文件怎么处理的。",
            capped(paths.into_iter().collect(), "个文件")
        ),
    ))
}

/// 自动跟进的发布：**先拼索引、后发布基线**（计划 §3 条件 12 的「结构性缺口」）。
///
/// [`publish`] 先 CAS `svn/base` 再拼索引，拼索引的决策出来时基线已经动了。这里把顺序倒过来：
/// 拿着 `.git/index.lock` 拼好索引、看过 [`stage_blocker`]，不满足就删锁、删 journal，一个 ref
/// 都没动；满足才 CAS 基线，之后与 [`publish`] 相同（[`swap_in`]）。
///
/// 崩溃恢复不用改：journal 一开始就记 [`FollowStep::PrepareIndex`]，这一步的恢复是「删掉我方的
/// 锁、清记录、重新跟进」，不管基线动没动都是一致状态（[`resume_locked`]）。代价是 index.lock
/// 多拿一次 `update-ref` 的时间。手动跟进的顺序不变：它不看决策，没理由多锁这一下。
fn publish_auto(repo: &str, opid: &str, plan: Plan) -> Result<AutoFollowOutcome> {
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
    let Target::Tip(tip) = target else {
        // auto_blocker 已经挡掉了冲突
        return Ok(skipped(GitError::new(
            "svn_auto_follow_conflict",
            format!("当前分支重放有冲突，要你来判断。{TAIL_FOLLOW}"),
        )));
    };
    let mut j = publishing_journal(
        opid,
        Moves {
            branch: &branch,
            old_head: &old_head,
            old_base: &old_base,
            new_base: &new_base,
            tip: Some(tip.clone()),
        },
        others,
        report,
        FollowStep::PrepareIndex,
    );
    if let Err(e) = svn_journal::write(repo, &j) {
        return Ok(skipped(e));
    }
    fault::reach(fault::Point::Journaled, None);
    fault::reach(fault::Point::BeforePrepare, None);

    // 什么都还没动：失败与不满足条件 12 都是删锁、删记录、丢掉钉住的旧索引（它就是此刻的真实索引）
    let give_up = |prepared: Option<PreparedIndex>, e: GitError| -> Result<AutoFollowOutcome> {
        let pin = prepared.as_ref().map(|p| p.pinned_ref().to_string());
        drop(prepared);
        svn_journal::remove(repo)?;
        let pin = pin.unwrap_or_else(|| format!("refs/mygitclient/follow/{opid}"));
        if let Err(err) = run_git(repo, &["update-ref", "-d", &pin]) {
            log::warn!("删除钉住的索引 {pin} 失败：{}", err.message);
        }
        Ok(skipped(e))
    };
    // 步骤留在 PrepareIndex：基线还没发布，恢复不能往前移分支（见 record_prepared）
    let heads = (old_head.as_str(), tip.as_str());
    let mut prepared =
        match compose_index(repo, opid, &mut j, &who, heads, FollowStep::PrepareIndex) {
            Ok(p) => p,
            Err(e) => return give_up(None, e),
        };
    if let Some(e) = stage_blocker(&prepared.report) {
        return give_up(Some(prepared), e);
    }
    if let Err(e) = move_baseline_ref(repo, &new_base, Some(&old_base)) {
        return give_up(Some(prepared), e);
    }
    fault::reach(fault::Point::BaseMoved, Some(&mut prepared));
    // 基线发布了才能推进：从这里起恢复会把分支往前移。写不进去时基线已动、分支没动，按
    // publish 拼索引失败那样收场（一致状态，「跟进」会走分支跟进入口）
    if let Err(e) = advance(repo, &mut j, FollowStep::MoveBranch) {
        prepared.abandon();
        svn_journal::remove(repo)?;
        return Err(baseline_kept(e));
    }

    match swap_in(repo, &mut j, prepared, &branch, &old_head, &tip)? {
        FollowOutcome::Done { report } => Ok(AutoFollowOutcome::Done { report }),
        // swap_in 只会以 Done 结束；万一不是，照实交给横幅
        other => Ok(skipped(GitError::new(
            "svn_follow_index_failed",
            format!("自动跟进的结局不是「完成」：{other:?}"),
        ))),
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

fn needs_manual(m: &ManualNeeded) -> FollowOutcome {
    FollowOutcome::NeedsManual {
        at: m.at.clone(),
        files: m.files.clone(),
        baseline_published: true,
        leaks: m.leaks.clone(),
        stale_disk: m.stale_disk,
        snapshot: m.snapshot.clone(),
        details: m.details.clone(),
    }
}

/// 冲突框里一个冲突文件的上下文（事故分析 6.5）。11-05 那次要手工跑 git log / svn log / svn cat
/// 才看清冲突是什么。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDetail {
    pub path: String,
    /// SVN 上最后一次改它的 revision 与作者（跟进录基线时读到的）。本次外部变更里没有它时为 `None`。
    pub svn_rev: Option<u64>,
    pub svn_author: Option<String>,
    /// SVN 上把它删了。
    pub svn_removed: bool,
    /// 第一处冲突块，标签换成人话，每一侧最多 [`PREVIEW_SIDE_LINES`] 行（见 [`preview_of`]）。二进制、太大、没有冲突标记（如修改与删除
    /// 撞上）时为 `None`。
    pub preview: Option<String>,
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
    let rebase_todo = snapshot_todo(repo, &scope, &head, &base)?;
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
        rebase_todo,
        ..Default::default()
    };
    svn_journal::write(repo, &j)?;
    stash_wip(repo, &mut j, &cfg.whitelist)?;
    fault::reach(fault::Point::ManualStashed, None);
    // S8 在这里不切分支（HEAD 一直在原分支上），只做复核；S9 重放到 `newBaselineOid`
    from_baseline_ready(repo, &cfg, &mut j)
}

/// 手动跟进的 rebase 要逐个 pick 的 commit（事故分析 6.10，方案 §3）。
///
/// rebase 原样 pick 分支上的 commit，快照进来的 SVN 内容会和新基线撞上：对象层重放已经把它们换成了
/// SVN 版本，手动跟进不跟着换的话，11-05 那 35 个冲突在磁盘上又回来了。这里为命中的 commit 造 C'
/// （[`commit_without`]），其余照用原 commit。一个都没命中时返回空，S9 照原样 `rebase <新基线>`。
///
/// 整个 commit 都是快照内容时（C' 的树等于 C^ 的树、原 C 本来不空），不放进列表（独立 review 第 3 条）：
/// rebase 会保留「一开始就为空」的 commit，`--empty=drop` 只丢重放后才变空的，分支上就会留下一个空的
/// 「svn up 之后整包提交」，与对象层把它记成 `dropped` 不一致。一个都不剩时列表是 [`NOOP`]。
///
/// 列表放在环境变量里交给 rebase（见 `svn_sync_in::rebase_with_todo`）。放不下时（[`too_many_for_todo`]）
/// 退回原样 rebase，并记日志（独立 review 第 9 条）。
fn snapshot_todo(repo: &str, scope: &str, head: &str, new: &str) -> Result<Vec<String>> {
    let commits = commits_between(repo, scope, head, &FOLLOW_CODES)?;
    if too_many_for_todo(commits.len()) {
        log::warn!(
            "待重放的 commit 有 {} 个，环境变量放不下，手动跟进按原样 rebase（快照内容不按基线走）",
            commits.len()
        );
        return Ok(Vec::new());
    }
    let rule = SnapshotRule::new(repo, new);
    let mut any = false;
    let mut todo = Vec::with_capacity(commits.len());
    for c in &commits {
        let hits = rule.paths(scope, c);
        let oid = commit_without(repo, c, &hits, &FOLLOW_CODES)?;
        if oid == c.oid {
            todo.push(oid);
            continue;
        }
        any = true;
        let trees = run_git(
            repo,
            &[
                "rev-parse",
                &format!("{oid}^{{tree}}"),
                &format!("{oid}^^{{tree}}"),
            ],
        )?;
        let mut t = trees.lines();
        // C' 换过路径所以原 C 一定改过东西；C' 再与父提交同树就是整个变空了
        if t.next() != t.next() {
            todo.push(oid);
        }
    }
    if !any {
        return Ok(Vec::new());
    }
    if todo.is_empty() {
        return Ok(vec![NOOP.to_string()]);
    }
    Ok(todo)
}

/// 待重放列表里「什么都不 pick」的那一行：分支上的 commit 全是快照内容。
pub(crate) const NOOP: &str = "noop";

/// 待重放列表放不进一个环境变量（Windows 约 32K 字符，每行 `pick <40 位 OID>\n` 46 个字符）。
/// 按分支上的 commit 数判，[`snapshot_todo`] 与 [`manual_snapshot`] 用同一个判据：冲突框说「会直接
/// 采用新基线」，手动跟进就一定是那样做的（第 2 轮 review 低危第 2 条）。
fn too_many_for_todo(commits: usize) -> bool {
    commits * 46 > 30_000
}

/// 冲突出口告诉用户的：手动跟进时哪些 commit 里的哪些路径会直接采用新基线（方案 §3）。
/// 与 [`snapshot_todo`] 用同一条规则；内容本来就等于 NEW 的不列（见 [`SnapshotRule::reported_paths`]）。
fn manual_snapshot(commits: &[ReplayCommit], rule: &SnapshotRule, scope: &str) -> PathsByCommit {
    if too_many_for_todo(commits.len()) {
        return Vec::new();
    }
    commits
        .iter()
        .filter_map(|c| {
            let p = rule.reported_paths(scope, c);
            (!p.is_empty()).then(|| (c.oid.clone(), p.into_iter().collect()))
        })
        .collect()
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
    Conflict(ManualNeeded),
}

/// 交给手动跟进的现场，字段见 [`FollowOutcome::NeedsManual`]。
struct ManualNeeded {
    at: String,
    files: Vec<String>,
    leaks: Vec<Leak>,
    stale_disk: bool,
    /// 手动跟进会按 6.10 直接采用新基线版本的路径，见 [`manual_snapshot`]。
    snapshot: PathsByCommit,
    details: Vec<ConflictDetail>,
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
    /// 条件 11 的核对没做成（[`overlap_files`] 出错）。报告里的 `overlap` 这时为空，自动跟进
    /// 不能把它当成「没有重叠」。
    overlap_err: Option<GitError>,
}

/// 判定「是不是我提交的」「早于外部提交吗」的线索（计划 Task 9）。全部拿不准时走保守分支。
struct Who {
    me: Option<String>,
    mark: Option<FollowMark>,
    /// 路径 → 最后一次提交的作者（`svn status -v` 的 `<commit author>`）。
    authors: BTreeMap<String, Option<String>>,
    /// 快照之后才进基线的路径（见 [`paths_after_mark`]）。`None`：分不清快照与外部提交的先后。
    after_mark: Option<BTreeSet<String>>,
    /// TortoiseSVN 钩子的本机事件（C1h；分析 §6.13.3）。只有钩子健康时才是 `Some`，这时按
    /// 事件判定，不再按作者猜。
    events: Option<HookEvidence>,
    /// 读收件箱的时刻，见 [`FollowReport::evidence_cutoff_ms`]。
    events_read_at_ms: Option<i64>,
}

impl Who {
    fn gather(repo: &str, repos_root: &str, nodes: &[NodeStatus]) -> Result<Self> {
        let health = fault::pretended_hook_health().unwrap_or_else(|| quick_health(repo));
        // 先记时刻再读。钩子的 `at_ms` 取在写文件之前，所以一条事件可能时刻早于截点却没被这次
        // 读到，随后被当成已处理清掉：只是少了一条证据，落到「拿不准」（Review 2 Minor 第 1 条）
        let (events, events_read_at_ms) = match health {
            HookHealth::Healthy { .. } => {
                let at = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_millis() as i64);
                (Some(read_evidence(repo)), Some(at))
            }
            _ => (None, None),
        };
        Ok(Self {
            me: my_svn_user(repo, repos_root),
            mark: read_mark(repo)?,
            authors: nodes
                .iter()
                .map(|n| (n.path.clone(), n.author.clone()))
                .collect(),
            after_mark: None,
            events,
            events_read_at_ms,
        })
    }

    /// 本机提交事件里、提交开始时刻晚于失焦快照的路径（分析 §6.13.3 表第 1、3 行的前提）。
    /// 快照或 t_commit 有一个不知道，都不算：拿不准。
    fn committed_after_mark(&self, c: &CommitEvidence) -> bool {
        let mark_ms = self.mark.as_ref().and_then(|m| m.at_ms);
        matches!((mark_ms, c.t_commit_ms), (Some(m), Some(t)) if m < t)
    }

    /// 按钩子事件翻成 [`PathFacts`]（计划 Task 5 的表）。同一路径既有提交又有更新时取
    /// revision 高的那条，相同时按提交：同一个 revision 里有我的提交，这个文件就是我交的。
    fn facts_from_events(
        &self,
        ev: &HookEvidence,
        path: &str,
        staged: Option<&BTreeMap<String, String>>,
    ) -> PathFacts {
        let commit = ev
            .commits
            .iter()
            .filter(|c| c.paths.contains(path))
            .max_by_key(|c| c.revision);
        let update = ev
            .updates
            .iter()
            .filter(|u| u.paths.contains(path))
            .max_by_key(|u| u.revision);
        let uncertain = PathFacts {
            author_is_me: Some(true),
            staged_before_mark: Some(false),
        };
        match (commit, update) {
            (Some(c), u) if u.is_none_or(|u| u.revision <= c.revision) => {
                if !self.committed_after_mark(c) {
                    return uncertain;
                }
                PathFacts {
                    author_is_me: Some(true),
                    staged_before_mark: self.staged_in_mark(path, staged),
                }
            }
            (_, Some(_)) => PathFacts {
                author_is_me: Some(false),
                staged_before_mark: None,
            },
            // 钩子健康却没有事件：另一台机器、命令行 svn、IDE 插件。作者是我也不按作者丢弃（R22）
            _ => match self.is_me(path) {
                Some(true) => uncertain,
                other => PathFacts {
                    author_is_me: other,
                    staged_before_mark: None,
                },
            },
        }
    }

    /// 当前暂存是否与快照里的一样。快照把暂存的删除记成全 0；索引里没有这条路径就是暂存了删除。
    fn staged_in_mark(
        &self,
        path: &str,
        staged: Option<&BTreeMap<String, String>>,
    ) -> Option<bool> {
        let m = self.mark.as_ref()?;
        let zero = "0".repeat(m.head_oid.len());
        let now = staged?.get(path).unwrap_or(&zero);
        Some(m.staged.get(path) == Some(now))
    }

    /// 匿名提交不可能是我：`Some(false)`。用户名取不到、节点不在：`None`（D4）。
    fn is_me(&self, path: &str) -> Option<bool> {
        let me = self.me.as_deref()?;
        let author = self.authors.get(path)?;
        Some(author.as_deref() == Some(me))
    }

    /// 拼索引要的事实。`staged` 是当前索引的暂存 blob，只在持索引锁时读一次（见 [`staged_blobs`]）。
    fn facts(&self, path: &str, staged: Option<&BTreeMap<String, String>>) -> PathFacts {
        if let Some(ev) = &self.events {
            return self.facts_from_events(ev, path, staged);
        }
        let after_mark = self.after_mark.as_ref().is_some_and(|s| s.contains(path));
        PathFacts {
            author_is_me: self.is_me(path),
            // 这条路径的 SVN 提交早于快照时，快照里的暂存可能是提交之后的新内容，推不出先后
            staged_before_mark: if after_mark {
                self.staged_in_mark(path, staged)
            } else {
                None
            },
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
///
/// `auto`：自动跟进只接 ExternallyAdvanced（日志 Task 1「偏离计划」），分支跟进入口在录制之前就停。
fn prepare(repo: &str, auto: bool) -> Result<Plan> {
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
        SyncVerdict::InSync if auto => return Err(branch_behind_not_auto()),
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
    // 有钩子事件时，「我在快照之后提交的」直接取本机提交事件，不按作者猜（分析 §6.13.3 表第 3 行）
    let mine: BTreeSet<String> = match &who.events {
        Some(ev) => ev
            .commits
            .iter()
            .filter(|c| who.committed_after_mark(c))
            .flat_map(|c| c.paths.iter().cloned())
            .collect(),
        None => ext.mine(&who),
    };
    let replaceable: BTreeSet<String> = match &who.after_mark {
        Some(after) => mine.intersection(after).cloned().collect(),
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
    // 6.10：快照进来的 SVN 内容按基线走。规则按分支自己的基线算，其他分支在 `plan_other` 里另拼
    let snapshot = SnapshotRule::new(repo, &new_base);
    let current = |c: &ReplayCommit| -> BTreeSet<String> {
        let mut p = superseded(c);
        p.extend(snapshot.paths(&scope, c));
        p
    };

    let map_floor = read_base_map(repo, &new_base)?.floor;
    let mut report = FollowReport {
        new_floor: map_floor,
        verified,
        changes,
        removed,
        evidence: if who.events.is_some() {
            Evidence::Hooks
        } else {
            Evidence::Heuristic
        },
        evidence_cutoff_ms: who.events_read_at_ms,
        ..Default::default()
    };

    // F2 当前分支。memo 跨分支共享：共享的 commit 只重放一次（R20）
    let mut memo = BTreeMap::new();
    let commits = commits_between(repo, &scope, &old_head, &FOLLOW_CODES)?;
    let replayed = replay_branch(
        repo, &scope, &old_head, &new_base, &commits, &current, &mut memo,
    )?;
    let stale = match &replayed {
        Replayed::Moved { tip, .. } => stale_disk_paths(repo, &scope, &old_base, &old_head, tip)?,
        Replayed::Conflict { .. } => Vec::new(),
    };
    let mut overlap_err = None;
    let (target, others) = match replayed {
        Replayed::Conflict { at, files, tree } => {
            // F3 也要算：分支没重放，但补丁可能已经上了服务器，用户要在手动跟进之前知道
            let leaks = local_only_leaks(repo, &branch, &commits, &paths, &new_base)?;
            let details = conflict_details(repo, &files, &tree, &ext);
            (
                Target::Conflict(ManualNeeded {
                    at,
                    files,
                    leaks,
                    stale_disk: false,
                    snapshot: manual_snapshot(&commits, &snapshot, &scope),
                    details,
                }),
                Vec::new(),
            )
        }
        // 重放得上，但只移 HEAD 会在「本地改动」里造出反向修改：交给会写磁盘的手动跟进
        Replayed::Moved { .. } if !stale.is_empty() => {
            let leaks = local_only_leaks(repo, &branch, &commits, &paths, &new_base)?;
            (
                Target::Conflict(ManualNeeded {
                    at: old_head.clone(),
                    files: stale,
                    leaks,
                    stale_disk: true,
                    snapshot: manual_snapshot(&commits, &snapshot, &scope),
                    details: Vec::new(),
                }),
                Vec::new(),
            )
        }
        Replayed::Moved {
            tip,
            dropped,
            rewritten,
        } => {
            report.dropped = dropped;
            let (snap, own) = split_snapshot(rewritten, &commits, &superseded, &snapshot, &scope);
            report.rewritten = own;
            report.snapshot = snap;
            // 只是告警：核对本身出错（文件被独占锁住之类）不能让跟进失败（review 第 6 条）
            report.drift = drift_paths(repo, &new_base, &tip, &paths).unwrap_or_else(|e| {
                log::warn!("核对重放结果与磁盘时出错，跳过：{}", e.message);
                Vec::new()
            });
            // 9.1：只读挑出能放回的。出错只是少一个按钮
            report.restorable = restorable_paths(repo, &new_base, &tip, &report.drift)
                .unwrap_or_else(|e| {
                    log::warn!("判断能否放回 Local-Only 补丁时出错，跳过：{}", e.message);
                    Vec::new()
                });
            // 条件 11 的逐文件核对。手动跟进只写进报告；出错时自动跟进不做（auto_blocker）
            match overlap_files(repo, &new_base, &tip, &paths, &report.drift) {
                Ok(o) => report.overlap = o,
                Err(e) => {
                    log::warn!("核对重叠文件与磁盘时出错，跳过：{}", e.message);
                    overlap_err = Some(e);
                }
            }
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
                    snapshot: &snapshot,
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
        overlap_err,
    })
}

/// 录新基线（分析 §6.3）。候选索引从旧基线树起步、沿用真实索引的 stat，免得每次跟进都把白名单
/// 全量哈希一遍（[`TempIndex::from_tree_keeping_stat`]）。
///
/// 原来从空索引起步，为的是甩掉服务器上已删除的文件；从旧基线起步时它们磁盘上已经不在，
/// `add -A` 照样摘掉。**文件集合**上不等价的只有一种：旧基线里有现在已不算 W 的文件（白名单或
/// 扩展名收窄过），它是已跟踪文件，`add -A` 会留下它，R2c 的纳入集合核对随即报
/// `include_set_mismatch`。这时退回空索引按原来的口径再录一次。
///
/// 集合之外还有两处与空索引不同（独立 review 2026-11-01 实测），都与 git 自己对真实索引的口径一致：
/// - mode：filemode=false 时沿用旧基线条目的 mode，不再每次归一成 100644（有意，见测试
///   `a_recording_from_the_old_base_keeps_its_file_modes`）。
/// - 过滤规则（autocrlf、`.gitattributes` 的 eol / filter）改了之后，stat 没变的文件不会按新规则
///   重新 clean，blob 仍是旧规则的结果；体检比的是磁盘与服务器，看不出来。已迁移的仓库由
///   `.git/info/attributes` 钉住 text / eol（`svn_attributes`），基本碰不到；碰到时「重建基线」
///   从空索引录，可以纠正。
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

/// (原 commit, 路径) 的列表，报告里 `rewritten` / `snapshot` 的形状。
type PathsByCommit = Vec<(String, Vec<String>)>;

/// 把重放换过的路径按来源分开：(6.10 快照内容, F2 ① 我自己提交到 SVN 的)。两条规则都命中的
/// 算 F2 ①：那是我自己的提交，「你已经提交到了 SVN」比「多半是快照」更准。
fn split_snapshot(
    rewritten: PathsByCommit,
    commits: &[ReplayCommit],
    f2: &dyn Fn(&ReplayCommit) -> BTreeSet<String>,
    rule: &SnapshotRule,
    scope: &str,
) -> (PathsByCommit, PathsByCommit) {
    let mut snap = Vec::new();
    let mut own = Vec::new();
    for (oid, paths) in rewritten {
        let (hits, reported, mine) = match commits.iter().find(|c| c.oid == oid) {
            Some(c) => (rule.paths(scope, c), rule.reported_paths(scope, c), f2(c)),
            None => (BTreeSet::new(), BTreeSet::new(), BTreeSet::new()),
        };
        let mut s = Vec::new();
        let mut o = Vec::new();
        for p in paths {
            if mine.contains(&p) || !hits.contains(&p) {
                o.push(p);
            } else if reported.contains(&p) {
                s.push(p);
            }
            // 其余：快照命中、且 commit 里的内容本来就与 NEW 相同，换不换都一样，不报（review 第 7 条）
        }
        if !s.is_empty() {
            snap.push((oid.clone(), s));
        }
        if !o.is_empty() {
            own.push((oid, o));
        }
    }
    (snap, own)
}

/// 冲突预览里每一侧（新基线 / 共同的旧版本 / 你的提交）最多几行。按侧限，不按总行数限（6.5 review M1：
/// SVN 那一侧一长，「你的提交」就整段看不到了）。冲突框是给人扫一眼的，完整内容在手动跟进时由合并工具给。
pub const PREVIEW_SIDE_LINES: usize = 6;
/// 单行最多几个字符：minified 的 JSON / JS 一行就能有几百 KB。
pub const PREVIEW_LINE_CHARS: usize = 200;
/// 预览最多给几个冲突文件：11-05 那次有 37 个，全取既慢又没人看。按取成功的计（review L1）。
const PREVIEW_FILES: usize = 5;
/// 最多尝试几个文件：前面的都是二进制或修改与删除撞上时，后面的文本冲突也要有机会。
const PREVIEW_TRIES: usize = 10;
/// 超过这个大小的冲突文件不读预览。
const PREVIEW_MAX_BYTES: u64 = 512 * 1024;

/// 每个冲突文件的上下文（事故分析 6.5）。SVN 侧的作者与 revision 来自这次录基线时的外部变更清单，
/// 不额外跑 svn；预览来自 merge-tree 的冲突树，只读 git 对象。预览读失败只是没有预览。
fn conflict_details(
    repo: &str,
    files: &[String],
    tree: &str,
    ext: &External,
) -> Vec<ConflictDetail> {
    let mut shown = 0;
    files
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let change = ext.changes.iter().find(|c| &c.path == path);
            let preview = if shown < PREVIEW_FILES && i < PREVIEW_TRIES {
                conflict_preview(repo, tree, path)
            } else {
                None
            };
            if preview.is_some() {
                shown += 1;
            }
            ConflictDetail {
                path: path.clone(),
                svn_rev: change.map(|c| c.rev).filter(|r| *r > 0),
                svn_author: change.and_then(|c| c.author.clone()),
                svn_removed: ext.removed.contains(path),
                preview,
            }
        })
        .collect()
}

/// 冲突树里 `path` 的第一处冲突块（见 [`preview_of`]）。先问大小再读（review L2：二进制冲突时树里
/// 留的是整份原文件，Unity 资源能有几百 MB）。不是 UTF-8 的（国内 Unity 工程里 GBK 的 `.cs` 常见）
/// 照样给，开头说一句可能是乱码（review L3）。
fn conflict_preview(repo: &str, tree: &str, path: &str) -> Option<String> {
    let spec = format!("{tree}:{path}");
    let size: u64 = run_git(repo, &["cat-file", "-s", &spec])
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if size > PREVIEW_MAX_BYTES {
        return None;
    }
    let bytes = run_git_bytes(repo, &["cat-file", "blob", &spec]).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    match std::str::from_utf8(&bytes) {
        Ok(text) => preview_of(text),
        Err(_) => preview_of(&String::from_utf8_lossy(&bytes))
            .map(|p| format!("（不是 UTF-8 编码，中文可能显示为乱码）\n{p}")),
    }
}

/// 文本里第一处冲突块，标签换成人话。merge-tree 的标签是 OID；重放时「我们这边」是新基线（第 2 个及
/// 以后的 commit 冲突时还含着你之前已重放的提交），「他们那边」是被重放的 commit，与 rebase 的方向相同
/// （11-05 用户用外部合并工具时就弄反过）。
///
/// 每一侧最多 [`PREVIEW_SIDE_LINES`] 行，多出的写「…（还有 N 行）」；分隔线与 `>>>>>>> 你的提交` 总是在。
/// 单行截到 [`PREVIEW_LINE_CHARS`] 个字符。没有冲突标记时为 `None`。
pub fn preview_of(text: &str) -> Option<String> {
    fn flush(out: &mut Vec<String>, side: &mut Vec<String>) {
        let n = side.len();
        out.extend(side.drain(..).take(PREVIEW_SIDE_LINES));
        if n > PREVIEW_SIDE_LINES {
            out.push(format!("…（还有 {} 行）", n - PREVIEW_SIDE_LINES));
        }
    }
    fn clip(line: &str) -> String {
        if line.chars().count() <= PREVIEW_LINE_CHARS {
            return line.to_string();
        }
        let mut s: String = line.chars().take(PREVIEW_LINE_CHARS).collect();
        s.push('…');
        s
    }
    let mut out: Vec<String> = Vec::new();
    let mut side: Vec<String> = Vec::new();
    let mut inside = false;
    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        if !inside {
            if line.starts_with("<<<<<<<") {
                inside = true;
                out.push("<<<<<<< 新基线（SVN）".to_string());
            }
            continue;
        }
        if line.starts_with("|||||||") {
            flush(&mut out, &mut side);
            out.push("||||||| 共同的旧版本".to_string());
        } else if line == "=======" {
            flush(&mut out, &mut side);
            out.push("=======".to_string());
        } else if line.starts_with(">>>>>>>") {
            flush(&mut out, &mut side);
            out.push(">>>>>>> 你的提交".to_string());
            return Some(out.join("\n"));
        } else {
            side.push(clip(line));
        }
    }
    if !inside {
        return None;
    }
    // 没有收尾标记（文件被截断之类）：有多少给多少
    flush(&mut out, &mut side);
    Some(out.join("\n"))
}

/// 对象层跟进重放后，分支 tip 与磁盘对不上的文件（方案 §4；PR 119 遗留）。
///
/// 只看本次外部变更改过的路径（`external`，`scope..NEW`）里 tip 与 NEW 不同的。磁盘等于 NEW 的
/// 版本，说明 SVN 对这个文件没有本地修改（磁盘 = BASE = NEW），我的 commit 里对它的改动要么已经在
/// SVN 上、要么不在磁盘上了，正确的 tip 应当等于 NEW；tip 却是别的内容，就是合错了。tip 有、NEW 与
/// 磁盘都没有的，是被带回来的 SVN 已删文件。磁盘是别的内容（我还没交给 SVN 的改动）时判断不了，
/// 不报。符号链接、子模块不判。磁盘内容按 `git add` 的过滤规则算（同 [`stale_disk_paths`]）。
fn drift_paths(
    repo: &str,
    new: &str,
    tip: &str,
    external: &BTreeSet<String>,
) -> Result<Vec<String>> {
    if new == tip || external.is_empty() {
        return Ok(Vec::new());
    }
    // raw 格式：`:<旧 mode> <新 mode> <旧 oid> <新 oid> <状态>` NUL `<路径>` NUL；旧 = NEW，新 = tip
    let raw = run_git(repo, &["diff-tree", "-r", "-z", "--no-renames", new, tip])?;
    let mut drift = Vec::new();
    let mut to_hash: Vec<(String, String)> = Vec::new();
    let mut fields = raw.split('\0').filter(|f| !f.is_empty());
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        if !external.contains(path) {
            continue;
        }
        let meta: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        let (Some(mode), Some(new_oid), Some(tip_oid), Some(status)) =
            (meta.first(), meta.get(2), meta.get(3), meta.get(4))
        else {
            continue;
        };
        // 只差 mode（filemode=false 下提交过 `--chmod=+x`）：内容没对不上，不报（review 第 5 条）
        if new_oid == tip_oid {
            continue;
        }
        let on_disk = Path::new(repo).join(path).symlink_metadata();
        if status.starts_with('A') {
            // NEW 里没有、tip 里有：磁盘上也没有就是被带回来的
            if on_disk.is_err() {
                drift.push(path.to_string());
            }
        } else if (*mode == "100644" || *mode == "100755")
            && on_disk.map(|m| m.is_file()).unwrap_or(false)
        {
            to_hash.push((path.to_string(), new_oid.to_string()));
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
        for ((p, new_oid), d) in to_hash.iter().zip(disk) {
            if d == new_oid {
                drift.push(p.clone());
            }
        }
    }
    drift.sort();
    Ok(drift)
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
        /// merge-tree 的冲突树，见 [`ReplayOutcome::Conflict`]。
        tree: String,
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
        ReplayOutcome::Conflict { at, files, tree } => Replayed::Conflict { at, files, tree },
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
    for branch in crate::engine::refs::branches(&git, Some(BranchType::Local))? {
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
    /// F2 ①。6.10 的快照规则按各分支自己的基线算，见 `snapshot`。
    superseded: &'a dyn Fn(&ReplayCommit) -> BTreeSet<String>,
    snapshot: &'a SnapshotRule<'a>,
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
            "分支上的 Local-Only 补丁里有改动已经或可能出现在 SVN 上，不自动移动".to_string(),
        ));
    }
    let sup = |c: &ReplayCommit| -> BTreeSet<String> {
        let mut p = (ctx.superseded)(c);
        p.extend(ctx.snapshot.paths(&own, c));
        p
    };
    let replayed = replay_branch(repo, &own, &b.oid, ctx.new_base, &commits, &sup, memo);
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
            let (snap, own_rewritten) =
                split_snapshot(rewritten, &commits, ctx.superseded, ctx.snapshot, &own);
            for r in own_rewritten {
                if !report.rewritten.contains(&r) {
                    report.rewritten.push(r);
                }
            }
            for r in snap {
                if !report.snapshot.contains(&r) {
                    report.snapshot.push(r);
                }
            }
            Ok(Ok(tip))
        }
        Ok(Replayed::Conflict { at, files, .. }) => Ok(Err(format!(
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
    let first = if old_base == new_base {
        FollowStep::PrepareIndex
    } else {
        FollowStep::Baseline
    };
    let mut j = publishing_journal(
        opid,
        Moves {
            branch: &branch,
            old_head: &old_head,
            old_base: &old_base,
            new_base: &new_base,
            tip: tip.clone(),
        },
        others,
        report,
        first,
    );
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
            Target::Conflict(m) => needs_manual(&m),
            Target::Tip(_) => unreachable!("tip 取自 target"),
        });
    };

    // ② 拼索引
    advance(repo, &mut j, FollowStep::PrepareIndex)?;
    fault::reach(fault::Point::BeforePrepare, None);
    let heads = (old_head.as_str(), tip.as_str());
    let prepared = match compose_index(repo, opid, &mut j, &who, heads, FollowStep::MoveBranch) {
        Ok(p) => p,
        Err(e) => {
            svn_journal::remove(repo)?;
            return Err(baseline_kept(e));
        }
    };
    swap_in(repo, &mut j, prepared, &branch, &old_head, &tip)
}

/// 发布要动的几个 ref 的旧值与新值，[`publishing_journal`] 的参数。
struct Moves<'a> {
    branch: &'a str,
    old_head: &'a str,
    old_base: &'a str,
    new_base: &'a str,
    tip: Option<String>,
}

/// 发布用的 journal，步骤从 `first` 开始。
fn publishing_journal(
    opid: &str,
    m: Moves<'_>,
    others: Vec<BranchMove>,
    report: FollowReport,
    first: FollowStep,
) -> Journal {
    Journal {
        opid: opid.to_string(),
        flow: Flow::Follow,
        phase: Phase::FollowPublishing,
        started_at: now_secs(),
        orig_branch: Some(m.branch.to_string()),
        orig_head: Some(m.old_head.to_string()),
        baseline_oid_before: Some(m.old_base.to_string()),
        new_baseline_oid: Some(m.new_base.to_string()),
        follow_steps: Some(FollowSteps {
            step: first,
            new_tip: m.tip,
            others,
            index_blob: None,
            index_stages: None,
            lock_created: false,
            report,
        }),
        ..Default::default()
    }
}

/// 拼索引（OLD → NEW_TIP），留在 `.git/index.lock` 里，并把指纹记进 journal、步骤推进到 `next`
/// （见 [`record_prepared`]）。失败时锁已删，journal 由调用方处理。
fn compose_index(
    repo: &str,
    opid: &str,
    j: &mut Journal,
    who: &Who,
    (old_head, tip): (&str, &str),
    next: FollowStep,
) -> Result<PreparedIndex> {
    // 暂存 blob 要在持索引锁之后读（facts 只在 prepare_index 持锁时调用），否则外部一次 add
    // 能让这里读到的与拼索引看到的不是同一份。交集为空时一次都不用读
    let staged = OnceCell::new();
    let facts = |p: &str| who.facts(p, staged.get_or_init(|| staged_blobs(repo)).as_ref());
    // 建好锁立刻记下来：恢复时只有这个记录能证明路径上的锁是我们的（Review 2 第 7 条）
    let mut on_locked = || -> Result<()> {
        steps_mut(j)?.lock_created = true;
        svn_journal::write(repo, j)?;
        fault::reach(fault::Point::IndexLocked, None);
        Ok(())
    };
    let mut prepared = prepare_index_with(repo, opid, old_head, tip, &facts, &mut on_locked)?;
    if let Err(e) = record_prepared(repo, j, &prepared, next) {
        prepared.abandon();
        return Err(e);
    }
    fault::reach(fault::Point::IndexPrepared, Some(&mut prepared));
    Ok(prepared)
}

/// ③–⑤：核对 HEAD → CAS 当前分支 → 再核对 HEAD → 换入索引 → 其他分支与 F6。基线此时已发布。
fn swap_in(
    repo: &str,
    j: &mut Journal,
    mut prepared: PreparedIndex,
    branch: &str,
    old_head: &str,
    tip: &str,
) -> Result<FollowOutcome> {
    // ③ 核对 HEAD → CAS 当前分支 → 再核对 HEAD
    if !head_on(repo, branch) {
        return Err(back_out(repo, j, prepared, false, head_moved(branch)));
    }
    fault::reach(fault::Point::HeadChecked, Some(&mut prepared));
    if let Err(e) = move_branch(repo, branch, tip, old_head) {
        return Err(back_out(repo, j, prepared, false, branch_moved(branch, &e)));
    }
    fault::reach(fault::Point::BranchMoved, Some(&mut prepared));
    // checkout -b 绕得过 index.lock（W23）。落在这里时新分支建在 NEW_TIP 上，拼好的索引对得上它，
    // 往前走；只有 HEAD 不在 NEW_TIP 上（CAS 之前切走的、symbolic-ref 指到别处的），换入才是错的
    if !head_at(repo, tip) {
        return Err(back_out(repo, j, prepared, true, head_moved(branch)));
    }

    // ④ 换入索引
    if let Err(e) = advance(repo, j, FollowStep::SwapIndex) {
        return Err(back_out(repo, j, prepared, true, e));
    }
    fault::reach(fault::Point::BeforeSwap, Some(&mut prepared));
    if let Err(e) = prepared.commit() {
        return Err(back_out(repo, j, prepared, true, e));
    }
    fault::reach(fault::Point::IndexSwapped, None);

    // ⑤ 其他分支 + F6
    advance(repo, j, FollowStep::MoveOthers)?;
    finish(repo, j)
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

/// 记下拼好的索引的指纹与暂存报告，步骤推进到 `next`。之后崩溃时靠指纹认锁文件，报告也不会丢。
///
/// `next` 是 [`FollowStep::MoveBranch`] 的前提是基线已经发布：恢复走到这一步会把分支往前移。
/// 自动跟进在这里还没 CAS 基线，所以留在 [`FollowStep::PrepareIndex`]，发布基线之后再推进。
fn record_prepared(
    repo: &str,
    j: &mut Journal,
    prepared: &PreparedIndex,
    next: FollowStep,
) -> Result<()> {
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
    s.step = next;
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
    // 跟进完成：读收件箱之前到的事件都已反映在新基线里（C1h）。之后才到的留给下一次
    if let Some(cutoff) = report.evidence_cutoff_ms {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        prune_inbox(repo, &|e| e.at_ms <= cutoff, now);
    }
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
            let s = steps(j)?;
            // 自动跟进拼好索引之后、发布基线之前也停在这一步，这时 journal 里已有指纹：拿它核对，
            // 锁要是被换成了别人的（残锁被手工删掉、又有 git 拿了锁），不删（review 低-3）。
            // 没有指纹（手动跟进拼到一半）时照旧：记过「锁是我方建的」就只能是我们的半成品
            let lock = git_dir(repo)?.join("index.lock");
            let ours = match &s.index_blob {
                Some(_) if lock.exists() => lock_is_ours(repo, &lock, s.index_blob.as_deref())?,
                _ => s.lock_created,
            };
            if ours {
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
    record_prepared(repo, j, &prepared, FollowStep::MoveBranch)?;
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
    use crate::engine::tsvn_hooks::HookHealth;
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
        static HOOK_HEALTH: RefCell<Option<HookHealth>> = const { RefCell::new(None) };
    }

    /// 本线程上跟进读到的钩子健康状态换成 `h`：测试不读写注册表。
    pub fn pretend_hook_health(h: HookHealth) -> Armed {
        HOOK_HEALTH.with(|g| *g.borrow_mut() = Some(h));
        Armed(())
    }

    pub(crate) fn pretended_hook_health() -> Option<HookHealth> {
        HOOK_HEALTH.with(|g| g.borrow().clone())
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
            HOOK_HEALTH.with(|g| *g.borrow_mut() = None);
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
