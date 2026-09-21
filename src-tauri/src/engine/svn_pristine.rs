//! 取单个文件在**服务器 rN 上**的字节（spec §16 R2b 的取数入口）。
//!
//! # 这个模块存在的理由
//!
//! 基线的语义是「白名单内的文件在 revision N 时**服务器上的样子**」（§6.4）。
//! 工作副本干净时磁盘就等于那个样子，直接 `git add` 即可；一旦放松了「必须干净」
//! 这条门槛（§16.1），脏文件的磁盘内容就不再是服务器内容，必须另找来源。
//!
//! 来源是 `svn cat -r BASE`——它只往 stdout 吐字节，**不碰磁盘**。这一点是整条
//! 重建流程能保持「对工作目录只读」的关键：它不是 `svn revert` 的变体。

use crate::engine::svn_cli::run_svn_bytes;
use crate::error::{GitError, Result};

/// `svn cat -r BASE` 的两种结局。
#[derive(Debug, Clone, PartialEq)]
pub enum BaseContent {
    /// 服务器 rN 上有这个文件，这是它的字节。
    Bytes(Vec<u8>),
    /// 服务器 rN 上**没有**这个文件——本地 `svn add` 了还没提交。
    ///
    /// **不是错误。** 它是一个正常的分类结果：这类文件要从基线树里剔除
    /// （§16 R2a 的「Drop」）。把它当错误处理，整条重建会在用户有一个未提交的
    /// 新文件时直接失败——而那是最常见的开发状态之一。
    NotOnServer,
}

/// 取 `rel_path` 在服务器 rN 上的字节。
///
/// `rel_path` 是仓库相对路径、正斜杠。`repo_root` 是 svn 命令的工作目录。
///
/// # 为什么是 `-r BASE` 而不是 `-r N`
///
/// 两者取到的字节相同（`BASE == N` 由 P2 的 `nodeRevs` 单值保证），但
/// **只有 `-r BASE` 离线可用**。2026-09-16 实测，把仓库移走之后：
///
/// ```text
/// svn cat -r BASE Code/A.cs@  →  "a-r1"   ← 走 .svn/pristine，不联网
/// svn cat -r 1    Code/A.cs@  →  E170013: Unable to connect to a repository
/// ```
///
/// 写成 `-r N` 会把重建基线从一条本地操作变成联网操作，断网时整个功能不可用，
/// 而它需要的字节明明就在磁盘上。见 spec §16.2。
///
/// ⚠️ 等价性**完全依赖 P2**。允许 mixed-revision 工作副本进来的话，BASE 不再是
/// 单一的 N，取回的字节会来自若干不同 revision，而 trailer 只写得下一个数——
/// 那正是撒谎基线。P2 在这里不是顺手做的校验，是 `-r BASE` 的授权条件。
pub fn cat_base(repo_root: &str, rel_path: &str) -> Result<BaseContent> {
    let path = rel_path.replace('\\', "/");
    if path.trim().is_empty() {
        // 空 target 会让 svn 去解释成「当前目录」。本仓库已经因为空 pathspec
        // 误伤过一次（AGENTS.md §6.2），这类边界一律显式拒绝。
        return Err(GitError::new(
            "svn_pristine_empty_path",
            "取服务器内容时收到空路径。空 target 会被 svn 解释成当前目录，\
             不能交给它去猜。",
        ));
    }

    // 追加空的 peg 后缀：SVN 把 target 里**最后一个** `@` 之后的部分当 peg
    // revision 解析，而 `--` 挡不住这一层（见 `svn_wc::peg_safe` 的实测记录）。
    let arg = format!("{path}@");
    let out = run_svn_bytes(repo_root, &["cat", "-r", "BASE", "--", &arg])?;
    if out.code == 0 {
        return Ok(BaseContent::Bytes(out.stdout));
    }

    // 「本地 add 了没提交」这一种失败。**两个错误码都要认**——它们取决于 `-r`
    // 的写法，2026-09-16 实测：
    //
    // ```text
    // svn cat -r 1    Code/Added.cs@  →  E195002: has no committed revision
    // svn cat -r BASE Code/Added.cs@  →  E200009: has no pristine version until it is committed
    // ```
    //
    // 本模块只用 `-r BASE`（见上），所以实际会走 E200009 那条；E195002 一并认是
    // 为了不把结论绑死在一个随参数形式变化的错误码上。
    //
    // ⚠️ **分类的权威来源不是这里，是 `svn status`。** R2a 已经按状态判定哪些
    // 文件该从树里剔除，那些文件根本不会走到这个函数。这里返回 `NotOnServer`
    // 是一层交叉校验：调用方对一个「本该在服务器上」的文件拿到它，说明分类与
    // 实际不符，必须中止而不是静默把文件丢掉（见 §16 R2b 的调用点）。
    if out.stderr.contains("E200009") || out.stderr.contains("E195002") {
        return Ok(BaseContent::NotOnServer);
    }

    Err(GitError::new(
        "svn_pristine_cat_failed",
        format!(
            "取不到 `{path}` 在服务器上的内容（svn 退出 {}）：{}",
            out.code,
            out.stderr.trim()
        ),
    ))
}
