//! 查出「哪些进程把这个 worktree 占着」，并在用户点名时结束其中一个。
//!
//! [`crate::engine::file_lock`] 能定位**哪个路径**被占，但只有「打开了里面的文件」那一种
//! 占用能靠 Restart Manager 查出进程。删 worktree 最常撞上的是另一种：某个进程的**工作目录**
//! 停在里面（终端、dev server、没退干净的 agent 子进程）。这里补上那一半：
//!
//! | 判据 | 怎么查 | 可信度 |
//! |---|---|---|
//! | 工作目录在 worktree 里 | 读进程 PEB → `ProcessParameters.CurrentDirectory` | 确凿：这就是挡住 rename 的那个句柄 |
//! | 打开了里面的文件 | Restart Manager（`file_lock`） | 确凿 |
//! | 启动参数里写着 worktree 路径 | `NtQueryInformationProcess(ProcessCommandLineInformation)` | 旁证：多半相关，不一定是它挡的 |
//!
//! 读 PEB 要 `PROCESS_VM_READ`，只对**本用户、未提权**的进程有效；读不到就跳过，不当错误。
//! 命令行只要 `PROCESS_QUERY_LIMITED_INFORMATION`，覆盖面更大，所以留作兜底。

use crate::error::{GitError, Result};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

/// 这个进程是怎么跟 worktree 扯上关系的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HoldKind {
    /// 工作目录停在 worktree 里。
    Cwd,
    /// 打开了 worktree 里的文件（Restart Manager 报的）。
    OpenFile,
    /// 只是启动参数里写着 worktree 的路径。
    CommandLine,
}

/// 一个占着 worktree 的进程。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Holder {
    pub pid: u32,
    /// 可执行文件名，如 `node.exe`；Restart Manager 报的是它给的应用名。
    pub name: String,
    pub kind: HoldKind,
    /// 被占的具体路径：工作目录、被打开的文件；命令行匹配时就是 worktree 根。
    pub path: String,
    /// 能不能从界面上结束。本应用自己（含它的子进程）、Restart Manager 标成系统关键的进程不给。
    pub killable: bool,
    /// 是本应用自己，或者它启动的子进程（WebView2、git 守护进程……）。
    /// 结束它们会把本应用自己弄坏，界面据此说明原因而不是「系统关键进程」。
    pub is_self: bool,
    /// 命令行一档才有：命中路径附近的一段命令行。只是旁证的那一档，
    /// 用户得看见依据才能判断要不要结束它（独立 review 第 1 条）。
    pub command_line: Option<String>,
}

impl Holder {
    fn new(
        p: &ProcEntry,
        kind: HoldKind,
        path: String,
        critical: bool,
        ours: &HashSet<u32>,
    ) -> Self {
        let is_self = ours.contains(&p.pid);
        Holder {
            pid: p.pid,
            name: p.name.clone(),
            kind,
            path,
            killable: !is_self && !critical,
            is_self,
            command_line: None,
        }
    }
}

/// 本进程和它的全部后代。
///
/// 从一个工作目录停在 worktree 里的终端启动本应用时，子进程（WebView2 浏览器进程等）
/// 都继承了那个工作目录，会被列成持有者；结束它们会把本应用自己弄崩。父 PID 可能已被
/// 复用成无关进程，那样只会多把一些进程标成「不给结束」，错在安全的方向。
fn own_tree(procs: &[ProcEntry]) -> HashSet<u32> {
    let mut ours = HashSet::from([std::process::id()]);
    // 父子表是一张快照里的，迭代到不再增长即可；深度有限，次数不会多
    loop {
        let before = ours.len();
        for p in procs {
            if p.pid != p.parent && ours.contains(&p.parent) {
                ours.insert(p.pid);
            }
        }
        if ours.len() == before {
            return ours;
        }
    }
}

/// 命令行里命中 `root` 的那一段，前后各留一些上下文，总长不超过 `CMDLINE_SNIPPET` 个字符。
fn cmdline_snippet(cmdline: &str, root: &str) -> String {
    const CMDLINE_SNIPPET: usize = 200;
    let chars: Vec<char> = cmdline.chars().collect();
    if chars.len() <= CMDLINE_SNIPPET {
        return cmdline.to_string();
    }
    // 命中位置按字符数算；大小写折叠偶尔改变字符数，偏一点只影响截取的窗口，不影响判定
    let hit = cmdline_match(cmdline, root).unwrap_or(0).min(chars.len());
    let start = hit.saturating_sub(CMDLINE_SNIPPET / 4);
    let end = (start + CMDLINE_SNIPPET).min(chars.len());
    let start = end.saturating_sub(CMDLINE_SNIPPET);
    let mut s: String = chars[start..end].iter().collect();
    if start > 0 {
        s.insert(0, '…');
    }
    if end < chars.len() {
        s.push('…');
    }
    s
}

/// 去掉 NT / 扩展长度前缀再归一化（正斜杠、去尾斜杠、Windows 上小写）。
///
/// PEB 里的工作目录偶尔带 `\\?\`，和界面传来的普通路径直接比会永远对不上。
fn clean(p: &str) -> String {
    let p = p
        .strip_prefix(r"\\?\")
        .or_else(|| p.strip_prefix(r"\??\"))
        .unwrap_or(p);
    crate::engine::path::normalize(p)
}

/// 归一化之后的根；明显不像一个 worktree 的（空串、盘符根）一律拒绝。
///
/// 这是结束进程的判据之一：根要是退化成 `c:`，C 盘上所有进程都会被当成持有者。
fn usable_root(root: &str) -> Option<String> {
    let r = clean(root);
    // 归一化去掉了尾斜杠，`C:\` 变成 `c:`，没有 `/` 就是盘符根或空串
    r.contains('/').then_some(r)
}

/// `candidate` 是不是 `root` 本身或它底下的路径。
pub(crate) fn path_within(candidate: &str, root: &str) -> bool {
    let Some(r) = usable_root(root) else {
        return false;
    };
    let c = clean(candidate);
    c == r || c.starts_with(&format!("{r}/"))
}

/// 能接在路径后面、让它变成**另一个**名字的字符。
///
/// `C:/wt` 后面跟着 `2`、`-old`、`.bak` 都是兄弟目录，不是它。
fn continues_name(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '$' | '@' | '+')
}

/// 命令行里有没有提到 `root`（本身或它底下的路径）。
///
/// 两端都要落在名字边界上：前面不能紧贴着名字字符，后面要么结束、要么是 `/` 或分隔符。
/// `--cwd=C:\wt`、`"C:\wt\src"`、`C:/wt` 都算，`C:\wt2` 不算。
pub(crate) fn cmdline_mentions(cmdline: &str, root: &str) -> bool {
    cmdline_match(cmdline, root).is_some()
}

/// 同 [`cmdline_mentions`]，答出命中处在命令行里的字符序号。
fn cmdline_match(cmdline: &str, root: &str) -> Option<usize> {
    let r = usable_root(root)?;
    let hay = cmdline.replace('\\', "/");
    let hay = if cfg!(windows) {
        hay.to_lowercase()
    } else {
        hay
    };
    let mut from = 0;
    while let Some(off) = hay[from..].find(&r) {
        let start = from + off;
        let end = start + r.len();
        let before_ok = hay[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !continues_name(c));
        let after_ok = hay[end..].chars().next().is_none_or(|c| !continues_name(c));
        if before_ok && after_ok {
            return Some(hay[..start].chars().count());
        }
        // 往后挪一个字符继续找；按字符边界挪，免得切在多字节字符中间
        from = start + hay[start..].chars().next().map_or(1, char::len_utf8);
    }
    None
}

/// 列出占着 `root` 的进程。
///
/// 进程表是快照，查完到用户点按钮之间可能变了，所以结束之前 [`terminate_holder`] 会再核一遍。
pub fn find_holders(root: &Path) -> Vec<Holder> {
    let root_s = root.to_string_lossy().to_string();
    if usable_root(&root_s).is_none() {
        return Vec::new();
    }
    let procs = sys::processes();
    let ours = own_tree(&procs);
    let mut out: Vec<Holder> = Vec::new();

    for p in &procs {
        let facts = sys::inspect_pid(p.pid);
        if let Some(cwd) = facts.cwd.filter(|c| path_within(c, &root_s)) {
            out.push(Holder::new(p, HoldKind::Cwd, cwd, false, &ours));
        } else if let Some(cmdline) = facts.cmdline.filter(|c| cmdline_mentions(c, &root_s)) {
            let mut h = Holder::new(p, HoldKind::CommandLine, root_s.clone(), false, &ours);
            h.command_line = Some(cmdline_snippet(&cmdline, &root_s));
            out.push(h);
        }
    }

    // 打开了里面文件的进程。工作目录那一类上面已经逐个读过了，探测时不再让 file_lock
    // 为每个被占的目录把全部进程重新枚举一遍（独立 review 第 4 条）
    for blocker in crate::engine::file_lock::find_blockers_with(root, 3, false) {
        for p in blocker.processes {
            let held = blocker.path.to_string_lossy().to_string();
            if let Some(existing) = out.iter_mut().find(|h| h.pid == p.pid) {
                // 同一个进程既是命令行匹配、又确实开着文件：按确凿的那条报
                if existing.kind == HoldKind::CommandLine {
                    existing.kind = HoldKind::OpenFile;
                    existing.path = held;
                    existing.command_line = None;
                }
                existing.killable &= !p.critical;
                continue;
            }
            let entry = ProcEntry {
                pid: p.pid,
                parent: 0,
                name: p.name,
            };
            out.push(Holder::new(
                &entry,
                HoldKind::OpenFile,
                held,
                p.critical,
                &ours,
            ));
        }
    }

    out.sort_by(|a, b| a.kind.cmp(&b.kind).then(a.pid.cmp(&b.pid)));
    out
}

/// 只看工作目录：哪些进程的 cwd 落在 `dir` 里。给 `file_lock` 补「查不出进程」那一行。
pub(crate) fn cwd_holders(dir: &Path) -> Vec<(u32, String)> {
    let dir_s = dir.to_string_lossy();
    if usable_root(&dir_s).is_none() {
        return Vec::new();
    }
    sys::processes()
        .into_iter()
        .filter(|p| {
            sys::inspect_pid(p.pid)
                .cwd
                .is_some_and(|c| path_within(&c, &dir_s))
        })
        .map(|p| (p.pid, p.name))
        .collect()
}

/// 结束占着 `root` 的进程 `pid`。
///
/// **先拿到进程句柄、再用这个句柄核对它确实占着 `root`，最后用同一个句柄结束它。**
/// 句柄握在手里，进程对象就不会被回收，PID 也就不会在核对和结束之间被别的进程复用。
/// 核对不过（已经退出、换了目录、根本不相干）就拒绝，什么都不做。
///
/// `held_path` 是列表里报的那个被占路径。开着文件的那一类只能问 Restart Manager，有了它就
/// 只问这一个路径，不必把整个 worktree 重新探测一遍（独立 review 第 4 条）；它必须落在
/// `root` 底下，否则拒绝。RM 把这个进程标成系统关键时，不管其余判据命中与否都拒绝，
/// 与界面「不给按钮」保持一致（独立 review 第 2 条）。
pub fn terminate_holder(root: &Path, pid: u32, held_path: Option<&str>) -> Result<()> {
    if own_tree(&sys::processes()).contains(&pid) {
        return Err(GitError::new(
            "holder_is_self",
            "这是本应用自己，或者它启动的子进程，不能从这里结束。\n\n\
             关掉打开着这个 worktree 的标签页；本应用要是从停在这个 worktree 里的终端启动的，\
             从别处重新启动它，再试一次删除。",
        ));
    }
    let root_s = root.to_string_lossy().to_string();
    if usable_root(&root_s).is_none() {
        return Err(not_holding(pid));
    }
    if held_path.is_some_and(|h| !path_within(h, &root_s)) {
        return Err(not_holding(pid));
    }
    sys::terminate_if(pid, |facts| {
        // 句柄握着才问 RM：问到的 PID 一定还是同一个进程
        let rm = held_path
            .map(|h| crate::engine::file_lock::blocking_processes(Path::new(h)))
            .unwrap_or_default();
        let rm_hit = rm.iter().find(|p| p.pid == pid);
        if rm_hit.is_some_and(|p| p.critical) {
            return Err(GitError::new(
                "holder_critical",
                format!("进程 {pid} 是系统关键进程，不能从这里结束。"),
            ));
        }
        let by_cwd = facts
            .cwd
            .as_deref()
            .is_some_and(|c| path_within(c, &root_s));
        let by_cmdline = facts
            .cmdline
            .as_deref()
            .is_some_and(|c| cmdline_mentions(c, &root_s));
        if by_cwd || by_cmdline || rm_hit.is_some() {
            Ok(())
        } else {
            Err(not_holding(pid))
        }
    })
}

fn not_holding(pid: u32) -> GitError {
    GitError::new(
        "holder_not_found",
        format!(
            "进程 {pid} 已经不占着这个 worktree 了（可能已经退出或换了目录），没有结束它。\n\n\
             直接再试一次删除。"
        ),
    )
}

/// 进程快照里的一条。
pub(crate) struct ProcEntry {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

/// 从一个进程身上读到的东西；读不到的就是 `None`。
#[derive(Debug, Default)]
pub(crate) struct ProcFacts {
    pub cwd: Option<String>,
    pub cmdline: Option<String>,
}

#[cfg(windows)]
#[allow(unsafe_code)] // 读进程 PEB、结束进程都只有 Win32 / NT C API，没有安全封装
mod sys {
    use super::{ProcEntry, ProcFacts};
    use crate::error::{GitError, Result};
    use windows::Wdk::System::Threading::{
        NtQueryInformationProcess, ProcessBasicInformation, ProcessCommandLineInformation,
        ProcessWow64Information,
    };
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS,
        PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        PROCESS_TERMINATE, PROCESS_VM_READ,
    };

    /// 工作目录、命令行的长度上限（字节）。UNICODE_STRING 的 Length 是 u16，本来就到不了更大，
    /// 这里再卡一道，免得读到一个损坏的结构时按它说的长度去读。
    const MAX_STRING_BYTES: usize = 0xFFFF;

    /// 关闭时自动 `CloseHandle` 的进程句柄。
    struct Proc(HANDLE);

    impl Drop for Proc {
        fn drop(&mut self) {
            // SAFETY: 句柄来自 OpenProcess / CreateToolhelp32Snapshot 的成功返回，只关一次
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<Proc> {
        // SAFETY: 纯 Win32 调用，失败返回 Err，不触碰任何指针
        unsafe { OpenProcess(access, false, pid) }.ok().map(Proc)
    }

    pub(super) fn processes() -> Vec<ProcEntry> {
        let mut out = Vec::new();
        // SAFETY: 快照句柄由 Proc 负责关闭；entry 是本函数的局部变量，dwSize 按文档先填好
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                return out;
            };
            let snap = Proc(snap);
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut ok = Process32FirstW(snap.0, &mut entry).is_ok();
            while ok {
                let pid = entry.th32ProcessID;
                // 0 是 Idle、4 是 System，都不是能占着用户目录的东西
                if pid > 4 {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    out.push(ProcEntry {
                        pid,
                        parent: entry.th32ParentProcessID,
                        name: String::from_utf16_lossy(&entry.szExeFile[..len]),
                    });
                }
                ok = Process32NextW(snap.0, &mut entry).is_ok();
            }
        }
        out
    }

    pub(super) fn inspect_pid(pid: u32) -> ProcFacts {
        // 能读内存就一次拿两样；提权进程之类读不了内存的，退到只要命令行的最低权限
        if let Some(p) = open(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ) {
            return inspect(&p);
        }
        ProcFacts {
            cwd: None,
            cmdline: open(pid, PROCESS_QUERY_LIMITED_INFORMATION).and_then(|p| cmdline(&p)),
        }
    }

    fn inspect(p: &Proc) -> ProcFacts {
        ProcFacts {
            cwd: cwd(p),
            cmdline: cmdline(p),
        }
    }

    /// 打开句柄 → 用这个句柄读出的信息核对 → 用同一个句柄结束。
    /// `holds` 不认（`Err`）就原样把它的理由交回去，不结束。
    pub(super) fn terminate_if(pid: u32, holds: impl Fn(&ProcFacts) -> Result<()>) -> Result<()> {
        let base = PROCESS_TERMINATE | PROCESS_SYNCHRONIZE;
        let p = open(pid, base | PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)
            .or_else(|| open(pid, base | PROCESS_QUERY_LIMITED_INFORMATION))
            .ok_or_else(|| {
                GitError::new(
                    "terminate_failed",
                    format!(
                        "打不开进程 {pid}：它可能已经退出了，或者是以管理员身份运行的，\
                         本应用没有权限结束它。\n\n请到任务管理器里手动结束。"
                    ),
                )
            })?;
        holds(&inspect(&p))?;
        // SAFETY: p 是带 PROCESS_TERMINATE | SYNCHRONIZE 打开的有效句柄
        unsafe {
            TerminateProcess(p.0, 1).map_err(|e| {
                GitError::new(
                    "terminate_failed",
                    format!("结束进程 {pid} 失败：{e}\n\n请到任务管理器里手动结束。"),
                )
            })?;
            // 等它真的退出、句柄真的释放，紧接着的重试删除才不会再撞上。退出一般是毫秒级
            let _ = WaitForSingleObject(p.0, 3000);
        }
        Ok(())
    }

    /// 读 `NtQueryInformationProcess` 的一个定长答案。
    ///
    /// # Safety
    /// `T` 必须是该信息类答案的确切内存布局（纯数据、任意位模式都合法）。
    unsafe fn query<T: Default>(
        p: &Proc,
        class: windows::Wdk::System::Threading::PROCESSINFOCLASS,
    ) -> Option<T> {
        let mut out = T::default();
        let mut ret = 0u32;
        // SAFETY: out 是本函数的局部变量，传入的长度就是它的大小
        let status = unsafe {
            NtQueryInformationProcess(
                p.0,
                class,
                (&mut out as *mut T).cast(),
                std::mem::size_of::<T>() as u32,
                &mut ret,
            )
        };
        (status.0 >= 0).then_some(out)
    }

    /// 从对方进程的 `addr` 读一个 `T`。
    ///
    /// # Safety
    /// 同 [`query`]：`T` 是任意位模式都合法的纯数据。
    unsafe fn read<T: Default>(p: &Proc, addr: u64) -> Option<T> {
        let mut out = T::default();
        let size = std::mem::size_of::<T>();
        let mut got = 0usize;
        // SAFETY: 目标缓冲区是本函数的局部变量，大小 size；对方地址无效时 API 返回错误而不是崩
        unsafe {
            ReadProcessMemory(
                p.0,
                addr as usize as *const _,
                (&mut out as *mut T).cast(),
                size,
                Some(&mut got),
            )
        }
        .ok()?;
        (got == size).then_some(out)
    }

    /// 从对方进程读 `bytes` 字节的 UTF-16 串。
    fn read_utf16(p: &Proc, addr: u64, bytes: usize) -> Option<String> {
        if addr == 0 || bytes == 0 || bytes > MAX_STRING_BYTES || !bytes.is_multiple_of(2) {
            return None;
        }
        let mut buf = vec![0u16; bytes / 2];
        let mut got = 0usize;
        // SAFETY: buf 的容量就是 bytes 字节
        unsafe {
            ReadProcessMemory(
                p.0,
                addr as usize as *const _,
                buf.as_mut_ptr().cast(),
                bytes,
                Some(&mut got),
            )
        }
        .ok()?;
        (got == bytes).then(|| String::from_utf16_lossy(&buf))
    }

    /// 读工作目录：PEB → ProcessParameters → CurrentDirectory.DosPath。
    ///
    /// 32 位进程（WOW64）有自己的一份 32 位 PEB，它的 `SetCurrentDirectory` 改的是那一份，
    /// 64 位那份不跟着变，所以要先问它是不是 WOW64，再按对应的布局读。偏移量见 decisions 日志。
    #[cfg(target_pointer_width = "64")]
    fn cwd(p: &Proc) -> Option<String> {
        // SAFETY: ProcessWow64Information 的答案是一个指针宽的整数
        let peb32 = unsafe { query::<usize>(p, ProcessWow64Information) }?;
        if peb32 != 0 {
            // PEB32.ProcessParameters @0x10；RTL_USER_PROCESS_PARAMETERS32.CurrentDirectory @0x24
            // UNICODE_STRING32 = { Length: u16, MaximumLength: u16, Buffer: u32 }
            // SAFETY: u32 / [u16;2]+u32 都是任意位模式合法的纯数据
            let params = unsafe { read::<u32>(p, peb32 as u64 + 0x10) }?;
            let s = unsafe { read::<UnicodeString32>(p, u64::from(params) + 0x24) }?;
            return read_utf16(p, u64::from(s.buffer), usize::from(s.length));
        }
        // PROCESS_BASIC_INFORMATION：第二个字段就是 PebBaseAddress。自己按 6 个指针宽的整数读，
        // 免得为了这一个字段去开 windows crate 的 PEB 结构体 feature
        // SAFETY: 布局是 6 个指针宽字段，任意位模式合法
        let pbi = unsafe { query::<[usize; 6]>(p, ProcessBasicInformation) }?;
        let peb = pbi[1] as u64;
        // PEB64.ProcessParameters @0x20；RTL_USER_PROCESS_PARAMETERS64.CurrentDirectory @0x38
        // UNICODE_STRING64 = { Length: u16, MaximumLength: u16, (pad 4), Buffer: u64 }
        // SAFETY: 同上，纯数据
        let params = unsafe { read::<u64>(p, peb + 0x20) }?;
        let s = unsafe { read::<UnicodeString64>(p, params + 0x38) }?;
        read_utf16(p, s.buffer, usize::from(s.length))
    }

    #[cfg(not(target_pointer_width = "64"))]
    fn cwd(_p: &Proc) -> Option<String> {
        // 本应用只发 64 位包。32 位构建下 64 位目标的 PEB 读不到，宁可不报也不要读错
        None
    }

    #[repr(C)]
    #[derive(Default)]
    struct UnicodeString32 {
        length: u16,
        _maximum_length: u16,
        buffer: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct UnicodeString64 {
        length: u16,
        _maximum_length: u16,
        _pad: u32,
        buffer: u64,
    }

    /// 读命令行。只要 `PROCESS_QUERY_LIMITED_INFORMATION`，提权进程也读得到。
    ///
    /// 答案是一个 UNICODE_STRING 头，后面紧跟字符数据，Buffer 指向**我们自己的**缓冲区里。
    fn cmdline(p: &Proc) -> Option<String> {
        let mut needed = 0u32;
        // 先问要多大。长度不够时它返回错误，但会把需要的大小写进 needed
        // SAFETY: 传空缓冲区、长度 0，只为拿 needed
        unsafe {
            let _ = NtQueryInformationProcess(
                p.0,
                ProcessCommandLineInformation,
                std::ptr::null_mut(),
                0,
                &mut needed,
            );
        }
        let needed = needed as usize;
        let header = std::mem::size_of::<UnicodeStringNative>();
        if needed < header || needed > MAX_STRING_BYTES + header + 16 {
            return None;
        }
        // 按 u64 分配保证对齐，UNICODE_STRING 里有指针
        let mut buf = vec![0u64; needed.div_ceil(8)];
        let cap = buf.len() * 8;
        let mut ret = 0u32;
        // SAFETY: buf 的容量 cap 字节，传入的长度不超过它
        let status = unsafe {
            NtQueryInformationProcess(
                p.0,
                ProcessCommandLineInformation,
                buf.as_mut_ptr().cast(),
                cap as u32,
                &mut ret,
            )
        };
        if status.0 < 0 {
            return None;
        }
        let base = buf.as_ptr() as usize;
        // SAFETY: buf 至少 header 字节且按 8 对齐，头部是纯数据
        let head = unsafe { std::ptr::read(buf.as_ptr().cast::<UnicodeStringNative>()) };
        let len = usize::from(head.length);
        let start = head.buffer;
        // Buffer 必须落在我们自己的缓冲区里，否则不信它
        // 偏移还得是偶数，否则按 u16 读会不对齐
        if len == 0
            || !len.is_multiple_of(2)
            || start < base
            // 用 checked_add：指针是对方进程里读回来的数，不该靠回绕绕过检查
            || start.checked_add(len).is_none_or(|e| e > base + cap)
            || !(start - base).is_multiple_of(2)
        {
            return None;
        }
        // SAFETY: 上面刚核过 [start, start+len) 在 buf 内、按 u16 对齐；buf 活到本函数结束
        let chars = unsafe { std::slice::from_raw_parts(start as *const u16, len / 2) };
        Some(String::from_utf16_lossy(chars))
    }

    /// 本进程位宽下的 UNICODE_STRING。
    #[repr(C)]
    struct UnicodeStringNative {
        length: u16,
        _maximum_length: u16,
        buffer: usize,
    }
}

#[cfg(not(windows))]
mod sys {
    use super::{ProcEntry, ProcFacts};
    use crate::error::{GitError, Result};

    // 类 Unix 上 rename 不受「有进程停在里面」影响，这类占用根本不会发生
    pub(super) fn processes() -> Vec<ProcEntry> {
        Vec::new()
    }

    pub(super) fn inspect_pid(_pid: u32) -> ProcFacts {
        ProcFacts::default()
    }

    pub(super) fn terminate_if(pid: u32, _holds: impl Fn(&ProcFacts) -> Result<()>) -> Result<()> {
        Err(GitError::new(
            "holder_not_found",
            format!("当前平台不支持从这里结束进程 {pid}。"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_within_accepts_the_root_and_paths_below_it() {
        assert!(path_within(r"C:\repo\wt", "C:/repo/wt"));
        assert!(path_within(r"C:\repo\wt\", "C:/repo/wt"));
        assert!(path_within(r"C:\repo\wt\src\deep", r"C:\repo\wt\"));
        // PEB 里偶尔带扩展长度前缀
        assert!(path_within(r"\\?\C:\repo\wt\src", "C:/repo/wt"));
    }

    #[cfg(windows)]
    #[test]
    fn path_within_ignores_case_on_windows() {
        assert!(path_within(r"c:\REPO\Wt\src", r"C:\repo\wt"));
    }

    #[test]
    fn path_within_rejects_siblings_and_parents() {
        assert!(!path_within(r"C:\repo\wt2", "C:/repo/wt"));
        assert!(!path_within(r"C:\repo\wt-old\x", "C:/repo/wt"));
        assert!(!path_within(r"C:\repo", "C:/repo/wt"));
    }

    /// 根退化成空串或盘符根时，**什么都不能算在它底下**——否则结束进程那一步会把
    /// 整个盘上的进程都当成持有者。
    #[test]
    fn degenerate_roots_match_nothing() {
        for root in ["", "/", r"C:\", "C:", "C:/"] {
            assert!(!path_within(r"C:\anything", root), "root {root:?}");
            assert!(
                !cmdline_mentions(r"x.exe C:\anything", root),
                "root {root:?}"
            );
        }
    }

    #[test]
    fn cmdline_mentions_finds_the_root_in_its_usual_shapes() {
        let root = "C:/Users/me/repo/.claude/worktrees/wt";
        // 真实案例的形状：--cwd 后面跟正斜杠路径
        assert!(cmdline_mentions(
            r"node.exe broker.mjs serve --cwd C:/Users/me/repo/.claude/worktrees/wt --pid-file x",
            root,
        ));
        assert!(cmdline_mentions(
            r#""C:\Program Files\nodejs\node.exe" "C:\Users\me\repo\.claude\worktrees\wt\server.js""#,
            root,
        ));
        assert!(cmdline_mentions(
            r"tool --dir=C:\Users\me\repo\.claude\worktrees\wt",
            root
        ));
        assert!(cmdline_mentions(
            r"tool C:\Users\me\repo\.claude\worktrees\wt\",
            root
        ));
    }

    #[test]
    fn cmdline_mentions_ignores_siblings_with_a_longer_name() {
        let root = r"C:\repo\wt";
        assert!(!cmdline_mentions(r"code C:\repo\wt2", root));
        assert!(!cmdline_mentions(r"code C:\repo\wt-old\src", root));
        assert!(!cmdline_mentions(r"code C:\repo\wt.bak", root));
        assert!(!cmdline_mentions(r"code D:\x\C:\repo\wtx", root));
        // 同一行里先出现兄弟、后出现它本身：要接着往后找
        assert!(cmdline_mentions(r"code C:\repo\wt2 C:\repo\wt", root));
    }

    /// 前端 `WorktreeHolder` 按这几个名字取字段，改了这里那边只会静默拿到 undefined。
    #[test]
    fn holder_serializes_in_the_shape_the_frontend_reads() {
        let p = ProcEntry {
            pid: 7,
            parent: 1,
            name: "node.exe".into(),
        };
        let mut h = Holder::new(
            &p,
            HoldKind::CommandLine,
            "C:/r/wt".into(),
            true,
            &HashSet::new(),
        );
        h.command_line = Some("node x --cwd C:/r/wt".into());
        assert_eq!(
            serde_json::to_value(h).unwrap(),
            serde_json::json!({
                "pid": 7,
                "name": "node.exe",
                "kind": "commandLine",
                "path": "C:/r/wt",
                "killable": false,
                "isSelf": false,
                "commandLine": "node x --cwd C:/r/wt",
            })
        );
        assert_eq!(
            serde_json::to_value(HoldKind::OpenFile).unwrap(),
            serde_json::json!("openFile")
        );
        assert_eq!(
            serde_json::to_value(HoldKind::Cwd).unwrap(),
            serde_json::json!("cwd")
        );
    }

    #[test]
    fn cmdline_mentions_survives_non_ascii_text() {
        let root = r"C:\仓库\wt";
        assert!(cmdline_mentions(r"编辑器 --open C:\仓库\wt\文件.txt", root));
        assert!(!cmdline_mentions(r"编辑器 --open C:\仓库\wt文件", root));
    }

    /// 本进程的后代（子、孙）都算「自己的」；父 PID 指向自己的环不能让它死循环。
    #[test]
    fn own_tree_covers_children_and_grandchildren_only() {
        let me = std::process::id();
        let e = |pid, parent| ProcEntry {
            pid,
            parent,
            name: String::new(),
        };
        let procs = [
            e(900_001, me),
            e(900_002, 900_001),
            e(900_003, 1),
            // 自己指向自己的条目（快照里偶见），不能把它卷进来，也不能卡住
            e(900_004, 900_004),
        ];
        let ours = own_tree(&procs);
        assert!(ours.contains(&me) && ours.contains(&900_001) && ours.contains(&900_002));
        assert!(!ours.contains(&900_003) && !ours.contains(&900_004));
    }

    #[test]
    fn cmdline_snippet_keeps_short_lines_whole_and_centres_long_ones_on_the_hit() {
        let root = r"C:\repo\wt";
        assert_eq!(cmdline_snippet(r"x C:\repo\wt", root), r"x C:\repo\wt");

        let long = format!("{} --cwd C:\\repo\\wt {}", "a".repeat(500), "b".repeat(500));
        let s = cmdline_snippet(&long, root);
        assert!(s.contains(r"C:\repo\wt"), "命中处必须在截取的那段里：{s}");
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(s.chars().count() <= 202, "{}", s.chars().count());
    }

    #[cfg(windows)]
    mod live {
        use super::super::*;
        use std::process::{Child, Command, Stdio};

        /// 一个**不是本进程后代**的空转 cmd：经 PowerShell 的 Start-Process 起，起它的
        /// powershell 随即退出。复刻真实案例里父进程早已退出的孤儿；本进程直接起的子进程
        /// 会被认成「本应用自己的」，不给结束（独立 review 第 3 条）。
        struct Detached(u32);

        impl Detached {
            fn start(exe: &str, cwd: &Path, args: &str) -> Self {
                Self::start_with(exe, cwd, args, None)
            }

            /// `stdout` 给了就把它的标准输出重定向进这个文件：进程活着，文件句柄就一直开着。
            fn start_with(exe: &str, cwd: &Path, args: &str, stdout: Option<&Path>) -> Self {
                // 路径来自 TempDir，不含单引号
                let redirect = stdout
                    .map(|p| format!(" -RedirectStandardOutput '{}'", p.display()))
                    .unwrap_or_default();
                let window = if stdout.is_some() {
                    // 重定向时不能同时指定窗口样式
                    " -NoNewWindow"
                } else {
                    " -WindowStyle Hidden"
                };
                // PID 经文件交回，三个标准流都不接管道：带重定向的 Start-Process 会让起出来的
                // 进程继承 powershell 的句柄，接了管道就得等那个进程退出才读得到 EOF
                // 不用 NamedTempFile：它开着文件，Set-Content 写不进去
                let pid_dir = tempfile::TempDir::new().unwrap();
                let pid_file = pid_dir.path().join("pid");
                let script = format!(
                    "(Start-Process -FilePath '{exe}' -ArgumentList '{args}' \
                     -WorkingDirectory '{}'{window}{redirect} -PassThru).Id \
                     | Set-Content -Encoding ascii '{}'",
                    cwd.display(),
                    pid_file.display()
                );
                let status = Command::new("powershell")
                    .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .expect("起不来 powershell");
                let text = std::fs::read_to_string(&pid_file).unwrap_or_default();
                let pid = text
                    .trim()
                    .parse()
                    .unwrap_or_else(|_| panic!("拿不到 PID（{status}）：{text:?}"));
                // 等它真的把 cwd 设进去
                std::thread::sleep(std::time::Duration::from_millis(600));
                Detached(pid)
            }

            fn alive(&self) -> bool {
                let out = Command::new("tasklist")
                    .args(["/FI", &format!("PID eq {}", self.0), "/NH"])
                    .output()
                    .expect("tasklist");
                String::from_utf8_lossy(&out.stdout).contains(&self.0.to_string())
            }
        }

        impl Drop for Detached {
            fn drop(&mut self) {
                let _ = Command::new("taskkill")
                    .args(["/F", "/PID", &self.0.to_string()])
                    .output();
            }
        }

        /// 本进程直接起的、一直等 stdin 的 cmd。
        fn child_cmd(cwd: &Path) -> Child {
            let child = Command::new("cmd")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .current_dir(cwd)
                .spawn()
                .expect("起不来子进程");
            std::thread::sleep(std::time::Duration::from_millis(600));
            child
        }

        /// 核心场景：有进程的工作目录停在 worktree 里，要能**点出它的 PID**——
        /// 这是 Restart Manager 给不出、本模块存在的理由。
        #[test]
        fn a_process_whose_cwd_is_inside_is_named() {
            let dir = tempfile::TempDir::new().unwrap();
            let sub = dir.path().join("sub");
            std::fs::create_dir_all(&sub).unwrap();
            let proc = Detached::start("cmd.exe", &sub, "/k");

            let holders = find_holders(dir.path());

            let h = holders
                .iter()
                .find(|h| h.pid == proc.0)
                .unwrap_or_else(|| panic!("没找到 cwd 停在里面的进程 {}：{holders:?}", proc.0));
            assert_eq!(h.kind, HoldKind::Cwd);
            assert!(h.killable && !h.is_self, "{h:?}");
            assert!(h.name.eq_ignore_ascii_case("cmd.exe"), "{h:?}");
            assert!(path_within(&h.path, &sub.to_string_lossy()), "{h:?}");
        }

        /// 32 位进程的工作目录在它自己那份 32 位 PEB 里，按 64 位布局读会读到错的东西。
        #[test]
        fn a_32_bit_process_whose_cwd_is_inside_is_named() {
            let wow_cmd = r"C:\Windows\SysWOW64\cmd.exe";
            if !Path::new(wow_cmd).exists() {
                eprintln!("跳过：这台机器没有 SysWOW64");
                return;
            }
            let dir = tempfile::TempDir::new().unwrap();
            let proc = Detached::start(wow_cmd, dir.path(), "/k");

            let holders = find_holders(dir.path());

            assert!(
                holders
                    .iter()
                    .any(|h| h.pid == proc.0 && h.kind == HoldKind::Cwd),
                "32 位进程 {} 该按工作目录被找到：{holders:?}",
                proc.0
            );
        }

        /// 工作目录在别处、只是启动参数提到 worktree 的进程，按命令行那一档报，
        /// 并且带上命令行让用户看得见依据。
        #[test]
        fn a_process_that_only_mentions_the_root_is_reported_with_its_commandline() {
            let dir = tempfile::TempDir::new().unwrap();
            let elsewhere = tempfile::TempDir::new().unwrap();
            // `/k rem <路径>`：rem 什么都不做，cmd 留在交互模式
            let proc = Detached::start(
                "cmd.exe",
                elsewhere.path(),
                &format!("/k rem \"{}\\x\"", dir.path().display()),
            );

            let holders = find_holders(dir.path());

            let h = holders
                .iter()
                .find(|h| h.pid == proc.0)
                .unwrap_or_else(|| panic!("没按命令行找到 {}：{holders:?}", proc.0));
            assert_eq!(h.kind, HoldKind::CommandLine);
            let shown = h.command_line.as_deref().unwrap_or_default();
            assert!(
                cmdline_mentions(shown, &dir.path().to_string_lossy()),
                "该带上命中的那段命令行：{h:?}"
            );
        }

        /// 本应用自己开着文件时照样列出来（让用户知道是谁），但不给结束。
        #[test]
        fn this_process_is_listed_but_not_killable() {
            use std::os::windows::fs::OpenOptionsExt;
            let dir = tempfile::TempDir::new().unwrap();
            let held = dir.path().join("held.txt");
            std::fs::write(&held, "1").unwrap();
            let _handle = std::fs::OpenOptions::new()
                .read(true)
                .share_mode(0)
                .open(&held)
                .unwrap();

            let holders = find_holders(dir.path());

            let me = holders
                .iter()
                .find(|h| h.pid == std::process::id())
                .unwrap_or_else(|| panic!("开着文件的本进程该被列出：{holders:?}"));
            assert_eq!(me.kind, HoldKind::OpenFile);
            assert!(!me.killable);
            assert!(me.is_self);
            let err = terminate_holder(dir.path(), std::process::id(), None).unwrap_err();
            assert_eq!(err.code, "holder_is_self");
        }

        /// 本应用起的子进程继承了停在 worktree 里的工作目录（从那里的终端启动本应用的形态）：
        /// 列出来、标成自己的、不给结束，后端也拒绝——结束 WebView2 会把本应用弄崩。
        #[test]
        fn a_child_of_this_process_is_listed_as_ours_and_not_killable() {
            let dir = tempfile::TempDir::new().unwrap();
            let mut child = child_cmd(dir.path());
            let pid = child.id();

            let holders = find_holders(dir.path());
            let result = terminate_holder(dir.path(), pid, None);
            let still_running = child.try_wait().ok().flatten().is_none();
            let _ = child.kill();
            let _ = child.wait();

            let h = holders
                .iter()
                .find(|h| h.pid == pid)
                .unwrap_or_else(|| panic!("子进程 {pid} 该被列出：{holders:?}"));
            assert!(h.is_self && !h.killable, "{h:?}");
            assert_eq!(result.unwrap_err().code, "holder_is_self");
            assert!(still_running, "本应用的子进程被结束了");
        }

        #[test]
        fn terminating_a_holder_ends_it() {
            let dir = tempfile::TempDir::new().unwrap();
            let proc = Detached::start("cmd.exe", dir.path(), "/k");
            assert!(proc.alive());

            terminate_holder(dir.path(), proc.0, None).unwrap();

            assert!(
                !proc.alive(),
                "terminate_holder 返回后进程 {} 该已经退出",
                proc.0
            );
            assert!(
                find_holders(dir.path()).iter().all(|h| h.pid != proc.0),
                "结束之后不该再被列出"
            );
        }

        /// 不占着这个 worktree 的进程，给了 PID 也**不能**结束——PID 可能已经被复用成
        /// 别的程序，前端传来的数字不可信。
        #[test]
        fn a_process_that_does_not_hold_the_root_is_left_alone() {
            let dir = tempfile::TempDir::new().unwrap();
            let elsewhere = tempfile::TempDir::new().unwrap();
            let proc = Detached::start("cmd.exe", elsewhere.path(), "/k");

            let result = terminate_holder(dir.path(), proc.0, None);

            assert_eq!(result.unwrap_err().code, "holder_not_found");
            assert!(proc.alive(), "不相干的进程被结束了");
        }

        /// 前端回传的被占路径不在 worktree 底下：拒绝，哪怕进程本身确实占着 worktree。
        #[test]
        fn a_held_path_outside_the_root_is_refused() {
            let dir = tempfile::TempDir::new().unwrap();
            let elsewhere = tempfile::TempDir::new().unwrap();
            let proc = Detached::start("cmd.exe", dir.path(), "/k");

            let result = terminate_holder(
                dir.path(),
                proc.0,
                Some(&elsewhere.path().to_string_lossy()),
            );

            assert_eq!(result.unwrap_err().code, "holder_not_found");
            assert!(proc.alive());
        }

        /// 占着文件的进程按 RM 的答案结束：只问回传的那一个路径，不再整树探测。
        /// 用一个开着文件、工作目录在别处的非后代进程复刻。
        #[test]
        fn a_process_holding_a_file_is_ended_via_the_held_path() {
            let dir = tempfile::TempDir::new().unwrap();
            let elsewhere = tempfile::TempDir::new().unwrap();
            let held = dir.path().join("held.txt");
            // 一个跑一分钟的 ping，标准输出重定向进 worktree 里的文件：它活着，文件就一直开着。
            // 用绝对路径：PATH 上常有同名的 GNU 版本。cmd /k 不行——没有自己的控制台，
            // 起它的 powershell 一退它就跟着退了
            let proc = Detached::start_with(
                r"C:\Windows\System32\PING.EXE",
                elsewhere.path(),
                "-n 60 127.0.0.1",
                Some(&held),
            );
            let held_s = held.to_string_lossy().to_string();
            let listed = find_holders(dir.path());
            assert!(
                listed
                    .iter()
                    .any(|h| h.pid == proc.0 && h.kind == HoldKind::OpenFile),
                "该按开着文件列出 {}：{listed:?}",
                proc.0
            );

            terminate_holder(dir.path(), proc.0, Some(&held_s)).unwrap();
            assert!(!proc.alive());
        }
    }
}
