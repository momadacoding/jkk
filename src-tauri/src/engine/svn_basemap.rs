//! 基线的记账：floor + 每文件 revision + 删除墓碑（分析 §6.1）。
//!
//! 单值、没有墓碑时，message 与现行的 `git-svn-id` 格式**逐字节相同**：旧版 jkk、
//! `git log --grep=^git-svn-id`、提交图上的基线标注都只认那一行。
//!
//! 混合时**不写** `git-svn-id`：那一行的语义是「这棵树 == 服务器 r<floor>」，
//! 混合时它是假的；不认识新格式的读者读不出基线，总好过读错（分析 §6.1）。
//!
//! 新 trailer 只放在 message 的最后一段，自己解析，不经 `interpret-trailers`：
//! 后者会改写前导空格、续行和 `#` 行（W27）。

use crate::engine::git_cli::run_git_bytes;
use crate::engine::svn_baseline::{format_trailer, parse_trailer, Trailer, DELIVERED_REV_KEY};
use crate::error::{GitError, Result};
use std::collections::BTreeMap;

/// 映射条目（`revs` + `gone`）的上限。【待决 D2】
///
/// 每条一行写进 commit message。千级在 message 体积、提交说明的可读性与
/// 「一周局部更新攒下来的条目」之间留了足够余量（分析 §6.1）。
pub const MAX_MAP_ENTRIES: usize = 5000;

const BASE_PREFIX: &str = "Svn-Base-";
const REPO_KEY: &str = "Svn-Base-Repo: ";
const FLOOR_KEY: &str = "Svn-Base-Floor: ";
const REV_KEY: &str = "Svn-Base-Rev: ";
const GONE_KEY: &str = "Svn-Base-Gone: ";

/// 写进 message 里会被某个解析器当成记账的行首。摘要行若恰好以它们开头，
/// 会被 `parse_trailer` / `parse_delivered_rev`（逐行扫全文）误读，见 [`summary_line`]。
const RESERVED_PREFIXES: [&str; 3] = ["git-svn-id:", BASE_PREFIX, "Delivered-Rev:"];

/// 一个基线 commit 记的「基线树里每个文件对应 SVN 的哪个 revision」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseMap {
    /// Repository Root，不是某个工作副本的 URL（理由同 `Trailer::url`）。
    pub repos_root: String,
    pub uuid: String,
    /// 没有单列的文件都在这个 revision。取映射里的最小值（分析 §6.1）。
    pub floor: u64,
    /// 只存 revision ≠ floor 的文件；键是仓库相对、正斜杠的路径。
    pub revs: BTreeMap<String, u64>,
    /// 墓碑：已在 SVN 上删除、基线树里也已没有的文件 → 删除确认时的服务器 HEAD。
    pub gone: BTreeMap<String, u64>,
}

impl BaseMap {
    pub fn single(repos_root: impl Into<String>, uuid: impl Into<String>, rev: u64) -> Self {
        Self {
            repos_root: repos_root.into(),
            uuid: uuid.into(),
            floor: rev,
            revs: BTreeMap::new(),
            gone: BTreeMap::new(),
        }
    }

    /// 单一 revision 且没有墓碑：这时才能写 `git-svn-id`。
    pub fn is_single(&self) -> bool {
        self.revs.is_empty() && self.gone.is_empty()
    }

    /// 基线树里文件 `path` 对应的 revision。墓碑路径不在基线树里，调用方应先查 `gone`。
    pub fn rev_of(&self, path: &str) -> u64 {
        self.revs.get(path).copied().unwrap_or(self.floor)
    }

    pub fn entry_count(&self) -> usize {
        self.revs.len() + self.gone.len()
    }
}

/// 把基线的记账拼成完整的 commit message（末尾带 `\n`）。
///
/// - `summary_extra`：混合时写在首行与 trailer 之间的说明，每条一行（例如「跟进 r1245–r1250：A、B」）
/// - `delivered_rev`：交付录的基线额外记的真实提交 revision
///
/// 单值且 `summary_extra` 为空时，与改造前 `create_baseline_commit_with` 拼出的
/// `svn: rN\n\n[Delivered-Rev: ..\n]git-svn-id: ...\n` 逐字节相同。
pub fn format_message(
    map: &BaseMap,
    summary_extra: &[String],
    delivered_rev: Option<u64>,
) -> String {
    let summary: Vec<String> = summary_extra
        .iter()
        .filter_map(|l| summary_line(l))
        .collect();
    let mut msg = String::new();
    if map.is_single() {
        msg.push_str(&format!("svn: r{}\n\n", map.floor));
        push_summary(&mut msg, &summary);
        if let Some(r) = delivered_rev {
            msg.push_str(&format!("{DELIVERED_REV_KEY} {r}\n"));
        }
        // 与旧格式共用 `format_trailer`，单值的那一行只有一处定义
        msg.push_str(&format_trailer(&Trailer {
            url: map.repos_root.clone(),
            rev: map.floor,
            uuid: map.uuid.clone(),
        }));
        msg.push('\n');
        return msg;
    }

    let mut parts = Vec::new();
    if !map.revs.is_empty() {
        parts.push(format!("另有 {} 个文件在更新的 revision", map.revs.len()));
    }
    if !map.gone.is_empty() {
        parts.push(format!("{} 个文件已删除", map.gone.len()));
    }
    msg.push_str(&format!("svn: r{}（{}）\n\n", map.floor, parts.join("，")));
    push_summary(&mut msg, &summary);
    msg.push_str(&format!("{REPO_KEY}{} {}\n", map.repos_root, map.uuid));
    msg.push_str(&format!("{FLOOR_KEY}{}\n", map.floor));
    // BTreeMap 的迭代顺序就是按路径排序
    for (p, r) in &map.revs {
        msg.push_str(&format!("{REV_KEY}{r} {}\n", encode_path(p)));
    }
    for (p, r) in &map.gone {
        msg.push_str(&format!("{GONE_KEY}{r} {}\n", encode_path(p)));
    }
    if let Some(r) = delivered_rev {
        msg.push_str(&format!("{DELIVERED_REV_KEY} {r}\n"));
    }
    msg
}

fn push_summary(msg: &mut String, summary: &[String]) {
    if summary.is_empty() {
        return;
    }
    for l in summary {
        msg.push_str(l);
        msg.push('\n');
    }
    msg.push('\n');
}

/// 一条摘要 → message 里的一行；空白行丢掉。
///
/// - 控制字符换成空格：摘要里夹着换行会凭空多出段落，trailer 就不再是最后一段
/// - 以记账键开头的行前面垫两个空格：`parse_trailer` 与 `parse_delivered_rev`
///   逐行扫全文、按行首匹配，摘要里的路径若恰好以 `git-svn-id:` 开头就会被当成记账
fn summary_line(raw: &str) -> Option<String> {
    let line: String = raw
        .chars()
        .map(|c| {
            if (c as u32) < 0x20 || c == '\u{7f}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    if line.trim().is_empty() {
        return None;
    }
    if RESERVED_PREFIXES.iter().any(|k| line.starts_with(k)) {
        return Some(format!("  {line}"));
    }
    Some(line)
}

/// 从 commit message 读回记账；读不出来就是 `None`。
///
/// 最后一段里只要出现 `Svn-Base-` 行，就按新格式**严格**解析，失败即 `None`，
/// 不再退回旧格式去猜（与 `parse_trailer` 的「不猜、不部分接受」一致）。
/// 否则退回 `parse_trailer`：旧格式的基线 → [`BaseMap::single`]。
///
/// `load_graph` 对最多 5000 条 commit 逐条调用，绝大多数连 `Svn-Base-` 这几个字都没有：
/// 先整段找一次，不必切段落。
pub fn parse_message(msg: &str) -> Option<BaseMap> {
    if msg.contains(BASE_PREFIX) {
        let last = last_paragraph(msg);
        if last.iter().any(|l| l.starts_with(BASE_PREFIX)) {
            return parse_new_trailer(&last);
        }
    }
    let t = parse_trailer(msg)?;
    Some(BaseMap::single(t.url, t.uuid, t.rev))
}

/// 最后一个非空段落的各行。`lines()` 已去掉 `\r\n` 的 `\r`。
///
/// 内容行**不 trim**：路径末尾可能是全角空格之类的 Unicode 空白，
/// `trim_end` 会把它当行尾空白削掉，读回来就差一个字符。
fn last_paragraph(msg: &str) -> Vec<&str> {
    let mut paras: Vec<Vec<&str>> = vec![Vec::new()];
    for l in msg.lines() {
        if l.trim().is_empty() {
            if !paras.last().is_some_and(|p| p.is_empty()) {
                paras.push(Vec::new());
            }
        } else if let Some(p) = paras.last_mut() {
            p.push(l);
        }
    }
    paras
        .into_iter()
        .rev()
        .find(|p| !p.is_empty())
        .unwrap_or_default()
}

fn parse_new_trailer(lines: &[&str]) -> Option<BaseMap> {
    let mut repo: Option<(String, String)> = None;
    let mut floor: Option<u64> = None;
    let mut revs = BTreeMap::new();
    let mut gone = BTreeMap::new();
    for &l in lines {
        if let Some(v) = l.strip_prefix(REPO_KEY) {
            // 只允许一行：两行 Repo 说明 message 被拼接或篡改过
            if repo.is_some() {
                return None;
            }
            // uuid 不含空格，root 本身也不许有空白（与 `parse_trailer` 对 url 的要求一致）
            let (root, uuid) = v.rsplit_once(' ')?;
            if root.is_empty()
                || root.chars().any(char::is_whitespace)
                || uuid.is_empty()
                || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
            {
                return None;
            }
            repo = Some((root.to_string(), uuid.to_string()));
        } else if let Some(v) = l.strip_prefix(FLOOR_KEY) {
            if floor.is_some() {
                return None;
            }
            floor = Some(parse_rev(v)?);
        } else if let Some(v) = l.strip_prefix(REV_KEY) {
            let (r, p) = parse_entry(v)?;
            if revs.insert(p, r).is_some() {
                return None;
            }
        } else if let Some(v) = l.strip_prefix(GONE_KEY) {
            let (r, p) = parse_entry(v)?;
            if gone.insert(p, r).is_some() {
                return None;
            }
        } else if let Some(v) = l
            .strip_prefix(DELIVERED_REV_KEY)
            .and_then(|v| v.strip_prefix(' '))
        {
            // 由 `parse_delivered_rev` 读，这里只校验形状
            parse_rev(v)?;
        } else {
            // 最后一段里出现不认识的行：不是我们写的，不猜
            return None;
        }
    }
    let (repos_root, uuid) = repo?;
    let floor = floor?;
    // floor 是映射里的最小值；比它还小的条目说明记账自相矛盾
    if revs.values().any(|&r| r < floor) {
        return None;
    }
    // 同一个路径不能既在基线树里、又是墓碑
    if revs.keys().any(|p| gone.contains_key(p)) {
        return None;
    }
    Some(BaseMap {
        repos_root,
        uuid,
        floor,
        revs,
        gone,
    })
}

fn parse_rev(s: &str) -> Option<u64> {
    // 只认纯数字：`u64::from_str` 会接受前导 `+`
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `<rev> <编码路径>`
fn parse_entry(v: &str) -> Option<(u64, String)> {
    let (r, enc) = v.split_once(' ')?;
    let p = decode_path(enc)?;
    if p.is_empty() {
        return None;
    }
    Some((parse_rev(r)?, p))
}

/// 百分号编码：`%`、空格以及 < 0x20 的字符（含 `\t` `\r` `\n`）写成 `%XX`。
///
/// trailer 一行一条、字段按空格切，路径里的这些字符会打断格式；其余字符
/// （含中文）原样保留，提交说明里仍然读得懂。
pub fn encode_path(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    for c in p.chars() {
        if c == '%' || c == ' ' || (c as u32) < 0x20 {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// [`encode_path`] 的逆。截断或非十六进制的 `%XX`、本该被编码却原样出现的字符、
/// 解码后不是 UTF-8 的字节序列，一律 `None`。
pub fn decode_path(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                if !hex.bytes().all(|h| h.is_ascii_hexdigit()) {
                    return None;
                }
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            // 空格（0x20）与控制字符本该被编码，原样出现说明不是我们写的
            c if c <= b' ' => return None,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// 读 `rev` 这个 commit 的记账。
///
/// 用 `cat-file commit` 取原始字节而不是 `log --format=%B`：后者会多一个结尾 `\n`（W27），
/// 且这里要的是写进去的原样。读不出记账沿用 `svn_baseline_no_trailer`。
pub fn read_base_map(repo: &str, rev: &str) -> Result<BaseMap> {
    let raw = run_git_bytes(repo, &["cat-file", "commit", rev])?;
    // 头部到第一个空行为止（`gpgsig` 之类的续行以空格开头，不会是空行）
    let body = raw
        .windows(2)
        .position(|w| w == b"\n\n")
        .map(|at| &raw[at + 2..])
        .unwrap_or(&[]);
    // 我们始终写 UTF-8；不是 UTF-8 就不是我们写的，不用 lossy 去猜（替换字符会混进路径）
    let parsed = std::str::from_utf8(body).ok().and_then(parse_message);
    parsed.ok_or_else(|| {
        GitError::new(
            "svn_baseline_no_trailer",
            format!(
                "commit {rev} 上没有可解析的基线记账（git-svn-id 或 Svn-Base-* trailer），\
                 它不是本应用录出来的基线。请重建基线。"
            ),
        )
    })
}
