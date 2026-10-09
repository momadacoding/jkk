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

use crate::engine::git_cli::{run_git, run_git_code, run_git_env};
use crate::engine::graph::parse_local_only;
use crate::error::{GitError, Result};
use std::collections::HashMap;

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

struct Commit {
    oid: String,
    parents: Vec<String>,
    subject: String,
    local_only: bool,
}

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
    let list = commits_between(repo, base, head)?;
    let by_oid: HashMap<&str, &Commit> = list.iter().map(|c| (c.oid.as_str(), c)).collect();
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
        let picks: Vec<&Commit> = list.iter().filter(|c| !c.local_only).collect();
        let locals: Vec<&Commit> = list.iter().filter(|c| c.local_only).collect();
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

/// `base..head`，从旧到新。`%B` 是完整 message，Local-Only 判定与图上的角标同一份实现。
fn commits_between(repo: &str, base: &str, head: &str) -> Result<Vec<Commit>> {
    let out = run_git(
        repo,
        &[
            "log",
            "--reverse",
            "--topo-order",
            "--format=%H%x00%P%x00%s%x00%B%x1e",
            &format!("{base}..{head}"),
        ],
    )?;
    let mut list = Vec::new();
    for rec in out.split('\u{1e}') {
        let rec = rec.trim_start_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let mut f = rec.splitn(4, '\0');
        let (Some(oid), Some(parents), Some(subject), Some(body)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            return Err(GitError::new(
                "sync_out_replay_failed",
                format!("读不懂 git log 的输出：{rec:?}"),
            ));
        };
        list.push(Commit {
            oid: oid.to_string(),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            subject: subject.to_string(),
            local_only: parse_local_only(body),
        });
    }
    Ok(list)
}

/// 慢路径：在对象层逐个重放待交付 commit（计划 B2）。
fn replay(repo: &str, onto: &str, picks: &[&Commit], locals: &[&Commit]) -> Result<String> {
    let mut cur = onto.to_string();
    for c in picks {
        let parent = c.parents.first().ok_or_else(|| {
            GitError::new("sync_out_replay_failed", format!("{} 没有父提交", c.oid))
        })?;
        let mb = format!("--merge-base={parent}");
        // `-z`：不带它 git 按 core.quotePath（默认 true）把非 ASCII 路径转义成八进制，
        // 冲突文件名就对不上 diff-tree -z 的原样路径（里程碑 review）
        let (code, out) = run_git_code(
            repo,
            &["merge-tree", "--write-tree", "-z", &mb, &cur, &c.oid],
        )?;
        match code {
            0 => {}
            1 => return Err(replay_conflict(repo, c, locals, &out)?),
            // 129 = 用法错误：多半是 git 太旧，不认识 --merge-base
            129 => {
                return Err(GitError::new(
                    "sync_out_git_too_old",
                    "本地补丁夹在待交付的提交中间，摘掉它们需要 git 2.40 或更新的版本\
                     （`git merge-tree --merge-base`）。请升级 git，或把本地补丁移到分支顶端\
                     （那样不需要重放）。",
                ))
            }
            _ => {
                return Err(GitError::new(
                    "sync_out_replay_failed",
                    format!(
                        "重放 {} 时 git merge-tree 退出 {code}：{}",
                        short(&c.oid),
                        out.trim()
                    ),
                ))
            }
        }
        let tree = out.split('\0').next().unwrap_or("").trim().to_string();
        cur = commit_like(repo, &tree, &cur, &c.oid)?;
    }
    Ok(cur)
}

/// 冲突：指出是哪个待交付 commit、哪些文件、可能是哪几个本地补丁（spec §13「D0.5 cherry-pick 冲突」）。
fn replay_conflict(repo: &str, c: &Commit, locals: &[&Commit], out: &str) -> Result<GitError> {
    // `-z` 形态：`<tree>\0`，随后每条 `<mode> <oid> <stage>\t<path>\0`，一个空字段之后是说明文字
    let mut files: Vec<String> = out
        .split('\0')
        .skip(1)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t').map(|(_, p)| p.to_string()))
        .collect();
    files.sort();
    files.dedup();
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
        "sync_out_local_only_conflict",
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

/// 用 `like` 的作者与 message 在 `parent` 上创建 `tree` 的 commit（计划 B15）。
fn commit_like(repo: &str, tree: &str, parent: &str, like: &str) -> Result<String> {
    let meta = run_git(
        repo,
        &[
            "log",
            "-1",
            "--date=raw",
            "--format=%an%x00%ae%x00%ad%x00%B",
            like,
        ],
    )?;
    let mut f = meta.splitn(4, '\0');
    let (name, email, date, msg) = (
        f.next().unwrap_or(""),
        f.next().unwrap_or(""),
        f.next().unwrap_or(""),
        f.next().unwrap_or("").trim_end_matches('\n'),
    );
    // message 走 -m：run_git* 的子进程 stdin 是 null，走 stdin 会产出空 message
    // （svn_baseline::create_baseline_commit 记过这个坑）
    Ok(run_git_env(
        repo,
        &["commit-tree", tree, "-p", parent, "-m", msg],
        &[
            ("GIT_AUTHOR_NAME", name),
            ("GIT_AUTHOR_EMAIL", email),
            ("GIT_AUTHOR_DATE", date),
        ],
    )?
    .trim()
    .to_string())
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

fn local_only_merge(c: &Commit) -> GitError {
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

fn merge_in_slow_path(m: &Commit) -> GitError {
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
