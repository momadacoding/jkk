use app_lib::engine::external_launch::{self, ExitReport, TempWorkspace};
use std::fs;
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

#[test]
fn expand_template_replaces_inside_elements() {
    let out = external_launch::expand_template(
        &s(&["/lefttitle=$LOCAL_TITLE", "$LOCAL"]),
        &[
            ("LOCAL", r"C:\t\P.LOCAL.cs".to_string()),
            ("LOCAL_TITLE", "main".to_string()),
        ],
    );
    assert_eq!(out, s(&["/lefttitle=main", r"C:\t\P.LOCAL.cs"]));
}

/// `$LOCAL_TITLE` 不能被当成 `$LOCAL` 加字面量 `_TITLE`，与变量的给出顺序无关。
#[test]
fn longest_name_wins_regardless_of_order() {
    let vars = [("LOCAL", "L".to_string()), ("LOCAL_TITLE", "T".to_string())];
    assert_eq!(
        external_launch::expand_template(&s(&["$LOCAL_TITLE|$LOCAL"]), &vars),
        s(&["T|L"])
    );
}

/// 替换进去的值来自用户的文件名和提交标题，什么字符都可能有，不能再被展开一次。
#[test]
fn expanded_values_are_not_reexpanded() {
    let out = external_launch::expand_template(
        &s(&["$LOCAL", "$REMOTE"]),
        &[
            ("LOCAL", "$REMOTE".to_string()),
            ("REMOTE", "r".to_string()),
        ],
    );
    assert_eq!(out, s(&["$REMOTE", "r"]));
}

#[test]
fn unknown_placeholder_is_kept() {
    let out = external_launch::expand_template(
        &s(&["$FOO", "a$", "$", "$LOCALX"]),
        &[("LOCAL", "x".to_string())],
    );
    assert_eq!(out, s(&["$FOO", "a$", "$", "$LOCALX"]));
}

#[test]
fn workspace_files_live_in_a_fresh_dir_under_root() {
    let ws = TempWorkspace::create().unwrap();
    let file = ws.file("P.LOCAL.cs");
    assert!(file.starts_with(external_launch::temp_root()));
    assert!(file.parent().unwrap().is_dir());

    let other = TempWorkspace::create().unwrap();
    assert_ne!(ws.file("x"), other.file("x"), "每次调用一个独立子目录");

    fs::write(&file, "x").unwrap();
    let dir = file.parent().unwrap().to_path_buf();
    ws.remove().unwrap();
    other.remove().unwrap();
    assert!(!dir.exists());
}

/// 删除前必须校验目录在应用临时根目录里，越界的一律拒绝、原样保留。
#[test]
fn workspace_remove_refuses_outside_root() {
    let outside = TempDir::new().unwrap();
    let ws = TempWorkspace::create_in(outside.path()).unwrap();
    let file = ws.file("keep.txt");
    fs::write(&file, "x").unwrap();

    let err = ws.remove().unwrap_err();
    assert_eq!(err.code, "external_tool_cleanup");
    assert!(file.exists(), "越界的目录绝不能删");
}

#[test]
fn lock_rejects_second_holder_and_releases_on_drop() {
    let guard =
        external_launch::try_lock(r"C:\repo-lock-test", "a/b.txt", "Beyond Compare").unwrap();

    let err = external_launch::try_lock(r"C:\repo-lock-test", "a/b.txt", "VS Code").unwrap_err();
    assert_eq!(err.code, "external_tool_busy");
    assert!(err.message.contains("Beyond Compare"), "{}", err.message);

    // 同一个仓库的另一个文件、另一个仓库的同名文件都不受影响
    drop(external_launch::try_lock(r"C:\repo-lock-test", "a/c.txt", "x").unwrap());
    drop(external_launch::try_lock(r"C:\other-lock-test", "a/b.txt", "x").unwrap());

    drop(guard);
    drop(external_launch::try_lock(r"C:\repo-lock-test", "a/b.txt", "x").unwrap());
}

/// Windows 上路径大小写、斜杠方向、尾部斜杠不同，仍是同一个文件。
#[cfg(windows)]
#[test]
fn lock_key_is_normalized_on_windows() {
    let _guard = external_launch::try_lock(r"C:\Repo-Norm-Test", r"Dir\File.txt", "BC").unwrap();
    let err = external_launch::try_lock("c:/repo-norm-test/", "dir/file.TXT", "BC").unwrap_err();
    assert_eq!(err.code, "external_tool_busy");
}

/// 退出事件里的 success / codeMessage 由工具自己的配置决定：BC 的 13 是「有差异」，不是故障。
#[test]
fn exit_event_judges_code_by_tool_config() {
    let cfg = app_lib::engine::external_tools_config::default_config();
    let bc = &cfg.tools[0];
    let report = |code| ExitReport { code, error: None };

    let ev = external_launch::exit_event("C:/r", "a.txt", bc, report(Some(13)));
    assert!(ev.success);
    assert_eq!(ev.code_message, None);
    assert_eq!(ev.tool_label, "Beyond Compare");

    let ev = external_launch::exit_event("C:/r", "a.txt", bc, report(Some(102)));
    assert!(!ev.success);
    assert!(ev.code_message.unwrap().contains("BCompare.exe"));

    // 没有退出码（被信号杀掉）或等待失败，一律不算成功
    assert!(!external_launch::exit_event("C:/r", "a.txt", bc, report(None)).success);
    let failed = ExitReport {
        code: Some(0),
        error: Some("x".into()),
    };
    assert!(!external_launch::exit_event("C:/r", "a.txt", bc, failed).success);
}

fn wait_exit(rx: &mpsc::Receiver<ExitReport>) -> ExitReport {
    rx.recv_timeout(Duration::from_secs(60))
        .expect("外部进程退出后应当回调 on_exit")
}

#[test]
fn spawn_missing_program_errors_synchronously() {
    for program in ["jkk-definitely-not-a-program", "C:/jkk-nope/missing.exe"] {
        let err =
            external_launch::spawn_watched(program, &[], |_| panic!("启动失败时不应回调 on_exit"))
                .unwrap_err();
        assert_eq!(err.code, "launcher", "{program}");
        assert!(err.message.contains(program), "{}", err.message);
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// 现场用 rustc 编一个把 argv 原样写进文件的小程序。
    ///
    /// 不在 Cargo.toml 里加 `[[bin]]`：那会改变 Tauri 打包时选主程序的方式。
    /// 跑得了 cargo test 的机器必然有 rustc，所以编不出来就让测试失败，不跳过。
    fn build_dumper(dir: &Path, name: &str) -> PathBuf {
        let src = dir.join("dump.rs");
        fs::write(
            &src,
            r#"fn main() {
                let out = std::env::current_exe().unwrap().with_extension("out");
                let args: Vec<String> = std::env::args().skip(1).collect();
                std::fs::write(out, args.join("\n")).unwrap();
            }"#,
        )
        .unwrap();
        let exe = dir.join(format!("{name}.exe"));
        let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
        let status = Command::new(rustc)
            .arg("-o")
            .arg(&exe)
            .arg(&src)
            .status()
            .expect("找不到 rustc");
        assert!(status.success(), "编译 argv 转储程序失败");
        exe
    }

    fn tricky_args() -> Vec<String> {
        s(&[
            "a b", "中文", "x&y", "50%", "!bang!", "\"q\"", "%PATH%", "^caret",
        ])
    }

    fn run_and_read(program: &Path, out: &Path, args: &[String]) -> Vec<String> {
        let (tx, rx) = mpsc::channel();
        external_launch::spawn_watched(program.to_str().unwrap(), args, move |r| {
            tx.send(r).unwrap();
        })
        .unwrap();
        let report = wait_exit(&rx);
        assert_eq!(report.code, Some(0), "{report:?}");
        let text = fs::read_to_string(out).expect("转储程序应当写出参数文件");
        text.split('\n').map(String::from).collect()
    }

    /// 直接启动 `.exe`（BComp.exe、UnityYAMLMerge.exe 的形态）。
    #[test]
    fn spawn_watched_delivers_argv_verbatim_to_exe() {
        let dir = TempDir::new().unwrap();
        let exe = build_dumper(dir.path(), "direct");
        let got = run_and_read(&exe, &exe.with_extension("out"), &tricky_args());
        assert_eq!(got, tricky_args());
    }

    /// 经 `.cmd` 转发 `%*` 再启动 `.exe`（`code.cmd` 的形态）。
    /// `.cmd` 由 std 交给 cmd.exe 并按 cmd 规则转义，这里验证转义后真实程序拿到的 argv。
    #[test]
    fn spawn_watched_delivers_argv_verbatim_through_cmd() {
        let dir = TempDir::new().unwrap();
        let exe = build_dumper(dir.path(), "forwarded");
        let cmd = dir.path().join("fwd.cmd");
        fs::write(&cmd, "@\"%~dp0forwarded.exe\" %*\r\n").unwrap();
        let got = run_and_read(&cmd, &exe.with_extension("out"), &tricky_args());
        assert_eq!(got, tricky_args());
    }

    #[test]
    fn spawn_watched_reports_nonzero_exit() {
        let dir = TempDir::new().unwrap();
        let cmd = dir.path().join("seven.cmd");
        fs::write(&cmd, "@exit /b 7\r\n").unwrap();

        let (tx, rx) = mpsc::channel();
        external_launch::spawn_watched(cmd.to_str().unwrap(), &[], move |r| {
            tx.send(r).unwrap();
        })
        .unwrap();
        let report = wait_exit(&rx);
        assert_eq!(report.code, Some(7));
        assert_eq!(report.error, None);
    }
}
