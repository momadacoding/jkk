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

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_attributes::{has_eol_rule, restore_attributes, write_attributes};
use crate::engine::svn_baseline::{
    git_dir, publish_baseline, swap_index_to_tree, TempIndex, Trailer,
};
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{
    normalize_whitelist_entry, write_config, SvnConfig, BASELINE_BRANCH, SVN_GIT_MODE,
};
use crate::engine::svn_dirty::{classify, explained_paths, read_dirty, Fix};
use crate::engine::svn_exclude::{generate_exclude, write_exclude};
use crate::engine::svn_externals::{detect_pinned, PinnedExternal};
use crate::engine::svn_health::{check_baseline, ExplainedOrigin, Health};
use crate::engine::svn_include_check::{dry_run_include_set, normalize_roots, verify_include_set};
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
    /// 各阶段耗时，按执行顺序。失败时同样的内容附在错误正文末尾。
    pub timings: Vec<PhaseTiming>,
}

/// 一个阶段花了多久。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseTiming {
    pub phase: String,
    pub millis: u64,
}

/// 建库各阶段的计时器。
///
/// # 为什么耗时要进 UI，而不只是写日志
///
/// Rust 日志插件只在 debug 构建启用（`lib.rs`），装机版里 `log::info!` 不落地。
/// 2026-10-27 用户在公司装机版上反馈「确认并建库」要半小时，却没法知道花在哪一步。
/// 所以成功时进 [`InitReport::timings`]，失败时由 [`PhaseClock::annotate`] 附在
/// 错误正文末尾；`log::info!` 只是开发时顺带看的。
#[derive(Default)]
struct PhaseClock {
    phases: Vec<PhaseTiming>,
    /// 最后一个失败的阶段在 `phases` 里的下标。
    failed: Option<usize>,
}

impl PhaseClock {
    /// 计时跑一段。**失败也记**：最该看耗时的恰恰是「跑了很久然后失败」的那一步。
    fn run<T>(&mut self, phase: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
        let started = std::time::Instant::now();
        let out = f();
        let millis = started.elapsed().as_millis() as u64;
        log::info!("建库阶段「{phase}」耗时 {millis} ms");
        if out.is_err() {
            self.failed = Some(self.phases.len());
        }
        self.phases.push(PhaseTiming {
            phase: phase.to_string(),
            millis,
        });
        out
    }

    /// 把耗时附在错误正文末尾。一个阶段都没跑过时原样返回。
    fn annotate(&self, mut e: GitError) -> GitError {
        if self.phases.is_empty() {
            return e;
        }
        let total: u64 = self.phases.iter().map(|p| p.millis).sum();
        e.message.push_str(&format!(
            "\n\n各阶段耗时（共 {}）：\n",
            human_duration(total)
        ));
        for (i, p) in self.phases.iter().enumerate() {
            let mark = if self.failed == Some(i) {
                "   ← 在这一步失败"
            } else {
                ""
            };
            e.message.push_str(&format!(
                "  {}  {}{mark}\n",
                p.phase,
                human_duration(p.millis)
            ));
        }
        e
    }
}

/// `1234` → `1.2 秒`，`95000` → `1 分 35 秒`。与前端 `formatDuration` 逐字同一口径。
///
/// 只用整数运算、四舍五入：`{:.1}` 对 `1.25` 按银行家舍入给 `1.2`，而前端
/// `toFixed(1)` 给 `1.3`，两处会对不上。档位按取整之后的值切，否则 59.95 秒会
/// 显示成「60.0 秒」。
fn human_duration(millis: u64) -> String {
    let tenths = (millis + 50) / 100;
    if tenths < 600 {
        format!("{}.{} 秒", tenths / 10, tenths % 10)
    } else {
        let secs = (millis + 500) / 1000;
        format!("{} 分 {} 秒", secs / 60, secs % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::human_duration;

    /// 与 `tests/vitest/svnInit.test.ts` 的「耗时的显示」逐条对应——两侧必须给出同样的字。
    #[test]
    fn human_duration_matches_the_frontend_word_for_word() {
        for (ms, want) in [
            (0, "0.0 秒"),
            (1234, "1.2 秒"),
            (1249, "1.2 秒"),
            (1250, "1.3 秒"),
            (59_949, "59.9 秒"),
            (59_950, "1 分 0 秒"),
            (60_000, "1 分 0 秒"),
            (89_500, "1 分 30 秒"),
            (95_000, "1 分 35 秒"),
            (31 * 60_000 + 12_400, "31 分 12 秒"),
        ] {
            assert_eq!(human_duration(ms), want, "millis = {ms}");
        }
    }
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
/// 纳入集合预检               ← git add 之前只读地问「将会收什么」，不一致即失败
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

    let mut clock = PhaseClock::default();
    match init_phases(req, root, &mut clock) {
        Ok(mut report) => {
            report.timings = clock.phases;
            Ok(report)
        }
        Err(e) => Err(clock.annotate(e)),
    }
}

/// [`init_hybrid`] 判完目录类型之后的全部步骤。抽出来是为了让耗时在**任何**
/// `?` 早退时都能附到错误上，不必在每个返回点各补一次。
fn init_phases(req: &InitRequest, root: &str, clock: &mut PhaseClock) -> Result<InitReport> {
    // 工作副本根的 URL 在一次运行里不会变，取一次传下去
    // （配置要写它，体检要拿它当基准）。
    let (svn_url, cfg) = clock.run("读取工作副本信息（svn info）", || {
        let svn_url = working_copy_url(root)?;
        let cfg = build_config(req, &svn_url)?;
        Ok((svn_url, cfg))
    })?;

    // ---- 前置校验：全部在创建 .git **之前**，失败时无需回退 ----
    let pre = clock.run("前置校验 P1–P4（svn status）", || {
        run_preflight(root, &cfg.whitelist)
    })?;
    // P5，命中不阻断
    let pinned = clock.run("检测钉死的 external（svn propget -R）", || {
        detect_pinned(root, &cfg.whitelist)
    })?;

    // ⚠️ **进入回退区之前，原子地「创建并占有」`.git`。**
    clock.run("占有 .git", || claim_git_dir(root))?;

    // ---- 从这里开始，失败必须删掉本次创建的 .git ----
    match build_repo(root, &cfg, &pre, &svn_url, clock) {
        Ok((baseline, health)) => Ok(InitReport {
            node_rev: pre.node_rev,
            baseline,
            health,
            pinned,
            timings: Vec::new(),
        }),
        Err(e) => {
            // 回退要删掉的 .git 里可能已经有 `git add` 写进去的几 GB 对象，
            // 这一步本身就可能很慢，必须算进耗时。它总是「成功」的（rollback
            // 自己吞掉删除失败），不会抢走前面那个失败阶段的标记。
            let _ = clock.run("回退（删除刚建的 .git）", || {
                rollback(root);
                Ok(())
            });
            Err(e)
        }
    }
}

/// 从**空索引**起步录服务器 rN 的树。初始化与 Phase 1 R2 用它（spec §6.3.1）。
fn record_server_tree(
    repo_root: &str,
    cfg: &SvnConfig,
    pre: &Preflight,
    clock: &mut PhaseClock,
) -> Result<(String, BTreeSet<String>)> {
    let idx = TempIndex::empty(repo_root)?;
    record_server_tree_timed(repo_root, cfg, pre, &idx, clock)
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
///
/// # 候选索引由调用方给
///
/// spec §6.3.1：「初始化 / Phase 1 R2 从空索引开始，其余录制入口从当前 HEAD 树
/// 开始」。`sync_in` 的 S7 属于后者。起点不同、后面的改写与核对完全相同，所以
/// 只把「从哪棵树起步」交出去。
pub fn record_server_tree_into(
    repo_root: &str,
    cfg: &SvnConfig,
    pre: &Preflight,
    idx: &TempIndex,
) -> Result<(String, BTreeSet<String>)> {
    record_server_tree_timed(repo_root, cfg, pre, idx, &mut PhaseClock::default())
}

/// [`record_server_tree_into`] 的本体，额外按子步骤计时。只有初始化看这份耗时
/// ——`git add` 是用户报告里最可疑的大头，必须单独拆出来。
fn record_server_tree_timed(
    repo_root: &str,
    cfg: &SvnConfig,
    pre: &Preflight,
    idx: &TempIndex,
    clock: &mut PhaseClock,
) -> Result<(String, BTreeSet<String>)> {
    let fixes = classify(&pre.dirty)?;
    // `S` 先算出来，再拿它驱动改写——**同一个集合**既决定动哪些路径，也交给体检。
    // 分两份算的话，两份迟早会不一致，而「体检那份更大」这个方向会悄悄削弱判据。
    let touched = explained_paths(&fixes, &cfg.whitelist, &cfg.extensions);

    clock.run("录入磁盘内容（git add）", || {
        idx.add_all(repo_root, &cfg.whitelist)
    })?;

    clock.run("把本地改动换回服务器内容（svn cat）", || {
        rewrite_to_server(repo_root, pre, idx, &fixes, &touched)
    })?;

    // R2c：核对与录制**共用同一份候选索引**（spec §6.3.1），核对完之后在同一个
    // `idx` 上 `write_tree`——不得核对一次后又重新 add 另录一棵树。
    clock.run("纳入集合核对（git ls-files）", || {
        verify_include_set(repo_root, idx, &cfg.whitelist, &cfg.extensions, &fixes)
    })?;
    let tree = clock.run("写树（git write-tree）", || idx.write_tree(repo_root))?;
    Ok((tree, touched))
}

/// R2b 的逐条改写：脏条目换成服务器 rN 的内容，或从候选索引里剔除。
fn rewrite_to_server(
    repo_root: &str,
    pre: &Preflight,
    idx: &TempIndex,
    fixes: &std::collections::BTreeMap<String, Fix>,
    touched: &BTreeSet<String>,
) -> Result<()> {
    for (path, fix) in fixes {
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
    Ok(())
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
    clock: &mut PhaseClock,
) -> Result<(String, Health)> {
    clock.run("建仓库、写规则（git init）", || {
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
        Ok(())
    })?;

    // 规则写好之后、`git add` 之前：只读地问一遍 git「你会收哪些文件」。
    // `git add` 要把白名单里每个文件哈希压缩一遍，大工程上要跑很久，跑完才发现
    // 被某个 `.gitignore` 挡掉了几百个文件、再整个回退，代价太大（2026-10-27
    // 用户报告约半小时）。最终核对照留，这一道只负责早点失败。
    //
    // 脏清单修正必须与录制用同一套算法（`classify` + `explained_paths`），
    // 见 `dry_run_include_set` 的说明。两者都是纯内存计算，算两遍不费事。
    clock.run("纳入集合预检（git ls-files）", || {
        let fixes = classify(&pre.dirty)?;
        let touched = explained_paths(&fixes, &cfg.whitelist, &cfg.extensions);
        dry_run_include_set(root, &cfg.whitelist, &cfg.extensions, &fixes, &touched)
    })?;

    // R2a–R2c：录出服务器 rN 的树。首个基线从**空**索引起步。
    let (tree, explained) = record_server_tree(root, cfg, pre, clock)?;

    let trailer = Trailer {
        // Repository Root，不是工作副本 URL（spec §6.4.1）
        url: pre.repos_root.clone(),
        rev: pre.node_rev,
        uuid: pre.uuid.clone(),
    };
    // 首个基线**无父提交**；`expected_old = None` 表示「这个 ref 现在不该存在」，
    // 刚 `git init` 出来的仓库正是如此。
    let baseline = clock.run("发布基线（git commit-tree）", || {
        publish_baseline(root, &tree, None, &trailer, None)
    })?;

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
    clock.run("对齐索引（git read-tree）", || {
        run_git(root, &["read-tree", &tree])
    })?;

    // Lying 的判定放在阶段**里面**：它是这一阶段的失败，耗时表要把「← 在这一步
    // 失败」标在体检上。放在外面的话，体检阶段算成功，表里一个失败标记都没有。
    let health = clock.run("基线体检（svn diff）", || {
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
        Ok(health)
    })?;

    // 立刻离开基线分支（spec §6.5 最后一步）。
    clock.run("切到工作分支（git checkout）", || {
        run_git(root, &["checkout", "-b", WORK_BRANCH])
    })?;
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

/// 行尾迁移：装上规则 + 按新规则重录基线（spec §7.3.0「迁移」）。
///
/// # 为什么是一条命令，不是「装规则」「重建」两步
///
/// 规则决定 git 读磁盘时跑不跑行尾过滤器，而基线 blob 的形状是**录制时**定下的。
/// 只装规则不重建，仓库就停在「基线是旧形状、工作树按新规则读」的状态上，而
/// `git status` 因为 stat cache 暂时还报干净——等某个文件的 mtime 被碰一下
/// （编辑一次、`svn update` 一次）它才单独冒出来。用户看到的是「文件一个一个地
/// 无故变脏」，比一次性报错难查得多。那不是一个可以让用户停留的状态。
///
/// # 失败时必须把规则还原
///
/// 同理：重建没成，规则就不能留着，否则留下的正是上面那个潜伏状态。回滚用
/// [`write_attributes`] 返回的原始字节，用户自己原有的 attributes 也能逐字节还回去。
///
/// # 为什么光重录基线不够 —— 三样东西必须一起换形状
///
/// 2026-09-24 独立 review 发现并实测确认：`rebuild_baseline` 按设计**全程不动
/// HEAD 和真实索引**（R1–R5 的核心约束），所以它只把 `svn/base` 换成了新形状。
/// 而 Phase 1 建库时，`build_repo` 是靠一句 `read-tree` 把**真实索引**也对齐的，
/// 工作分支 tip 又恰好指向那个基线 commit —— 也就是说真实的 Phase 1 仓库里，
/// **基线树、工作分支 tip、真实索引三样同时是旧形状**。
///
/// 只换第一样的后果，正是本函数声称要消灭的那个潜伏故障：
///
/// ```text
/// 迁移后 git status  →  M Code/Nat.cs     ← 内容一个字没改，只是索引里的 blob 还是 LF
/// git diff svn/base HEAD → Code/Nat.cs    ← Phase 3 会把这批纯行尾差异提交进 SVN
/// ```
///
/// 所以迁移成功后必须把工作分支与真实索引一起搬到新基线上。
///
/// # 工作分支上有自己的提交时，本期拒绝
///
/// 那些 commit 里的 blob 是旧形状，**移一下 ref 修不了它们**——要修得逐路径把
/// 内容转成新的行尾形态，而目标形态只能靠数 `\r\n` 猜。猜错就是改坏用户已有的
/// 提交，这是本项目最保守的那一类操作。
///
/// 2026-09-24 实测推翻了一个看似现成的办法：`git add --renormalize`（以及它背后
/// 的 `cat-file --filters`）给出的是「git 会往磁盘写什么」，在 `autocrlf=true`
/// 下**所有文本文件都是 CRLF**；而磁盘的真实形态由 SVN 决定。一个 SVN 存 LF 的
/// 文件会被它改成 CRLF，凭空造出一批新的假阳性。
///
/// 「把一个分支重放到新基线上」本来就是 Phase 2 `sync_in` S9 的职责，那里有
/// rebase 和完整的冲突解决流程兜底。本期不在这里另造一套会猜错的转换逻辑，
/// 而是**在动任何东西之前**拒绝，并说清原因。
/// 迁移期间的仓库级互斥锁。
///
/// # 为什么必须有
///
/// 2026-09-25 PR #45 第二轮 review。`has_eol_rule` 判定、写规则、重建基线之间
/// 原先没有互斥，两条请求（两个仓库标签页各有一条横幅，命令又都是
/// `#[tauri::command(async)]` 跑在线程池上）可以同时通过判定：
///
/// ```text
/// A 写规则，记下 previous = None（原本没有这个文件）
/// B 写规则，记下 previous = A 刚写的那份
/// B 抢先 publish 基线 → 成功
/// A 的 publish 撞上旧值守卫失败 → 按 previous = None 把文件删掉
/// B 一路成功返回
/// ```
///
/// 结果是 B 告诉用户「迁移成功」，而规则文件已经被 A 的回滚删了——仓库恰好停在
/// 这一期要消灭的那个潜伏状态上。**这不是推演**：`two_concurrent_migrations_…`
/// 那条测试在没有本锁的实现上第 1 轮就复现了它。
///
/// # 与 [`IndexLock`] 分工不同，两把都要
///
/// 这把挡的是**本应用自己的**并发调用。挡不住外部 git 工具——它们不看我们的锁，
/// 那一段由 `swap_index_to_tree` 持有的 git 自己的索引锁负责。
struct MigrationLock {
    path: std::path::PathBuf,
}

impl MigrationLock {
    fn acquire(repo_root: &str) -> Result<Self> {
        let path = git_dir(repo_root)?.join("mygitclient-migrate.lock");
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(Self { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(GitError::new(
                "svn_migrate_busy",
                format!(
                    "这个仓库上已经有一次行尾迁移在进行中。请等它结束后再看；\
                     如果确认没有迁移在跑，那是上一次中途退出留下的残锁——\
                     此时仓库可能停在半迁移状态，建议先检查 `git status` 与 \
                     `git log svn/base -1`，确认无误后手工删掉 {} 再重试。",
                    path.display()
                ),
            )),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        // Drop 里不能 panic（展开途中 panic 会直接 abort）。
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn migrate_eol(repo_root: &str) -> Result<RebuildReport> {
    // 这一条必须在拿锁**之前**：锁文件要落在 `.git` 里，而这里正是在判断
    // `.git` 存不存在。顺序反过来的话，对着一个普通目录调用会先撞上 git 的错，
    // 用户拿到的就不是「这不是混合仓库」这句话了。纯文件系统读，无竞态。
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库，没有基线可迁移。",
        ));
    }
    // **随后立刻拿锁**，在读 `has_eol_rule` 之前——判定与动手之间只要有缝，
    // 两条请求就会双双通过判定（见 `MigrationLock` 的文档注释）。
    let _lock = MigrationLock::acquire(repo_root)?;

    if has_eol_rule(repo_root) {
        return Err(GitError::new(
            "svn_eol_already_migrated",
            "行尾规则已经装好了，不需要再迁移一次。\
             重建会移动基线指针，没有理由时不该动它。",
        ));
    }
    // **在写任何东西之前**判定能不能对齐。全或全无：拒绝时磁盘一个字节没动。
    let branch = check_can_align(repo_root)?;

    let previous = write_attributes(repo_root)?;
    let report = match rebuild_baseline(repo_root) {
        Ok(r) => r,
        Err(e) => {
            // 回滚失败不能把原来那个错盖掉：用户要先知道重建**为什么**没成。
            // 两个都失败时把回滚的情况附在消息里——否则仓库真的停在「规则装了
            // 但基线是旧的」状态上，而界面此后会因为 has_eol_rule 报 true 而
            // 不再提示迁移，等于无人知晓。
            return Err(match restore_attributes(repo_root, previous.as_deref()) {
                Ok(()) => e,
                Err(re) => GitError::new(
                    &e.code,
                    format!(
                        "{}\n\n另外：还原 .git/info/attributes 也失败了（{}）。\
                         这个仓库的基线现在是旧形状、而行尾规则已生效，\
                         请手工删掉 .git/info/attributes 后重试。",
                        e.message, re.message
                    ),
                ),
            });
        }
    };

    if let Err(e) = align_branch(repo_root, &branch, &report) {
        // 对齐没成 ⇒ 把基线也退回去，别留下「基线新、分支旧」这个半迁移状态。
        // `update-ref` 带旧值守卫：基线被外部改动过就不动它，如实报出来。
        let undo = run_git(
            repo_root,
            &[
                "update-ref",
                "refs/heads/svn/base",
                &report.old_baseline,
                &report.new_baseline,
            ],
        );
        // 两个回滚都要做，且**都不能吞错误**：吞掉的那个恰好是用户此后无从察觉的
        // 那种——界面会因为 `has_eol_rule` 报 true 而不再提示迁移。
        let restored = restore_attributes(repo_root, previous.as_deref());
        let mut msg = e.message.clone();
        if let Err(ue) = undo {
            msg.push_str(&format!(
                "\n\n另外：把基线退回旧 commit 也失败了（{}）。\
                 仓库现在是「基线已换新形状、工作分支还是旧形状」的半迁移状态，\
                 请手工执行 `git update-ref refs/heads/svn/base {}` 后重试。",
                ue.message, report.old_baseline
            ));
        }
        if let Err(re) = restored {
            msg.push_str(&format!(
                "\n\n另外：还原 .git/info/attributes 也失败了（{}）。\
                 行尾规则现在是生效的，而基线不是按它录的，\
                 请手工删掉 .git/info/attributes 后重试。",
                re.message
            ));
        }
        return Err(GitError::new(&e.code, msg));
    }
    Ok(report)
}

/// `git rev-list --count <range>`。数不出来时返回 0 —— 这个数字只进错误文案，
/// 不参与任何判定，拿它当判据才需要把失败区分出来。
fn rev_count(repo_root: &str, range: &str) -> Result<u32> {
    let (code, out) = run_git_code(repo_root, &["rev-list", "--count", range])?;
    if code != 0 {
        return Ok(0);
    }
    Ok(out.trim().parse().unwrap_or(0))
}

/// 迁移前置：能不能把工作分支与真实索引一起搬到新基线上？返回分支名。
///
/// 四条都必须成立，任一不成立就**在动任何东西之前**拒绝（见 [`migrate_eol`]
/// 的文档注释）。判据全部是纯读。
fn check_can_align(repo_root: &str) -> Result<String> {
    // HEAD 必须附着在分支上：detached 时没有 ref 可移
    let (code, out) = run_git_code(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"])?;
    let branch = out.trim().to_string();
    if code != 0 || branch.is_empty() {
        return Err(GitError::new(
            "svn_eol_migrate_head_detached",
            "HEAD 没有附着在分支上（detached HEAD），迁移需要把工作分支一起搬到\
             新基线上，没有分支可搬。请先 checkout 回你的工作分支再试。",
        ));
    }
    if branch == BASELINE_BRANCH {
        return Err(GitError::new(
            "svn_eol_migrate_on_baseline",
            format!(
                "当前在基线分支 `{BASELINE_BRANCH}` 上。请先 checkout 回你的工作分支再迁移\
                 ——基线分支由本应用维护，不该在它上面开发。"
            ),
        ));
    }

    let head = run_git(repo_root, &["rev-parse", "HEAD"])?
        .trim()
        .to_string();
    let base = run_git(
        repo_root,
        &["rev-parse", &format!("refs/heads/{BASELINE_BRANCH}")],
    )?
    .trim()
    .to_string();
    if head != base {
        // 两种都不能迁移，但成因完全不同，消息必须分开说——否则用户按着错的
        // 提示去找不存在的「本地提交」。
        //
        // `behind` ≠ 0：工作分支落后于基线。这是 Phase 1 的**合法常态**，
        // spec §16 明写「重建基线的代价是工作分支不会自动跟进」。
        let ahead = rev_count(repo_root, &format!("{base}..HEAD"))?;
        let behind = rev_count(repo_root, &format!("HEAD..{base}"))?;
        let why = if ahead > 0 {
            format!("分支 `{branch}` 上有 {ahead} 笔本地提交还没进基线")
        } else {
            format!(
                "分支 `{branch}` 落后基线 {behind} 笔提交——基线被单独重建过，\
                 而工作分支还没跟上（这是正常的，重建基线本来就不推进工作分支）"
            )
        };
        return Err(GitError::new(
            "svn_eol_migrate_branch_not_at_baseline",
            format!(
                "{why}，本期无法迁移。\n\n\
                 工作分支上那些 commit 里记的内容是旧的行尾形状，光移动分支指针\
                 修不了它们；而要修就得逐个文件改写你已有提交的内容，判断目标行尾\
                 形态只能靠猜，猜错就是把你的历史改坏。\n\n\
                 「把一个分支重放到新基线上」是后续 sync_in 的职责，那里有完整的\
                 冲突解决流程。在那之前，这个仓库继续按现在的方式使用即可——\
                 不迁移不会让情况变得更糟。"
            ),
        ));
    }

    // 索引里不能有暂存的改动：对齐要 `read-tree` 覆盖索引，有暂存就会丢掉
    let (code, _) = run_git_code(repo_root, &["diff-index", "--cached", "--quiet", &base])?;
    if code != 0 {
        return Err(GitError::new(
            "svn_eol_migrate_index_dirty",
            "暂存区里有改动。迁移要重写索引，会把它们冲掉。\
             请先提交或取消暂存（`git restore --staged`）再试。\
             工作目录里未暂存的改动不受影响，不用管。",
        ));
    }
    Ok(branch)
}

/// 把工作分支与真实索引一起搬到新基线上。
///
/// 前提由 [`check_can_align`] 保证：分支 tip == 旧基线、索引 == 旧基线树。
/// 因此这里既不会丢提交，也不会丢暂存内容；磁盘全程不碰（`read-tree` **不带
/// `-u`**，与 `build_repo` 里那句同样的理由）。
///
/// # 为什么先 `read-tree` 后 `update-ref`
///
/// 2026-09-25 PR #45 review。原先是反的，于是 `read-tree` 失败时分支已经移走了，
/// 而失败路径只退得回基线——留下的状态**比迁移没成糟得多**：
///
/// ```text
/// git status                          →  M <每个文本文件>   ← 用户从没暂存过
/// git rev-list --count svn/base..HEAD →  1                  ← 凭空多一笔提交
/// ```
///
/// 那个 `1` 会让下一次迁移撞进 [`check_can_align`] 的「分支上有本地提交」分支被
/// 永久拒绝，仓库从此在界面里迁不动，只能手工 `git update-ref` 救。
///
/// 换成现在的顺序之后：`read-tree` 是更容易失败的一步（`.git/index.lock` 随便
/// 一个 git 进程都会造出来），而它失败时**一个 ref 都还没动**，天然全或全无；
/// 后面的 `update-ref` 自带旧值守卫、本身原子，万一失败就把索引 `read-tree`
/// 回旧基线——每一步之前的状态都是可恢复的。
fn align_branch(repo_root: &str, branch: &str, report: &RebuildReport) -> Result<()> {
    // 对齐真实索引。**这一步是 compare-and-swap，不是「先查再写」。**
    //
    // `check_can_align` 那次检查发生在 `rebuild_baseline` 之前，而重建要跑完整的
    // SVN 往返（含网络）——这中间用户或别的 git 工具 `git add` 进来的东西，
    // 如果只是「再查一次然后 read-tree」，仍然会在那两个 git 进程之间的缝里被
    // 静默冲掉（磁盘内容还在，但暂存状态没了）。`swap_index_to_tree` 全程持有
    // git 自己的 `.git/index.lock`，校验与换上之间别的 git 进程进不来。
    //
    // 换上去的索引由 `read-tree`（**不带 `-u`**）在独立的临时索引里生成：
    // 绝不能用基线内容去覆盖磁盘，那会抹掉用户未提交的 SVN 改动
    //（`build_repo` 里记过同一条理由）。
    swap_index_to_tree(repo_root, &report.old_baseline, &report.new_baseline).map_err(|e| {
        if e.code == "svn_index_moved" {
            GitError::new(
                "svn_eol_migrate_index_changed",
                "迁移过程中暂存区发生了变化（有别的 git 操作在同时进行？）。\
                 对齐索引会把这些暂存内容冲掉，所以这次迁移已中止，\
                 仓库回到了迁移前的样子。\
                 请先提交或取消暂存（`git restore --staged`）再重试。",
            )
        } else {
            e
        }
    })?;

    // 带旧值守卫：这中间若有外部把分支挪走了，宁可失败也不覆盖
    if let Err(e) = run_git(
        repo_root,
        &[
            "update-ref",
            &format!("refs/heads/{branch}"),
            &report.new_baseline,
            &report.old_baseline,
        ],
    ) {
        // 索引已经换成新基线树了，退回去——否则 `git status` 会报一堆用户从没
        // 暂存过的改动。同样走 CAS：这中间若有别人动过索引，宁可不动也不覆盖。
        return Err(
            match swap_index_to_tree(repo_root, &report.new_baseline, &report.old_baseline) {
                Ok(_) => e,
                Err(re) => GitError::new(
                    &e.code,
                    format!(
                        "{}\n\n另外：把索引退回旧基线也失败了（{}）。\
                     `git status` 现在会显示一批你从没暂存过的改动，\
                     请手工执行 `git read-tree {}` 复位。",
                        e.message, re.message, report.old_baseline
                    ),
                ),
            },
        );
    }
    Ok(())
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
    // 重建不报耗时：计时只为初始化向导服务，这里给一个用完即弃的计时器。
    let (tree, explained) = record_server_tree(repo_root, &cfg, &pre, &mut PhaseClock::default())?;

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
