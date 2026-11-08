#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 守住全局单例的三条装配约束。读源码断言，不起进程：单例要两个真实进程才测得出来，
//! 那部分的验证记在 PR 里（release 二进制 + 隔离的 identifier 实测）。
//!
//! 这三条写错都不会编译失败，也不会被 clippy 抓到：
//! - 插件不是第一个注册：官方文档要求它先于其他插件，否则第二个实例可能已经初始化了
//!   托盘、注册了全局快捷键才退出——托盘图标闪一下，快捷键注册失败还会弹设置告警。
//! - debug 构建也注册：`pnpm tauri dev` 与安装版共用 identifier，开着安装版时 dev 会
//!   直接唤起安装版然后退出；多个 worktree 同时跑 dev，后起的会悄悄唤起先起的那个，
//!   看到的不是自己的代码。
//! - TortoiseSVN 钩子分流晚于 `run()`：钩子是同一个 exe 带 `--tsvn-hook` 被调起，
//!   jkk 开着时它会被单例插件转给已有实例后退出，提交前的拦截就静默失效了。

use std::fs;
use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()))
}

/// `run()` 函数体：从签名到 `.run(` 收尾，只看真正装配 Builder 的那段。
fn run_body(lib: &str) -> &str {
    let start = lib
        .find("pub fn run()")
        .expect("lib.rs 里没有 pub fn run()");
    let body = &lib[start..];
    let end = body
        .find(".run(tauri::generate_context!())")
        .expect("run() 里没有 .run(generate_context!())");
    &body[..end]
}

#[test]
fn single_instance_is_the_first_plugin() {
    let lib = read("src/lib.rs");
    let body = run_body(&lib);
    // setup 闭包里的 `app.handle().plugin(` 也算：全局快捷键、日志都在那里装
    let first_plugin = body.find(".plugin(").expect("run() 里一个插件都没注册");
    assert!(
        body[first_plugin..].starts_with(".plugin(tauri_plugin_single_instance::init"),
        "单例插件必须是第一个注册的插件，现在第一个是: {}",
        body[first_plugin..].lines().next().unwrap()
    );
}

#[test]
fn single_instance_only_in_release_desktop_builds() {
    let lib = read("src/lib.rs");
    let body = run_body(&lib);
    let single = body
        .find("tauri_plugin_single_instance::init")
        .expect("run() 里没有注册单例插件");
    let gate = body[..single]
        .rfind("#[cfg(")
        .expect("单例插件前面没有 #[cfg(...)]");
    let cfg_line = body[gate..].lines().next().unwrap();
    assert_eq!(
        cfg_line.trim(),
        "#[cfg(all(desktop, not(debug_assertions)))]"
    );
}

#[test]
fn second_launch_shows_the_existing_window() {
    let lib = read("src/lib.rs");
    let body = run_body(&lib);
    let single = body.find("tauri_plugin_single_instance::init").unwrap();
    // 只 set_focus 不够：关到托盘后窗口是隐藏的，最小化时也要先还原
    let callback = &body[single..];
    let callback = &callback[..callback.find("}));").expect("单例回调没有按 `}));` 收尾")];
    assert!(
        callback.contains("desktop::show_main_window("),
        "第二次启动要走 show_main_window: {callback}"
    );
}

#[test]
fn tsvn_hook_dispatch_runs_before_tauri() {
    let main = read("src/main.rs");
    let hook = main
        .find("tsvn_hook_main(")
        .expect("main.rs 里没有钩子分流");
    let run = main
        .find("app_lib::run()")
        .expect("main.rs 里没有 app_lib::run()");
    assert!(hook < run, "TortoiseSVN 钩子分流必须在 app_lib::run() 之前");
}
