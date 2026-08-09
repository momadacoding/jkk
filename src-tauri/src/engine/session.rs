use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 最近打开列表的上限。再往下用户不会翻，而且这份文件每次标签变动都要重写一遍。
const MAX_RECENT: usize = 10;

/// 落盘的会话状态。
///
/// 字段名带多词（`active_tab`），serde 的 `camelCase` 决定了文件里写的是
/// `activeTab`；这和 Tauri 命令参数的 snake_case → camelCase 是两件事，
/// 但保持同一风格能少一层心智负担。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// 目前恒为 1，只为将来迁移留位；解析时不校验其值。
    #[serde(default = "default_version")]
    version: u32,
    /// 上次退出时打开的标签，保持标签栏顺序。
    #[serde(default)]
    pub tabs: Vec<String>,
    /// 上次的活动标签。存路径而不是下标——还原时会跳过打不开的仓库，下标会错位。
    #[serde(default)]
    pub active_tab: Option<String>,
    /// 最近打开过的仓库，最新在前。
    #[serde(default)]
    pub recent: Vec<String>,
}

fn default_version() -> u32 {
    1
}

impl Default for Session {
    fn default() -> Self {
        Self {
            version: default_version(),
            tabs: Vec::new(),
            active_tab: None,
            recent: Vec::new(),
        }
    }
}

impl Session {
    pub fn new(tabs: Vec<String>, active_tab: Option<String>, recent: Vec<String>) -> Self {
        Self {
            version: default_version(),
            tabs,
            active_tab,
            recent,
        }
    }
}

/// 交给前端的一条最近记录。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentRepo {
    pub path: String,
    /// 路径当前是否还在。只做一次文件系统探测，不校验它是不是 git 仓库——
    /// 「仓库被删了」是常态，「还在但已经不是仓库」不是。
    pub exists: bool,
}

/// 启动时交给前端的还原视图。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub tabs: Vec<String>,
    pub active_tab: Option<String>,
    pub recent: Vec<RecentRepo>,
    /// 存档里有、但这次打不开、因而没有还原的标签。
    ///
    /// 前端只用它判断「这次还原是不完整的」，进而**不要立刻把结果写回去**：
    /// 仓库放在移动硬盘或网络盘上、启动时盘没挂上，标签会被全部滤掉，
    /// 紧接着的那次自动保存就会把标签集永久截断成空——盘接回来也找不回了。
    pub dropped_tabs: Vec<String>,
}

/// 读取会话；文件不存在、读不动或解析失败，一律退化成空会话。
///
/// 这里和 `custom_config::load_from` 的取舍**相反**，因为文件的来源不同：
/// `customCommands.json` 是用户手写的，解析失败必须报错并保住原文件；
/// 而 `session.json` 是程序自己写的，坏了只说明上次写到一半（断电、强杀）。
/// 那种情况下最好的行为是当作没有会话、落到空态页，
/// 而不是把一个用户无从下手的解析错误顶到启动路径上。
pub fn load_from(path: &Path) -> Session {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Session::default();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// 临时文件名的序号，保证同一进程内两次并发保存不会撞名。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 临时文件路径：`session.json.<pid>.<seq>.tmp`。
///
/// **必须唯一**。共用一个固定的 `.tmp` 时，两次并发保存会这样交错：
/// T1 正在往 tmp 里写，T2 把同一个 tmp rename 成正式文件——搬过去的就是
/// T1 写到一半的内容，恰好是原子写本来要防的那种半截 JSON。
/// 带上 pid 是为了让多开实例之间也不撞。
fn tmp_path(path: &Path) -> PathBuf {
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{}.tmp", std::process::id(), seq));
    path.with_file_name(name)
}

/// 原子落盘：先写同目录的临时文件，再 rename 覆盖。
///
/// 直接覆写的话，正在写的瞬间被杀就会留下半截 JSON，上次的会话等于没了。
/// 临时文件必须和目标**同目录**：跨卷的 rename 不是原子操作，Windows 上会直接失败。
///
/// 任何一步失败都要把临时文件收掉，否则配置目录会慢慢攒出一堆 `.tmp`。
pub fn save_to(path: &Path, session: &Session) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(session)
        .map_err(|e| GitError::new("session", e.to_string()))?;
    let tmp = tmp_path(path);
    if let Err(e) = std::fs::write(&tmp, text) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    // Windows 上 std::fs::rename 走 MoveFileEx + MOVEFILE_REPLACE_EXISTING，
    // 目标已存在也能覆盖；这一点由 tests/session.rs 的用例守住。
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// 归一化只用于比较，存进文件的仍然是原始路径。
///
/// 规则与前端 `src/engine/path.ts` 的 `normalizePath` 一致：反斜杠转正斜杠、
/// 去掉尾部斜杠、忽略大小写。忽略大小写在 Linux 上严格来说不对，但两侧规则
/// 一旦漂移，同一个仓库会出现「标签去重认得、最近列表不认得」这种极难查的表现，
/// 那个代价更大。
fn compare_key(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// 把本次打开的仓库并进最近列表：最新在前、按归一化路径去重、截断到 `MAX_RECENT`。
///
/// `opened` 是标签栏顺序，新标签总是追加在右边，所以倒着遍历才是「最新在前」。
/// 空路径直接丢掉——它既不是仓库，出现在列表里也点不开。
pub fn merge_recent(previous: &[String], opened: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in opened.iter().rev().chain(previous.iter()) {
        if path.is_empty() {
            continue;
        }
        if seen.insert(compare_key(path)) {
            out.push(path.clone());
        }
        if out.len() >= MAX_RECENT {
            break;
        }
    }
    out
}

/// 一个路径能不能作为仓库打开。
///
/// 用 `git2::Repository::open` 而不是 `engine::repo::open`：后者还会读 HEAD，
/// 一个刚 `git init`、还没有任何提交的仓库会被它判成打不开，
/// 于是这种仓库永远还原不回来。
fn is_repo(path: &str) -> bool {
    git2::Repository::open(path).is_ok()
}

/// 给最近列表里的每条标上「还在不在」。
pub fn to_recent(paths: Vec<String>) -> Vec<RecentRepo> {
    paths
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|path| RecentRepo {
            exists: Path::new(&path).exists(),
            path,
        })
        .collect()
}

/// 组装还原视图。
///
/// `tabs` 分成两拨：能打开的照常还原，打不开的进 `dropped_tabs`——
/// 还原出一个必然报错的标签没有意义，但也不能让前端以为存档里本来就没有它们
/// （见 `SessionView::dropped_tabs` 的说明）。
/// `recent` **不**过滤，只标注是否还在：网络盘、移动硬盘临时不在线是常事，
/// 因为一次离线就把历史永久删掉，用户再也找不回那个仓库。
pub fn to_view(session: Session) -> SessionView {
    let (tabs, dropped_tabs): (Vec<String>, Vec<String>) = session
        .tabs
        .into_iter()
        .filter(|p| !p.is_empty())
        .partition(|p| is_repo(p));
    // 活动标签本身可能刚被滤掉，此时交回 None，由前端回落到第一个标签
    let active_tab = session
        .active_tab
        .filter(|a| tabs.iter().any(|t| compare_key(t) == compare_key(a)));
    SessionView {
        tabs,
        active_tab,
        recent: to_recent(session.recent),
        dropped_tabs,
    }
}
