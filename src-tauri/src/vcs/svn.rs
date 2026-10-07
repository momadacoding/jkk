//! SVN + git 混合仓库的后端。
//!
//! **Phase 1a 的它几乎只做一件事：声明 capabilities。** 全部 git 语义的读写
//! 原样委托给 `GitBackend`——§16 Phase 1 的验收标准就是「能打开 SVN 仓库，
//! 本地 git 的暂存/分支/stash 可用」。SVN 侧的状态判定、基线、混合图都在 1b。
//!
//! 之所以现在就要有这个类型：Phase 0 的交接指出接缝**不完整**，
//! `checkout_remote_branch` / `create_tag` / `cherry_pick` / `save_patch` /
//! `fetch` / `pull` / `push` 与整个 `commands/worktree.rs` 仍直连 git。
//! 光有 trait 拦不住它们，必须有一个能说「我不支持这些」的后端。

use super::git::GitBackend;
use super::{Capabilities, Vcs};
use crate::engine::diff::{DiffData, StatusData};
use crate::engine::graph::GraphData;
use crate::engine::repo::RepoInfo;
use crate::engine::stash::StashEntry;
use crate::engine::svn_config::{SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_journal;
use crate::engine::svn_lock::assert_not_syncing;
use crate::engine::svn_sync_stash::stash_position;
use crate::error::{GitError, Result};

pub struct SvnGitBackend {
    git: GitBackend,
    /// 已校验过的配置。持有它而不是每次读盘，正是 `trait Vcs` 绑定工作目录
    /// （而非每个方法收 path）的理由，见 `vcs/mod.rs` 开头。
    ///
    /// Phase 1a 只在 `open()` 里用它做校验，字段本身还没有读者——Phase 1b 的
    /// 状态判定、基线与体检都要用。现在就持有是为了不在 1b 改构造函数签名。
    ///
    /// 用裸 `#[allow(dead_code)]` 而不是带 `reason = "…"`：后者需要 Rust 1.81，
    /// 而 `Cargo.toml` 声明的 `rust-version` 是 1.77.2，用它等于悄悄抬高 MSRV。
    /// `AGENTS.md` §7.3 要求的「必须说明理由」由上面这段注释满足。
    #[allow(dead_code)]
    config: SvnConfig,
    /// 工作目录根。`GitBackend` 自己也存了一份，但那个字段是私有的、没有访问器，
    /// 而同步锁守卫（spec §5.1）每个写方法都要用它，所以这里自己留一份。
    /// 原样保存调用方给的写法，不做规范化——与 `GitBackend::root` 同一条理由。
    root: String,
}

impl SvnGitBackend {
    /// 打开并校验。配置不合法时**这里就失败**，不要让半个坏配置流到下游。
    ///
    /// 两道校验缺一不可：`read_config` 只看字符串本身（纯函数），
    /// `validate_whitelist_against_disk` 才能抓到「目录条目漏写尾斜杠」和
    /// 「大小写写错」——这两种配置看着都没问题，但一个会让整棵子树绕过扩展名
    /// 过滤进 git，另一个会让纳入集合核对在正常仓库上误报。
    pub fn open(path: &str) -> Result<Self> {
        let config = crate::engine::svn_config::read_config(path)?;
        crate::engine::svn_config::validate_whitelist_against_disk(path, &config.whitelist)?;
        Ok(Self {
            git: GitBackend::new(path),
            config,
            root: path.to_string(),
        })
    }
}

impl Vcs for SvnGitBackend {
    /// SVN 模式下这些功能在概念上就不存在，前端不该渲染入口。
    ///
    /// 逐条理由见 spec §1.2 与 Phase 0 决策日志的交接表：worktree / tag /
    /// 远程分支 / PR 在 SVN 侧无对应物；`cherry_pick` 依赖 git 的 DAG 语义；
    /// `save_patch` 是 `format-patch`，git 专属。
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            vcs: "svn",
            worktrees: false,
            tags: false,
            remotes: false,
            pull_requests: false,
            cherry_pick: false,
            patches: false,
        }
    }

    // 以下全部原样委托。Phase 1a 不改变任何 git 行为。
    fn open(&self) -> Result<RepoInfo> {
        self.git.open()
    }
    fn graph(&self) -> Result<GraphData> {
        self.git.graph()
    }
    fn status(&self) -> Result<StatusData> {
        self.git.status()
    }
    fn diff(&self, scope: &str) -> Result<DiffData> {
        self.git.diff(scope)
    }
    fn stage(&self, paths: &[String]) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.stage(paths)
    }
    fn unstage(&self, paths: &[String]) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.unstage(paths)
    }
    fn discard(&self, paths: &[String], staged: bool) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.discard(paths, staged)
    }
    fn commit(&self, message: &str, amend: bool) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.commit(message, amend)
    }
    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.create_branch(name, start_point)
    }
    fn delete_branch(&self, name: &str) -> Result<()> {
        assert_not_syncing(&self.root)?;
        refuse_baseline_branch(name)?;
        self.git.delete_branch(name)
    }
    fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        assert_not_syncing(&self.root)?;
        refuse_baseline_branch(old)?;
        refuse_baseline_branch(new)?;
        self.git.rename_branch(old, new)
    }
    fn checkout(&self, rev: &str) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.checkout(rev)
    }
    fn list_stashes(&self) -> Result<Vec<StashEntry>> {
        self.git.list_stashes()
    }
    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.stash_push(message, include_untracked)
    }
    fn stash_apply(&self, index: usize) -> Result<()> {
        assert_not_syncing(&self.root)?;
        self.git.stash_apply(index)
    }
    fn stash_pop(&self, index: usize) -> Result<()> {
        assert_not_syncing(&self.root)?;
        assert_not_sync_backup(&self.root, index)?;
        self.git.stash_pop(index)
    }
    fn stash_drop(&self, index: usize) -> Result<()> {
        assert_not_syncing(&self.root)?;
        assert_not_sync_backup(&self.root, index)?;
        self.git.stash_drop(index)
    }
}

/// 未收尾的同步把 WIP 的原始备份放在 stash 里。冲突出口放开了锁，`assert_not_syncing`
/// 拦不到；应用内把那一条删掉（drop / pop）之后再「继续」，最后一个钉住它的 ref 也会被拆掉，
/// 备份就再也找不回来了（PR #49 Codex review #2）。只挡那一条，别的 stash 照常。
fn assert_not_sync_backup(root: &str, index: usize) -> Result<()> {
    let Some(oid) = svn_journal::read(root)?.and_then(|j| j.stash_oid) else {
        return Ok(());
    };
    if stash_position(root, &oid)? == Some(index) {
        return Err(GitError::new(
            "sync_in_unfinished",
            "这条 stash 是上一次「更新」给未提交改动留的备份，那次同步还没有收尾。\
             请先在顶部横幅上选择「继续」或「放弃」，再处理它。",
        ));
    }
    Ok(())
}

/// 基线分支不能被删、不能被改名走，别的分支也不能改名成它（PR #49 Codex review #2）。
///
/// 失同步检测、体检、重建、更新全都以 `svn/base` 为参照；未收尾的同步还按 OID
/// 依赖着它。界面的「删除」已置灰，但后端不能只靠界面。
///
/// 整名匹配，与 `capabilities.ts::isBaselineBranch` 一致：`svn/base-backup` 这类
/// 名字是用户自己的分支。容忍 `refs/heads/` 前缀。
///
/// 但**不区分大小写**：2026-09-30 实测（Windows / NTFS，ref 为散装文件），
/// `git branch -D SVN/BASE` 删掉的就是 `svn/base`，`-m` 同理。
///
/// 名字里带 `@{` 的一律拒绝：2026-10-07 实测（git 2.51.2），`git branch -D` / `-m`
/// 会把 `@{-1}`、`@{u}`、`work@{upstream}` 展开成别的分支——一次更新先切到基线再切回来，
/// 之后 `@{-1}` 就是 `svn/base`。不去解析它指向谁：`check-ref-format` 本来就不许分支名含
/// `@{`，拒掉它挡不住任何一个真实的分支，而解析与删除之间又会多出一道缝。
fn refuse_baseline_branch(name: &str) -> Result<()> {
    if name.contains("@{")
        || name
            .strip_prefix("refs/heads/")
            .unwrap_or(name)
            .eq_ignore_ascii_case(BASELINE_BRANCH)
    {
        return Err(GitError::new(
            "svn_baseline_branch_protected",
            format!(
                "`{BASELINE_BRANCH}` 是 SVN 基线分支，记着「上次与服务器对齐时的内容」，\
                 不能删除或改名，别的分支也不能改名成它。基线不对时请用「重建基线」。"
            ),
        ));
    }
    Ok(())
}
