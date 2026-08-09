//! 守住「命令不得跑在 UI 线程上」这条约束。
//!
//! Tauri 2 里没有 `async` 标记的命令**在主线程上执行**（官方文档 calling-rust：
//! non-async commands run on the main thread）。而前端 `invoke` 是由 WebView2 的
//! 自定义协议处理器在**窗口过程内部**同步调用的，所以一个同步命令只要阻塞
//! （等 git 子进程、等网络），窗口消息泵就停摆，界面直接冻住、连拖动都做不到。
//!
//! 这不是假想风险：`read_worktrees_cmd` 等 `git worktree list` 的子进程输出，
//! 曾让启动后的窗口有约 5 秒无法拖动（用 SendMessageTimeout 实测，7 次复现 5 次）。
//!
//! 因此所有 `#[tauri::command]` 必须写成 `#[tauri::command(async)]`。

use std::fs;
use std::path::Path;

/// 扫源码而不是靠 review 盯着——新增命令时忘记加 `(async)` 是无声的性能回归，
/// 编译器和 clippy 都不会报。
#[test]
fn every_command_is_declared_async() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands");
    assert!(dir.is_dir(), "找不到命令目录: {}", dir.display());

    let mut offenders = Vec::new();
    let mut total = 0usize;

    for entry in fs::read_dir(&dir).expect("读取命令目录失败") {
        let path = entry.expect("读取目录项失败").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let src = fs::read_to_string(&path).expect("读取源文件失败");
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        for (i, line) in src.lines().enumerate() {
            let t = line.trim();
            if !t.starts_with("#[tauri::command") {
                continue;
            }
            total += 1;
            // 只接受 `(async)`；裸 `#[tauri::command]` 会落到主线程上
            if !t.contains("(async)") {
                offenders.push(format!("{}:{}: {}", name, i + 1, t));
            }
        }
    }

    assert!(total > 0, "一个命令都没扫到，测试本身失效了");
    assert!(
        offenders.is_empty(),
        "以下命令没有声明 async，会在 UI 线程上执行并冻住窗口：\n  {}\n\
         改成 #[tauri::command(async)]。",
        offenders.join("\n  ")
    );
}
