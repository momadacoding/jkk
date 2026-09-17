//! §16 R2a：把 `svn status` 的条目分成「录基线时要怎么处理」。
//!
//! # 这一层为什么单独存在
//!
//! 基线的语义是「白名单内的文件在 revision N 时**服务器上的样子**」（§6.4）。
//! 工作副本干净时磁盘就等于那个样子；放松了「必须干净」之后（§16.1），每个脏
//! 条目都要单独回答一个问题：**服务器 rN 上这个位置是什么？**
//!
//! | 磁盘上的状态 | 服务器 rN 上是什么 | 怎么办 |
//! |---|---|---|
//! | 改过 / 替换过 / 冲突中 | 一份旧内容 | 取回 BASE |
//! | 本地删了 / 文件丢了 | 还在 | 取回 BASE |
//! | 本地新增（已 add 或未版本控制） | **没有** | 从树里剔除 |
//! | 只改了属性 | 同一份内容 | 不用动 |
//!
//! 判错一条就是一个假基线，而假基线看起来完全正常——这是整套设计里最贵的那类
//! 错误（§6.8.1）。所以这里对没见过的 item **报错而不是跳过**。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_include_check::{is_w_file, normalize_roots};
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_status_entries, StatusEntry};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};

/// 读出 `targets` 覆盖范围内所有**非 pristine** 的条目。**不做任何阻断判定。**
///
/// 三个参数缺一不可（spec §6.5）：
///
/// ```text
/// svn status --no-ignore --ignore-externals --xml -- <target>
///            ↑ 不带它会漏掉 SVN-ignored 文件，而其中匹配扩展名的那些 git 照收
///                        ↑ 不带它 external 内部的节点会混进来（§6.6.4）
/// ```
///
/// `targets` 必须是白名单根**加上**其底下每一条嵌套 external 的根。只传白名单根
/// 的话，external 里的改动在这条命令下只显示成一条 `item="external"`，而下面正好
/// 把它跳过——等于完全没查（spec §6.5.5 / §6.6.5）。
///
/// # 为什么读与判分开
///
/// 两个调用方要的东西不一样：初始化 / 重建要在 `~` 上**阻断**，而打开仓库时的
/// 服务器侧核对（[`crate::engine::svn_server_set`]）只是想知道哪些文件是本地删的，
/// 不该因为一处 obstructed 就让仓库打不开。判定留给各自的调用点。
pub fn read_dirty(repo_root: &str, targets: &[String]) -> Result<Vec<StatusEntry>> {
    let mut dirty: Vec<StatusEntry> = Vec::new();
    for target in targets {
        let arg = peg_safe(target);
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
            // `item="external"` 即使带了 `--ignore-externals` 也照样出现（实测），
            // 它不是脏状态。本设计的核心场景就是「代码在 external 里」，把它判脏
            // 会让向导在目标仓库上永远过不去。
            if e.item == "external" {
                continue;
            }
            // `normal` + `props == "none"` 才是真干净。只改了属性的节点是
            // `item="normal" props="modified"`（2026-09-14 实测），它**要**进清单：
            // 本函数的定义是「非 pristine」。[`classify`] 再把它判成「不用动」。
            if e.item == "normal" && e.props == "none" {
                continue;
            }
            dirty.push(e);
        }
    }
    // 同一条路径不该来自两个 target（`--ignore-externals` 让白名单根那次查询不会
    // 下沉进 external），但去重的代价近乎为零，而重复会让 `S` 多出条目。
    dirty.sort_by(|a, b| a.path.cmp(&b.path));
    dirty.dedup_by(|a, b| a.path == b.path);
    Ok(dirty)
}

/// 把分类结果收敛成体检判据里的 `S`（spec §6.8.7）。
///
/// 只保留 **W-file**：不归 git 管的路径（白名单目录里一个未版本控制的 `.txt`、
/// 一个改脏的 `.psd`）在基线树里本来就不存在，录制时对它们动手是 no-op。
/// 把它们记进 `S` 不会让任何东西变对，却会让 `D \ S` 变小——判据被悄悄削弱，
/// 而不会有任何测试因此变红（§16.1「S 必须恰好」）。
///
/// 抽成一个函数是因为**三个地方都要算它**：录制时（决定改写哪些路径）、录制后
/// 的体检、以及用户手动点体检。三份各写一遍，迟早有一份会和另外两份不一致，
/// 而不一致的方向若是「多算了」，判据就被削弱了且没有信号。
pub fn explained_paths(
    fixes: &BTreeMap<String, Fix>,
    whitelist: &[String],
    extensions: &[String],
) -> BTreeSet<String> {
    let roots = normalize_roots(whitelist);
    fixes
        .keys()
        .filter(|p| is_w_file(p, &roots, extensions))
        .cloned()
        .collect()
}

/// 录基线时要对这个路径做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// 把树里的内容换成 `svn cat -r BASE` 的字节。
    TakeBase,
    /// 从树里摘掉——服务器 rN 上没有这个文件。
    Drop,
}

/// 把脏清单分类。返回的**只有需要动的路径**，干净条目不出现。
///
/// 返回值同时就是体检判据里的 `S`（§6.8.7），所以它必须**恰好**是会被改写的那批
/// 路径：多一条，`D \ S` 就小一分，判据被悄悄削弱，而不会有任何测试变红。
/// `BTreeMap` 保证有序且去重。
pub fn classify(entries: &[StatusEntry]) -> Result<BTreeMap<String, Fix>> {
    let mut out: BTreeMap<String, Fix> = BTreeMap::new();

    for e in entries {
        let path = e.path.replace('\\', "/");
        if path.trim().is_empty() {
            return Err(GitError::new(
                "svn_dirty_empty_path",
                "脏清单里出现了空路径。它会变成一条空 pathspec 的 git 调用，\
                 而本仓库已经因为空 pathspec 误伤过一次（AGENTS.md §6.2）。",
            ));
        }

        let fix = match e.item.as_str() {
            // 磁盘上的不是服务器内容。`deleted` / `missing` 是磁盘上干脆没有——
            // 它们仍然要取回来，因为基线记的是**服务器**上的样子，本地的删除是
            // 一处待交付的改动，不是基线的一部分。
            "modified" | "replaced" | "conflicted" | "deleted" | "missing" => Fix::TakeBase,

            // 服务器 rN 上根本没有它们。`ignored` 与 `unversioned` 都算：
            // 被 `svn:ignore` 挡住不代表 git 不收它（正是 `--no-ignore` 存在的
            // 理由，§6.5 P1），而 git 收了它就会进基线树。
            "added" | "unversioned" | "ignored" => Fix::Drop,

            // 干净，或者只改了属性（`item="normal" props="modified"`）。
            //
            // 属性变更**故意**落在这里：内容等于服务器内容，换不换字节都一样，
            // 而让它进结果就等于让它进 `S`，判据随之被削弱（§16.1「S 必须恰好」）。
            "normal" => continue,

            // `--ignore-externals` 也拦不住这条标记（实测）。它不是脏状态，
            // external 内部的节点由 `check_targets` 作为独立 target 各查一次。
            "external" => continue,

            // P1 在 §16.1 之后唯一仍然阻断的状态，不该走到这一层。
            "obstructed" => {
                return Err(GitError::new(
                    "svn_dirty_obstructed",
                    format!(
                        "`{path}` 是 obstructed，本该在 P1 就被拦下。\
                         这个位置上 svn 自己都说不清是什么，取不到它在服务器上的内容。"
                    ),
                ))
            }

            other => {
                return Err(GitError::new(
                    "svn_dirty_unknown_item",
                    format!(
                        "不认识的 svn status item：`{other}`（路径 `{path}`）。\
                         默认当成「不用动」会把这个文件按**磁盘内容**录进基线，\
                         而基线声称自己是服务器 rN 的内容——那正是本期要消灭的\
                         那种假基线，且没有任何信号。所以这里停下来。"
                    ),
                ))
            }
        };

        out.insert(path, fix);
    }

    Ok(out)
}
