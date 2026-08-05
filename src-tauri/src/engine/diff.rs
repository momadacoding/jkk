use crate::error::Result;
use git2::{DiffOptions, Repository};
use serde::Serialize;
use std::collections::HashMap;

#[derive(Serialize, Debug, Clone)]
pub struct FileChange {
    pub path: String,
    pub status: String, // "modified", "added", "deleted", "renamed", "conflicted"
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

/// 工作区 diff 的通用选项：未跟踪文件也要出现在 diff 里，
/// 否则新增文件在界面上永远显示 "No changes"。
fn workdir_diff_opts() -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.context_lines(3)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    opts
}

/// 逐文件统计 diff 的增删行数。
fn line_stats(diff: &git2::Diff) -> Result<HashMap<String, (usize, usize)>> {
    let mut stats: HashMap<String, (usize, usize)> = HashMap::new();
    diff.foreach(
        &mut |_delta, _progress| true,
        None,
        Some(&mut |_delta, _hunk| true),
        Some(&mut |delta, _hunk, line| {
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            let entry = stats.entry(path).or_insert((0, 0));
            match line.origin() {
                '+' => entry.0 += 1,
                '-' => entry.1 += 1,
                _ => {}
            }
            true
        }),
    )?;
    Ok(stats)
}

/// index 相对 HEAD 的状态（已暂存那一侧）。
fn index_status(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::INDEX_NEW) {
        "added"
    } else if status.contains(git2::Status::INDEX_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::INDEX_RENAMED) {
        "renamed"
    } else {
        "modified"
    }
}

/// 工作区相对 index 的状态（未暂存那一侧）。
fn worktree_status(status: git2::Status) -> &'static str {
    if status.contains(git2::Status::WT_NEW) {
        "added"
    } else if status.contains(git2::Status::WT_DELETED) {
        "deleted"
    } else if status.contains(git2::Status::WT_RENAMED) {
        "renamed"
    } else {
        "modified"
    }
}

pub fn read_status(path: &str) -> Result<StatusData> {
    let repo = Repository::open(path)?;

    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let staged_stats = line_stats(&repo.diff_tree_to_index(head_tree.as_ref(), None, None)?)?;
    let unstaged_stats = line_stats(
        &repo.diff_index_to_workdir(None, Some(&mut workdir_diff_opts()))?,
    )?;

    let mut status_opts = git2::StatusOptions::new();
    status_opts
        .include_untracked(true)
        .recurse_untracked_dirs(true);
    let statuses = repo.statuses(Some(&mut status_opts))?;

    let mut staged = Vec::new();
    let mut unstaged = Vec::new();

    let make = |path_str: &str, kind: &str, stats: &HashMap<String, (usize, usize)>| {
        let (additions, deletions) = stats.get(path_str).copied().unwrap_or((0, 0));
        FileChange {
            path: path_str.to_string(),
            status: kind.to_string(),
            additions,
            deletions,
        }
    };

    for entry in statuses.iter() {
        let status = entry.status();
        let path_str = entry.path().unwrap_or("");

        // 冲突文件既不在 INDEX_* 也不在 WT_* 里，单独处理，
        // 否则 merge 冲突时界面上根本看不到冲突文件。
        if status.contains(git2::Status::CONFLICTED) {
            unstaged.push(make(path_str, "conflicted", &unstaged_stats));
            continue;
        }

        if status.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED,
        ) {
            staged.push(make(path_str, index_status(status), &staged_stats));
        }
        if status.intersects(
            git2::Status::WT_NEW
                | git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED,
        ) {
            unstaged.push(make(path_str, worktree_status(status), &unstaged_stats));
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
    // default: workdir vs HEAD（空仓库没有 HEAD，此时与空树比较）
    let tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let diff = repo.diff_tree_to_workdir_with_index(
        tree.as_ref(),
        Some(&mut workdir_diff_opts()),
    )?;
    diff_to_data(diff)
}

fn diff_to_data(diff: git2::Diff) -> Result<DiffData> {
    let mut files: Vec<FileDiff> = Vec::new();
    diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        // libgit2 会把文件头（'F'）、hunk 头（'H'）、二进制标记（'B'）
        // 也作为行回调抛出来，直接渲染就会出现 "F diff --git ..." 这种脏行。
        if !matches!(line.origin(), '+' | '-' | ' ') {
            return true;
        }
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
