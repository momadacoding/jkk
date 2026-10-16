use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitNode {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    /// **只有标题**（`commit.summary()`，第一段且空白已折叠）。
    ///
    /// 提交列表一行一条，正文放不下也没人看。要正文里的东西请走下面两个字段，
    /// 不要指望从这里解析——`svn_graph_contract.rs` 把这条约定钉成了测试。
    pub message: String,
    /// 这个 commit 的 `git-svn-id` trailer 记的 revision（spec §6.4.1）；
    /// 不是基线就是 `None`。
    ///
    /// # 为什么解析放在这里，而不是前端
    ///
    /// trailer 写在 message 的**正文**里，而上面的 `message` 是 `summary()`，
    /// 只有第一段。前端拿到的数据里根本没有这一行，在那边解析必然恒为 None
    /// ——2026-09-16 独立 review 发现的就是这个，整个混合图接合静默不生效。
    ///
    /// 解析用 `svn_baseline::parse_trailer`，与录制基线的是同一份实现，
    /// 不存在两侧规则漂移的问题。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline_rev: Option<u64>,
    /// 交付录的基线额外记下的「真正提交出去的 revision」（`Delivered-Rev:`）。
    ///
    /// `baseline_rev` 是交付之后更新到的 revision：中间同事紧跟着提交时它是同事的，图上收起的
    /// 本地提交按它归属就会挂到同事名下（2026-10-16 review）。初始化、更新录的基线没有这一项。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered_rev: Option<u64>,
    /// `Local-Only: true`（spec §6.7）：用户勾了「仅本地，不交付」。
    /// 同样只能从正文里看出来。
    #[serde(skip_serializing_if = "is_false")]
    pub local_only: bool,
}

/// `skip_serializing_if` 用。普通 git 仓库的每一条提交都会带上这两个字段，
/// 5000 条 × 两个恒定值是白给的传输量。
fn is_false(b: &bool) -> bool {
    !*b
}

/// `Local-Only: true`（spec §6.7）。键按 git 的惯例大小写不敏感，
/// 且**逐行完整匹配**——正文里顺口提到这个词的提交不算。
/// `svn_deliver` 用同一份判定摘掉本地补丁——两处规则不能漂移。
pub(crate) fn parse_local_only(message: &str) -> bool {
    message.lines().any(|line| {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else {
            return false;
        };
        key.eq_ignore_ascii_case("local-only") && value.trim().eq_ignore_ascii_case("true")
    })
}

#[derive(Serialize)]
pub struct GraphData {
    pub commits: Vec<CommitNode>,
    pub refs: Vec<crate::engine::refs::RefPointer>,
}

pub fn load(path: &str) -> Result<GraphData> {
    let repo = Repository::open(path)?;
    // 先收 refs：下面 revwalk 要用里面已经 peel 好的 tag 目标做起点，
    // 在这里重新 peel 一遍等于把 refs.rs 的逻辑抄第二份。
    let refs = crate::engine::refs::collect(&repo)?;
    let mut walk = repo.revwalk()?;
    // TOPOLOGICAL 只保证「子在父之前」，互不相关的两个提交之间顺序是任意的，
    // 取决于下面 push 的顺序。加上 TIME 才是 `git log --date-order` 的语义：
    // 在不破坏拓扑的前提下按提交时间从新到旧。
    //
    // 少了 TIME 会有具体的视觉后果：一个比合并提交更新的未合并分支 tip 会掉到
    // 合并提交下面，而合并连向第二父的线正好经过该 tip 所在的车道，
    // 于是那条线从它的圆点上穿过去，看上去它不是分支 tip 而是合并线的中转点。
    walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
    walk.push_head()?;
    for branch in repo.branches(None)? {
        let (branch, _) = branch?;
        if let Some(oid) = branch.get().target() {
            walk.push(oid)?;
        }
    }
    // tag 也当起点：一条提交可能只被 tag 指着（分支已经挪走），不 push 的话它不进图，
    // 侧边栏点这个 tag 就会定位失败。
    //
    // 只取 tag，不顺手把 other 也 push 进来——refs/stash、refs/notes 会把一堆
    // 与提交历史无关的东西拉进图里。
    //
    // push 失败就跳过：一条悬空或损坏的 tag 不该让整张图加载失败。
    for r in refs.iter().filter(|r| r.kind == "tag") {
        if let Ok(oid) = git2::Oid::from_str(&r.hash) {
            let _ = walk.push(oid);
        }
    }

    let mut commits = Vec::new();
    let limit = 5000;
    for oid in walk {
        let oid = oid?;
        let commit = repo.find_commit(oid)?;
        // 正文只在这里用一次：解析完就丢，不进 payload。
        let full = commit.message().unwrap_or("");
        commits.push(CommitNode {
            id: oid.to_string(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            message: commit.summary().unwrap_or("").to_string(),
            baseline_rev: crate::engine::svn_baseline::parse_trailer(full).map(|t| t.rev),
            delivered_rev: crate::engine::svn_baseline::parse_delivered_rev(full),
            local_only: parse_local_only(full),
        });
        if commits.len() >= limit {
            break;
        }
    }

    if commits.is_empty() {
        return Err(GitError::new(
            "empty_repo",
            "This repository has no commits yet",
        ));
    }

    Ok(GraphData { commits, refs })
}
