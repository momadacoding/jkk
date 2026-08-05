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
        // 符号引用（如 refs/remotes/origin/HEAD）的 target() 为 None，跳过
        if let Some(oid) = reference.target() {
            refs.push(RefPointer {
                name: name.clone(),
                hash: oid.to_string(),
                kind: kind.to_string(),
            });
        }
    }

    // HEAD 不在 references() 的遍历范围内（那只列 refs/*），必须单独取。
    // 少了它前端就认不出当前 commit，圆点强调和 .ref-pill.head 样式都用不上。
    if let Ok(head) = repo.head() {
        if let Some(oid) = head.resolve().ok().and_then(|r| r.target()) {
            refs.push(RefPointer {
                name: "HEAD".to_string(),
                hash: oid.to_string(),
                kind: "head".to_string(),
            });
        }
    }

    Ok(refs)
}
