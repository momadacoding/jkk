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
use crate::engine::svn_config::SvnConfig;
use crate::error::Result;

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
}

impl SvnGitBackend {
    /// 打开并校验。配置不合法时**这里就失败**，不要让半个坏配置流到下游。
    pub fn open(path: &str) -> Result<Self> {
        let config = crate::engine::svn_config::read_config(path)?;
        Ok(Self {
            git: GitBackend::new(path),
            config,
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
        self.git.stage(paths)
    }
    fn unstage(&self, paths: &[String]) -> Result<()> {
        self.git.unstage(paths)
    }
    fn discard(&self, paths: &[String], staged: bool) -> Result<()> {
        self.git.discard(paths, staged)
    }
    fn commit(&self, message: &str, amend: bool) -> Result<()> {
        self.git.commit(message, amend)
    }
    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()> {
        self.git.create_branch(name, start_point)
    }
    fn delete_branch(&self, name: &str) -> Result<()> {
        self.git.delete_branch(name)
    }
    fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        self.git.rename_branch(old, new)
    }
    fn checkout(&self, rev: &str) -> Result<()> {
        self.git.checkout(rev)
    }
    fn list_stashes(&self) -> Result<Vec<StashEntry>> {
        self.git.list_stashes()
    }
    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()> {
        self.git.stash_push(message, include_untracked)
    }
    fn stash_apply(&self, index: usize) -> Result<()> {
        self.git.stash_apply(index)
    }
    fn stash_pop(&self, index: usize) -> Result<()> {
        self.git.stash_pop(index)
    }
    fn stash_drop(&self, index: usize) -> Result<()> {
        self.git.stash_drop(index)
    }
}
