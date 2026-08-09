//! 守住「不许有第二个 spawn 入口」这条约束。
//!
//! release 构建带 `windows_subsystem = "windows"`（`src/main.rs`），GUI 进程本身
//! 没有 console。Windows 的 CreateProcess 在这种父进程下启动 console 子进程时，
//! 会**新分配一个 console 窗口**——git.exe 正是 console 程序，于是 pull、删分支、
//! commit 等每个操作都闪一下黑框。debug 构建是 console 子系统、有 console 可继承，
//! 所以 `pnpm tauri dev` 里完全看不到这个缺陷，只有装机版才暴露。
//!
//! 解法是 `CREATE_NO_WINDOW`，统一封在 `engine::proc::hidden_command` 里。
//! 这个测试保证今后新增的子进程调用不会绕开它——漏掉不会编译失败、不会被
//! clippy 抓到，只会在装机版上重新开始闪。

use std::fs;
use std::path::{Path, PathBuf};

/// 唯一允许出现裸 `Command::new` 的地方：helper 自己。
const ALLOWED: &str = "proc.rs";

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("读取源码目录失败") {
        let path = entry.expect("读取目录项失败").path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_subprocess_goes_through_hidden_command() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(src.is_dir(), "找不到源码目录: {}", src.display());

    let mut files = Vec::new();
    collect_rs(&src, &mut files);
    assert!(!files.is_empty(), "一个源文件都没扫到，测试本身失效了");

    let mut offenders = Vec::new();
    let mut allowed_hits = 0usize;

    for path in &files {
        let text = fs::read_to_string(path).expect("读取源文件失败");
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        for (i, line) in text.lines().enumerate() {
            // `#[cfg(test)]` 之后是单测模块，跑在 console 子系统的测试进程里，
            // 不受这条约束管；本仓库的测试模块一律放在文件末尾。
            if line.trim_start().starts_with("#[cfg(test)]") {
                break;
            }
            // 注释里提 Command::new 是在解释这段历史，不是调用
            if line.trim_start().starts_with("//") {
                continue;
            }
            if !line.contains("Command::new(") {
                continue;
            }
            if name == ALLOWED {
                allowed_hits += 1;
                continue;
            }
            offenders.push(format!("{}:{}: {}", name, i + 1, line.trim()));
        }
    }

    assert!(
        offenders.is_empty(),
        "以下位置直接构造了 Command，装机版上每次调用都会闪出一个 console 黑框：\n  {}\n\
         改用 engine::proc::hidden_command（git 命令则走 engine::git_cli::run_git）。",
        offenders.join("\n  ")
    );
    assert!(
        allowed_hits > 0,
        "在 {ALLOWED} 里没扫到 Command::new，helper 可能被改名或挪走了，\
         这个测试已经失去意义，请同步更新 ALLOWED。"
    );
}
