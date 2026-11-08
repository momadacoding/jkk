//! debug 构建（`pnpm tauri dev`）用自己的配置目录，不再与安装版共用。
//!
//! Tauri 的配置目录、WebView2 数据目录都按运行期的 `identifier` 解析。dev 原来与安装版
//! 同为 `com.momadacoding.jkk`：dev 一启动就改写安装版的 `session.json`，改设置也直接
//! 落到安装版身上。debug 构建在运行前给 identifier 加上 `.dev`，两边彻底分开。
//!
//! 分开之后 dev 首次启动是一片空白：外部工具（Beyond Compare）、自定义命令、打开过的仓库
//! 都没了。所以 `.dev` 目录还不存在时，从安装版的目录复制一份种子；之后两边各写各的，
//! 安装版的目录只读不写。

use std::fs;
use std::io;
use std::path::Path;
use tauri::{App, Manager, Runtime};

/// 首次启动 dev 时从安装版复制的配置文件，即应用写在 `app_config_dir()` 下的全部文件。
/// 新增配置文件时 `tests/dev_profile.rs` 会提醒同步这里。
pub const SEED_FILES: [&str; 4] = [
    "settings.json",
    "session.json",
    "externalTools.json",
    "customCommands.json",
];

const DEV_SUFFIX: &str = ".dev";

/// 运行期实际使用的 identifier：release 原样，debug 加 `.dev`。
///
/// 已经是 `.dev` 结尾的不再叠加：`pnpm tauri dev --config` 显式给了 dev 用的 identifier 时照用。
/// 给了别的（如 worktree 验证用的 `.sitest`）照样加后缀，仍与安装版、普通 dev 都隔开。
pub fn runtime_identifier(base: &str, debug: bool) -> String {
    if debug && !base.ends_with(DEV_SUFFIX) {
        format!("{base}{DEV_SUFFIX}")
    } else {
        base.to_string()
    }
}

/// `dev_dir` 还不存在时，把 `installed_dir` 里有的种子文件复制过去，返回复制了哪些。
///
/// `dev_dir` 已经存在就什么都不做，哪怕是空的：说明 dev 跑过了，用户可能故意清空了配置，
/// 不能每次启动都拿安装版的盖回来。安装版的目录不存在（新机器没装过）也什么都不做，
/// 连空目录都不建，留给应用按需创建。
pub fn seed_dev_config(installed_dir: &Path, dev_dir: &Path) -> io::Result<Vec<&'static str>> {
    if dev_dir.exists() || !installed_dir.is_dir() {
        return Ok(Vec::new());
    }
    fs::create_dir_all(dev_dir)?;
    let mut copied = Vec::new();
    for name in SEED_FILES {
        let from = installed_dir.join(name);
        if from.is_file() {
            fs::copy(&from, dev_dir.join(name))?;
            copied.push(name);
        }
    }
    Ok(copied)
}

/// 在 `setup` 一开头调用（`desktop::setup` 读 settings.json 之前）：debug 构建首次启动时
/// 从 `installed_identifier` 的配置目录复制种子。release 构建什么都不做。
///
/// 失败只记日志：种子是方便，不是前提，复制不了也就是从空白配置起步。
pub fn seed_on_first_dev_launch<R: Runtime>(app: &App<R>, installed_identifier: &str) {
    if !cfg!(debug_assertions) {
        return;
    }
    let path = app.path();
    let (root, dev_dir) = match (path.config_dir(), path.app_config_dir()) {
        (Ok(root), Ok(dev_dir)) => (root, dev_dir),
        _ => {
            log::warn!("解析不出配置目录，dev 不复制安装版的配置");
            return;
        }
    };
    match seed_dev_config(&root.join(installed_identifier), &dev_dir) {
        Ok(copied) if !copied.is_empty() => {
            log::info!(
                "dev 首次启动，从安装版复制了配置：{copied:?} → {}",
                dev_dir.display()
            );
        }
        Ok(_) => {}
        Err(e) => log::warn!("复制安装版配置到 {} 失败：{e}", dev_dir.display()),
    }
}
