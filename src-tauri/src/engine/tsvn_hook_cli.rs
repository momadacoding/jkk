//! `--tsvn-hook` 子命令：TortoiseSVN 调起 jkk 的 exe 当客户端钩子（分析 §6.13.2；计划 Task 2）。
//!
//! # 进程形态
//!
//! 与 GUI 是同一个 exe。`main.rs` 在启动 Tauri 之前先问 [`tsvn_hook_main`]，返回 `Some` 就直接
//! 退出。release 构建是 GUI 子系统，被 TortoiseSVN 调起时不会闪控制台；stderr 走 TortoiseSVN
//! 给的管道，退出码照常传回（日志 H5）。
//!
//! # 绝不挡住提交
//!
//! 除了 Check-commit 命中 Local-Only 泄漏，其余任何情况一律退出 0：参数不对、收件箱写不进、
//! panic 都一样（fail-open）。钩子是附加的精确模式，坏了就退回 C1a 的推断，不能让用户提交不了。
//!
//! # 参数（日志 H2、H4）
//!
//! 命令行是 `"<exe>" --tsvn-hook <kind> --repo "<仓库根>"`，后面接 TortoiseSVN 追加的参数，
//! 位置随类型不同。`CWD` 是勾选文件的公共父目录，不是工作副本根（H3），所以靠注册时写死的
//! `--repo` 定位仓库。

use crate::engine::tsvn_inbox::{write_event, TsvnEvent, EVENT_VERSION};
use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;

const FLAG: &str = "--tsvn-hook";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HookArgs {
    pub kind: String,
    pub repo: String,
    pub path_file: Option<PathBuf>,
    pub revision: Option<u64>,
    pub error_file: Option<PathBuf>,
    pub result_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookOutcome {
    pub code: i32,
    /// 给 TortoiseSVN 显示的错误。只有 Check-commit 拦截时非空。
    pub stderr: String,
    /// `stderr` 的英文版。线程代码页装不下 `stderr` 时改发它（[`hook_stderr_bytes`]）；
    /// 空表示没有英文版，照发 `stderr`。
    pub stderr_en: String,
}

/// 盘符根在命令行里只能写成 `--repo "D:"`（`hook_command` 的注释），而 `D:` 是驱动器相对路径，
/// 相对钩子的工作目录解析，打不开仓库。这里补回结尾的 `\`。放在钩子侧，已经注册好的命令行
/// 不用重新注册（final review 第 1 条）。
fn restore_drive_root(repo: &str) -> String {
    match repo.as_bytes() {
        [d, b':'] if d.is_ascii_alphabetic() => format!("{repo}\\"),
        _ => repo.to_string(),
    }
}

/// 解析 `--tsvn-hook` 之后的参数（不含 argv0）。不是钩子调用、或参数不全时返回 `None`。
pub fn parse_hook_args(args: &[OsString]) -> Option<HookArgs> {
    let s: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    if s.first().map(String::as_str) != Some(FLAG) || s.get(2).map(String::as_str) != Some("--repo")
    {
        return None;
    }
    let kind = s.get(1)?.clone();
    let repo = restore_drive_root(s.get(3)?);
    let t = &s[4..];
    let file = |i: usize| t.get(i).map(PathBuf::from);
    let rev = |i: usize| -> Option<Option<u64>> { Some(t.get(i)?.parse().ok()) };
    // 下标见日志 H2、H4：commit 类第 0 个都是 PATH
    let (min_len, revision, error_file, result_file) = match kind.as_str() {
        "check_commit" => (3, None, None, None),
        "pre_commit" => (4, None, None, None),
        "post_commit" => (6, rev(3)?, file(4), None),
        "post_update" => (6, rev(2)?, file(3), file(5)),
        _ => return None,
    };
    if t.len() < min_len {
        return None;
    }
    Some(HookArgs {
        kind,
        repo,
        path_file: file(0),
        revision,
        error_file,
        result_file,
    })
}

/// 去掉仓库根前缀，得到正斜杠的仓库相对路径。大小写不敏感，`\` 与 `/` 等价。仓库外的路径、
/// 仓库根本身返回 `None`。
///
/// 逐字符比较而不是先整体转小写再切片：非 ASCII 字符转小写后字节长度可能变，切片会错位。
pub fn to_repo_relative(repo: &str, abs: &str) -> Option<String> {
    let repo = repo.replace('\\', "/");
    let repo = repo.trim_end_matches('/');
    let abs = abs.replace('\\', "/");
    let mut rest = abs.char_indices();
    for rc in repo.chars() {
        let (_, ac) = rest.next()?;
        if !rc.to_lowercase().eq(ac.to_lowercase()) {
            return None;
        }
    }
    let (i, sep) = rest.next()?;
    if sep != '/' {
        return None;
    }
    let rel = abs[i + 1..].trim_end_matches('/');
    (!rel.is_empty()).then(|| rel.to_string())
}

/// 读 PATH / RESULTPATH：UTF-8 无 BOM，每行 CRLF（H2）。读不了返回 `None`。
fn read_list(path: &PathBuf) -> Option<Vec<String>> {
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .map(|l| l.trim_end_matches('\r').trim_start_matches('\u{feff}'))
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

pub fn run_hook(a: &HookArgs, now_ms: i64) -> HookOutcome {
    if a.kind == "check_commit" {
        // Check-commit 不写事件：判定用不上它，只会在收件箱里堆垃圾
        return check_commit(a);
    }
    let list = if a.kind == "post_update" {
        &a.result_file
    } else {
        &a.path_file
    };
    let Some(lines) = list.as_ref().and_then(read_list) else {
        return HookOutcome::default();
    };
    let paths: Vec<String> = lines
        .iter()
        .filter_map(|l| to_repo_relative(&a.repo, l))
        .collect();
    if paths.is_empty() {
        return HookOutcome::default();
    }
    let error = a
        .error_file
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|e| e.trim().to_string())
        .filter(|e| !e.is_empty());
    let ev = TsvnEvent {
        version: EVENT_VERSION,
        kind: a.kind.clone(),
        at_ms: now_ms,
        paths,
        revision: a.revision,
        error,
    };
    // 写不进去也放行：跟进时没有这条事件，只会落到「拿不准」
    let _ = write_event(&a.repo, &ev);
    HookOutcome::default()
}

/// 有泄漏时钩子的结果：退出 1，带中英文两版说明。编不出来的预览不进消息，免得一行源码就把
/// 中文消息挤成英文版（见 `leak_message`）。判定用的是本线程的代码页，与写 stderr 时一致。
pub fn leak_outcome(leaks: &[crate::engine::svn_leak::Leak]) -> HookOutcome {
    use crate::engine::svn_leak::{leak_message, leak_message_en};
    let fits = |p: &str| encodes_exactly(p);
    HookOutcome {
        code: 1,
        stderr: leak_message(leaks, &fits),
        stderr_en: leak_message_en(leaks, &fits),
    }
}

/// 按勾选的文件检查 Local-Only 泄漏（分析 §6.13.4；日志 H3：拦截放在 Check-commit，用户
/// Abort 后回到提交框，消息还在）。命中退出 1；读不到、算不出一律放行，由跟进时的 F3 兜底。
fn check_commit(a: &HookArgs) -> HookOutcome {
    use crate::engine::svn_leak::local_only_leaks_on_disk;
    let Some(paths) = a.path_file.as_ref().and_then(read_list) else {
        return HookOutcome::default();
    };
    match local_only_leaks_on_disk(&a.repo, &paths) {
        Ok(leaks) if !leaks.is_empty() => leak_outcome(&leaks),
        Ok(_) => HookOutcome::default(),
        Err(e) => {
            log::warn!("Check-commit 读不出 Local-Only 信息，放行：{}", e.message);
            HookOutcome::default()
        }
    }
}

/// 按当前线程的 ANSI 代码页（由用户 locale 决定）编码：TortoiseSVN 这样解读钩子的 stderr
/// （日志 H4）。不能用 `CP_ACP`：系统 locale 是英文、用户 locale 是中文的机器上它是 1252，
/// 中文全变成 `?`，而 TortoiseSVN 显示 GBK 字节是正常的。代码页里没有的字符由系统换成
/// 默认字符（通常是 `?`）。
#[allow(unsafe_code)] // CP_THREAD_ACP 转码只有 WideCharToMultiByte，没有安全封装
pub fn encode_thread_acp(s: &str) -> Vec<u8> {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{WideCharToMultiByte, CP_THREAD_ACP};
        let wide: Vec<u16> = s.encode_utf16().collect();
        if wide.is_empty() {
            return Vec::new();
        }
        // SAFETY：先问长度，再按长度分配缓冲
        let n = unsafe { WideCharToMultiByte(CP_THREAD_ACP, 0, &wide, None, None, None) };
        if n <= 0 {
            return s.as_bytes().to_vec();
        }
        let mut buf = vec![0u8; n as usize];
        let n = unsafe { WideCharToMultiByte(CP_THREAD_ACP, 0, &wide, Some(&mut buf), None, None) };
        if n <= 0 {
            return s.as_bytes().to_vec();
        }
        buf.truncate(n as usize);
        buf
    }
    #[cfg(not(windows))]
    {
        s.as_bytes().to_vec()
    }
}

/// 按线程代码页编码，一个字都不丢时返回 `Some`。没有 best-fit 映射的汉字不加 flag 也会置
/// `lpUsedDefaultChar`；`WC_NO_BEST_FIT_CHARS` 管的是有 best-fit 的字符（全角「，」→ `,`），
/// 让它们也算丢字，「能原样显示」才是判据。
/// 线程代码页不支持这两个参数时（UTF-8 代码页，「Beta：使用 Unicode UTF-8」），调用会失败；
/// UTF-8 装得下任何字符，按不丢字处理，退回 [`encode_thread_acp`]。`&str` 里没有孤立代理项、
/// 也没传 `WC_ERR_INVALID_CHARS`，失败不会是别的原因。
#[allow(unsafe_code)] // 同 encode_thread_acp
fn encode_thread_acp_exact(s: &str) -> Option<Vec<u8>> {
    #[cfg(windows)]
    {
        use windows::Win32::Globalization::{
            WideCharToMultiByte, CP_THREAD_ACP, WC_NO_BEST_FIT_CHARS,
        };
        let wide: Vec<u16> = s.encode_utf16().collect();
        if wide.is_empty() {
            return Some(Vec::new());
        }
        let mut lossy = windows::core::BOOL(0);
        // SAFETY：先问长度，再按长度分配缓冲；lossy 活过两次调用
        let n = unsafe {
            WideCharToMultiByte(
                CP_THREAD_ACP,
                WC_NO_BEST_FIT_CHARS,
                &wide,
                None,
                None,
                Some(&mut lossy),
            )
        };
        if n <= 0 {
            return Some(encode_thread_acp(s));
        }
        if lossy.as_bool() {
            return None;
        }
        let mut buf = vec![0u8; n as usize];
        let n = unsafe {
            WideCharToMultiByte(
                CP_THREAD_ACP,
                WC_NO_BEST_FIT_CHARS,
                &wide,
                Some(&mut buf),
                None,
                Some(&mut lossy),
            )
        };
        if n <= 0 {
            return Some(encode_thread_acp(s));
        }
        if lossy.as_bool() {
            return None;
        }
        buf.truncate(n as usize);
        Some(buf)
    }
    #[cfg(not(windows))]
    {
        Some(s.as_bytes().to_vec())
    }
}

/// `s` 在当前线程代码页里一个字都不丢。
pub fn encodes_exactly(s: &str) -> bool {
    encode_thread_acp_exact(s).is_some()
}

/// 钩子写给 TortoiseSVN 的 stderr 字节。`text` 在线程代码页里一个字不丢就发它，否则发
/// `fallback`（英文版，见 [`HookOutcome::stderr_en`]）；没有 `fallback` 就照发 `text`。
/// 只看线程代码页能不能装下，不猜 TortoiseSVN 用哪个代码页：日志 H4 已实测它按线程代码页解读，
/// 装不下的中文不管发 GBK 还是 UTF-8 都显示不出来（2026-11-11 使用报告）。
pub fn hook_stderr_bytes(text: &str, fallback: &str) -> Vec<u8> {
    if fallback.is_empty() {
        return encode_thread_acp(text);
    }
    encode_thread_acp_exact(text).unwrap_or_else(|| encode_thread_acp(fallback))
}

/// `main.rs` 的入口。`args` 含 argv0。不是钩子调用返回 `None`，照常启动 GUI；是钩子调用就
/// 一定返回 `Some`，哪怕参数不全：TortoiseSVN 在等这个进程退出，绝不能落到 GUI。
pub fn tsvn_hook_main(args: Vec<OsString>) -> Option<i32> {
    let rest = args.get(1..).unwrap_or_default();
    if rest.first().map(|a| a.to_string_lossy()) != Some(FLAG.into()) {
        return None;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();
    let outcome = std::panic::catch_unwind(|| match parse_hook_args(rest) {
        Some(a) => run_hook(&a, now_ms),
        None => HookOutcome::default(),
    })
    .unwrap_or_default();
    if !outcome.stderr.is_empty() {
        use std::io::Write;
        let mut err = std::io::stderr();
        let _ = err.write_all(&hook_stderr_bytes(&outcome.stderr, &outcome.stderr_en));
        let _ = err.flush();
    }
    Some(outcome.code)
}
