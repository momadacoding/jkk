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
use crate::engine::stash::StashEntry;
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
    ///
    /// 契约见 [`Vcs::discard`] 的空 paths 说明，同样适用。
    fn stage(&self, paths: &[String]) -> Result<()>;

    /// 把指定路径移出暂存区。
    ///
    /// 契约见 [`Vcs::discard`] 的空 paths 说明，同样适用。git 实现上这条尤其
    /// 要紧：`git reset HEAD` 不带 pathspec 会清空整个暂存区且退出 0。
    fn unstage(&self, paths: &[String]) -> Result<()>;

    /// 丢弃指定路径的改动。`staged` 决定回退到暂存区还是 HEAD。
    ///
    /// # 空 paths 契约
    ///
    /// **接收路径列表的每个方法，空列表都必须是 no-op**，绝不能退化成「全部」。
    /// 这条对 `stage` / `unstage` / `discard` 一体适用。
    ///
    /// 理由是各命令行工具在这里的默认语义都危险且不一致：`git reset HEAD` 无
    /// pathspec 清空整个暂存区并退出 0；`git checkout --` 无 pathspec 会覆盖整个
    /// 工作区；Phase 3 的 `svn commit` 无 target 会提交整个工作副本且**不可撤销**。
    /// 本项目已因空 pathspec 出过一次误伤整个仓库的事故（`AGENTS.md` §6.2）。
    ///
    /// 各实现在**进入命令行之前**挡住空列表，不要指望下游工具报错。
    ///
    /// # 部分失败契约
    ///
    /// discard 天然分成若干不可回滚的阶段（git 实现里是「恢复内容 → 移出暂存区并
    /// 删除 → 删未跟踪文件」）。**没有任何一步能撤销**：抹掉的工作区内容没有副本。
    /// 因此实现必须做到两件事：
    ///
    /// 1. **按可逆性从高到低排列阶段，任一步失败立刻停下**，不再执行后面更不可逆
    ///    的操作
    /// 2. 失败时返回 `discard_partial`，消息里说清**已完成 / 可能已部分处理 /
    ///    未处理**三类路径。只把底层工具的 stderr 抛上去是不够的——用户看不出仓库
    ///    停在哪，会以为什么都没发生
    ///
    /// 同理，用于分类的探测命令失败时**不得降级成空集**。空集会直接改变分类结果，
    /// 把「应当恢复」的文件送进「删除」桶。唯一的例外是能被正向确认的合法情形
    /// （git 侧是「仓库还没有任何提交」），其余一律抛出。
    fn discard(&self, paths: &[String], staged: bool) -> Result<()>;

    /// 提交暂存区内容。
    fn commit(&self, message: &str, amend: bool) -> Result<()>;

    /// 新建分支。`start_point` 为 None 时从当前 HEAD 开。
    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()>;

    /// 删除本地分支。
    fn delete_branch(&self, name: &str) -> Result<()>;

    /// 重命名本地分支。
    fn rename_branch(&self, old: &str, new: &str) -> Result<()>;

    /// 切到指定 revision。
    fn checkout(&self, rev: &str) -> Result<()>;

    /// 列出所有 stash，最新的在前。
    fn list_stashes(&self) -> Result<Vec<StashEntry>>;

    /// 把当前改动压进 stash。
    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()>;

    /// 应用第 index 条 stash，保留该条目。
    fn stash_apply(&self, index: usize) -> Result<()>;

    /// 应用第 index 条 stash 并删除它。
    fn stash_pop(&self, index: usize) -> Result<()>;

    /// 删除第 index 条 stash，不应用。
    fn stash_drop(&self, index: usize) -> Result<()>;
}

/// 为某个工作目录挑一个后端。
///
/// Phase 0 永远返回 GitBackend。**不要在这里加 `.svn` 判断**——现在加会改变
/// 「一个同时有 .svn 和 .git 的目录」的现有行为（今天它就是个普通 git 仓库），
/// 那就不是零行为变化了。SVN 分支在 Phase 1 连同配置文件识别一起加。
pub fn backend_for(path: &str) -> Result<Box<dyn Vcs>> {
    Ok(Box::new(git::GitBackend::new(path)))
}
