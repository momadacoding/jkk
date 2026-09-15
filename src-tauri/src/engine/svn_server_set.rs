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
use crate::engine::svn_config::read_config;
use crate::engine::svn_include_check::{expected_set, is_w_file, normalize_roots};
use crate::engine::svn_wc::enumerate_wcs;
use crate::engine::svn_xml::parse_list;
use crate::error::Result;
use std::collections::BTreeSet;

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
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
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
        for entry in listed.entries {
            // 目录也是 entry（实测），只要文件。
            if entry.kind != "file" {
                continue;
            }
            // `<name>` 是**相对 `<list path>`** 的递归路径（实测 N4），
            // 接上这个单位在仓库里的位置才是仓库内相对路径。
            let rel = format!("{}/{}", unit.wc_root.trim_end_matches('/'), entry.name);
            // 与 git 侧**同一套**判据过滤：服务器上的美术资产本来就不该进 git，
            // 不过滤的话每个真实工程一打开就报几千条。
            if is_w_file(&rel, &roots, &cfg.extensions) {
                server.insert(rel);
            }
        }
    }

    let disk = expected_set(repo_root, &cfg.whitelist, &cfg.extensions)?;

    // **只报这一个方向。** 反方向（磁盘有、服务器没有）是「本地新增还没 svn add」，
    // 完全正常；报了就是噪音，而噪音会让用户学会忽略这条提示。
    let missing: Vec<String> = server.difference(&disk).cloned().collect();
    if missing.is_empty() {
        return Ok(ServerAudit::Matched);
    }
    let total = missing.len();
    Ok(ServerAudit::Missing {
        files: missing.into_iter().take(MAX_LISTED).collect(),
        total,
    })
}
