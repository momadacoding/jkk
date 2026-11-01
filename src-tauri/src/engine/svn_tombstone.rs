//! 墓碑核对与服务器 HEAD（分析 §4.1、§6.2、§6.4；计划 Task 4）。
//!
//! 墓碑 `g → d` 是一句可被服务器证伪的话：「服务器在 d 上没有 g」。这里只负责问服务器，
//! 什么时候问、问出来怎么用由 `svn_health::check_baseline_map` 决定。
//!
//! # 为什么批量走 `svn info --xml --targets`（E16）
//!
//! - 一个跟进可能要确认几十上百个路径，逐条起进程太慢；命令行又有长度上限，`--targets`
//!   读文件，没有这个问题
//! - 不存在的 target 让整条命令 rc=1，但**存在的照常输出到 stdout 的 XML**，每个不存在的
//!   在 stderr 里一条 W170000，末尾一条 E200009。所以 rc=1 不等于全部失败
//! - XML 的 `path` 属性只有文件名，要按 `<url>` 对回
//! - revision 不存在是 E160006（另一回事），stdout 里的 XML 还会被截断在半路

use crate::engine::svn_baseline::TempFile;
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_xml::parse_info;
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};

/// 服务器 HEAD revision。`<root>@HEAD` 的 peg 写法顺带避开 URL 里含 `@` 的歧义。
///
/// 连不上服务器报 `svn_server_unreachable`。交付侧要的是另一个错误码和文案，
/// 走 [`server_head_rev_with`]。
pub fn server_head_rev(repo: &str, repos_root: &str) -> Result<u64> {
    server_head_rev_with(repo, repos_root, |raw| {
        GitError::new(
            "svn_server_unreachable",
            format!("问服务器当前 revision 失败了。svn 原话：{raw}"),
        )
    })
}

/// 同 [`server_head_rev`]，`svn info` 跑不成时用 `on_fail(svn 原话)` 造错误。
pub(crate) fn server_head_rev_with(
    repo: &str,
    repos_root: &str,
    on_fail: impl FnOnce(&str) -> GitError,
) -> Result<u64> {
    fault::hit(fault::Point::ServerHead)?;
    let target = format!("{repos_root}@HEAD");
    let xml = run_svn(repo, &["info", "--xml", "--non-interactive", &target])
        .map_err(|e| on_fail(e.message.trim()))?;
    let e = parse_info(&xml)?
        .into_iter()
        .next()
        .ok_or_else(|| GitError::new("svn_xml", "svn info 没有返回任何 entry"))?;
    u64::try_from(e.revision).map_err(|_| {
        GitError::new(
            "svn_xml",
            format!("svn info 给了一个负的 revision：{}", e.revision),
        )
    })
}

/// 逐个确认 `url@rev` 在服务器上存不存在。返回 `rel → 存在`。
///
/// - `urls_at`：`(仓库相对路径, <url>@<rev>)`。url 部分应当已经百分号编码（[`encode_url_path`]），
///   这里再把剩下的非 ASCII 与空白编码一遍：`--targets` 文件按本机代码页读，纯 ASCII 才不受影响
/// - 空输入不发命令
///
/// # 判定
///
/// - stdout 里出现了的 `<url>` → 存在
/// - 没出现的 → 不存在，但前提是 stderr 里 W170000 的条数正好等于没出现的个数。对不上
///   （比如 rc=0 却少了一条、或混进了别的错误码）说明不是「不存在」这一种原因，报错，不猜
/// - E160006（revision 不存在）报 `svn_tombstone_no_such_rev`
pub fn confirm_absent(repo: &str, urls_at: &[(String, String)]) -> Result<BTreeMap<String, bool>> {
    if urls_at.is_empty() {
        return Ok(BTreeMap::new());
    }
    fault::hit(fault::Point::ConfirmAbsent)?;
    // 同一个 url 只问一次：重复的 target 会让 W170000 的条数对不上
    let mut targets: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut rel_url: Vec<(&str, String)> = Vec::with_capacity(urls_at.len());
    for (rel, url_at) in urls_at {
        let t = ascii_safe(url_at);
        // 形状不对的先拦下，不起进程
        target_key(&t)?;
        if seen.insert(t.clone()) {
            targets.push(t.clone());
        }
        rel_url.push((rel.as_str(), t));
    }

    let mut body = targets.join("\n");
    body.push('\n');
    let file = TempFile::for_svn_arg(repo, "targets", body.as_bytes())?;
    let path = file.svn_arg(repo);
    let out = run_svn_code(repo, &["info", "--xml", "--targets", &path])?;

    let codes = error_codes(&out.stderr);
    if codes.iter().any(|c| c == "E160006") {
        return Err(GitError::new(
            "svn_tombstone_no_such_rev",
            format!(
                "核对删除记录时服务器说 revision 不存在。svn 原话：{}",
                out.stderr.trim()
            ),
        ));
    }
    if let Some(other) = codes.iter().find(|c| *c != "W170000" && *c != "E200009") {
        return Err(GitError::new(
            "svn_tombstone_check_failed",
            format!(
                "核对删除记录时 svn 报了 {other}，没法据此判断文件在不在。svn 原话：{}",
                out.stderr.trim()
            ),
        ));
    }
    let entries = parse_info(&out.stdout).map_err(|e| {
        GitError::new(
            "svn_tombstone_check_failed",
            format!(
                "核对删除记录时读不懂 svn info 的输出：{}；svn 原话：{}",
                e.message,
                out.stderr.trim()
            ),
        )
    })?;
    // 两边都解码再比：svn 输出的 url 用它自己的编码规则（`@` 不编码、`%20` 等），
    // 与我们写进 targets 的写法不必逐字相同。
    //
    // 键里要带 revision：同一个路径可能在两个 revision 上各问一次（一个在、一个不在），
    // 只按 url 对回会把不在的那条也当成在。entry 的 revision 属性就是 peg revision
    // （实测 `A.cs@2` 的 entry 是 `revision="2"`，与最后修改 revision 无关）。
    let found: BTreeSet<(String, u64)> = entries
        .iter()
        .filter_map(|e| {
            let url = decode_url(e.url.trim_end_matches('/'))?;
            Some((url.to_ascii_lowercase(), u64::try_from(e.revision).ok()?))
        })
        .collect();

    let mut absent_targets = 0usize;
    for t in &targets {
        if !found.contains(&target_key(t)?) {
            absent_targets += 1;
        }
    }
    let warned = codes.iter().filter(|c| *c == "W170000").count();
    if warned != absent_targets {
        return Err(GitError::new(
            "svn_tombstone_check_failed",
            format!(
                "核对删除记录时有 {absent_targets} 个路径没有出现在 svn info 的输出里，\
                 但 svn 只对 {warned} 个报了「不存在」（W170000）。两者对不上，没法判断。\
                 svn 原话：{}",
                out.stderr.trim()
            ),
        ));
    }

    let mut result = BTreeMap::new();
    for (rel, t) in rel_url {
        result.insert(rel.to_string(), found.contains(&target_key(&t)?));
    }
    Ok(result)
}

/// `<url>@<rev>` → 与 `(<url>, revision)` 比对的键：拆出 peg、解码、ASCII 小写
/// （`file:///C:/` 的盘符大小写不保证一致）。没有数字 peg 的一律报错：不带 peg 时
/// svn 取 HEAD，问的就不是调用方想问的那个 revision。
fn target_key(url_at: &str) -> Result<(String, u64)> {
    let bad = || {
        GitError::new(
            "svn_tombstone_check_failed",
            format!("核对删除记录用的 `{url_at}` 不是「百分号编码的 URL@revision」"),
        )
    };
    let (url, rev) = url_at.rsplit_once('@').ok_or_else(bad)?;
    if rev.is_empty() || !rev.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad());
    }
    let rev: u64 = rev.parse().map_err(|_| bad())?;
    let url = decode_url(url.trim_end_matches('/')).ok_or_else(bad)?;
    Ok((url.to_ascii_lowercase(), rev))
}

/// stderr 里的 svn 错误码（`E123456` / `W123456`），按出现顺序。
///
/// 只认码不认文字：svn 的提示语会被本地化，码不会。
pub(crate) fn error_codes(stderr: &str) -> Vec<String> {
    let b = stderr.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= b.len() {
        let is_code = (b[i] == b'E' || b[i] == b'W')
            && b[i + 1..i + 7].iter().all(u8::is_ascii_digit)
            && b[i + 7] == b':'
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if is_code {
            out.push(stderr[i..i + 7].to_string());
            i += 8;
        } else {
            i += 1;
        }
    }
    out
}

/// 把仓库相对路径（相对某个 URL）编码成 URL 的路径部分。
///
/// 除 `A-Z a-z 0-9 - . _ ~ /` 以外的字节一律 `%XX`。`@` 也编码：svn 把 target 里**最后
/// 一个** `@` 当 peg，路径里的 `@` 不编码就会和后面追加的 `@<rev>` 抢位置；`%` 编码成
/// `%25`，否则路径里的 `%20` 会被 svn 当成空格。
pub fn encode_url_path(rel: &str) -> String {
    let mut out = String::with_capacity(rel.len());
    for &b in rel.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 只编码非 ASCII 与空白 / 控制字符，已有的 `%XX` 原样保留。用在已经是 URL 的字符串上。
fn ascii_safe(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for &b in url.as_bytes() {
        if b <= b' ' || b >= 0x7f {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    out
}

/// 百分号解码。截断或非十六进制的 `%XX`、解码后不是 UTF-8 的，一律 `None`。
///
/// svn 输出的 URL（`svn info` 的 `<url>`、`svn diff --summarize` 的路径）都是编码过的：
/// 空格是 `%20`、中文是 UTF-8 的 `%XX` 序列、`%` 本身是 `%25`。
pub fn decode_url(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            if !hex.bytes().all(|h| h.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// 测试注入点：让体检里某一次「向服务器确认」或「扫描被排除节点」必定失败。
///
/// # 为什么需要它（Review 1 第 1 条）
///
/// 「D 已知、确认失败」这条路径要求 diff 成功、紧接着的那次查询失败，fixture 用的
/// `file://` 仓库造不出这种时序（同一个仓库，要么都通要么都不通）。注入点是线程局部的：
/// 体检全程在调用线程上同步执行，`cargo test` 并行跑的其他测试碰不到；没 arm 时每个
/// 注入点只是一次线程局部读。
///
/// 只给集成测试用，`#[doc(hidden)]`；生产代码里没有任何地方 arm 它。
#[doc(hidden)]
pub mod fault {
    use crate::error::{GitError, Result};
    use std::cell::Cell;

    /// 可注入失败的位置。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Point {
        /// [`super::server_head_rev`]
        ServerHead,
        /// [`super::confirm_absent`]
        ConfirmAbsent,
        /// 体检里每单元一次的 `svn info -R`（找被 depth-exclude 的节点）
        ExcludedScan,
        /// 体检里对点不了名的被排除节点跑的 `svn ls -R`（D8）
        ListAtFloor,
        /// 体检的第一条 `svn diff`
        FirstDiff,
        /// 体检第二条起的 `svn diff`
        LaterDiff,
        /// 把 `svn diff` 输出里的路径还原成仓库相对路径
        DiffPath,
    }

    thread_local! {
        static ARMED: Cell<Option<Point>> = const { Cell::new(None) };
    }

    /// 本线程上 `p` 此后每次都失败，直到返回的守卫被 drop。
    #[must_use = "守卫一 drop，注入就撤销"]
    pub fn arm(p: Point) -> Armed {
        ARMED.with(|a| a.set(Some(p)));
        Armed(())
    }

    /// [`arm`] 的守卫。
    pub struct Armed(());

    impl Drop for Armed {
        fn drop(&mut self) {
            ARMED.with(|a| a.set(None));
        }
    }

    /// 注入点本身：`p` 被 arm 时报 `svn_fault_injected`。
    pub(crate) fn hit(p: Point) -> Result<()> {
        if ARMED.with(|a| a.get()) == Some(p) {
            return Err(GitError::new(
                "svn_fault_injected",
                format!("测试注入的失败：{p:?}"),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_picked_out_of_localized_text() {
        let s = "svn: warning: W170000: URL 'x' non-existent\n\nsvn: E200009: 有些不存在\n";
        assert_eq!(error_codes(s), vec!["W170000", "E200009"]);
        assert!(error_codes("svn: XE1234567: nope").is_empty());
    }

    #[test]
    fn targets_file_lives_under_git_dir_with_an_ascii_relative_arg() {
        // Review 1 第 7 条：路径传给 svn 要过系统代码页。放 `%TEMP%` 时用户名里有代码页表示不了
        // 的字符就必败；`.git/` 下的相对路径是纯 ASCII，仓库路径是中文也不受影响
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("仓库");
        std::fs::create_dir(&repo).unwrap();
        let repo = repo.to_string_lossy().to_string();
        crate::engine::git_cli::run_git(&repo, &["init", "-q"]).unwrap();

        let file = TempFile::for_svn_arg(&repo, "targets", b"x\n").unwrap();
        let arg = file.svn_arg(&repo);
        assert!(arg.is_ascii() && arg.starts_with(".git"), "{arg}");
        let on_disk = std::path::Path::new(&repo).join(&arg);
        assert!(on_disk.is_file(), "{on_disk:?}");
        drop(file);
        assert!(!on_disk.exists(), "用完即删");
    }

    #[test]
    fn targets_file_falls_back_to_temp_dir_outside_a_git_repo() {
        // 体检不要求是 git 仓库（纯 SVN 工作副本）
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_string_lossy().to_string();
        let file = TempFile::for_svn_arg(&repo, "targets", b"x\n").unwrap();
        let arg = file.svn_arg(&repo);
        assert!(std::path::Path::new(&arg).is_absolute(), "{arg}");
        assert!(std::path::Path::new(&arg).is_file(), "{arg}");
    }

    #[test]
    fn url_path_encoding_round_trips_and_hides_peg_and_percent() {
        let p = "Code/sp ace/中文 %x@y.cs";
        let enc = encode_url_path(p);
        assert!(!enc.contains('@') && !enc.contains(' '));
        assert!(enc.is_ascii());
        assert_eq!(decode_url(&enc).as_deref(), Some(p));
        assert_eq!(decode_url("a%2"), None);
        assert_eq!(decode_url("a%zz"), None);
    }
}
