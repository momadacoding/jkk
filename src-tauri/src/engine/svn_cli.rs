//! svn 子进程的唯一入口。形状对齐 `engine::git_cli`。

use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use std::process::Command;

/// 构造一条指向 `path` 的 svn 命令。
///
/// 走 [`hidden_command`] 是为了不在装机版上闪出 console 黑框，理由与 git 完全
/// 相同：release 构建是 GUI 子系统，启动 console 程序会被系统新分配一个窗口。
///
/// `--non-interactive` 由 [`run_svn`] / [`run_svn_code`] 注入，不在这里加——
/// 这个函数也被可用性探测复用，那里跑的是 `svn --version`，没有工作目录可言。
pub fn svn_command(path: &str) -> Command {
    let mut cmd = hidden_command("svn");
    cmd.current_dir(path);
    cmd
}

/// 执行一条 svn 命令，成功返回 stdout，失败把 stderr 包成 GitError。
///
/// **`--non-interactive` 在这里统一注入**，调用点不需要（也不应该）自己加。
/// 理由与 `GIT_TERMINAL_PROMPT=0` 一字不差：GUI 应用没有终端，svn 一旦等待
/// stdin 输入凭据就会永久挂死并占住 Tauri 线程池。它是 svn 的**全局选项**，
/// 所有子命令都接受（2026-09-13 实测 info/status/log/diff/update/commit 六个）。
///
/// ⚠️ **`--ignore-externals` 绝不能照此办理。** 它只被 `status` / `update` 接受，
/// `info` 与 `diff` 会以 `Subcommand 'x' doesn't accept option` 退出 1，而
/// `svn info` 正是仓库识别、UUID 校验与记账单位判定的唯一入口——统一注入会让
/// 这三处全部失败，仓库根本打不开。哪些调用该带由调用点决定，见 spec §4.4 的
/// 子命令表。
pub fn run_svn(path: &str, args: &[&str]) -> Result<String> {
    let output = svn_command(path).args(with_global(args)).output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("svn_cli", stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// 同 [`run_svn`]，但把**退出码和 stderr 都交给调用方判断**，非零不算失败。
///
/// ⚠️ **不要照抄 `git_cli::run_git_code` 只返回 `(code, stdout)` 的形状。**
/// 那个形状对 git 成立，因为 git 用 1 与 128 区分「查询无结果」和「fatal」；
/// **svn 没有这层区分**——各类失败基本一律退出 1，光看退出码分不出任何两种
/// 失败。所以这里必须把 stderr 一起给出去，否则调用方拿到的信息还不如
/// [`run_svn`]（后者至少把 stderr 包进了 GitError）。
///
/// 进程本身起不来仍然返回 `Err`。
pub fn run_svn_code(path: &str, args: &[&str]) -> Result<SvnOutput> {
    let output = svn_command(path).args(with_global(args)).output()?;
    // 正常退出时 code() 必有值；拿不到只可能是被信号杀掉（Windows 上不存在），
    // 那种情况用 -1 表示「未知」，调用方不会把它误当成某个有意义的退出码。
    Ok(SvnOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// 一次 svn 调用的完整结果。
#[derive(Debug, Clone)]
pub struct SvnOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// 把全局选项拼到调用方给的参数前面。
/// 在参数表最前面插入全局选项。
///
/// **是 `pub` 的，为的是能被直接断言。** 只测「各子命令不因这个参数而失败」
/// 证明不了注入真的发生了——把本函数改成原样返回，那种测试照样全绿
/// （2026-09-13 独立 review 指出）。这是个纯函数，没有需要靠私有性保护的不变量，
/// 公开出去不损失任何东西。
pub fn with_global<'a>(args: &[&'a str]) -> Vec<&'a str> {
    let mut full: Vec<&str> = Vec::with_capacity(args.len() + 1);
    full.push("--non-interactive");
    full.extend_from_slice(args);
    full
}

/// svn 的三种可用状态。
///
/// `StalePath` 必须与 `NotInstalled` 分开，这是 spec §17.1 的要求：安装器写的是
/// **持久** PATH，而已经在跑的进程持有启动时的环境快照。用户装完 SVN 之后，
/// 一个更早启动的 Tauri 应用**仍然看不见** svn，而用户会认为「我明明装了」。
/// 两种状态的正确引导完全相反——一个是「去安装」，一个是「重启应用」。
#[derive(Debug, Clone, PartialEq)]
pub enum SvnAvailability {
    /// 当前进程能跑起 svn。`version` 形如 `1.14.2-SlikSvn`。
    Available { version: String },
    /// 当前进程环境里没有，但持久 PATH 里有 → 提示**重启应用**，别让用户重装。
    StalePath,
    /// 持久 PATH 里也没有 → 引导去安装。
    NotInstalled,
}

/// 探测 svn 是否可用。
pub fn probe_svn() -> SvnAvailability {
    // `--version --quiet` 只打一行版本号，是最便宜的存活探测（实测输出
    // `1.14.2-SlikSvn`）。用 `hidden_command` 而不是 `svn_command`：这里没有
    // 工作目录可言，设 current_dir 反而可能指向一个不存在的路径。
    if let Ok(out) = hidden_command("svn")
        .args(["--version", "--quiet"])
        .output()
    {
        if out.status.success() {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !version.is_empty() {
                return SvnAvailability::Available { version };
            }
        }
    }
    if persistent_path_has_svn() {
        SvnAvailability::StalePath
    } else {
        SvnAvailability::NotInstalled
    }
}

/// 持久 PATH（注册表里的 Machine + User）中是否存在一个含 svn 可执行文件的目录。
///
/// **走 `reg.exe` 而不是引入注册表 crate**：仓库现有的 `windows` crate 只打开了
/// `Win32_Foundation` + `Win32_System_RestartManager` 两个 feature，为读一次 PATH
/// 再打开 `Win32_System_Registry` 不划算；而「shell out 到 CLI」本来就是本项目
/// 既有的做法（git、svn 都是）。`reg query` 是只读的。
///
/// ⚠️ 手工核对这段行为**不要在 Git Bash 里跑** `reg query ... /v Path`：MSYS 会把
/// `/v` 当成路径改写成 `C:/Program Files/Git/v`，得到 `ERROR: Invalid syntax`
/// （2026-09-13 实测）。Rust 的 `std::process::Command` 不经过这层转换，没有这个
/// 问题；要手工验证请走 PowerShell。
#[cfg(windows)]
fn persistent_path_has_svn() -> bool {
    const MACHINE: &str = r"HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
    const USER: &str = r"HKCU\Environment";
    [MACHINE, USER].iter().any(|key| {
        let Ok(out) = hidden_command("reg")
            .args(["query", key, "/v", "Path"])
            .output()
        else {
            return false;
        };
        if !out.status.success() {
            return false;
        }
        path_dirs_from_reg_output(&String::from_utf8_lossy(&out.stdout))
            .iter()
            .any(|d| dir_has_svn_exe(d))
    })
}

/// 从 `reg query … /v Path` 的输出里抽出目录列表。
///
/// **不能直接把整段 stdout 按 `;` 切。** 实测输出是三行：
///
/// ```text
/// <空行>
/// HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Session Manager\Environment
///     Path    REG_EXPAND_SZ    C:\第一个目录;C:\第二个目录;…
/// ```
///
/// 整段 split 之后，**第一个 chunk 是「注册表键名 + 换行 + `Path REG_EXPAND_SZ ` +
/// 第一个目录」**（2026-09-13 实测，45 个 chunk 里第一个正是这个形态）。`trim()`
/// 只去首尾空白，中间的换行和键名还在，于是 `join("svn.exe").is_file()` 恒为 false
/// ——**PATH 的第一个条目被静默吞掉**。
///
/// 这不是理论问题：安装器把自己 prepend 到 PATH 是常见做法，SVN 恰好排第一时，
/// `probe_svn()` 会报 `NotInstalled` 并引导用户「去安装」，而这个函数存在的**唯一
/// 理由**就是要在这种情况下说「重启应用」——两种引导正好相反。
///
/// 正确做法：先定位值所在的那一行，切掉类型标记（`REG_SZ` / `REG_EXPAND_SZ`）
/// 之前的部分，再按 `;` 切。
///
/// **有意不加 `#[cfg(windows)]`**：它是纯字符串处理，在任何平台上都能编译，
/// 这样 `tests/svn_cli.rs` 不必按平台分叉就能覆盖它。调用方才是 Windows 专属的。
pub fn path_dirs_from_reg_output(raw: &str) -> Vec<String> {
    for line in raw.lines() {
        // 类型标记两侧是若干空格；取标记之后的全部内容就是值本身。
        for marker in ["REG_EXPAND_SZ", "REG_SZ"] {
            if let Some(idx) = line.find(marker) {
                return line[idx + marker.len()..]
                    .split(';')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
    Vec::new()
}

/// 非 Windows 平台上没有「持久 PATH 与进程 PATH 不一致」这回事——shell 的 rc
/// 文件要重开终端才生效，用户对此有直觉，不需要客户端解释。
#[cfg(not(windows))]
fn persistent_path_has_svn() -> bool {
    false
}

#[cfg(windows)]
fn dir_has_svn_exe(dir: &str) -> bool {
    // 只看文件在不在，不执行它。不展开 %SystemRoot% 这类变量：svn 的安装目录
    // 不会藏在变量后面，而展开会引入一整套转义问题。
    std::path::Path::new(dir).join("svn.exe").is_file()
}
