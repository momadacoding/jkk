#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 全局单例的**源码装配**约束。这里只读 `lib.rs` / `main.rs` 的文本，不起进程：
//! 第二次启动唤出窗口、钩子不被转走这些**行为**，要两个真实进程才测得出来，
//! 由 `scripts/verify-single-instance.ps1` 在 release exe 上实测，测试名也只声称装配。
//!
//! 这几条写错都不会编译失败，也不会被 clippy 抓到：
//! - 插件不是第一个注册：官方文档要求它先于其他插件，否则第二个实例可能已经初始化了
//!   托盘、注册了全局快捷键才退出——托盘图标闪一下，快捷键注册失败还会弹设置告警。
//! - debug 构建也注册：多个 worktree 同时跑 dev 时共用 `<identifier>.dev`（见 dev_profile），
//!   后起的会悄悄唤起先起的那个，看到的不是自己的代码。
//! - TortoiseSVN 钩子分流晚于 `run()`：钩子是同一个 exe 带 `--tsvn-hook` 被调起，
//!   jkk 开着时它会被单例插件转给已有实例后退出，提交前的拦截就静默失效了。

use std::fs;
use std::path::Path;

/// 读源码并去掉整行注释：注释里提到 `.plugin(`、`app_lib::run()` 不能算数。
fn code(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    let src =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {}: {e}", path.display()));
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `run()` 函数体：从签名到 `.run(` 收尾，只看真正装配 Builder 的那段。
fn run_body(lib: &str) -> &str {
    let start = lib
        .find("pub fn run()")
        .expect("lib.rs 里没有 pub fn run()");
    let body = &lib[start..];
    // context 先生成、改过 identifier 再交给 run，见 dev_profile
    let end = body
        .find(".run(context)")
        .expect("run() 里没有 .run(context)");
    &body[..end]
}

/// 单例插件的注册语句：从 `let builder = if cfg!(debug_assertions)` 到 `};`。
fn registration(body: &str) -> &str {
    let start = body
        .find("let builder = if cfg!(debug_assertions)")
        .expect("run() 里没有按 cfg!(debug_assertions) 分支注册单例插件");
    let stmt = &body[start..];
    &stmt[..stmt.find("};").expect("注册语句没有按 `};` 收尾") + 2]
}

#[test]
fn lib_registers_single_instance_as_first_plugin() {
    let lib = code("src/lib.rs");
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
fn lib_skips_single_instance_in_debug_builds_only() {
    let lib = code("src/lib.rs");
    let body = run_body(&lib);
    let stmt = registration(body);
    // debug 分支原样返回 builder，release 分支才注册
    let debug_branch = &stmt[stmt.find('{').unwrap() + 1..stmt.find("} else {").unwrap()];
    assert_eq!(debug_branch.trim(), "builder", "debug 分支不能注册任何东西");
    let release_branch = &stmt[stmt.find("} else {").unwrap()..];
    assert!(release_branch.contains("tauri_plugin_single_instance::init"));
    // 只在桌面平台：移动端没有这个插件（Cargo.toml 里按 target 引入）
    let before = body[..body.find(stmt).unwrap()].trim_end();
    assert!(
        before.ends_with("#[cfg(desktop)]"),
        "注册语句前面要有 #[cfg(desktop)]"
    );
}

#[test]
fn lib_single_instance_callback_calls_show_main_window() {
    let lib = code("src/lib.rs");
    let stmt = registration(run_body(&lib));
    // 只 set_focus 不够：关到托盘后窗口是隐藏的，最小化时也要先还原
    assert!(
        stmt.contains("desktop::show_main_window("),
        "第二次启动的回调要走 show_main_window: {stmt}"
    );
}

#[test]
fn main_dispatches_tsvn_hook_before_tauri_run() {
    let main = code("src/main.rs");
    let hook = main
        .find("tsvn_hook_main(")
        .expect("main.rs 里没有钩子分流");
    let run = main
        .find("app_lib::run()")
        .expect("main.rs 里没有 app_lib::run()");
    assert!(hook < run, "TortoiseSVN 钩子分流必须在 app_lib::run() 之前");
}
