//! 对象层重放引擎：交付（D0.5 摘 Local-Only）与跟进（分析 §6.5 F2、§6.11.3）共用。
//!
//! # 为什么在对象层重放
//!
//! `git merge-tree --write-tree --merge-base=<C^> <cur> <C>` 与 cherry-pick 用同一套合并
//! 引擎（ort），但只产出对象：磁盘、索引、HEAD、worktree 注册一样都不碰（W10 / W11）。
//! 交付的预览、跟进的准备阶段都不拿锁，所以重放不能留下任何需要事后清理的东西——
//! 没有任何 ref 指着重放结果，直到调用方自己去 CAS。
//!
//! # 跟进比交付多出来的两个开关
//!
//! - **丢弃变空（`drop_empty`）：** 结果树与**当前父提交**（上一个重放结果）的树相同就丢弃
//!   （W18：与 onto 的树比，两个方向都会错）。原本就空的 commit 保留，与 git rebase 默认一致。
//! - **自己提交的路径取 SVN 版本（`superseded`）：** 对 C 给出路径集合 P，把 C 的树里 P 的
//!   条目换成 C^ 的条目得到 C'，再重放 C'（W19）。merge-tree 对 P 看到的是「theirs 没改」，
//!   直接取 onto 的版本，W10-R2 不再冲突，W14 不再出现重复内容。
//!
//! 另有「已全部提交」捷径 [`all_committed`]（W20），先于逐个重放判断，不要求线性。
//!
//! 代价：`--merge-base` 要 git ≥ 2.40，旧 git 报调用方给的 `too_old` 码。

use crate::engine::git_cli::{run_git, run_git_code, run_git_env, run_git_stdin_env};
use crate::engine::graph::parse_local_only;
use crate::engine::svn_baseline::{TempFile, TempIndex};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};

/// 调用方各自的错误码。交付与跟进的错误要落在各自的前端引导上，引擎不替它们选。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayCodes {
    /// 冲突本身不是错误（[`ReplayOutcome::Conflict`]），这个码给调用方把它变成错误时用。
    pub conflict: &'static str,
    /// git 不认识 `--merge-base`（退出 129）。
    pub too_old: &'static str,
    /// `too_old` 的文案：升级之外还有什么出路，只有调用方说得清。
    pub too_old_message: &'static str,
    /// 其余意外失败（读不懂输出、merge-tree 异常退出、P 里放了目录等内部错误）。
    pub failed: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayCommit {
    pub oid: String,
    pub parents: Vec<String>,
    pub subject: String,
    /// 完整 message（`%B`）。
    pub body: String,
    /// `Local-Only: true`，判定与图上的角标同一份实现（`graph::parse_local_only`）。
    pub local_only: bool,
}

pub struct ReplayOpts<'a> {
    pub drop_empty: bool,
    /// 对每个 commit 给出「已被自己更晚的 SVN 提交取代」的路径集合 P。**只能放文件路径**
    /// （W19：喂目录会写坏索引），遇到目录报 `codes.failed`。
    pub superseded: &'a dyn Fn(&ReplayCommit) -> BTreeSet<String>,
    pub codes: ReplayCodes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayOutcome {
    Done {
        tip: String,
        /// 原 OID → 新 OID；丢弃的 commit 映射到它父提交的新 OID。
        map: BTreeMap<String, String>,
        dropped: Vec<String>,
        /// (原 OID, 换成 SVN 版本的路径)。只列真正换了条目的路径（C 没改的路径换不换都一样）。
        rewritten: Vec<(String, Vec<String>)>,
    },
    /// 在原 commit `at` 处冲突。什么都没有发布——重放只建了对象。
    Conflict { at: String, files: Vec<String> },
}

/// 交付用：不换任何路径。
pub fn no_superseded(_: &ReplayCommit) -> BTreeSet<String> {
    BTreeSet::new()
}

/// `base..head`，从旧到新（`--topo-order --reverse`；区间里有 merge 也照列）。
pub fn commits_between(
    repo: &str,
    base: &str,
    head: &str,
    codes: &ReplayCodes,
) -> Result<Vec<ReplayCommit>> {
    for oid in [base, head] {
        require_oid(oid, codes.failed)?;
    }
    let out = run_git(
        repo,
        &[
            "log",
            "--reverse",
            "--topo-order",
            "--format=%H%x00%P%x00%s%x00%B%x1e",
            &format!("{base}..{head}"),
        ],
    )?;
    let mut list = Vec::new();
    for rec in out.split('\u{1e}') {
        let rec = rec.trim_start_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let mut f = rec.splitn(4, '\0');
        let (Some(oid), Some(parents), Some(subject), Some(body)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            return Err(GitError::new(
                codes.failed,
                format!("读不懂 git log 的输出：{rec:?}"),
            ));
        };
        list.push(ReplayCommit {
            oid: oid.to_string(),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            subject: subject.to_string(),
            body: body.to_string(),
            local_only: parse_local_only(body),
        });
    }
    Ok(list)
}

/// 把 `commits`（从旧到新、线性）逐个重放到 `onto` 上。**不写任何 ref、不动磁盘与索引。**
///
/// `memo` 跨多次调用共享（R20）：几条分支共享的 commit 只重放一次。否则 `commit_like`
/// 每次取当前时间作提交者时间，会得到 OID 不同的重复 commit。命中时核对它确实接在当前链上
/// ——调用方对不同的 onto 共用一份 memo 是编程错误，报 `codes.failed`，不猜。
///
/// 冲突返回 [`ReplayOutcome::Conflict`]；只有 git 太旧或意外失败才是 `Err`。
pub fn replay_onto(
    repo: &str,
    onto: &str,
    commits: &[ReplayCommit],
    opts: &ReplayOpts,
    memo: &mut BTreeMap<String, String>,
) -> Result<ReplayOutcome> {
    let codes = &opts.codes;
    let mut cur = onto.to_string();
    // 当前父提交的树，懒取：只有 drop_empty 才用得上
    let mut cur_tree: Option<String> = None;
    let mut map = BTreeMap::new();
    let mut dropped = Vec::new();
    let mut rewritten = Vec::new();

    for c in commits {
        let parent = single_parent(c, codes)?;
        let p = (opts.superseded)(c);
        let swaps = if p.is_empty() {
            Vec::new()
        } else {
            swaps_for(repo, c, parent, &p, codes)?
        };
        if !swaps.is_empty() {
            rewritten.push((
                c.oid.clone(),
                swaps.iter().map(|s| s.path.clone()).collect(),
            ));
        }

        if let Some(prev) = memo.get(&c.oid).cloned() {
            // 新 commit 不可能等于它自己的父提交，所以 prev == cur ⇔ 上次是丢弃
            if prev == cur {
                dropped.push(c.oid.clone());
            } else {
                let pp = rev_parse(repo, &format!("{prev}^"))?;
                if pp != cur {
                    return Err(GitError::new(
                        codes.failed,
                        format!(
                            "内部错误：{} 的重放结果 {} 不接在当前的重放链上（{}）。",
                            short(&c.oid),
                            short(&prev),
                            short(&cur)
                        ),
                    ));
                }
                cur = prev.clone();
                cur_tree = None;
            }
            map.insert(c.oid.clone(), prev);
            continue;
        }

        let theirs = if swaps.is_empty() {
            c.oid.clone()
        } else {
            c_prime(repo, c, parent, &swaps, codes)?
        };
        let tree = match merge(repo, &cur, parent, &theirs, c, codes)? {
            Merged::Clean(t) => t,
            Merged::Conflict(files) => {
                return Ok(ReplayOutcome::Conflict {
                    at: c.oid.clone(),
                    files,
                })
            }
        };
        if opts.drop_empty {
            let ct = match cur_tree.take() {
                Some(t) => t,
                None => rev_parse(repo, &format!("{cur}^{{tree}}"))?,
            };
            let became_empty = tree == ct && !originally_empty(repo, &c.oid, parent)?;
            cur_tree = Some(ct);
            if became_empty {
                dropped.push(c.oid.clone());
                map.insert(c.oid.clone(), cur.clone());
                memo.insert(c.oid.clone(), cur.clone());
                continue;
            }
        }
        cur = commit_like(repo, &tree, &cur, &c.oid)?;
        cur_tree = Some(tree);
        map.insert(c.oid.clone(), cur.clone());
        memo.insert(c.oid.clone(), cur.clone());
    }
    Ok(ReplayOutcome::Done {
        tip: cur,
        map,
        dropped,
        rewritten,
    })
}

/// 「已全部提交」捷径（W20）：`old_base..head` 改过的每个路径（含删除、mode），`head` 里的
/// 条目都与 `new` 相同，且区间里没有 Local-Only commit。成立时调用方取 NEW_TIP = NEW，
/// 不重放，也不要求线性（区间里可以有 merge commit，U5）。
///
/// 两次 `diff-tree` 的交集为空 ⇔ 上面的条件（与逐路径 `ls-tree` 比较结论一致，W20）。
/// 不带 pathspec，路径都在 stdout 里，没有命令行长度问题。
pub fn all_committed(repo: &str, old_base: &str, head: &str, new: &str) -> Result<bool> {
    for oid in [old_base, head, new] {
        require_oid(oid, "svn_replay_failed")?;
    }
    let mine = changed_paths(repo, old_base, head)?;
    if !mine.is_empty() {
        let vs_new = changed_paths(repo, head, new)?;
        if mine.intersection(&vs_new).next().is_some() {
            return Ok(false);
        }
    }
    // Local-Only 被 TortoiseSVN 一起提交上去时内容也会对上（泄漏），那正是不能静默收掉的情形
    let log = run_git(
        repo,
        &["log", "--format=%B%x1e", &format!("{old_base}..{head}")],
    )?;
    Ok(!log.split('\u{1e}').any(parse_local_only))
}

fn changed_paths(repo: &str, a: &str, b: &str) -> Result<BTreeSet<String>> {
    let out = run_git(
        repo,
        &[
            "--literal-pathspecs",
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-renames",
            a,
            b,
        ],
    )?;
    Ok(out
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// C' 里要换掉的一条：把 `path` 写成 C^ 的条目；`entry == None` 表示 C^ 没有它（写 mode 0 删掉）。
struct Swap {
    path: String,
    entry: Option<(String, String)>,
}

/// P 里 C 真正改过的路径及其 C^ 的条目。C 没改的路径在 C 里本来就等于 C^，不用换。
fn swaps_for(
    repo: &str,
    c: &ReplayCommit,
    parent: &str,
    p: &BTreeSet<String>,
    codes: &ReplayCodes,
) -> Result<Vec<Swap>> {
    for path in p {
        if path.is_empty() || path.ends_with('/') || path.contains('\\') {
            return Err(not_a_file(c, path, codes));
        }
    }
    let in_c = ls_entries(repo, &c.oid, p, c, codes)?;
    let in_parent = ls_entries(repo, parent, p, c, codes)?;
    Ok(p.iter()
        .filter(|path| in_c.get(*path) != in_parent.get(*path))
        .map(|path| Swap {
            path: path.clone(),
            entry: in_parent.get(path).cloned(),
        })
        .collect())
}

/// `rev` 的树里 `paths` 各自的 (mode, oid)；不存在的不出现在结果里。目录报内部错误。
fn ls_entries(
    repo: &str,
    rev: &str,
    paths: &BTreeSet<String>,
    c: &ReplayCommit,
    codes: &ReplayCodes,
) -> Result<BTreeMap<String, (String, String)>> {
    let mut found = BTreeMap::new();
    let list: Vec<&str> = paths.iter().map(String::as_str).collect();
    // 分批：Windows 命令行上限约 32K 字符
    let mut start = 0;
    while start < list.len() {
        let mut end = start;
        let mut len = 0;
        while end < list.len() && (end == start || len + list[end].len() < 16_000) {
            len += list[end].len() + 1;
            end += 1;
        }
        // `--full-tree`：不受 cwd 影响；`--literal-pathspecs`：`[`、`*` 不当 glob（W19 / W20）。
        // 不带 `-r`：路径是目录时列出的是它自己的 tree 条目，正好拿来拒绝
        let mut args = vec![
            "--literal-pathspecs",
            "ls-tree",
            "--full-tree",
            "-z",
            rev,
            "--",
        ];
        args.extend_from_slice(&list[start..end]);
        let out = run_git(repo, &args)?;
        for rec in out.split('\0').filter(|r| !r.is_empty()) {
            let Some((meta, path)) = rec.split_once('\t') else {
                return Err(GitError::new(
                    codes.failed,
                    format!("读不懂 git ls-tree 的输出：{rec:?}"),
                ));
            };
            if !paths.contains(path) {
                continue;
            }
            let mut m = meta.split(' ');
            let (Some(mode), Some(kind), Some(oid)) = (m.next(), m.next(), m.next()) else {
                return Err(GitError::new(
                    codes.failed,
                    format!("读不懂 git ls-tree 的输出：{rec:?}"),
                ));
            };
            if kind == "tree" {
                return Err(not_a_file(c, path, codes));
            }
            found.insert(path.to_string(), (mode.to_string(), oid.to_string()));
        }
        start = end;
    }
    Ok(found)
}

/// 构造 C'：临时索引读入 C，把 swaps 写成 C^ 的条目，`write-tree` 后挂在 C^ 上（W19）。
/// C' 只给 merge-tree 当 theirs，元数据无所谓；最终 commit 的元数据仍取自原 C。
fn c_prime(
    repo: &str,
    c: &ReplayCommit,
    parent: &str,
    swaps: &[Swap],
    codes: &ReplayCodes,
) -> Result<String> {
    let idx = TempIndex::empty(repo)?;
    let abs = idx
        .abs_path()
        .map_err(|e| GitError::new(codes.failed, e.message))?;
    let env = [("GIT_INDEX_FILE", abs.as_str())];
    run_git_env(repo, &["read-tree", &c.oid], &env)?;
    let zero = "0".repeat(c.oid.len());
    let mut input = Vec::new();
    for s in swaps {
        let line = match &s.entry {
            Some((mode, oid)) => format!("{mode} {oid}\t{}", s.path),
            None => format!("0 {zero}\t{}", s.path),
        };
        input.extend_from_slice(line.as_bytes());
        input.push(0);
    }
    run_git_stdin_env(repo, &["update-index", "-z", "--index-info"], &input, &env)?;
    let tree = run_git_env(repo, &["write-tree"], &env)?.trim().to_string();
    let oid = run_git(
        repo,
        &[
            "commit-tree",
            &tree,
            "-p",
            parent,
            "-m",
            "mygitclient: replay C'",
        ],
    )?
    .trim()
    .to_string();
    // `idx` 在这里离开作用域被删；上面任何一步 `?` 早退也一样
    drop(idx);
    Ok(oid)
}

enum Merged {
    Clean(String),
    Conflict(Vec<String>),
}

fn merge(
    repo: &str,
    cur: &str,
    parent: &str,
    theirs: &str,
    c: &ReplayCommit,
    codes: &ReplayCodes,
) -> Result<Merged> {
    let mb = format!("--merge-base={parent}");
    // `-z`：不带它 git 按 core.quotePath（默认 true）把非 ASCII 路径转义成八进制，
    // 冲突文件名就对不上 diff-tree -z 的原样路径（里程碑 review）
    let (code, out) = run_git_code(
        repo,
        &["merge-tree", "--write-tree", "-z", &mb, cur, theirs],
    )?;
    match code {
        0 => Ok(Merged::Clean(
            out.split('\0').next().unwrap_or("").trim().to_string(),
        )),
        1 => Ok(Merged::Conflict(conflict_files(&out))),
        // 129 = 用法错误：多半是 git 太旧，不认识 --merge-base
        129 => Err(GitError::new(codes.too_old, codes.too_old_message)),
        _ => Err(GitError::new(
            codes.failed,
            format!(
                "重放 {} 时 git merge-tree 退出 {code}：{}",
                short(&c.oid),
                out.trim()
            ),
        )),
    }
}

/// `-z` 形态：`<tree>\0`，随后每条 `<mode> <oid> <stage>\t<path>\0`，一个空字段之后是说明
/// 文字。只认 stage 行，不解析说明文字（W17）。
fn conflict_files(out: &str) -> Vec<String> {
    let mut files: Vec<String> = out
        .split('\0')
        .skip(1)
        .take_while(|l| !l.is_empty())
        .filter_map(|l| l.split_once('\t').map(|(_, p)| p.to_string()))
        .collect();
    files.sort();
    files.dedup();
    files
}

/// 用 `like` 的作者与 message 在 `parent` 上创建 `tree` 的 commit（计划 B15）。
fn commit_like(repo: &str, tree: &str, parent: &str, like: &str) -> Result<String> {
    let meta = run_git(
        repo,
        &[
            "log",
            "-1",
            "--date=raw",
            "--format=%an%x00%ae%x00%ad%x00%B",
            like,
        ],
    )?;
    let mut f = meta.splitn(4, '\0');
    let (name, email, date, msg) = (
        f.next().unwrap_or(""),
        f.next().unwrap_or(""),
        f.next().unwrap_or(""),
        f.next().unwrap_or("").trim_end_matches('\n'),
    );
    // message 走 `-F <文件>`（W27）：`-m` 受 Windows 命令行约 32K 字符的上限；`-F -`（stdin）
    // 在 run_git* 下读到的是 null，会产出空 message。内容按 `-m` 的规矩补一个结尾换行
    // （commit-tree 对 `-m` 做 complete_line、对 `-F` 不做），与改造前逐字节相同
    let body = if msg.is_empty() {
        String::new()
    } else {
        format!("{msg}\n")
    };
    let file = TempFile::in_git_dir(repo, "msg", body.as_bytes())?;
    let path = file.abs_path()?;
    let oid = run_git_env(
        repo,
        &["commit-tree", tree, "-p", parent, "-F", &path],
        &[
            ("GIT_AUTHOR_NAME", name),
            ("GIT_AUTHOR_EMAIL", email),
            ("GIT_AUTHOR_DATE", date),
        ],
    )?
    .trim()
    .to_string();
    drop(file);
    Ok(oid)
}

/// 原本就空（C 的树 == C^ 的树）的 commit 不算「变空」，保留。
fn originally_empty(repo: &str, oid: &str, parent: &str) -> Result<bool> {
    let out = run_git(
        repo,
        &[
            "rev-parse",
            &format!("{oid}^{{tree}}"),
            &format!("{parent}^{{tree}}"),
        ],
    )?;
    let mut l = out.lines();
    Ok(l.next() == l.next())
}

fn single_parent<'c>(c: &'c ReplayCommit, codes: &ReplayCodes) -> Result<&'c str> {
    match c.parents.as_slice() {
        [p] => Ok(p.as_str()),
        [] => Err(GitError::new(codes.failed, format!("{} 没有父提交", c.oid))),
        _ => Err(GitError::new(
            codes.failed,
            format!(
                "内部错误：{} {} 是 merge commit，逐个重放不处理 merge（应先过线性检查）。",
                short(&c.oid),
                c.subject
            ),
        )),
    }
}

fn not_a_file(c: &ReplayCommit, path: &str, codes: &ReplayCodes) -> GitError {
    GitError::new(
        codes.failed,
        format!(
            "内部错误：重放 {} 时要换成 SVN 版本的路径 `{path}` 不是文件。只能按文件换，\
             喂目录会写坏临时索引（W19）。",
            short(&c.oid)
        ),
    )
}

fn rev_parse(repo: &str, rev: &str) -> Result<String> {
    crate::engine::git_read::resolve(repo, rev)?.ok_or_else(|| {
        GitError::new(
            "git_cli",
            format!("fatal: Needed a single revision（{rev}）"),
        )
    })
}

/// 空 OID 会让区间退化成 `..head`（= `HEAD..head`），查的就不是那一段了。
fn require_oid(oid: &str, code: &str) -> Result<()> {
    if oid.len() != 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(GitError::new(
            code,
            format!("重放拿到的不是一个完整 OID：`{oid}`"),
        ));
    }
    Ok(())
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}
