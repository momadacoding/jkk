//! 版本控制后端抽象。
//!
//! 一个后端**绑定到一个工作目录**，而不是每个方法都收一个 path。
//! 这是为 Phase 1 的 SVN 后端准备的：它要持有白名单、svnUrl、基线分支名
//! 这些每仓库状态（来自 `.git/mygitclient.json`），按 path 传参会让每个方法
//! 都重复读盘解析一次配置。
//!
//! Phase 0 只有 GitBackend 一个实现，方法体一律委托给 `engine::*`——
//! 这一期的目的是把接缝切出来，不是改行为。

pub mod git;

use crate::engine::diff::{DiffData, StatusData};
use crate::engine::graph::GraphData;
use crate::engine::repo::RepoInfo;
use crate::error::Result;

/// 绑定到某个工作目录的版本控制后端。
///
/// `Send + Sync` 是给 Phase 2 的仓库级操作锁准备的：那个注册表要跨线程持有
/// 后端实例。现在不加，到时候就是一次涉及所有实现的破坏性改动。
pub trait Vcs: Send + Sync {
    /// 读取仓库基本信息（HEAD 名与 oid）。
    fn open(&self) -> Result<RepoInfo>;

    /// 读取提交图。
    fn graph(&self) -> Result<GraphData>;

    /// 读取工作区状态（已暂存 / 未暂存）。
    fn status(&self) -> Result<StatusData>;

    /// 读取指定 scope 的 diff。scope 的取值形式由各后端自行解释。
    fn diff(&self, scope: &str) -> Result<DiffData>;

    /// 把指定路径加入暂存区。
    fn stage(&self, paths: &[String]) -> Result<()>;

    /// 把指定路径移出暂存区。
    fn unstage(&self, paths: &[String]) -> Result<()>;

    /// 丢弃指定路径的改动。`staged` 决定回退到暂存区还是 HEAD。
    ///
    /// **空 paths 必须是 no-op**，不能退化成「全部」——那是一次误伤整个仓库的
    /// 不可逆操作。各实现都要守住这条。
    fn discard(&self, paths: &[String], staged: bool) -> Result<()>;

    /// 提交暂存区内容。
    fn commit(&self, message: &str, amend: bool) -> Result<()>;
}

/// 为某个工作目录挑一个后端。
///
/// Phase 0 永远返回 GitBackend。**不要在这里加 `.svn` 判断**——现在加会改变
/// 「一个同时有 .svn 和 .git 的目录」的现有行为（今天它就是个普通 git 仓库），
/// 那就不是零行为变化了。SVN 分支在 Phase 1 连同配置文件识别一起加。
pub fn backend_for(path: &str) -> Result<Box<dyn Vcs>> {
    Ok(Box::new(git::GitBackend::new(path)))
}
