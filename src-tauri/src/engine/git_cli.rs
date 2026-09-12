use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use std::process::Command;

/// 构造一个指向 `path` 的 git 命令，并关掉所有交互式提示。
///
/// `GIT_TERMINAL_PROMPT=0` 是必须的：GUI 应用没有终端，git 一旦往 stderr
/// 打印 "Username:" 就会无限等待 stdin，子进程永久挂死并占住 Tauri 的线程池。
/// 关掉之后 git 会立即失败，错误能走到界面上的错误横幅。
///
/// 凭据管理器（Windows 的 GCM）弹自己的 GUI 窗口不受影响——那条路径不经过终端。
///
/// 走 [`hidden_command`] 是为了不在装机版上闪出 console 黑框，理由见该函数。
pub fn git_command(path: &str) -> Command {
    let mut cmd = hidden_command("git");
    cmd.current_dir(path);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

/// 执行一条 git 命令，成功返回 stdout，失败把 stderr 包成 GitError。
pub fn run_git(path: &str, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&run_git_bytes(path, args)?).to_string())
}

/// 同 [`run_git`]，但把**退出码交给调用方判断**，非零不算失败。
///
/// 只在「不同的非零退出码意味着不同的事实」时才用它。git 的惯例是 1 表示
/// 「查询没有结果」、128 表示 fatal，而 [`run_git`] 把任何非零都塌缩成同一个
/// `GitError`，区分不出来。已知必须区分的地方：`show-ref --verify` 的
/// 「ref 不存在」（1）与「ref 损坏」（128）——把后者当成前者会删用户的文件，
/// 见 `engine::ops::head_is_unborn`。
///
/// 进程本身起不来（git 不在 PATH）仍然是错误，照常返回 `Err`。
pub fn run_git_code(path: &str, args: &[&str]) -> Result<(i32, String)> {
    let output = git_command(path).args(args).output()?;
    // 正常退出时 code() 必有值；拿不到只可能是被信号杀掉（Windows 上不存在），
    // 那种情况用 -1 表示「未知」，调用方不会把它误当成某个有意义的退出码
    let code = output.status.code().unwrap_or(-1);
    Ok((code, String::from_utf8_lossy(&output.stdout).to_string()))
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
