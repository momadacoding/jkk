//! 启动子进程的唯一入口。

use std::ffi::OsStr;
use std::process::Command;

/// Windows `CreateProcess` 的 `CREATE_NO_WINDOW`（`processthreadsapi.h`）。
///
/// 直接写字面量而不是用 `windows` crate 的常量：那个常量在
/// `Win32_System_Threading` feature 下，为一个 `u32` 打开整个 feature 不划算。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 构造一个不会闪出 console 黑框的 `Command`。
///
/// release 构建带 `windows_subsystem = "windows"`（见 `src/main.rs`），GUI 进程
/// 自己没有 console。Windows 在这种父进程下启动 console 子进程时会**新分配一个
/// console 窗口**，而 git.exe 正是 console 程序——于是 pull、删分支、commit 等
/// 每个操作都闪一下黑框。`CREATE_NO_WINDOW` 就是让系统别分配这个窗口。
///
/// debug 构建是 console 子系统、有 console 可继承，所以 `pnpm tauri dev` 里
/// 看不到这个缺陷，只有装机版才暴露——不要用 dev 窗口来验证它。
///
/// 该标志只影响 console 分配：对 GUI 程序（explorer.exe、VS Code）被系统忽略，
/// 也不影响 stdin/stdout/stderr 的管道重定向，因此 `output()` 照常拿得到 stdout。
#[cfg(windows)]
pub fn hidden_command<S: AsRef<OsStr>>(program: S) -> Command {
    use std::os::windows::process::CommandExt;
    let mut cmd = Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// 非 Windows 平台不存在「给子进程分配 console 窗口」这回事，直通即可。
#[cfg(not(windows))]
pub fn hidden_command<S: AsRef<OsStr>>(program: S) -> Command {
    Command::new(program)
}
