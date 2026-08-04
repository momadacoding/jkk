use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize)]
pub struct RepoInfo {
    pub path: String,
    pub head: String,
}

pub fn open(path: &str) -> Result<RepoInfo> {
    let repo = Repository::open(path).map_err(|_| {
        GitError::new("not_repo", format!("{} is not a git repository", path))
    })?;
    let head = repo.head()?;
    let head_name = head.shorthand().unwrap_or("HEAD").to_string();
    Ok(RepoInfo {
        path: path.to_string(),
        head: head_name,
    })
}
