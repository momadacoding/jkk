//! 工作树里的 `.gitignore`：问 git「这条路径是谁裁决的」、以及把裁决写成人看得懂的话。
//!
//! # 为什么要单独成模块
//!
//! 工作树里的 `.gitignore` 优先级高于本应用生成的 `.git/info/exclude`（2026-10-27
//! 实测），是白名单压不住的唯一一类规则。纳入集合报告、建库向导的冲突检测、
//! 「尊重 .gitignore」口径下的 W-file 判定，问的都是同一个问题，答案必须出自同一处。

use crate::engine::git_cli::{run_git, run_git_code, run_git_stdin_partial};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::DisabledGitignore;
use crate::engine::svn_dirty::Fix;
use crate::engine::svn_exclude::{generate_exclude, write_exclude};
use crate::engine::svn_include_check::{expected_set_folded, normalize_roots};
use crate::engine::svn_scope::WScope;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::parse_status_entries;
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 在哪个 git 上下文里问。
///
/// `Probe` 给「还没有 `.git`」的时候用：一个一次性的 git 目录，配上一棵与工作副本同构的
/// 影子树当 work tree（见 [`ProbeGitDir`]）。cwd 就是那棵影子树，路径与真仓库里一致。
pub(crate) enum GitCtx<'a> {
    Repo(&'a str),
    Probe {
        work_tree: &'a str,
        git_dir: &'a Path,
    },
}

impl GitCtx<'_> {
    pub(crate) fn work_tree(&self) -> &str {
        match self {
            GitCtx::Repo(root) => root,
            GitCtx::Probe { work_tree, .. } => work_tree,
        }
    }

    /// `--git-dir` / `--work-tree` 是全局选项，必须排在子命令之前。
    fn full_args<'b>(&'b self, args: &[&'b str], scratch: &'b mut Vec<String>) -> Vec<&'b str> {
        let mut out: Vec<&str> = Vec::with_capacity(args.len() + 2);
        if let GitCtx::Probe { work_tree, git_dir } = self {
            scratch.push(format!("--git-dir={}", git_dir.display()));
            scratch.push(format!("--work-tree={work_tree}"));
            out.extend(scratch.iter().map(String::as_str));
        }
        out.extend_from_slice(args);
        out
    }

    /// 退出码不为 0 也返回 `Ok`，由调用方判断（同 `run_git_code`）。
    pub(crate) fn run_code(&self, args: &[&str]) -> Result<(i32, String)> {
        let mut scratch = Vec::new();
        let full = self.full_args(args, &mut scratch);
        run_git_code(self.work_tree(), &full)
    }

    /// 这个上下文里 git 的忽略规则是否大小写不敏感（`core.ignoreCase`）。
    fn ignore_case(&self) -> bool {
        match self.run_code(&["config", "--bool", "--get", "core.ignoreCase"]) {
            Ok((_, out)) => out.trim().eq_ignore_ascii_case("true"),
            Err(_) => false,
        }
    }

    /// 同 `run_git_stdin_partial`：fatal 时把已经输出的部分一并交回。
    pub(crate) fn run_stdin_partial(
        &self,
        args: &[&str],
        stdin: &[u8],
    ) -> Result<(String, Option<GitError>)> {
        let mut scratch = Vec::new();
        let full = self.full_args(args, &mut scratch);
        run_git_stdin_partial(self.work_tree(), &full, stdin)
    }
}

/// git 对一个路径的裁决：`check-ignore -v` 的「来源 / 行号 / 模式」三段。
///
/// 没有匹配任何规则时三段都为空。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub source: String,
    pub line: String,
    pub pattern: String,
}

impl Verdict {
    /// 这条裁决是不是「忽略」。以 `!` 开头的是反选，即「不忽略」。
    pub(crate) fn ignores(&self) -> bool {
        !self.pattern.is_empty() && !self.pattern.starts_with('!')
    }

    /// 来源是不是工作树里的某个 `.gitignore`（而不是 `.git/info/exclude` 或全局文件）。
    ///
    /// git 对工作树里的 `.gitignore` 给的是仓库相对路径（`Code/.gitignore`），
    /// 对全局文件给的是绝对路径，所以「相对路径且以 `.gitignore` 结尾」就够了。
    pub(crate) fn in_worktree_gitignore(&self) -> bool {
        let s = self.source.replace('\\', "/");
        let file_name = s.rsplit('/').next().unwrap_or("");
        file_name == ".gitignore" && !Path::new(&s).is_absolute() && !s.starts_with(".git/")
    }

    /// 写成用户看得懂的一句话（spec §6）。
    ///
    /// 不用 git 的 `文件:行号:规则`：用户看不出那是一个可以去打开的文件，也看不出
    /// 它在哪一层目录。本应用生成的规则不暴露 `.git/info/exclude` 这个路径——
    /// 那不是用户该去改的地方。
    pub(crate) fn describe(&self, repo_root: &str) -> String {
        if self.in_worktree_gitignore() {
            return format!(
                "{} 第 {} 行 `{}`",
                display_path(repo_root, &self.source),
                self.line,
                self.pattern
            );
        }
        if self.source.replace('\\', "/").ends_with("info/exclude") {
            return format!("本应用生成的规则 `{}`", self.pattern);
        }
        format!("{} 第 {} 行 `{}`", self.source, self.line, self.pattern)
    }
}

/// 仓库相对路径 → 完整的 Windows 形态路径，供显示用。
///
/// 内部比较一律用正斜杠相对路径；只有给人看的时候才拼成这个形状（spec §7）。
pub fn display_path(repo_root: &str, rel: &str) -> String {
    let root = repo_root.replace('/', "\\");
    format!(
        "{}\\{}",
        root.trim_end_matches('\\'),
        rel.replace('/', "\\")
    )
}

/// 一次性问 git「这批文件各自是被哪条规则决定的」。
///
/// 原来是每个文件一次子进程，只能查前 20 个；几百个文件时逐个查又太慢。
/// `--stdin` 一次查完，才能按规则分组。
///
/// # 参数逐个不能省
///
/// - `--no-index`：「多收」方向的文件**必然已经在索引里**，而 `check-ignore` 默认
///   会先看索引、对已跟踪文件一律给空裁决。2026-09-14 实测同一个文件
///   （`Code/Junk.psd`，被 `/Code/**` 排除，用 `git add -f` 强行入索引）不带它得到
///   `::<TAB>Code/Junk.psd`，带上得到 `.git/info/exclude:3:/Code/**`。
/// - `--non-matching`：没被忽略的文件也给出裁决（反选规则或空），两个方向都能指认。
/// - `-z`：输入输出都按 NUL 分隔、不做引号转义。不带它中文路径会被转义成八进制，
///   回填时对不上号。输出是四段一组：`来源\0行号\0模式\0路径\0`（2026-10-27 实测）。
///
/// # 某个路径让 git fatal 时，跳过它接着查
///
/// `check-ignore` 逐个路径作答，遇到它拒绝的路径就整个退出。已知的触发是白名单里的
/// **目录符号链接**：期望集合跟随它，于是 `Code/Link/x.cs` 出现在「实际没有」一侧，
/// 而 git 对它报 `pathspec … is beyond a symbolic link`（PR #90 第二轮 review 指出）。
/// 整批作废的话，几百个被 `.gitignore` 挡掉的文件也一起变成「查不到」。
///
/// 拿 fatal 之前的输出（每个路径答完就刷出，实测 fatal 前的记录完整可读），第一个
/// 没答的就是出事的路径，跳过它续查。与它**同目录及更深**的路径共享同一段前导路径，
/// 越过符号链接时会一个接一个 fatal，所以一并跳过，免得每个都重起一次进程。
/// 查不到的路径不进结果：报告里落进「查不到裁决规则」，「尊重」口径下视为没被挡。
///
/// `Err` 只剩「连进程都起不来」——那种情况再试也一样，交给调用方决定怎么办。
pub(crate) fn query_verdicts(ctx: &GitCtx, files: &[&str]) -> Result<BTreeMap<String, Verdict>> {
    let mut out = BTreeMap::new();
    let mut rest: Vec<&str> = files.to_vec();
    let mut resumes = 0;
    while !rest.is_empty() {
        let (answered, fatal) = verdict_batch(ctx, &rest, &mut out)?;
        let Some(e) = fatal else {
            // 没有 fatal 却没答全：git 回显的路径与输入对不上号，其余路径落进「查不到」。
            // 至今没见过这种形态，留个痕迹，免得只剩一个解释不了的不一致报告
            if answered < rest.len() {
                log::warn!(
                    "check-ignore 的输出从 {:?} 起对不上输入，其余 {} 个路径不带规则",
                    rest[answered],
                    rest.len() - answered
                );
            }
            break;
        };
        let Some(&culprit) = rest.get(answered) else {
            break;
        };
        log::warn!(
            "git 拒绝查询 {culprit} 的裁决规则，跳过它续查：{}",
            e.message.trim()
        );
        resumes += 1;
        if resumes > MAX_RESUMES {
            log::warn!("续查已达 {MAX_RESUMES} 次，其余路径不带规则");
            break;
        }
        // 父目录为空（仓库根下的文件）时只跳它自己——空前缀会匹配所有路径
        let dir = culprit.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        rest = rest[answered + 1..]
            .iter()
            .copied()
            .filter(|f| dir.is_empty() || !f.starts_with(&format!("{dir}/")))
            .collect();
    }
    Ok(out)
}

/// fatal 之后最多续查几次。每次都要重起一个 git 进程；超过这个数多半不是个别
/// 路径的问题（仓库本身坏了），再查也是白花时间。
const MAX_RESUMES: usize = 32;

/// 一次 git 调用。返回「从头算起连续答完了几个」和 fatal（若有）。
fn verdict_batch(
    ctx: &GitCtx,
    files: &[&str],
    out: &mut BTreeMap<String, Verdict>,
) -> Result<(usize, Option<GitError>)> {
    let mut input = Vec::new();
    for f in files {
        input.extend_from_slice(f.as_bytes());
        input.push(0);
    }
    let (stdout, fatal) = ctx.run_stdin_partial(
        &[
            "check-ignore",
            "--no-index",
            "-v",
            "--non-matching",
            "-z",
            "--stdin",
        ],
        &input,
    )?;
    let parts: Vec<&str> = stdout.split('\0').collect();
    let mut answered = 0;
    for c in parts.chunks_exact(4) {
        // 答案按输入顺序一条对一条；对不上就不再往后数，第一个没对上的当成出事的那个
        if files.get(answered) == Some(&c[3]) {
            answered += 1;
        } else {
            break;
        }
        out.insert(
            c[3].to_string(),
            Verdict {
                source: c[0].to_string(),
                line: c[1].to_string(),
                pattern: c[2].to_string(),
            },
        );
    }
    Ok((answered, fatal))
}

/// 冲突列表里每条规则最多列的样例数。
const SAMPLES_PER_RULE: usize = 3;

/// 一个起作用的 `.gitignore`（2026-10-28 设计 §3 第 3 步的列表项）。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitignoreConflict {
    /// 仓库相对路径、正斜杠。建库请求的 `acknowledged` 用它对账。
    pub path: String,
    /// 完整的 Windows 形态路径，给人看的。
    pub full_path: String,
    /// 在 SVN 里受版本控制（含已 `svn add` 未提交）。受控的不改名（设计 §3）。
    pub versioned: bool,
    /// 现在被外层的 `.gitignore` 挡着、根本没被 git 读到；外层改名停用之后才开始起作用。
    ///
    /// git 不进入已被忽略的目录，所以只有把外层停用之后才看得到它。第 2 步在临时目录里
    /// 模拟停用把它提前查出来，好让用户在第 3 步一次看全（2026-10-29 真实窗口验证发现：
    /// 不提前列出的话，建库时冒出来、送回第 3 步、再查还是看不到，死循环）。
    pub active_after_rename: bool,
    /// 起作用的每一行，按行号排序。
    pub rules: Vec<GitignoreRule>,
}

/// 白名单范围内磁盘上的样子：所有文件，以及能影响它们的 `.gitignore`。
pub(crate) struct TreeScan {
    /// 工作副本根与白名单各级祖先目录里的 `.gitignore`，加上白名单子树里所有的。
    pub gitignores: BTreeSet<String>,
    /// 白名单子树里所有的文件（仓库相对路径、正斜杠）。不进 `.svn` / `.git`，
    /// 不进嵌套的 git 仓库——git 自己也不进去。
    pub files: BTreeSet<String>,
}

/// 扫一遍磁盘，得到 [`TreeScan`]。
///
/// 跟随 junction、只查祖先链防环，与 `svn_include_check::walk` 同一套理由。
pub(crate) fn scan_tree(repo_root: &str, whitelist: &[String]) -> Result<TreeScan> {
    let root = Path::new(repo_root);
    let mut scan = TreeScan {
        gitignores: BTreeSet::new(),
        files: BTreeSet::new(),
    };
    for r in normalize_roots(whitelist) {
        // 祖先目录（含工作副本根）里的 `.gitignore`：只看这一层，不往旁支走——
        // 旁支目录里的规则管不到白名单内的路径
        let segs: Vec<&str> = r.split('/').collect();
        for depth in 0..segs.len() {
            let dir = segs[..depth].join("/");
            let rel = if dir.is_empty() {
                ".gitignore".to_string()
            } else {
                format!("{dir}/.gitignore")
            };
            if root.join(&rel).is_file() {
                scan.gitignores.insert(rel);
            }
        }
        let abs = root.join(&r);
        if abs.is_file() {
            scan.files.insert(r.clone());
        } else if abs.is_dir() {
            let mut chain = vec![std::fs::canonicalize(&abs)?];
            scan_dir(&abs, root, &mut chain, &mut scan)?;
        }
    }
    Ok(scan)
}

fn scan_dir(dir: &Path, root: &Path, chain: &mut Vec<PathBuf>, scan: &mut TreeScan) -> Result<()> {
    for ent in std::fs::read_dir(dir)? {
        let p = ent?.path();
        let name = p
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.eq_ignore_ascii_case(".svn") || name.eq_ignore_ascii_case(".git") {
            continue;
        }
        if p.is_dir() {
            if p.join(".git").exists() {
                continue; // 嵌套 git 仓库：git 只记一个子模块指针，不进去
            }
            let canon = std::fs::canonicalize(&p)?;
            if chain.contains(&canon) {
                continue;
            }
            chain.push(canon);
            scan_dir(&p, root, chain, scan)?;
            chain.pop();
        } else {
            let rel = p
                .strip_prefix(root)
                .map_err(|_| GitError::new("include_set", "路径不在仓库内"))?
                .to_string_lossy()
                .replace('\\', "/");
            // git 打开的是 `目录/.gitignore`：大小写不敏感的卷上 `.GitIgnore` 也会被读到，
            // 而裁决来源照样报 `目录/.gitignore`（2026-10-30 实测）。按 git 的写法记，
            // 影子树复制与改名停用都按这个名字打开同一个文件；大小写敏感的卷上
            // `.GitIgnore` 打不开 `.gitignore`，git 也不读它，不收
            if name.eq_ignore_ascii_case(".gitignore") && p.with_file_name(".gitignore").is_file() {
                let as_git = match rel.rsplit_once('/') {
                    Some((d, _)) => format!("{d}/.gitignore"),
                    None => ".gitignore".to_string(),
                };
                scan.gitignores.insert(as_git);
            }
            scan.files.insert(rel);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitignoreRule {
    pub line: u32,
    pub pattern: String,
    pub effect: RuleEffect,
    /// 按设计 §3 的口径计数：少收数 SVN 里有的 W-file，多收数用户没选的文件。
    pub count: usize,
    /// 最多 [`SAMPLES_PER_RULE`] 个，按字母序。
    pub samples: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleEffect {
    /// 挡住了 SVN 里有的 W-file（少收）。
    Excludes,
    /// 用 `!` 把用户没选的文件纳入了 git（多收）。
    Includes,
}

/// 一次性的探针 git 目录：建库向导第 2 步还没有 `.git`，又必须得到与建库后完全一致的
/// 裁决（设计 §3）。
///
/// 放在系统临时目录，里面写本应用将要生成的 `info/exclude`。git 读的 work tree 是同一
/// 临时目录下的一棵**影子树**（[`ProbeGitDir::shadow`]）：只放 `.gitignore` 文件，
/// 好模拟「某些已经改名停用」。`check-ignore` 只看路径，不要求文件真的存在
/// （`scope_excludes_a_missing_file_under_an_ignored_directory` 实测钉住）。
/// 工作副本里不留任何东西；`Drop` 时整个删掉。
pub(crate) struct ProbeGitDir {
    dir: PathBuf,
    git_dir: PathBuf,
}

static PROBE_SEQ: AtomicU64 = AtomicU64::new(0);

impl ProbeGitDir {
    pub(crate) fn create(
        work_tree: &str,
        whitelist: &[String],
        extensions: &[String],
    ) -> Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "jkk-ignore-probe-{}-{nanos}-{}",
            std::process::id(),
            PROBE_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        // create_dir 而不是 create_dir_all：撞名时失败，绝不复用别人的目录
        std::fs::create_dir(&dir)?;
        // 先建出值，后面任何一步失败都由 Drop 收拾
        let probe = Self {
            git_dir: dir.join(".git"),
            dir,
        };
        let d = probe.dir.to_string_lossy().to_string();
        run_git(&d, &["init", "-q"])?;
        // 建库后真仓库的 `core.ignoreCase` 由 git init 在工作副本所在的卷上探测。
        // 探针在另一个目录，可能落在另一种卷上，这里按工作副本的实际情况显式设：
        // 两边对大小写的裁决不一致，第 2 步看到的清单就与建库时拦下的不同。
        let wt = Path::new(work_tree);
        if wt.join(".svn").exists() {
            let ci = wt.join(".SVN").exists();
            run_git(
                &d,
                &[
                    "config",
                    "core.ignoreCase",
                    if ci { "true" } else { "false" },
                ],
            )?;
        }
        write_exclude(&d, &generate_exclude(whitelist, extensions))?;
        Ok(probe)
    }

    /// 重建影子树：把 `gitignores` 从真实工作副本复制过来，`disabled` 里的跳过
    /// （等于「已经改名停用」）。返回影子树的路径，配 [`ProbeGitDir::ctx_on`] 用。
    pub(crate) fn shadow(
        &self,
        real_root: &str,
        gitignores: &BTreeSet<String>,
        disabled: &BTreeSet<String>,
    ) -> Result<String> {
        let shadow = self.dir.join("shadow");
        if shadow.exists() {
            std::fs::remove_dir_all(&shadow)?;
        }
        std::fs::create_dir(&shadow)?;
        for g in gitignores.iter().filter(|g| !disabled.contains(*g)) {
            let dst = shadow.join(g);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(Path::new(real_root).join(g), &dst)?;
        }
        Ok(shadow.to_string_lossy().to_string())
    }

    pub(crate) fn ctx_on<'a>(&'a self, work_tree: &'a str) -> GitCtx<'a> {
        GitCtx::Probe {
            work_tree,
            git_dir: &self.git_dir,
        }
    }
}

/// 模拟 / 执行改名停用的轮数上限。每一轮至少停掉一个文件，正常工程一两轮就停；
/// 上限只防病态目录结构把循环拖死。
pub(crate) const MAX_RENAME_ROUNDS: usize = 16;

/// 第 2 步：查出起作用的 `.gitignore`，**连同外层停用之后才会起作用的**。
///
/// 不受版本控制的会在建库时被改名（不勾「尊重」时），于是一轮轮在影子树里把它们
/// 拿掉再查，直到不再冒出新的。受版本控制的不会被改名，不往下模拟。
pub(crate) fn detect_with_renames(
    probe: &ProbeGitDir,
    real_root: &str,
    whitelist: &[String],
    extensions: &[String],
    fixes: &BTreeMap<String, Fix>,
) -> Result<Vec<GitignoreConflict>> {
    let scan = scan_tree(real_root, whitelist)?;
    let mut disabled: BTreeSet<String> = BTreeSet::new();
    let mut all: Vec<GitignoreConflict> = Vec::new();
    for round in 0..MAX_RENAME_ROUNDS {
        let shadow = probe.shadow(real_root, &scan.gitignores, &disabled)?;
        let found = detect_conflicts(
            &probe.ctx_on(&shadow),
            real_root,
            &scan,
            whitelist,
            extensions,
            fixes,
        )?;
        let mut grew = false;
        for mut c in found {
            if all.iter().any(|a| a.path == c.path) {
                continue;
            }
            c.active_after_rename = round > 0;
            if !c.versioned {
                disabled.insert(c.path.clone());
                grew = true;
            }
            all.push(c);
        }
        if !grew {
            break;
        }
    }
    Ok(all)
}

impl Drop for ProbeGitDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 查出起作用的 `.gitignore`（设计 §3）。
///
/// - **少收**：挡住了服务器期望集合（与 `server_expected_set` 同算法）里的文件。
///   只挡未版本控制文件的不算——那些本来就不进基线。
/// - **多收**：`!` 反选把扩展名不匹配的文件纳入了 git。
///
/// 只认工作树里的 `.gitignore`；本应用生成的 `info/exclude` 与全局 `excludesFile`
/// 不算（后者压不过前者的反选，PR #90 实测）。
///
/// `ctx` 是问裁决的地方（真仓库，或探针 + 影子树），`root` 永远是**真实**工作副本根：
/// 期望集合、是否受版本控制、给人看的路径都以它为准。「多收」的候选取自 `scan`
/// 而不是 `ls-files -o`——影子树里没有真实文件，两种上下文必须同一种求法。
pub(crate) fn detect_conflicts(
    ctx: &GitCtx,
    root: &str,
    scan: &TreeScan,
    whitelist: &[String],
    extensions: &[String],
    fixes: &BTreeMap<String, Fix>,
) -> Result<Vec<GitignoreConflict>> {
    // 空白名单 / 空扩展名绝不能退化成「全部」（AGENTS.md §6.2）
    if normalize_roots(whitelist).is_empty() || extensions.is_empty() {
        return Ok(Vec::new());
    }
    let disk = expected_set_folded(root, whitelist, extensions, ctx.ignore_case())?;

    // 服务器期望集合：磁盘 W-file − 服务器上没有的（Drop）+ 磁盘上没有但服务器有的（TakeBase）
    let plain = WScope::plain(whitelist, extensions);
    let take_base = plain.filter(
        fixes
            .iter()
            .filter(|(_, f)| **f == Fix::TakeBase)
            .map(|(p, _)| p.as_str()),
    )?;
    let mut server = disk.clone();
    for (path, fix) in fixes {
        match fix {
            Fix::Drop => {
                server.remove(path);
            }
            Fix::TakeBase => {
                if take_base.contains(path) {
                    server.insert(path.clone());
                }
            }
        }
    }

    // 来源 → 行号 → (规则, 作用, 命中的文件)
    type Hits = BTreeMap<String, BTreeMap<u32, (String, RuleEffect, Vec<String>)>>;
    let mut hits: Hits = BTreeMap::new();
    let mut record = |v: Verdict, effect: RuleEffect, path: String| {
        hits.entry(v.source)
            .or_default()
            .entry(v.line.parse().unwrap_or(0))
            .or_insert_with(|| (v.pattern, effect, Vec::new()))
            .2
            .push(path);
    };

    let ask: Vec<&str> = server.iter().map(String::as_str).collect();
    for (path, v) in query_verdicts(ctx, &ask)? {
        if v.ignores() && v.in_worktree_gitignore() {
            record(v, RuleEffect::Excludes, path);
        }
    }

    // 多收：不是用户选的文件，却被某个 `.gitignore` 的 `!` 判成「不忽略」。父目录被
    // 忽略时 `check-ignore` 报的是父目录那条规则，所以 `!` 的裁决就是最终结论。
    // 嵌套 git 仓库里的文件 `scan` 不收——那不是 .gitignore 的问题，最终核对会单独点名。
    let over: Vec<&str> = scan
        .files
        .iter()
        .filter(|f| !disk.contains(*f))
        .map(String::as_str)
        .collect();
    for (path, v) in query_verdicts(ctx, &over)? {
        if v.pattern.starts_with('!') && v.in_worktree_gitignore() {
            record(v, RuleEffect::Includes, path);
        }
    }

    let mut out = Vec::with_capacity(hits.len());
    for (source, lines) in hits {
        let rel = source.replace('\\', "/");
        out.push(GitignoreConflict {
            full_path: display_path(root, &rel),
            versioned: is_versioned(root, &rel)?,
            active_after_rename: false,
            rules: lines
                .into_iter()
                .map(|(line, (pattern, effect, mut files))| {
                    files.sort();
                    GitignoreRule {
                        line,
                        pattern,
                        effect,
                        count: files.len(),
                        samples: files.into_iter().take(SAMPLES_PER_RULE).collect(),
                    }
                })
                .collect(),
            path: rel,
        });
    }
    Ok(out)
}

/// 这个文件在 SVN 里是否受版本控制。已 `svn add` 未提交的也算——改它的名同样会变成
/// 一个要交付的 SVN 改动。
///
/// # 为什么在文件所在目录里问、只传文件名
///
/// svn 的命令行参数走系统 ANSI 代码页。2026-10-29 实测（ACP 1252、SlikSvn 1.14.2）：
/// `资源/.gitignore` 到了 svn 那里变成 `??/.gitignore`，报「找不到节点」——若按「不受控」
/// 处理，一个**团队提交的** `.gitignore` 就会被改名（反目标第 2 条）。工作目录走的是
/// Unicode 的进程 API，不受代码页影响，而文件名 `.gitignore` 是纯 ASCII。
fn is_versioned(repo_root: &str, rel: &str) -> Result<bool> {
    let abs = Path::new(repo_root).join(rel);
    let (Some(dir), Some(name)) = (abs.parent(), abs.file_name()) else {
        return Ok(false);
    };
    let name = name.to_string_lossy().to_string();
    let xml = match run_svn(
        &dir.to_string_lossy(),
        &[
            "status",
            "--xml",
            "-v",
            "--depth",
            "empty",
            "--non-interactive",
            "--",
            &peg_safe(&name),
        ],
    ) {
        Ok(x) => x,
        // 所在目录本身不受控（W155007「不是工作副本」/ W155010「找不到节点」）：
        // 那它自然不受控
        Err(e) if e.message.contains("155007") || e.message.contains("155010") => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(parse_status_entries(&xml)?
        .first()
        .is_some_and(|e| !matches!(e.item.as_str(), "unversioned" | "ignored" | "none")))
}

/// 停用名的上限：`.gitignore.jkk-disabled`、`-2` … `-99`。撞满说明那个目录里堆了一百个
/// 停用文件，那不是正常状态，停下来让用户看。
const MAX_DISABLED_SUFFIX: u32 = 99;

/// 把 `rel`（一个 `.gitignore`）改名停用，返回新的相对路径（设计 §5）。
///
/// # 绝不覆盖
///
/// Windows 上 `std::fs::rename` 会**替换**已存在的目标（`MoveFileExW` 带
/// `MOVEFILE_REPLACE_EXISTING`），「先判存在再改名」中间又有窗口。所以先用
/// `create_new` 原子地占住一个候选名——已存在就试下一个——再把文件改名到这个
/// **自己刚建的**占位上。被替换掉的只可能是那个空占位。
pub fn disable_gitignore(repo_root: &str, rel: &str) -> Result<String> {
    let root = Path::new(repo_root);
    let src = root.join(rel);
    let base = format!("{rel}.jkk-disabled");
    for n in 1..=MAX_DISABLED_SUFFIX {
        let cand_rel = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let cand = root.join(&cand_rel);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&cand)
        {
            Ok(placeholder) => {
                drop(placeholder);
                return match std::fs::rename(&src, &cand) {
                    Ok(()) => Ok(cand_rel),
                    Err(e) => {
                        let _ = std::fs::remove_file(&cand);
                        Err(GitError::new(
                            "svn_gitignore_rename",
                            format!(
                                "没能把 {} 改名为 {}：{e}",
                                display_path(repo_root, rel),
                                display_path(repo_root, &cand_rel)
                            ),
                        ))
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(GitError::new(
        "svn_gitignore_rename",
        format!(
            "{} 旁边已经有 {MAX_DISABLED_SUFFIX} 个停用文件，找不到可用的新名字。请先清理一下。",
            display_path(repo_root, rel)
        ),
    ))
}

/// 把改过的名逐个改回去，返回改不回来的那些的说明（空 = 全部改回）。
///
/// 原位置**又出现了一个文件**时不覆盖它（Review Focus 第 5 条）：那可能是用户或 SVN
/// 更新刚放进来的。两个文件都留着，点名让用户处理。倒序改回，与改名顺序相反。
///
/// # 为什么不是「判存在再 rename」
///
/// 与 [`disable_gitignore`] 同一个理由：Windows 上 `fs::rename` 会替换已存在的目标，判完
/// 之后、改名之前冒出来的文件会被静默盖掉（/code-review 第 3 条）。`hard_link` 在目标已存在
/// 时原子地失败，所以先链接到原名、再删停用名。卷不支持硬链接（FAT / exFAT）时退回
/// 「判存在再 rename」——那里没有原子的不覆盖改名可用，窗口只剩这一种卷。
pub fn restore_gitignores(repo_root: &str, renamed: &[DisabledGitignore]) -> Vec<String> {
    let root = Path::new(repo_root);
    let mut failures = Vec::new();
    for r in renamed.iter().rev() {
        let from = root.join(&r.from);
        let to = root.join(&r.to);
        let (from_full, to_full) = (
            display_path(repo_root, &r.from),
            display_path(repo_root, &r.to),
        );
        let reappeared = format!("{from_full} 又出现了一个新文件，{to_full} 没有改回去");
        if from.exists() {
            failures.push(reappeared);
            continue;
        }
        match std::fs::hard_link(&to, &from) {
            Ok(()) => {
                // 原名已经回来了；停用名删不掉只是多留一份副本，照样点名
                if let Err(e) = std::fs::remove_file(&to) {
                    failures.push(format!("{from_full} 已改回，但 {to_full} 没删掉：{e}"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => failures.push(reappeared),
            Err(_) => {
                if let Err(e) = std::fs::rename(&to, &from) {
                    failures.push(format!("{to_full} → {from_full}：{e}"));
                }
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::ProbeGitDir;

    #[test]
    fn the_probe_directory_is_removed_on_drop() {
        let wt = tempfile::TempDir::new().unwrap();
        let probe = ProbeGitDir::create(
            wt.path().to_str().unwrap(),
            &["Code/".to_string()],
            &[".cs".to_string()],
        )
        .unwrap();
        let dir = probe.dir.clone();
        assert!(dir.join(".git").join("info").join("exclude").is_file());
        drop(probe);
        assert!(!dir.exists(), "探针目录没删干净：{}", dir.display());
        assert!(!wt.path().join(".git").exists(), "工作副本里不能出现 .git");
    }
}
