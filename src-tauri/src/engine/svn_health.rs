//! §6.8 基线体检：让基线可被服务器证伪。
//!
//! 基线记着「上次与 SVN 对齐时是 rN」。这条检查不去推断 revision 号对不对，
//! 而是**直接拿磁盘跟服务器上的 rN 比**——有内容差异即证伪，没有就说明基线
//! 至少在内容上站得住。
//!
//! # 它证明不了什么（spec §6.8.3）
//!
//! 比的是**磁盘**与服务器，不是**基线树**与服务器，两者之间隔着「git 跟踪的集合
//! == 白名单内应跟踪的集合」这个假设。那个假设归 `svn_include_check` 的纳入集合
//! 核对管。两条检查互不替代，不要互相冒充。
//!
//! # 前提
//!
//! 调用时必须满足「磁盘上白名单内的内容 == 基线树的内容」。spec §6.8.5 列的四个
//! 时机全部满足它：HEAD 在 `svn/base` 上且工作区干净，或基线树刚从磁盘录出来。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::parse_diff_summary;
use crate::error::{GitError, Result};

/// 体检的三种结果。**必须分开**（spec §6.8.6）。
#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    /// 命令成功，按 §6.8.4 过滤后无 W-file 内容差异 —— 基线可信。
    Verified { rev: u64 },
    /// 命令成功，仍有 W-file 内容差异 —— **基线撒谎**。阻断并列出差异文件。
    Lying { files: Vec<String> },
    /// 命令没跑成（断网 / 认证 / URL 不可达 / revision 不存在）。
    ///
    /// **这不是「基线有问题」。** 把「没能验证」当成「验证不通过」会让离线场景
    /// 寸步难行；当成「验证通过」则等于这条检查根本不存在。所以它是独立的第三种。
    NotChecked { reason: String },
}

/// 拿磁盘跟服务器上的 `rev` 比一次。
///
/// # 命令形态（spec §6.8.2）
///
/// ```text
/// svn diff --old=<svn_url>@<rev> --new=. --summarize --xml -- <白名单根...>
/// ```
///
/// - **用 `--xml`**：文本版靠列位置区分「内容变更」与「属性变更」，还会混进目录
///   条目（spec §6.8.4）。`--xml` 把这三件事拆成 `item` / `props` / `kind` 三个
///   独立属性，那个解析坑从结构上消失。
/// - **不带 `--ignore-externals`**：`svn diff` 会以 exit 1 拒绝它，而且**不需要**
///   ——URL 与工作副本目录比对时本来就不走进 externals（spec §6.8.2 R3）。
/// - **白名单根不过 `peg_safe`**。这是与 `info` / `status` 相反的一条，见下。
///
/// # ⚠️ `svn diff` 的尾随 target 不接受空 peg 后缀
///
/// 2026-09-14 实测：`svn diff --old=… --new=. -- Code@` 报
/// `E155010: The node '…\wc\Code@' was not found` —— 它把 `@` 当成了字面路径的
/// 一部分，而不是 peg 记号。`svn_wc::peg_safe` 的「无条件追加没有代价」那条结论
/// 是在 `info` / `status` 上验的，**对这里不成立**。
///
/// 代价：白名单路径里含 `@` 时这条命令没法用。那种路径本来就过不了
/// `enumerate_wcs`（它反过来**必须**加后缀），所以两边都能工作的写法目前不存在，
/// 记为已知限制。
pub fn check_baseline(
    repo_root: &str,
    svn_url: &str,
    rev: u64,
    whitelist: &[String],
    extensions: &[String],
) -> Result<Health> {
    let roots = normalize(whitelist);
    if roots.is_empty() {
        // 不传 target 时 `svn diff` 会拿**整个工作副本**跟服务器比。对一个 Unity
        // 工程那是几个 GB 的美术资产，而且任何一处正常的本地改动都会报成基线撒谎。
        // 返回 Verified 更糟——等于宣布一个空白名单的基线是健康的。
        return Err(GitError::new(
            "svn_health_empty_whitelist",
            "白名单是空的，体检没有比对范围。不传 target 会让 svn diff 拿整个\
             工作副本去比，既慢又必然误报。",
        ));
    }

    let base = svn_url.trim_end_matches('/');
    if let Some(reason) = url_mismatch(repo_root, base)? {
        return Ok(Health::NotChecked { reason });
    }

    let old = format!("--old={base}@{rev}");
    let mut args: Vec<&str> = vec!["diff", &old, "--new=.", "--summarize", "--xml", "--"];
    args.extend(roots.iter().map(String::as_str));

    // 命令失败一律落到 NotChecked，**不能**当成「没有差异」。
    //
    // 这里不能用「解析得出来就算成功」：2026-09-14 实测，URL 不可达时 svn 仍然
    // 往 stdout 打出 `<?xml …?>` 和 `<diff><paths>`，错误只在 stderr 和退出码里。
    // 照那种写法，断网时会得到一个空的 paths 列表，于是**报告基线健康**。
    let xml = match run_svn(repo_root, &args) {
        Ok(x) => x,
        Err(e) => return Ok(Health::NotChecked { reason: e.message }),
    };

    let prefix = format!("{base}/");
    let mut files: Vec<String> = Vec::new();
    for entry in parse_diff_summary(&xml)? {
        // §6.8.4 的三条过滤，靠 --xml 的三个独立属性完成：
        //   剔除目录条目
        if entry.kind != "file" {
            continue;
        }
        //   剔除纯属性变更（item="none" props="modified"）
        if entry.item == "none" {
            continue;
        }
        let Some(rel) = strip_prefix_ci(&entry.path, &prefix) else {
            // 剥不掉前缀说明 svn_url 与输出的基准 URL 对不上（最常见是把它配成了
            // 仓库里的子路径）。此时无法判断哪些是 W-file，既不能说健康也不能说
            // 撒谎——只能说没验成。
            return Ok(Health::NotChecked {
                reason: format!(
                    "svn diff 输出里的路径 `{}` 不在配置的 svnUrl `{base}` 之下，\
                     没法还原成仓库相对路径。检查 svnUrl 是不是配成了子路径。",
                    entry.path
                ),
            });
        };
        //   剔除扩展名不在白名单里的文件
        if !is_w_file(&rel, &roots, extensions) {
            continue;
        }
        files.push(rel);
    }

    if files.is_empty() {
        return Ok(Health::Verified { rev });
    }
    files.sort();
    Ok(Health::Lying { files })
}

/// `svn_url` 必须就是**工作副本根**的 URL；不是的话返回一句说得清的理由。
///
/// # 为什么值得多发一条 `svn info`
///
/// `--old=<svn_url>@<rev>` 与 `--new=.` 是一对：svn 把每个 target 分别接在两边
/// 后面去比。`svn_url` 若配成了仓库里的**子路径**（比如把白名单根的 URL 填了
/// 进来），`--old` 那侧指向 `<svn_url>/Code`，一个根本不存在的路径，于是白名单里
/// 每个文件都被报成差异。
///
/// 2026-09-14 实测：这种情况下**前缀照样剥得掉**（输出路径就是
/// `<svn_url>/<target>/…`），所以「剥不掉前缀」那道守卫拦不住它。结果是一份
/// 看起来完全合理的 `Lying` 清单——而按 spec §6.8.6，初始化时 `Lying` 会触发
/// **整个回退**。把一个配置错误变成「基线撒谎」是这条检查最坏的失败形态。
///
/// 判定不成立时返回 `NotChecked` 而不是 `Err`：这确实属于「没能验证」，
/// 而且不该阻断（spec §6.8.6 第三行）。
fn url_mismatch(repo_root: &str, base: &str) -> Result<Option<String>> {
    let xml = match run_svn(repo_root, &["info", "--xml"]) {
        Ok(x) => x,
        // 连 `svn info` 都跑不成，后面的 diff 更不可能成。归到同一个桶里。
        Err(e) => return Ok(Some(e.message)),
    };
    let Some(entry) = crate::engine::svn_xml::parse_info(&xml)?.into_iter().next() else {
        return Ok(Some("svn info 对工作副本根没有输出".to_string()));
    };
    let actual = entry.url.trim_end_matches('/');
    if actual.eq_ignore_ascii_case(base) {
        return Ok(None);
    }
    Ok(Some(format!(
        "配置里的 svnUrl 是 `{base}`，而工作副本根 `{repo_root}` 实际指向 `{actual}`。\
         体检要拿这两者当同一个位置的两侧来比，对不上就没法验证。\
         svnUrl 应当填工作副本根的 URL，不是某个白名单目录的 URL。"
    )))
}

fn normalize(whitelist: &[String]) -> Vec<String> {
    whitelist
        .iter()
        .map(|w| w.replace('\\', "/").trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// 这条仓库相对路径算不算 W-file。
///
/// # 为什么是纯路径判断，不走磁盘
///
/// `svn_include_check::expected_set` 遍历磁盘算 W-file，但这里**不能**用它：
/// T7 那类差异（服务器上删掉了 `Code/Deep/D.cs`）的文件在磁盘上根本不存在，
/// 走磁盘的判断会把它整个漏掉——而那正是 §9.4.1 那类漏检要抓的东西。
///
/// 扩展名比较大小写不敏感：Windows 上 git 会把 `A.CS` 一起收进来
/// （`core.ignoreCase` 默认 true），那它就是 W-file，体检不该当它不存在。
fn is_w_file(rel: &str, roots: &[String], extensions: &[String]) -> bool {
    let under = roots
        .iter()
        .any(|r| rel == r || rel.starts_with(&format!("{r}/")));
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

/// 大小写不敏感地剥前缀。`file:///C:/…` 里盘符的大小写不保证与配置里一致。
fn strip_prefix_ci(path: &str, prefix: &str) -> Option<String> {
    if path.len() <= prefix.len() {
        return None;
    }
    // 按字节比，避免在多字节字符中间切片。前缀末尾是 `/`，字节段匹配时
    // prefix.len() 必定落在字符边界上。
    if path.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()) {
        Some(path[prefix.len()..].to_string())
    } else {
        None
    }
}
