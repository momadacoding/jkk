use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize)]
pub struct CommitNode {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    pub time: i64,
    pub message: String,
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
        commits.push(CommitNode {
            id: oid.to_string(),
            parents: commit.parent_ids().map(|id| id.to_string()).collect(),
            author: commit.author().name().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            message: commit.summary().unwrap_or("").to_string(),
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
