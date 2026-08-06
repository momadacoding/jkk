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
///
/// `only` 非空时用 pathspec 把范围限定到这些路径，用来绕开读不了的坏路径。
fn workdir_diff_opts(only: Option<&[String]>) -> DiffOptions {
    let mut opts = DiffOptions::new();
    opts.context_lines(3)
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true);
    if let Some(paths) = only {
        for p in paths {
            opts.pathspec(p);
        }
    }
    opts
}

/// 挑出工作区里**能读**的路径。
///
/// 悬空的符号链接/目录联接、权限不足、扫描途中被删掉的文件，libgit2 在加载内容
/// 时会报 "could not stat" 并中止**整个** diff。光是「保留已收集的部分」不够用：
/// 坏路径若排在前面（`.pnpm-store/...` 这种点开头的路径几乎总是排最前），
/// 就一个文件都拿不到。所以要先把坏路径挑出来，用 pathspec 排除掉再重新构造。
///
/// 构造 diff 本身只看目录列表、不读内容，所以这一趟是廉价的。
/// 返回 `None` 表示全都能读，按原样走快路径，不付任何额外代价。
fn readable_only(repo: &Repository, diff: &git2::Diff) -> Option<Vec<String>> {
    let workdir = repo.workdir()?;
    let mut good = Vec::new();
    let mut found_bad = false;

    for delta in diff.deltas() {
        let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
            continue;
        };
        let as_str = path.to_string_lossy().to_string();
        // 已删除的文件本来就不在工作区，不算坏路径
        if delta.status() == git2::Delta::Deleted {
            good.push(as_str);
            continue;
        }
        // metadata 会跟随链接，正好能识别出悬空链接
        if std::fs::metadata(workdir.join(path)).is_ok() {
            good.push(as_str);
        } else {
            found_bad = true;
            log::warn!("工作区路径读不了，已从 diff 中跳过：{as_str}");
        }
    }

    if found_bad {
        Some(good)
    } else {
        None
    }
}

/// 逐文件统计 diff 的增删行数。
///
/// **不会因为个别坏路径而整体失败。** 构造 diff 只需要目录列表，而这里的行回调
/// 才真正去打开文件；工作区里的悬空符号链接/联接、权限不足、扫描途中被删掉的
/// 文件，都会让 libgit2 报 "could not stat" 并中止整个遍历。原生 git 遇到同样的
/// 情况只打一条 warning 就继续（实测 `warning: could not open directory ...`），
/// 这里对齐它：已经统计到的照常返回，其余退化为 0 行——文件本身仍然出现在列表里。
/// 一个坏路径不该把整个变更视图弄瞎。
fn line_stats(diff: &git2::Diff) -> HashMap<String, (usize, usize)> {
    let mut stats: HashMap<String, (usize, usize)> = HashMap::new();
    let outcome = diff.foreach(
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
    );
    if let Err(e) = outcome {
        log::warn!(
            "diff 行数统计中断，已保留 {} 个文件的结果：{}",
            stats.len(),
            e.message()
        );
    }
    stats
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
    let staged_stats = line_stats(&repo.diff_tree_to_index(head_tree.as_ref(), None, None)?);

    let workdir_diff = repo.diff_index_to_workdir(None, Some(&mut workdir_diff_opts(None)))?;
    let unstaged_stats = match readable_only(&repo, &workdir_diff) {
        None => line_stats(&workdir_diff),
        // 全都读不了就没什么可统计的，行数一律退化为 0，
        // 但文件列表来自 statuses()，不受影响
        Some(good) if good.is_empty() => HashMap::new(),
        Some(good) => line_stats(
            &repo.diff_index_to_workdir(None, Some(&mut workdir_diff_opts(Some(&good))))?,
        ),
    };

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
        Some(&mut workdir_diff_opts(None)),
    )?;
    match readable_only(&repo, &diff) {
        None => diff_to_data(diff),
        // 一个能读的都没有：返回空 diff 而不是让整个视图报错。
        // 注意此时不能拿空的 pathspec 去重建——那等于不加限制，又会撞上坏路径。
        Some(good) if good.is_empty() => Ok(DiffData { files: Vec::new() }),
        Some(good) => diff_to_data(repo.diff_tree_to_workdir_with_index(
            tree.as_ref(),
            Some(&mut workdir_diff_opts(Some(&good))),
        )?),
    }
}

/// 把 diff 摊平成前端要的结构。
///
/// 与 `line_stats` 同理：`print` 会去打开工作区文件，个别坏路径（悬空链接、
/// 权限不足、扫描途中被删）会让 libgit2 中止整个输出。这里保留已经收集到的
/// 文件而不是整体报错——看得到大部分 diff，好过什么都看不到。
fn diff_to_data(diff: git2::Diff) -> Result<DiffData> {
    let mut files: Vec<FileDiff> = Vec::new();
    let outcome = diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
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
    });
    if let Err(e) = outcome {
        log::warn!(
            "diff 输出中断，已保留 {} 个文件的结果：{}",
            files.len(),
            e.message()
        );
    }
    Ok(DiffData { files })
}
