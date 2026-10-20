//! 本机 merge driver 配置检查（spec §4.6）。
//!
//! `.gitattributes` 跟着仓库走，driver 的配置却写在每台机器自己的 git 配置里。完全没配时 git 会
//! 退回逐行文本合并，往 Unity 场景里写冲突标记；配了一半时整个合并直接 fatal；程序路径失效时
//! driver 失败、自动合并失效。这里在合并发生之前把这三种情况找出来。
//!
//! 生效的属性以 `git check-attr` 的结果为准：子目录覆盖、`[attr]` 宏、`info/attributes`、
//! `core.attributesFile` 都由 git 算好，直接扫 `.gitattributes` 原文会误报也会漏报。

use crate::engine::conflicts::{merge_config_raw, parse_merge_config, MergeConfig};
use crate::engine::git_cli::{run_git, run_git_bytes, run_git_stdin};
use crate::error::Result;
use git2::Repository;
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DriverState {
    /// 配了 driver，程序也在（或不是绝对路径、没法检查）
    Ok,
    /// 本机完全没配这个名字：合并时 git 会写冲突标记
    Missing,
    /// 有 `merge.<名字>.*` 的其他项但没有 `driver`：合并会 fatal
    Incomplete,
    /// `driver` 命令的程序是绝对路径且不存在
    ProgramMissing { program: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverFinding {
    pub name: String,
    pub state: DriverState,
    /// 用这个 driver 的路径，最多 3 条，放进提示里
    pub sample_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverReport {
    pub findings: Vec<DriverFinding>,
    /// 检查本身失败时的原因。非 None 时界面显示「检查未完成」，不能当成配置正常
    pub incomplete: Option<String>,
}

const MAX_SAMPLES: usize = 3;
/// `check-attr` 的值里这些不是需要检查的自定义 driver 名字。
///
/// `lfs`：`git lfs track` 往每个 LFS 仓库都写 `merge=lfs`，而 git-lfs 从来不定义 `merge.lfs`
/// 这个 driver，git 一直是退回文本合并 pointer 文件（Task 5 实测）。这是所有 LFS 仓库的常态，
/// 报出来只会让每个 LFS 仓库都挂一条「Unity 场景会被写坏」的红色横幅、每次合并都弹确认框。
/// LFS 冲突本身由冲突显示的 ④（`filter=lfs`）兜住。
const NOT_CUSTOM: &[&str] = &[
    "unspecified",
    "set",
    "unset",
    "text",
    "binary",
    "union",
    "lfs",
];

/// 进程内缓存：仓库路径 → (fingerprint, 结果)。只缓存成功的结果。
fn cache() -> &'static Mutex<HashMap<String, (u64, DriverReport)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (u64, DriverReport)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// 每个仓库实际跑了几次 `check-attr`。按仓库分开计：测试并行跑，全局计数会互相干扰。
fn counts() -> &'static Mutex<HashMap<String, usize>> {
    static COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    COUNTS.get_or_init(Default::default)
}

#[doc(hidden)]
pub fn check_count(repo: &str) -> usize {
    counts()
        .lock()
        .map(|c| c.get(repo).copied().unwrap_or(0))
        .unwrap_or(0)
}

/// 检查这个仓库（可以是 linked worktree）里生效的自定义 merge driver 在本机的配置状态。
///
/// **不返回 Err**：任何一步失败都写进 `incomplete`，界面显示「检查未完成」，
/// 不能因为检查失败就显示成配置正常。
pub fn check_merge_drivers(repo: &str) -> DriverReport {
    run(repo).unwrap_or_else(|e| DriverReport {
        findings: Vec::new(),
        incomplete: Some(e.message.trim().to_string()),
    })
}

fn run(repo: &str) -> Result<DriverReport> {
    let raw = merge_config_raw(repo)?;
    let fp = fingerprint(repo, &raw)?;
    if let Ok(cache) = cache().lock() {
        if let Some((cached_fp, report)) = cache.get(repo) {
            if *cached_fp == fp {
                return Ok(report.clone());
            }
        }
    }
    let report = compute(repo, &parse_merge_config(&raw))?;
    if let Ok(mut cache) = cache().lock() {
        cache.insert(repo.to_string(), (fp, report.clone()));
    }
    Ok(report)
}

fn mtime(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// 没设 `core.attributesFile` 时 git 读的默认位置：`$XDG_CONFIG_HOME/git/attributes`，
/// 没有 XDG 时是 `~/.config/git/attributes`。
fn default_attributes_file() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg).join("git").join("attributes"));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".config")
            .join("git")
            .join("attributes"),
    )
}

/// 决定缓存是否还能用的指纹。任何一项变了都重新跑 `check-attr`：
///
/// - `merge.*` 配置的原始输出；
/// - index 里所有路径（新增了一个场景文件，规则没变也要重新算，否则缺 driver 永远报不出来）；
/// - index 里每个 `.gitattributes` 的 oid，以及它在工作区的 mtime（改了还没提交也算）；
/// - 根目录 `.gitattributes` 的 mtime（还没被跟踪的新文件也会生效）；
/// - `info/attributes`（按 `git rev-parse --git-path` 解析，linked worktree 也对）的路径和 mtime；
/// - `core.attributesFile`（没设时是默认位置）的路径和 mtime。
///
/// 存一次普通文件这些都不变，所以文件监听的每次触发都能命中缓存。
fn fingerprint(repo_path: &str, raw_cfg: &str) -> Result<u64> {
    let mut h = DefaultHasher::new();
    raw_cfg.hash(&mut h);
    let repo = Repository::open(repo_path)?;
    let workdir = repo.workdir().map(Path::to_path_buf);
    let index = repo.index()?;
    for entry in index.iter() {
        entry.path.hash(&mut h);
        let is_attr = entry.path == b".gitattributes" || entry.path.ends_with(b"/.gitattributes");
        if is_attr {
            entry.id.as_bytes().hash(&mut h);
            if let Some(wd) = &workdir {
                let rel = String::from_utf8_lossy(&entry.path).into_owned();
                let mut abs = wd.clone();
                abs.extend(rel.split('/'));
                format!("{:?}", mtime(&abs)).hash(&mut h);
            }
        }
    }
    if let Some(wd) = &workdir {
        format!("{:?}", mtime(&wd.join(".gitattributes"))).hash(&mut h);
    }
    let info = run_git(repo_path, &["rev-parse", "--git-path", "info/attributes"])?;
    let info = PathBuf::from(info.trim());
    let info = if info.is_absolute() {
        info
    } else {
        Path::new(repo_path).join(info)
    };
    info.hash(&mut h);
    format!("{:?}", mtime(&info)).hash(&mut h);
    let attributes_file = repo
        .config()
        .ok()
        .and_then(|c| c.get_path("core.attributesFile").ok())
        .or_else(default_attributes_file);
    attributes_file.hash(&mut h);
    format!("{:?}", attributes_file.as_deref().and_then(mtime)).hash(&mut h);
    Ok(h.finish())
}

/// `ls-files -z` 和 `check-attr --stdin -z merge` 各跑一次，不按文件逐个调用。
fn compute(repo: &str, cfg: &MergeConfig) -> Result<DriverReport> {
    let files = run_git_bytes(repo, &["ls-files", "-z"])?;
    let attrs = if files.is_empty() {
        String::new()
    } else {
        if let Ok(mut c) = counts().lock() {
            *c.entry(repo.to_string()).or_default() += 1;
        }
        run_git_stdin(repo, &["check-attr", "--stdin", "-z", "merge"], &files)?
    };
    // `merge.default` 是自定义名字时，没写 merge 属性的文件都会交给它
    let default_custom = cfg.default.as_deref().filter(|d| !NOT_CUSTOM.contains(d));

    let mut names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(d) = default_custom {
        names.entry(d.to_string()).or_default();
    }
    // 输出是 `<path>\0merge\0<value>\0` 的三元组
    let fields: Vec<&str> = attrs.split('\0').collect();
    for triple in fields.chunks_exact(3) {
        let (path, value) = (triple[0], triple[2]);
        let name = match value {
            "unspecified" => match default_custom {
                Some(d) => d,
                None => continue,
            },
            v if NOT_CUSTOM.contains(&v) => continue,
            v => v,
        };
        let samples = names.entry(name.to_string()).or_default();
        if samples.len() < MAX_SAMPLES {
            samples.push(path.to_string());
        }
    }

    let findings = names
        .into_iter()
        .map(|(name, sample_paths)| DriverFinding {
            state: driver_state(cfg, &name),
            name,
            sample_paths,
        })
        .collect();
    Ok(DriverReport {
        findings,
        incomplete: None,
    })
}

fn driver_state(cfg: &MergeConfig, name: &str) -> DriverState {
    match cfg.drivers.get(name) {
        None => DriverState::Missing,
        Some(d) => match &d.driver {
            None => DriverState::Incomplete,
            Some(cmd) => match program_of(cmd) {
                Some(program)
                    if Path::new(&program).is_absolute() && !Path::new(&program).exists() =>
                {
                    DriverState::ProgramMissing { program }
                }
                _ => DriverState::Ok,
            },
        },
    }
}

/// driver 命令的第一个词：带引号就取引号里的整段（路径里常有空格），否则取到第一个空白。
fn program_of(cmd: &str) -> Option<String> {
    let cmd = cmd.trim_start();
    let first = cmd.chars().next()?;
    if first == '\'' || first == '"' {
        let rest = &cmd[1..];
        let end = rest.find(first)?;
        return Some(rest[..end].to_string());
    }
    cmd.split_whitespace().next().map(str::to_string)
}
