//! 窗口标题里的版本信息。
//!
//! 同时开着安装版、绿色版、`pnpm tauri dev` 时，标题是唯一能一眼分清
//! 「这是哪个构建」的地方。`JKK_GIT_DESCRIBE` 由 build.rs 在编译期写入。

/// 当前构建的窗口标题。
pub fn window_title() -> String {
    format_title(
        env!("CARGO_PKG_VERSION"),
        option_env!("JKK_GIT_DESCRIBE"),
        cfg!(debug_assertions),
        // release 不显示 checkout 名，也就没必要把构建机路径编进二进制
        if cfg!(debug_assertions) {
            checkout_name(env!("CARGO_MANIFEST_DIR"))
        } else {
            None
        },
    )
}

/// 只有「release 构建、且正好在 `v<version>` 这个 tag 上」才显示裸版本号；
/// 其余情况都把 describe 挂出来，避免开发构建冒充正式版。
///
/// dev 构建再带上 checkout 目录名：几个 worktree 基于同一提交起的 dev 实例
/// describe 完全一样，只有目录名能分开。release 构建不带——构建机上的路径对用户没意义。
pub fn format_title(
    version: &str,
    describe: Option<&str>,
    dev: bool,
    checkout: Option<&str>,
) -> String {
    let mut title = format!("jkk {version}");
    match describe.filter(|d| !d.is_empty()) {
        Some(d) if d == format!("v{version}") => {}
        Some(d) => title.push_str(&format!(" ({d})")),
        None => title.push_str(" (未知提交)"),
    }
    if dev {
        match checkout {
            Some(name) => title.push_str(&format!(" [dev · {name}]")),
            None => title.push_str(" [dev]"),
        }
    }
    title
}

/// `CARGO_MANIFEST_DIR` 是 `<checkout>/src-tauri`，取 `<checkout>` 的目录名。
/// 按字符串切而不用 `Path`：编译期路径可能来自另一种平台的分隔符。
pub fn checkout_name(manifest_dir: &str) -> Option<&str> {
    let mut parts = manifest_dir
        .split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .rev();
    parts.next()?;
    parts.next()
}
