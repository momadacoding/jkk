//! W-file 口径：一条路径归不归 git 管（spec §6.3.2，2026-10-28 设计 §4）。
//!
//! # 全项目只有这一处定义
//!
//! 初始化、重建、体检、服务器集合核对、更新、交付前检查都要问同一个问题。两份判据
//! 迟早分叉，而分叉的表现很难查——要么「每次打开都报几千条漏收」，要么「永远报没问题」。
//! 原来的纯路径判断 `is_w_file` 收进本模块并改成私有：任何入口想判 W-file 都只能经过
//! [`WScope`]，「尊重 .gitignore」时漏掉一处，编译就过不去。
//!
//! # 「尊重 .gitignore」
//!
//! ```text
//! W(p) = 白名单 ∧ 扩展名
//!        ∧ ( 不尊重 ∨ p 在当前基线树里 ∨ 没被工作树里的 .gitignore 排除 )
//! ```
//!
//! - **只做减法。** `.gitignore` 的 `!` 反选不能把扩展名不匹配的文件加进来——先过
//!   路径判断，再问 `.gitignore`。
//! - **已在基线里的不会被移出去。** git 本身的语义就是已跟踪文件不受 `.gitignore` 影响，
//!   参照统一取 `refs/heads/svn/base` 的树。建库途中这个 ref 还不存在，参照为空。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_config::{SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_gitignore::{query_verdicts, GitCtx};
use crate::engine::svn_include_check::normalize_roots;
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::Path;

/// 一次操作里的 W-file 口径。每个入口在操作开头建一个，整次操作复用。
#[derive(Debug, Clone)]
pub struct WScope {
    whitelist: Vec<String>,
    roots: Vec<String>,
    extensions: Vec<String>,
    respect: Option<Respect>,
}

#[derive(Debug, Clone)]
struct Respect {
    repo_root: String,
    /// 当前基线树里的文件。
    tracked: BTreeSet<String>,
}

impl WScope {
    /// 只看白名单与扩展名。不尊重 `.gitignore` 的仓库、以及还没有仓库的场合用它。
    pub fn plain(whitelist: &[String], extensions: &[String]) -> Self {
        Self {
            whitelist: whitelist.to_vec(),
            roots: normalize_roots(whitelist),
            extensions: extensions.to_vec(),
            respect: None,
        }
    }

    /// 按仓库配置建口径。不尊重时与 [`WScope::plain`] 相同，**不起任何进程**。
    pub fn for_repo(repo_root: &str, cfg: &SvnConfig) -> Result<Self> {
        let mut scope = Self::plain(&cfg.whitelist, &cfg.extensions);
        if cfg.respect_gitignore {
            scope.respect = Some(Respect {
                repo_root: repo_root.to_string(),
                tracked: baseline_files(repo_root)?,
            });
        }
        Ok(scope)
    }

    pub fn whitelist(&self) -> &[String] {
        &self.whitelist
    }

    pub fn extensions(&self) -> &[String] {
        &self.extensions
    }

    /// 白名单条目归一后的形式（正斜杠、无尾斜杠）。
    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    pub fn respects_gitignore(&self) -> bool {
        self.respect.is_some()
    }

    /// 这批路径里哪些是 W-file。尊重 `.gitignore` 时整批只问 git 一次。
    pub fn filter<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> Result<BTreeSet<String>> {
        let mut out: BTreeSet<String> = paths
            .into_iter()
            .filter(|p| path_is_w(p, &self.roots, &self.extensions))
            .map(str::to_string)
            .collect();
        self.retain_not_ignored(&mut out)?;
        Ok(out)
    }

    /// 只做「尊重」那一半：从 `set` 里去掉被工作树 `.gitignore` 排除、且不在基线里的路径。
    ///
    /// 给已经按别的办法过滤过路径的集合用（`expected_set` 遍历磁盘时按 `core.ignoreCase`
    /// 比扩展名，与 [`path_is_w`] 的写法不同，不能再过一遍路径判断）。
    pub(crate) fn retain_not_ignored(&self, set: &mut BTreeSet<String>) -> Result<()> {
        let Some(r) = &self.respect else {
            return Ok(());
        };
        let ask: Vec<&str> = set
            .iter()
            .filter(|p| !r.tracked.contains(*p))
            .map(String::as_str)
            .collect();
        let hidden = hidden_by_gitignore(&r.repo_root, &ask)?;
        set.retain(|p| !hidden.contains(p));
        Ok(())
    }

    /// 已在基线里、磁盘上还在、但被工作树 `.gitignore` 挡住的文件。
    ///
    /// 从**空索引**起步的入口（重建基线、手动体检的磁盘树、同步前检查）用 `git add -A`
    /// 收不到它们，要显式补回去，否则基线会悄悄变小、体检会误报撒谎（设计 §4）。
    /// 磁盘上已经不在的那些由 svn 脏清单修正负责（`Fix::TakeBase`），这里不管。
    pub fn hidden_tracked_on_disk(&self) -> Result<Vec<String>> {
        let Some(r) = &self.respect else {
            return Ok(Vec::new());
        };
        let root = Path::new(&r.repo_root);
        let ask: Vec<&str> = r
            .tracked
            .iter()
            .filter(|p| path_is_w(p, &self.roots, &self.extensions))
            .filter(|p| root.join(p.as_str()).is_file())
            .map(String::as_str)
            .collect();
        Ok(hidden_by_gitignore(&r.repo_root, &ask)?
            .into_iter()
            .collect())
    }
}

/// `files` 里被工作树 `.gitignore` 排除的那些。
fn hidden_by_gitignore(repo_root: &str, files: &[&str]) -> Result<BTreeSet<String>> {
    Ok(query_verdicts(&GitCtx::Repo(repo_root), files)?
        .into_iter()
        .filter(|(_, v)| v.ignores() && v.in_worktree_gitignore())
        .map(|(p, _)| p)
        .collect())
}

/// 当前基线树里的文件。基线 ref 还不存在（建库途中）时为空。
fn baseline_files(repo_root: &str) -> Result<BTreeSet<String>> {
    let reference = format!("refs/heads/{BASELINE_BRANCH}");
    let (code, _) = run_git_code(repo_root, &["rev-parse", "--verify", "-q", &reference])?;
    match code {
        0 => {}
        1 => return Ok(BTreeSet::new()),
        other => {
            return Err(GitError::new(
                "git_cli",
                format!("读基线 ref 失败（git rev-parse 退出 {other}）"),
            ))
        }
    }
    Ok(run_git(
        repo_root,
        &["ls-tree", "-r", "-z", "--name-only", &reference],
    )?
    .split('\0')
    .filter(|s| !s.is_empty())
    .map(str::to_string)
    .collect())
}

/// 纯路径判断：在白名单内 ∧ 扩展名匹配。
///
/// # 为什么是纯路径判断，不走磁盘
///
/// 服务器上存在、磁盘上不存在的文件（被删掉的、或被 depth-exclude 掉的）恰恰是要抓的
/// 东西，走磁盘的判断会把它们整个漏掉。
///
/// 扩展名比较大小写不敏感：Windows 上 git 会把 `A.CS` 一起收进来
/// （`core.ignoreCase` 默认 true），那它就是 W-file。
fn path_is_w(rel: &str, roots: &[String], extensions: &[String]) -> bool {
    // **单文件白名单条目不过扩展名过滤**，与 `svn_exclude` 的第 ③ 段和
    // `expected_set` 保持一致（spec §6.3）。
    //
    // 两边不一致的后果是假阴性：`whitelist:["Code/notes.txt"]` + `extensions:[".cs"]`
    // 时，`notes.txt` 会被录进基线（那两处收它），而体检把它按扩展名滤掉
    // ——服务器上改了它，体检照样报 `Verified`。
    if roots.iter().any(|r| rel == r) {
        return true;
    }
    let under = roots.iter().any(|r| rel.starts_with(&format!("{r}/")));
    if !under {
        return false;
    }
    if extensions.iter().any(|e| e == "*") {
        return true;
    }
    let lower = rel.to_lowercase();
    extensions
        .iter()
        .any(|e| lower.ends_with(&e.to_lowercase()))
}
