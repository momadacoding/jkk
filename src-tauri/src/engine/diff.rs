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
        // 按字面量匹配：`f[1].txt` 当模式会连 `f1.txt` 一起匹配上，
        // 而那恰好可能就是要绕开的坏路径，拉回来之后整个输出又会中止
        opts.disable_pathspec_match(true);
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
        // 已删除的文件本来就不在工作区，不算坏路径。冲突条目 libgit2 不读内容，
        // 工作区里被删掉也照样要留在列表里（行内容由 `fill_conflicted` 另外补）
        if matches!(
            delta.status(),
            git2::Delta::Deleted | git2::Delta::Conflicted
        ) {
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

/// 变更列表里一条 delta 的状态；`None` 表示这一类不进列表。
///
/// 取值与 `statuses()` 的换算一致（libgit2 `status.c` 的 `*_delta2status`）：
/// Untracked 与 Added 同为新增。类型变化与读不了的条目原本就不在列表里
/// （原实现的掩码没有 `*_TYPECHANGE` / `WT_UNREADABLE`），这里保持不变。
fn listed_status(delta: git2::Delta) -> Option<&'static str> {
    match delta {
        git2::Delta::Added | git2::Delta::Copied | git2::Delta::Untracked => Some("added"),
        git2::Delta::Deleted => Some("deleted"),
        git2::Delta::Modified => Some("modified"),
        git2::Delta::Renamed => Some("renamed"),
        git2::Delta::Conflicted => Some("conflicted"),
        _ => None,
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

/// 「本地改动」两个列表与各自的增删行数。
///
/// **只扫一遍工作区。** `statuses()` 内部就是 `diff_tree_to_index` + `diff_index_to_workdir`
/// 再按路径配对（libgit2 `status.c`），这两个 diff 我们为了算行数本来就要做，再调一次
/// `statuses()` 等于把整个工作区重新 stat 一遍。大工作副本上一遍约 1 秒（2026-10-30 排查），
/// 所以列表直接从这两个 diff 取。两个 diff 各自保持路径顺序，配对结果里筛出某一侧的项，
/// 顺序与这一侧 diff 的顺序相同，列表顺序因此不变。
pub fn read_status(path: &str) -> Result<StatusData> {
    let repo = Repository::open(path)?;
    // index 只读一次，几个 diff 共用。不传的话每个 diff 都会在 index 变了时从磁盘重读
    // （libgit2 `diff_load_index`）：两次之间若有一次 `git add`，Staged 和 Unstaged 两个列表
    // 就来自两个版本的 index，刚暂存的文件可能两边都在或两边都没有。`statuses()` 也是只读一次
    let index = repo.index()?;
    Ok(StatusData {
        staged: staged_list(&repo, &index)?,
        unstaged: unstaged_list(&repo, &index, None)?,
    })
}

/// Stage / Unstage 之后只重读刚操作的那几个路径（2026-11-05 排查报告 6.13 方案 A）。
#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatusPatch {
    /// 整份 Staged，与 `read_status` 的相同。它只比 HEAD 和 index，不碰工作区，所以便宜。
    pub staged: Vec<FileChange>,
    /// 只有点名的这几个路径在 Unstaged 里的项，内容和顺序与 `read_status` 里的那几项相同。
    pub unstaged: Vec<FileChange>,
    /// index 是否不区分大小写（`core.ignorecase`）。前端按 git 的顺序把新项插回列表要用它：
    /// libgit2 的 diff 在不区分大小写的 index 上按不区分大小写的顺序排。
    pub ignore_case: bool,
}

/// 「本地改动」的一次局部重读：整份 Staged，加上这几个路径的 Unstaged。
///
/// 为什么只重读这几个路径就够：Stage / Unstage 只改 index 里这几个路径的项，工作区一个字节
/// 都不动；Unstaged 比的是 index 和工作区，别的路径两边都没变。例外是 `git add` 的连带改动
/// （比如加 `t/x` 会把 index 里的文件 `t` 换掉），它一定反映在 Staged 上，前端比对 Staged 发现
/// 别的路径变了就退回全量重读（`statusPatch.ts`）。
///
/// 全量读一次要把整个工作区 stat 一遍（大工作副本上 1.35–2.33 秒）；带上字面量 pathspec，
/// libgit2 的工作区迭代器只进这几个路径所在的目录。
///
/// **空列表不读 Unstaged。** 空 pathspec 在 libgit2 里是「全部」，不拦的话前端拿到整份
/// Unstaged，再追加到原列表上，每一项都会出现两次。
pub fn read_status_paths(path: &str, paths: &[String]) -> Result<StatusPatch> {
    let repo = Repository::open(path)?;
    let index = repo.index()?;
    let unstaged = if paths.is_empty() {
        Vec::new()
    } else {
        // 字面量 pathspec 在 libgit2 里仍按目录前缀匹配：点名 `t` 会把 `t/x` 也带出来。
        // 只留点名路径自己的项，否则前端会把带出来的那项和列表里原有的那项各留一份
        // （`a_named_path_does_not_bring_in_entries_under_it`）
        let named: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        let mut list = unstaged_list(&repo, &index, Some(paths))?;
        list.retain(|f| named.contains(f.path.as_str()));
        list
    };
    // libgit2 打开 index 时按同一个配置项决定是否不区分大小写（`GIT_INDEX_CAPABILITY_FROM_OWNER`）
    let ignore_case = repo
        .config()
        .and_then(|c| c.get_bool("core.ignorecase"))
        .unwrap_or(false);
    Ok(StatusPatch {
        staged: staged_list(&repo, &index)?,
        unstaged,
        ignore_case,
    })
}

fn change(path: String, kind: &str, stats: &HashMap<String, (usize, usize)>) -> FileChange {
    let (additions, deletions) = stats.get(&path).copied().unwrap_or((0, 0));
    FileChange {
        path,
        status: kind.to_string(),
        additions,
        deletions,
    }
}

/// HEAD 对比 index。只读对象库，不碰工作区。
fn staged_list(repo: &Repository, index: &git2::Index) -> Result<Vec<FileChange>> {
    let head_tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    // `statuses()` 固定带 INCLUDE_TYPECHANGE。不带的话类型变化会拆成一删一增，
    // 列表里凭空多出两项；带上之后它是一条 Typechange，按原样不列
    let mut staged_opts = DiffOptions::new();
    staged_opts.include_typechange(true);
    let staged_diff =
        repo.diff_tree_to_index(head_tree.as_ref(), Some(index), Some(&mut staged_opts))?;
    let staged_stats = line_stats(&staged_diff);

    let mut staged = Vec::new();
    for delta in staged_diff.deltas() {
        // 冲突条目两个 diff 里都有（两边的 iterator 都带 INCLUDE_CONFLICTS），只在 Unstaged
        // 列一次。Staged 也列的话会多出一项，而 Staged 的 diff 已经把它滤掉了
        if delta.status() == git2::Delta::Conflicted {
            continue;
        }
        if let Some(kind) = listed_status(delta.status()) {
            staged.push(change(delta_path(&delta), kind, &staged_stats));
        }
    }
    Ok(staged)
}

/// index 对比工作区。`only` 非空时只看这几个路径（字面量匹配），见 `read_status_paths`。
fn unstaged_list(
    repo: &Repository,
    index: &git2::Index,
    only: Option<&[String]>,
) -> Result<Vec<FileChange>> {
    let mut wd_opts = workdir_diff_opts(only);
    wd_opts.include_typechange(true);
    let workdir_diff = repo.diff_index_to_workdir(Some(index), Some(&mut wd_opts))?;
    let unstaged_stats = match readable_only(repo, &workdir_diff) {
        None => line_stats(&workdir_diff),
        // 全都读不了就没什么可统计的，行数一律退化为 0，
        // 但文件列表来自上面那个 diff 的 deltas，不受影响
        Some(good) if good.is_empty() => HashMap::new(),
        // 绕开坏路径重建的这一遍只为行数，列表不从它取。`good` 取自上面的 diff，
        // 不会比 `only` 的范围大
        Some(good) => line_stats(
            &repo.diff_index_to_workdir(Some(index), Some(&mut workdir_diff_opts(Some(&good))))?,
        ),
    };

    let mut unstaged = Vec::new();
    for delta in workdir_diff.deltas() {
        let path_str = delta_path(&delta);
        let Some(kind) = listed_status(delta.status()) else {
            continue;
        };
        // 只对新增项做可读性检查（与原来的 WT_NEW 同一范围）。已跟踪文件被删掉走的是
        // Deleted，那是真实变更，即使工作区里已经没有这个路径也必须保留。
        if kind == "added" && !is_readable(repo, &path_str) {
            continue;
        }
        unstaged.push(change(path_str, kind, &unstaged_stats));
    }
    Ok(unstaged)
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
    // 空仓库没有 HEAD，此时与空树比较
    let tree = repo.head().ok().and_then(|h| h.peel_to_tree().ok());
    // `staged:<路径>` / `unstaged:<路径>`：「本地改动」右侧只显示选中的那一个文件，只读它
    // （2026-11-05 排查报告 6.13 方案 C）。整区读要把整个工作区 stat 一遍、再读出每个改动文件的
    // 内容，而界面只用其中一项。结果与整区读里这个路径的那一项相同（`tests/diff_file_scope.rs`）
    if let Some((section, file)) = scope
        .split_once(':')
        .filter(|(s, _)| matches!(*s, "staged" | "unstaged"))
    {
        // 空路径必须报错：空 pathspec 在 libgit2 里是「全部」，这条快路径会悄悄变回整区读
        if file.is_empty() {
            return Err(crate::error::GitError::new(
                "bad_diff_scope",
                format!("无法解析的 diff 范围：{scope}"),
            ));
        }
        return if section == "staged" {
            staged_side(&repo, tree.as_ref(), Some(file))
        } else {
            workdir_side(&repo, tree.as_ref(), Some(file), |opts| {
                repo.diff_index_to_workdir(None, Some(opts))
            })
        };
    }
    match scope.as_str() {
        // 「本地改动」按分区比较（第 65 条），与外部 diff 的两侧取法一致：
        // Staged = HEAD 对比暂存区，即这次提交会提交什么
        "staged" => staged_side(&repo, tree.as_ref(), None),
        // Unstaged = 暂存区对比工作区，即还没暂存的那部分
        "unstaged" => workdir_side(&repo, tree.as_ref(), None, |opts| {
            repo.diff_index_to_workdir(None, Some(opts))
        }),
        // workdir（以及其余认不出的 scope）：HEAD 对比工作区
        _ => workdir_side(&repo, tree.as_ref(), None, |opts| {
            repo.diff_tree_to_workdir_with_index(tree.as_ref(), Some(opts))
        }),
    }
}

/// HEAD 对比暂存区，即这次提交会提交什么。`file` 非空时只要这一个文件，见 `diff_to_data_only`。
fn staged_side(
    repo: &Repository,
    tree: Option<&git2::Tree>,
    file: Option<&str>,
) -> Result<DiffData> {
    let mut opts = DiffOptions::new();
    if let Some(f) = file {
        opts.pathspec(f);
        // 字面量匹配，理由同 `workdir_diff_opts`
        opts.disable_pathspec_match(true);
    }
    // 不做改名检测：Staged 列表来自 `read_status`，它把 `git mv` 列成一删一增两项。
    // 这里若合成一条改名，点到旧路径那一项时右侧就找不到它的 diff
    let diff = repo.diff_tree_to_index(tree, None, Some(&mut opts))?;
    let mut data = diff_to_data_only(diff, file)?;
    // 合并冲突中 index 里的冲突条目也会出现在这里，但 `read_status` 只把冲突文件放进
    // Unstaged（显示 HEAD 对比工作区）。留着它，Staged 的 diff 就多出列表里没有的项
    data.files.retain(|f| f.status != "conflicted");
    Ok(data)
}

/// 拿 `build` 给出的那一侧与工作区比。坏路径与冲突文件的处理两个 scope 共用。
/// `file` 非空时只要这一个文件（字面量 pathspec + 精确匹配），见 `diff_to_data_only`。
fn workdir_side<'r>(
    repo: &'r Repository,
    head_tree: Option<&git2::Tree>,
    file: Option<&str>,
    build: impl Fn(&mut DiffOptions) -> std::result::Result<git2::Diff<'r>, git2::Error>,
) -> Result<DiffData> {
    let only = file.map(|f| vec![f.to_string()]);
    let diff = build(&mut workdir_diff_opts(only.as_deref()))?;
    let mut data = match readable_only(repo, &diff) {
        None => diff_to_data_only(diff, file)?,
        // 一个能读的都没有：返回空 diff 而不是让整个视图报错。
        // 注意此时不能拿空的 pathspec 去重建——那等于不加限制，又会撞上坏路径。
        Some(good) if good.is_empty() => DiffData { files: Vec::new() },
        Some(good) => diff_to_data_only(build(&mut workdir_diff_opts(Some(&good)))?, file)?,
    };
    // 补不出来只是冲突文件没有行内容，不能让整个变更视图跟着报错
    if let Err(e) = fill_conflicted(repo, head_tree, &mut data) {
        log::warn!("冲突文件的 diff 读取失败：{}", e.message);
    }
    Ok(data)
}

/// 给冲突文件补上 HEAD 对比工作区的 hunk（spec §4.1 ②），`status` 仍是 `conflicted`。
///
/// 带上 index 时 libgit2 对冲突条目只给一个 `Conflicted`、不带 hunk（第 63 条的根因）。
/// 对这些路径单独做一次 tree 对比工作区（不经过 index），libgit2 就按普通改动比较，
/// 并且套用 autocrlf 等过滤器——直接拿 blob 和工作区字节比（`Patch::from_blob_and_buffer`）
/// 在 autocrlf 下会把每一行都当成改动。行的收集复用 `diff_to_data`。
fn fill_conflicted(
    repo: &Repository,
    tree: Option<&git2::Tree>,
    data: &mut DiffData,
) -> Result<()> {
    let paths: Vec<String> = data
        .files
        .iter()
        .filter(|f| f.status == "conflicted")
        .map(|f| f.path.clone())
        .collect();
    if paths.is_empty() {
        return Ok(());
    }
    let mut opts = workdir_diff_opts(Some(&paths));
    // 路径按字面量匹配：`f[1].txt` 不能被当成模式去匹配 `f1.txt`
    opts.disable_pathspec_match(true);
    let extra = diff_to_data(repo.diff_tree_to_workdir(tree, Some(&mut opts))?)?;
    for found in extra.files {
        if let Some(file) = data
            .files
            .iter_mut()
            .find(|f| f.status == "conflicted" && f.path == found.path)
        {
            file.hunks = found.hunks;
            file.binary = found.binary;
        }
    }
    Ok(())
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

/// 与 `listed_status`（变更列表）保持同一套取值，前端只认这一套。
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
    diff_to_data_only(diff, None)
}

/// 同 `diff_to_data`，`exact` 非空时只要路径恰好是它的那一项。
///
/// 单文件 scope 用的字面量 pathspec 在 libgit2 里仍按目录前缀匹配：已跟踪的文件 `assets` 换成了
/// 未跟踪的同名目录时，点名 `assets` 会带出目录下每一个文件。`diff.print` 会把它们的内容全读一遍，
/// 「只读一个文件」反而比整区读还慢（第二轮独立 review 第 6 条）。所以列表只建精确匹配的那一项，
/// 补丁也只对它生成（`Patch::from_diff`），其余 delta 不碰内容。
fn diff_to_data_only(diff: git2::Diff, exact: Option<&str>) -> Result<DiffData> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let wanted = |path: &str| exact.map_or(true, |e| e == path);

    for delta in diff.deltas() {
        let path = delta_path(&delta);
        if index.contains_key(&path) || !wanted(&path) {
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

    let mut on_line =
        |delta: git2::DiffDelta, hunk: Option<git2::DiffHunk>, line: git2::DiffLine| {
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
            // 上面刚保证过至少有一个 hunk，else 分支走不到；不 unwrap 是为了 panic 不进回调
            let Some(current) = file.hunks.last_mut() else {
                return true;
            };
            current.lines.push(DiffLine {
                origin: line.origin(),
                content: std::str::from_utf8(line.content())
                    .unwrap_or("")
                    .trim_end_matches('\n')
                    .to_string(),
                old_lineno: line.old_lineno(),
                new_lineno: line.new_lineno(),
            });
            true
        };
    let outcome = match exact {
        None => diff.print(git2::DiffFormat::Patch, |d, h, l| on_line(d, h, l)),
        Some(_) => (|| {
            for i in 0..diff.deltas().len() {
                let keep = diff.get_delta(i).is_some_and(|d| wanted(&delta_path(&d)));
                if !keep {
                    continue;
                }
                if let Some(mut patch) = git2::Patch::from_diff(&diff, i)? {
                    patch.print(&mut |d, h, l| on_line(d, h, l))?;
                }
            }
            Ok(())
        })(),
    };
    if let Err(e) = outcome {
        log::warn!(
            "diff 输出中断，已保留 {} 个文件的结果：{}",
            files.len(),
            e.message()
        );
    }
    Ok(DiffData { files })
}
