//! §6.3.1 的**服务器侧**证伪：拿 `svn ls -R` 的清单与磁盘上的 W-file 比。
//!
//! # 它补的是一个四道检查全绿的盲区
//!
//! 2026-09-15 实测，`svn update --set-depth=exclude Code/Deep` 之后：
//!
//! | 检查 | 结果 |
//! |---|---|
//! | P1 白名单内 `svn status` | 零条目 |
//! | P2 `nodeRevs` | 单值 |
//! | P3 工作副本状态 | 干净 |
//! | §6.8 基线体检 | **Verified**（差异只剩 `kind=dir`，被既有过滤剔掉） |
//!
//! 而基线里静默少了服务器上确实存在的 `Code/Deep/D.cs`——**用户以为源码受 git
//! 保护，实际没有**（spec §6.3.1 称之为最危险的一类缺陷）。
//!
//! `svn info` 对父目录照样报 `depth=infinity`，靠读 depth 检测不出来。唯一看得见
//! 它的入口是 `svn ls -R <url>@N`（实测：它仍然列出那棵被排除的子树）。
//!
//! # 这条检查只加提示，不参与任何既有门槛
//!
//! 它走网络、可能慢、可能失败。所以它的结论不进初始化 / 重建 / 交付的任何判定，
//! 只在界面上多挂一条说明。误报的代价止于噪音，漏报的代价与改动前相同。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{read_config, SvnConfig};
use crate::engine::svn_dirty::{classify, read_dirty, Fix};
use crate::engine::svn_include_check::{normalize_roots, server_expected_set};
use crate::engine::svn_preflight::check_targets;
use crate::engine::svn_scope::WScope;
use crate::engine::svn_wc::{enumerate_wcs, WcIdentity};
use crate::engine::svn_xml::parse_list;
use crate::error::Result;
use std::collections::{BTreeMap, BTreeSet};

/// 白名单内当前的本地改动分类（spec §16 R2a 的那一份）。
///
/// 与录制路径走的是**同一对函数**（`read_dirty` + `classify`），所以这里算出来的
/// 就是录制时会算出来的那一份，不是另编的一份。两份一旦分叉，这条核对报的
/// 「缺失」就会和基线里的实际内容对不上。
fn dirty_fixes(
    repo_root: &str,
    cfg: &SvnConfig,
    units: &[WcIdentity],
) -> Result<BTreeMap<String, Fix>> {
    let targets = check_targets(&normalize_roots(&cfg.whitelist), units);
    classify(&read_dirty(repo_root, &targets)?)
}

/// 列出来给用户看的上限。再多就只报个数——几千行的清单没人看，还会把界面撑爆。
const MAX_LISTED: usize = 20;

/// 服务器侧核对的三种结论。形状与 [`crate::engine::svn_health::Health`] 对齐。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ServerAudit {
    /// 服务器上白名单内的代码文件，磁盘上一个不少。
    Matched,
    /// 服务器上有、磁盘上没有——git 没有在保护它们。
    Missing {
        /// 最多 [`MAX_LISTED`] 条，仓库内相对路径、正斜杠。
        files: Vec<String>,
        /// 实际漏掉的总数，可能大于 `files.len()`。
        total: usize,
    },
    /// 没验成（断网 / URL 不可达 / revision 不存在）。**不是「没漏收」。**
    NotChecked { reason: String },
}

/// 用**当前基线的 revision** 跑一次服务器侧核对。
///
/// 基线还没建起来（或 trailer 读不出来）时返回 `NotChecked` 而不是 `Err`：
/// 调用方是「打开仓库」那条热路径，没有基线是一个正常状态。
pub fn audit_server_set_at_baseline(repo_root: &str) -> Result<ServerAudit> {
    match crate::engine::svn_sync::read_baseline_rev(repo_root) {
        Ok(rev) => audit_server_set(repo_root, rev),
        Err(e) => Ok(ServerAudit::NotChecked {
            reason: format!("读不到基线 revision，没法核对服务器内容：{}", e.message),
        }),
    }
}

/// 拿服务器在 `rev` 上的文件清单与磁盘上的 W-file 比一次。
///
/// # ⚠️ 必须走 `run_svn`，退出码非零即 `Err`
///
/// 实测 P1：`svn ls` 对不存在的路径 **exit 1**，但 stdout 仍是一份结构完整的空
/// `<lists><list></list></lists>`，错误只在 stderr。而空仓库 `@0` 是 **exit 0**
/// 加同样的空清单——**两者在 stdout 上一模一样**。
///
/// 若忽略退出码，一个打错的 URL 会得到空的服务器集合，而
/// 「服务器有、磁盘没有」的差集在空集上**恒为空** ⇒ 报告「没有漏收」。
/// 那就是用修 NB-2 的代码重新造一个 NB-2。
pub fn audit_server_set(repo_root: &str, rev: u64) -> Result<ServerAudit> {
    let cfg = read_config(repo_root)?;
    let scope = WScope::for_repo(repo_root, &cfg)?;
    if scope.roots().is_empty() {
        return Ok(ServerAudit::NotChecked {
            reason: "白名单是空的，没有核对范围。".to_string(),
        });
    }
    let units = enumerate_wcs(repo_root, &cfg.whitelist)?;

    // 服务器侧集合：每个记账单位各列一次，用它自己的 url(P)。
    let mut server: BTreeSet<String> = BTreeSet::new();
    for unit in &units {
        let unit_url = unit.url.trim_end_matches('/');
        let target = format!("{unit_url}@{rev}");
        let xml = match run_svn(repo_root, &["ls", "-R", "--xml", "--", &target]) {
            Ok(x) => x,
            Err(e) => {
                // 一个单位没列成就整体判「没验成」。「其余的都对得上所以 Matched」
                // 是这条检查最坏的结论——它要防的就是「以为受保护、实际没有」。
                return Ok(ServerAudit::NotChecked {
                    reason: format!("列举 `{}` 的服务器内容失败：{}", unit.wc_root, e.message),
                });
            }
        };
        let listed = match parse_list(&xml) {
            Ok(l) => l,
            // revision 不存在时 svn 把 XML 截断在半路（实测 P2），解析会失败。
            // 这条检查是锦上添花，不该让打开仓库失败——落成 NotChecked。
            Err(e) => {
                return Ok(ServerAudit::NotChecked {
                    reason: format!("解析 `{}` 的服务器清单失败：{}", unit.wc_root, e.message),
                })
            }
        };
        // **单文件记账单位**：白名单条目可以是一个具体文件（设计明确支持，
        // 见 `is_w_file` 的文档）。那时这个单位的 url 直接指向那个文件，
        // 2026-09-16 实测 `svn ls -R --xml <url>/Code/notes.txt@N` 返回：
        //
        // ```text
        // <list path=".../Code/notes.txt"><entry kind="file"><name>notes.txt</name>
        // ```
        //
        // 照「接前缀」的通则拼出来是 `Code/notes.txt/notes.txt` —— 一个磁盘上
        // 永远不存在的路径，落进 missing 差集，变成一条对**完全健康的仓库**
        // 常挂不下的误报。`svn_health` 里有同一个相等分支，这里补上。
        let unit_is_file = listed.entries.len() == 1
            && listed.entries[0].kind == "file"
            && unit_url
                .to_lowercase()
                .ends_with(&format!("/{}", listed.entries[0].name.to_lowercase()));

        for entry in listed.entries {
            // 目录也是 entry（实测），只要文件。
            if entry.kind != "file" {
                continue;
            }
            // `<name>` 是**相对 `<list path>`** 的递归路径（实测 N4），
            // 接上这个单位在仓库里的位置才是仓库内相对路径。
            let rel = if unit_is_file {
                unit.wc_root.trim_end_matches('/').to_string()
            } else {
                format!("{}/{}", unit.wc_root.trim_end_matches('/'), entry.name)
            };
            server.insert(rel);
        }
    }
    // 与 git 侧**同一套**判据过滤：服务器上的美术资产本来就不该进 git，
    // 不过滤的话每个真实工程一打开就报几千条。「尊重 .gitignore」时被它排除的
    // 同样不算——那些文件用户明确选了不交给 git。
    let server = scope.filter(server.iter().map(String::as_str))?;

    // 比的是**服务器该有的那个集合**，不是磁盘上现有的（spec §6.3.1）。
    //
    // # 为什么不能直接用 `expected_set`
    //
    // 它遍历磁盘。而 §16.1 放松「工作副本必须干净」之后，用户本地 `svn delete`
    // 掉的文件**服务器上有、磁盘上没有**——正好落进 `server − disk` 这个差集，
    // 于是每一次本地删除都会被报成「服务器上有代码文件不在本地，git 没在保护它」。
    //
    // 那是彻头彻尾的误报：那个文件恰恰**在基线里**（录制时从 BASE 取回来了），
    // git 保护得好好的，用户只是主动删了它。而这条提示的措辞是在指控
    // `--set-depth=exclude` 那类**用户不知情**的漏收——方向完全相反。
    //
    // 误报的代价不止是噪音：它会让用户学会忽略这条提示，而这条提示要抓的那类
    // 缺陷（磁盘上压根看不见的漏收）是四道检查全绿时唯一的信号。
    let expected = match dirty_fixes(repo_root, &cfg, &units) {
        Ok(fixes) => server_expected_set(repo_root, &scope, &fixes)?,
        // 读不到脏清单（比如白名单里有 obstructed）时**不猜**。这条检查是锦上添花，
        // 退回 `NotChecked` 比拿一个可能误报的集合去指控用户强。
        Err(e) => {
            return Ok(ServerAudit::NotChecked {
                reason: format!(
                    "读不到白名单内的本地改动，没法判断哪些缺失是你自己删的：{}",
                    e.message
                ),
            })
        }
    };

    // **只报这一个方向。** 反方向（磁盘有、服务器没有）是「本地新增还没 svn add」，
    // 完全正常；报了就是噪音，而噪音会让用户学会忽略这条提示。
    let missing: Vec<String> = server.difference(&expected).cloned().collect();
    if missing.is_empty() {
        return Ok(ServerAudit::Matched);
    }
    let total = missing.len();
    Ok(ServerAudit::Missing {
        files: missing.into_iter().take(MAX_LISTED).collect(),
        total,
    })
}
