//! 起 git 子进程时，绕开 Git for Windows 的 `cmd\git.exe` 启动器，直接起真正的 `git.exe`。
//!
//! # 为什么
//!
//! PATH 上的 `git` 通常是 `C:\Program Files\Git\cmd\git.exe`。它只是个启动器：设好几个环境变量，
//! 再起 `mingw64\bin\git.exe`，等它退出。于是每条 git 命令是**两次**进程创建。2026-11-14 实测
//! 开发机上 `rev-parse` 一条 40 ms → 30 ms（日志 `2026-11-14-follow-overlap-decisions.md` T3）；
//! 一次自动跟进起四五十个 git 进程。公司机器上进程创建要过安全软件，每次几十到上百毫秒，
//! 省掉一半的创建更值。
//!
//! # 怎样才算等价
//!
//! 启动器做的只有两件与子进程有关的事（同一日志里的实测，子进程 `env | sort` 逐行对比）：
//!
//! - PATH 前面加上 `<top>\<prefix>\bin;<top>\usr\bin;<HOME>\bin;`
//! - `MSYSTEM` 设成 `<PREFIX>`（如 `MINGW64`）
//!
//! `HOME`、`PLINK_PROTOCOL` 由真正的 `git.exe` 自己补，与经过启动器时相同。真正的 `git.exe` 在
//! `MSYSTEM` 没设时会自己往 PATH 前面加 `~/bin`、`mingw64/bin`、`usr/bin`，顺序与启动器不同；
//! 设了 `MSYSTEM` 就不加。所以照启动器的样子把两样都设好，子进程看到的环境与原来逐行相同。
//!
//! # 只在认得出来时才绕
//!
//! 下面任何一条不满足就照旧起 `git`（经过启动器或别的什么），行为与改动前相同：
//!
//! - PATH 上第一个 `git.exe` 在名为 `cmd` 的目录里（Git for Windows 的启动器）；
//! - `git --exec-path` 指向同一个安装目录下的 `<prefix>\libexec\git-core`，`<prefix>` 是认得的
//!   （`mingw64` / `mingw32` / `clangarm64`），`<prefix>\bin\git.exe` 与 `usr\bin` 都在；
//! - 起进程的那一刻父进程环境里没有 `MSYSTEM`（从 Git Bash 里启动本应用时有，启动器那时怎么
//!   处理没有实测，不冒险）。

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// 认得出来的 Git for Windows 安装：直接起它的 `git.exe` 要的东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Direct {
    /// `<top>\<prefix>\bin\git.exe`
    pub exe: PathBuf,
    /// `<top>\<prefix>\bin`
    pub prefix_bin: PathBuf,
    /// `<top>\usr\bin`
    pub usr_bin: PathBuf,
    /// `MINGW64` 之类
    pub msystem: String,
}

const PREFIXES: [&str; 3] = ["mingw64", "mingw32", "clangarm64"];

/// 由 PATH 与 `git --exec-path` 的输出判断能不能直接起。纯函数：文件系统由参数给。
///
/// - `path_var`：父进程的 PATH
/// - `exec_path`：经过启动器跑出来的 `git --exec-path`（已去掉首尾空白）
pub fn plan_direct(
    path_var: &OsStr,
    exec_path: &str,
    is_file: impl Fn(&Path) -> bool,
    is_dir: impl Fn(&Path) -> bool,
) -> Option<Direct> {
    // Rust 的 `Command::new("git")` 按 PATH 顺序找 `git.exe`；第一个就是现在实际起的那个
    let launcher_dir = std::env::split_paths(path_var)
        .filter(|d| !d.as_os_str().is_empty())
        .find(|d| is_file(&d.join("git.exe")))?;
    if !launcher_dir
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("cmd")
    {
        return None;
    }
    let top = launcher_dir.parent()?;

    let git_core = PathBuf::from(exec_path);
    let libexec = git_core.parent()?;
    let prefix_dir = libexec.parent()?;
    if !git_core
        .file_name()?
        .to_string_lossy()
        .eq_ignore_ascii_case("git-core")
        || !libexec
            .file_name()?
            .to_string_lossy()
            .eq_ignore_ascii_case("libexec")
    {
        return None;
    }
    let prefix = prefix_dir.file_name()?.to_string_lossy().to_string();
    if !PREFIXES.iter().any(|p| p.eq_ignore_ascii_case(&prefix)) {
        return None;
    }
    // exec-path 用正斜杠（`C:/Program Files/Git/...`），PATH 里是反斜杠：按规范化后的文本比
    if !same_dir(prefix_dir.parent()?, top) {
        return None;
    }

    let prefix_bin = top.join(&prefix).join("bin");
    let exe = prefix_bin.join("git.exe");
    let usr_bin = top.join("usr").join("bin");
    if !is_file(&exe) || !is_dir(&usr_bin) {
        return None;
    }
    Some(Direct {
        exe,
        prefix_bin,
        usr_bin,
        msystem: prefix.to_ascii_uppercase(),
    })
}

/// 两个目录是不是同一个：斜杠方向、大小写、末尾的分隔符都不算差别（Windows 路径）。
fn same_dir(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .replace('/', "\\")
            .trim_end_matches('\\')
            .to_ascii_lowercase()
    };
    norm(a) == norm(b)
}

/// 启动器给子进程的 PATH：`<prefix>\bin;<usr>\bin;<home>\bin;<原 PATH>`。没有 home 时不加那一段。
pub fn launcher_path(d: &Direct, home: Option<&Path>, parent_path: Option<&OsStr>) -> OsString {
    let mut out = OsString::new();
    out.push(d.prefix_bin.as_os_str());
    out.push(";");
    out.push(d.usr_bin.as_os_str());
    out.push(";");
    if let Some(h) = home {
        out.push(h.join("bin").as_os_str());
        out.push(";");
    }
    if let Some(p) = parent_path {
        out.push(p);
    }
    out
}

/// 启动器认的 HOME：`HOME`；没有就 `HOMEDRIVE` + `HOMEPATH`（目录存在时）；再没有就 `USERPROFILE`。
pub fn launcher_home(
    var: impl Fn(&str) -> Option<OsString>,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Some(h) = var("HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(h));
    }
    if let (Some(drive), Some(path)) = (var("HOMEDRIVE"), var("HOMEPATH")) {
        let mut joined = drive;
        joined.push(path);
        let p = PathBuf::from(joined);
        if !p.as_os_str().is_empty() && exists(&p) {
            return Some(p);
        }
    }
    var("USERPROFILE")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXEC: &str = "C:/Program Files/Git/mingw64/libexec/git-core";

    fn files(list: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| {
            let s = p.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
            list.iter().any(|f| f.to_ascii_lowercase() == s)
        }
    }

    const INSTALL: &[&str] = &[
        r"C:\Program Files\Git\cmd\git.exe",
        r"C:\Program Files\Git\mingw64\bin\git.exe",
    ];

    fn usr_bin_exists(p: &Path) -> bool {
        p.to_string_lossy().to_ascii_lowercase() == r"c:\program files\git\usr\bin"
    }

    #[test]
    fn the_git_for_windows_launcher_is_bypassed() {
        let path = OsString::from(r"C:\Windows\system32;C:\Program Files\Git\cmd;C:\tools");
        let d = plan_direct(&path, EXEC, files(INSTALL), usr_bin_exists).expect("应当认得出来");
        assert_eq!(
            d.exe,
            PathBuf::from(r"C:\Program Files\Git\mingw64\bin\git.exe")
        );
        assert_eq!(
            d.prefix_bin,
            PathBuf::from(r"C:\Program Files\Git\mingw64\bin")
        );
        assert_eq!(d.usr_bin, PathBuf::from(r"C:\Program Files\Git\usr\bin"));
        assert_eq!(d.msystem, "MINGW64");
    }

    #[test]
    fn anything_that_is_not_the_launcher_is_left_alone() {
        // PATH 上第一个 git 已经是真正的 git.exe：现在就没有启动器，不能平白多设环境
        let path = OsString::from(r"C:\Program Files\Git\mingw64\bin;C:\Program Files\Git\cmd");
        assert_eq!(
            plan_direct(&path, EXEC, files(INSTALL), usr_bin_exists),
            None
        );
        // 找不到 git
        assert_eq!(
            plan_direct(
                &OsString::from(r"C:\tools"),
                EXEC,
                files(INSTALL),
                usr_bin_exists
            ),
            None
        );
        // 空 PATH、空条目
        assert_eq!(
            plan_direct(&OsString::new(), EXEC, files(INSTALL), usr_bin_exists),
            None
        );
    }

    #[test]
    fn the_exec_path_must_belong_to_the_same_install() {
        let path = OsString::from(r"C:\Program Files\Git\cmd");
        // 别的安装（scoop 之类）的 exec-path
        assert_eq!(
            plan_direct(
                &path,
                "C:/scoop/apps/git/current/mingw64/libexec/git-core",
                files(INSTALL),
                usr_bin_exists
            ),
            None
        );
        // Cygwin 那种 POSIX 路径
        assert_eq!(
            plan_direct(
                &path,
                "/usr/libexec/git-core",
                files(INSTALL),
                usr_bin_exists
            ),
            None
        );
        // 认不得的前缀
        assert_eq!(
            plan_direct(
                &path,
                "C:/Program Files/Git/ucrt64/libexec/git-core",
                files(INSTALL),
                usr_bin_exists
            ),
            None
        );
        // 大小写、斜杠方向不同仍是同一个安装
        let d = plan_direct(
            &OsString::from(r"c:\program files\git\CMD\"),
            "C:/Program Files/Git/MINGW64/libexec/git-core",
            files(INSTALL),
            usr_bin_exists,
        );
        assert!(d.is_some(), "大小写、末尾分隔符不该影响判断");
    }

    #[test]
    fn a_missing_real_git_or_usr_bin_keeps_the_launcher() {
        let path = OsString::from(r"C:\Program Files\Git\cmd");
        assert_eq!(
            plan_direct(
                &path,
                EXEC,
                files(&[r"C:\Program Files\Git\cmd\git.exe"]),
                usr_bin_exists
            ),
            None
        );
        assert_eq!(plan_direct(&path, EXEC, files(INSTALL), |_| false), None);
    }

    #[test]
    fn the_launcher_path_puts_the_install_and_home_first() {
        let d = Direct {
            exe: PathBuf::from(r"C:\G\mingw64\bin\git.exe"),
            prefix_bin: PathBuf::from(r"C:\G\mingw64\bin"),
            usr_bin: PathBuf::from(r"C:\G\usr\bin"),
            msystem: "MINGW64".to_string(),
        };
        let p = launcher_path(
            &d,
            Some(Path::new(r"C:\Users\me")),
            Some(OsStr::new(r"C:\x;C:\y")),
        );
        assert_eq!(
            p,
            OsString::from(r"C:\G\mingw64\bin;C:\G\usr\bin;C:\Users\me\bin;C:\x;C:\y")
        );
        let p = launcher_path(&d, None, None);
        assert_eq!(p, OsString::from(r"C:\G\mingw64\bin;C:\G\usr\bin;"));
    }

    #[test]
    fn home_follows_the_launcher_order() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| OsString::from(v))
            }
        };
        let all = &[
            ("HOME", r"D:\h"),
            ("HOMEDRIVE", "C:"),
            ("HOMEPATH", r"\Users\me"),
            ("USERPROFILE", r"C:\Users\local"),
        ];
        assert_eq!(
            launcher_home(env(all), |_| true),
            Some(PathBuf::from(r"D:\h"))
        );
        let no_home = &[
            ("HOMEDRIVE", "C:"),
            ("HOMEPATH", r"\Users\me"),
            ("USERPROFILE", r"C:\Users\local"),
        ];
        assert_eq!(
            launcher_home(env(no_home), |_| true),
            Some(PathBuf::from(r"C:\Users\me"))
        );
        // HOMEDRIVE + HOMEPATH 不存在（漫游配置掉线）时退到 USERPROFILE
        assert_eq!(
            launcher_home(env(no_home), |_| false),
            Some(PathBuf::from(r"C:\Users\local"))
        );
        assert_eq!(launcher_home(env(&[]), |_| true), None);
    }
}
