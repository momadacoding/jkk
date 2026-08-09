use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use std::path::PathBuf;
use std::process::Stdio;

fn not_found(program: &str) -> GitError {
    GitError::new(
        "launcher",
        format!("找不到可执行文件 `{program}`，请检查 customCommands.json"),
    )
}

/// 把裸程序名解析成实际存在的可执行文件路径。
///
/// Windows 专属：CreateProcess 查 PATH 时只补 `.exe`，而 VS Code 装的是
/// `code.cmd`——直接 `Command::new("code")` 必然 NotFound。这里按 PATHEXT
/// 自己搜一遍，拿到带扩展名的路径再交给 std。
///
/// 交给 std 之后 `.cmd` / `.bat` 由它经 cmd.exe 启动并转义参数（CVE-2024-24576
/// 的修复，1.77.2 起）。实测确认含空格与 `&` 的参数能作为单个 argv 完整送达，
/// 因此这里不需要自己拼 `cmd /c`。
#[cfg(windows)]
pub fn resolve_program(program: &str) -> Result<PathBuf> {
    let path = std::path::Path::new(program);
    if path.extension().is_some() || program.contains('\\') || program.contains('/') {
        return Ok(PathBuf::from(program));
    }
    let exts = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    let dirs = std::env::var_os("PATH").ok_or_else(|| not_found(program))?;
    for dir in std::env::split_paths(&dirs) {
        for ext in exts.split(';').filter(|e| !e.is_empty()) {
            let candidate = dir.join(format!("{program}{ext}"));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(not_found(program))
}

/// 非 Windows 平台交给系统自己查 PATH，找不到时由 spawn 报 NotFound。
#[cfg(not(windows))]
pub fn resolve_program(program: &str) -> Result<PathBuf> {
    Ok(PathBuf::from(program))
}

/// 启动一个外部程序并立即返回。
///
/// 四个要点：
/// 1. 只 `spawn` 不 `wait`——等编辑器退出会把整个 UI 卡死。
/// 2. 标准流全部接到 null，不让子进程继承 GUI 进程的句柄。
/// 3. args 是数组，直传给 CreateProcess/execvp，**不经过 shell**。
///    仓库路径里的空格、`&`、中文因此不会被解析成新命令。
/// 4. 走 [`hidden_command`] 不分配 console 窗口。`code.cmd` 这类 `.cmd`/`.bat`
///    由 std 经 cmd.exe 启动，在装机版上会闪黑框；反正标准流已经全接到 null，
///    那个窗口里本来也看不到任何输出。
pub fn spawn_detached(program: &str, args: &[String], cwd: Option<&str>) -> Result<()> {
    let resolved = resolve_program(program)?;
    let mut command = hidden_command(&resolved);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => not_found(program),
        _ => GitError::new("launcher", format!("启动 `{program}` 失败：{e}")),
    })?;

    // Unix 上不 wait 会留下僵尸进程。丢给一个后台线程去收尸：
    // 它只是阻塞在 wait 上，不占 CPU，也不挡住调用方。
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

/// 用系统默认程序打开一个路径。
///
/// Windows 上 `explorer.exe <file>` 会走默认关联程序打开文件；它的退出码
/// 常年非 0，但我们本来就不看退出码。这样就不必为「打开文件」引入新插件。
pub fn open_in_default_app(path: &str) -> Result<()> {
    let args = [path.to_string()];
    #[cfg(windows)]
    {
        spawn_detached("explorer.exe", &args, None)
    }
    #[cfg(target_os = "macos")]
    {
        spawn_detached("open", &args, None)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        spawn_detached("xdg-open", &args, None)
    }
}
