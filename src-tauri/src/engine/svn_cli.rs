//! svn 子进程的唯一入口。形状对齐 `engine::git_cli`。

use crate::engine::perf;
use crate::engine::proc::{self, hidden_command};
use crate::error::{GitError, Result};
use std::cell::RefCell;
use std::collections::HashMap;
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
    let key = scan_key(path, args);
    if let Some(hit) = key.as_deref().and_then(cached_scan) {
        if perf::enabled() {
            perf::record_cached(svn_command(path).args(with_global(args)));
        }
        return Ok(hit);
    }
    let output = proc::output(svn_command(path).args(with_global(args)))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("svn_cli", stderr));
    }
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    if let Some(k) = key {
        remember_scan(k, &out);
    }
    Ok(out)
}

/// 一条多目标命令的命令行上限（字符数按 UTF-8 字节估，偏保守）。
///
/// Windows 的命令行上限是 32767 个 UTF-16 单元，留出余量给 svn 路径与全局参数。
/// 与 `svn_health::MAX_DIFF_CMDLINE` 同一口径。
pub const MAX_TARGETS_CMDLINE: usize = 24000;

/// 命令行长度的保守估计：每个参数按字节数 + 两个引号 + 一个空格。
pub fn cmdline_estimate(head: &[&str], targets: &[String]) -> usize {
    "svn --non-interactive".len()
        + head.iter().map(|a| a.len() + 3).sum::<usize>()
        + targets.iter().map(|a| a.len() + 3).sum::<usize>()
}

/// 把 `targets` 按顺序切成若干批，每批接在 `head` 后面不超过 `max`。
///
/// 空列表返回空：svn 不带 target 时作用于当前目录，也就是整个工作副本。单个 target
/// 就超过上限时自成一批，交给 svn 去报错，不静默丢掉。
pub fn chunk_targets<'a>(head: &[&str], targets: &'a [String], max: usize) -> Vec<&'a [String]> {
    let base = cmdline_estimate(head, &[]);
    let mut out = Vec::new();
    let mut start = 0;
    let mut len = base;
    for (i, t) in targets.iter().enumerate() {
        let add = t.len() + 3;
        if i > start && len + add > max {
            out.push(&targets[start..i]);
            start = i;
            len = base;
        }
        len += add;
    }
    if start < targets.len() {
        out.push(&targets[start..]);
    }
    out
}

/// 同一条命令一次带多个 target 跑，按命令行长度分批，每批一份输出（按批的顺序）。
///
/// # 为什么
///
/// 跟进与后台检查原来每个白名单目录起一个 svn 进程。公司机器上起一个进程（打开 117 MB
/// 的 wc.db、过四层文件过滤驱动）就要几十到上百毫秒，23 个目录一次传：`info` 1251 → 82 ms、
/// `status` 8126 → 4139 ms（2026-11-07 报告 3.4 节）。
///
/// # 与逐个跑的差别
///
/// - 输出是逐个 target 结果的拼接：`status` 每个 target 一个 `<target>` 块，`info` 每个
///   target 一条 entry、按传入顺序，`ls` 每个 target 一个 `<list>`（计划 2026-11-09 实测
///   M1–M12）。调用方要按块或按顺序对回 target。
/// - 任何一个 target 失败整批报错，与逐个跑时第一个失败就 `?` 相同。
/// - `targets` 为空时不起进程，返回空列表（`head` 末尾的 `--` 之后什么都没有时，svn 会
///   拿当前目录当 target）。
///
/// `head` 必须以 `--` 结尾；target 要不要过 `svn_wc::peg_safe` 由调用方决定。
pub fn run_svn_targets(path: &str, head: &[&str], targets: &[String]) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for batch in chunk_targets(head, targets, MAX_TARGETS_CMDLINE) {
        let mut args: Vec<&str> = head.to_vec();
        args.extend(batch.iter().map(String::as_str));
        out.push(run_svn(path, &args)?);
    }
    Ok(out)
}

/// 白名单的全量扫描：找嵌套 external（`svn_wc::enumerate_wcs`）、读节点 revision
/// （`svn_nodes::read_node_status`）、清点脏条目（`svn_dirty::read_dirty`）**共用这一条命令**，
/// target 接在后面。
///
/// # 为什么三处必须逐字相同
///
/// [`with_scan_cache`] 按「工作目录 + 全部参数」认同一条命令。原来三处各带各的参数
/// （`status`、`status -v --ignore-externals`、`status -v --no-ignore --ignore-externals`），同一片
/// 白名单每个范围里要遍历三遍；一遍在公司机器上约 4 秒（2026-11-07 报告 3.4 节），一次自动跟进
/// 连同前后的检测一共十遍。参数取三者的并集之后，白名单底下没有嵌套 external 时三处发的是同一条
/// 命令，一个范围里只遍历一遍（`tests/svn_follow_scan_budget.rs` 钉住遍数）。有嵌套 external 时
/// 枚举按层发、另两处一次传全部 target，命令不同，多遍历一遍，不比原来多。
///
/// 找嵌套 external 只在范围里才用它；不在范围里时没有人共用，照旧发它自己那条输出很小的命令
/// （`svn_wc::enumerate_wcs`）。
///
/// # 为什么并集对三处都够用
///
/// | 参数 | 谁要它 | 对另外两处的影响 |
/// |---|---|---|
/// | `-v` | 节点 revision（不带它干净节点不输出）、脏条目（被 `svn delete` 的目录的子文件） | 找 external 只看 `item="external"`，多出来的条目不看 |
/// | `--no-ignore` | 脏条目（SVN-ignored 而 git 照收的文件） | 只多出 `item="ignored"` 的条目；节点 revision 那边读完就丢掉它们，结果与原来逐条相同 |
/// | `--ignore-externals` | 节点 revision、脏条目（external 内部是另一个仓库的 revision 序列） | external 根那条 `item="external"` 带不带它都出现（`svn_xml::assert_externals_were_ignored` 的实测表）；更深一层的由枚举下一层去找 |
///
/// 以 `--` 结尾，是 `run_svn_targets` 的 `head`。target 要过 `svn_wc::peg_safe`。
pub const STATUS_SCAN: &[&str] = &[
    "status",
    "-v",
    "--no-ignore",
    "--ignore-externals",
    "--xml",
    "--",
];

thread_local! {
    /// [`with_scan_cache`] 范围内的 `svn info` / `svn status` 输出。`None` = 不在范围内。
    static SCANS: RefCell<Option<HashMap<String, String>>> = const { RefCell::new(None) };
}

/// 在 `f` 执行期间，同一条 `svn info` / `svn status`（同一工作目录、同样的参数）只跑一次。
///
/// # 为什么
///
/// 跟进的准备阶段里，失同步判定、R1 前置校验、R3b 体检各自调 `enumerate_wcs` 与
/// `read_node_status`，同一片白名单被 `svn status` 全量扫了约 10 遍。2026-11-01 实测一万五千个
/// 文件每遍约 0.2 秒（开发机）；公司机器上 git 同类操作约慢 7～10 倍（stage-refresh-perf 日志）。
///
/// # 为什么是安全的
///
/// - 只缓存只读的 `info` / `status`，范围由调用方划定（跟进只包住准备阶段），出了范围就丢。
/// - 范围内不发生 SVN 写操作：跟进全程不写工作区、不发 SVN 写命令。外部（TortoiseSVN）在范围内
///   的改动由两道防线兜住：体检直接问服务器；发布前的锁内复核（跟进的 R3c，
///   `svn_follow::still_valid`）不在范围内，照常读最新的。别的录制入口在范围里做 R3c 时先
///   [`forget_node_scans`]。
/// - 只按线程缓存：Tauri 命令在线程池里跑，一次跟进从头到尾在同一个线程上。
///
/// 嵌套调用沿用外层的缓存，不重置。
pub fn with_scan_cache<T>(f: impl FnOnce() -> T) -> T {
    /// 范围结束（含 panic 展开）时把缓存拿掉。
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            SCANS.with(|s| s.borrow_mut().take());
        }
    }
    if SCANS.with(|s| s.borrow().is_some()) {
        return f();
    }
    SCANS.with(|s| *s.borrow_mut() = Some(HashMap::new()));
    let _scope = Scope;
    f()
}

/// 当前线程是不是在 [`with_scan_cache`] 的范围里。
pub fn in_scan_cache() -> bool {
    SCANS.with(|s| s.borrow().is_some())
}

/// 在范围里临时不用扫描缓存：`f` 里的 `info` / `status` 都去读最新的，也不往缓存里记。
/// 结束（含 panic 展开）时把原来的缓存放回去。
///
/// 给「专门用来发现期间有没有变化」的重读用（`svn_base_blobs` 写缓存前的复核）：在缓存里它会
/// 拿到第一次读的结果，等于没查（独立 review 2026-11-14）。
pub fn bypass_scan_cache<T>(f: impl FnOnce() -> T) -> T {
    type Scans = Option<HashMap<String, String>>;
    struct Restore(Scans);
    impl Drop for Restore {
        fn drop(&mut self) {
            let saved = self.0.take();
            SCANS.with(|s| *s.borrow_mut() = saved);
        }
    }
    let _restore = Restore(SCANS.with(|s| s.borrow_mut().take()));
    f()
}

/// 忘掉缓存里的节点状态扫描（`status -v`，即 [`STATUS_SCAN`]），下一次照常去读。R3c 用：它存在的
/// 意义就是发现录制期间工作副本被外部改了，读缓存等于没查。`info` 留着——那是工作副本的布局，
/// 不是 revision。找 external 用的也是同一条扫描，跟着一起重读。不在范围内时什么都不做。
pub fn forget_node_scans() {
    SCANS.with(|s| {
        if let Some(m) = s.borrow_mut().as_mut() {
            m.retain(|k, _| !k.split('\0').any(|a| a == "-v"));
        }
    });
}

/// 可以缓存的扫描的键；其余命令返回 `None`。
fn scan_key(path: &str, args: &[&str]) -> Option<String> {
    if !matches!(args.first(), Some(&"info") | Some(&"status")) {
        return None;
    }
    if SCANS.with(|s| s.borrow().is_none()) {
        return None;
    }
    let mut k = path.to_string();
    for a in args {
        k.push('\0');
        k.push_str(a);
    }
    Some(k)
}

/// 在后台线程里提前跑的一条多目标扫描，见 [`prefetch_scan`]。
pub struct ScanAhead {
    /// 每一批的缓存键，与 [`run_svn_targets`] 分批后逐条 [`run_svn`] 用的键相同。
    keys: Vec<String>,
    handle: Option<std::thread::JoinHandle<Result<Vec<String>>>>,
}

impl ScanAhead {
    /// 等后台那条命令跑完，把输出记进**当前线程**的扫描缓存。之后同一条命令（[`run_svn_targets`]
    /// 带同样的 `head` 与 `targets`）直接拿到这份输出。
    ///
    /// 后台那条失败时什么都不记：调用方随后照常自己跑一遍，错误从那里报出来，与没有预跑时相同。
    pub fn adopt(mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        match handle.join() {
            Ok(Ok(outputs)) if outputs.len() == self.keys.len() => {
                for (key, out) in std::mem::take(&mut self.keys).into_iter().zip(outputs) {
                    store_scan(key, out);
                }
            }
            Ok(Ok(_)) => log::warn!("预跑的 svn 扫描批数对不上，不用它的结果"),
            Ok(Err(e)) => log::info!("预跑的 svn 扫描失败，稍后照常重跑：{}", e.message),
            Err(_) => log::warn!("预跑 svn 扫描的线程 panic 了，稍后照常重跑"),
        }
    }
}

impl Drop for ScanAhead {
    /// 没被 [`ScanAhead::adopt`] 的（调用方中途报错返回）也等它跑完：不留一个还在读工作副本的
    /// svn 进程在调用方返回之后继续跑（测试的临时目录这时正要删）。
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// 在后台线程里提前跑 `run_svn_targets(path, head, targets)`，让它与调用方接下来的工作并行。
/// 返回的 [`ScanAhead`] 在真正要用结果之前 [`ScanAhead::adopt`] 一下。
///
/// 只对 [`with_scan_cache`] 范围内、可以缓存的只读扫描（`info` / `status`）有意义，其余情况
/// 返回 `None`、不起线程：
///
/// - 不在范围内：结果没地方放；
/// - 每一批都已经在缓存里：没有可省的；
/// - `targets` 为空：[`run_svn_targets`] 这时一个进程都不起。
///
/// # 为什么是安全的
///
/// 与 [`with_scan_cache`] 相同：只读命令，结果只在调用方划定的范围里用。提前跑只是把同一条
/// 命令的执行时刻往前挪了一点，仍在范围之内；范围内本来就约定同一条扫描只读一次。svn 的只读
/// 命令可以并发（录制里取服务器内容早就是多个 `svn cat` 并行）。
pub fn prefetch_scan(path: &str, head: &[&str], targets: &[String]) -> Option<ScanAhead> {
    let mut keys = Vec::new();
    for batch in chunk_targets(head, targets, MAX_TARGETS_CMDLINE) {
        let mut args: Vec<&str> = head.to_vec();
        args.extend(batch.iter().map(String::as_str));
        keys.push(scan_key(path, &args)?);
    }
    if keys.is_empty() || keys.iter().all(|k| scan_is_cached(k)) {
        return None;
    }
    let (path, targets) = (path.to_string(), targets.to_vec());
    let head: Vec<String> = head.iter().map(|a| a.to_string()).collect();
    let handle = std::thread::Builder::new()
        .name("svn-scan-ahead".to_string())
        .spawn(move || {
            let head: Vec<&str> = head.iter().map(String::as_str).collect();
            // 新线程不在任何扫描缓存范围里，这里一定真的起进程
            run_svn_targets(&path, &head, &targets)
        })
        .ok()?;
    Some(ScanAhead {
        keys,
        handle: Some(handle),
    })
}

fn cached_scan(key: &str) -> Option<String> {
    SCANS.with(|s| s.borrow().as_ref().and_then(|m| m.get(key).cloned()))
}

fn remember_scan(key: String, out: &str) {
    store_scan(key, out.to_string());
}

/// 同 [`remember_scan`]，调用方已经拥有这份输出时用：全量扫描的输出有十几 MB，不多拷一份。
fn store_scan(key: String, out: String) {
    SCANS.with(|s| {
        if let Some(m) = s.borrow_mut().as_mut() {
            m.insert(key, out);
        }
    });
}

/// 缓存里有没有这条扫描。只问有没有，不像 [`cached_scan`] 那样把输出拷出来。
fn scan_is_cached(key: &str) -> bool {
    SCANS.with(|s| s.borrow().as_ref().is_some_and(|m| m.contains_key(key)))
}

/// 同 [`run_svn`]，但把 stdout 按**原始字节**返回，且把退出码和 stderr 一起给出去。
///
/// # 为什么不能复用 `run_svn`
///
/// [`run_svn`] 走 `String::from_utf8_lossy`。那对 `--xml` 输出是对的（svn 的 XML
/// 声明是 UTF-8），但对 `svn cat` 是**破坏性**的：源码文件很可能不是 UTF-8——国内
/// Unity 工程里 GBK 的 `.cs` 很常见——lossy 会把每一个非法字节换成 U+FFFD，
/// 而那棵被悄悄改坏的树录进基线之后，**基线体检查不出来**（体检比的是磁盘与
/// 服务器，不比基线树，spec §6.8.3）。
///
/// 返回 `SvnBytes` 而不是 `Result<Vec<u8>>`：`svn cat` 的失败里有一种
/// （`E195002`，本地 add 了没提交）是**正常分类结果**而非错误，调用方必须看得到
/// stderr 才能分辨，见 [`crate::engine::svn_pristine`]。
pub fn run_svn_bytes(path: &str, args: &[&str]) -> Result<SvnBytes> {
    let output = proc::output(svn_command(path).args(with_global(args)))?;
    Ok(SvnBytes {
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// [`run_svn_bytes`] 的结果。stdout 是字节，stderr 仍按字符串给——诊断信息是
/// svn 自己产的、一定是 UTF-8，而且要拿去匹配错误码。
#[derive(Debug, Clone)]
pub struct SvnBytes {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
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
    let output = proc::output(svn_command(path).args(with_global(args)))?;
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
    if let Ok(out) = proc::output(hidden_command("svn").args(["--version", "--quiet"])) {
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
        let Ok(out) = proc::output(hidden_command("reg").args(["query", key, "/v", "Path"])) else {
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
