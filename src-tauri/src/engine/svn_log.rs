//! `svn log` 的拉取编排。
//!
//! 本模块只负责**打给谁、打什么区间、失败算谁的**；解析归 [`crate::engine::svn_xml`]，
//! 缓存归 [`crate::engine::svn_log_cache`]。
//!
//! # 为什么是「每个工作副本一次调用」
//!
//! 2026-09-15 实测（临时 fixture，SlikSvn 1.14.2）：
//!
//! | 写法 | 结果 |
//! |---|---|
//! | `svn log <url1> <url2>` | ❌ `E205000: Only relative paths can be specified after a URL` |
//! | `svn log <url> Code Other` | ✅ 成功，返回**并集**，降序去重 |
//! | `svn log . Code` | ❌ `E200007: When specifying working copy paths, only one target may be given` |
//! | `svn log <url> Code NoSuch` | ❌ `E160013: File not found` —— **一条坏路径炸掉整次调用** |
//!
//! 第二行那个形态看起来能把 N 次调用压成 1 次，**但没有采用**，理由是第四行：
//! 白名单里的目录被上游删掉、或在早期 revision 上还不存在，都是正常情况，
//! 而本设计的白名单**天然可能分布在不同的 external / 工作副本**里
//! （spec §6.6.5），URL 前缀本来就不止一个。
//! 每个 WC 一次调用的话，一个失败只损失那一个 WC 的历史。
//!
//! # ⚠️ 这里的 target 绝不能过 `svn_wc::peg_safe`
//!
//! 实测 R5：`svn log <URL> "Code@"` → `E160013: File not found: path '/Code@'`。
//! 与 `svn info` / `svn status`（**必须**加空 peg 后缀）正好相反。这是本仓库
//! 第三处 peg 规则互相矛盾的地方，另两处见 `svn_wc::peg_safe` 的文档。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::{parse_log, SvnLogEntry};
use crate::error::{GitError, Result};

/// 读某个 URL 当前的 HEAD revision。
///
/// 走 `svn info --show-item revision`：实测（P7）stdout 就是裸的十进制数字，
/// 不用过 XML 解析。
///
/// ⚠️ **`svn info` 绝不能带 `--ignore-externals`**（实测 exit 1，它拒绝这个参数）。
pub fn head_revision(repo_root: &str, url: &str) -> Result<u64> {
    let out = run_svn(repo_root, &["info", "--show-item", "revision", "--", url])?;
    let text = out.trim();
    text.parse::<u64>().map_err(|_| {
        GitError::new(
            "svn_log_bad_head",
            format!("svn info --show-item revision 返回了不是数字的内容：`{text}`"),
        )
    })
}

/// 拉一段区间的 log。**区间两端都会先被钳到 `[1, HEAD]`**，方向保持不变。
///
/// # 为什么必须钳制，而不是把错误抛上去
///
/// 实测 P4/P5：`-r` 的**任一端**越界，整条命令 `exit 1` 并报
/// `E160006: No such revision N`。而增量拉取的自然写法就是
/// 「上次覆盖到 N，这次拉 `N+1 : HEAD`」——没有新提交时 `N+1 > HEAD`，
/// 于是**每一次「没有新东西」都会变成一次错误**，界面上就会挂出一个
/// 「SVN 历史拉取失败」而其实什么事都没有。判成「拉到 0 条」才是它真实的含义。
///
/// `limit == 0` 同理：实测 P6 `--limit 0` 直接 `E200004`，而 limit 是调用方算出来
/// 的剩余配额，算到 0 是正常的。
pub fn fetch_range(
    repo_root: &str,
    url: &str,
    from: u64,
    to: u64,
    limit: u32,
) -> Result<Vec<SvnLogEntry>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let head = head_revision(repo_root, url)?;
    if head == 0 {
        // 空仓库。实测 P9：svn log 对 HEAD=0 返回 exit 0 + 空 <log>，
        // 但我们连命令都不用发。
        return Ok(Vec::new());
    }
    // **整段**都在 HEAD 之上时直接返回空。
    // 少了这一条，下面的钳制会把 `5:9`（HEAD=4）压成 `4:4`，于是「没有新东西」
    // 被拉回一条早就有的记录 —— 比报错更糟，因为它看起来是成功的。
    if from > head && to > head {
        return Ok(Vec::new());
    }
    let clamp = |v: u64| v.clamp(1, head);
    // 不排序、不交换：区间方向决定 svn 的输出顺序（实测 M3/M4），
    // 调用方按用途选方向，这里原样转发。
    let range = format!("{}:{}", clamp(from), clamp(to));
    let limit_s = limit.to_string();
    let xml = run_svn(
        repo_root,
        &["log", "--xml", "-r", &range, "--limit", &limit_s, "--", url],
    )?;
    parse_log(&xml)
}

/// 补拉一个**孤儿 baseline** 的锚点 commit。
///
/// # 这不是边界情况，是常态（2026-09-15 实测，C 组）
///
/// spec §10.3 把孤儿 baseline 描述成「若某个 baseline 的 revision **太老**」。
/// 成因与新旧无关：`svn update` 会把工作副本内**所有**节点的 `wc-status revision`
/// 一律推到当时的 HEAD，**不管那个 revision 有没有动过它们**。所以
/// `baselineRev`（取自 `nodeRevs`）通常就是一个根本没碰过白名单的 revision。
///
/// 实测对照：
///
/// ```text
/// svn log -r 4:4 <url>/Code            → 空          （Q1）
/// svn log -r 4:4 <Repository Root>     → 拿到 r4     （Q2）
/// ```
///
/// **所以锚点必须打给 Repository Root。** 打给白名单 URL 会拿到空结果，
/// 图上那个 `svn:N` 节点就永远建不出来，工作分支挂不上主干。
///
/// revision 高于 HEAD 时返回 `Ok(None)` 而不是 `Err`：缓存里记着的 baseline
/// 可能指向本仓库没有的 revision（换了远端、或仓库被重建过），那该跳过这一个，
/// 不该让整次拉取失败。
pub fn fetch_anchor(repo_root: &str, repos_root: &str, rev: u64) -> Result<Option<SvnLogEntry>> {
    let entries = fetch_range(repo_root, repos_root, rev, rev, 1)?;
    Ok(entries.into_iter().next())
}

/// 本仓库要拉哪些 target：每个白名单所属工作副本的 URL，各一条，已去重。
///
/// 去重用大小写不敏感比较——同一个 URL 的大小写写法在 Windows 上会有出入，
/// 重复的 target 会让同一段历史被拉两遍并在缓存里占两份。
pub fn log_targets(repo_root: &str) -> Result<Vec<String>> {
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let wcs = crate::engine::svn_wc::enumerate_wcs(repo_root, &cfg.whitelist)?;
    let mut out: Vec<String> = Vec::new();
    for wc in wcs {
        let url = wc.url.trim_end_matches('/').to_string();
        if !out.iter().any(|u| u.eq_ignore_ascii_case(&url)) {
            out.push(url);
        }
    }
    Ok(out)
}
