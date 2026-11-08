//! `pnpm tauri dev` 用自己的配置目录，不再与安装版共用。
//!
//! Tauri 的配置目录、WebView2 数据目录都按运行期的 `identifier` 解析。dev 原来与安装版
//! 同为 `com.momadacoding.jkk`：dev 一启动就改写安装版的 `session.json`，改设置也直接
//! 落到安装版身上。dev 在运行前给 identifier 加上 `.dev`，两边彻底分开。
//!
//! 「dev」按 `tauri::is_dev()` 认，不按 `debug_assertions`：`tauri dev --release` 仍是 dev，
//! `tauri build --debug` 打出来的安装包仍是安装版。
//!
//! 分开之后，dev 首次启动时从安装版复制外部工具、自定义命令两份配置作种子——配一次
//! Beyond Compare 不该还得在 dev 里再配一次。会话和设置**不**复制，见 [`NOT_SEEDED`]。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use tauri::plugin::TauriPlugin;
use tauri::{App, AppHandle, Manager, Runtime};

/// 安装版的 identifier，与 `tauri.conf.json` 一致（测试守着）。
pub const INSTALLED_IDENTIFIER: &str = "com.momadacoding.jkk";

/// dev 首次启动时从安装版复制的配置文件。
pub const SEED_FILES: [&str; 2] = ["externalTools.json", "customCommands.json"];

/// 同在 `app_config_dir()` 下、故意不复制的配置文件：
/// - `session.json`：带过去 dev 一启动就打开安装版里的真实仓库，dev 构建的缺陷会直接
///   作用在用户的工作仓库上（AGENTS.md 开头那条：禁止把真实仓库当试验场）。
/// - `settings.json`：里面有全局快捷键，带过去 dev 与安装版抢同一个系统级快捷键，
///   后起的那个永远注册失败、弹告警。
pub const NOT_SEEDED: [&str; 2] = ["session.json", "settings.json"];

const DEV_SUFFIX: &str = ".dev";

/// 运行期实际使用的 identifier。
///
/// 只有 dev 且 identifier 是安装版那个时才加 `.dev`。`pnpm tauri dev --config` 显式换了
/// identifier（worktree 验证用的 `.sitest` 之类）就原样使用、也不复制种子：调用方已经自己
/// 隔离好了，事先写进那个目录的 `session.json` 必须照读。
pub fn runtime_identifier(configured: &str, dev: bool) -> String {
    if dev && configured == INSTALLED_IDENTIFIER {
        format!("{configured}{DEV_SUFFIX}")
    } else {
        configured.to_string()
    }
}

/// `dev_dir` 还不存在时，把 `installed_dir` 里有的种子文件复制过去，返回复制了哪些。
///
/// - `dev_dir` 已经存在就什么都不做，哪怕是空的：dev 跑过了，用户可能故意清空了配置。
///   这个判断可靠的前提是调用得足够早，见 [`plugin`]。
/// - 安装版里一个种子都没有（新机器没装过）也什么都不做，连空目录都不建。
/// - 要么全到、要么没有：先复制到旁边的 `<dev_dir>.seeding`，齐了再整个改名。复制到一半
///   失败（杀毒软件占着、安装版正在原子写）不会留下半份配置——留下的话下次启动看到目录
///   已存在，就再也不补了。
pub fn seed_dev_config(installed_dir: &Path, dev_dir: &Path) -> io::Result<Vec<&'static str>> {
    if dev_dir.exists() {
        return Ok(Vec::new());
    }
    let present: Vec<&'static str> = SEED_FILES
        .into_iter()
        .filter(|name| installed_dir.join(name).is_file())
        .collect();
    if present.is_empty() {
        return Ok(Vec::new());
    }
    let staging = staging_dir(dev_dir)?;
    // 上次复制到一半进程没了，留下的半成品先清掉
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    let copy_all = || -> io::Result<()> {
        fs::create_dir_all(&staging)?;
        for name in &present {
            fs::copy(installed_dir.join(name), staging.join(name))?;
        }
        fs::rename(&staging, dev_dir)
    };
    if let Err(e) = copy_all() {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    Ok(present)
}

fn staging_dir(dev_dir: &Path) -> io::Result<PathBuf> {
    let name = dev_dir.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("配置目录没有末级目录名：{}", dev_dir.display()),
        )
    })?;
    let mut staging = name.to_os_string();
    staging.push(".seeding");
    Ok(dev_dir.with_file_name(staging))
}

/// 种子复制的结果，等日志插件装好之后由 [`log_seed_outcome`] 记下来。
struct SeedOutcome(Result<Vec<&'static str>, String>);

/// 复制种子的插件。只在 `runtime_identifier` 真的加了后缀时注册。
///
/// 做成插件而不是放进 `setup` 钩子：Tauri 先初始化插件，事件循环 Ready 之后才按配置建
/// 窗口、再调 `setup` 钩子。放在钩子里时 webview 已经在加载前端，前端的
/// `read_external_tools` 可能抢在复制完成之前读到空配置；那时也可能已经有代码建出了
/// 配置目录，让「目录存在 = dev 跑过」的判断失效。插件这里两件事都还没发生。
/// 路径用 Tauri 自己的解析器，与应用读写配置的位置一致。
pub fn plugin<R: Runtime>(installed_identifier: String) -> TauriPlugin<R> {
    tauri::plugin::Builder::new("dev-profile")
        .setup(move |app, _api| {
            let outcome = seed_from_installed(app, &installed_identifier);
            app.manage(SeedOutcome(outcome));
            Ok(())
        })
        .build()
}

fn seed_from_installed<R: Runtime>(
    app: &AppHandle<R>,
    installed_identifier: &str,
) -> Result<Vec<&'static str>, String> {
    let path = app.path();
    let root = path
        .config_dir()
        .map_err(|e| format!("解析不出配置根目录：{e}"))?;
    let dev_dir = path
        .app_config_dir()
        .map_err(|e| format!("解析不出 dev 配置目录：{e}"))?;
    seed_dev_config(&root.join(installed_identifier), &dev_dir)
        .map_err(|e| format!("复制安装版配置到 {} 失败：{e}", dev_dir.display()))
}

/// 在 `setup` 钩子里、日志插件装好之后调用。没注册种子插件时什么都不记。
///
/// 失败只记日志：种子是方便，不是前提，复制不了也就是从空白配置起步。
pub fn log_seed_outcome<R: Runtime>(app: &App<R>) {
    let Some(outcome) = app.try_state::<SeedOutcome>() else {
        return;
    };
    match &outcome.0 {
        Ok(copied) if !copied.is_empty() => {
            log::info!("dev 首次启动，从安装版复制了配置：{copied:?}");
        }
        Ok(_) => {}
        Err(e) => log::warn!("{e}"),
    }
}
