//! 查出「是谁占着这个目录」。
//!
//! 删 worktree 靠一次整目录 rename。只要里面有**任何一个**文件或目录被别的进程
//! 握着，整个 rename 就失败，而系统只会甩一句 os error 32，不说是哪个文件、更不说
//! 是哪个进程。这个模块负责把那句话翻译成能动手的信息。
//!
//! # Windows 上有两种占用，难度差很远
//!
//! | 占用方式 | 例子 | 查得到进程吗 |
//! |---|---|---|
//! | 打开了里面的**文件** | 编辑器开着某个源文件 | 能，Restart Manager 直接给进程名和 PID |
//! | 把目录当**工作目录**（cwd） | 一个终端 `cd` 在里面 | 查不到进程。RM 只认文件，枚举进程 cwd 要走 `NtQueryInformationProcess` |
//!
//! 所以这里分两步：先用「独占打开」探测出**是哪个路径**被占（两种都能测出来），
//! 再对那个路径问一次 RM 要进程（只有第一种能拿到）。拿不到进程也是有用的结论——
//! 那基本就是有程序的工作目录停在里面。

use std::path::{Path, PathBuf};

/// 占着某个路径的进程。
#[derive(Debug, Clone)]
pub struct BlockingProcess {
    pub name: String,
    pub pid: u32,
}

/// 一个占用点：哪个路径被占了，以及（如果查得到）被谁占着。
#[derive(Debug, Clone)]
pub struct Blocker {
    pub path: PathBuf,
    /// 空表示查不出进程——通常意味着这是个「工作目录停在里面」的占用。
    pub processes: Vec<BlockingProcess>,
}

/// 遍历时最多探测多少个条目。
///
/// 装了依赖的 worktree 有十几万个文件，全测一遍太慢。真占用通常几百个条目内就撞上，
/// 探不到就退回泛泛的提示，总比让界面卡十几秒强。
const MAX_PROBES: usize = 20_000;

/// 找出 `root` 底下被占用的路径，最多报 `limit` 个。
///
/// 探测是**只读**的：只尝试独占打开，不移动、不修改任何东西。
pub fn find_blockers(root: &Path, limit: usize) -> Vec<Blocker> {
    let mut found = Vec::new();
    let mut probes = 0usize;
    // 先测根自己：整个 worktree 目录被当成 cwd 是最常见的情形
    probe_into(root, limit, &mut probes, &mut found);
    found
}

fn probe_into(path: &Path, limit: usize, probes: &mut usize, found: &mut Vec<Blocker>) {
    if found.len() >= limit || *probes >= MAX_PROBES {
        return;
    }
    *probes += 1;
    if is_occupied(path) {
        // 复检一次再下结论。杀毒软件和索引服务会在文件刚落盘时短暂握住它所在的
        // 目录，只测一次会把这种一闪而过的东西报成占用——指着一条无辜的路径让人
        // 去找，比不报还糟。实测这类瞬时占用几十毫秒内就消失。
        std::thread::sleep(std::time::Duration::from_millis(30));
        if is_occupied(path) {
            found.push(Blocker {
                path: path.to_path_buf(),
                processes: blocking_processes(path),
            });
            // 目录本身被占住时不必再往里钻，报出来就够动手了
            return;
        }
        // 复检发现是一闪而过的，那就当没占用，继续往里探——真正卡住 rename 的
        // 东西可能就在这个目录底下
    }

    // 只跟进真正的目录。junction / symlink 一律不进去——跟进去会走到仓库外面，
    // 甚至绕成环；node_modules 里全是这种链接。
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        if found.len() >= limit || *probes >= MAX_PROBES {
            return;
        }
        probe_into(&entry.path(), limit, probes, found);
    }
}

/// 能不能独占地拿到这个路径？拿不到就是有人占着。
#[cfg(windows)]
fn is_occupied(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    /// 没这个标志 `CreateFile` 打不开目录。
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    match std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0) // 不允许任何人同时持有
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
    {
        Ok(_) => false,
        // 只有「共享冲突 / 拒绝访问」才算被占。找不到、权限不足之类的不是这里要报的事，
        // 当成没占用，免得把一堆无关路径塞进错误信息里
        Err(e) => matches!(e.raw_os_error(), Some(32) | Some(33)),
    }
}

#[cfg(not(windows))]
fn is_occupied(_path: &Path) -> bool {
    // 类 Unix 上 rename 不受「文件被打开」影响，这类失败根本不会发生
    false
}

/// 问 Restart Manager：哪些进程正握着这个文件？
///
/// 只对**文件句柄**有效。目录被当成工作目录占用时 RM 什么也给不出来，返回空。
#[cfg(windows)]
fn blocking_processes(path: &Path) -> Vec<BlockingProcess> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::RestartManager::{
        RmEndSession, RmGetList, RmRegisterResources, RmStartSession, RM_PROCESS_INFO,
    };

    let wide: Vec<u16> = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    };

    let mut session: u32 = 0;
    // 文档要求这个 key 缓冲区是 CCH_RM_SESSION_KEY+1 个宽字符
    let mut key = [0u16; 33];
    // SAFETY: session/key 都是本函数的局部缓冲区，长度按文档给足。
    let started = unsafe { RmStartSession(&mut session, None, PWSTR(key.as_mut_ptr())) };
    if !started.is_ok() {
        return Vec::new();
    }

    let mut out = Vec::new();
    // SAFETY: 全程持有一个已开好的 session，末尾无条件 RmEndSession 收尾；
    // 传进去的指针都指向本函数的局部缓冲区，`infos` 的容量与传入的 count 一致。
    unsafe {
        let files = [PCWSTR(wide.as_ptr())];
        if RmRegisterResources(session, Some(&files), None, None).is_ok() {
            let mut needed: u32 = 0;
            let mut count: u32 = 0;
            let mut reason: u32 = 0;
            // 先问要多大，再按这个大小要一次。中间可能有新进程加进来，拿不全就算了
            // ——这只是给人看的诊断信息，不值得为此重试到底
            let _ = RmGetList(session, &mut needed, &mut count, None, &mut reason);
            if needed > 0 {
                let mut infos = vec![RM_PROCESS_INFO::default(); needed as usize];
                count = needed;
                if RmGetList(
                    session,
                    &mut needed,
                    &mut count,
                    Some(infos.as_mut_ptr()),
                    &mut reason,
                )
                .is_ok()
                {
                    for info in infos.iter().take(count as usize) {
                        let name = String::from_utf16_lossy(&info.strAppName)
                            .trim_end_matches('\0')
                            .to_string();
                        out.push(BlockingProcess {
                            name: if name.is_empty() {
                                "未知程序".into()
                            } else {
                                name
                            },
                            pid: info.Process.dwProcessId,
                        });
                    }
                }
            }
        }
        let _ = RmEndSession(session);
    }
    out
}

#[cfg(not(windows))]
fn blocking_processes(_path: &Path) -> Vec<BlockingProcess> {
    Vec::new()
}

/// 把探测结果写成给人看的说明。
///
/// `root` 用来把长路径缩成相对路径——错误框里塞完整绝对路径没人读得下去。
pub fn describe(root: &Path, blockers: &[Blocker]) -> String {
    if blockers.is_empty() {
        return "挨个查了一遍，没找到具体被占用的文件。\n\
                这种情况基本是**有程序的工作目录停在这个 worktree 里**（终端、\
                另一个编辑器窗口、还没退干净的 dev server）。把它们关掉或 cd 到别处再试。"
            .to_string();
    }

    let mut lines = vec!["卡在这些地方：".to_string()];
    for b in blockers {
        let shown = b.path.strip_prefix(root).unwrap_or(&b.path);
        let shown = if shown.as_os_str().is_empty() {
            "（worktree 目录本身）".to_string()
        } else {
            shown.to_string_lossy().to_string()
        };
        if b.processes.is_empty() {
            // 查不出进程本身就是个结论，别让它看起来像「查了但没查到」
            lines.push(format!(
                "  {shown}  —— 查不出是哪个进程，多半是有程序的工作目录停在这里"
            ));
        } else {
            let who = b
                .processes
                .iter()
                .map(|p| format!("{} (PID {})", p.name, p.pid))
                .collect::<Vec<_>>()
                .join("、");
            lines.push(format!("  {shown}  —— {who}"));
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没人占用时不该报任何东西——误报会让这段提示彻底失去可信度。
    #[test]
    fn a_free_directory_has_no_blockers() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("f.txt"), "1").unwrap();

        assert!(find_blockers(dir.path(), 5).is_empty());
    }

    /// 探测**不能跟进链接**，否则会走到仓库外面去。
    #[cfg(windows)]
    #[test]
    fn probing_does_not_follow_junctions() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::process::CommandExt;
        let dir = tempfile::TempDir::new().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let inside = dir.path().join("inside");
        std::fs::create_dir_all(&inside).unwrap();

        // 在链接目标里放一个「被占用」的东西：文件句柄一直开着
        let victim = outside.join("held.txt");
        std::fs::write(&victim, "1").unwrap();
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&victim)
            .unwrap();

        let out = std::process::Command::new("cmd")
            .arg("/c")
            .raw_arg(format!(
                "mklink /J \"{}\" \"{}\"",
                inside.join("link").display(),
                outside.display()
            ))
            .output()
            .unwrap();
        assert!(out.status.success());

        // 跟进 junction 的话会撞见 outside/held.txt 并报出来
        let blockers = find_blockers(&inside, 5);
        assert!(
            blockers.is_empty(),
            "探测跟进了 junction，报出了链接外面的东西：{blockers:?}"
        );
    }

    /// 有人开着文件时，既要定位到文件，**也要报出是哪个进程**——这是 Restart
    /// Manager 那半边的唯一证据。这里持有句柄的就是测试进程自己，所以断言得到的
    /// PID 就是自己的。
    #[cfg(windows)]
    #[test]
    fn a_held_file_is_found_and_its_holder_is_named() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let held = dir.path().join("held.txt");
        std::fs::write(&held, "1").unwrap();
        let _handle = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&held)
            .unwrap();

        let blockers = find_blockers(dir.path(), 5);

        // 不断言「正好一个」：临时目录偶尔会被杀软/索引服务瞬时握住，
        // 那不是这个测试要管的事
        let held_entry = blockers
            .iter()
            .find(|b| b.path.ends_with("held.txt"))
            .unwrap_or_else(|| panic!("没找到那个被握住的文件，实际找到：{blockers:?}"));
        let pids: Vec<u32> = held_entry.processes.iter().map(|p| p.pid).collect();
        assert!(
            pids.contains(&std::process::id()),
            "Restart Manager 该指出持有句柄的进程，实际拿到：{:?}",
            held_entry.processes,
        );
    }

    /// 有进程把目录当工作目录时也要定位得到——这是删 worktree 最常撞上的情形，
    /// 而它恰恰是 RM 查不出进程的那一种，所以只断言路径。
    #[cfg(windows)]
    #[test]
    fn a_directory_held_as_a_working_directory_is_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        // 无参数的 cmd 进交互模式，把 stdin 接成管道它就会一直等输入、不退出。
        // 不用 `timeout` / `ping` 之类：PATH 上常有同名的 GNU 版本，会被截走。
        let mut child = std::process::Command::new("cmd")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .current_dir(&sub)
            .spawn()
            .expect("起不来子进程");
        // 等它真的把 cwd 设进去
        std::thread::sleep(std::time::Duration::from_millis(600));

        let blockers = find_blockers(dir.path(), 5);

        // 先收拾子进程，再断言——断言失败也不能把进程漏在外面
        let _ = child.kill();
        let _ = child.wait();

        assert!(
            blockers.iter().any(|b| b.path.ends_with("sub")),
            "把目录当 cwd 的占用该被找到，实际：{blockers:?}"
        );
    }

    #[test]
    fn describe_says_something_useful_when_nothing_was_found() {
        let text = describe(Path::new("C:/repo/wt"), &[]);
        assert!(text.contains("工作目录"), "该给出可动手的方向：{text}");
    }

    #[test]
    fn describe_shows_paths_relative_to_the_worktree() {
        let blockers = [Blocker {
            path: PathBuf::from("C:/repo/wt/src/App.tsx"),
            processes: vec![BlockingProcess {
                name: "Code.exe".into(),
                pid: 1234,
            }],
        }];
        let text = describe(Path::new("C:/repo/wt"), &blockers);
        assert!(text.contains("Code.exe (PID 1234)"), "{text}");
        assert!(
            !text.contains("C:/repo/wt/src"),
            "不该再带绝对路径前缀：{text}"
        );
    }
}
