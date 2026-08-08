use crate::error::{GitError, Result};
use std::process::Command;

/// 构造一个指向 `path` 的 git 命令，并关掉所有交互式提示。
///
/// `GIT_TERMINAL_PROMPT=0` 是必须的：GUI 应用没有终端，git 一旦往 stderr
/// 打印 "Username:" 就会无限等待 stdin，子进程永久挂死并占住 Tauri 的线程池。
/// 关掉之后 git 会立即失败，错误能走到界面上的错误横幅。
///
/// 凭据管理器（Windows 的 GCM）弹自己的 GUI 窗口不受影响——那条路径不经过终端。
pub fn git_command(path: &str) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(path);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// 执行一条 git 命令，成功返回 stdout，失败把 stderr 包成 GitError。
pub fn run_git(path: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&run_git_bytes(path, args)?).to_string())
}

/// 同 [`run_git`]，但原样返回 stdout 的字节。
///
/// patch 要落成文件，不能过 `from_utf8_lossy`：非 UTF-8 的文件名或内容会被替换成
/// U+FFFD，生成的 patch 打回去就对不上了。需要写文件的场景一律走这个。
pub fn run_git_bytes(path: &str, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_command(path).args(args).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    Ok(output.stdout)
}
