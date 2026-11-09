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
    run_git_stdin_env(path, args, stdin_bytes, &[])
}

/// 同 [`run_git_stdin`]，额外注入一组环境变量（理由同 [`run_git_env`]）。
///
/// 已知需要它的地方：在临时索引（`GIT_INDEX_FILE`）上 `update-index --add --stdin`
/// ——路径可能有几千条，走命令行会撞上 Windows 的命令行长度上限。
pub fn run_git_stdin_env(
    path: &str,
    args: &[&str],
    stdin_bytes: &[u8],
    envs: &[(&str, &str)],
) -> Result<String> {
    let (output, written) = pipe_through(path, args, stdin_bytes, envs)?;
    // git 先失败退出时写入会撞上断开的管道；那时真正的原因在 stderr 里，先报它
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    written?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 让 git 从 stdin 读 NUL 分隔的路径。由 [`run_git_paths`] 自己接在参数末尾，调用方不拼。
///
/// `add` / `reset` / `checkout` / `rm` 都认（git 2.26 起 `rm` 也认）；`ls-files`、`ls-tree`
/// 不认，那两条只读探测改用 [`cmdline_batches`] 分批。
const PATHSPEC_STDIN: [&str; 3] = ["--pathspec-from-file=-", "--pathspec-file-nul", "--"];

/// 跑一条带路径列表的 git **写**命令，路径从 stdin 喂，不拼进命令行。
///
/// `args` 只写到路径之前为止（如 `["--literal-pathspecs", "reset", "HEAD"]`），这里在末尾补上
/// `--pathspec-from-file=- --pathspec-file-nul --`。
///
/// # 为什么
///
/// Windows 的命令行上限是 32767 个 UTF-16 字符。2 万文件的 SVN 混合 fixture 上实测
/// （2026-11-09）：区头 Stage All 一次交下 631 个路径，`git add -- <路径…>` 起不来，
/// 报 `The filename or extension is too long. (os error 206)`。写命令不能拆批：拆了就
/// 不再是一次原子操作，中途失败时一半路径已经改了、一半没改。从 stdin 读还是一条命令。
///
/// # 为什么末尾的 `--` 不能省
///
/// 从文件读的路径不会被当成选项，但 `reset HEAD` / `checkout HEAD` 里的 `HEAD` 没有 `--`
/// 收尾时，git 要先判断它是 revision 还是文件名：还没有提交的仓库里报
/// `ambiguous argument 'HEAD': unknown revision`，根目录有个叫 `HEAD` 的文件时报
/// `both revision and filename`，Unstage 整个用不了（独立 review 实测，2026-11-09）。
/// 由这里统一补，不交给每个调用方记——stdin 参数漏写同样危险：git 不读 stdin，
/// `reset HEAD --` 就成了清空整个暂存区。
///
/// `--literal-pathspecs` 对从文件读的路径照样生效（`tests/long_pathspec.rs` 里的 `f[1].txt`）。
///
/// # 空列表是错误，不是 no-op
///
/// 空的 pathspec 文件对 git 来说就是「没给 pathspec」，`reset` 会因此清空整个暂存区
/// （`AGENTS.md` §6.2）。调用方本该先挡住空列表；漏挡时这里报错，绝不起进程。
pub fn run_git_paths<S: AsRef<str>>(path: &str, args: &[&str], paths: &[S]) -> Result<String> {
    if paths.is_empty() {
        return Err(GitError::new(
            "empty_pathspec",
            "内部错误：没有路径却要执行带路径的 git 命令，已拒绝（空 pathspec 等于「全部」）"
                .to_string(),
        ));
    }
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_ref().as_bytes());
        input.push(0);
    }
    let mut full: Vec<&str> = args.to_vec();
    full.extend(PATHSPEC_STDIN);
    run_git_stdin(path, &full, &input)
}

/// 每一批路径在命令行上最多占多少个 UTF-16 字符。
///
/// 上限 32767 还要装下 git 的路径和前面的参数，留 8000 余量。批数要少：`ls-files --others`
/// 每跑一批都可能扫一遍工作区（独立 review 指出），预算越小扫得越多。
const CMDLINE_BATCH_BUDGET: usize = 24000;

/// 把路径切成一批批，每批拼进命令行都放得下。只给**只读**命令用（见 [`run_git_paths`]
/// 为什么写命令不能拆批）。
///
/// 每批都非空，空输入得到零批：调用方按批跑命令，永远不会跑出一条不带 pathspec、
/// 等于查询「全部」的命令。单个路径就超出预算时独占一批（真实路径到不了这个长度）。
pub fn cmdline_batches<'a>(paths: &[&'a str]) -> Vec<Vec<&'a str>> {
    let mut batches: Vec<Vec<&'a str>> = Vec::new();
    let mut current: Vec<&'a str> = Vec::new();
    let mut used = 0;
    for &p in paths {
        // 加 3：引号一对、分隔的空格一个
        let cost = p.encode_utf16().count() + 3;
        if !current.is_empty() && used + cost > CMDLINE_BATCH_BUDGET {
            batches.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(p);
        used += cost;
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// 同 [`run_git_stdin`]，但退出码 1 不算失败（与 [`run_git_code`] 同理），
/// 而且 fatal 时**把已经输出的部分一并交回**。返回 `(stdout, fatal)`。
///
/// 已知需要它的地方：`check-ignore --stdin`。
///
/// - 它在「一个路径都没被忽略」时退出 1，而那正是核对报告里「多收」一侧的常态。
/// - 它逐个路径输出、遇到某个路径 fatal 就整个退出（比如路径越过了符号链接）。
///   调用方拿着 fatal 之前的输出，就能跳过出事的那个路径接着查，而不是整批作废。
///
/// fatal 仍然以 `Some(GitError)` 明确交回——把它当成「没有结果」会让报告看起来
/// 正常、其实什么也没查。
pub fn run_git_stdin_partial(
    path: &str,
    args: &[&str],
    stdin_bytes: &[u8],
) -> Result<(String, Option<GitError>)> {
    let (output, written) = pipe_through(path, args, stdin_bytes, &[])?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let code = output.status.code().unwrap_or(-1);
    if code != 0 && code != 1 {
        // git 中途退出时写入撞上断开的管道是预期内的，原因在 stderr 里
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Ok((stdout, Some(GitError::new("git_cli", stderr))));
    }
    written?;
    Ok((stdout, None))
}

/// 起进程、一边写 stdin 一边读空 stdout / stderr。写入结果单独交回，由调用方
/// 决定先看退出码还是先看写入——见 [`run_git_stdin`] 开头的两个坑。
fn pipe_through(
    path: &str,
    args: &[&str],
    stdin_bytes: &[u8],
    envs: &[(&str, &str)],
) -> Result<(std::process::Output, Result<()>)> {
    use std::io::Write;
    use std::process::Stdio;

    let mut cmd = git_command(path);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let mut child = cmd
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
    let written = match written {
        Ok(r) => r.map_err(GitError::from),
        Err(_) => Err(GitError::new(
            "git_cli",
            "写入子进程 stdin 的线程异常退出".to_string(),
        )),
    };
    Ok((output?, written))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_paths_are_refused_without_starting_git() {
        // 路径根本不存在：要是起了进程，报的会是 io 错误而不是 empty_pathspec
        let none: [&str; 0] = [];
        let err = run_git_paths("Z:/no/such/dir", &["reset", "HEAD"], &none).unwrap_err();
        assert_eq!(err.code, "empty_pathspec");
    }

    #[test]
    fn batches_keep_every_path_in_order_and_stay_under_budget() {
        let owned: Vec<String> = (0..5000)
            .map(|i| format!("目录/sub/file_{i:05}.txt"))
            .collect();
        let paths: Vec<&str> = owned.iter().map(String::as_str).collect();
        let batches = cmdline_batches(&paths);
        assert!(batches.len() > 1, "这么多路径应当分成多批");
        for b in &batches {
            assert!(!b.is_empty());
            let cost: usize = b.iter().map(|p| p.encode_utf16().count() + 3).sum();
            assert!(cost <= CMDLINE_BATCH_BUDGET, "一批 {cost} 超出预算");
        }
        let flat: Vec<&str> = batches.concat();
        assert_eq!(flat, paths, "不丢、不重、不乱序");
    }

    #[test]
    fn empty_input_gives_no_batches() {
        assert!(cmdline_batches(&[]).is_empty());
    }

    #[test]
    fn an_oversized_path_gets_a_batch_of_its_own() {
        let long = "x".repeat(CMDLINE_BATCH_BUDGET + 10);
        let batches = cmdline_batches(&["a.txt", &long, "b.txt"]);
        assert_eq!(
            batches,
            vec![vec!["a.txt"], vec![long.as_str()], vec!["b.txt"]]
        );
    }
}
