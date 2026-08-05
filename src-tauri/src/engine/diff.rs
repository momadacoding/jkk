use crate::error::Result;
use git2::{DiffOptions, Repository};
use serde::Serialize;

#[derive(Serialize, Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub status: String, // "modified", "added", "deleted", "renamed"
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Serialize, Debug, Clone)]
pub struct StatusData {
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
}

#[derive(Serialize, Debug, Clone)]
pub struct DiffLine {
    pub origin: char, // '+', '-', ' '
    pub content: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<DiffHunk>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffData {
    pub files: Vec<FileDiff>,
}

pub fn read_status(path: &str) -> Result<StatusData> {
    let repo = Repository::open(path)?;
    let statuses = repo.statuses(None)?;
    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    for entry in statuses.iter() {
        let status = entry.status();
        let path_str = entry.path().unwrap_or("").to_string();
        let change = FileChange {
            path: path_str,
            status: if status.contains(git2::Status::INDEX_NEW) {
                "added"
            } else if status.contains(git2::Status::INDEX_DELETED) {
                "deleted"
            } else if status.contains(git2::Status::INDEX_RENAMED) {
                "renamed"
            } else {
                "modified"
            }
            .to_string(),
            additions: 0,
            deletions: 0,
        };
        if status.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED,
        ) {
            staged.push(change.clone());
        }
        if status.intersects(
            git2::Status::WT_NEW
                | git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED,
        ) {
            unstaged.push(change);
        }
    }

    Ok(StatusData { staged, unstaged })
}

pub fn read_diff(path: &str, scope: String) -> Result<DiffData> {
    let repo = Repository::open(path)?;
    if let Some(hash) = scope.strip_prefix("commit:") {
        let commit = repo.find_commit(git2::Oid::from_str(hash)?)?;
        let tree = commit.tree()?;
        let parent_tree = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        return diff_to_data(diff);
    }
    // default: workdir vs HEAD
    let head = repo.head()?;
    let tree = head.peel_to_tree()?;
    let diff = repo.diff_tree_to_workdir_with_index(
        Some(&tree),
        Some(DiffOptions::new().context_lines(3)),
    )?;
    diff_to_data(diff)
}

fn diff_to_data(diff: git2::Diff) -> Result<DiffData> {
    let mut files: Vec<FileDiff> = Vec::new();
    diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        let path = delta
            .new_file()
            .path()
            .unwrap_or(delta.old_file().path().unwrap_or(std::path::Path::new("")))
            .to_string_lossy()
            .to_string();
        let hunk_info = hunk.map(|h| (h.old_start(), h.new_start())).unwrap_or((0, 0));
        let origin = line.origin();
        let content = std::str::from_utf8(line.content())
            .unwrap_or("")
            .trim_end_matches('\n')
            .to_string();

        if files.is_empty() || files.last().unwrap().path != path {
            files.push(FileDiff {
                path,
                hunks: Vec::new(),
            });
        }
        let file = files.last_mut().unwrap();
        if file.hunks.is_empty() || file.hunks.last().unwrap().old_start != hunk_info.0 {
            file.hunks.push(DiffHunk {
                old_start: hunk_info.0,
                new_start: hunk_info.1,
                lines: Vec::new(),
            });
        }
        file.hunks
            .last_mut()
            .unwrap()
            .lines
            .push(DiffLine { origin, content });
        true
    })?;
    Ok(DiffData { files })
}
