//! 跟进冲突时，冲突文件能不能直接用磁盘上的内容（设计 `2026-11-25-follow-resolve-with-disk.md`
//! §6、§11）。
//!
//! 场景：同一处冲突已经在 TortoiseSVN 里解决过（update 时 SVN 报冲突，用户合好、标了 Resolved），
//! 磁盘上就是想要的结果。重放把分支上的提交放到新基线上时，这个文件又冲突了；这里判断「把重放的
//! 结果直接换成磁盘内容」是否安全。
//!
//! # 危险的方向
//!
//! 把不该进提交的内容带进去：合并之后又在别处手改、还没提交的内容。所以除了「磁盘上没有冲突标记」，
//! 还要求冲突块以外的部分与 git 自己合出来的结果逐行一致。冲突块内部混着的未提交改动分不出来，
//! 由冲突框里的外部对比交给人看。
//!
//! 判不准的一律按不能用：只是退回手动跟进（git 与 SVN 的合并算法划的冲突块可能不同，这类假阴性
//! 可以接受）。
//!
//! # 为什么不用 merge-tree 的冲突树当模板
//!
//! 冲突树按用户的 `merge.conflictStyle`（diff3 / zdiff3 多一段 `|||||||`）和 `conflict-marker-size`
//! 属性写标记，标签是 OID，文件内容里也可能有形似标记的行。这里对 stage 1/2/3 用
//! `git merge-file -p --object-id` 重新合一次（git ≥ 2.43，跟进本来就要求 2.43），钉住两段式、
//! 用带 OID 的唯一标签，按标签切冲突块。
//!
//! 只读 git 对象与磁盘；唯一的写入是把磁盘内容写成 blob（`hash-object -w`），之后拼树要用它。

use crate::engine::git_cli::{run_git, run_git_bytes, run_git_code_bytes, run_git_stdin};
use crate::engine::git_read;
use crate::engine::svn_replay::{entries_at, ConflictStage, ReplayCodes, Resolution};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 超过这个大小不做逐行核对。
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// 一个冲突文件的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskCheck {
    /// 能用：磁盘内容已经写成 blob，mode 取两边一致的那个。
    Usable(Resolution),
    /// 不能用，写给用户看的原因。
    Refused(String),
}

/// `files`：冲突文件 → merge-tree 给的 stage 条目（[`crate::engine::svn_replay::ReplayOutcome::Conflict`]）。
/// `scope..head` 是分支在基线之后的提交（数「有几个提交改过它」用）。
///
/// 每个文件都有结论；核对本身出错（文件被独占锁住之类）也只是「不能用」，写明原因。
pub fn check_disk(
    repo: &str,
    scope: &str,
    head: &str,
    files: &BTreeMap<String, Vec<ConflictStage>>,
) -> BTreeMap<String, DiskCheck> {
    let mut out = BTreeMap::new();
    // 过了前几条、要读磁盘的：(路径, mode, stage 1, 2, 3)
    let mut pending: Vec<(String, String, String, String, String)> = Vec::new();
    for (path, stages) in files {
        match precheck(repo, scope, head, path, stages) {
            Ok(Ok(p)) => pending.push(p),
            Ok(Err(why)) => {
                out.insert(path.clone(), DiskCheck::Refused(why));
            }
            Err(e) => {
                out.insert(path.clone(), failed(&e));
            }
        }
    }
    if pending.is_empty() {
        return out;
    }
    let blobs = match hash_disk(repo, pending.iter().map(|p| p.0.as_str())) {
        Ok(b) => b,
        Err(e) => {
            for p in pending {
                out.insert(p.0, failed(&e));
            }
            return out;
        }
    };
    for ((path, mode, s1, s2, s3), disk) in pending.into_iter().zip(blobs) {
        let verdict = match judge(repo, &disk, &s1, &s2, &s3) {
            Ok(None) => DiskCheck::Usable(Resolution { mode, blob: disk }),
            Ok(Some(why)) => DiskCheck::Refused(why),
            Err(e) => failed(&e),
        };
        out.insert(path, verdict);
    }
    out
}

fn failed(e: &GitError) -> DiskCheck {
    DiskCheck::Refused(format!("核对时出错：{}", e.message.trim()))
}

type Pending = (String, String, String, String, String);

/// 只看对象与文件元数据的几条。`Ok(Err(原因))` 是不能用。
fn precheck(
    repo: &str,
    scope: &str,
    head: &str,
    path: &str,
    stages: &[ConflictStage],
) -> Result<std::result::Result<Pending, String>> {
    if stages.is_empty() {
        return Ok(Err("不是同一处内容的冲突".into()));
    }
    let at = |n: u8| stages.iter().find(|s| s.stage == n);
    let (s1, s2, s3) = match (at(1), at(2), at(3)) {
        (Some(a), Some(b), Some(c)) => (a, b, c),
        (_, None, _) => return Ok(Err("SVN 上删除了这个文件，不是同一处内容的冲突".into())),
        (_, _, None) => return Ok(Err("你的提交删除了这个文件，不是同一处内容的冲突".into())),
        (None, ..) => return Ok(Err("两边各自新建了这个文件，不是同一处内容的冲突".into())),
    };
    let regular = |m: &str| m == "100644" || m == "100755";
    if ![s1, s2, s3].iter().all(|s| regular(&s.mode)) {
        return Ok(Err("不是普通文件（符号链接或子模块）".into()));
    }
    if s2.mode != s3.mode {
        return Ok(Err("两边的文件权限也不一样（可执行位）".into()));
    }
    // 线性历史（重放的前提）上按路径数提交就是「改过它的提交」
    let n: usize = run_git(
        repo,
        &[
            "--literal-pathspecs",
            "rev-list",
            "--count",
            &format!("{scope}..{head}"),
            "--",
            path,
        ],
    )?
    .trim()
    .parse()
    .map_err(|_| GitError::new("svn_follow_disk", "git rev-list --count 的输出读不懂"))?;
    if n > 1 {
        return Ok(Err(format!(
            "分支上有 {n} 个提交改过它，磁盘内容只有一份，分不出该归到哪个提交"
        )));
    }
    match Path::new(repo).join(path).symlink_metadata() {
        Ok(m) if m.is_file() => {
            if m.len() > MAX_BYTES {
                return Ok(Err("文件太大（超过 8 MB），不做逐行核对".into()));
            }
        }
        _ => return Ok(Err("磁盘上没有这个文件".into())),
    }
    Ok(Ok((
        path.to_string(),
        s3.mode.clone(),
        s1.oid.clone(),
        s2.oid.clone(),
        s3.oid.clone(),
    )))
}

/// 磁盘内容写成 blob：`--stdin-paths` 逐个按路径走与 `git add` 相同的过滤器（换行符规则）。
/// 必须带 `-w`：拼树时要用这个 blob（设计 §11 第 1 条）。
fn hash_disk<'a>(repo: &str, paths: impl Iterator<Item = &'a str>) -> Result<Vec<String>> {
    let paths: Vec<&str> = paths.collect();
    let input: String = paths.iter().map(|p| format!("{p}\n")).collect();
    let out = run_git_stdin(
        repo,
        &["hash-object", "-w", "--stdin-paths"],
        input.as_bytes(),
    )?;
    let blobs: Vec<String> = out.lines().map(|l| l.trim().to_string()).collect();
    if blobs.len() != paths.len() {
        return Err(GitError::new(
            "svn_follow_hash_failed",
            "git hash-object 的输出行数对不上",
        ));
    }
    Ok(blobs)
}

/// 读内容之后的几条。`Ok(None)` 是能用。
fn judge(repo: &str, disk: &str, s1: &str, s2: &str, s3: &str) -> Result<Option<String>> {
    let read = |oid: &str| run_git_bytes(repo, &["cat-file", "blob", oid]);
    let mine = read(disk)?;
    let svn = read(s2)?;
    let theirs = read(s3)?;
    if [&mine, &svn, &theirs].iter().any(|b| b.contains(&0)) {
        return Ok(Some("二进制文件，没法逐行核对".into()));
    }
    if has_conflict_markers(&mine) {
        return Ok(Some("磁盘上还有冲突标记".into()));
    }
    if disk == s3 {
        return Ok(Some(
            "磁盘上还是你提交里的版本，SVN 那边的改动不在磁盘上".into(),
        ));
    }
    if disk == s2 {
        return Ok(Some(
            "磁盘上就是 SVN 的新版本：等于在 SVN 里放弃了你对这个文件的改动，这一版不支持直接用它跟进"
                .into(),
        ));
    }
    Ok(match remerge(repo, s2, s1, s3)? {
        Remerged::Clean(_) => Some("git 重新合并这个文件时没有冲突，和重放的结果对不上".into()),
        Remerged::Conflict(t) => match t.outside_matches(&mine) {
            Some(true) => None,
            Some(false) => {
                Some("冲突处以外还有别的改动（合并之后又改过，或者少了 SVN 的改动）".into())
            }
            None => Some("核对时出错：读不懂 git 重新合并的结果".into()),
        },
    })
}

/// [`remerge`] 的结果。
enum Remerged {
    /// 没有冲突：合并结果。
    Clean(Vec<u8>),
    Conflict(Template),
}

/// 带冲突标记的合并结果，标记行是带 OID 的唯一标签（见模块注释）。
struct Template {
    body: Vec<u8>,
    open: String,
    close: String,
}

impl Template {
    /// `content` 在冲突块以外是否与合并结果逐行一致（[`outside_hunks_match`]）。
    fn outside_matches(&self, content: &[u8]) -> Option<bool> {
        outside_hunks_match(
            &self.body,
            content,
            self.open.as_bytes(),
            self.close.as_bytes(),
        )
    }
}

/// `git merge-file -p --object-id <ours> <base> <theirs>`，钉住两段式标记（冲突框的磁盘核对与
/// F2 ① 的核对共用）。
fn remerge(repo: &str, ours: &str, base: &str, theirs: &str) -> Result<Remerged> {
    let open = format!("jkk-ours-{ours}");
    let close = format!("jkk-theirs-{theirs}");
    let (code, body) = run_git_code_bytes(
        repo,
        &[
            "-c",
            "merge.conflictStyle=merge",
            "merge-file",
            "-p",
            "--object-id",
            "-L",
            &open,
            "-L",
            "jkk-base",
            "-L",
            &close,
            ours,
            base,
            theirs,
        ],
    )?;
    match code {
        0 => Ok(Remerged::Clean(body)),
        // 冲突个数（git 封顶 127）
        1..=127 => Ok(Remerged::Conflict(Template {
            body,
            open: format!("<<<<<<< {open}"),
            close: format!(">>>>>>> {close}"),
        })),
        _ => Err(GitError::new(
            "svn_follow_disk",
            format!("git merge-file 退出 {code}"),
        )),
    }
}

/// [`supersede_safe`] 的结论。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SupersedeCheck {
    /// 可以换的路径。提交没改过的也在里面：换不换一样，留着给后面的提交继承。
    pub safe: BTreeSet<String>,
    /// 提交改过、却不换的路径：核对没通过、核对不了，或者更早的提交在这条路径上已经没通过。
    pub refused: BTreeSet<String>,
}

/// 判二进制只看开头这么多字节（与 git 的 `buffer_is_binary` 相同）。
const BINARY_PROBE: usize = 8000;

/// F2 ①（自己更晚用 TortoiseSVN 提交过的路径取 SVN 版本）换之前的核对（2026-11-26 使用报告第 6.2 节，
/// 计划 `2026-11-27-follow-mark-gap-and-supersede-check.md` Task 2）。
///
/// 换掉的是提交 `commit` 对这个文件的改动：重放时它等于没改过，文件取新基线的版本。只有改动已经
/// 全在 SVN 版本里时这才不丢东西。以 `parent` 的版本为合并基础、新基线的版本为 ours、`commit` 的
/// 版本为 theirs 合一次：
///
/// - 干净：合并结果必须等于 SVN 版本。不等说明 `commit` 有改动不在 SVN 上（TortoiseSVN 只提交了一部分）
/// - 有冲突：SVN 版本在冲突块以外必须与合并结果逐行一致（与冲突框的磁盘核对同一套方法）。冲突块
///   内部分不清，按「SVN 版本是最终版」放行，这是 F2 ① 原本的前提（中间版本 W10-R2）
///
/// 不通过就不换，`commit` 照常重放：干净就保留改动，冲突就进冲突框，都不会静默丢。
///
/// `blocked`：更早的提交在这些路径上没通过、照常重放了。这个提交也不能换：前面的改动重放上去之后，
/// 再把这个提交的改动（比如删掉前面加的一行）换掉，等于把用户删掉的内容又带回来（独立 review 第 1 条）。
///
/// 直接放行的几种：两边已经相同、有一边删了文件或不是普通文件（整个文件的操作，不存在只提交一部分）、
/// 二进制（TortoiseSVN 只能整个提交）。不放行：带 UTF-16 / UTF-32 BOM 的文本（字节里有 NUL，逐行核对
/// 不了，又不是二进制）、超过 8 MB 的文本、核对时出错。
pub fn supersede_safe(
    repo: &str,
    commit: &str,
    parent: &str,
    new_base: &str,
    paths: &BTreeSet<String>,
    blocked: &BTreeSet<String>,
    codes: &ReplayCodes,
) -> Result<SupersedeCheck> {
    let in_c = entries_at(repo, commit, paths, codes)?;
    let in_p = entries_at(repo, parent, paths, codes)?;
    let in_n = entries_at(repo, new_base, paths, codes)?;
    let regular = |e: &(String, String)| e.0 == "100644" || e.0 == "100755";
    let mut empty_blob: Option<String> = None;
    let mut out = SupersedeCheck::default();
    for p in paths {
        let (c, par, n) = (in_c.get(p), in_p.get(p), in_n.get(p));
        // 先看 blocked：这个提交没改它时也不能放回 `safe`，否则之后的提交又能换它了
        if blocked.contains(p) {
            if c != par {
                out.refused.insert(p.clone());
            }
            continue;
        }
        if c == par {
            out.safe.insert(p.clone());
            continue;
        }
        let (Some(c), Some(n)) = (c, n) else {
            out.safe.insert(p.clone());
            continue;
        };
        if c.1 == n.1 || !regular(c) || !regular(n) || par.is_some_and(|e| !regular(e)) {
            out.safe.insert(p.clone());
            continue;
        }
        let verdict = (|| {
            let base = match par {
                Some(e) => e.1.clone(),
                None => match &empty_blob {
                    Some(b) => b.clone(),
                    None => {
                        let b = run_git_stdin(repo, &["hash-object", "-w", "--stdin"], b"")?
                            .trim()
                            .to_string();
                        empty_blob = Some(b.clone());
                        b
                    }
                },
            };
            supersede_ok(repo, &n.1, &base, &c.1)
        })();
        let short = crate::engine::svn_util::short(commit);
        match verdict {
            Ok(true) => {
                out.safe.insert(p.clone());
            }
            Ok(false) => {
                log::info!("F2 ①：{short} 对 {p} 的改动不全在 SVN 版本里，不换，照常重放");
                out.refused.insert(p.clone());
            }
            // 一个文件核对不了，不连累别的文件
            Err(e) => {
                log::warn!(
                    "F2 ①：核对 {short} 对 {p} 的改动时出错，不换：{}",
                    e.message
                );
                out.refused.insert(p.clone());
            }
        }
    }
    Ok(out)
}

/// `parent → commit` 改过的路径里落在 `within` 的那些。进程内读不了时交回 `git diff-tree`。
pub fn touched_paths(
    repo: &str,
    parent: &str,
    commit: &str,
    within: &BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let all: Vec<String> = git_read::or_cli(
        "列 commit 改过的路径",
        || {
            Ok(git_read::tree_changes(repo, parent, commit)?
                .into_iter()
                .map(|c| c.path)
                .collect())
        },
        || {
            Ok(run_git(
                repo,
                &[
                    "diff-tree",
                    "-r",
                    "-z",
                    "--no-renames",
                    "--name-only",
                    parent,
                    commit,
                ],
            )?
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect())
        },
    )?;
    Ok(all.into_iter().filter(|p| within.contains(p)).collect())
}

/// [`supersede_safe`] 的一个文件：`svn` 是新基线的 blob，`base` / `mine` 是提交前后的 blob。
///
/// 只走 git 命令：与 [`remerge`] 读到的是同一份内容。libgit2 在 SHA-256 仓库、有 replace ref 时读不了
/// 或读到的不一样（第二轮独立 review 第 1 条）。先取大小：有一份超过 8 MB 时只读 SVN 那一份判二进制，
/// 提交前后的两份不读（第 2 条）。
fn supersede_ok(repo: &str, svn: &str, base: &str, mine: &str) -> Result<bool> {
    let bad = || GitError::new("svn_follow_disk", "git cat-file --batch-check 的输出读不懂");
    let sizes = run_git_stdin(
        repo,
        &["cat-file", "--batch-check=%(objectsize)"],
        format!("{svn}\n{base}\n{mine}\n").as_bytes(),
    )?
    .lines()
    .map(|l| l.trim().parse::<u64>().map_err(|_| bad()))
    .collect::<Result<Vec<u64>>>()?;
    if sizes.len() != 3 {
        return Err(bad());
    }
    let read = |oid: &str| run_git_bytes(repo, &["cat-file", "blob", oid]);
    if sizes.iter().any(|n| *n > MAX_BYTES) {
        // TortoiseSVN 提交的那一版是二进制（贴图、模型），就是整个提交的；是文本就不核对、不换
        return Ok(is_binary(&read(svn)?));
    }
    let blobs = [read(svn)?, read(base)?, read(mine)?];
    if blobs.iter().any(|b| wide_bom(b)) {
        return Ok(false);
    }
    if blobs.iter().any(|b| is_binary(b)) {
        return Ok(true);
    }
    let [svn_bytes, ..] = blobs;
    Ok(match remerge(repo, svn, base, mine)? {
        Remerged::Clean(merged) => merged == svn_bytes,
        Remerged::Conflict(t) => t.outside_matches(&svn_bytes) == Some(true),
    })
}

/// 与 git 的 `buffer_is_binary` 相同：前 8000 字节里有 NUL。带 UTF-16 / UTF-32 BOM 的不算（那是文本）。
fn is_binary(b: &[u8]) -> bool {
    !wide_bom(b) && b[..b.len().min(BINARY_PROBE)].contains(&0)
}

/// UTF-16 / UTF-32 的 BOM（`FF FE` 同时盖住 UTF-32 LE）。这类文本按字节有 NUL，git 当成二进制。
fn wide_bom(head: &[u8]) -> bool {
    head.starts_with(&[0xFF, 0xFE])
        || head.starts_with(&[0xFE, 0xFF])
        || head.starts_with(&[0x00, 0x00, 0xFE, 0xFF])
}

/// 一行（不含行尾）是不是 `open` / `close` 标记行。
fn is_line(line: &[u8], marker: &[u8]) -> bool {
    let l = line.strip_suffix(b"\n").unwrap_or(line);
    let l = l.strip_suffix(b"\r").unwrap_or(l);
    l == marker
}

/// `template` 去掉冲突块（`open` 行到 `close` 行，含两行）之后剩下的各段，`disk` 是否等于
/// `s0 X1 s1 … Xn sn`（每个 `Xi` 可以是任意行，包括空）。按行比，行尾一起比。
///
/// 贪心最左匹配：开头段与结尾段钉死在两头，中间各段依次找最左的出现。对这种只有通配的模式，
/// 存在任何一种切法时贪心也一定找得到。
///
/// 模板里没有冲突块、或者标记不成对时为 `None`。
pub fn outside_hunks_match(
    template: &[u8],
    disk: &[u8],
    open: &[u8],
    close: &[u8],
) -> Option<bool> {
    let mut segs: Vec<Vec<&[u8]>> = vec![Vec::new()];
    let mut inside = false;
    for line in template.split_inclusive(|b| *b == b'\n') {
        if is_line(line, open) {
            if inside {
                return None;
            }
            inside = true;
        } else if is_line(line, close) {
            if !inside {
                return None;
            }
            inside = false;
            segs.push(Vec::new());
        } else if !inside {
            segs.last_mut()?.push(line);
        }
    }
    if inside || segs.len() < 2 {
        return None;
    }
    let d: Vec<&[u8]> = disk.split_inclusive(|b| *b == b'\n').collect();
    let first = &segs[0];
    let last = &segs[segs.len() - 1];
    if first.len() + last.len() > d.len() {
        return Some(false);
    }
    if d[..first.len()] != first[..] || d[d.len() - last.len()..] != last[..] {
        return Some(false);
    }
    let end = d.len() - last.len();
    let mut pos = first.len();
    for s in &segs[1..segs.len() - 1] {
        match find(&d[pos..end], s) {
            Some(i) => pos += i + s.len(),
            None => return Some(false),
        }
    }
    Some(true)
}

/// `hay` 里 `needle` 最左一次连续出现的位置。空的 `needle` 在 0。
fn find(hay: &[&[u8]], needle: &[&[u8]]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == needle[..])
}

/// 行首正好 7 个 `<` 或 `>`，后面是空格或行尾。单独的 `=======` 不算：注释里很常见，而且
/// TortoiseSVN 留下的冲突一定带着开头与结尾两行。
pub fn has_conflict_markers(bytes: &[u8]) -> bool {
    bytes.split(|b| *b == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        b"<>".iter().any(|ch| {
            line.len() >= 7
                && line[..7].iter().all(|b| b == ch)
                && (line.len() == 7 || line[7] == b' ')
        })
    })
}
