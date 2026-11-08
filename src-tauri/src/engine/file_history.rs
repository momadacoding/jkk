//! 文件历史、Blame、单个文件在某次提交里的 diff（计划 2026-11-08-file-history-blame）。
//!
//! 历史与 blame 走 git CLI 而不是 libgit2（决策日志 D1）：libgit2 没有 `--follow`，
//! 跨重命名追溯得自己逐个提交做改名检测，结果还和 git 的判定不一致；它的 blame 在长历史上
//! 明显更慢，也不认 `blame.ignoreRevsFile` 这类用户配置。两条命令都只读。
//!
//! # 参数防护
//!
//! - `file` 为空直接报错：空 pathspec 在 git 里是「全部」（AGENTS.md §6.2）。
//! - rev 为空或以 `-` 开头直接报错：它排在 `--` 前面，`--output=x` 这种会被 git 当成选项执行。

use crate::engine::diff::DiffData;
use crate::engine::git_cli::run_git_bytes;
use crate::error::{GitError, Result};
use git2::{DiffOptions, Repository};
use serde::Serialize;
use std::collections::HashMap;

/// 历史最多列多少条。`--follow` 要逐个提交做改名检测，几万条的历史整份走一遍要好几秒，
/// 列表也没人翻得完。
pub const MAX_HISTORY: usize = 1000;

/// Blame 最多送多少行，与「File Tree」页整文件显示的上限一致（`FileContentView`）。
pub const MAX_BLAME_LINES: usize = 5000;

/// 工作区里还没提交的行，`git blame` 把它归到这个全 0 的 sha 上。
pub const UNCOMMITTED: &str = "0000000000000000000000000000000000000000";

const BAD_ARGS: &str = "bad_history_args";

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileHistoryEntry {
    pub id: String,
    pub parents: Vec<String>,
    pub author: String,
    /// 作者时间，unix 秒。
    pub time: i64,
    /// 提交标题（第一行）。
    pub subject: String,
    /// 这次提交里这个文件的路径。改名之前的提交是旧路径——Blame、读 diff 都要用它。
    pub path: String,
    /// 改名 / 复制的来源路径；其余情况为 None。
    pub old_path: Option<String>,
    /// 与 `FileChange::status` 同一套取值。万一某条没有 name-status，按 `modified`（见 `parse_log`）。
    pub status: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileHistory {
    /// 从新到旧。
    pub entries: Vec<FileHistoryEntry>,
    /// 超过上限被截断。
    pub truncated: bool,
}

#[derive(Serialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BlamePrevious {
    /// 这一提交之前、这个文件上一次被改的那个提交。
    pub commit: String,
    /// 那时的路径。
    pub path: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BlameCommit {
    pub author: String,
    /// 作者时间，unix 秒。
    pub time: i64,
    pub summary: String,
    /// 「Blame 此改动之前的版本」用；根提交与边界提交没有。
    pub previous: Option<BlamePrevious>,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BlameLine {
    pub commit: String,
    /// 这一行在那个提交里的行号，从 1 起。
    pub orig_line: u32,
    /// 这一行在被 blame 的这一版里的行号，从 1 起。
    pub final_line: u32,
    /// 这一行在那个提交里所在文件的路径（改名之前是旧路径）。
    pub orig_path: String,
    /// 行内容，不含换行符（CRLF 的 `\r` 也去掉）。
    pub content: String,
}

#[derive(Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BlameData {
    pub lines: Vec<BlameLine>,
    /// 行里出现过的提交 → 提交信息。porcelain 输出只在第一次出现时给，这里补全成表。
    pub commits: HashMap<String, BlameCommit>,
    /// 二进制文件：`lines` 为空，前端不该按行显示。
    pub binary: bool,
    pub truncated: bool,
    /// 文件的总行数，截断时也是完整的行数。
    pub total_lines: usize,
}

fn bad(message: impl Into<String>) -> GitError {
    GitError::new(BAD_ARGS, message)
}

fn check_file(file: &str) -> Result<()> {
    if file.is_empty() {
        return Err(bad("没有指定文件"));
    }
    Ok(())
}

fn check_rev(rev: &str) -> Result<()> {
    if rev.is_empty() || rev.starts_with('-') {
        return Err(bad(format!("无法识别的版本：{rev:?}")));
    }
    Ok(())
}

/// `git log --follow`：只列改过这个文件的提交，跨重命名追溯。`revs` 是起点，取并集。
pub fn file_history(repo: &str, file: &str, revs: &[String]) -> Result<FileHistory> {
    file_history_limited(repo, file, revs, MAX_HISTORY)
}

/// 同 [`file_history`]，上限可调（测试用）。
pub fn file_history_limited(
    repo: &str,
    file: &str,
    revs: &[String],
    limit: usize,
) -> Result<FileHistory> {
    check_file(file)?;
    if revs.is_empty() {
        return Err(bad("没有指定从哪个版本开始"));
    }
    for r in revs {
        check_rev(r)?;
    }
    // 多要一条，才分得清「恰好这么多」与「被截断了」
    let max = format!("--max-count={}", limit + 1);
    let mut args: Vec<&str> = vec![
        // 路径按字面量匹配：`a[1].txt` 不能当通配符匹配到 `a1.txt`，`:` 开头不能当 pathspec magic
        // （与 `commit_file_diff` 的 `disable_pathspec_match` 同一口径，独立 review 第 3 条）
        "--literal-pathspecs",
        "log",
        "--follow",
        // 显式打开改名检测：用户把 diff.renames 关掉时，改名那一条的 name-status 会变成一删一增
        "-M",
        "-z",
        "--no-color",
        // 记录之间用 \x1e，头里的字段之间用 \x1f：标题里可能有 tab、引号、竖线
        "--format=%x1e%H%x1f%P%x1f%an%x1f%at%x1f%s",
        "--name-status",
        &max,
    ];
    args.extend(revs.iter().map(String::as_str));
    args.push("--");
    args.push(file);
    let out = run_git_bytes(repo, &args)?;
    let mut entries = parse_log(&out, file);
    let truncated = entries.len() > limit;
    entries.truncate(limit);
    Ok(FileHistory { entries, truncated })
}

/// 解析 `git log -z --format=<上面那个> --name-status` 的输出。
///
/// 一条记录（2026-11-08 fixture 实测）：`\x1e<头>\0\n<状态>\0<路径>\0`，改名 / 复制是
/// `R100\0<旧>\0<新>\0`。`--follow` 下没有 diff 的合并提交 git 整条不输出（`tests/file_history.rs` 实测）。
///
/// `current` 从新到旧往下传，改名那一条之后换成旧路径。万一出现只有头、没有 name-status 的记录
/// （防御，真实 git 目前不给），它沿用上一条（更新的那一条）追到的路径。
fn parse_log(out: &[u8], file: &str) -> Vec<FileHistoryEntry> {
    let text = String::from_utf8_lossy(out);
    let mut current = file.to_string();
    let mut entries = Vec::new();
    for record in text.split('\x1e').filter(|r| !r.is_empty()) {
        let (head, rest) = record.split_once('\0').unwrap_or((record, ""));
        let mut fields = head.split('\x1f');
        let (Some(id), Some(parents), Some(author), Some(time), Some(subject)) = (
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
            fields.next(),
        ) else {
            log::warn!("文件历史里有一条解析不了的记录：{head:?}");
            continue;
        };
        let mut status = "modified";
        let mut path = current.clone();
        let mut old_path = None;
        let mut parts = rest.trim_start_matches('\n').split('\0');
        if let Some(code) = parts.next().filter(|c| !c.is_empty()) {
            let kind = code.chars().next().unwrap_or('M');
            status = match kind {
                'A' => "added",
                'D' => "deleted",
                'R' => "renamed",
                'C' => "copied",
                _ => "modified",
            };
            if matches!(kind, 'R' | 'C') {
                if let (Some(old), Some(new)) = (parts.next(), parts.next()) {
                    path = new.to_string();
                    old_path = Some(old.to_string());
                    current = old.to_string();
                }
            } else if let Some(p) = parts.next().filter(|p| !p.is_empty()) {
                path = p.to_string();
                current = p.to_string();
            }
        }
        entries.push(FileHistoryEntry {
            id: id.to_string(),
            parents: parents
                .split(' ')
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect(),
            author: author.to_string(),
            time: time.parse().unwrap_or(0),
            subject: subject.to_string(),
            path,
            old_path,
            status: status.to_string(),
        });
    }
    entries
}

/// `git blame --porcelain`。`rev` 为 None 时 blame 工作区里的这个文件，没提交的行归到 [`UNCOMMITTED`]。
pub fn blame_file(repo: &str, file: &str, rev: Option<&str>) -> Result<BlameData> {
    blame_file_limited(repo, file, rev, MAX_BLAME_LINES)
}

/// 同 [`blame_file`]，上限可调（测试用）。
pub fn blame_file_limited(
    repo: &str,
    file: &str,
    rev: Option<&str>,
    limit: usize,
) -> Result<BlameData> {
    check_file(file)?;
    if let Some(r) = rev {
        check_rev(r)?;
    }
    // 二进制先挡掉：git blame 会把它按行切开照样输出，大文件还要算很久
    if is_binary(repo, file, rev)? {
        return Ok(BlameData {
            lines: Vec::new(),
            commits: HashMap::new(),
            binary: true,
            truncated: false,
            total_lines: 0,
        });
    }
    // quotePath 关掉，`filename` / `previous` 里的中文路径才不会被转成八进制；
    // 引号、控制字符仍会被 C 风格转义，`unquote` 负责还原
    let mut args = vec!["-c", "core.quotePath=false", "blame", "--porcelain"];
    if let Some(r) = rev {
        args.push(r);
    }
    args.push("--");
    args.push(file);
    let out = run_git_bytes(repo, &args)?;
    let data = parse_porcelain(&out, limit);
    // 有输出却一行都没认出来：格式变了，报出来，不要显示成「空文件」
    if data.total_lines == 0 && out.iter().any(|b| !b.is_ascii_whitespace()) {
        return Err(GitError::new(
            "blame_unparsed",
            "认不出 git blame 的输出格式".to_string(),
        ));
    }
    Ok(data)
}

/// 用 git 的判据（前 8000 字节里有 NUL）判断是不是二进制。读不到就交给 `git blame` 去报错。
fn is_binary(repo: &str, file: &str, rev: Option<&str>) -> Result<bool> {
    const PROBE: usize = 8000;
    match rev {
        Some(r) => {
            let repo = Repository::open(repo)?;
            let Ok(obj) = repo.revparse_single(r) else {
                return Ok(false);
            };
            let Ok(entry) = obj
                .peel_to_tree()
                .and_then(|t| t.get_path(std::path::Path::new(file)))
            else {
                return Ok(false);
            };
            let Ok(blob) = entry.to_object(&repo).and_then(|o| o.peel_to_blob()) else {
                return Ok(false);
            };
            Ok(blob.is_binary())
        }
        None => {
            use std::io::Read;
            let Ok(f) = std::fs::File::open(std::path::Path::new(repo).join(file)) else {
                return Ok(false);
            };
            let mut head = Vec::with_capacity(PROBE);
            f.take(PROBE as u64).read_to_end(&mut head)?;
            Ok(head.contains(&0))
        }
    }
}

/// 解析 porcelain 输出（2026-11-08 fixture 实测）：
///
/// ```text
/// <sha> <orig> <final> [<本组行数>]
/// author A            ← 这几行只在这个提交第一次出现时给
/// author-time 1794152492
/// summary add a
/// previous <sha> <path>
/// filename a.txt      ← 同样只在第一次出现时给
/// \t<内容>
/// ```
fn parse_porcelain(out: &[u8], limit: usize) -> BlameData {
    let mut lines = Vec::new();
    let mut commits: HashMap<String, BlameCommit> = HashMap::new();
    let mut filenames: HashMap<String, String> = HashMap::new();
    let mut total = 0usize;
    // 当前这一行的头：(sha, orig, final)
    let mut cur: Option<(String, u32, u32)> = None;

    for raw in out.split(|b| *b == b'\n') {
        if let Some(content) = raw.strip_prefix(b"\t") {
            let Some((sha, orig, fin)) = cur.take() else {
                continue;
            };
            total += 1;
            if lines.len() >= limit {
                continue;
            }
            let content = content.strip_suffix(b"\r").unwrap_or(content);
            lines.push(BlameLine {
                orig_path: filenames.get(&sha).cloned().unwrap_or_default(),
                commit: sha,
                orig_line: orig,
                final_line: fin,
                content: String::from_utf8_lossy(content).into_owned(),
            });
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        if line.is_empty() {
            continue;
        }
        if let Some(header) = parse_header(&line) {
            commits
                .entry(header.0.clone())
                .or_insert_with(|| BlameCommit {
                    author: String::new(),
                    time: 0,
                    summary: String::new(),
                    previous: None,
                });
            cur = Some(header);
            continue;
        }
        let Some((sha, _, _)) = cur.as_ref() else {
            continue;
        };
        let (key, value) = line.split_once(' ').unwrap_or((&line, ""));
        let Some(info) = commits.get_mut(sha) else {
            continue;
        };
        match key {
            "author" => info.author = value.to_string(),
            "author-time" => info.time = value.parse().unwrap_or(0),
            "summary" => info.summary = value.to_string(),
            "previous" => {
                if let Some((commit, path)) = value.split_once(' ') {
                    info.previous = Some(BlamePrevious {
                        commit: commit.to_string(),
                        path: unquote(path),
                    });
                }
            }
            "filename" => {
                filenames.insert(sha.clone(), unquote(value));
            }
            _ => {}
        }
    }

    // 截断之后用不到的提交不往前端送
    let used: std::collections::HashSet<&str> = lines.iter().map(|l| l.commit.as_str()).collect();
    commits.retain(|k, _| used.contains(k.as_str()));
    BlameData {
        truncated: total > lines.len(),
        lines,
        commits,
        binary: false,
        total_lines: total,
    }
}

/// `<sha> <orig> <final>[ <n>]`。sha 是 40 位（SHA-1）或 64 位（SHA-256 仓库，独立 review 第 8 条）。
fn parse_header(line: &str) -> Option<(String, u32, u32)> {
    let mut it = line.split(' ');
    let sha = it.next()?;
    if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let orig = it.next()?.parse().ok()?;
    let fin = it.next()?.parse().ok()?;
    Some((sha.to_string(), orig, fin))
}

/// 还原 git 的 C 风格引号路径（`"a\"b\\c\t\303\244"`）；没加引号的原样返回。
fn unquote(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|t| t.strip_suffix('"')) else {
        return s.to_string();
    };
    let bytes = inner.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' || i + 1 >= bytes.len() {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let c = bytes[i + 1];
        let simple = match c {
            b'n' => Some(b'\n'),
            b't' => Some(b'\t'),
            b'r' => Some(b'\r'),
            b'a' => Some(0x07),
            b'b' => Some(0x08),
            b'f' => Some(0x0c),
            b'v' => Some(0x0b),
            b'\\' => Some(b'\\'),
            b'"' => Some(b'"'),
            _ => None,
        };
        if let Some(b) = simple {
            out.push(b);
            i += 2;
            continue;
        }
        let oct = &bytes[i + 1..bytes.len().min(i + 4)];
        if oct.len() == 3 && oct.iter().all(|d| (b'0'..=b'7').contains(d)) {
            let v = oct
                .iter()
                .fold(0u32, |acc, d| acc * 8 + u32::from(d - b'0'));
            out.push(v as u8);
            i += 4;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 一个文件在某次提交里的 diff（对第一个父提交；根提交对空树）。
///
/// `old_file` 是改名前的路径（历史那一条的 `old_path`）：pathspec 要把新旧两个路径都放进来，
/// 改名检测才配得成一对，否则一次改名显示成整份新增。空串当成没有——放进 pathspec 就成了「全部」。
pub fn commit_file_diff(
    repo: &str,
    rev: &str,
    file: &str,
    old_file: Option<&str>,
) -> Result<DiffData> {
    check_file(file)?;
    check_rev(rev)?;
    let r = Repository::open(repo)?;
    let commit = r.revparse_single(rev)?.peel_to_commit()?;
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let mut opts = DiffOptions::new();
    opts.pathspec(file);
    if let Some(old) = old_file.filter(|o| !o.is_empty() && *o != file) {
        opts.pathspec(old);
    }
    // 字面量匹配：`f[1].txt` 不能当成模式去匹配 `f1.txt`
    opts.disable_pathspec_match(true);
    let mut diff = r.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), Some(&mut opts))?;
    diff.find_similar(None)?;
    crate::engine::diff::diff_to_data_only(diff, Some(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquote_handles_escapes_and_octal_utf8() {
        assert_eq!(unquote("plain path.txt"), "plain path.txt");
        assert_eq!(unquote(r#""a\"b\\c\td""#), "a\"b\\c\td");
        assert_eq!(unquote(r#""\344\270\255.txt""#), "中.txt");
    }

    #[test]
    fn parse_log_tolerates_a_record_without_name_status() {
        // 防御用例，不是真实形态：`--follow` 下 git 不输出这种只有头的记录（真实形态见 tests/file_history.rs 的合并用例）。万一出现，路径沿用上一条追到的
        let out = "\x1eaaa\x1fp1 p2\x1fA\x1f10\x1fmerge\0\
                   \x1ebbb\x1fp1\x1fA\x1f9\x1frename\0\nR090\0old.txt\0new.txt\0\
                   \x1eccc\x1f\x1fA\x1f8\x1fadd\0\nA\0old.txt\0";
        let e = parse_log(out.as_bytes(), "new.txt");
        assert_eq!(e.len(), 3);
        assert_eq!(
            (e[0].path.as_str(), e[0].status.as_str()),
            ("new.txt", "modified")
        );
        assert_eq!(e[0].parents, ["p1", "p2"]);
        assert_eq!(e[1].old_path.as_deref(), Some("old.txt"));
        assert_eq!(
            (e[2].path.as_str(), e[2].status.as_str()),
            ("old.txt", "added")
        );
        assert!(e[2].parents.is_empty());
    }
}
