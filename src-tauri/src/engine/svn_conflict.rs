//! 工作副本里的 SVN 冲突：分类、列表、解决、外部合并（设计 2026-11-28-update-conflict-resolve-in-jkk）。
//!
//! # 哪些冲突 jkk 能放行、能解决
//!
//! 只有**可放行的冲突**（[`SvnConflict::resolvable`]）：不是树冲突、不是目录、不是 W-file。
//!
//! - 树冲突：命令行上没有干净的非交互解法（预研 §5.1 实测），交给 TortoiseSVN。
//! - 目录：目录上的属性可能改变哪些文件受 SVN 管理（`svn:externals`、`svn:ignore`）。
//! - W-file：git 管着它，SVN 冲突标记会被 git 当成内容。`sync_in` 在 S2 把它们换回原版、
//!   S4 核对干净才 update，正常走不到这里；这一条是兜底。
//!
//! 其余的（美术、`.prefab`、`.meta` 等 git 不管的文件上的内容冲突与属性冲突）不进基线，
//! 更新照常做完，留给用户在冲突列表里解决。

use crate::engine::external_diff::{check_rel_path, launcher_error, path_arg};
use crate::engine::external_launch::{expand_template, spawn_watched, try_lock, ExitReport};
use crate::engine::external_tools_config::{find_tool, ExternalToolsConfig};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{read_config, SvnConfig};
use crate::engine::svn_lock;
use crate::engine::svn_scope::WScope;
use crate::engine::svn_sync_check::list_paths;
use crate::engine::svn_xml::{
    parse_info_conflicts, parse_update_status, InfoConflict, UpdateStatusEntry,
};
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 一处 SVN 冲突。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnConflict {
    /// 相对工作副本根，正斜杠。
    pub path: String,
    /// 内容冲突。
    pub text: bool,
    /// 属性冲突。
    pub props: bool,
    /// 树冲突。
    pub tree: bool,
    /// 磁盘上是目录。
    pub dir: bool,
    /// W-file（git 管的文件）。
    pub git_managed: bool,
    /// jkk 能放行、能解决：不是树冲突、不是目录、不是 W-file。
    pub resolvable: bool,
    /// 能交给外部合并工具：可放行、有内容冲突、svn 留下了「我的」那一份（二进制冲突没有）。
    /// 只有 [`list_conflicts`] 会填，分类时一律为假。
    pub mergeable: bool,
}

/// 用户在冲突列表上选的解法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResolveChoice {
    /// 整个文件（含属性）用我的：`--accept mine-full`。服务器这次的改动丢掉。
    Mine,
    /// 整个文件（含属性）用服务器的：`--accept theirs-full`。本地改动丢掉。
    Theirs,
    /// 用户已经在外部工具里存好合并结果：`--accept working`。
    Merged,
}

impl ResolveChoice {
    fn accept(self) -> &'static str {
        match self {
            Self::Mine => "mine-full",
            Self::Theirs => "theirs-full",
            Self::Merged => "working",
        }
    }
}

/// 读冲突并分类。`target` 为 `None` 时读整个工作副本（不限白名单：白名单外的美术文件也算），
/// 否则只读这一个路径。
pub(crate) fn read_conflicts(
    repo: &str,
    cfg: &SvnConfig,
    target: Option<&str>,
) -> Result<Vec<SvnConflict>> {
    let peg;
    let mut args = vec!["status", "--xml"];
    if let Some(t) = target {
        // `@` 结尾：路径里本身带 `@` 时不被当成 peg revision
        peg = format!("{t}@");
        args.extend(["--", peg.as_str()]);
    }
    let xml = run_svn(repo, &args)?;
    let entries: Vec<_> = parse_update_status(&xml)?
        .entries
        .into_iter()
        .filter(|e| e.is_conflicted())
        .collect();
    let w = WScope::for_repo(repo, cfg)?.filter(entries.iter().map(|e| e.path.as_str()))?;
    Ok(entries
        .iter()
        .map(|e| SvnConflict::classify(repo, e, &w))
        .collect())
}

/// `.gitignore` / `.gitattributes`（任何一级目录）：扩展名不在列表里，但它们决定哪些文件是 W-file、
/// 基线怎么归一化行尾。带冲突标记时 S6–S7 会按两边规则的混合录基线，不报任何错（独立 review），
/// 所以与 W-file 同等对待。
fn is_git_rule_file(path: &str) -> bool {
    matches!(
        path.rsplit('/').next(),
        Some(".gitignore" | ".gitattributes")
    )
}

impl SvnConflict {
    /// 一个冲突条目分类。`w_files` 是这批条目里的 W-file（[`WScope::filter`] 的结果）。
    pub(crate) fn classify(repo: &str, e: &UpdateStatusEntry, w_files: &BTreeSet<String>) -> Self {
        let dir = Path::new(repo).join(&e.path).is_dir();
        let git_managed = w_files.contains(&e.path) || is_git_rule_file(&e.path);
        Self {
            path: e.path.clone(),
            text: e.item == "conflicted",
            props: e.props == "conflicted",
            tree: e.tree_conflicted,
            dir,
            git_managed,
            resolvable: !e.tree_conflicted && !dir && !git_managed,
            mergeable: false,
        }
    }
}

/// 冲突列表（给界面）：整个工作副本的冲突，可放行的补上能否外部合并。
pub fn list_conflicts(repo: &str) -> Result<Vec<SvnConflict>> {
    let cfg = read_config(repo)?;
    let mut all = read_conflicts(repo, &cfg, None)?;
    let candidates: Vec<String> = all
        .iter()
        .filter(|c| c.resolvable && c.text)
        .map(|c| c.path.clone())
        .collect();
    // 一次 `svn info` 带所有候选；冲突一般就几个，几十个也远不到命令行长度上限。分批是为了不赌
    for chunk in candidates.chunks(50) {
        for (path, conflicts) in read_info_conflicts(repo, chunk)? {
            if text_sides(&conflicts).is_some() {
                if let Some(c) = all.iter_mut().find(|c| c.path == path) {
                    c.mergeable = true;
                }
            }
        }
    }
    Ok(all)
}

fn read_info_conflicts(repo: &str, paths: &[String]) -> Result<Vec<(String, Vec<InfoConflict>)>> {
    let pegged: Vec<String> = paths.iter().map(|p| format!("{p}@")).collect();
    let mut args = vec!["info", "--xml", "--"];
    args.extend(pegged.iter().map(String::as_str));
    parse_info_conflicts(&run_svn(repo, &args)?)
}

/// 文本冲突的三方文件都在时返回它。
fn text_sides(conflicts: &[InfoConflict]) -> Option<&InfoConflict> {
    conflicts.iter().find(|c| {
        c.kind == "text"
            && c.prev_base_file.is_some()
            && c.prev_wc_file.is_some()
            && c.cur_base_file.is_some()
    })
}

/// 现读这一个文件，确认它此刻是可放行的冲突。不信任前端手里可能已经过期的列表。
fn current_conflict(repo: &str, cfg: &SvnConfig, file: &str) -> Result<SvnConflict> {
    check_rel_path(file)?;
    let c = read_conflicts(repo, cfg, Some(file))?
        .into_iter()
        .find(|c| c.path == file)
        .ok_or_else(|| {
            GitError::new(
                "svn_conflict_state_changed",
                format!("{file} 已经不是冲突状态了（可能已在 TortoiseSVN 里解决），请刷新列表。"),
            )
        })?;
    if !c.resolvable {
        return Err(GitError::new(
            "svn_conflict_not_resolvable",
            format!(
                "{file} 是{}，本应用不处理，请用 TortoiseSVN 解决。",
                refuse_reason(&c)
            ),
        ));
    }
    Ok(c)
}

fn refuse_reason(c: &SvnConflict) -> &'static str {
    if c.tree {
        "树冲突"
    } else if c.dir {
        "目录上的冲突"
    } else {
        "git 管的文件上的 SVN 冲突"
    }
}

/// 磁盘上还带着冲突标记：svn 写的标记行以 `<<<<<<< ` / `>>>>>>> ` 开头。
fn has_conflict_markers(abs: &Path) -> Result<bool> {
    let bytes = std::fs::read(abs)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text
        .lines()
        .any(|l| l.starts_with("<<<<<<< ") || l.starts_with(">>>>>>> ")))
}

/// 解决一个冲突（设计 §3.1）。解决完再读一次，仍是冲突就报错。
///
/// 同步（更新、交付、跟进）正在跑时拒绝：S5 的 `svn update` 与这里的 `svn resolve` 会抢 wc.db 的写锁，
/// 读冲突那一步也会读到半途的状态。
///
/// 外部合并工具还开着这个文件时也拒绝（与 [`open_conflict_merge`] 共用 `try_lock`）：svn 会改写工作副本里的
/// 文件、删掉工具正在用的 `.mine` / `.rN`，用户之后在工具里一保存，合并结果又盖回来（独立 review）。
pub fn resolve_conflict(repo: &str, file: &str, choice: ResolveChoice) -> Result<()> {
    svn_lock::assert_not_syncing(repo)?;
    check_rel_path(file)?;
    let _tool = try_lock(repo, file, "svn resolve")?;
    let cfg = read_config(repo)?;
    let c = current_conflict(repo, &cfg, file)?;
    if choice == ResolveChoice::Merged {
        if !c.text {
            return Err(GitError::new(
                "svn_conflict_not_mergeable",
                format!("{file} 只有属性冲突，没有可合并的内容，请选「用我的」或「用服务器的」。"),
            ));
        }
        if has_conflict_markers(&Path::new(repo).join(file))? {
            return Err(GitError::new(
                "svn_conflict_markers_left",
                format!(
                    "{file} 里还有冲突标记（以 <<<<<<< 或 >>>>>>> 开头的行），\
                     没有标记为已解决。合并完保存之后再点一次。"
                ),
            ));
        }
    }
    let peg = format!("{file}@");
    run_svn(
        repo,
        &[
            "resolve",
            "--accept",
            choice.accept(),
            "--non-interactive",
            "--",
            &peg,
        ],
    )?;
    if read_conflicts(repo, &cfg, Some(file))?
        .iter()
        .any(|c| c.path == file)
    {
        return Err(GitError::new(
            "svn_conflict_still_conflicted",
            format!("svn resolve 跑完了，但 {file} 仍然是冲突状态。请在 TortoiseSVN 里查看。"),
        ));
    }
    Ok(())
}

/// 外部合并要的四个文件和标题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedConflictMerge {
    pub base: PathBuf,
    pub local: PathBuf,
    pub remote: PathBuf,
    /// 工作副本里的真实文件，工具把合并结果存到这里。
    pub merged: PathBuf,
    pub base_title: String,
    pub local_title: String,
    pub remote_title: String,
    pub merged_title: String,
}

/// 三方文件取 `svn info --xml` 给的路径，不按 `.rN` 文件名猜（预研 §7.3 第 3 条）。
pub fn prepare_conflict_merge(repo: &str, file: &str) -> Result<PreparedConflictMerge> {
    let cfg = read_config(repo)?;
    let c = current_conflict(repo, &cfg, file)?;
    let unsupported = || {
        GitError::new(
            "external_merge_unsupported",
            format!("{file} 没有可以逐行合并的内容（二进制文件或只有属性冲突），请选「用我的」或「用服务器的」。"),
        )
    };
    if !c.text {
        return Err(unsupported());
    }
    let info = read_info_conflicts(repo, &[file.to_string()])?;
    let sides = info
        .iter()
        .find(|(p, _)| p == file)
        .and_then(|(_, cs)| text_sides(cs))
        .ok_or_else(unsupported)?;
    let side = |p: &Option<String>| -> Result<PathBuf> {
        let path = PathBuf::from(p.as_deref().ok_or_else(unsupported)?);
        if !path.is_file() {
            return Err(GitError::new(
                "svn_conflict_state_changed",
                format!("svn 留下的冲突文件不见了：{}。请刷新列表。", path.display()),
            ));
        }
        Ok(path)
    };
    let rev = |r: Option<u64>| r.map(|n| format!(" r{n}")).unwrap_or_default();
    Ok(PreparedConflictMerge {
        base: side(&sides.prev_base_file)?,
        local: side(&sides.prev_wc_file)?,
        remote: side(&sides.cur_base_file)?,
        merged: Path::new(repo).join(file),
        base_title: format!("共同祖先{}", rev(sides.left_rev)),
        local_title: "我的（更新前）".to_string(),
        remote_title: format!("服务器{}", rev(sides.right_rev)),
        merged_title: "结果（工作副本）".to_string(),
    })
}

/// 用外部工具合并一个 SVN 冲突。立即返回；工具退出后调用 `on_exit`。不自动标记已解决。
///
/// 三方文件是 svn 自己留在工作副本里的（`.mine`、`.rN`），解决时 svn 会删掉它们，这里不复制、不清理。
pub fn open_conflict_merge(
    repo: &str,
    cfg: &ExternalToolsConfig,
    config_path: &Path,
    tool_id: &str,
    file: &str,
    on_exit: impl FnOnce(ExitReport) + Send + 'static,
) -> Result<()> {
    svn_lock::assert_not_syncing(repo)?;
    let tool = find_tool(cfg, tool_id)?;
    let template = tool.merge_args.as_ref().ok_or_else(|| {
        GitError::new(
            "external_tool_unsupported",
            format!("{} 没有配置 mergeArgs，不能用来合并", tool.label),
        )
    })?;
    let lock = try_lock(repo, file, &tool.label)?;
    let m = prepare_conflict_merge(repo, file)?;
    let args = expand_template(
        template,
        &[
            ("LOCAL", path_arg(&m.local)?),
            ("REMOTE", path_arg(&m.remote)?),
            ("BASE", path_arg(&m.base)?),
            ("MERGED", path_arg(&m.merged)?),
            ("LOCAL_TITLE", m.local_title),
            ("REMOTE_TITLE", m.remote_title),
            ("BASE_TITLE", m.base_title),
            ("MERGED_TITLE", m.merged_title),
        ],
    );
    spawn_watched(&tool.program, &args, Path::new(repo), move |report| {
        drop(lock);
        on_exit(report);
    })
    .map_err(|e| launcher_error(e, config_path))
}

/// 冲突按「jkk 能不能放行」分成两组给人看。`refused_hint` 写要回退的那组怎么办。
pub(crate) fn describe_groups(conflicts: &[SvnConflict], refused_hint: &str) -> String {
    let (kept, refused): (Vec<&SvnConflict>, Vec<&SvnConflict>) =
        conflicts.iter().partition(|c| c.resolvable);
    let paths =
        |cs: &[&SvnConflict]| list_paths(&cs.iter().map(|c| c.path.clone()).collect::<Vec<_>>());
    let mut parts = Vec::new();
    if !refused.is_empty() {
        parts.push(format!(
            "下面这些是树冲突、目录上的冲突或 git 管的文件，本应用不处理：\n{}\n{refused_hint}",
            paths(&refused)
        ));
    }
    if !kept.is_empty() {
        let lead = if refused.is_empty() {
            "下面这些 git 不管的文件有 SVN 冲突"
        } else {
            "另有下面这些 git 不管的文件的冲突一起留下了"
        };
        parts.push(format!(
            "{lead}，可以在本应用的「SVN 冲突列表」里解决（用我的 / 用服务器的 / 用外部工具合并）：\n{}",
            paths(&kept)
        ));
    }
    parts.join("\n\n")
}
