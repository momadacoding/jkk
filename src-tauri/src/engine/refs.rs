use crate::error::Result;
use git2::Repository;
use serde::Serialize;

#[derive(Serialize)]
pub struct RefPointer {
    pub name: String,
    pub hash: String,
    pub kind: String, // "head", "branch", "remote", "tag"
}

pub fn collect(repo: &Repository) -> Result<Vec<RefPointer>> {
    let mut refs = Vec::new();
    for reference in repo.references()? {
        let reference = reference?;
        let name = reference.name().unwrap_or("").to_string();
        let kind = if name == "HEAD" {
            "head"
        } else if name.starts_with("refs/heads/") {
            "branch"
        } else if name.starts_with("refs/remotes/") {
            "remote"
        } else if name.starts_with("refs/tags/") {
            "tag"
        } else {
            "other"
        };
        if let Some(oid) = reference.target() {
            refs.push(RefPointer {
                name: name.clone(),
                hash: oid.to_string(),
                kind: kind.to_string(),
            });
        }
    }
    Ok(refs)
}
