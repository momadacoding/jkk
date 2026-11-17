//! 只读的 git 查询，走 libgit2（C1a 决策日志「性能 P3」）。
//!
//! 项目约定「读走 libgit2、写走 git CLI」，SVN 同步 / 跟进这一路原先连查一个 ref 都起 git 子进程：
//! 一次跟进一百多个，Windows 上每个约 45 ms，起进程的开销占了跟进耗时的大头（2026-11-01 实测）。
//!
//! 这里只收与 CLI 一一对应的查询。`diff-tree` 的重命名检测、有 merge 时 `log` / `rev-list` 的
//! 排序、`worktree list` 的 prunable 判定两边语义有差，不收；跟进关键路径上的读只在更窄的条件下
//! 收（不带重命名、只接线性区间、只读分支名，见下半部分与
//! `docs/superpowers/plans/2026-11-16-follow-git-inproc-reads-decisions.md`）。
//!
//! # 「不存在」与「坏了」要分开
//!
//! [`resolve`] 只把 ref / 路径层面的 `NotFound` / `InvalidSpec` / `Ambiguous` 当成「没有」；
//! ref 文件损坏、ref 指向的对象缺失都返回 `Err`。把损坏当成不存在会让调用方以为基线还没建，
//! 进而覆盖掉一段真实历史（`svn_baseline::read_baseline`）。
//!
//! 这两种损坏上 CLI 并不报错：内容是垃圾的 ref，`rev-parse -q --verify` 警告后退出 1（算「没有」）；
//! 悬空 ref 退出 0 照样打印 OID。这里有意比 CLI 严（`tests/git_read.rs`，独立 review 2026-11-01）。

use crate::error::Result;
use git2::{ErrorClass, ErrorCode, ObjectType, Patch, Repository, TreeWalkMode, TreeWalkResult};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn open(repo: &str) -> Result<Repository> {
    Ok(Repository::open(repo)?)
}

fn absent(e: &git2::Error) -> bool {
    // ref 在、对象没了（悬空 ref）也报 NotFound，但类别是 Odb：那是对象库坏了，不是「没有」
    matches!(
        e.code(),
        ErrorCode::NotFound | ErrorCode::InvalidSpec | ErrorCode::Ambiguous
    ) && e.class() != ErrorClass::Odb
}

/// `git rev-parse --git-dir`，绝对路径。linked worktree 里是 `.git/worktrees/<名字>`，与 CLI 相同。
pub fn git_dir(repo: &str) -> Result<PathBuf> {
    let r = open(repo)?;
    // libgit2 给的是带尾斜杠的正斜杠路径；去掉尾斜杠，后面 join 出来的文件名才干净
    let p = r.path().to_string_lossy();
    Ok(PathBuf::from(p.trim_end_matches(['/', '\\'])))
}

/// `git rev-parse -q --verify <spec>` 的完整 OID；解析不出来时 `None`。支持 `X^{commit}`、
/// `X^1`、`X:path` 这些写法（libgit2 的 revparse 与 CLI 同一套语法）。
pub fn resolve(repo: &str, spec: &str) -> Result<Option<String>> {
    match open(repo)?.revparse_single(spec) {
        Ok(o) => Ok(Some(o.id().to_string())),
        Err(e) if absent(&e) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `git symbolic-ref -q HEAD`：HEAD 指向的 ref 全名；游离 HEAD 时 `None`。
pub fn symbolic_head(repo: &str) -> Result<Option<String>> {
    let r = open(repo)?;
    let head = r.find_reference("HEAD")?;
    Ok(head.symbolic_target().map(str::to_string))
}

/// commit 说明的原始字节（`cat-file commit` 里空行之后的部分），不做任何清理。
pub fn commit_message(repo: &str, rev: &str) -> Result<Vec<u8>> {
    let r = open(repo)?;
    let c = r.revparse_single(rev)?.peel_to_commit()?;
    Ok(c.message_raw_bytes().to_vec())
}

/// `git merge-base <a> <b>`；没有共同祖先时 `None`。
pub fn merge_base(repo: &str, a: &str, b: &str) -> Result<Option<String>> {
    let r = open(repo)?;
    let a = r.revparse_single(a)?.peel_to_commit()?.id();
    let b = r.revparse_single(b)?.peel_to_commit()?.id();
    match r.merge_base(a, b) {
        Ok(o) => Ok(Some(o.to_string())),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `git merge-base --is-ancestor <a> <b>`：`a` 是 `b` 本身或其祖先。
pub fn is_ancestor(repo: &str, a: &str, b: &str) -> Result<bool> {
    let r = open(repo)?;
    let a = r.revparse_single(a)?.peel_to_commit()?.id();
    let b = r.revparse_single(b)?.peel_to_commit()?.id();
    Ok(a == b || r.graph_descendant_of(b, a)?)
}

/// `git ls-tree -r --name-only <rev>`：树里所有非目录条目（文件、符号链接、子模块指针）的路径。
/// 非 UTF-8 的名字按 lossy 处理，与原来 `run_git` 的输出一致。
pub fn tree_paths(repo: &str, rev: &str) -> Result<BTreeSet<String>> {
    let r = open(repo)?;
    let tree = r.revparse_single(rev)?.peel_to_tree()?;
    let mut out = BTreeSet::new();
    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() != Some(ObjectType::Tree) {
            let name = String::from_utf8_lossy(entry.name_bytes());
            out.insert(format!("{dir}{name}"));
        }
        TreeWalkResult::Ok
    })?;
    Ok(out)
}

/// 索引里未合并（冲突态）的路径，即 `git diff --name-only --diff-filter=U`。
pub fn unmerged_paths(repo: &str) -> Result<Vec<String>> {
    let r = open(repo)?;
    let idx = r.index()?;
    let mut out = BTreeSet::new();
    for c in idx.conflicts()? {
        let c = c?;
        if let Some(e) = c.our.or(c.their).or(c.ancestor) {
            out.insert(String::from_utf8_lossy(&e.path).into_owned());
        }
    }
    Ok(out.into_iter().collect())
}

/// 一条路径在两个提交之间的增删行数，同 `git diff --numstat` 的一行。二进制时行数为 0、`binary` 为真
/// （numstat 写 `-`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineStat {
    pub added: usize,
    pub removed: usize,
    pub binary: bool,
}

/// 默认配置下的 `git diff --numstat --no-renames <from> <to> -- <paths>`：只回答问到的路径，两边内容
/// 相同或两边都没有的不出现。比的是对象库里的 blob，不经过 autocrlf / eol 过滤；二进制按
/// `.gitattributes`（`-diff`、`binary`）与内容判定（`tests/git_read_line_stats.rs` 逐个对照）。
///
/// 与 numstat 不同的地方（跟进报告碰不到或不在乎）：只差 mode 的路径不出现（numstat 写 `0 0`，
/// `drift_paths` 本来就不报它）；固定 myers，不认用户的 `diff.algorithm`；不跑 textconv；不按
/// `core.bigFileThreshold` 把大文件当二进制。
///
/// 跟进报告用它写「分支比 SVN 少 7 行」（计划 `2026-11-16-follow-report-usability.md` 的 B）：drift
/// 的文件磁盘就是新基线的 blob，所以分支相对磁盘差几行 = 新基线对重放后的 tip，不用读磁盘。
///
/// 某一条路径读不出来（对象缺失之类）只跳过它、记日志，其余照常回答：这是给人看的附注，一个坏对象
/// 不该让整份报告的行数都没了（独立 review nit 4）。打不开仓库、解析不了两个提交才返回 `Err`。
pub fn line_stats(
    repo: &str,
    from: &str,
    to: &str,
    paths: &[String],
) -> Result<BTreeMap<String, LineStat>> {
    let mut out = BTreeMap::new();
    if paths.is_empty() {
        return Ok(out);
    }
    let r = open(repo)?;
    let a = r.revparse_single(from)?.peel_to_tree()?;
    let b = r.revparse_single(to)?.peel_to_tree()?;
    for p in paths {
        match line_stat(&r, &a, &b, p) {
            Ok(Some(s)) => {
                out.insert(p.clone(), s);
            }
            Ok(None) => {}
            Err(e) => log::warn!("统计 {p} 差几行时出错，跳过：{}", e.message),
        }
    }
    Ok(out)
}

/// [`line_stats`] 的一条路径。两边相同或两边都没有时 `None`。
fn line_stat(r: &Repository, a: &git2::Tree, b: &git2::Tree, p: &str) -> Result<Option<LineStat>> {
    let at = Some(Path::new(p));
    // git2 的 Patch 没有「一边没有 blob」的入口：只有一边时拿它对空内容比。新增的文件反着比再把
    // 增删对调，这样两种情形都由仓库里的 blob 带路径，`.gitattributes` 的二进制判定照样生效
    let (patch, swap) = match (blob_at(r, a, p)?, blob_at(r, b, p)?) {
        (None, None) => return Ok(None),
        (Some(o), Some(n)) if o.id() == n.id() => return Ok(None),
        (Some(o), Some(n)) => (Patch::from_blobs(&o, at, &n, at, None)?, false),
        (Some(o), None) => (Patch::from_blob_and_buffer(&o, at, b"", at, None)?, false),
        (None, Some(n)) => (Patch::from_blob_and_buffer(&n, at, b"", at, None)?, true),
    };
    let binary = patch.delta().flags().is_binary();
    let (_, plus, minus) = patch.line_stats()?;
    let (added, removed) = if swap { (minus, plus) } else { (plus, minus) };
    Ok(Some(LineStat {
        added,
        removed,
        binary,
    }))
}

/// 树里 `p` 处的 blob。子模块指针、目录算没有：numstat 对子模块写的是提交号的变化，跟进报告里不会有它们。
fn blob_at<'r>(r: &'r Repository, t: &git2::Tree, p: &str) -> Result<Option<git2::Blob<'r>>> {
    match t.get_path(Path::new(p)) {
        Ok(e) if e.kind() == Some(ObjectType::Blob) => Ok(Some(r.find_blob(e.id())?)),
        Ok(_) => Ok(None),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `git config --bool --get core.ignoreCase`；没设或读不出来都按 `false`。
pub fn ignore_case(repo: &str) -> bool {
    open(repo)
        .and_then(|r| Ok(r.config()?.get_bool("core.ignoreCase").unwrap_or(false)))
        .unwrap_or(false)
}

// —— 跟进关键路径上的读（日志 `2026-11-16-follow-git-inproc-reads-decisions.md`）——
//
// 每个函数对应调用点原来的一条 git 命令，`tests/git_read_follow.rs` 逐个场景与那条命令比。
// 认不出来的形状返回 `Err`，调用方经 [`or_cli`] 退回原来的命令。

/// 先走 libgit2，失败时退回原来的 git 命令。`fast` 只在它确定与 CLI 等价时返回 `Ok`。
pub fn or_cli<T>(
    what: &str,
    fast: impl FnOnce() -> Result<T>,
    slow: impl FnOnce() -> Result<T>,
) -> Result<T> {
    // 性能记录里与子进程并列，看得出进程内的读花了多少（`kind = "inproc"`）
    let started = crate::engine::perf::enabled().then(crate::engine::perf::now_ms);
    let got = fast();
    if let Some(ts) = started {
        let mut rec = crate::engine::perf::Record::rust("inproc", what);
        rec.ts = ts;
        rec.dur = crate::engine::perf::now_ms() - ts;
        rec.ok = got.is_ok();
        crate::engine::perf::record(&rec);
    }
    match got {
        Ok(v) => Ok(v),
        Err(e) => {
            log::info!("{what} 改用 git 命令：{}", e.message);
            slow()
        }
    }
}

fn unsupported(what: impl Into<String>) -> crate::error::GitError {
    crate::error::GitError::new("git_read_unsupported", what)
}

/// 仓库用 SHA-1 对象名。SHA-256 仓库（`extensions.objectFormat = sha256`）这里一律不接。
fn sha1_repo(r: &Repository) -> Result<()> {
    match r.config()?.get_string("extensions.objectformat") {
        Ok(f) if !f.eq_ignore_ascii_case("sha1") => Err(unsupported(format!("对象格式是 {f}"))),
        _ => Ok(()),
    }
}

/// 仓库里没有替换对象。git 的 `log` / `diff-tree` / `cat-file` 默认按 `refs/replace/` 把对象换掉，
/// libgit2 不换：有替换时两边答案不同，交回 git。换了替换 ref 的位置（`GIT_REPLACE_REF_BASE`）
/// 时同样不接。`info/grafts`（旧式的替换父提交）与 `shallow`（浅克隆的截断点）也会改变 git
/// 看到的历史，两边是否一致没有核实过，有就交回 git（独立 review 第 1 条）。
fn no_replace(r: &Repository) -> Result<()> {
    if std::env::var_os("GIT_REPLACE_REF_BASE").is_some() {
        return Err(unsupported("设了 GIT_REPLACE_REF_BASE"));
    }
    if r.references_glob("refs/replace/*")?.next().is_some() {
        return Err(unsupported("仓库里有 refs/replace/ 替换对象"));
    }
    for f in ["info/grafts", "shallow"] {
        if r.commondir().join(f).exists() || r.path().join(f).exists() {
            return Err(unsupported(format!("仓库里有 {f}")));
        }
    }
    Ok(())
}

/// `git hash-object --no-filters --stdin`：这段字节的 blob OID，不写对象库、不过任何过滤器。
pub fn blob_oid(repo: &str, bytes: &[u8]) -> Result<String> {
    sha1_repo(&open(repo)?)?;
    Ok(git2::Oid::hash_object(ObjectType::Blob, bytes)?.to_string())
}

/// `git cat-file --batch-check` 里类型是 blob 的那些 OID（对象库里确实有的 blob）。
/// 不存在、不是 blob、写法不是 OID 的都不在结果里，与 CLI 打 `missing` 一样。
pub fn present_blobs<'a>(
    repo: &str,
    oids: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<String>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    no_replace(&r)?;
    let odb = r.odb()?;
    let mut out = BTreeSet::new();
    for s in oids {
        let Ok(oid) = git2::Oid::from_str(s) else {
            continue;
        };
        // 短写法 from_str 也收（补零），CLI 却按前缀找：只收完整的 40 位
        if s.len() != 40 {
            continue;
        }
        match odb.read_header(oid) {
            Ok((_, ObjectType::Blob)) => {
                out.insert(oid.to_string());
            }
            Ok(_) => {}
            Err(e) if e.code() == ErrorCode::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// `git for-each-ref --format='%(refname) %(objectname) %(upstream)' refs/heads/`：
/// `(分支名, OID, 有没有 upstream)`，按 ref 名排序（for-each-ref 的默认顺序）。
///
/// upstream 按配置算（`branch.<名字>.remote` / `.merge` 加上 remote 的 fetch refspec），与 CLI
/// 相同：远端分支还没取回也算有，remote 没定义算没有（都有测试）。libgit2 报 NotFound 以外的错
/// 时算「有」：调用方见了 upstream 就不移动这条分支，说宽了只是少移一条。
pub fn local_branches(repo: &str) -> Result<Vec<(String, String, bool)>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    let mut out = Vec::new();
    // 只列 refs/heads/ 底下的（libgit2 的 `*` 跨 `/`，`feature/x` 也在，有测试）：别处的 ref
    // 再多、名字再怪都不碰
    for reference in r.references_glob("refs/heads/*")? {
        let reference = reference?;
        let Some(full) = reference.name() else {
            return Err(unsupported("有名字不是 UTF-8 的 ref"));
        };
        let Some(name) = full.strip_prefix("refs/heads/") else {
            continue;
        };
        let oid = reference
            .resolve()?
            .target()
            .ok_or_else(|| unsupported(full))?;
        let upstream = match r.branch_upstream_name(full) {
            Ok(_) => true,
            Err(e) if e.code() == ErrorCode::NotFound => false,
            Err(_) => true,
        };
        out.push((name.to_string(), oid.to_string(), upstream));
    }
    // for-each-ref 按 refname 的字节序
    out.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    Ok(out)
}

/// `git for-each-ref --sort=-refname --sort=-committerdate --format=%(refname) <prefix>`：
/// 提交时间新的在前，同一秒的按 ref 名倒序（最后一个 `--sort` 是主键）。指着的不是 commit 时不接。
pub fn refs_newest_first(repo: &str, prefix: &str) -> Result<Vec<String>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    // for-each-ref 的 committerdate 读的是替换之后的 commit
    no_replace(&r)?;
    let mut refs = Vec::new();
    for reference in r.references_glob(&format!("{prefix}*"))? {
        let reference = reference?;
        let Some(name) = reference.name() else {
            return Err(unsupported("有名字不是 UTF-8 的 ref"));
        };
        if !name.starts_with(prefix) {
            continue;
        }
        let Ok(commit) = reference.peel_to_commit() else {
            return Err(unsupported(format!("{name} 指着的不是 commit")));
        };
        refs.push((commit.committer().when().seconds(), name.to_string()));
    }
    refs.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.as_bytes().cmp(a.1.as_bytes()))
    });
    Ok(refs.into_iter().map(|(_, n)| n).collect())
}

/// `git diff-tree -r --no-renames` raw 输出里的一条。mode 是 6 位八进制、OID 是完整 40 位，
/// 没有的那一侧是 `000000` 与全零，与 CLI 打印的一字不差。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChange {
    pub old_mode: String,
    pub new_mode: String,
    pub old_oid: String,
    pub new_oid: String,
    /// `A` / `D` / `M` / `T`。
    pub status: String,
    pub path: String,
}

/// `git diff-tree -r -z --no-renames <a> <b>`：两棵树（或两个 commit 的树）之间每个变了的非目录
/// 条目，按 CLI 输出的顺序。
///
/// 不做重命名检测（与 `--no-renames` 相同）；文件 ↔ 符号链接是一条 `T`（CLI 同样）；文件 ↔ 目录
/// 是删一条、加里面的每一条。路径不是 UTF-8、变化涉及 gitlink 时不接。
///
/// **不用 libgit2 的 `diff_tree_to_tree`**：实测它把两棵树的每个条目都走一遍，2 万文件的树一次
/// 64–150 ms，比起一个 git 进程还慢。这里照 git 的做法逐层比，OID 相同的子树整棵跳过，跟进时
/// 两个基线之间通常只差几个目录。
pub fn tree_changes(repo: &str, a: &str, b: &str) -> Result<Vec<RawChange>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    no_replace(&r)?;
    let old = r.revparse_single(a)?.peel_to_tree()?;
    let new = r.revparse_single(b)?.peel_to_tree()?;
    let mut out = Vec::new();
    diff_trees(&r, Some(&old), Some(&new), "", &mut out)?;
    // CLI 按树的遍历顺序输出：目录按「名字/」比较。全路径按字节比与它相同（`/` 在比较里就是
    // 0x2F；文件 ↔ 目录时两条路径是 `a` 与 `a/…`，本来就不相等）
    out.sort_by(|x, y| x.path.as_bytes().cmp(y.path.as_bytes()));
    Ok(out)
}

/// gitlink（子模块指针）的 mode。子模块的 `ignore` 设置会影响 git 的 diff，两边不一定一致，
/// 碰到就交回 git。
const GITLINK: u32 = 0o160000;
const TREE: u32 = 0o040000;

/// 树里的一条：名字、mode、OID。
struct Item {
    name: Vec<u8>,
    mode: u32,
    oid: git2::Oid,
}

impl Item {
    fn is_tree(&self) -> bool {
        self.mode == TREE
    }

    /// git 给树条目排序的键：目录按「名字/」比（`tree.c` 的 `base_name_compare`）。
    fn key(&self) -> Vec<u8> {
        let mut k = self.name.clone();
        if self.is_tree() {
            k.push(b'/');
        }
        k
    }
}

fn items(t: Option<&git2::Tree>) -> Vec<Item> {
    t.map(|t| {
        t.iter()
            .map(|e| Item {
                name: e.name_bytes().to_vec(),
                mode: e.filemode() as u32,
                oid: e.id(),
            })
            .collect()
    })
    .unwrap_or_default()
}

fn diff_trees(
    r: &Repository,
    old: Option<&git2::Tree>,
    new: Option<&git2::Tree>,
    prefix: &str,
    out: &mut Vec<RawChange>,
) -> Result<()> {
    if let (Some(a), Some(b)) = (old, new) {
        if a.id() == b.id() {
            return Ok(());
        }
    }
    let (a, b) = (items(old), items(new));
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let order = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) => x.key().cmp(&y.key()),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        match order {
            std::cmp::Ordering::Equal => {
                change(r, Some(&a[i]), Some(&b[j]), prefix, out)?;
                i += 1;
                j += 1;
            }
            std::cmp::Ordering::Less => {
                change(r, Some(&a[i]), None, prefix, out)?;
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                change(r, None, Some(&b[j]), prefix, out)?;
                j += 1;
            }
        }
    }
    Ok(())
}

/// 同一个排序键下的一对条目（某一边可能没有）。两边都是树时递归；只有一边时整棵展开成 A / D。
fn change(
    r: &Repository,
    old: Option<&Item>,
    new: Option<&Item>,
    prefix: &str,
    out: &mut Vec<RawChange>,
) -> Result<()> {
    let item = old.or(new).ok_or_else(|| unsupported("空的树条目"))?;
    let name = std::str::from_utf8(&item.name).map_err(|_| unsupported("路径不是 UTF-8"))?;
    let path = format!("{prefix}{name}");
    if old.is_some_and(|o| new.is_some_and(|n| o.mode == n.mode && o.oid == n.oid)) {
        return Ok(());
    }
    if [old, new].iter().flatten().any(|e| e.mode == GITLINK) {
        return Err(unsupported("有子模块指针"));
    }
    // 排序键相同就说明两边同是树、或同不是树
    if item.is_tree() {
        let tree = |e: Option<&Item>| e.map(|e| r.find_tree(e.oid)).transpose();
        let (a, b) = (tree(old)?, tree(new)?);
        return diff_trees(r, a.as_ref(), b.as_ref(), &format!("{path}/"), out);
    }
    let kind = |m: u32| m & 0o170000;
    let status = match (old, new) {
        (None, _) => "A",
        (_, None) => "D",
        (Some(o), Some(n)) if kind(o.mode) != kind(n.mode) => "T",
        _ => "M",
    };
    let zero = git2::Oid::zero();
    let side = |e: Option<&Item>| e.map_or((0, zero), |e| (e.mode, e.oid));
    let ((om, oo), (nm, no)) = (side(old), side(new));
    out.push(RawChange {
        old_mode: format!("{om:06o}"),
        new_mode: format!("{nm:06o}"),
        old_oid: oo.to_string(),
        new_oid: no.to_string(),
        status: status.to_string(),
        path,
    });
    Ok(())
}

/// 索引里的一条（`git ls-files --stage` 的一行，外加两个标志）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRecord {
    pub mode: u32,
    pub oid: String,
    pub stage: u8,
    pub path: String,
    pub skip_worktree: bool,
    pub assume_unchanged: bool,
}

// libgit2 `git_index_entry` 的标志位（include/git2/index.h）
const ENTRY_STAGE_SHIFT: u16 = 12;
const ENTRY_VALID: u16 = 0x8000;
const ENTRY_EXT_SKIP_WORKTREE: u16 = 1 << 14;
/// sparse index 里代表整个目录的条目。
const SPARSE_DIR: u32 = 0o040000;

/// 读 `index_file`（等于 `GIT_INDEX_FILE` 指向它时 git 读的那份）。文件不存在时与 git 一样当成
/// 空索引。
///
/// libgit2 不支持的形状一律不接，交回 git：split index（`core.splitIndex`、或 git 目录里有
/// `sharedindex.*`；libgit2 打开时报 `unsupported mandatory extension: 'link'`，这里先拦住，
/// 不靠它报错）、sparse index（`index.sparse`、或出现目录条目：`sdir` 是可选扩展，libgit2 会
/// 读进来一条 mode 040000 的「目录条目」）、路径不是 UTF-8。
pub fn index_records(repo: &str, index_file: &std::path::Path) -> Result<Vec<IndexRecord>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    let cfg = r.config()?;
    for key in ["core.splitindex", "index.sparse"] {
        if cfg.get_bool(key).unwrap_or(false) {
            return Err(unsupported(format!("{key} 打开了")));
        }
    }
    for dir in [r.path(), r.commondir()] {
        for f in std::fs::read_dir(dir)? {
            if f?.file_name().to_string_lossy().starts_with("sharedindex.") {
                return Err(unsupported("有 split index 的 sharedindex 文件"));
            }
        }
    }
    if !index_file.exists() {
        return Ok(Vec::new());
    }
    let index = git2::Index::open(index_file)?;
    let mut out = Vec::with_capacity(index.len());
    for e in index.iter() {
        if e.mode == SPARSE_DIR {
            return Err(unsupported("索引里有目录条目（sparse index）"));
        }
        let path = String::from_utf8(e.path).map_err(|_| unsupported("路径不是 UTF-8"))?;
        out.push(IndexRecord {
            mode: e.mode,
            oid: e.id.to_string(),
            stage: ((e.flags >> ENTRY_STAGE_SHIFT) & 0x3) as u8,
            path,
            skip_worktree: e.flags_extended & ENTRY_EXT_SKIP_WORKTREE != 0,
            assume_unchanged: e.flags & ENTRY_VALID != 0,
        });
    }
    Ok(out)
}

/// 有没有未合并（stage 非 0）的条目，即 `git ls-files -u` 有没有输出。
pub fn has_unmerged(records: &[IndexRecord]) -> bool {
    records.iter().any(|e| e.stage != 0)
}

fn only_stage_zero(records: &[IndexRecord]) -> Result<()> {
    if has_unmerged(records) {
        return Err(unsupported("索引里有未合并的条目"));
    }
    Ok(())
}

/// `git ls-files -v -z`，逐字节相同：`H`（普通）/ `S`（skip-worktree），assume-unchanged 时小写
/// （`ls-files.c` 的 `get_tag`）。有未合并条目时不接（那时是 `M`，还有去重的讲究）。
pub fn ls_files_v(records: &[IndexRecord]) -> Result<String> {
    only_stage_zero(records)?;
    let mut out = String::new();
    for e in records {
        let tag = match (e.skip_worktree, e.assume_unchanged) {
            (true, false) => "S",
            (true, true) => "s",
            (false, false) => "H",
            (false, true) => "h",
        };
        out.push_str(tag);
        out.push(' ');
        out.push_str(&e.path);
        out.push('\0');
    }
    Ok(out)
}

/// `git ls-files --stage -z`，逐字节相同（`<mode> <oid> <stage>\t<path>\0`）。跟进把它的哈希记进
/// journal、恢复时再比，所以必须与 CLI 一字不差。有未合并条目时不接。
pub fn ls_files_stage(records: &[IndexRecord]) -> Result<String> {
    only_stage_zero(records)?;
    Ok(records
        .iter()
        .map(|e| format!("{:06o} {} {}\t{}\0", e.mode, e.oid, e.stage, e.path))
        .collect())
}

/// `git ls-files -z`，逐字节相同。有未合并条目时不接。
pub fn ls_files(records: &[IndexRecord]) -> Result<String> {
    only_stage_zero(records)?;
    Ok(records.iter().map(|e| format!("{}\0", e.path)).collect())
}

/// `git log --format=%H%x00%P%x00%s%x00%B` 的一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogCommit {
    pub oid: String,
    pub parents: Vec<String>,
    /// `%s`：说明开头的空白行跳过，第一段的各行去掉行尾空白、用空格连起来（git `pretty.c`
    /// 的 `format_subject`）。
    pub subject: String,
    /// `%B`：说明原样。
    pub body: String,
}

/// `git log` 输出时按 `i18n.logOutputEncoding`（没设时取 `i18n.commitEncoding`）转码。不是 UTF-8 时
/// 两边给出的字节不同，交回 git（独立 review 第 3 条）。
fn utf8_log_output(r: &Repository) -> Result<()> {
    let cfg = r.config()?;
    for key in ["i18n.logoutputencoding", "i18n.commitencoding"] {
        if let Ok(enc) = cfg.get_string(key) {
            if !enc.eq_ignore_ascii_case("utf-8") && !enc.eq_ignore_ascii_case("utf8") {
                return Err(unsupported(format!("{key} 是 {enc}")));
            }
        }
    }
    Ok(())
}

/// 说明按 UTF-8 原样取。带 `encoding` 头（CLI 会转码）或不是合法 UTF-8 的不接。
fn utf8_message(c: &git2::Commit) -> Result<String> {
    if let Some(enc) = c.message_encoding() {
        if !enc.eq_ignore_ascii_case("utf-8") && !enc.eq_ignore_ascii_case("utf8") {
            return Err(unsupported(format!("{} 的说明编码是 {enc}", c.id())));
        }
    }
    std::str::from_utf8(c.message_raw_bytes())
        .map(str::to_string)
        .map_err(|_| unsupported(format!("{} 的说明不是 UTF-8", c.id())))
}

/// git 的 `isspace`（`sane_ctype` 里的 GIT_SPACE）：空格、`\t`、`\n`、`\r`。
fn git_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// `pretty.c` 的 `%s`：跳过开头只有空白的行，取第一段，每行去掉行尾空白，用空格连起来。
fn subject_of(msg: &str) -> String {
    let mut lines = msg.split_inclusive('\n').peekable();
    while lines.peek().is_some_and(|l| l.bytes().all(git_space)) {
        lines.next();
    }
    let mut parts = Vec::new();
    for line in lines {
        let trimmed = line.trim_end_matches(|c: char| c.is_ascii() && git_space(c as u8));
        if trimmed.is_empty() {
            break;
        }
        parts.push(trimmed);
    }
    parts.join(" ")
}

fn walk_range<'r>(r: &'r Repository, base: &str, head: &str) -> Result<git2::Revwalk<'r>> {
    let mut walk = r.revwalk()?;
    walk.push(r.revparse_single(head)?.peel_to_commit()?.id())?;
    walk.hide(r.revparse_single(base)?.peel_to_commit()?.id())?;
    Ok(walk)
}

/// `git log --reverse --topo-order --format=%H%x00%P%x00%s%x00%B <base>..<head>`，只接**线性**的
/// 区间：区间里有 merge 时 topo-order 的排法两边可能不同，交回 CLI。
pub fn linear_commits(repo: &str, base: &str, head: &str) -> Result<Vec<LogCommit>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    no_replace(&r)?;
    utf8_log_output(&r)?;
    let mut out = Vec::new();
    for oid in walk_range(&r, base, head)? {
        let c = r.find_commit(oid?)?;
        if c.parent_count() > 1 {
            return Err(unsupported(format!("区间里有 merge {}", c.id())));
        }
        let body = utf8_message(&c)?;
        out.push(LogCommit {
            oid: c.id().to_string(),
            parents: c.parent_ids().map(|p| p.to_string()).collect(),
            subject: subject_of(&body),
            body,
        });
    }
    // 线性区间里每个 commit 的父提交就是上一个：倒过来就是从旧到新
    out.reverse();
    if out.windows(2).any(|w| w[1].parents != [w[0].oid.clone()]) {
        return Err(unsupported("区间不是一条链"));
    }
    Ok(out)
}

/// `git log --format=%B <base>..<head>` 里每个 commit 的说明（顺序不保证）。merge 也算。
pub fn bodies(repo: &str, base: &str, head: &str) -> Result<Vec<String>> {
    let r = open(repo)?;
    sha1_repo(&r)?;
    no_replace(&r)?;
    utf8_log_output(&r)?;
    let mut out = Vec::new();
    for oid in walk_range(&r, base, head)? {
        out.push(utf8_message(&r.find_commit(oid?)?)?);
    }
    Ok(out)
}

/// 一个 HEAD 文件指着哪条分支：`Some(Some(名字))` 是 `refs/heads/` 下的分支，`Some(None)` 是游离
/// HEAD（40 位 OID）或指着别处的 ref，`None` 是读不懂。
///
/// git 认 `ref:` 后面任意多的空白（含没有，`refs.c` 的 `parse_loose_ref_contents`）。读不懂的
/// 不能当成游离：那会漏报一条已检出的分支，跟进就可能去移动它（独立 review 第 2 条）。
fn head_branch(text: &str) -> Option<Option<String>> {
    let line = text.strip_suffix('\n').unwrap_or(text);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if let Some(target) = line.strip_prefix("ref:") {
        let target = target.trim_start_matches([' ', '\t']);
        if target.is_empty() || target.contains(char::is_whitespace) {
            return None;
        }
        return Some(target.strip_prefix("refs/heads/").map(str::to_string));
    }
    let detached = line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit());
    detached.then_some(None)
}

/// `git worktree list --porcelain` 里各个 `branch refs/heads/<名字>`：主工作区与每个 linked
/// worktree 的 HEAD 指着的分支。
///
/// 直接读 `<commondir>/HEAD` 与 `<commondir>/worktrees/*/HEAD`，与 CLI 读的是同一批文件。目录已
/// 删、还没 prune 的 worktree 照样算（CLI 也列出它）。**有意的分歧**：主仓库是 bare 时 CLI 不报
/// 主工作区的分支，这里照报；调用方见了就不移动那条分支，说宽了只是少移一条。
pub fn worktree_branches(repo: &str) -> Result<BTreeSet<String>> {
    let r = open(repo)?;
    let common = r.commondir().to_path_buf();
    let mut heads = vec![common.join("HEAD")];
    match std::fs::read_dir(common.join("worktrees")) {
        Ok(dirs) => {
            for d in dirs {
                let d = d?;
                if d.file_type()?.is_dir() {
                    heads.push(d.path().join("HEAD"));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut out = BTreeSet::new();
    for head in heads {
        // 符号链接形式的 HEAD（很老的 git 写法）git 会跟过去读，这里不跟：交回 git
        if std::fs::symlink_metadata(&head).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(unsupported(format!("{} 是符号链接", head.display())));
        }
        let text = match std::fs::read_to_string(&head) {
            Ok(t) => t,
            // 半建好的 worktree 目录里还没有 HEAD：CLI 也不报它的分支
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        match head_branch(&text) {
            Some(Some(branch)) => {
                out.insert(branch);
            }
            Some(None) => {}
            None => return Err(unsupported(format!("读不懂 {}", head.display()))),
        }
    }
    Ok(out)
}
