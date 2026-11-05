//! TortoiseSVN 客户端钩子的注册（分析 §6.13.5；计划 Task 1；日志 H1、H3）。
//!
//! # 存在哪里
//!
//! `HKCU\Software\TortoiseSVN` 的 `hooks` 值（REG_SZ）。每条钩子 6 行，`\n` 分隔、行尾都有 `\n`：
//! `<kind>_hook` / 工作副本路径 / 命令行 / wait / show / 第 6 字段。第 6 字段 `ask` 对应编辑框里
//! 没勾的「Always execute the script」（H1、H5）。TortoiseSVN 不用重启就认程序写进去的条目。
//!
//! # 只碰自己的条目
//!
//! jkk 的条目靠命令行里的 ` --tsvn-hook ` 认出来，`--repo` 说明属于哪个仓库。其余条目逐字节保留：
//! 用户或公司可能已经挂了自己的钩子。
//!
//! # 冲突
//!
//! TortoiseSVN 按路径前缀匹配钩子（H3：挂在 `wc` 上，从 `wc\Code` 发起也触发）。同一类型在
//! 同一路径、祖先或子孙路径上已有别人的钩子时，到底调哪条没测过，所以一律当冲突停下（分析
//! §6.13.5「不覆盖用户已有的同类钩子」）。

use crate::engine::svn_config::read_config;
use crate::engine::svn_wc::enumerate_wcs;
use crate::error::{GitError, Result};
use std::path::Path;

/// 命令行里认出 jkk 条目的标记。
const MARK: &str = " --tsvn-hook ";

/// 跟进判定离不开的三种钩子。`check_commit` 只负责拦截，可选。
pub const REQUIRED_KINDS: [&str; 3] = ["pre_commit", "post_commit", "post_update"];
pub const BLOCK_KIND: &str = "check_commit";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEntry {
    /// 不带 `_hook` 后缀，例如 `post_commit`。
    pub kind: String,
    pub wc_path: String,
    pub command: String,
    pub wait: String,
    pub show: String,
    pub enforce: String,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HookHealth {
    NotRegistered,
    Healthy {
        block_leaks: bool,
    },
    /// 注册过，但不完整或指向别的程序（便携版换了目录）。跟进按没注册处理。
    Stale {
        reasons: Vec<String>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegPlan {
    /// 还没有、要写入的条目。
    pub add: Vec<HookEntry>,
    /// 挡住注册的别人的条目。
    pub conflicts: Vec<HookEntry>,
}

/// 解析 `hooks` 值。行数凑不满 6 的尾巴原样忽略：那不是一条完整的钩子。
pub fn parse_hooks(value: &str) -> Vec<HookEntry> {
    let lines: Vec<&str> = value.split('\n').collect();
    lines
        .chunks_exact(6)
        .map(|c| HookEntry {
            kind: c[0].strip_suffix("_hook").unwrap_or(c[0]).to_string(),
            wc_path: c[1].to_string(),
            command: c[2].to_string(),
            wait: c[3].to_string(),
            show: c[4].to_string(),
            enforce: c[5].to_string(),
        })
        .collect()
}

pub fn serialize_hooks(entries: &[HookEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        // 少数类型在 TortoiseSVN 里不带 `_hook` 后缀（如 `manual_precommit`），解析时没剥掉，原样写回
        let kind = if e.kind.ends_with("_hook") || e.kind == "manual_precommit" {
            e.kind.clone()
        } else {
            format!("{}_hook", e.kind)
        };
        for line in [&kind, &e.wc_path, &e.command, &e.wait, &e.show, &e.enforce] {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

pub fn is_ours(e: &HookEntry) -> bool {
    e.command.contains(MARK)
}

/// `"<exe>" --tsvn-hook <kind> --repo "<repo>"`。TortoiseSVN 会把自己的参数追加在后面。
///
/// repo 去掉结尾分隔符：`--repo "D:\"` 里的 `\"` 按 Windows 的拆参规则是字面引号，TortoiseSVN
/// 追加的参数会全部并进 repo，钩子一条事件都写不出来（review 1 第 3 条）。
pub fn hook_command(exe: &Path, kind: &str, repo: &str) -> String {
    let repo = repo.trim_end_matches(['\\', '/']);
    format!("\"{}\"{MARK}{kind} --repo \"{repo}\"", exe.display())
}

/// 命令行里引号括起的 exe。
fn command_exe(command: &str) -> Option<&str> {
    command.strip_prefix('"')?.split('"').next()
}

/// 命令行里 `--repo "<…>"` 的值。
fn command_repo(command: &str) -> Option<&str> {
    command.split("--repo \"").nth(1)?.split('"').next()
}

/// Windows 路径比较：大小写不敏感，`/` 与 `\` 等价，忽略结尾分隔符。
fn norm(p: &str) -> String {
    p.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn same_path(a: &str, b: &str) -> bool {
    norm(a) == norm(b)
}

/// a 等于 b，或者一方是另一方的祖先目录。
fn related(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    a == b || b.starts_with(&format!("{a}\\")) || a.starts_with(&format!("{b}\\"))
}

fn ours_for(e: &HookEntry, repo: &str) -> bool {
    is_ours(e) && command_repo(&e.command).is_some_and(|r| same_path(r, repo))
}

fn entry(kind: &str, root: &str, exe: &Path, repo: &str) -> HookEntry {
    HookEntry {
        kind: kind.to_string(),
        wc_path: root.to_string(),
        command: hook_command(exe, kind, repo),
        wait: "true".to_string(),
        show: "hide".to_string(),
        enforce: "ask".to_string(),
    }
}

/// 这个仓库应该有的全部条目。
pub fn desired_entries(
    wc_roots: &[String],
    exe: &Path,
    repo: &str,
    block_leaks: bool,
) -> Vec<HookEntry> {
    let mut kinds: Vec<&str> = Vec::new();
    if block_leaks {
        kinds.push(BLOCK_KIND);
    }
    kinds.extend(REQUIRED_KINDS);
    wc_roots
        .iter()
        .flat_map(|root| kinds.iter().map(move |k| entry(k, root, exe, repo)))
        .collect()
}

pub fn plan_registration(
    existing: &[HookEntry],
    wc_roots: &[String],
    exe: &Path,
    repo: &str,
    block_leaks: bool,
) -> RegPlan {
    let desired = desired_entries(wc_roots, exe, repo, block_leaks);
    let conflicts = existing
        .iter()
        .filter(|e| !ours_for(e, repo))
        .filter(|e| {
            desired
                .iter()
                .any(|d| d.kind == e.kind && related(&d.wc_path, &e.wc_path))
        })
        .cloned()
        .collect();
    let add = desired
        .into_iter()
        .filter(|d| {
            !existing.iter().any(|e| {
                e.kind == d.kind && same_path(&e.wc_path, &d.wc_path) && e.command == d.command
            })
        })
        .collect();
    RegPlan { add, conflicts }
}

/// 去掉这个仓库的 jkk 条目，其余（含别的仓库的 jkk 条目）原样保留。
pub fn without_ours(existing: &[HookEntry], repo: &str) -> Vec<HookEntry> {
    existing
        .iter()
        .filter(|e| !ours_for(e, repo))
        .cloned()
        .collect()
}

/// 注册之后 `hooks` 应有的值：先去掉本仓库的旧条目（可能指向旧 exe），再整套追加。
/// 有冲突时报 `tsvn_hooks_conflict`。
pub fn registered_value(
    existing_value: &str,
    wc_roots: &[String],
    exe: &Path,
    repo: &str,
    block_leaks: bool,
) -> Result<String> {
    check_recognized(existing_value)?;
    let existing = parse_hooks(existing_value);
    let plan = plan_registration(&existing, wc_roots, exe, repo, block_leaks);
    if !plan.conflicts.is_empty() {
        let list: Vec<String> = plan
            .conflicts
            .iter()
            .map(|e| format!("{}：{}", e.kind, e.wc_path))
            .collect();
        return Err(GitError::new(
            "tsvn_hooks_conflict",
            format!(
                "这些路径上已经有同类型的 TortoiseSVN 钩子，jkk 不覆盖：{}",
                list.join("；")
            ),
        ));
    }
    let mut out = without_ours(&existing, repo);
    out.extend(desired_entries(wc_roots, exe, repo, block_leaks));
    Ok(serialize_hooks(&out))
}

/// 撤销注册之后的值。一条都不剩时返回 `None`（删掉整个值，还原成从没注册过）。
pub fn unregistered_value(existing_value: &str, repo: &str) -> Option<String> {
    let kept = without_ours(&parse_hooks(existing_value), repo);
    (!kept.is_empty()).then(|| serialize_hooks(&kept))
}

/// 同 [`unregistered_value`]，但先确认这个值认得出来（见 [`check_recognized`]）。
pub fn try_unregistered_value(existing_value: &str, repo: &str) -> Result<Option<String>> {
    check_recognized(existing_value)?;
    Ok(unregistered_value(existing_value, repo))
}

/// 注册与撤销都要从解析结果重写整个值。解析有损时（CRLF、凑不满 6 行的残条、旧版的行数、
/// 错位）重写会改坏别人的钩子，所以先确认：逐字节往返一致，且每条的类型行、wait、show
/// 都是认得的取值（行数恰好凑成 6 的倍数的错位，往返照样一致，要靠取值才能看出来）。
/// 认不出来就拒绝写，让用户去 TortoiseSVN 设置里看一眼（review 1 第 2 条）。
fn check_recognized(value: &str) -> Result<()> {
    if value.is_empty() {
        return Ok(());
    }
    let entries = parse_hooks(value);
    let fields_ok = entries.iter().all(|e| {
        !e.kind.is_empty()
            && !e.kind.contains(['\\', '/', ':', '\r', ' '])
            && matches!(e.wait.as_str(), "true" | "false")
            && matches!(e.show.as_str(), "show" | "hide")
    });
    if fields_ok && serialize_hooks(&entries) == value {
        return Ok(());
    }
    Err(GitError::new(
        "tsvn_hooks_unrecognized",
        "TortoiseSVN 的钩子设置格式认不出来，jkk 不改动它，以免弄坏已有的钩子。请在 TortoiseSVN 设置 → Hook Scripts 里检查后重新保存一次。",
    ))
}

pub fn judge_health(
    existing: &[HookEntry],
    wc_roots: &[String],
    exe: &Path,
    repo: &str,
) -> HookHealth {
    let mine: Vec<&HookEntry> = existing.iter().filter(|e| ours_for(e, repo)).collect();
    if mine.is_empty() {
        return HookHealth::NotRegistered;
    }
    let exe_now = exe.display().to_string();
    let mut reasons = Vec::new();
    let mut block_leaks = !wc_roots.is_empty();
    for root in wc_roots {
        let at_root: Vec<&&HookEntry> = mine
            .iter()
            .filter(|e| same_path(&e.wc_path, root))
            .collect();
        for kind in REQUIRED_KINDS {
            match at_root.iter().find(|e| e.kind == kind) {
                None => reasons.push(format!("{root} 缺少 {kind} 钩子")),
                Some(e) => {
                    let old = command_exe(&e.command).unwrap_or_default();
                    if !same_path(old, &exe_now) {
                        reasons.push(format!("{kind} 钩子指向 {old}，当前程序在 {exe_now}"));
                    }
                }
            }
        }
        block_leaks &= at_root.iter().any(|e| e.kind == BLOCK_KIND);
    }
    if reasons.is_empty() {
        HookHealth::Healthy { block_leaks }
    } else {
        reasons.dedup();
        HookHealth::Stale { reasons }
    }
}

/// 跟进时用的健康检查：只读注册表，不跑 svn。
///
/// 工作副本根取本仓库 jkk 条目自己登记的路径，而不是重新 `enumerate_wcs`：那要对每条白名单
/// 跑 `svn info`。代价是「白名单后来多了一个工作副本、那里没挂钩子」看不出来；那个子树没有
/// 事件，判定落到「拿不准」，方向安全。完整的检查（含新根）在 `tsvn_hooks_status` 命令里做。
/// 读不了注册表、值认不出来：按没注册处理。
pub fn quick_health(repo: &str) -> HookHealth {
    let value = match read_hooks_value() {
        Ok(Some(v)) => v,
        Ok(None) => return HookHealth::NotRegistered,
        Err(e) => {
            log::warn!("读不出 TortoiseSVN 钩子设置，按没注册处理：{}", e.message);
            return HookHealth::NotRegistered;
        }
    };
    let entries = parse_hooks(&value);
    let mut roots: Vec<String> = Vec::new();
    for e in entries.iter().filter(|e| ours_for(e, repo)) {
        if !roots.iter().any(|r| same_path(r, &e.wc_path)) {
            roots.push(e.wc_path.clone());
        }
    }
    let Ok(exe) = std::env::current_exe() else {
        return HookHealth::NotRegistered;
    };
    judge_health(&entries, &roots, &exe, repo)
}

/// 要挂钩子的工作副本根：白名单覆盖到的各工作副本的根，反斜杠、去重。祖先已在集合里的根
/// 丢掉：TortoiseSVN 按前缀匹配（H3），挂在祖先上已经覆盖。
pub fn registration_roots(repo: &str) -> Result<Vec<String>> {
    let cfg = read_config(repo)?;
    let mut roots: Vec<String> = Vec::new();
    for wc in enumerate_wcs(repo, &cfg.whitelist)? {
        let Some(abs) = wc.wc_root_abspath else {
            continue;
        };
        let abs = abs.replace('/', "\\").trim_end_matches('\\').to_string();
        if !roots.iter().any(|r| same_path(r, &abs)) {
            roots.push(abs);
        }
    }
    let all = roots.clone();
    roots.retain(|r| {
        !all.iter()
            .any(|o| !same_path(o, r) && norm(r).starts_with(&format!("{}\\", norm(o))))
    });
    roots.sort();
    Ok(roots)
}

#[cfg(windows)]
#[allow(unsafe_code)] // HKCU 注册表读写只有 Win32 C API，没有安全封装
mod registry {
    use crate::error::{GitError, Result};
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const SUBKEY: PCWSTR = w!("Software\\TortoiseSVN");
    const VALUE: PCWSTR = w!("hooks");

    fn fail(what: &str, e: impl std::fmt::Debug) -> GitError {
        GitError::new(
            "tsvn_hooks_failed",
            format!("{what} TortoiseSVN 钩子设置失败：{e:?}"),
        )
    }

    pub fn read() -> Result<Option<String>> {
        let mut size: u32 = 0;
        // SAFETY：只问长度，输出缓冲为空
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                SUBKEY,
                VALUE,
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if rc == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        rc.ok().map_err(|e| fail("读取", e))?;
        let mut buf = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY：缓冲按上一步问到的字节数分配
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                SUBKEY,
                VALUE,
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        rc.ok().map_err(|e| fail("读取", e))?;
        let len = (size as usize / 2).min(buf.len());
        let s = String::from_utf16_lossy(&buf[..len]);
        Ok(Some(s.trim_end_matches('\0').to_string()))
    }

    pub fn write(value: Option<&str>) -> Result<()> {
        match value {
            Some(v) => {
                let wide: Vec<u16> = v.encode_utf16().chain(std::iter::once(0)).collect();
                // SAFETY：数据指针与字节长度来自同一个 Vec
                let rc = unsafe {
                    RegSetKeyValueW(
                        HKEY_CURRENT_USER,
                        SUBKEY,
                        VALUE,
                        REG_SZ.0,
                        Some(wide.as_ptr().cast()),
                        (wide.len() * 2) as u32,
                    )
                };
                rc.ok().map_err(|e| fail("写入", e))
            }
            None => {
                // SAFETY：只传静态宽字符串
                let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, SUBKEY, VALUE) };
                if rc == ERROR_FILE_NOT_FOUND {
                    return Ok(());
                }
                rc.ok().map_err(|e| fail("删除", e))
            }
        }
    }
}

/// 读 `hooks` 值。没有这个值（从没挂过钩子）是 `Ok(None)`。
pub fn read_hooks_value() -> Result<Option<String>> {
    #[cfg(windows)]
    {
        registry::read()
    }
    #[cfg(not(windows))]
    {
        Err(unsupported())
    }
}

/// CAS 写入：写之前重读，与 `expected` 不同就不写；写完再读回核对。`new == None` 删掉这个值。
///
/// 两道核对防的是 TortoiseSVN 设置窗口开着时整值写回（计划 Review Focus 第 2 条）：写之前
/// 能挡住已经发生的改动，写之后能发现马上被覆盖的情形。几分钟之后才被覆盖的发现不了，所以
/// 注册框提醒先关掉设置窗口（日志 Final Review 第 3 条）。
pub fn cas_write(
    expected: Option<&str>,
    new: Option<&str>,
    read: impl Fn() -> Result<Option<String>>,
    write: impl Fn(Option<&str>) -> Result<()>,
) -> Result<()> {
    let changed = || {
        GitError::new(
            "tsvn_hooks_registry_changed",
            "TortoiseSVN 的钩子设置刚被别的程序改过。请先关掉 TortoiseSVN 的设置窗口，再试一次。",
        )
    };
    if read()?.as_deref() != expected {
        return Err(changed());
    }
    write(new)?;
    if read()?.as_deref() != new {
        return Err(changed());
    }
    Ok(())
}

/// [`cas_write`] 落到 `HKCU\Software\TortoiseSVN\hooks`。
pub fn write_hooks_value(expected: Option<&str>, new: Option<&str>) -> Result<()> {
    #[cfg(windows)]
    {
        cas_write(expected, new, registry::read, registry::write)
    }
    #[cfg(not(windows))]
    {
        let _ = (expected, new);
        Err(unsupported())
    }
}

#[cfg(not(windows))]
fn unsupported() -> GitError {
    GitError::new(
        "tsvn_hooks_unsupported",
        "TortoiseSVN 钩子只在 Windows 上可用。",
    )
}
