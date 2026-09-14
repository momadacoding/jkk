//! 初始化向导的前置校验（spec §6.5 的 P1–P4）。
//!
//! **动手之前必须先校验工作副本，否则会造出一个虚假基线。** 基线一旦录错，
//! 后面每一次失同步判定、每一次交付清单都建立在它之上，而它本身看起来完全正常。
//!
//! P5（external 有没有被 `-r` 钉死）不在这里：它**不阻断**，只切换到 pinned
//! 模式，属于另一条路径（`engine::svn_externals`）。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_wc::{check_single_repository, enumerate_wcs, peg_safe, WcIdentity};
use crate::engine::svn_xml::{parse_node_revs, parse_status_entries};
use crate::error::{GitError, Result};
use std::collections::BTreeSet;

/// 录基线需要、而且只有走完校验才敢用的三个事实。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preflight {
    /// 记账单位 N。**取自 `nodeRevs`，不是 `svn info .` 的 revision。**
    ///
    /// 2026-09-14 实测（spec §6.5.1 的形态）：根留在 r1、`Code` 子树更到 r3 时，
    /// `svn info .` 报 1 而 `nodeRevs` = `{3}`。取成 1 的话，基线 trailer 写着
    /// r1 而工作副本在 r3 ——初始化**完成的那一瞬间**就判失同步。
    pub node_rev: u64,
    /// Repository Root。基线 trailer 的 `<url>` 用它（spec §6.4.1）。
    pub repos_root: String,
    /// 仓库 UUID。trailer 的第三个字段。
    pub uuid: String,
}

/// 一次跑完 P1–P4；全过则返回录基线需要的三个事实，任一不过返回 `Err`。
///
/// # 执行顺序不是风格问题
///
/// | 次序 | 检查 | 为什么在这个位置 |
/// |---|---|---|
/// | 0 | 白名单非空 | 空白名单会录出一个空基线，而空基线在体检里**看起来是通过的** |
/// | 1 | 枚举工作副本 | 后面三步都要它的结果；也是第一次真正接触 svn |
/// | 2 | P4 同一仓库 | 跨仓库时 `svn commit` 直接拒（E200007），交付永远做不成 |
/// | 3 | P3 没有未完成操作 | 树没更新完时，P1 和 P2 读到的东西本身就不可信 |
/// | 4 | P1 工作副本干净 | **必须先于 P2** |
/// | 5 | P2 nodeRevs 单值 | |
///
/// P1 先于 P2 的理由是实测出来的：`svn add` 了但没提交的节点带
/// `revision="-1"`，它会正常进 `nodeRevs` 使其变成多值。顺序反了的话用户看到的
/// 是「工作副本是 mixed-revision，请先跑一次完整 svn update」——而真实原因是他有
/// 个没提交的新文件，那句引导做了也没用。
pub fn run_preflight(repo_root: &str, whitelist: &[String]) -> Result<Preflight> {
    let roots = whitelist_roots(whitelist);
    if roots.is_empty() {
        return Err(GitError::new(
            "svn_preflight_empty_whitelist",
            "白名单是空的，没有任何文件可以录进基线。\
             空基线在体检里看起来是通过的（没有文件就没有差异），\
             所以这里直接拒绝，而不是录一个空的出去。",
        ));
    }

    let wcs = enumerate_wcs(repo_root, whitelist)?;
    check_single_repository(&wcs)?; // P4
    check_no_operation_in_progress(repo_root, &wcs)?; // P3
    check_clean(repo_root, &roots)?; // P1
    let node_rev = check_single_node_rev(repo_root, &roots, &wcs)?; // P2

    let first = wcs.first().ok_or_else(|| {
        GitError::new(
            "svn_preflight",
            "白名单非空却没枚举到任何工作副本，这不该发生",
        )
    })?;
    Ok(Preflight {
        node_rev,
        repos_root: first.repos_root.clone(),
        uuid: first.uuid.clone(),
    })
}

/// 白名单条目归一成 svn target：正斜杠、去尾斜杠、丢掉空条目。
fn whitelist_roots(whitelist: &[String]) -> Vec<String> {
    whitelist
        .iter()
        .map(|w| w.replace('\\', "/").trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// P3：工作副本上没有没跑完的 svn 操作。
///
/// # 为什么查的是工作副本根，而不是白名单根
///
/// 2026-09-14 实测（Windows 上独占占住一个待更新的文件，让 `svn update` 中途失败）：
///
/// | 状态 | 工作副本根 `--depth=empty` | 白名单根 `Code@` |
/// |---|---|---|
/// | update 半路失败 | `wc-locked="true"` + `item="incomplete"` | 只有 `wc-locked` |
/// | **再跑一次 `svn cleanup`** | **仍是 `item="incomplete"`** | **一条 entry 都没有** |
///
/// 最后一行是最现实的场景：update 失败 → SVN 提示 run cleanup → 用户照做 → 以为
/// 好了。那时 P1 全绿、P2 单值，只有工作副本根还写着 incomplete，意味着有节点没
/// 更新完。只查白名单根的 P3 会在这里放行。
///
/// 所以查两类目标：仓库根自己的工作副本，以及白名单递归覆盖到的每一个工作副本根
/// （白名单落在 external 里时两者不同）。`--depth=empty` 让每次只看一个节点，
/// 在几万文件的工作副本上也是常数开销。
fn check_no_operation_in_progress(repo_root: &str, wcs: &[WcIdentity]) -> Result<()> {
    let mut targets: Vec<String> = vec![repo_root.replace('\\', "/")];
    for w in wcs {
        if let Some(abs) = &w.wc_root_abspath {
            let norm = abs.replace('\\', "/");
            if !targets.iter().any(|t| t.eq_ignore_ascii_case(&norm)) {
                targets.push(norm);
            }
        }
    }

    for target in &targets {
        let arg = peg_safe(target);
        let xml = run_svn(
            repo_root,
            &[
                "status",
                "--depth=empty",
                "--ignore-externals",
                "--xml",
                "--",
                &arg,
            ],
        )?;
        for e in parse_status_entries(&xml)? {
            if e.wc_locked {
                return Err(operation_in_progress(
                    target,
                    "工作副本上有一把没释放的写锁（svn status 的 `L`）",
                    "先在 TortoiseSVN 里对这个目录跑一次 Cleanup",
                ));
            }
            if e.item == "incomplete" {
                return Err(operation_in_progress(
                    target,
                    "工作副本处于 incomplete 状态——上一次 svn update 没跑完",
                    "先跑一次完整的 svn update 把它补齐。\
                     ⚠️ 只跑 Cleanup 不够：Cleanup 清掉的是写锁，incomplete 会原样留着",
                ));
            }
        }
    }
    Ok(())
}

fn operation_in_progress(target: &str, what: &str, how: &str) -> GitError {
    GitError::new(
        "svn_preflight_operation_in_progress",
        format!(
            "{target}：{what}。\
             这种状态下工作副本里有节点没更新到位，拿它录基线会录出一棵残缺的树，\
             而基线本身看起来完全正常。{how}。"
        ),
    )
}

/// P1：白名单内 `svn status` 必须干净。
///
/// 三个参数缺一不可（spec §6.5）：
///
/// ```text
/// svn status --no-ignore --ignore-externals --xml -- <白名单根>
///            ↑ 不带它会漏掉 SVN-ignored 文件，而其中匹配扩展名的那些 git 照收
///                        ↑ 不带它 external 内部的节点会混进来（§6.6.4）
/// ```
///
/// **不自动规范化。** revert 用户的本地修改是破坏性的，只拦住并引导去
/// TortoiseSVN 自己处理（对齐 spec §8.1 #2）。
///
/// # `ignored` 与 `unversioned` 一律算脏
///
/// spec 的禁止清单里明写着 `?`，而 `--no-ignore` 的作用就是让被 `svn:ignore`
/// 挡住的那些也现形。所以判据就是字面的「白名单内 status 必须干净」，
/// 不按扩展名或别的条件放行。
///
/// 曾经想按「git 会不会收」收窄（只拦匹配白名单扩展名的那些），已撤销：
/// 那既越过了 spec 的明文，风险方向也反了。误拦是**看得见**的——错误信息里
/// 直接列出路径，用户当场就能判断；而放行一个 SVN 不管的文件是**看不见**的，
/// 它会进基线、进交付清单，且基线本身一切正常。
fn check_clean(repo_root: &str, roots: &[String]) -> Result<()> {
    let mut dirty: Vec<String> = Vec::new();
    let mut has_untracked = false;

    for root in roots {
        let arg = peg_safe(root);
        let xml = run_svn(
            repo_root,
            &[
                "status",
                "--no-ignore",
                "--ignore-externals",
                "--xml",
                "--",
                &arg,
            ],
        )?;
        for e in parse_status_entries(&xml)? {
            // `item="external"` 即使带了 --ignore-externals 也照样出现（实测），
            // 它不是脏状态。本设计的核心场景就是「代码在 external 里」，
            // 把它判脏会让向导在目标仓库上永远过不去。
            if e.item == "external" {
                continue;
            }
            match e.item.as_str() {
                "unversioned" | "ignored" => {
                    has_untracked = true;
                    dirty.push(format!("{} （{}，不在版本控制里）", e.path, e.item));
                }
                "normal" => {
                    if e.props != "none" {
                        dirty.push(format!("{} （属性被改过，未提交）", e.path));
                    }
                }
                other => dirty.push(format!("{} （{other}）", e.path)),
            }
        }
    }

    if dirty.is_empty() {
        return Ok(());
    }
    dirty.sort();
    let shown: Vec<&String> = dirty.iter().take(20).collect();
    let more = if dirty.len() > shown.len() {
        format!("\n……另有 {} 条未列出", dirty.len() - shown.len())
    } else {
        String::new()
    };
    let ghost_hint = if has_untracked {
        // 三条出路都要给全。独立 review 指出：只说「svn add 或移出白名单目录」时，
        // 对一个被 svn:ignore 挡住的构建产物目录来说两条都不可行，用户会卡死。
        "\n带「不在版本控制里」的那些，三条出路：\
         ① 该进版本控制的先 svn add；\
         ② 不该进的从白名单目录里移走；\
         ③ 把白名单收窄到不含它（比如从 `Code/` 改成 `Code/Scripts/`）。\n\
         被 svn:ignore 挡住的文件也在此列——SVN 不管它，git 却可能照收，\
         那样录进基线的就是一个 SVN 里根本不存在的文件，交付时必然对不上。"
    } else {
        ""
    };
    Err(GitError::new(
        "svn_preflight_dirty",
        format!(
            "白名单内的工作副本不干净，共 {} 处：\n{}{}\n\
             本应用不会替你 revert（那是破坏性操作），请先在 TortoiseSVN 里\
             提交或还原这些改动，再回来建库。{}",
            dirty.len(),
            shown
                .iter()
                .map(|s| format!("  - {s}"))
                .collect::<Vec<_>>()
                .join("\n"),
            more,
            ghost_hint
        ),
    ))
}

/// P2：`nodeRevs` 必须是单元素集合 `{N}`。
///
/// 读法按 spec §6.6.2：带 `-v` 和 `--ignore-externals`，取 `<wc-status revision>`。
/// 这两条都由 [`parse_node_revs`] 自己把关，漏了会响亮报错而不是静默算错。
fn check_single_node_rev(repo_root: &str, roots: &[String], wcs: &[WcIdentity]) -> Result<u64> {
    let mut all: BTreeSet<i64> = BTreeSet::new();
    for root in roots {
        let arg = peg_safe(root);
        let xml = run_svn(
            repo_root,
            &["status", "-v", "--xml", "--ignore-externals", "--", &arg],
        )?;
        all.extend(parse_node_revs(&xml)?);
    }

    if all.len() > 1 {
        let list = all
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        // spec §6.5 给的引导是「先跑一次完整 svn update」，但 §6.6.4 明确指出
        // **白名单落在 external 里时这条引导永远无效**——external 的 revision
        // 来自另一个仓库的序列，本工作副本的 update 动不了它。给错引导比不给更糟：
        // 用户会反复跑 update，反复失败，而错在别处。
        let hint = if touches_external(repo_root, wcs) {
            "白名单落在 external 里，**跑 svn update 解决不了**——\
             external 的 revision 来自另一个仓库的序列。\
             需要改 svn:externals 的钉死写法，或把白名单收到单一 revision 的子树内。"
        } else {
            "工作副本是 mixed-revision，请先跑一次完整的 svn update 把它对齐到同一个 revision。"
        };
        return Err(GitError::new(
            "svn_preflight_mixed_revision",
            format!("白名单内出现了多个 revision：{{{list}}}。{hint}"),
        ));
    }

    let rev = all.into_iter().next().ok_or_else(|| {
        GitError::new(
            "svn_preflight",
            "没读到任何 revision——这本该由 parse_node_revs 的空集守卫拦下",
        )
    })?;
    u64::try_from(rev).map_err(|_| {
        // 负数只可能来自 `svn add` 了没提交的节点（`revision="-1"`），
        // 而那种节点 P1 已经拦住了。走到这里说明顺序被人调过。
        GitError::new(
            "svn_preflight",
            format!("读到一个负的 revision（{rev}），说明 P1 没有先于 P2 执行"),
        )
    })
}

/// 白名单是否递归覆盖到了仓库根工作副本之外的工作副本（即落进了 external）。
fn touches_external(repo_root: &str, wcs: &[WcIdentity]) -> bool {
    let root = repo_root.replace('\\', "/");
    wcs.iter().any(|w| {
        w.wc_root_abspath
            .as_ref()
            .map(|a| !a.replace('\\', "/").eq_ignore_ascii_case(&root))
            .unwrap_or(false)
    })
}
