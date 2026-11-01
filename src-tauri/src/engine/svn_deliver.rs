//! D0.5：从 HEAD 摘掉 Local-Only commit，构造交付树（spec §8.2.3；计划 B2）。
//!
//! # 为什么在对象层重放，而不是临时 worktree + cherry-pick
//!
//! spec 的两条硬约束是「D0.5 不动主工作副本」与「D0.5 之后的早退必须清干净」。
//! `git merge-tree --write-tree --merge-base=<C^> <cur> <C>` 与 cherry-pick 用同一套
//! 合并引擎（ort），但只产出对象：磁盘、索引、HEAD、worktree 注册一样都不碰，
//! 于是两条约束天然成立——没有东西需要清理。计划开工前实测 M1：不重叠时两种做法
//! 得到的 tree 相同；重叠时 merge-tree 退出 1，并列出冲突文件的三个 stage。
//!
//! 这也是预览能把交付树算给用户看的前提（计划 B1）：预览不拿锁、不写 journal，
//! 所以它不能留下任何需要事后清理的东西。
//!
//! 代价：`--merge-base` 要 git ≥ 2.40。只在慢路径上用到，旧 git 报 `sync_out_git_too_old`。
//!
//! 重放本身在 [`crate::engine::svn_replay`]（与跟进共用）；交付不丢空 commit、不换路径，
//! 错误码与文案由本文件的 `DELIVER_CODES` 给出。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_replay::{
    commits_between, no_superseded, replay_onto, ReplayCodes, ReplayCommit, ReplayOpts,
    ReplayOutcome,
};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, HashMap};

/// 交付树 ref 的前缀。不在 `refs/heads/` 下，与用户分支不可能撞名（spec §8.2.3、必测 #25）。
pub const DELIVER_REF_PREFIX: &str = "refs/mygitclient/deliver/";

/// 被摘掉的本地补丁。确认框必须列出它们（spec §8.2.3「漏标是主要人为风险」）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOnlyCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverPlan {
    /// 交付树所在的 commit。快路径是一个已有 commit，慢路径是重放出来的新 commit
    /// （只是对象，没有任何 ref 指着它，直到执行时 [`pin`]）。
    pub deliver_oid: String,
    pub deliver_tree: String,
    pub local_only: Vec<LocalOnlyCommit>,
    /// 全部 Local-Only commit 构成分支顶端连续的单父后缀。
    pub fast: bool,
}

/// 交付的三类错误码与 git 太旧时的文案（对象层重放在 `svn_replay`）。
const DELIVER_CODES: ReplayCodes = ReplayCodes {
    conflict: "sync_out_local_only_conflict",
    too_old: "sync_out_git_too_old",
    too_old_message: "本地补丁夹在待交付的提交中间，摘掉它们需要 git 2.40 或更新的版本\
                      （`git merge-tree --merge-base`）。请升级 git，或把本地补丁移到分支顶端\
                      （那样不需要重放）。",
    failed: "sync_out_replay_failed",
};

pub fn deliver_ref(opid: &str) -> String {
    format!("{DELIVER_REF_PREFIX}{opid}")
}

/// 算出交付树。**不写任何 ref、不动磁盘**——预览与执行都调它。
///
/// 包含 §8.1 第 6、7 条：慢路径上历史里不许有 merge；快路径的 Local-Only 后缀必须单父。
pub fn plan_deliver(repo: &str, base: &str, head: &str) -> Result<DeliverPlan> {
    for oid in [base, head] {
        // 空 OID 会让区间退化成 `..head`（= `HEAD..head`），查的就不是那一段了
        if oid.len() != 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GitError::new(
                "sync_out_replay_failed",
                format!("构造交付树拿到的不是一个完整 OID：`{oid}`"),
            ));
        }
    }
    let list = commits_between(repo, base, head, &DELIVER_CODES)?;
    let by_oid: HashMap<&str, &ReplayCommit> = list.iter().map(|c| (c.oid.as_str(), c)).collect();
    let local_only: Vec<LocalOnlyCommit> = list
        .iter()
        .filter(|c| c.local_only)
        .map(|c| LocalOnlyCommit {
            oid: c.oid.clone(),
            subject: c.subject.clone(),
        })
        .collect();

    // 沿第一父从 HEAD 往回走，收集顶端连续的 Local-Only 后缀
    let mut cut = head.to_string();
    let mut suffix: Vec<&str> = Vec::new();
    while let Some(c) = by_oid.get(cut.as_str()) {
        if !c.local_only {
            break;
        }
        if c.parents.len() != 1 {
            return Err(local_only_merge(c)); // §8.1 第 7 条
        }
        suffix.push(c.oid.as_str());
        cut = c.parents[0].clone();
    }
    let fast = list
        .iter()
        .all(|c| !c.local_only || suffix.contains(&c.oid.as_str()));

    let deliver_oid = if fast {
        cut // 后缀之前的那个已有 commit（全是补丁时就是 base）
    } else {
        // §8.1 第 6 条：cherry-pick 语义下 merge 没有安全的自动解（spec §8.2.3 T4 / T5）
        if let Some(m) = list.iter().find(|c| c.parents.len() > 1) {
            return Err(merge_in_slow_path(m));
        }
        let picks: Vec<ReplayCommit> = list.iter().filter(|c| !c.local_only).cloned().collect();
        let locals: Vec<&ReplayCommit> = list.iter().filter(|c| c.local_only).collect();
        replay(repo, base, &picks, &locals)?
    };
    let deliver_tree = run_git(repo, &["rev-parse", &format!("{deliver_oid}^{{tree}}")])?
        .trim()
        .to_string();
    Ok(DeliverPlan {
        deliver_oid,
        deliver_tree,
        local_only,
        fast,
    })
}

/// 慢路径：在对象层逐个重放待交付 commit（计划 B2）。交付不丢空 commit、不换路径。
fn replay(
    repo: &str,
    onto: &str,
    picks: &[ReplayCommit],
    locals: &[&ReplayCommit],
) -> Result<String> {
    let opts = ReplayOpts {
        drop_empty: false,
        superseded: &no_superseded,
        codes: DELIVER_CODES,
    };
    match replay_onto(repo, onto, picks, &opts, &mut BTreeMap::new())? {
        ReplayOutcome::Done { tip, .. } => Ok(tip),
        ReplayOutcome::Conflict { at, files } => {
            let c = picks.iter().find(|c| c.oid == at).ok_or_else(|| {
                GitError::new(
                    DELIVER_CODES.failed,
                    format!("重放报告的冲突 commit {at} 不在待交付的提交里"),
                )
            })?;
            Err(replay_conflict(repo, c, locals, &files)?)
        }
    }
}

/// 冲突：指出是哪个待交付 commit、哪些文件、可能是哪几个本地补丁（spec §13「D0.5 cherry-pick 冲突」）。
fn replay_conflict(
    repo: &str,
    c: &ReplayCommit,
    locals: &[&ReplayCommit],
    files: &[String],
) -> Result<GitError> {
    let mut blamed = Vec::new();
    for l in locals {
        let touched = run_git(
            repo,
            &[
                "diff-tree",
                "--no-commit-id",
                "--name-only",
                "-r",
                "-z",
                &l.oid,
            ],
        )?;
        if touched.split('\0').any(|p| files.iter().any(|f| f == p)) {
            blamed.push(format!("  - {} {}", short(&l.oid), l.subject));
        }
    }
    Ok(GitError::new(
        DELIVER_CODES.conflict,
        format!(
            "摘掉本地补丁之后，待交付的提交 {} {} 放不上去——它和下面的本地补丁改到了同一处：\n{}\n\
             冲突的文件：\n{}\n\
             什么都没有改动，也没有碰 SVN。请把这个补丁移到分支顶端，或者把相邻的改动拆开，再交付。",
            short(&c.oid),
            c.subject,
            if blamed.is_empty() {
                "  （没认出具体是哪个）".to_string()
            } else {
                blamed.join("\n")
            },
            files
                .iter()
                .map(|f| format!("  - {f}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
    ))
}

/// 给交付树起名字。**旧值写空串**：ref 必须还不存在，撞上残留就报错而不是覆盖。
pub fn pin(repo: &str, opid: &str, oid: &str) -> Result<String> {
    let r = deliver_ref(opid);
    run_git(repo, &["update-ref", &r, oid, ""])?;
    Ok(r)
}

/// 删交付 ref。幂等（spec §8.2.4：崩溃恢复会重跑收尾），且**只删本前缀下的**（§8.5 末）。
pub fn unpin(repo: &str, r: &str) -> Result<()> {
    let ok = r
        .strip_prefix(DELIVER_REF_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('/') && !rest.contains(".."));
    if !ok {
        return Err(GitError::new(
            "sync_out_bad_deliver_ref",
            format!("拒绝删除 `{r}`：它不是本应用建的交付 ref（必须在 {DELIVER_REF_PREFIX} 下）。"),
        ));
    }
    let (code, _) = run_git_code(repo, &["rev-parse", "-q", "--verify", r])?;
    if code != 0 {
        return Ok(()); // 已经不在了
    }
    run_git(repo, &["update-ref", "-d", r])?;
    Ok(())
}

fn local_only_merge(c: &ReplayCommit) -> GitError {
    GitError::new(
        "sync_out_local_only_merge",
        format!(
            "分支顶端的本地补丁里有一个 merge commit：{} {}\n交付之后这些补丁要重放到新基线上，\
             而 rebase 会压平 merge，只存在于 merge 结果里的改动可能无声消失。\
             请把本地补丁整理成一串普通提交，再交付。什么都没有改动。",
            short(&c.oid),
            c.subject
        ),
    )
}

fn merge_in_slow_path(m: &ReplayCommit) -> GitError {
    GitError::new(
        "sync_out_merge_in_slow_path",
        format!(
            "本地补丁夹在待交付的提交中间，摘掉它们要逐个重放待交付的提交，而历史里有一个\
             merge commit：{} {}\n重放没法安全地处理 merge。可以把本地补丁移到分支顶端\
             （那样不需要重放，merge 原样交付），或者保留原分支、人工整理成线性历史。\
             什么都没有改动。",
            short(&m.oid),
            m.subject
        ),
    )
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}
