//! P5：白名单覆盖到的 `svn:externals` 有没有被 `-r` 钉死（spec §6.5.2 手段一）。
//!
//! # 命中不阻断
//!
//! 初版 P5 是硬阻断 + 要求团队改 `svn:externals` 定义。那条路走不通——改别人的
//! externals 定义不是一个客户端能要求的事，而且实测表明客户端自己就能绕开
//! （显式指名那个目录 `svn update` 就不认 peg）。所以本模块**只报告，不报错**：
//! 返回一份清单，流程据此切到 pinned 模式（spec §6.5.3）。
//!
//! # 这里只有手段一
//!
//! spec §6.5.2 还有一条「手段二：行为校验」——`svn update` 成功之后未被钉住的
//! external 必然停在该仓库 HEAD，不等即疑似被钉住。它不依赖任何文本解析，负责
//! 兜住手段一的盲区。**本期不做**：它要在一次真实 `svn update` 之后才能取数，
//! 而本期根本不发 `update`（那是 §7.3 的 S5，属于同步流程）。记账给 Phase 2。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::parse_externals_props;
use crate::error::Result;

/// 一行 `svn:externals` 定义解析出来的东西。
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalDef {
    /// external 在**定义所在目录**下的本地路径，正斜杠。
    pub local_path: String,
    /// external 的 URL 原文（`^/lib`、`http://…`），**peg 后缀已剥离**。
    pub url: String,
    /// 钉死到的 revision；`None` 表示跟随 HEAD。
    pub pinned_rev: Option<u64>,
}

/// 一条被钉死、且与白名单有交集的 external。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedExternal {
    /// 定义这条 external 的目录，仓库内相对路径（正斜杠）；定义在仓库根上时是空串。
    pub defined_in: String,
    /// external 落在**仓库内**的完整相对路径 = `defined_in` + 定义里的本地路径。
    ///
    /// 这个才是拿来和白名单比对、显示给用户、以及 pinned 模式下显式
    /// `svn update <这里>` 的那个路径（spec §6.5.3 第 1 条）。
    pub local_path: String,
    pub pinned_rev: u64,
}

/// 解析一行 `svn:externals` 定义。**纯函数，不发任何命令。**
///
/// # 为什么不能按列取字段
///
/// 两种格式的字段顺序是**相反**的：
///
/// | 格式 | 形状 | 例子 |
/// |---|---|---|
/// | 新 | `[-r N] URL LOCAL` | `-r1 ^/lib pinA` |
/// | 旧 | `LOCAL [-r N] URL` | `pinD -r1 ^/lib` |
///
/// 而且两者可以混在**同一个属性值**里（实测五种写法共存）。所以判别哪个是 URL
/// 只能看 token 本身长不长得像 URL，不能看它排第几。
///
/// # 判据：`-r <N>` 记号，**或** URL 带 `@<N>` 后缀
///
/// 2026-09-14 实测四种钉法全部生效：`-r1` / `-r 1` / `^/lib@1` / 旧格式 `-r1`。
/// **`@<N>` 那种最容易漏，因为整行不含 `-r`。**
///
/// 解析不出来时返回 `None` 而不是 panic：旧格式配 `^/` 相对 URL 是一种签出会
/// 直接失败的写法（spec §6.5.2 实测），它本来就没法用，但不该让整个检测挂掉。
pub fn parse_external_line(line: &str) -> Option<ExternalDef> {
    let toks: Vec<&str> = line.split_whitespace().collect();
    let mut operative: Option<u64> = None;
    let mut rest: Vec<&str> = Vec::new();

    let mut i = 0;
    while i < toks.len() {
        let t = toks[i];
        if t == "-r" {
            // `-r 1`：数字在下一个 token。取不到或不是数字 → 整行作废。
            operative = Some(toks.get(i + 1)?.parse::<u64>().ok()?);
            i += 2;
            continue;
        }
        if let Some(d) = t.strip_prefix("-r") {
            operative = Some(d.parse::<u64>().ok()?);
            i += 1;
            continue;
        }
        rest.push(t);
        i += 1;
    }

    // 去掉 `-r N` 之后必须正好剩「一个 URL + 一个本地路径」。多了少了都不认。
    if rest.len() != 2 {
        return None;
    }
    let url_idx = match (looks_like_url(rest[0]), looks_like_url(rest[1])) {
        (true, false) => 0,
        (false, true) => 1,
        // 两个都像或都不像 URL 时无从判别，宁可不认也不猜
        _ => return None,
    };
    let (url, peg) = split_peg(rest[url_idx]);
    Some(ExternalDef {
        local_path: rest[1 - url_idx].replace('\\', "/"),
        url,
        // `-r` 是 operative revision，`@` 是 peg；内容来自前者，所以它优先。
        pinned_rev: operative.or(peg),
    })
}

/// 找出白名单覆盖到的、被钉死的 external。
///
/// `svn propget svn:externals -R --xml` 从**仓库根**递归取——要找的是「白名单
/// **所在**的那条 external」，它定义在白名单的上方，只查白名单子树看不到它。
///
/// 过滤条件是**双向**的：external 包住白名单（`Code/` 落在被钉死的 external 里），
/// 或白名单包住 external（`Code/` 底下挂着一条被钉死的 external）。两种情况下
/// 那条 external 的内容都会进基线，递归 update 把它退回 peg 时都会出问题。
/// 与白名单无交集的 external 不报——报了只是噪音，还会让用户以为自己得去改一个
/// 跟他无关的定义。
///
/// 空白名单一条命令都不发，返回空列表（对齐 `enumerate_wcs`，spec §12.2）。
pub fn detect_pinned(repo_root: &str, whitelist: &[String]) -> Result<Vec<PinnedExternal>> {
    let roots = normalize_whitelist(whitelist);
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    let xml = run_svn(repo_root, &["propget", "svn:externals", "-R", "--xml"])?;
    let mut out: Vec<PinnedExternal> = Vec::new();
    for prop in parse_externals_props(&xml)? {
        // propget 给的 <target path> 是**绝对路径**（实测 E7，与 status 的相对
        // 反斜杠完全不同形状），先换算成仓库内相对路径。
        let Some(defined_in) = rel_to_root(repo_root, &prop.target) else {
            continue;
        };
        for line in prop.raw.lines() {
            let Some(def) = parse_external_line(line) else {
                continue;
            };
            let Some(rev) = def.pinned_rev else {
                continue;
            };
            let path = join_rel(&defined_in, &def.local_path);
            if !overlaps(&path, &roots) {
                continue;
            }
            out.push(PinnedExternal {
                defined_in: defined_in.clone(),
                local_path: path,
                pinned_rev: rev,
            });
        }
    }
    out.sort_by(|a, b| a.local_path.cmp(&b.local_path));
    Ok(out)
}

/// 这个 token 是不是一个 external URL。
///
/// SVN 认四种相对写法（`^/` 仓库根、`//` 同协议、`/` 同主机、`../` 同目录）
/// 加上完整 URL。本地路径不会长成这些样子。
fn looks_like_url(t: &str) -> bool {
    t.starts_with("^/")
        || t.starts_with("//")
        || t.starts_with("../")
        || t.starts_with('/')
        || t.contains("://")
}

/// 剥掉 URL 尾部的 peg 后缀。只有「最后一个 `@` 之后全是数字」才算 peg。
///
/// 不能一见 `@` 就当 peg：`^/team@home/lib` 是一个合法路径，误判会把它的 URL
/// 截断成 `^/team`，还凭空报出一条「钉死」。
fn split_peg(url: &str) -> (String, Option<u64>) {
    let Some(i) = url.rfind('@') else {
        return (url.to_string(), None);
    };
    let (head, tail) = (&url[..i], &url[i + 1..]);
    if tail.is_empty() {
        // 空 peg 后缀（`^/lib@`）是「这个 @ 是字面量」的转义写法，不是钉死
        return (head.to_string(), None);
    }
    match tail.parse::<u64>() {
        Ok(n) => (head.to_string(), Some(n)),
        Err(_) => (url.to_string(), None),
    }
}

fn normalize_whitelist(whitelist: &[String]) -> Vec<String> {
    whitelist
        .iter()
        .map(|w| w.replace('\\', "/").trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// 绝对路径换算成仓库内相对路径；不在仓库内则返回 `None`。
///
/// 大小写不敏感比较：Windows 上 svn 回的盘符大小写不保证与调用方传进来的一致。
fn rel_to_root(repo_root: &str, abs: &str) -> Option<String> {
    let root = repo_root.replace('\\', "/");
    let root = root.trim_end_matches('/');
    let path = abs.replace('\\', "/");
    let path = path.trim_end_matches('/');
    if path.eq_ignore_ascii_case(root) {
        return Some(String::new());
    }
    let prefix = format!("{root}/");
    // 按字节比，避免在多字节字符中间切片 panic。前缀末尾是 `/`，所以只要字节段
    // 匹配，prefix.len() 这个位置必定是字符边界。
    if path.len() > prefix.len()
        && path.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
    {
        return Some(path[prefix.len()..].to_string());
    }
    None
}

fn join_rel(base: &str, child: &str) -> String {
    if base.is_empty() {
        child.trim_start_matches('/').to_string()
    } else {
        format!("{}/{}", base, child.trim_start_matches('/'))
    }
}

/// 这条 external 与白名单有没有交集（任一方向）。
fn overlaps(path: &str, roots: &[String]) -> bool {
    roots.iter().any(|w| {
        w == path || w.starts_with(&format!("{path}/")) || path.starts_with(&format!("{w}/"))
    })
}
