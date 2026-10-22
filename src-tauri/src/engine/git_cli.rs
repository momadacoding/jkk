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

/// 跑一条已经组装好的 git 命令：成功返回 stdout 字节，失败把 stderr 包成 GitError。
///
/// 抽出来是为了让 [`run_git_bytes`] 和 [`run_git_env`] 共用**同一套失败映射**。
/// 两处各写一遍 `if !status.success()`，迟早会有一处漏掉而把失败当成空输出——
/// 对基线录制来说「`git add` 悄悄失败」会直接录出一个残缺的 tree。
fn output_or_error(mut cmd: Command) -> Result<Vec<u8>> {
    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    Ok(output.stdout)
}

/// 同 [`run_git`]，但原样返回 stdout 的字节。
///
/// patch 要落成文件，不能过 `from_utf8_lossy`：非 UTF-8 的文件名或内容会被替换成
/// U+FFFD，生成的 patch 打回去就对不上了。需要写文件的场景一律走这个。
pub fn run_git_bytes(path: &str, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = git_command(path);
    cmd.args(args);
    output_or_error(cmd)
}

/// 同 [`run_git`]，但把一段**字节**喂给子进程的 stdin。
///
/// # 为什么需要它
///
/// `git hash-object -w --stdin` 是把「一段内存里的字节」变成 git 对象的唯一途径，
/// 而它只从 stdin 收。落成临时文件再 `hash-object <path>` 也行，但那要在用户的
/// 工作副本里创建和删除文件——录基线这条路径的全部安全性就建立在「不碰磁盘」上
/// （spec §16.1），为了省一个管道去破坏它不划算。
///
/// 内容是 `&[u8]` 不是 `&str`：源码文件很可能不是 UTF-8（国内 Unity 工程里 GBK 的
/// `.cs` 很常见），任何一次字符串化都会把非法字节换成 U+FFFD（§16.2 末）。
///
/// ⚠️ **写 stdin 和读 stdout 必须同时进行。** 两个坑：
///
/// - 不关掉 stdin 就等进程结束，双方各等各的：git 等更多输入，我们等它退出。
///   写入线程写完即 drop `ChildStdin`，管道随之关闭。
/// - 先把 stdin 全部写完再读 stdout，遇到边读边写的命令（`check-attr --stdin`）会死锁：
///   输出写满管道缓冲区后 git 阻塞在写 stdout 上、不再读 stdin，我们阻塞在写 stdin 上。
///   5 万文件的仓库做 merge driver 检查时实测卡死（2026-10-21）。所以 stdin 放在单独的
///   线程里写，主线程同时 `wait_with_output` 把 stdout / stderr 读空。
pub fn run_git_stdin(path: &str, args: &[&str], stdin_bytes: &[u8]) -> Result<String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = git_command(path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut sink = child.stdin.take().ok_or_else(|| {
        GitError::new("git_cli", "拿不到子进程的 stdin，无法喂入内容".to_string())
    })?;
    let (output, written) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || sink.write_all(stdin_bytes));
        let output = child.wait_with_output();
        (output, writer.join())
    });
    let output = output?;
    // git 先失败退出时写入会撞上断开的管道；那时真正的原因在 stderr 里，先报它
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    written
        .map_err(|_| GitError::new("git_cli", "写入子进程 stdin 的线程异常退出".to_string()))??;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 同 [`run_git`]，但额外注入一组环境变量。
///
/// # 为什么需要它
///
/// `GIT_INDEX_FILE` 是把 `git add` 引到**临时索引**上的唯一手段——`git add` 没有
/// `--index-output`（只有 `git read-tree` 有），而基线录制的全部安全性都建立在
/// 「不碰用户的真实暂存区」上（spec §16 R2）。
///
/// 走这里而不是在调用点自己 `Command::new("git")`，是因为那样会绕开
/// [`git_command`] 里的 `GIT_TERMINAL_PROMPT=0` 与 `hidden_command`——前者防子进程
/// 等 stdin 挂死，后者防装机版闪 console 黑框（`AGENTS.md` §7.3）。
pub fn run_git_env(path: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<String> {
    let mut cmd = git_command(path);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    Ok(String::from_utf8_lossy(&output_or_error(cmd)?).to_string())
}
