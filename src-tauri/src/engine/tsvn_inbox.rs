//! TortoiseSVN 钩子事件的收件箱（分析 §6.13.2；计划 Task 2、Task 3）。
//!
//! 钩子进程只往这里写一条事件就退出，不连接 jkk、不读 git。jkk 跟进时再读。事件只是线索：
//! 真相仍是基线映射，读不懂的文件跳过即可。
//!
//! 事件里不记提交消息：只有路径、revision、时刻与错误文本，够判定用就行（AGENTS.md §11 的
//! 精神，用户的仓库内容不往外多带）。

use crate::error::{GitError, Result};
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const INBOX: &str = "tsvn-inbox";

/// Pre 与 Post 最多相隔多久还算同一次提交。一次提交从点 OK 到 Post-commit 实测不到 1 秒（H2），
/// 大提交也就几分钟；窗口卡得紧，是为了不让过期失败留下的孤儿 Pre（H4）和几天后的提交配上，
/// 那样 t_commit 会早得离谱（计划 Review Focus 第 3 条）。
pub const PAIR_WINDOW_MS: i64 = 600_000;
/// 没有配上 Post 的 Pre 留多久。
pub const ORPHAN_PRE_TTL_MS: i64 = 86_400_000;
/// 任何事件最多留多久。超过之后再跟进，按没有事件处理（拿不准，方向安全）。
pub const EVENT_TTL_MS: i64 = 14 * 86_400_000;
pub const MAX_EVENTS: usize = 2000;

/// 本机的一次成功提交。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitEvidence {
    /// 提交开始的时刻（配对上的 Pre 的 `at_ms`）。配不上时是 `None`，判定按拿不准处理。
    pub t_commit_ms: Option<i64>,
    pub revision: u64,
    pub paths: BTreeSet<String>,
}

/// 本机的一次更新：这些路径的变化来自服务器（同事）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateEvidence {
    pub revision: u64,
    pub paths: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookEvidence {
    pub commits: Vec<CommitEvidence>,
    pub updates: Vec<UpdateEvidence>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TsvnEvent {
    pub version: u32,
    /// `pre_commit` / `post_commit` / `post_update`。
    pub kind: String,
    /// 钩子进程启动时的本机时刻（unix 毫秒）。
    pub at_ms: i64,
    /// 仓库相对路径，正斜杠。commit 类取 `PATH`，`post_update` 取 `RESULTPATH`。
    pub paths: Vec<String>,
    pub revision: Option<u64>,
    /// `ERROR` 文件的内容；成功时为 `None`。
    pub error: Option<String>,
}

pub const EVENT_VERSION: u32 = 1;

pub fn inbox_dir(repo: &str) -> PathBuf {
    Path::new(repo).join(".git").join("mygitclient").join(INBOX)
}

/// 原子写一条事件：先写 `.tmp` 再 rename，读的一方永远看不到半个文件。
///
/// `.git` 不是目录（linked worktree、或者根本不是仓库）时报错，绝不凭空造出 `.git`
/// （与 `svn_follow_mark::write_mark` 同一理由）。
pub fn write_event(repo: &str, ev: &TsvnEvent) -> Result<PathBuf> {
    if !Path::new(repo).join(".git").is_dir() {
        return Err(GitError::new(
            "tsvn_inbox_no_git_dir",
            format!("{repo} 下没有 .git 目录"),
        ));
    }
    let dir = inbox_dir(repo);
    fs::create_dir_all(&dir)?;
    let name = format!("{}-{}-{}.json", ev.at_ms, std::process::id(), ev.kind);
    let tmp = dir.join(format!("{name}.tmp"));
    let path = dir.join(name);
    let body = serde_json::to_vec(ev)
        .map_err(|e| GitError::new("tsvn_inbox_failed", format!("事件序列化失败：{e}")))?;
    fs::write(&tmp, body)?;
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(path)
}

/// 收件箱里读得懂的事件，按 `at_ms` 升序。读不懂的跳过（`.tmp`、坏 JSON、版本不认识）。
fn read_all(repo: &str) -> Vec<(PathBuf, TsvnEvent)> {
    let Ok(rd) = fs::read_dir(inbox_dir(repo)) else {
        return Vec::new();
    };
    let mut out: Vec<(PathBuf, TsvnEvent)> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|p| {
            let bytes = fs::read(&p).ok()?;
            match serde_json::from_slice::<TsvnEvent>(&bytes) {
                Ok(ev) if ev.version == EVENT_VERSION => Some((p, ev)),
                _ => {
                    log::warn!("收件箱里有读不懂的事件，跳过：{}", p.display());
                    None
                }
            }
        })
        .collect();
    out.sort_by(|a, b| (a.1.at_ms, &a.0).cmp(&(b.1.at_ms, &b.0)));
    out
}

/// 能当提交证据的 Post：成功、有 revision。
fn is_commit_post(e: &TsvnEvent) -> bool {
    e.kind == "post_commit" && e.error.is_none() && e.revision.is_some()
}

/// 给每条 Post 找 Pre：同一路径集合、不晚于 Post、相隔不超过窗口里最晚的那条。返回 Pre 的下标。
fn pair_of(events: &[(PathBuf, TsvnEvent)], post: usize) -> Option<usize> {
    let p = &events[post].1;
    let paths: BTreeSet<&String> = p.paths.iter().collect();
    events
        .iter()
        .enumerate()
        .filter(|(_, (_, e))| {
            e.kind == "pre_commit"
                && e.at_ms <= p.at_ms
                && p.at_ms - e.at_ms <= PAIR_WINDOW_MS
                && e.paths.iter().collect::<BTreeSet<_>>() == paths
        })
        .max_by_key(|(_, (_, e))| e.at_ms)
        .map(|(i, _)| i)
}

/// 读出判定用的证据。读不了的一律跳过，不报错：没有证据只会落到「拿不准」。
pub fn read_evidence(repo: &str) -> HookEvidence {
    let events = read_all(repo);
    let mut out = HookEvidence::default();
    for (i, (_, e)) in events.iter().enumerate() {
        let Some(revision) = e.revision else {
            continue;
        };
        let paths: BTreeSet<String> = e.paths.iter().cloned().collect();
        if is_commit_post(e) {
            out.commits.push(CommitEvidence {
                t_commit_ms: pair_of(&events, i).map(|j| events[j].1.at_ms),
                revision,
                paths,
            });
        } else if e.kind == "post_update" {
            // 更新带错误时（部分失败）照样算：RESULTPATH 里的路径确实是被更新碰过的
            out.updates.push(UpdateEvidence { revision, paths });
        }
    }
    out
}

/// 清理收件箱：
/// - `processed` 认定已处理的事件删掉；被删的成功 Post 连同它配上的 Pre 一起删；
/// - 没配上的 Pre 超过 [`ORPHAN_PRE_TTL_MS`] 删；
/// - 任何事件超过 [`EVENT_TTL_MS`] 删；
/// - 剩下的超过 [`MAX_EVENTS`] 条时从最旧的删起。
///
/// 删不掉的文件留着，下次再删。
pub fn prune_inbox(repo: &str, processed: &dyn Fn(&TsvnEvent) -> bool, now_ms: i64) {
    let events = read_all(repo);
    let mut drop: HashSet<usize> = HashSet::new();
    let mut paired: HashSet<usize> = HashSet::new();
    for (i, (_, e)) in events.iter().enumerate() {
        if !is_commit_post(e) {
            continue;
        }
        if let Some(j) = pair_of(&events, i) {
            paired.insert(j);
            if processed(e) {
                drop.insert(j);
            }
        }
    }
    for (i, (_, e)) in events.iter().enumerate() {
        let age = now_ms - e.at_ms;
        let orphan_pre = e.kind == "pre_commit" && !paired.contains(&i);
        if (e.kind != "pre_commit" && processed(e))
            || age > EVENT_TTL_MS
            || (orphan_pre && age > ORPHAN_PRE_TTL_MS)
        {
            drop.insert(i);
        }
    }
    let mut keep: Vec<usize> = (0..events.len()).filter(|i| !drop.contains(i)).collect();
    if keep.len() > MAX_EVENTS {
        // events 已按 at_ms 升序，前面的最旧
        let excess = keep.len() - MAX_EVENTS;
        drop.extend(keep.drain(..excess));
    }
    for i in drop {
        if let Err(e) = fs::remove_file(&events[i].0) {
            log::warn!("收件箱事件删不掉，下次再删：{}：{e}", events[i].0.display());
        }
    }
}
