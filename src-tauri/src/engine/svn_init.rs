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
use crate::engine::svn_attributes::{has_eol_rule, write_attributes};
use crate::engine::svn_baseline::{publish_baseline, TempIndex, Trailer};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{
    normalize_whitelist_entry, write_config, SvnConfig, BASELINE_BRANCH, SVN_GIT_MODE,
};
use crate::engine::svn_dirty::{classify, explained_paths, read_dirty, Fix};
use crate::engine::svn_exclude::{generate_exclude, write_exclude};
use crate::engine::svn_externals::{detect_pinned, PinnedExternal};
use crate::engine::svn_health::{check_baseline, ExplainedOrigin, Health};
use crate::engine::svn_include_check::{normalize_roots, verify_include_set};
use crate::engine::svn_preflight::{check_targets, read_node_rev, run_preflight, Preflight};
use crate::engine::svn_pristine::{cat_base, BaseContent};
use crate::engine::svn_xml::parse_info;
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};
use std::collections::BTreeSet;

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

    // 工作副本根的 URL 在一次运行里不会变，取一次传下去
    // （配置要写它，体检要拿它当基准）。
    let svn_url = working_copy_url(root)?;
    let cfg = build_config(req, &svn_url)?;

    // ---- 前置校验：全部在创建 .git **之前**，失败时无需回退 ----
    let pre = run_preflight(root, &cfg.whitelist)?; // P1–P4
    let pinned = detect_pinned(root, &cfg.whitelist)?; // P5，命中不阻断

    // ⚠️ **进入回退区之前，原子地「创建并占有」`.git`。**
    claim_git_dir(root)?;

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

/// 录出「**服务器 rN 的树**」（spec §16 R2a–R2c）。返回 `(TREE, S)`。
///
/// `S` 是本次**实际动过**的路径集合，交给体检当 §6.8.7 的 `S` 用。
///
/// # 三步
///
/// ```text
/// R2a  按 svn status 把脏条目分成 取回BASE / 剔除 / 不用动
/// R2b  先 git add -A 录下磁盘，再逐条改写成服务器内容
/// R2c  核对文件集合 == 服务器 rN 的 W-file 集合
/// ```
///
/// # 为什么不直接从服务器逐个取
///
/// 干净文件本来就等于服务器内容（`svn status` 说它是 pristine），绕道 `svn cat`
/// 取一遍只是白花几万次子进程调用。真正需要换的只有脏的那几条，而那正是
/// `svn status` 列出来的那份清单。
///
/// # `S` 必须恰好
///
/// 只有真正调用了 `replace_blob` / `remove_path` 的路径才进 `S`。非 W-file 的脏
/// 条目（比如白名单目录里一个未版本控制的 `.txt`）在树里本来就不存在，对它动手
/// 是 no-op——但把它记进 `S` 会让体检的 `D \ S` 变小，判据被悄悄削弱而没有任何
/// 测试会变红（spec §16.1）。
fn record_server_tree(
    repo_root: &str,
    cfg: &SvnConfig,
    pre: &Preflight,
) -> Result<(String, BTreeSet<String>)> {
    let fixes = classify(&pre.dirty)?;
    // `S` 先算出来，再拿它驱动改写——**同一个集合**既决定动哪些路径，也交给体检。
    // 分两份算的话，两份迟早会不一致，而「体检那份更大」这个方向会悄悄削弱判据。
    let touched = explained_paths(&fixes, &cfg.whitelist, &cfg.extensions);

    let idx = TempIndex::empty(repo_root)?;
    idx.add_all(repo_root, &cfg.whitelist)?;

    for (path, fix) in &fixes {
        if !touched.contains(path) {
            // 不归 git 管，树里本来就没有它。见上面「`S` 必须恰好」。
            continue;
        }
        match fix {
            Fix::TakeBase => match cat_base(repo_root, path)? {
                BaseContent::Bytes(bytes) => idx.replace_blob(repo_root, path, &bytes)?,
                // 分类说「服务器上有」，服务器说「没有」。两边对不上时**停下**，
                // 不能自作主张把它当成 Drop 处理掉——那等于用一个猜测悄悄改变
                // 基线的内容集合。
                BaseContent::NotOnServer => {
                    return Err(GitError::new(
                        "svn_baseline_classification_conflict",
                        format!(
                            "`{path}` 按 svn status 是「服务器上有」（{fix:?}），\
                             但取它在服务器 r{} 上的内容时 svn 说这个文件还没提交过。\
                             两个判断对不上，停下来而不是猜一个。\
                             工作副本可能在录制过程中被外部改动了，请重试一次。",
                            pre.node_rev
                        ),
                    ))
                }
            },
            Fix::Drop => idx.remove_path(repo_root, path)?,
        }
    }

    // R2c：核对与录制**共用同一份候选索引**（spec §6.3.1），核对完之后在同一个
    // `idx` 上 `write_tree`——不得核对一次后又重新 add 另录一棵树。
    verify_include_set(repo_root, &idx, &cfg.whitelist, &cfg.extensions, &fixes)?;
    let tree = idx.write_tree(repo_root)?;
    Ok((tree, touched))
}

/// 原子地创建 `.git` 目录，作为「这次初始化拥有它」的凭据。
///
/// # 为什么不是「先判存在、再 `git init`」
///
/// 判断与创建是两次系统调用，中间的窗口足以让第二条请求也判出「不存在」。
/// 2026-09-15 在 fixture 上用两条线程实测，6 轮里命中 2 轮：
///
/// ```text
/// Err("git_cli / fatal: not a git repository …")
/// Err("git_cli / could not lock config file …/.git/config: File exists")
/// ```
///
/// ——两条请求**同时**在同一个 `.git` 上跑 `git init`，撞在 config 锁上双双失败，
/// 然后各自 `rollback`，把目录删回未初始化。更坏的交错是其中一条走完全程报了成功，
/// 另一条在 `publish_baseline` 的「该 ref 不该存在」前置条件上失败
/// （实测 `cannot lock ref 'refs/heads/svn/base': reference already exists`）
/// 并回退——`remove_dir_all` 删掉的就是**赢家刚建好的仓库**，而赢家那边界面上
/// 显示「初始化成功」。
///
/// `git init` 自己挡不住这件事：对已初始化的仓库它**以 exit 0 静默 re-init**
/// （实测还会 `warning: re-init: ignored --initial-branch=svn/base`，意味着
/// HEAD 会留在别人建库时的分支上）。
///
/// `std::fs::create_dir` 是一次系统调用，目标已存在时以 `AlreadyExists` 失败，
/// 「判断」与「占有」在其中不可分割。实测 `git init -b svn/base` 在一个**已存在的
/// 空 `.git` 目录**上照常工作（exit 0，HEAD 指向 `refs/heads/svn/base`），
/// 所以拿它当凭据不需要改动后面任何一步。
///
/// 这道判定必须留在**回退区之外**：放进 `build_repo` 的话，它自己的 `Err`
/// 就会触发 `rollback`，正好删掉那个刚发现的外来 `.git`。
///
/// 跨进程同样有效（用户自己在命令行跑 `git init`），因为它靠的是文件系统而不是
/// 进程内的锁。
fn claim_git_dir(root: &str) -> Result<()> {
    let git = std::path::Path::new(root).join(".git");
    match std::fs::create_dir(&git) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(GitError::new(
            "svn_unconfigured_git",
            "校验期间这个目录里出现了一个 .git（可能是另一个窗口、或者命令行建的）。\
             本应用不会碰它——请确认来源后再重试。",
        )),
        Err(e) => Err(GitError::new(
            "svn_init",
            format!("建不出 {} ：{e}", git.display()),
        )),
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
    // **必须在 `record_server_tree` 之前。** 这条规则决定 `git add` 与
    // `hash-object --path` 读磁盘时跑不跑行尾过滤器，也就决定了第一个基线里
    // blob 长什么样。放到录制之后等于建库当场就造出一个需要迁移的旧形状基线
    // ——而那种基线的故障是潜伏的：stat cache 会让 `git status` 一开始假报
    // 干净，等某个文件的 mtime 被碰一下才单独冒出来（spec §7.3.0）。
    write_attributes(root)?;

    // R2a–R2c：录出服务器 rN 的树。首个基线从**空**索引起步。
    let (tree, explained) = record_server_tree(root, cfg, pre)?;

    let trailer = Trailer {
        // Repository Root，不是工作副本 URL（spec §6.4.1）
        url: pre.repos_root.clone(),
        rev: pre.node_rev,
        uuid: pre.uuid.clone(),
    };
    // 首个基线**无父提交**；`expected_old = None` 表示「这个 ref 现在不该存在」，
    // 刚 `git init` 出来的仓库正是如此。
    let baseline = publish_baseline(root, &tree, None, &trailer, None)?;

    // 对齐真实索引（spec §6.3.1）。**不带 `-u`**：绝不能用基线内容去覆盖磁盘。
    //
    // 这一步不能省：此刻 HEAD 已经附着在 svn/base 上并解析到刚才那个 commit，
    // 而真实索引还是空的——不对齐的话 `git status` 会把每个 W-file 都显示成
    // 「已删除」，用户一打开就看到满屏红色。
    //
    // §16.1 之后这一步还多了一层意义：基线树是**服务器内容**，磁盘上是用户改过的
    // 内容，两者故意不同。对齐索引之后 `git status` 显示的恰好就是用户原有的那些
    // 本地 SVN 修改（`M` → 已修改、`?` → 未跟踪、`D` → 已删除），第一眼就是对的。
    // 带 `-u` 会把这些改动**抹掉**——那是本期最不能犯的错。
    run_git(root, &["read-tree", &tree])?;

    let health = check_baseline(
        root,
        svn_url,
        pre.node_rev,
        &cfg.whitelist,
        &cfg.extensions,
        &explained,
        // 刚刚由 R2b 从 `svn cat -r BASE` 写进去的，而 P2 保证 `BASE == node_rev`
        // ——这批路径由构造保证正确，不必报成「未覆盖」。
        ExplainedOrigin::JustRecorded,
    )?;
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

/// 行尾规则的安装状态（spec §7.3.0）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EolStatus {
    /// `.git/info/attributes` 里有没有我们写的那条规则。
    pub rule_installed: bool,
    /// 要不要走一次迁移。
    ///
    /// 等价于 `!rule_installed`——规则没装，说明这个仓库的基线是在旧的行尾设置
    /// 下录的。**判据只能是这个**：spec §7.3.0 实测，翻完设置之后 `git status`
    /// 会因为 git 的 stat cache 假报 0 个脏文件，要等某个文件的 mtime 被碰一下
    /// 才单独冒出来。拿「现在看着没事」当判据必然漏判。
    ///
    /// 两个字段值恒定互补，看起来冗余，但前端要分别用：一个决定横幅挂不挂，
    /// 一个用在文案里。留两个字段比让前端自己取反更难写错。
    pub needs_migration: bool,
}

/// 读出行尾规则的安装状态。**纯读，一个字节都不写。**
pub fn eol_status(repo_root: &str) -> Result<EolStatus> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库。",
        ));
    }
    let installed = has_eol_rule(repo_root);
    Ok(EolStatus {
        rule_installed: installed,
        needs_migration: !installed,
    })
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

    // R2a–R2c：**从空索引起步**，录出服务器 rN 的树。
    let (tree, explained) = record_server_tree(repo_root, &cfg, &pre)?;

    // R3b：**在 R4 之前**。判据是 §6.8.7 的 `D \ S == ∅`——磁盘与服务器的每一处
    // 差异都必须被一条已知的本地修改解释。指针还没动，中止的代价为零。
    let health = check_baseline(
        repo_root,
        &svn_url,
        pre.node_rev,
        &cfg.whitelist,
        &cfg.extensions,
        &explained,
        // 刚刚由 R2b 从 `svn cat -r BASE` 写进去的，而 P2 保证 `BASE == node_rev`
        // ——这批路径由构造保证正确，不必报成「未覆盖」。
        ExplainedOrigin::JustRecorded,
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

    // R3c：复查 `nodeRevs` 仍是 {N}。纯本地，不走网络。
    //
    // 防的是 R2a 到此刻之间有人在 TortoiseSVN 里 commit / update，把 BASE 推到了
    // N+1——那时 `svn cat -r BASE` 取回的是 N+1 的内容，而 trailer 写的是 rN
    // （spec §16.1 末的竞态表）。体检通常已先一步拦下，这里是第二道。
    let now = read_node_rev(repo_root, &cfg.whitelist)?;
    if now != pre.node_rev {
        return Err(GitError::new(
            "svn_rebuild_revision_moved",
            format!(
                "重建过程中工作副本被外部改动了：开始时白名单停在 r{}，现在是 r{now}。\
                 录下来的内容会对应错误的 revision，所以中止了——基线指针没有改动。\
                 请重新点一次「重建基线」。",
                pre.node_rev
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
///
/// # ⚠️ 必须先校验 `check_baseline` 的前提，不能直接调
///
/// `svn_health` 的模块头写着它的前提：**「磁盘上白名单内的内容 == 基线树的内容」**。
/// spec §6.8.5 列的另外三个时机天然满足（树刚从磁盘录出来），**这一个不满足**
/// ——用户就在 `work` 分支上改代码，那正是他会点这个按钮的时候。
///
/// 不校验前提的话，`check_baseline` 比的是「当前磁盘 vs 服务器」，而结论却被当成
/// 「基线 vs 服务器」来汇报：一个再普通不过的未提交改动会被报成 `Lying`，
/// 而基线完全正确。指控错对象比不指控更糟——用户会照着引导去**重建基线**，
/// 把一份好基线换掉，而他的改动还在原地。
///
/// 所以这里先把磁盘上的 W-file 录成一棵树，和基线树比：
///
/// - 一致 → 前提成立，`check_baseline` 的结论就是关于基线的，照常返回；
/// - 不一致 → 这条检查**分辨不出**「基线撒谎」和「我有未提交改动」，
///   按 spec §6.8.6 第三行返回 `NotChecked`。
///
/// 录树复用 `rebuild_baseline` 那套临时索引，全程不碰真实 index、HEAD 和磁盘。
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

    // 前提校验：磁盘上的 W-file 录成树，必须与基线树逐字节相同。
    // 用 `%T` 而不是 `<ref>^{tree}`——后者在若干 Windows shell 里要转义，
    // 而 `run_git` 是直接 spawn 进程、不过 shell，两种写法这里都能用，
    // 但 `%T` 不带任何需要转义的字符，少一处将来会被改错的地方。
    let baseline_tree = run_git(
        repo_root,
        &[
            "log",
            "-1",
            "--format=%T",
            &format!("refs/heads/{BASELINE_BRANCH}"),
        ],
    )?
    .trim()
    .to_string();
    // 这里曾经有一道门槛：「磁盘内容必须与基线树逐字节相同，否则不下结论」。
    //
    // **那道门槛和 §16.1 里那条「工作副本必须干净」是同一个病**：程序员的工作区
    // 99% 的时间都是脏的，于是这颗「体检基线」按钮永远只会回答「没法下结论」。
    // 换成 §6.8.7 的判据之后它重新变得可用，而且比原来**更强**——原来「基线树
    // 与磁盘不一致」一律落 NotChecked，现在解释不了的那些会被算成证据。
    //
    // 现在的 `S` 取「此刻 svn 说哪些 W-file 是脏的」。它成立的理由：基线树若录得
    // 正确就等于服务器 rN，而 `D \ S == ∅` 给出「磁盘在 S 之外等于服务器」，
    // 两者合起来即「基线树在 S 之外等于服务器」。
    //
    // ⚠️ **代价要说清楚：`S` 里的那些文件，这条检查什么也证明不了。** 它比的是
    // 磁盘与服务器，而那些路径上的磁盘内容本来就是用户改过的。工作副本干净时
    // `S = ∅`，覆盖面才是完整的（spec §6.8.7）。
    let wcs = crate::engine::svn_wc::enumerate_wcs(repo_root, &cfg.whitelist)?;
    let targets = check_targets(&normalize_roots(&cfg.whitelist), &wcs);
    let fixes = classify(&read_dirty(repo_root, &targets)?)?;
    let explained = explained_paths(&fixes, &cfg.whitelist, &cfg.extensions);

    // 基线树与磁盘树的差异里，**解释不了**的那些是独立的一份证据：它直接说明
    // 基线树本身就不对（老版本录错了、有人手工在 svn/base 上提交过），而拿磁盘
    // 去比服务器的那条路径看不见这种错误。
    let idx = TempIndex::empty(repo_root)?;
    idx.add_all(repo_root, &cfg.whitelist)?;
    let disk_tree = idx.write_tree(repo_root)?;
    let tree_evidence: Vec<String> = run_git(
        repo_root,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            &baseline_tree,
            &disk_tree,
        ],
    )?
    .split('\0')
    .filter(|s| !s.is_empty())
    .filter(|p| !explained.contains(*p))
    .map(str::to_string)
    .collect();

    let health = check_baseline(
        repo_root,
        &cfg.svn_url,
        trailer.rev,
        &cfg.whitelist,
        &cfg.extensions,
        &explained,
        // 手动体检：基线是**什么时候、哪个版本**录的都不知道，这条检查覆盖不到
        // `S` 里那些路径，必须如实报出去（2026-09-21 独立 review）。
        ExplainedOrigin::Unknown,
    )?;

    // 合并两份证据。`NotChecked` 原样透传——那是「没能验证」，不是「验证不通过」，
    // 把它降级成 Lying 会让离线场景寸步难行（spec §6.8.6）。
    //
    // ⚠️ `unverified` 要原样带过去。第二份证据（基线树 vs 磁盘树）**同样**把
    // `explained` 滤掉了，所以两份证据合起来仍然对那批路径一无所知。
    Ok(match health {
        Health::NotChecked { reason } => Health::NotChecked { reason },
        Health::Verified { rev, unverified } if tree_evidence.is_empty() => {
            Health::Verified { rev, unverified }
        }
        Health::Verified { .. } => Health::Lying {
            files: tree_evidence,
        },
        Health::Lying { mut files } => {
            files.extend(tree_evidence);
            files.sort();
            files.dedup();
            Health::Lying { files }
        }
    })
}

/// 当前仓库里被钉死、且与白名单有交集的 external（spec §6.5.3 的常驻说明用它）。
///
/// 不是混合仓库时返回**空列表**而不是报错：调用方是「打开仓库」那条热路径，
/// 普通 git 仓库走到这里是正常的。
pub fn pinned_externals(repo_root: &str) -> Result<Vec<PinnedExternal>> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Ok(Vec::new());
    }
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    detect_pinned(repo_root, &cfg.whitelist)
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
/// 调用点唯一：[`init_hybrid`] 在 `build_repo` 返回 `Err` 时。而在那之前
/// [`claim_git_dir`] 已经**原子地创建**了这个目录——`create_dir` 成功即证明它
/// 在那一刻不存在、且是我们建的。这比「开头 `detect_repo_kind` 断言过没有 `.git`」
/// 强：后者与 `git init` 之间隔着整个前置校验，那段时间里别人可以插进来建一个
/// （实测会发生，见 [`claim_git_dir`]）。
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
fn build_config(req: &InitRequest, svn_url: &str) -> Result<SvnConfig> {
    let mut whitelist = Vec::with_capacity(req.whitelist.len());
    for entry in &req.whitelist {
        whitelist.push(normalize_whitelist_entry(entry)?);
    }
    let cfg = SvnConfig {
        version: 1,
        mode: SVN_GIT_MODE.to_string(),
        svn_url: svn_url.to_string(),
        whitelist,
        extensions: req.extensions.clone(),
        baseline_branch: BASELINE_BRANCH.to_string(),
    };
    crate::engine::svn_config::validate(&cfg)?;
    // ⚠️ 这一条**不能省**。`SvnGitBackend::open` 每次打开混合仓库都会跑它，
    // 而这里是唯一**写入**配置的地方——两边不接上的话，向导能建出一个自己
    // 打不开的仓库：
    //
    //   用户把 `Assets/Scripts/` 写成 `Assets/Scripts`（漏尾斜杠，是很自然的手误），
    //   而该目录下的文件扩展名恰好全在 extensions 里 → 期望集合与候选索引相等 →
    //   纳入集合核对通过 → 初始化成功 → 配置落盘 → 下一次 open_repo 被
    //   `validate_whitelist_against_disk` 拒掉。此刻目录已经是 SvnHybrid，
    //   向导那条分支只认 `svn_not_initialized`，用户在应用内**没有任何出路**。
    //
    // 放在 `git init` 之前，属于「失败无需回退」那一段。
    crate::engine::svn_config::validate_whitelist_against_disk(&req.repo_root, &cfg.whitelist)?;
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
