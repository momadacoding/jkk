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
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub origin: char, // '+', '-', ' '
    pub content: String,
    /// 该行在旧版本里的行号；新增行为 None。行号栏靠这两个字段。
    pub old_lineno: Option<u32>,
    /// 该行在新版本里的行号；删除行为 None。
    pub new_lineno: Option<u32>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: u32,
    pub new_start: u32,
    /// git 原样的 `@@ -a,b +c,d @@ 上下文` 头，界面上直接显示。
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    /// 与 `FileChange::status` 同一套取值，文件树的状态角标靠它。
    pub status: String,
    /// 重命名前的路径；其余情况为 None。
    pub old_path: Option<String>,
    /// 二进制文件没有可显示的行，界面要换一种呈现方式。
    pub binary: bool,
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

/// 工作区里这个相对路径能不能读到。
///
/// 用来对齐原生 git：遇到读不了的目录项（悬空的符号链接/目录联接等），
/// git 只打一条 `warning: could not open directory ...` 然后**不列出它**；
/// 而 libgit2 会把它报成 `WT_NEW`，界面上就凭空多出一条 git 不认的未跟踪变更。
fn is_readable(repo: &Repository, rel: &str) -> bool {
    match repo.workdir() {
        Some(wd) => std::fs::metadata(wd.join(rel)).is_ok(),
        None => true,
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
            // 只对未跟踪项做可读性检查。已跟踪文件被删掉走的是 WT_DELETED，
            // 那是真实变更，即使工作区里已经没有这个路径也必须保留。
            let phantom = status.contains(git2::Status::WT_NEW) && !is_readable(&repo, path_str);
            if !phantom {
                unstaged.push(make(path_str, worktree_status(status), &unstaged_stats));
            }
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
        let mut diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
        // 不做改名检测的话，一次重命名会摊成一加一删两条，文件树里看不出是同一个文件
        diff.find_similar(None)?;
        return diff_to_data(diff);
    }
    // `range:<a>..<b>`：两个任意 commit 之间。SVN 节点用它比前后两版本地基线——交付录的
    // 基线以交付树为父（spec §8.2.2），`commit:<基线>` 比的是它和交付树，恰好漏掉
    // 「这一版交付了什么」。解析不了必须报错：落到下面的默认分支会把工作区改动当成节点的变化
    if let Some(range) = scope.strip_prefix("range:") {
        let (from, to) = range
            .split_once("..")
            .filter(|(a, b)| !a.is_empty() && !b.is_empty())
            .ok_or_else(|| {
                crate::error::GitError::new(
                    "bad_diff_scope",
                    format!("无法解析的 diff 范围：{scope}"),
                )
            })?;
        let old = repo.find_commit(git2::Oid::from_str(from)?)?.tree()?;
        let new = repo.find_commit(git2::Oid::from_str(to)?)?.tree()?;
        let mut diff = repo.diff_tree_to_tree(Some(&old), Some(&new), None)?;
        diff.find_similar(None)?;
        return diff_to_data(diff);
    }
    // default: workdir vs HEAD（空仓库没有 HEAD，此时与空树比较）
    let tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    let diff =
        repo.diff_tree_to_workdir_with_index(tree.as_ref(), Some(&mut workdir_diff_opts(None)))?;
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

/// delta 在界面上用哪个路径标识。重命名时用新路径，删除时退回旧路径。
fn delta_path(delta: &git2::DiffDelta) -> String {
    delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// 与 `index_status` / `worktree_status` 保持同一套取值，前端只认这一套。
fn delta_status(status: git2::Delta) -> &'static str {
    match status {
        git2::Delta::Added | git2::Delta::Untracked => "added",
        git2::Delta::Deleted => "deleted",
        git2::Delta::Renamed => "renamed",
        git2::Delta::Copied => "copied",
        git2::Delta::Conflicted => "conflicted",
        _ => "modified",
    }
}

/// 把 diff 摊平成前端要的结构。
///
/// 文件列表先从 `deltas()` 建好，再由 `print` 往里填 hunk。这个顺序是有意的：
/// `print` 会去打开工作区文件，个别坏路径（悬空链接、权限不足、扫描途中被删）
/// 会让 libgit2 中止整个输出；先建列表的话，即使输出中断，**文件树仍然是全的**，
/// 只是部分文件没有行内容。二进制文件同理——它压根不产出 '+'/'-' 行，
/// 只有列表能证明它变过。
fn diff_to_data(diff: git2::Diff) -> Result<DiffData> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for delta in diff.deltas() {
        let path = delta_path(&delta);
        if index.contains_key(&path) {
            continue;
        }
        let old_path = delta
            .old_file()
            .path()
            .map(|p| p.to_string_lossy().to_string())
            .filter(|p| *p != path);
        index.insert(path.clone(), files.len());
        files.push(FileDiff {
            path,
            status: delta_status(delta.status()).to_string(),
            old_path,
            binary: delta.flags().contains(git2::DiffFlags::BINARY),
            hunks: Vec::new(),
        });
    }

    let outcome = diff.print(git2::DiffFormat::Patch, |delta, hunk, line| {
        let Some(&i) = index.get(&delta_path(&delta)) else {
            return true;
        };
        let file = &mut files[i];

        // 二进制标记只在真正读了内容之后才立得起来，所以这里补一次：
        // 建列表时 delta 的 BINARY 标志往往还没置上。
        if line.origin() == 'B' || delta.flags().contains(git2::DiffFlags::BINARY) {
            file.binary = true;
        }
        // libgit2 会把文件头（'F'）、hunk 头（'H'）、二进制标记（'B'）
        // 也作为行回调抛出来，直接渲染就会出现 "F diff --git ..." 这种脏行。
        if !matches!(line.origin(), '+' | '-' | ' ') {
            return true;
        }

        let (old_start, new_start, header) = match hunk {
            Some(h) => (
                h.old_start(),
                h.new_start(),
                String::from_utf8_lossy(h.header())
                    .trim_end_matches('\n')
                    .to_string(),
            ),
            None => (0, 0, String::new()),
        };
        let same_hunk = file
            .hunks
            .last()
            .is_some_and(|h| h.old_start == old_start && h.new_start == new_start);
        if !same_hunk {
            file.hunks.push(DiffHunk {
                old_start,
                new_start,
                header,
                lines: Vec::new(),
            });
        }
        file.hunks.last_mut().unwrap().lines.push(DiffLine {
            origin: line.origin(),
            content: std::str::from_utf8(line.content())
                .unwrap_or("")
                .trim_end_matches('\n')
                .to_string(),
            old_lineno: line.old_lineno(),
            new_lineno: line.new_lineno(),
        });
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
