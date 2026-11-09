use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use std::fmt;
use std::process::Command;
use std::sync::OnceLock;

/// git 的版本号，只取前三段（`2.43.0.windows.1` → `2.43.0`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GitVersion(pub u32, pub u32, pub u32);

impl fmt::Display for GitVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// 本客户端对**所有** git 调用要求的最低版本。
///
/// 把代码里出现过的 git 参数全列出来，按 Git for Windows 自带的 RelNotes
/// （`mingw64/share/doc/git-doc/RelNotes`）逐条查出现的版本（2026-11-09）。从新到旧：
///
/// | 版本 | 参数 | 用在哪 | 谁来挡 |
/// |---|---|---|---|
/// | 2.43 | `merge-file --object-id` | SVN 跟进合并暂存区（`svn_follow_index`） | 跟进自己的预检，`svn_follow_git_too_old` |
/// | 2.40 | `merge-tree --merge-base` | SVN 回放（`svn_replay`） | 同上；交付只在慢路径用到，退出 129 时报 `sync_out_git_too_old` |
/// | 2.38 | `merge-tree --write-tree` | 同上 | 同上 |
/// | 2.36 | `worktree list --porcelain -z` | worktree 列表 | **这里** |
/// | 2.31 | `rev-parse --path-format=absolute` | 新建 worktree | 这里 |
/// | 2.26 | `rm --pathspec-from-file`（`add` / `reset` / `checkout` 是 2.25） | Stage / Unstage / Discard | 这里 |
///
/// 上面三行 SVN 的功能本来就按功能降级（设计见 `docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out.md`
/// B2、`2026-10-31-svn-mixed-baseline-c1a-decisions.md` D9）：git 不够新只是那一个功能不能用，
/// 交付的快路径照常。所以这里的全局下限取**其余**功能里最新的那个，不把它们一起抬到 2.43；
/// 否则 2.36–2.42 上原本能用的看图、提交、Stage 也会被一起拒绝。
///
/// 往代码里加更新的参数时：要么同步抬高这里和 README 的「环境要求」，要么像 SVN 那样
/// 给那个功能单独加预检，二选一，并补进上表。
pub const MIN_GIT_VERSION: GitVersion = GitVersion(2, 36, 0);

/// 从 `git --version` 的输出里取版本号。
///
/// 实际见过的形态：`git version 2.51.2.windows.1`（Git for Windows）、
/// `git version 2.39.3 (Apple Git-145)`、`git version 2.45.0-rc1.windows.1`。
/// 每段只取开头的数字；主、次版本号缺一个就认不出，补丁号缺了按 0。
pub fn parse_git_version(output: &str) -> Option<GitVersion> {
    let rest = output.trim().strip_prefix("git version ")?;
    let token = rest.split_whitespace().next()?;
    let mut parts = token.split('.').map(|part| {
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse::<u32>().ok()
    });
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next().flatten().unwrap_or(0);
    Some(GitVersion(major, minor, patch))
}

/// 判断 `git --version` 的输出是否满足 [`MIN_GIT_VERSION`]。
///
/// 认不出版本号时**拒绝**而不是放行：放行的话，用到新参数的那一刻才失败，
/// 那正是这道检查要防的情况。真实的 git 一直是 `git version X.Y.Z` 这个格式，
/// 走到这里多半是 PATH 上有个冒名的 `git`；报错里带上原始输出，便于排查。
pub fn check_git_version(output: &str) -> Result<GitVersion> {
    match parse_git_version(output) {
        Some(v) if v >= MIN_GIT_VERSION => Ok(v),
        Some(v) => Err(GitError::new(
            "git_too_old",
            format!(
                "本机的 git 是 {v}，本客户端需要 {MIN_GIT_VERSION} 及以上。\
                 请升级 Git for Windows（https://git-scm.com/download/win）后重试，不用重启本客户端。"
            ),
        )),
        None => Err(GitError::new(
            "git_version_unknown",
            format!(
                "认不出 PATH 上 git 的版本（`git --version` 输出：{:?}）。\
                 请确认装的是 Git for Windows {MIN_GIT_VERSION} 及以上。",
                output.trim()
            ),
        )),
    }
}

/// 进程内已经确认过的 git 版本。只缓存「满足要求」：不满足时每次都重新探测，
/// 用户升级 git 后不用重启也能恢复（探测一次约几十毫秒，只在出错的机器上反复发生）。
static SUPPORTED_GIT: OnceLock<GitVersion> = OnceLock::new();

/// 确认 PATH 上的 git 满足 [`MIN_GIT_VERSION`]，结果见 [`SUPPORTED_GIT`]。
///
/// 放在 [`git_command`] 里，是因为所有 git 子进程都从那里起：在这里拦住，
/// 就不会有哪条命令在老 git 上跑到一半才失败。比如 Discard 先 `checkout` 再 `rm`，
/// 在 2.25 上 `checkout` 成功、`rm` 不认 `--pathspec-from-file`，选中的文件只丢弃了一部分。
fn ensure_supported_git() -> Result<()> {
    ensure_supported_git_with(&SUPPORTED_GIT, || {
        hidden_command("git")
            .arg("--version")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
    })
}

/// [`ensure_supported_git`] 的本体。缓存和探测都由参数传入，单测才能造出
/// 「git 太老」「找不到 git」这些本机造不出来的情况，并确认失败不会被缓存。
fn ensure_supported_git_with(
    cache: &OnceLock<GitVersion>,
    probe: impl FnOnce() -> std::io::Result<std::process::Output>,
) -> Result<()> {
    if cache.get().is_some() {
        return Ok(());
    }
    let output = probe().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            GitError::new(
                "git_not_found",
                format!(
                    "找不到 git。请安装 Git for Windows {MIN_GIT_VERSION} 及以上\
                     （https://git-scm.com/download/win），确认它在 PATH 里，然后重启本客户端。"
                ),
            )
        } else {
            GitError::from(e)
        }
    })?;
    if !output.status.success() {
        // stderr 常常是空的；不写明是哪一步，横幅上就只剩一个空白的 git_cli
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GitError::new(
            "git_version_unknown",
            format!(
                "检查 git 版本时 `git --version` 失败（{}）：{}。\
                 请确认 PATH 上的 git 是 Git for Windows {MIN_GIT_VERSION} 及以上。",
                output.status,
                stderr.trim()
            ),
        ));
    }
    let v = check_git_version(&String::from_utf8_lossy(&output.stdout))?;
    // 并发的首次调用可能都走到这里，谁先写进去都一样
    let _ = cache.set(v);
    Ok(())
}

/// 构造一个指向 `path` 的 git 命令，并关掉所有交互式提示。
///
/// 返回 `Result`：先过 [`ensure_supported_git`]。检查放在这里而不是各个 `run_git*` 里，
/// 是为了让「拿到一个 git 命令」和「过了版本检查」成为同一件事，没有绕过它的路。
///
/// `GIT_TERMINAL_PROMPT=0` 是必须的：GUI 应用没有终端，git 一旦往 stderr
/// 打印 "Username:" 就会无限等待 stdin，子进程永久挂死并占住 Tauri 的线程池。
/// 关掉之后 git 会立即失败，错误能走到界面上的错误横幅。
///
/// 凭据管理器（Windows 的 GCM）弹自己的 GUI 窗口不受影响——那条路径不经过终端。
///
/// 走 [`hidden_command`] 是为了不在装机版上闪出 console 黑框，理由见该函数。
pub fn git_command(path: &str) -> Result<Command> {
    ensure_supported_git()?;
    let mut cmd = hidden_command("git");
    cmd.current_dir(path);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    Ok(cmd)
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
    let output = git_command(path)?.args(args).output()?;
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
    let mut cmd = git_command(path)?;
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

    let mut cmd = git_command(path)?;
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
    let mut cmd = git_command(path)?;
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
    fn version_is_read_from_every_known_output_shape() {
        for (raw, want) in [
            ("git version 2.51.2.windows.1\n", GitVersion(2, 51, 2)),
            ("git version 2.39.3 (Apple Git-145)", GitVersion(2, 39, 3)),
            ("git version 2.45.0-rc1.windows.1", GitVersion(2, 45, 0)),
            ("git version 2.43.0", GitVersion(2, 43, 0)),
            ("git version 2.44", GitVersion(2, 44, 0)),
            ("  git version 3.0.0  \r\n", GitVersion(3, 0, 0)),
        ] {
            assert_eq!(parse_git_version(raw), Some(want), "{raw:?}");
        }
    }

    #[test]
    fn unrecognisable_output_gives_no_version() {
        for raw in [
            "",
            "git version",
            "git version abc",
            "git version 2",
            "git version .43.0",
            "hub version 2.14.2",
        ] {
            assert_eq!(parse_git_version(raw), None, "{raw:?}");
        }
    }

    #[test]
    fn the_minimum_version_passes_and_anything_older_is_refused() {
        assert_eq!(
            check_git_version("git version 2.36.0.windows.1").unwrap(),
            MIN_GIT_VERSION
        );
        // 2.36–2.42 只是 SVN 跟进不能用，其余照常，全局这一道必须放行
        assert!(check_git_version("git version 2.39.3 (Apple Git-145)").is_ok());
        assert!(check_git_version("git version 2.51.2.windows.1").is_ok());
        assert!(check_git_version("git version 3.0.0").is_ok());
        // 按数字比，不按字符串比：字符串比较下 "2.9" 会排在 "2.36" 后面
        for old in [
            "git version 2.35.9",
            "git version 2.9.5",
            "git version 2.25.1.windows.1",
            "git version 1.99.0",
        ] {
            let err = check_git_version(old).unwrap_err();
            assert_eq!(err.code, "git_too_old", "{old}");
            assert!(
                err.message.contains("2.36.0"),
                "报错要写出需要的版本：{}",
                err.message
            );
        }
        let v = check_git_version("git version 2.35.9").unwrap_err();
        assert!(v.message.contains("2.35.9"), "报错要写出本机的版本");
    }

    #[test]
    fn unrecognisable_version_is_refused_not_waved_through() {
        let err = check_git_version("not git at all").unwrap_err();
        assert_eq!(err.code, "git_version_unknown");
        assert!(
            err.message.contains("not git at all"),
            "带上原始输出便于排查"
        );
    }

    fn fake_output(code: u32, stdout: &str) -> std::io::Result<std::process::Output> {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        // unix 的原始值是 wait 状态，退出码在高 8 位
        #[cfg(unix)]
        let raw = (code as i32) << 8;
        #[cfg(windows)]
        let raw = code;
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(raw),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }

    #[test]
    fn an_old_git_is_refused_and_asked_again_next_time() {
        let cache = OnceLock::new();
        let err =
            ensure_supported_git_with(&cache, || fake_output(0, "git version 2.25.1.windows.1"))
                .unwrap_err();
        assert_eq!(err.code, "git_too_old");
        assert!(cache.get().is_none(), "失败不能缓存，升级 git 后要能恢复");

        // 用户升级之后，同一进程里下一次调用就放行
        ensure_supported_git_with(&cache, || fake_output(0, "git version 2.51.2.windows.1"))
            .unwrap();
        assert_eq!(cache.get(), Some(&GitVersion(2, 51, 2)));
    }

    #[test]
    fn once_passed_git_is_not_probed_again() {
        let cache = OnceLock::new();
        ensure_supported_git_with(&cache, || fake_output(0, "git version 2.43.0")).unwrap();
        ensure_supported_git_with(&cache, || panic!("已经确认过的版本不该再探测")).unwrap();
    }

    #[test]
    fn missing_git_says_so_instead_of_a_bare_io_error() {
        let cache = OnceLock::new();
        let err = ensure_supported_git_with(&cache, || {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        })
        .unwrap_err();
        assert_eq!(err.code, "git_not_found");
        assert!(err.message.contains("2.36.0"), "要写出需要的版本");
        assert!(cache.get().is_none());
    }

    #[test]
    fn a_failing_version_probe_is_an_error_not_a_pass() {
        let cache = OnceLock::new();
        let err = ensure_supported_git_with(&cache, || fake_output(1, "")).unwrap_err();
        assert_eq!(err.code, "git_version_unknown");
        assert!(
            err.message.contains("git --version"),
            "要写明是版本检查失败"
        );
        assert!(cache.get().is_none());
    }

    #[test]
    fn the_git_on_this_machine_passes_and_is_remembered() {
        // 走 git_command 而不是直接调 ensure_supported_git，顺带守「拿命令必过检查」的接线。
        // 只在单独跑这一条时可靠（`cargo test --lib git_cli::tests::the_git`）：缓存是进程级的，
        // 整套跑时别的测试经 run_git 可能先把它填上。把检查挪出 git_command 后单独跑，实测失败（2026-11-09）
        git_command(".").unwrap();
        assert!(SUPPORTED_GIT.get().is_some_and(|v| *v >= MIN_GIT_VERSION));
    }

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
