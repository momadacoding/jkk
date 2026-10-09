//! 初始化向导的前置校验（spec §6.5 的 P1–P4）。
//!
//! **动手之前必须先校验工作副本，否则会造出一个虚假基线。** 基线一旦录错，
//! 后面每一次失同步判定、每一次交付清单都建立在它之上，而它本身看起来完全正常。
//!
//! P5（external 有没有被 `-r` 钉死）不在这里：它**不阻断**，只切换到 pinned
//! 模式，属于另一条路径（`engine::svn_externals`）。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_dirty::read_dirty;
use crate::engine::svn_wc::{check_single_repository, enumerate_wcs, peg_safe, WcIdentity};
use crate::engine::svn_xml::{parse_node_revs, parse_status_entries, StatusEntry};
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
    /// 白名单内所有**非 pristine** 的条目，spec §16 R2a 的输入。
    ///
    /// # 为什么不过 IPC
    ///
    /// `#[serde(skip)]` 是有意的。这份清单在真实工程上可能有几千条（白名单目录
    /// 底下的每一个未版本控制文件都算），而前端一条都不需要——它只关心校验过没过、
    /// 记账单位是几。每次 preflight 都把几千条路径搬过 IPC 是纯粹的浪费。
    ///
    /// 需要给用户看的那一小部分（比如 obstructed 指名了哪几条）走错误信息，
    /// 不走这个字段。
    #[serde(skip)]
    pub dirty: Vec<StatusEntry>,
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
/// | 3.5 | 算出检查目标 | 白名单根 + 嵌套 external 根，见 [`check_targets`] |
/// | 4 | P1 清点脏条目，只拦 `~` | obstructed 时后面都不用跑了 |
/// | 5 | P2 nodeRevs 单值 | |
///
/// ⚠️ **P1 先于 P2 这条顺序曾经承载过一个不变量，现在不再承载了。** 旧版 P1 要求
/// 工作副本干净，顺带把「`svn add` 了没提交、带 `revision="-1"` 的节点」挡在 P2
/// 之外；spec §16.1 放松 P1 之后那层保护没了，改由 [`parse_node_revs`] 自己滤掉
/// 负 revision（未提交的节点没有服务器 revision，本就不该进 `nodeRevs`）。
///
/// 记在这里是因为顺序看起来没变、含义却变了：**现在把 P1 和 P2 对调也不会出错**，
/// 但没有理由去调。
///
/// [`parse_node_revs`]: crate::engine::svn_xml::parse_node_revs
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
    let targets = check_targets(&roots, &wcs);
    let dirty = collect_dirty(repo_root, &targets)?; // P1
    let node_rev = check_single_node_rev(repo_root, &targets, &wcs)?; // P2

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
        dirty,
    })
}

/// 只重读一次 `nodeRevs`，不做别的校验（spec §16 R3c）。
///
/// # 为什么需要单独一个入口
///
/// R2a 读脏清单与 R3b 跑体检之间隔着若干次子进程调用，期间用户可能在外部编辑器
/// 或 TortoiseSVN 里继续动。最危险的一种是**外部 commit**：那会把某个文件的 BASE
/// 推到 N+1，而 `svn cat -r BASE` 取回的就成了 N+1 的内容，trailer 却写着 rN。
///
/// 体检通常已经先一步拦下它（那个文件会进 `D`、不进 `S`），这里是第二道：
/// 纯本地、不走网络，代价近乎为零，而它挡的是一种**静默**的错误。
///
/// 不复用 [`run_preflight`] 是因为那会把 P1/P3/P4 也重跑一遍，失败时报出来的是
/// 「obstructed」或「操作未完成」，而真正发生的事是「录制期间工作副本被改了」——
/// 给错原因比不给更糟。
pub fn read_node_rev(repo_root: &str, whitelist: &[String]) -> Result<u64> {
    let roots = whitelist_roots(whitelist);
    let wcs = enumerate_wcs(repo_root, whitelist)?;
    let targets = check_targets(&roots, &wcs);
    check_single_node_rev(repo_root, &targets, &wcs)
}

/// 本流程对 svn 发命令时用的 target：白名单根，加上其中的嵌套工作副本根。
///
/// 与 [`read_node_rev`] 同一套（external 里的代码要显式指到，spec §6.5.3 / §6.6.5）。
/// 空白名单直接报错——空 target 会变成「整个工作副本」（spec §12.2）。
pub(crate) fn svn_targets(repo_root: &str, whitelist: &[String]) -> Result<Vec<String>> {
    let roots = whitelist_roots(whitelist);
    if roots.is_empty() {
        return Err(GitError::new(
            "svn_empty_whitelist",
            "白名单是空的，拒绝对整个工作副本发命令。",
        ));
    }
    let wcs = enumerate_wcs(repo_root, whitelist)?;
    Ok(check_targets(&roots, &wcs))
}

/// P1 / P2 的检查目标：白名单根，**外加白名单底下每一条嵌套 external 的根**。
///
/// # 为什么不能只查白名单根
///
/// P4 对**同仓库**的嵌套 external 按设计放行（spec §6.5.5：「同仓库的嵌套 external
/// 不阻断」），于是它是唯一能走完全程的形态。2026-09-15 实测（白名单 `Code/`，
/// `Code/Vendor` 是同仓库 external）：
///
/// | 关卡 | 只查白名单根时的实测结果 |
/// |---|---|
/// | P1 `status --no-ignore --ignore-externals -- Code@` | 只有一条 `item="external"`，里面的 `M` **看不见** |
/// | P2 `status -v --ignore-externals -- Code@` | `{3}`，external 停在 r1 这件事**不在集合里** |
/// | §6.8 体检 `diff --old=URL@3 --new=. -- Code` | `<paths>` 空的，diff **不下沉进 external** |
/// | `git add -- Code` | `Code/Vendor/V.cs` **照收** |
///
/// 四条叠起来：一个未提交、或停在别的 revision 的子树被录进基线、标着根的
/// revision，而体检报 `Verified`。spec §6.5.5 把这块叫「git 管着、SVN 侧却没人盯的
/// 盲区」，§6.6.5 的对策是「在**每个 `wc(P)`** 上各跑一次……取并集」——本函数就是
/// 那个「每个 `wc(P)`」。
///
/// `--ignore-externals` 仍然照加：它负责挡住**更深**一层的嵌套 external，而那一层
/// 由 [`enumerate_wcs`] 递归枚举出来、作为独立 target 各查一次（spec §6.6.5 末尾
/// 那条「在 `wc(P)` 内部用它是对的」）。
pub fn check_targets(roots: &[String], wcs: &[WcIdentity]) -> Vec<String> {
    let mut targets = roots.to_vec();
    for w in wcs {
        // `wc_root` 与 `roots` 都经过同一套归一（正斜杠 + 去尾斜杠），白名单根自己
        // 也在 `wcs` 里，所以这里同时完成「去重」与「只补嵌套 external」两件事。
        if !targets.iter().any(|t| t.eq_ignore_ascii_case(&w.wc_root)) {
            targets.push(w.wc_root.clone());
        }
    }
    targets
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

/// P1：收集白名单内所有**非 pristine** 的条目；只有 `~`（obstructed）阻断。
///
/// 三个参数缺一不可（spec §6.5）：
///
/// ```text
/// svn status --no-ignore --ignore-externals --xml -- <白名单根>
///            ↑ 不带它会漏掉 SVN-ignored 文件，而其中匹配扩展名的那些 git 照收
///                        ↑ 不带它 external 内部的节点会混进来（§6.6.4）
/// ```
///
/// # 为什么不再要求干净
///
/// 初版这里要求「一条脏都不许有」。那条门槛让重建基线在真实开发节奏下**永远
/// 点不动**：git 与 SVN 共用同一个工作目录，`git commit` 之后 `svn status` 仍然
/// 是 `M`，所以只要用户干过活白名单就永远脏，而失同步横幅偏偏一直在推荐它
/// （spec §16.1 有完整的复现记录）。
///
/// 修法是换掉**录制来源**，不是拆掉门槛：脏文件改取 `svn cat -r BASE` 的字节
/// （§16 R2a/R2b）。所以这个函数从「拦住」变成「如实清点」——它的输出是那一步的
/// 输入。放行本身不会让基线变松，真正的把关移到了体检的 `D \ S == ∅`（§6.8.7）。
///
/// # 唯一仍然阻断的是 `~`
///
/// obstructed 意味着 svn 自己都说不清那个位置上是什么（典型形态：一个版本控制下
/// 的文件被换成了同名目录）。取不到可信的 BASE 内容，就录不出「服务器在 rN 的
/// 样子」。**不自动修复**——revert 和删目录都是破坏性的，只拦住并引导去
/// TortoiseSVN 自己处理（对齐 spec §8.1 #2）。
///
/// # 清点故意不按扩展名过滤
///
/// 这一层是 `svn status` 的如实转述。按扩展名筛选是下游的事（R2a 只改写 W-file，
/// 纳入集合核对另有判据）。这一层多报的代价是零——下游会忽略它；少报的代价是
/// 漏掉一个该从基线里剔除的文件，而那种错误**看不见**。
///
/// `targets` 来自 [`check_targets`]：白名单根**加上**嵌套 external 的根。只传白名单
/// 根的话，嵌套 external 里的改动在这条命令下只显示成一条 `item="external"`，
/// 而下面正好把它跳过——等于完全没查。
fn collect_dirty(repo_root: &str, targets: &[String]) -> Result<Vec<StatusEntry>> {
    // 读取本身下沉到了 `svn_dirty::read_dirty`——服务器侧核对要用同一份清单，
    // 而它**不能**因为一处 obstructed 就让仓库打不开。所以「读」和「判」分开，
    // 阻断判定留在这里。
    let dirty = read_dirty(repo_root, targets)?;
    let mut obstructed: Vec<String> = dirty
        .iter()
        .filter(|e| e.item == "obstructed")
        .map(|e| e.path.clone())
        .collect();

    if !obstructed.is_empty() {
        obstructed.sort();
        let shown: Vec<&String> = obstructed.iter().take(20).collect();
        let more = if obstructed.len() > shown.len() {
            format!("\n……另有 {} 条未列出", obstructed.len() - shown.len())
        } else {
            String::new()
        };
        return Err(GitError::new(
            "svn_preflight_obstructed",
            format!(
                "白名单内有 {} 处 obstructed（svn status 的 `~`）：\n{}{}\n\
                 这种位置上 svn 自己都说不清是什么（典型情况：一个受版本控制的文件\
                 被换成了同名目录），取不到它在服务器上的内容，也就录不出基线。\n\
                 本应用不会替你动这些路径（删目录、revert 都是破坏性操作），\
                 请先在 TortoiseSVN 里处理——通常是把多出来的目录挪走、\
                 再对该路径跑一次 Update 或 Revert。\n\
                 ⚠️ 除了 `~`，其余的本地修改（改过、新增、删除、未版本控制）\
                 现在都**不再阻断**建库或重建基线。",
                obstructed.len(),
                shown
                    .iter()
                    .map(|s| format!("  - {s}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
                more
            ),
        ));
    }

    Ok(dirty)
}

/// P2：`nodeRevs` 必须是单元素集合 `{N}`。
///
/// 读法按 spec §6.6.2：带 `-v` 和 `--ignore-externals`，取 `<wc-status revision>`。
/// 这两条都由 [`parse_node_revs`] 自己把关，漏了会响亮报错而不是静默算错。
///
/// `targets` 来自 [`check_targets`]，对应 spec §6.6.5 的「在**每个 `wc(P)`** 上各跑
/// 一次……取并集」。被 `-r` 钉住的嵌套 external 正是靠这条并集才现形——它自己那份
/// revision 不会出现在父目录的 status 里。
fn check_single_node_rev(repo_root: &str, targets: &[String], wcs: &[WcIdentity]) -> Result<u64> {
    let mut all: BTreeSet<i64> = BTreeSet::new();
    for root in targets {
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
        // 负数来自 `svn add` 了没提交的节点（`revision="-1"`，2026-09-16 实测）。
        //
        // 旧版靠「P1 先拦住那种节点」来保证这里不会收到负数；spec §16.1 放松 P1
        // 之后那条保证没了，改由 `parse_node_revs` 自己把负数滤掉（它在语义上就
        // 不该进 `nodeRevs`——未提交的节点没有服务器 revision）。所以走到这里说明
        // 过滤被人删了，是实现缺陷，不是用户的工作副本有问题。
        GitError::new(
            "svn_preflight",
            format!(
                "读到一个负的 revision（{rev}）。未提交节点的 revision=\"-1\" \
                 本该在 parse_node_revs 里就被滤掉。"
            ),
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
