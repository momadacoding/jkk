//! 初始化向导的编排与失败回退（spec §6.5），以及重建基线 R1–R5（spec §16）。
//!
//! 两者放同一个文件，是因为它们共用同一段录制序列（临时索引 → `write-tree` →
//! `commit-tree` → 体检 → `update-ref`）。拆开会让这段最不能写错的代码被抄两遍。
//!
//! # 回退是这里的核心，不是收尾
//!
//! 初始化会在用户的 SVN 工作副本里**创建 `.git`**。任何一步失败都必须把它删干净，
//! 否则用户下次打开这个目录时会看到一个半成品混合仓库——而 §6.1 的识别会把它判成
//! `SvnSuspicious` 并阻断，用户完全不知道发生了什么。

use crate::engine::git_cli::run_git;
use crate::engine::svn_baseline::{publish_baseline, TempIndex, Trailer};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{
    normalize_whitelist_entry, write_config, SvnConfig, BASELINE_BRANCH, SVN_GIT_MODE,
};
use crate::engine::svn_exclude::{generate_exclude, write_exclude};
use crate::engine::svn_externals::{detect_pinned, PinnedExternal};
use crate::engine::svn_health::{check_baseline, Health};
use crate::engine::svn_include_check::verify_include_set;
use crate::engine::svn_preflight::{run_preflight, Preflight};
use crate::engine::svn_xml::parse_info;
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};

/// 初始化完成后立刻切上去的工作分支。UI 禁止在基线分支上开发提交（spec §6.4）。
const WORK_BRANCH: &str = "work";

pub struct InitRequest {
    pub repo_root: String,
    pub whitelist: Vec<String>,
    pub extensions: Vec<String>,
}

/// 初始化的结果。**`health` 与 `pinned` 都不是失败**，是要在 UI 上如实显示的状态。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitReport {
    /// 记账单位 N，取自 P2 读到的 `nodeRevs`。
    pub node_rev: u64,
    /// 首个基线 commit 的 OID。
    pub baseline: String,
    /// 体检结果。`NotChecked` **不阻断**，但必须显示成「基线未经体检」
    /// （spec §6.8.6 第三行）。
    pub health: Health,
    /// P5 命中的被钉死 external。非空即进入 pinned 模式（spec §6.5.3），
    /// 同样不阻断。
    pub pinned: Vec<PinnedExternal>,
}

/// 走完整个初始化向导。任何一步失败都会把本次创建的 `.git` 删干净。
///
/// 序列逐字照 spec §6.5：
///
/// ```text
/// P1–P5                      ← P1–P4 硬门槛，P5 命中只切 pinned 模式
/// git init -b svn/base       ← 显式指定，不吃 init.defaultBranch 的默认值
/// 写入 .git/mygitclient.json
/// 生成 .git/info/exclude
/// 纳入集合核对（§6.3.1）      ← 临时索引，期望集合 vs ls-files，得到 TREE
/// 用该 TREE 录首个基线        ← rev 取自 P2 的 N；无父提交
/// git read-tree TREE         ← 对齐真实索引，**不带 -u**、不改磁盘
/// 基线体检（§6.8）            ← Lying 则整个回退
/// git checkout -b work       ← 立刻离开基线分支
/// ```
pub fn init_hybrid(req: &InitRequest) -> Result<InitReport> {
    let root = req.repo_root.as_str();

    // ⚠️ 这里是全项目唯一一处会 `remove_dir_all` 的代码，所以**自己再判一次**，
    // 不依赖上游调用点判对了。`backend_for` 在 1a 已经拦过一道，但那一道挡不住
    // 直接调用本函数的新入口。
    match detect_repo_kind(root) {
        RepoKind::SvnUninitialized => {}
        RepoKind::SvnHybrid => {
            return Err(GitError::new(
                "svn_already_initialized",
                "这个目录已经是混合仓库了（.git 与配置文件都在）。\
                 要重新建库请先自行处理已有的 .git。",
            ))
        }
        RepoKind::SvnSuspicious => {
            return Err(GitError::new(
                "svn_unconfigured_git",
                "这个 SVN 工作副本里已经有一个 .git，但没有本应用的配置文件。\
                 它可能是别人建的、或者上一次初始化留下的残骸。\
                 本应用不会碰它——请自行确认后处理。",
            ))
        }
        RepoKind::Git => {
            return Err(GitError::new(
                "svn_not_a_working_copy",
                "这个目录里没有 .svn，不是 SVN 工作副本。",
            ))
        }
    }

    let cfg = build_config(req)?;

    // ---- 前置校验：全部在创建 .git **之前**，失败时无需回退 ----
    let pre = run_preflight(root, &cfg.whitelist)?; // P1–P4
    let pinned = detect_pinned(root, &cfg.whitelist)?; // P5，命中不阻断
    let svn_url = working_copy_url(root)?;

    // ---- 从这里开始，失败必须删掉本次创建的 .git ----
    match build_repo(root, &cfg, &pre, &svn_url) {
        Ok((baseline, health)) => Ok(InitReport {
            node_rev: pre.node_rev,
            baseline,
            health,
            pinned,
        }),
        Err(e) => {
            rollback(root);
            Err(e)
        }
    }
}

/// `git init` 之后的全部步骤。抽出来是为了让**唯一的**回退点长在一个地方——
/// 中间任何一步 `?` 早退都走同一条 `Err` 分支，不会漏掉某条路径忘了回退。
fn build_repo(
    root: &str,
    cfg: &SvnConfig,
    pre: &Preflight,
    svn_url: &str,
) -> Result<(String, Health)> {
    // `-b svn/base` 必须显式给：不给的话吃 `init.defaultBranch`，用户配成什么
    // 都有可能，而后面每一处都按 `svn/base` 这个名字找基线。
    run_git(root, &["init", "-b", BASELINE_BRANCH])?;
    write_config(root, cfg)?;
    write_exclude(root, &generate_exclude(&cfg.whitelist, &cfg.extensions))?;

    // 纳入集合核对与录制**共用同一个候选索引**（spec §6.3.1）。
    // 首个基线从**空**索引起步。
    let idx = TempIndex::empty(root)?;
    idx.add_all(root, &cfg.whitelist)?;
    verify_include_set(root, &idx, &cfg.whitelist, &cfg.extensions)?;
    let tree = idx.write_tree(root)?;

    let trailer = Trailer {
        // Repository Root，不是工作副本 URL（spec §6.4.1）
        url: pre.repos_root.clone(),
        rev: pre.node_rev,
        uuid: pre.uuid.clone(),
    };
    // 首个基线**无父提交**；`expected_old = None` 表示「这个 ref 现在不该存在」，
    // 刚 `git init` 出来的仓库正是如此。
    let baseline = publish_baseline(root, &tree, None, &trailer, None)?;

    // 对齐真实索引（spec §6.3.1）。**不带 `-u`**：磁盘上已经是这份内容了，
    // 带上只会多一次无谓的写盘，还可能改动文件时间戳。
    //
    // 这一步不能省：此刻 HEAD 已经附着在 svn/base 上并解析到刚才那个 commit，
    // 而真实索引还是空的——不对齐的话 `git status` 会把每个 W-file 都显示成
    // 「已删除」，用户一打开就看到满屏红色。
    run_git(root, &["read-tree", &tree])?;

    let health = check_baseline(root, svn_url, pre.node_rev, &cfg.whitelist, &cfg.extensions)?;
    if let Health::Lying { files } = &health {
        // spec §6.8.5：此刻还没建工作分支，整个回退代价最小。
        return Err(GitError::new(
            "svn_baseline_lying",
            format!(
                "基线体检没通过：磁盘内容与服务器上的 r{} 不一致，共 {} 处。\
                 继续下去会得到一个从一开始就在撒谎的基线，所以整个回退了。\n{}",
                pre.node_rev,
                files.len(),
                files
                    .iter()
                    .take(20)
                    .map(|f| format!("  - {f}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ));
    }

    // 立刻离开基线分支（spec §6.5 最后一步）。
    run_git(root, &["checkout", "-b", WORK_BRANCH])?;
    Ok((baseline, health))
}

/// 重建基线的结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RebuildReport {
    /// 新的记账单位 N。
    pub node_rev: u64,
    /// 重建前基线指向谁。
    pub old_baseline: String,
    /// 重建后指向谁。同一棵 tree 也会产出新的 commit（spec §6.3.1）。
    pub new_baseline: String,
    pub health: Health,
}

/// 重建基线 R1–R5（spec §16 Phase 1）。
///
/// ```text
/// R1  校验：白名单内 svn status 干净、nodeRevs 单值 {N}
/// R2  临时索引录 tree，全程不碰真实 index、不碰 HEAD
/// R3  git commit-tree TREE -p svn/base …            → NEWBASE
/// R3b 基线体检（§6.8）                                ← 有差异就中止，指针还没动
/// R4  git update-ref refs/heads/svn/base NEWBASE     ← 只动指针
/// R5  删掉临时索引文件                                 ← TempIndex 的 Drop
/// ```
///
/// # ⚠️ 绝对不能先 `git checkout svn/base`
///
/// spec §16 专门写了一段。§9.4 的恢复流程确实先 checkout 旧基线，但它后面**紧跟着
/// `svn revert`**，由 revert 把服务器内容重新铺回磁盘。这里**没有 revert**
/// （前提就是工作副本干净），先 checkout 会把磁盘**换回旧基线内容**，
/// 随后录下来的就是旧内容却标着新 revision ——**正好造出这一节要防的那种撒谎基线**。
///
/// 全程不动 HEAD 还有两个附带好处：工作分支上有未提交改动也不受影响，
/// 也不存在「流程结束忘了切回原分支」。
///
/// # R2 从**空索引**起步，不是从 HEAD 树
///
/// spec §6.3.1 把「初始化 / Phase 1 R2」列为从空索引起步的两个入口，这里照办。
/// 重建要录的是「磁盘上现在的 W-file」；从旧基线树或 HEAD 树起步会把那棵树里
/// 已经不在磁盘上的文件一起带进来——而甩掉它们正是重建的目的之一
/// （服务器删过文件的场景，spec §9.4.1）。
pub fn rebuild_baseline(repo_root: &str) -> Result<RebuildReport> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库，没有基线可重建。",
        ));
    }
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let old_baseline = read_baseline_oid(repo_root)?;

    // R1：与 P1/P2 同一套判据，复用 run_preflight（它顺带把 P3/P4 也跑了，
    // 那两条在重建时同样必须成立——工作副本半路挂着没跑完的 update，
    // 录出来就是一棵残缺的树）。
    let pre = run_preflight(repo_root, &cfg.whitelist)?;
    let svn_url = working_copy_url(repo_root)?;

    // R2：**从空索引起步**，录磁盘上现在的 W-file。核对与录制共用同一份索引。
    let idx = TempIndex::empty(repo_root)?;
    idx.add_all(repo_root, &cfg.whitelist)?;
    verify_include_set(repo_root, &idx, &cfg.whitelist, &cfg.extensions)?;
    let tree = idx.write_tree(repo_root)?;

    // R3b：**在 R4 之前**。此刻 TREE 就是磁盘内容，体检的前提成立；
    // 而指针还没动，中止的代价为零。
    let health = check_baseline(
        repo_root,
        &svn_url,
        pre.node_rev,
        &cfg.whitelist,
        &cfg.extensions,
    )?;
    if let Health::Lying { files } = &health {
        return Err(GitError::new(
            "svn_baseline_lying",
            format!(
                "基线体检没通过：磁盘内容与服务器上的 r{} 不一致，共 {} 处。\
                 基线指针没有改动。\n{}",
                pre.node_rev,
                files.len(),
                files
                    .iter()
                    .take(20)
                    .map(|f| format!("  - {f}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ));
    }

    // R3 + R4：建 commit 并以旧 OID 作比较条件更新指针。
    let trailer = Trailer {
        url: pre.repos_root.clone(),
        rev: pre.node_rev,
        uuid: pre.uuid.clone(),
    };
    let new_baseline = publish_baseline(
        repo_root,
        &tree,
        Some(&old_baseline),
        &trailer,
        Some(&old_baseline),
    )?;

    // R5 由 `idx` 的 Drop 负责。
    Ok(RebuildReport {
        node_rev: pre.node_rev,
        old_baseline,
        new_baseline,
        health,
    })
}

/// 体检当前基线——**rev 取自基线自己的 trailer**，不是 `nodeRevs`。
///
/// 这是 spec §6.8.5 那张表里「用户手动点体检」那一行，也是**唯一一个
/// `Health::Lying` 真正可达的入口**：
///
/// `init_hybrid` 与 `rebuild_baseline` 的 rev 都取自 `nodeRevs`，而 P1–P3 已经
///保证了工作副本与那个 revision 一致，于是体检在那两处结构上必然通过。
/// 这里读的是基线**自称**的 revision——它可能早就过时了（有人在项目根跑了递归
/// update 把 external 退回 peg、恢复流程没走完、或者基线本来就录错了），
/// 这正是 §6.8.1 说的「让基线可被服务器证伪」。
pub fn check_current_baseline(repo_root: &str) -> Result<Health> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库，没有基线可体检。",
        ));
    }
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let msg = run_git(
        repo_root,
        &[
            "log",
            "-1",
            "--format=%B",
            &format!("refs/heads/{BASELINE_BRANCH}"),
        ],
    )?;
    let trailer = crate::engine::svn_baseline::parse_trailer(&msg).ok_or_else(|| {
        // 「解析失败即视为该 commit 不是 baseline」（spec §6.4.1，不猜、不部分接受）。
        GitError::new(
            "svn_baseline_no_trailer",
            "基线分支最新的 commit 上没有可解析的 git-svn-id trailer，\
             它不是本应用录出来的基线。体检无从谈起——请重建基线。",
        )
    })?;
    check_baseline(
        repo_root,
        &cfg.svn_url,
        trailer.rev,
        &cfg.whitelist,
        &cfg.extensions,
    )
}

fn read_baseline_oid(repo_root: &str) -> Result<String> {
    let out = run_git(
        repo_root,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{BASELINE_BRANCH}"),
        ],
    )?;
    Ok(out.trim().to_string())
}

/// 删掉本次创建的 `.git`。
///
/// # 为什么这里可以 `remove_dir_all`
///
/// 调用点唯一：[`init_hybrid`] 在 `build_repo` 返回 `Err` 时。而 `init_hybrid`
/// 开头已经断言过 `detect_repo_kind == SvnUninitialized`，也就是**进来时没有
/// `.git`**——现在这个必然是本次 `git init` 建的。
///
/// 再加两道与那个判定无关的独立守卫，防止将来有人把这个函数接到别处：
/// 目标必须是目录（worktree 里 `.git` 是文件，那种不该出现在这里），
/// 且同级必须有 `.svn`（证明我们确实在一个 SVN 工作副本里，而不是别的什么地方）。
///
/// 删不掉不 panic、也不覆盖原始错误：用户看到的应当是「为什么失败」，
/// 而不是「回退时又失败了」。残留的 `.git` 会被 §6.1 识别成 `SvnSuspicious`
/// 并阻断，不会造成数据损坏。
fn rollback(root: &str) {
    let base = std::path::Path::new(root);
    let git = base.join(".git");
    if !base.join(".svn").is_dir() || !git.is_dir() {
        return;
    }
    let _ = std::fs::remove_dir_all(&git);
}

/// 按请求组装并校验配置。规范化必须在校验之前（与 `read_config` 同序）。
fn build_config(req: &InitRequest) -> Result<SvnConfig> {
    let mut whitelist = Vec::with_capacity(req.whitelist.len());
    for entry in &req.whitelist {
        whitelist.push(normalize_whitelist_entry(entry)?);
    }
    let cfg = SvnConfig {
        version: 1,
        mode: SVN_GIT_MODE.to_string(),
        svn_url: working_copy_url(&req.repo_root)?,
        whitelist,
        extensions: req.extensions.clone(),
        baseline_branch: BASELINE_BRANCH.to_string(),
    };
    crate::engine::svn_config::validate(&cfg)?;
    Ok(cfg)
}

/// 工作副本**根**的 URL。
///
/// 体检的 `--old=<svnUrl>@<rev>` 与 `--new=.` 是一对，所以这里必须是根的 URL，
/// 不是某个白名单目录的（`svn_health::check_baseline` 会自己再核对一次）。
fn working_copy_url(repo_root: &str) -> Result<String> {
    let xml = run_svn(repo_root, &["info", "--xml"])?;
    parse_info(&xml)?
        .into_iter()
        .next()
        .map(|e| e.url)
        .ok_or_else(|| GitError::new("svn_init", "svn info 对工作副本根没有输出"))
}
