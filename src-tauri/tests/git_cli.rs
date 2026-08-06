use app_lib::engine::git_cli;

#[test]
fn git_command_disables_terminal_prompt() {
    // GUI 里没人能回答 git 的 "Username:" 提示，子进程会永久挂死。
    // 这个变量是唯一的防线，改动 git_cli 时不能把它弄丢。
    let cmd = git_cli::git_command(".");
    let envs: Vec<_> = cmd.get_envs().collect();
    let prompt = envs
        .iter()
        .find(|(k, _)| *k == std::ffi::OsStr::new("GIT_TERMINAL_PROMPT"))
        .expect("GIT_TERMINAL_PROMPT not set");
    assert_eq!(prompt.1, Some(std::ffi::OsStr::new("0")));
}

#[test]
fn run_git_surfaces_stderr_on_failure() {
    let dir = tempfile::TempDir::new().unwrap();
    let err = git_cli::run_git(dir.path().to_str().unwrap(), &["rev-parse", "HEAD"])
        .expect_err("expected failure outside a repo");
    assert_eq!(err.code, "git_cli");
    assert!(!err.message.is_empty());
}

#[test]
fn run_git_returns_stdout_on_success() {
    let out = git_cli::run_git(".", &["--version"]).unwrap();
    assert!(out.starts_with("git version"), "got {out}");
}
