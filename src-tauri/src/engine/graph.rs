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
    let mut walk = repo.revwalk()?;
    walk.set_sorting(git2::Sort::TOPOLOGICAL)?;
    walk.push_head()?;
    for branch in repo.branches(None)? {
        let (branch, _) = branch?;
        if let Some(oid) = branch.get().target() {
            walk.push(oid)?;
        }
    }

    let mut commits = Vec::new();
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
    }

    if commits.is_empty() {
        return Err(GitError::new("empty_repo", "This repository has no commits yet"));
    }

    let refs = crate::engine::refs::collect(&repo)?;
    Ok(GraphData { commits, refs })
}
