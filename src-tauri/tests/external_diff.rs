use app_lib::engine::blob_export;
use app_lib::engine::external_diff::{self, DiffTarget};
use app_lib::engine::external_launch::{self, TempWorkspace};
use app_lib::engine::external_tools_config::{ExternalTool, ExternalToolsConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "t@t.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    git(dir.path(), &["config", "core.autocrlf", "false"]);
    dir
}

fn commit_file(dir: &Path, rel: &str, content: &[u8], msg: &str) {
    let abs = dir.join(rel);
    fs::create_dir_all(abs.parent().unwrap()).unwrap();
    fs::write(&abs, content).unwrap();
    git(dir, &["add", "--", rel]);
    git(dir, &["commit", "-q", "-m", msg]);
}

fn p(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

fn prepare(dir: &Path, target: &DiffTarget) -> (Vec<u8>, PathBuf, String, String, TempWorkspace) {
    let ws = TempWorkspace::create().unwrap();
    let (left, right, lt, rt) = external_diff::prepare_diff(p(dir), target, &ws).unwrap();
    (fs::read(&left).unwrap(), right, lt, rt, ws)
}

/// Unstaged：左边是 index，右边直接是工作区里的真实文件（在 BC 里改了直接存盘）。
#[test]
fn unstaged_exports_index_vs_workdir() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    fs::write(dir.path().join("a.txt"), "v2\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    fs::write(dir.path().join("a.txt"), "v3\n").unwrap();

    let target = DiffTarget::Unstaged {
        file: "a.txt".into(),
    };
    let (left, right, lt, rt, ws) = prepare(dir.path(), &target);
    assert_eq!(left, b"v2\n");
    assert_eq!(canon(&right), canon(&dir.path().join("a.txt")));
    assert!(lt.contains("a.txt") && rt.contains("a.txt"), "{lt} / {rt}");
    ws.remove().unwrap();
}

#[test]
fn staged_exports_head_vs_index() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    fs::write(dir.path().join("a.txt"), "v2\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    fs::write(dir.path().join("a.txt"), "v3\n").unwrap();

    let ws = TempWorkspace::create().unwrap();
    let (left, right, _, _) = external_diff::prepare_diff(
        p(dir.path()),
        &DiffTarget::Staged {
            file: "a.txt".into(),
        },
        &ws,
    )
    .unwrap();
    assert_eq!(fs::read(&left).unwrap(), b"v1\n");
    assert_eq!(fs::read(&right).unwrap(), b"v2\n");
    assert!(
        right.starts_with(external_launch::temp_root()),
        "index 那侧是临时文件"
    );
    ws.remove().unwrap();
}

#[test]
fn commit_with_rename_uses_old_path_on_left() {
    let dir = repo();
    let body = "line\n".repeat(30);
    commit_file(dir.path(), "old.txt", body.as_bytes(), "c1");
    git(dir.path(), &["mv", "old.txt", "new.txt"]);
    fs::write(dir.path().join("new.txt"), format!("{body}extra\n")).unwrap();
    git(dir.path(), &["add", "new.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "rename"]);
    let oid = git(dir.path(), &["rev-parse", "HEAD"]);

    let ws = TempWorkspace::create().unwrap();
    let (left, right, lt, rt) = external_diff::prepare_diff(
        p(dir.path()),
        &DiffTarget::Commit {
            oid: oid.clone(),
            file: "new.txt".into(),
            old_file: Some("old.txt".into()),
        },
        &ws,
    )
    .unwrap();
    assert_eq!(fs::read_to_string(&left).unwrap(), body);
    assert_eq!(
        fs::read_to_string(&right).unwrap(),
        format!("{body}extra\n")
    );
    assert!(lt.contains("old.txt"), "{lt}");
    assert!(rt.contains(&oid[..7]) && rt.contains("rename"), "{rt}");
    ws.remove().unwrap();
}

#[test]
fn root_commit_left_is_empty_file() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "root");
    let oid = git(dir.path(), &["rev-parse", "HEAD"]);

    let ws = TempWorkspace::create().unwrap();
    let (left, right, _, _) = external_diff::prepare_diff(
        p(dir.path()),
        &DiffTarget::Commit {
            oid,
            file: "a.txt".into(),
            old_file: None,
        },
        &ws,
    )
    .unwrap();
    assert_eq!(fs::read(&left).unwrap(), b"");
    assert_eq!(fs::read(&right).unwrap(), b"v1\n");
    ws.remove().unwrap();
}

#[test]
fn added_file_left_is_empty_file() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    fs::write(dir.path().join("new.txt"), "n\n").unwrap();

    // 未跟踪：index 里没有
    let (left, right, _, _, ws) = prepare(
        dir.path(),
        &DiffTarget::Unstaged {
            file: "new.txt".into(),
        },
    );
    assert_eq!(left, b"");
    assert_eq!(fs::read(&right).unwrap(), b"n\n");
    ws.remove().unwrap();

    // 已暂存的新增：HEAD 里没有
    git(dir.path(), &["add", "new.txt"]);
    let (left, _, _, _, ws) = prepare(
        dir.path(),
        &DiffTarget::Staged {
            file: "new.txt".into(),
        },
    );
    assert_eq!(left, b"");
    ws.remove().unwrap();
}

/// 工作区里删掉的文件：右边用空临时文件，不能把一个不存在的路径交给工具。
#[test]
fn deleted_workdir_file_right_is_empty_temp_file() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    fs::remove_file(dir.path().join("a.txt")).unwrap();

    let (left, right, _, _, ws) = prepare(
        dir.path(),
        &DiffTarget::Unstaged {
            file: "a.txt".into(),
        },
    );
    assert_eq!(left, b"v1\n");
    assert!(right.starts_with(external_launch::temp_root()));
    assert_eq!(fs::read(&right).unwrap(), b"");
    assert!(!dir.path().join("a.txt").exists(), "不能在工作区里造出文件");
    ws.remove().unwrap();
}

#[test]
fn range_uses_from_and_to() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    let from = git(dir.path(), &["rev-parse", "HEAD"]);
    commit_file(dir.path(), "a.txt", b"v2\n", "c2");
    commit_file(dir.path(), "a.txt", b"v3\n", "c3");
    let to = git(dir.path(), &["rev-parse", "HEAD"]);

    let ws = TempWorkspace::create().unwrap();
    let (left, right, _, _) = external_diff::prepare_diff(
        p(dir.path()),
        &DiffTarget::Range {
            from,
            to,
            file: "a.txt".into(),
            old_file: None,
        },
        &ws,
    )
    .unwrap();
    assert_eq!(fs::read(&left).unwrap(), b"v1\n");
    assert_eq!(fs::read(&right).unwrap(), b"v3\n");
    ws.remove().unwrap();
}

/// 临时文件要是工作区形态：经过 autocrlf 等过滤器，和用户 checkout 出来的一致。
#[test]
fn export_applies_autocrlf() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"x\ny\n", "c1");
    git(dir.path(), &["config", "core.autocrlf", "true"]);
    let blob = git(dir.path(), &["rev-parse", "HEAD:a.txt"]);

    let out = TempDir::new().unwrap();
    let dest = out.path().join("a.LOCAL.txt");
    blob_export::export_filtered(p(dir.path()), "a.txt", &blob, &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), b"x\r\ny\r\n");
}

/// UTF-16 这类非 UTF-8 内容必须逐字节原样落地，不能过 from_utf8_lossy。
#[test]
fn export_keeps_utf16_bytes() {
    let dir = repo();
    fs::write(dir.path().join(".gitattributes"), "u16.txt -text\n").unwrap();
    let bytes: Vec<u8> = vec![
        0xFF, 0xFE, b'h', 0x00, b'i', 0x00, 0x2D, 0x4E, 0x0D, 0x00, 0x0A, 0x00, 0x80, 0xD8,
    ];
    git(dir.path(), &["add", ".gitattributes"]);
    commit_file(dir.path(), "u16.txt", &bytes, "utf16");
    let blob = git(dir.path(), &["rev-parse", "HEAD:u16.txt"]);

    let out = TempDir::new().unwrap();
    let dest = out.path().join("u16.LOCAL.txt");
    blob_export::export_filtered(p(dir.path()), "u16.txt", &blob, &dest).unwrap();
    assert_eq!(fs::read(&dest).unwrap(), bytes);
}

/// 过滤器或 git 失败时不能拿空文件顶替，用户会以为那一侧是空的。
#[test]
fn export_failure_does_not_write_file() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");

    let out = TempDir::new().unwrap();
    let dest = out.path().join("a.LOCAL.txt");
    let res = blob_export::export_filtered(
        p(dir.path()),
        "a.txt",
        "00000000000000000000000000000000deadbeef",
        &dest,
    );
    assert!(res.is_err());
    assert!(!dest.exists());
}

#[test]
fn export_rejects_empty_path() {
    let dir = repo();
    let out = TempDir::new().unwrap();
    let err =
        blob_export::export_filtered(p(dir.path()), "", "HEAD", &out.path().join("x")).unwrap_err();
    assert_eq!(err.code, "external_tool_export");
}

#[test]
fn side_file_name_keeps_extension() {
    assert_eq!(
        blob_export::side_file_name("Code/P.cs", "LOCAL"),
        "P.LOCAL.cs"
    );
    assert_eq!(
        blob_export::side_file_name("Makefile", "LOCAL"),
        "Makefile.LOCAL"
    );
    assert_eq!(
        blob_export::side_file_name("a/Main.unity", "REMOTE"),
        "Main.REMOTE.unity"
    );
    assert_eq!(
        blob_export::side_file_name(".gitignore", "BASE"),
        ".gitignore.BASE"
    );
    assert_eq!(
        blob_export::side_file_name(r"a\b\x.tar.gz", "LOCAL"),
        "x.tar.LOCAL.gz"
    );
}

#[test]
fn directory_is_not_a_regular_file() {
    let dir = TempDir::new().unwrap();
    let err = blob_export::ensure_regular_file(dir.path()).unwrap_err();
    assert_eq!(err.code, "external_tool_not_regular");
}

/// 造一个指向别处的链接：优先文件符号链接（要开发者模式），拿不到就用目录联接。
#[cfg(windows)]
fn make_link(link: &Path, target: &Path) -> bool {
    if std::os::windows::fs::symlink_file(target, link).is_ok() {
        return true;
    }
    let dir_target = target.with_extension("dir");
    fs::create_dir_all(&dir_target).unwrap();
    Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(&dir_target)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn make_link(link: &Path, target: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}

/// 工作区那一侧是符号链接时拒绝：外部工具顺着链接存盘会写到别的文件上。
#[test]
fn symlink_workdir_side_is_refused() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    let outside = TempDir::new().unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "keep").unwrap();
    if !make_link(&dir.path().join("link.txt"), &victim) {
        eprintln!("跳过：本机既不能建符号链接也不能建目录联接");
        return;
    }

    let ws = TempWorkspace::create().unwrap();
    let err = external_diff::prepare_diff(
        p(dir.path()),
        &DiffTarget::Unstaged {
            file: "link.txt".into(),
        },
        &ws,
    )
    .unwrap_err();
    assert_eq!(err.code, "external_tool_not_regular");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
    ws.remove().unwrap();
}

/// linked worktree 里的 index 和工作区文件都要按那个 worktree 解析，不能落到主工作区。
#[test]
fn linked_worktree_unstaged_uses_that_worktree() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"main\n", "c1");
    let holder = TempDir::new().unwrap();
    let wt = holder.path().join("wt");
    git(
        dir.path(),
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    fs::write(wt.join("a.txt"), "wt-index\n").unwrap();
    git(&wt, &["add", "a.txt"]);
    fs::write(wt.join("a.txt"), "wt-work\n").unwrap();

    let (left, right, _, _, ws) = prepare(
        &wt,
        &DiffTarget::Unstaged {
            file: "a.txt".into(),
        },
    );
    assert_eq!(left, b"wt-index\n");
    assert_eq!(canon(&right), canon(&wt.join("a.txt")));
    ws.remove().unwrap();
}

#[test]
fn chinese_space_bracket_path_roundtrip() {
    let dir = repo();
    let rel = "目录 a/f[1] x.txt";
    commit_file(dir.path(), rel, b"old\n", "c1");
    fs::write(dir.path().join(rel), "new\n").unwrap();
    git(dir.path(), &["add", "--", rel]);

    let ws = TempWorkspace::create().unwrap();
    let (left, right, lt, _) =
        external_diff::prepare_diff(p(dir.path()), &DiffTarget::Staged { file: rel.into() }, &ws)
            .unwrap();
    assert_eq!(fs::read(&left).unwrap(), b"old\n");
    assert_eq!(fs::read(&right).unwrap(), b"new\n");
    assert!(left
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("f[1] x."));
    assert!(lt.contains(rel), "{lt}");
    ws.remove().unwrap();
}

/// 前端发来的 target 是 camelCase：`{ kind: 'commit', oid, file, oldFile }`。
#[test]
fn diff_target_deserializes_camel_case() {
    let t: DiffTarget = serde_json::from_str(
        r#"{"kind":"range","from":"a","to":"b","file":"n.txt","oldFile":"o.txt"}"#,
    )
    .unwrap();
    assert_eq!(
        t,
        DiffTarget::Range {
            from: "a".into(),
            to: "b".into(),
            file: "n.txt".into(),
            old_file: Some("o.txt".into()),
        }
    );
    let t: DiffTarget = serde_json::from_str(r#"{"kind":"unstaged","file":"x"}"#).unwrap();
    assert_eq!(t, DiffTarget::Unstaged { file: "x".into() });
}

fn tool(program: &str, diff_args: Option<&[&str]>) -> ExternalToolsConfig {
    ExternalToolsConfig {
        version: 1,
        default_diff: Some("t".into()),
        default_merge: None,
        rules: Vec::new(),
        tools: vec![ExternalTool {
            id: "t".into(),
            label: "Test Tool".into(),
            program: program.into(),
            diff_args: diff_args.map(|a| a.iter().map(|s| s.to_string()).collect()),
            merge_args: None,
            success_exit_codes: None,
            exit_code_messages: None,
        }],
    }
}

fn never(_: external_launch::ExitReport) {
    panic!("没启动成功时不应回调 on_exit");
}

#[test]
fn tool_without_diff_args_is_unsupported() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    let cfg = tool("whatever", None);
    let target = DiffTarget::Unstaged {
        file: "a.txt".into(),
    };
    let err = external_diff::open_external_diff(
        p(dir.path()),
        &cfg,
        Path::new("C:/cfg/externalTools.json"),
        "t",
        &target,
        never,
    )
    .unwrap_err();
    assert_eq!(err.code, "external_tool_unsupported");
}

/// 找不到程序时要指向 externalTools.json，并且当场释放锁。
#[test]
fn missing_program_error_points_to_config_and_releases_lock() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    let cfg = tool("jkk-no-such-diff-tool", Some(&["$LOCAL", "$REMOTE"]));
    let config_path = Path::new("C:/cfg/externalTools.json");
    let target = DiffTarget::Unstaged {
        file: "a.txt".into(),
    };
    let err =
        external_diff::open_external_diff(p(dir.path()), &cfg, config_path, "t", &target, never)
            .unwrap_err();
    assert_eq!(err.code, "launcher");
    assert!(
        err.message.contains("externalTools.json"),
        "{}",
        err.message
    );
    assert!(!err.message.contains("customCommands"), "{}", err.message);

    drop(external_launch::try_lock(p(dir.path()), "a.txt", "x").unwrap());
}

#[test]
fn second_open_of_same_file_is_busy() {
    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    let _held = external_launch::try_lock(p(dir.path()), "a.txt", "Beyond Compare").unwrap();
    let cfg = tool("whatever", Some(&["$LOCAL"]));
    let err = external_diff::open_external_diff(
        p(dir.path()),
        &cfg,
        Path::new("externalTools.json"),
        "t",
        &DiffTarget::Unstaged {
            file: "a.txt".into(),
        },
        never,
    )
    .unwrap_err();
    assert_eq!(err.code, "external_tool_busy");
}

/// 进程退出后：回调拿到退出码、临时目录已删、锁已放开、工作区文件还在。
#[cfg(windows)]
#[test]
fn exit_cleans_workspace_and_releases_lock() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = repo();
    commit_file(dir.path(), "a.txt", b"v1\n", "c1");
    fs::write(dir.path().join("a.txt"), "v2\n").unwrap();
    let bin = TempDir::new().unwrap();
    let script = bin.path().join("fake.cmd");
    let record = bin.path().join("args.txt");
    fs::write(&script, "@echo %~1>\"%~dp0args.txt\"\r\n@exit /b 3\r\n").unwrap();

    let cfg = tool(script.to_str().unwrap(), Some(&["$LOCAL", "$REMOTE"]));
    let (tx, rx) = mpsc::channel();
    external_diff::open_external_diff(
        p(dir.path()),
        &cfg,
        Path::new("externalTools.json"),
        "t",
        &DiffTarget::Unstaged {
            file: "a.txt".into(),
        },
        move |r| tx.send(r).unwrap(),
    )
    .unwrap();
    let report = rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(report.code, Some(3));

    let left = PathBuf::from(fs::read_to_string(&record).unwrap().trim());
    assert!(left.starts_with(external_launch::temp_root()), "{left:?}");
    assert!(!left.parent().unwrap().exists(), "临时子目录应已删除");
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), b"v2\n");
    drop(external_launch::try_lock(p(dir.path()), "a.txt", "x").unwrap());
}
