use app_lib::engine::custom_config::{self, CustomCommand};
use app_lib::engine::launcher;
use std::fs;
use tempfile::TempDir;

/// 配置文件缺失时要写出当前平台的默认预设，而不是返回空列表。
#[test]
fn missing_file_writes_platform_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("customCommands.json");

    let commands = custom_config::load_from(&path).unwrap();

    assert!(!commands.is_empty(), "默认预设不该是空的");
    assert!(path.exists(), "缺失时应当把默认预设落盘");
    // 落盘的内容必须能被自己再读回来，否则用户第一次改配置就会踩到格式问题
    let reread = custom_config::load_from(&path).unwrap();
    assert_eq!(commands, reread);
}

/// 用户手写配置写错一个逗号，不能把整份配置冲掉。
#[test]
fn broken_json_errors_and_leaves_file_untouched() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    let broken = b"{ \"commands\": [ ";
    fs::write(&path, broken).unwrap();

    let err = custom_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "custom_config");

    let after = fs::read(&path).unwrap();
    assert_eq!(after, broken, "解析失败时绝不能改写用户的文件");
}

#[test]
fn unknown_fields_are_ignored() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"future":"whatever","commands":[
            {"id":"a","label":"A","program":"echo","args":["x"],"extra":true}
        ]}"#,
    )
    .unwrap();

    let commands = custom_config::load_from(&path).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, "echo");
}

#[test]
fn args_and_cwd_fall_back_to_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"commands":[{"id":"a","label":"A","program":"code"}]}"#,
    )
    .unwrap();

    let commands = custom_config::load_from(&path).unwrap();
    assert_eq!(commands[0].args, Vec::<String>::new());
    assert_eq!(commands[0].cwd, "$WORKTREE");
}

#[test]
fn empty_command_list_is_valid() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(&path, r#"{"version":1,"commands":[]}"#).unwrap();

    assert_eq!(custom_config::load_from(&path).unwrap(), Vec::new());
}

/// id 重复只是小瑕疵，不值得让用户丢掉整份配置。
#[test]
fn duplicate_ids_are_kept() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"commands":[
            {"id":"a","label":"One","program":"echo"},
            {"id":"a","label":"Two","program":"echo"}
        ]}"#,
    )
    .unwrap();

    assert_eq!(custom_config::load_from(&path).unwrap().len(), 2);
}

/// 验证一个**关于 Rust std 的**假设，而不是验证我们自己的代码：
/// Rust 1.77.2 起（CVE-2024-24576 / BatBadBut 的修复），`Command` 对 `.bat`/`.cmd`
/// 目标会自动经 cmd.exe 启动并正确转义参数。VS Code 的 `code` 就是 `code.cmd`，
/// 而仓库路径里出现空格和 `&` 是常态——这条不成立的话，整个 launcher 要换写法。
#[test]
#[cfg(windows)]
fn rust_std_escapes_arguments_for_batch_files() {
    use std::process::Command;

    let dir = TempDir::new().unwrap();
    let probe = dir.path().join("probe.cmd");
    // 必须用 %1 而不是 %~1：batch 的参数展开发生在该行被解析**之前**，
    // %~1 剥掉引号后，值里的 & 会被当成命令分隔符（实测：报 'c' 不是内部命令）。
    // 保留引号的 %1 才能验证「Rust 把整串作为一个 argv 传了过来」。
    fs::write(&probe, "@echo off\r\n>\"%~dp0out.txt\" echo %1\r\n").unwrap();

    let status = Command::new(&probe).arg("a b&c").status().unwrap();
    assert!(status.success(), "batch 文件没能启动");

    let out = fs::read_to_string(dir.path().join("out.txt")).unwrap();
    assert_eq!(out.trim(), "\"a b&c\"", "参数没有作为单个 argv 完整送达");
}

/// 默认预设里每一条都必须字段完整，否则第一次启动就是坏的。
#[test]
fn platform_defaults_are_well_formed() {
    for cmd in custom_config::default_commands() {
        let CustomCommand {
            id, label, program, ..
        } = &cmd;
        assert!(!id.is_empty(), "{cmd:?} 缺 id");
        assert!(!label.is_empty(), "{cmd:?} 缺 label");
        assert!(!program.is_empty(), "{cmd:?} 缺 program");
    }
}

/// Windows 上 `Command::new("git")` 靠 CreateProcess 查 PATH，只补 .exe。
/// 裸名字必须自己按 PATHEXT 解析，否则 `code`（实为 code.cmd）永远找不到。
#[test]
#[cfg(windows)]
fn resolve_program_finds_a_bare_name_on_path() {
    let resolved = launcher::resolve_program("git").unwrap();
    assert!(resolved.is_file(), "解析结果不是个文件：{resolved:?}");
    let name = resolved
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    assert_eq!(name, "git.exe");
}

/// 已经带扩展名或带路径分隔符的，原样交给系统，不要多此一举去搜 PATH。
#[test]
#[cfg(windows)]
fn resolve_program_passes_through_explicit_paths() {
    assert_eq!(
        launcher::resolve_program(r"C:\tools\my app\thing.exe").unwrap(),
        std::path::PathBuf::from(r"C:\tools\my app\thing.exe"),
    );
}

/// 非 Windows 平台 `resolve_program` 不做查找，缺失由 spawn 报出来，
/// 所以这条只在 Windows 上成立。
#[test]
#[cfg(windows)]
fn resolve_program_reports_a_missing_executable() {
    let err = launcher::resolve_program("definitely-not-a-real-program-xyz").unwrap_err();
    assert_eq!(err.code, "launcher");
    // 报错必须直接告诉用户去哪改
    assert!(
        err.message.contains("customCommands.json"),
        "报错没指向配置文件：{}",
        err.message
    );
}

/// argv 与 cwd 都要真的生效：在临时目录里跑 `git init --quiet probe`，
/// 然后等它把 probe/.git 建出来。既证明参数没被 shell 吃掉，也证明 cwd 用对了。
#[test]
fn spawn_detached_runs_with_the_given_args_and_cwd() {
    let dir = TempDir::new().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();

    launcher::spawn_detached(
        "git",
        &[
            "init".to_string(),
            "--quiet".to_string(),
            "probe".to_string(),
        ],
        Some(&cwd),
    )
    .unwrap();

    // 不等待子进程退出是这个函数的设计要点，所以这里只能轮询
    let marker = dir.path().join("probe").join(".git");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(marker.exists(), "子进程没有在 cwd 里跑起来");
}

#[test]
fn spawn_detached_reports_a_missing_executable() {
    let err = launcher::spawn_detached("definitely-not-a-real-program-xyz", &[], None).unwrap_err();
    assert_eq!(err.code, "launcher");
    assert!(err.message.contains("customCommands.json"));
}
