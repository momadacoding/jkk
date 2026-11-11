#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 钩子子命令与收件箱写入（计划 Task 2；日志 H2、H4）。
//!
//! 临时文件的内容按实测的真实形状造：UTF-8 无 BOM、正斜杠绝对路径、每行 CRLF。

use app_lib::engine::tsvn_hook_cli::{
    parse_hook_args, run_hook, to_repo_relative, tsvn_hook_main, HookArgs,
};
use app_lib::engine::tsvn_inbox::{inbox_dir, TsvnEvent};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

struct Fx {
    _dir: TempDir,
    repo: PathBuf,
    tmp: PathBuf,
}

/// 仓库根带空格与中文：注册时写进 `--repo` 的就是这种路径。
fn fx() -> Fx {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("项目 repo");
    fs::create_dir_all(repo.join(".git")).unwrap();
    let tmp = dir.path().join("tsvn tmp");
    fs::create_dir_all(&tmp).unwrap();
    Fx {
        _dir: dir,
        repo,
        tmp,
    }
}

impl Fx {
    fn repo_str(&self) -> String {
        self.repo.display().to_string()
    }

    /// TortoiseSVN 写的路径：正斜杠绝对路径。
    fn tsvn_path(&self, rel: &str) -> String {
        format!("{}/{rel}", self.repo_str().replace('\\', "/"))
    }

    /// 造一个 PATH / RESULTPATH 文件：每行 CRLF，最后一行也带。
    fn list_file(&self, name: &str, lines: &[String]) -> PathBuf {
        let p = self.tmp.join(name);
        let body: String = lines.iter().map(|l| format!("{l}\r\n")).collect();
        fs::write(&p, body).unwrap();
        p
    }

    fn file(&self, name: &str, body: &str) -> PathBuf {
        let p = self.tmp.join(name);
        fs::write(&p, body).unwrap();
        p
    }

    fn events(&self) -> Vec<TsvnEvent> {
        let dir = inbox_dir(&self.repo_str());
        let Ok(rd) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut names: Vec<PathBuf> = rd.map(|e| e.unwrap().path()).collect();
        names.sort();
        names
            .iter()
            .map(|p| {
                assert_eq!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("json"),
                    "{p:?}"
                );
                serde_json::from_slice(&fs::read(p).unwrap()).unwrap()
            })
            .collect()
    }
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

fn s(p: &Path) -> String {
    p.display().to_string()
}

/// `--tsvn-hook <kind> --repo <repo>` 加上 TortoiseSVN 追加的参数。
fn hook_args(fx: &Fx, kind: &str, tail: &[String]) -> Vec<OsString> {
    let mut v = os(&["--tsvn-hook", kind, "--repo"]);
    v.push(fx.repo_str().into());
    v.extend(tail.iter().map(OsString::from));
    v
}

fn cwd(fx: &Fx) -> String {
    s(&fx.repo.join("Code"))
}

/// 把 fixture 的仓库换成真仓库：`svn/base` 上有 `Code/A.cs`，main 上一个 commit，
/// `local_only` 决定它是不是 Local-Only。它把第 2 行改成补丁。磁盘留在 main 的状态。
fn git_repo(fx: &Fx, local_only: bool) {
    let r = fx.repo_str();
    fs::remove_dir_all(fx.repo.join(".git")).unwrap();
    let git = |args: &[&str]| {
        app_lib::engine::git_cli::run_git(&r, args).unwrap();
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "core.autocrlf", "false"]);
    fs::create_dir_all(fx.repo.join("Code")).unwrap();
    let b: String = (1..=20).map(|i| format!("line {i:02}\n")).collect();
    fs::write(fx.repo.join("Code/A.cs"), &b).unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "B"]);
    git(&["branch", "svn/base"]);
    fs::write(
        fx.repo.join("Code/A.cs"),
        b.replace("line 02\n", "line 02 DEBUG_PATCH\n"),
    )
    .unwrap();
    git(&["add", "-A"]);
    let msg = if local_only {
        "调试补丁\n\nLocal-Only: true"
    } else {
        "功能"
    };
    git(&["commit", "-q", "-m", msg]);
}

fn check_commit(fx: &Fx) -> app_lib::engine::tsvn_hook_cli::HookOutcome {
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("Code/A.cs")]);
    let m = fx.file("m.tmp", "");
    let a = parse_hook_args(&hook_args(fx, "check_commit", &[s(&p), s(&m), cwd(fx)])).unwrap();
    run_hook(&a, 1)
}

#[test]
fn check_commit_hit_exits_one_with_message() {
    let fx = fx();
    git_repo(&fx, true);
    let out = check_commit(&fx);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Code/A.cs"), "{}", out.stderr);
    assert!(out.stderr.contains("调试补丁"), "{}", out.stderr);
    assert_eq!(
        out.stderr.lines().last(),
        Some("取消勾选这些文件，或用「提交后恢复」删掉这些改动再提交。")
    );
    // 英文备用说明带同样的路径与 commit 标题，线程代码页装不下中文时发它
    assert!(out.stderr_en.contains("Code/A.cs"), "{}", out.stderr_en);
    assert!(out.stderr_en.contains("调试补丁"), "{}", out.stderr_en);
    assert_eq!(
        out.stderr_en.lines().last(),
        Some("Uncheck these files, or use \"Restore after commit\" to drop these changes before committing.")
    );
    // 拦截也不写事件
    assert!(fx.events().is_empty());
}

#[test]
fn check_commit_without_local_only_commits_exits_zero() {
    let fx = fx();
    git_repo(&fx, false);
    let out = check_commit(&fx);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
}

#[test]
fn check_commit_on_error_fails_open() {
    let fx = fx();
    git_repo(&fx, true);
    app_lib::engine::git_cli::run_git(&fx.repo_str(), &["update-ref", "-d", "refs/heads/svn/base"])
        .unwrap();
    let out = check_commit(&fx);
    assert_eq!((out.code, out.stderr.as_str()), (0, ""));
}

/// 真实进程、真实命令行：注册时写的 `hook_command` 原样拼上 TortoiseSVN 追加的参数，交给
/// Windows 自己拆参（review 1 第 3 条）。同时覆盖 `main.rs` 的分流：落到 GUI 的话进程不会退出，
/// 超时即失败。TortoiseSVN 给自己的参数加不加引号没测过（日志「未测」），这里按加引号拼。
#[cfg(windows)]
#[test]
fn real_exe_with_registered_command_line_writes_one_event() {
    use app_lib::engine::tsvn_hooks::hook_command;
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};

    let fx = fx();
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("代码/中文 文件.cs")]);
    let m = fx.file("m.tmp", "t1");
    let e = fx.file("e.tmp", "");
    let exe = Path::new(env!("CARGO_BIN_EXE_app"));
    // 仓库根带结尾反斜杠：hook_command 必须去掉它
    let full = hook_command(exe, "post_commit", &format!("{}\\", fx.repo_str()));
    let after_exe = full.split_once("\" ").unwrap().1;
    let tail = format!(
        " \"{}\" 3 \"{}\" 7 \"{}\" \"{}\"",
        s(&p),
        s(&m),
        s(&e),
        cwd(&fx)
    );
    let mut child = std::process::Command::new(exe)
        .raw_arg(after_exe)
        .raw_arg(tail)
        .spawn()
        .unwrap();
    let start = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("钩子进程 30 秒没退出：可能落到了 GUI");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0));
    let evs = fx.events();
    assert_eq!(evs.len(), 1, "{evs:?}");
    assert_eq!(evs[0].paths, vec!["代码/中文 文件.cs".to_string()]);
    assert_eq!(evs[0].revision, Some(7));
}

#[test]
fn parses_each_kind_by_position() {
    let fx = fx();
    let p = s(&fx.list_file("p.tmp", &[]));
    let m = s(&fx.file("m.tmp", "msg"));
    let e = s(&fx.file("e.tmp", ""));
    let r = s(&fx.list_file("r.tmp", &[]));

    let a = parse_hook_args(&hook_args(
        &fx,
        "check_commit",
        &[p.clone(), m.clone(), cwd(&fx)],
    ))
    .unwrap();
    assert_eq!(
        (a.kind.as_str(), a.repo.clone()),
        ("check_commit", fx.repo_str())
    );
    assert_eq!(a.path_file.as_deref(), Some(Path::new(&p)));
    assert_eq!(
        (a.revision, a.error_file.clone(), a.result_file.clone()),
        (None, None, None)
    );

    let a = parse_hook_args(&hook_args(
        &fx,
        "pre_commit",
        &[p.clone(), "3".into(), m.clone(), cwd(&fx)],
    ))
    .unwrap();
    assert_eq!(a.path_file.as_deref(), Some(Path::new(&p)));
    assert_eq!(a.revision, None);

    let a = parse_hook_args(&hook_args(
        &fx,
        "post_commit",
        &[
            p.clone(),
            "3".into(),
            m.clone(),
            "7".into(),
            e.clone(),
            cwd(&fx),
        ],
    ))
    .unwrap();
    assert_eq!(a.revision, Some(7));
    assert_eq!(a.error_file.as_deref(), Some(Path::new(&e)));

    let a = parse_hook_args(&hook_args(
        &fx,
        "post_update",
        &[
            p.clone(),
            "-2".into(),
            "5".into(),
            e.clone(),
            cwd(&fx),
            r.clone(),
        ],
    ))
    .unwrap();
    assert_eq!(a.revision, Some(5));
    assert_eq!(a.error_file.as_deref(), Some(Path::new(&e)));
    assert_eq!(a.result_file.as_deref(), Some(Path::new(&r)));
}

#[test]
fn non_hook_args_return_none() {
    assert!(parse_hook_args(&[]).is_none());
    assert!(parse_hook_args(&os(&["--foo"])).is_none());
    // 普通启动：argv 只有程序本身
    assert_eq!(tsvn_hook_main(os(&[r"C:\jkk\app.exe"])), None);
    assert_eq!(
        tsvn_hook_main(os(&[r"C:\jkk\app.exe", "D:\\some\\repo"])),
        None
    );
}

#[test]
fn malformed_hook_args_still_exit_zero_without_starting_the_gui() {
    // 带了 --tsvn-hook 就绝不能落到 GUI：TortoiseSVN 在等这个进程退出
    assert_eq!(tsvn_hook_main(os(&["app.exe", "--tsvn-hook"])), Some(0));
    assert_eq!(
        tsvn_hook_main(os(&[
            "app.exe",
            "--tsvn-hook",
            "no_such_kind",
            "--repo",
            "x"
        ])),
        Some(0)
    );
    assert_eq!(
        tsvn_hook_main(os(&[
            "app.exe",
            "--tsvn-hook",
            "post_commit",
            "--repo",
            "x",
            "p"
        ])),
        Some(0)
    );
}

#[test]
fn post_commit_writes_one_event_with_repo_relative_paths() {
    let fx = fx();
    let p = fx.list_file(
        "p.tmp",
        &[fx.tsvn_path("Code/A.cs"), fx.tsvn_path("代码/中文 文件.cs")],
    );
    let m = fx.file("m.tmp", "t1");
    let e = fx.file("e.tmp", "");
    let a = parse_hook_args(&hook_args(
        &fx,
        "post_commit",
        &[s(&p), "3".into(), s(&m), "3".into(), s(&e), s(&fx.repo)],
    ))
    .unwrap();
    let out = run_hook(&a, 1_000);
    assert_eq!(out.code, 0);
    let evs = fx.events();
    assert_eq!(evs.len(), 1);
    let ev = &evs[0];
    assert_eq!(ev.version, 1);
    assert_eq!(ev.kind, "post_commit");
    assert_eq!(ev.at_ms, 1_000);
    assert_eq!(
        ev.paths,
        vec!["Code/A.cs".to_string(), "代码/中文 文件.cs".to_string()]
    );
    assert_eq!(ev.revision, Some(3));
    assert_eq!(ev.error, None);
    // 不留 .tmp
    assert!(fs::read_dir(inbox_dir(&fx.repo_str())).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".tmp")));
}

#[test]
fn pre_commit_event_has_no_revision() {
    let fx = fx();
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("Code/A.cs")]);
    let m = fx.file("m.tmp", "t1");
    let a = parse_hook_args(&hook_args(
        &fx,
        "pre_commit",
        &[s(&p), "3".into(), s(&m), cwd(&fx)],
    ))
    .unwrap();
    assert_eq!(run_hook(&a, 5).code, 0);
    let evs = fx.events();
    assert_eq!(evs.len(), 1);
    assert_eq!(
        (evs[0].kind.as_str(), evs[0].revision, evs[0].at_ms),
        ("pre_commit", None, 5)
    );
}

#[test]
fn paths_outside_the_repo_are_dropped_and_an_empty_event_is_not_written() {
    let fx = fx();
    let outside = format!("{}/x.cs", s(&fx.tmp).replace('\\', "/"));
    let p = fx.list_file("p.tmp", &[outside.clone(), fx.tsvn_path("Code/A.cs")]);
    let m = fx.file("m.tmp", "");
    let e = fx.file("e.tmp", "");
    let tail = [s(&p), "3".into(), s(&m), "4".into(), s(&e), cwd(&fx)];
    assert_eq!(
        run_hook(
            &parse_hook_args(&hook_args(&fx, "post_commit", &tail)).unwrap(),
            1
        )
        .code,
        0
    );
    assert_eq!(fx.events()[0].paths, vec!["Code/A.cs".to_string()]);

    let fx = fx_only_outside();
    assert!(fx.events().is_empty());
}

/// 只有仓库外的路径：不写事件。
fn fx_only_outside() -> Fx {
    let fx = fx();
    let outside = format!("{}/x.cs", s(&fx.tmp).replace('\\', "/"));
    let p = fx.list_file("p.tmp", &[outside]);
    let m = fx.file("m.tmp", "");
    let e = fx.file("e.tmp", "");
    let tail = [s(&p), "3".into(), s(&m), "4".into(), s(&e), cwd(&fx)];
    assert_eq!(
        run_hook(
            &parse_hook_args(&hook_args(&fx, "post_commit", &tail)).unwrap(),
            1
        )
        .code,
        0
    );
    fx
}

#[test]
fn post_update_takes_resultpath_including_deleted_files() {
    let fx = fx();
    // PATH 只有右键选的目录；RESULTPATH 才是实际被碰的路径（H4）。B.cs 已被删，磁盘上没有
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("Code")]);
    let r = fx.list_file(
        "r.tmp",
        &[
            fx.repo_str().replace('\\', "/"),
            fx.tsvn_path("Code/A.cs"),
            fx.tsvn_path("Code/B.cs"),
        ],
    );
    let e = fx.file("e.tmp", "");
    let tail = [s(&p), "-2".into(), "5".into(), s(&e), cwd(&fx), s(&r)];
    assert_eq!(
        run_hook(
            &parse_hook_args(&hook_args(&fx, "post_update", &tail)).unwrap(),
            9
        )
        .code,
        0
    );
    let evs = fx.events();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0].kind, "post_update");
    assert_eq!(evs[0].revision, Some(5));
    // 仓库根本身不是文件，不记
    assert_eq!(
        evs[0].paths,
        vec!["Code/A.cs".to_string(), "Code/B.cs".to_string()]
    );
}

#[test]
fn post_commit_with_nonempty_error_records_the_error() {
    let fx = fx();
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("Code/A.cs")]);
    let m = fx.file("m.tmp", "");
    let e = fx.file("e.tmp", "E155011: out of date\r\n");
    let tail = [s(&p), "3".into(), s(&m), "0".into(), s(&e), cwd(&fx)];
    assert_eq!(
        run_hook(
            &parse_hook_args(&hook_args(&fx, "post_commit", &tail)).unwrap(),
            1
        )
        .code,
        0
    );
    assert_eq!(
        fx.events()[0].error.as_deref(),
        Some("E155011: out of date")
    );
}

#[test]
fn check_commit_writes_no_event_and_exits_zero() {
    let fx = fx();
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("Code/A.cs")]);
    let m = fx.file("m.tmp", "");
    let a = parse_hook_args(&hook_args(&fx, "check_commit", &[s(&p), s(&m), cwd(&fx)])).unwrap();
    let out = run_hook(&a, 1);
    assert_eq!(out.code, 0);
    assert!(out.stderr.is_empty());
    assert!(fx.events().is_empty());
}

#[test]
fn unwritable_inbox_still_exits_zero() {
    let fx = fx();
    fs::remove_dir(fx.repo.join(".git")).unwrap();
    fs::write(fx.repo.join(".git"), "gitdir: elsewhere").unwrap();
    let p = fx.list_file("p.tmp", &[fx.tsvn_path("Code/A.cs")]);
    let m = fx.file("m.tmp", "");
    let e = fx.file("e.tmp", "");
    let tail = [s(&p), "3".into(), s(&m), "4".into(), s(&e), cwd(&fx)];
    assert_eq!(
        run_hook(
            &parse_hook_args(&hook_args(&fx, "post_commit", &tail)).unwrap(),
            1
        )
        .code,
        0
    );
    // 没有凭空造出 .git 目录
    assert!(fx.repo.join(".git").is_file());
}

#[test]
fn missing_path_file_exits_zero_without_event() {
    let fx = fx();
    let a = HookArgs {
        kind: "post_commit".into(),
        repo: fx.repo_str(),
        path_file: Some(fx.tmp.join("gone.tmp")),
        revision: Some(4),
        error_file: None,
        result_file: None,
    };
    assert_eq!(run_hook(&a, 1).code, 0);
    assert!(fx.events().is_empty());
}

#[cfg(windows)]
#[test]
fn encode_thread_acp_round_trips_ascii() {
    use app_lib::engine::tsvn_hook_cli::encode_thread_acp;
    assert_eq!(
        encode_thread_acp("jkk: Code/A.cs\n"),
        b"jkk: Code/A.cs\n".to_vec()
    );
    assert!(encode_thread_acp("").is_empty());
}

/// 真实形态：系统 locale 是 en-US（ACP 1252）、用户 locale 是中文时，TortoiseSVN 按线程代码页
/// 解读 stderr。按 `CP_ACP` 编码，中文全部变成 `?`。这里只改本测试线程的 locale，所以结果
/// 不依赖跑测试的机器；但在 ACP 本身就是 936 的机器上，这条抓不到退回 `CP_ACP` 的改动。
#[cfg(windows)]
#[test]
#[allow(unsafe_code)] // 改本线程 locale 只有 Win32 C API
fn stderr_text_follows_the_thread_code_page_not_the_system_one() {
    use app_lib::engine::tsvn_hook_cli::encode_thread_acp;
    use windows::Win32::Globalization::{GetThreadLocale, SetThreadLocale};
    const ZH_CN: u32 = 0x0804;
    let before = unsafe { GetThreadLocale() };
    assert!(unsafe { SetThreadLocale(ZH_CN) }.as_bool());
    let got = encode_thread_acp("钩子");
    let _ = unsafe { SetThreadLocale(before) };
    assert_eq!(got, vec![0xB9, 0xB3, 0xD7, 0xD3]);
}

/// 在本测试线程上临时换 locale 跑 `f`，跑完换回。
#[cfg(windows)]
#[allow(unsafe_code)] // 改本线程 locale 只有 Win32 C API
fn with_thread_locale<T>(lcid: u32, f: impl FnOnce() -> T) -> T {
    use windows::Win32::Globalization::{GetThreadLocale, SetThreadLocale};
    let before = unsafe { GetThreadLocale() };
    assert!(unsafe { SetThreadLocale(lcid) }.as_bool());
    let got = f();
    let _ = unsafe { SetThreadLocale(before) };
    got
}

#[cfg(windows)]
const EN_CA: u32 = 0x1009;
#[cfg(windows)]
const ZH_CN: u32 = 0x0804;

/// 真实形态（2026-11-11 使用报告）：系统 locale 中文、区域格式 en-CA，线程代码页 1252。
/// 中文提示编码后全是 `?`，要改发英文版。
#[cfg(windows)]
#[test]
fn hook_stderr_falls_back_to_english_when_the_thread_code_page_cannot_hold_chinese() {
    use app_lib::engine::tsvn_hook_cli::hook_stderr_bytes;
    let got = with_thread_locale(EN_CA, || hook_stderr_bytes("jkk：中文", "jkk: English"));
    assert_eq!(got, b"jkk: English".to_vec());
}

#[cfg(windows)]
#[test]
fn hook_stderr_keeps_chinese_when_the_thread_code_page_holds_it() {
    use app_lib::engine::tsvn_hook_cli::hook_stderr_bytes;
    let got = with_thread_locale(ZH_CN, || hook_stderr_bytes("钩子", "hook"));
    assert_eq!(got, vec![0xB9, 0xB3, 0xD7, 0xD3]);
}

/// 1252 会把全角「，」best-fit 成 `,`、不加 `WC_NO_BEST_FIT_CHARS` 时不报替换。汉字本身不靠这个
/// flag 也会报；这条守的是「能原样显示」这个判据，不让 best-fit 过的文字算作装得下。
#[cfg(windows)]
#[test]
fn best_fit_substitution_counts_as_lossy() {
    use app_lib::engine::tsvn_hook_cli::hook_stderr_bytes;
    let got = with_thread_locale(EN_CA, || hook_stderr_bytes("a，b", "fallback"));
    assert_eq!(got, b"fallback".to_vec());
}

/// 英文版里的路径与 commit 标题可能本身是中文：模板照样是英文，装不下的字照旧变 `?`，
/// 不会因此退回中文版或整段丢掉。
#[cfg(windows)]
#[test]
fn english_fallback_with_chinese_path_keeps_the_english_template() {
    use app_lib::engine::tsvn_hook_cli::hook_stderr_bytes;
    let got = with_thread_locale(EN_CA, || {
        hook_stderr_bytes("jkk：代码/甲.lua", "jkk: 代码/A.lua from \"小婉\"")
    });
    assert_eq!(got, b"jkk: ??/A.lua from \"??\"".to_vec());
}

/// 真实拦截消息走一遍：en-CA 下发出去的是英文版，中文的 commit 标题变 `?`。
#[cfg(windows)]
#[test]
fn real_leak_message_under_en_ca_is_sent_in_english() {
    use app_lib::engine::tsvn_hook_cli::hook_stderr_bytes;
    let fx = fx();
    git_repo(&fx, true);
    let out = check_commit(&fx);
    assert_eq!(out.code, 1);
    let got = with_thread_locale(EN_CA, || hook_stderr_bytes(&out.stderr, &out.stderr_en));
    let text = String::from_utf8(got).unwrap();
    assert!(
        text.starts_with("jkk: these files contain Local-Only patches"),
        "{text}"
    );
    assert!(text.contains("Code/A.cs: line "), "{text}");
    assert!(text.contains("from commit \"????\""), "{text}");
    // 同一条消息在 zh-CN 下照发中文
    let zh = with_thread_locale(ZH_CN, || hook_stderr_bytes(&out.stderr, &out.stderr_en));
    assert!(zh.starts_with(&[b'j', b'k', b'k', 0xA3, 0xBA]), "{zh:?}");
}

#[test]
fn empty_fallback_sends_the_original_text() {
    use app_lib::engine::tsvn_hook_cli::hook_stderr_bytes;
    assert_eq!(hook_stderr_bytes("plain", ""), b"plain".to_vec());
}

/// 注册的命令行里盘符根写成 `--repo "D:"`（`hook_command` 必须去掉结尾的 `\`），而 `D:` 是
/// 驱动器相对路径：`join(".git")` 得到 `D:.git`，落在钩子的工作目录（勾选文件的公共父目录）
/// 下面，打不开仓库。所以这里断言解析出来的路径能正确拼出 `.git`，只比字符串看不出这一点。
#[cfg(windows)]
#[test]
fn drive_root_repo_is_restored_to_an_absolute_path() {
    let a = parse_hook_args(&os(&[
        "--tsvn-hook",
        "pre_commit",
        "--repo",
        "D:",
        "p",
        "3",
        "m",
        r"D:\Code",
    ]))
    .unwrap();
    assert_eq!(a.repo, r"D:\");
    assert_eq!(
        std::path::Path::new(&a.repo).join(".git"),
        std::path::Path::new(r"D:\.git"),
    );
}

#[test]
fn repo_that_is_not_a_bare_drive_is_left_alone() {
    for repo in [r"D:\work", "relative", "ab", "1:"] {
        let a = parse_hook_args(&os(&[
            "--tsvn-hook",
            "pre_commit",
            "--repo",
            repo,
            "p",
            "3",
            "m",
            "c",
        ]))
        .unwrap();
        assert_eq!(a.repo, repo);
    }
}

#[test]
fn to_repo_relative_handles_case_and_separators() {
    assert_eq!(
        to_repo_relative(r"c:\repo", "C:/Repo/Code/A.cs").as_deref(),
        Some("Code/A.cs")
    );
    assert_eq!(
        to_repo_relative(r"C:\Repo\", "C:/Repo/Code/A.cs").as_deref(),
        Some("Code/A.cs")
    );
    assert_eq!(to_repo_relative(r"c:\repo", "C:/Repo2/x"), None);
    assert_eq!(to_repo_relative(r"c:\repo", "C:/Repo"), None);
    assert_eq!(to_repo_relative(r"c:\repo", "C:/Repo/"), None);
    assert_eq!(
        to_repo_relative(r"D:\项目 Repo", "d:/项目 repo/代码/中文 文件.cs").as_deref(),
        Some("代码/中文 文件.cs")
    );
}
