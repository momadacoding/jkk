//! 只读的 git 查询，走 libgit2（C1a 决策日志「性能 P3」）。
//!
//! 项目约定「读走 libgit2、写走 git CLI」，SVN 同步 / 跟进这一路原先连查一个 ref 都起 git 子进程：
//! 一次跟进一百多个，Windows 上每个约 45 ms，起进程的开销占了跟进耗时的大头（2026-11-01 实测）。
//!
//! 这里只收与 CLI 一一对应的查询。`diff-tree`（重命名检测）、`log` / `rev-list` 的排序、
//! `worktree list`（prunable 判定）两边语义有差，留在 CLI。
//!
//! # 「不存在」与「坏了」要分开
//!
//! [`resolve`] 只把 ref / 路径层面的 `NotFound` / `InvalidSpec` / `Ambiguous` 当成「没有」；
//! ref 文件损坏、ref 指向的对象缺失都返回 `Err`。把损坏当成不存在会让调用方以为基线还没建，
//! 进而覆盖掉一段真实历史（`svn_baseline::read_baseline`）。
//!
//! 这两种损坏上 CLI 并不报错：内容是垃圾的 ref，`rev-parse -q --verify` 警告后退出 1（算「没有」）；
//! 悬空 ref 退出 0 照样打印 OID。这里有意比 CLI 严（`tests/git_read.rs`，独立 review 2026-11-01）。

use crate::error::Result;
use git2::{ErrorClass, ErrorCode, ObjectType, Repository, TreeWalkMode, TreeWalkResult};
use std::collections::BTreeSet;
use std::path::PathBuf;

fn open(repo: &str) -> Result<Repository> {
    Ok(Repository::open(repo)?)
}

fn absent(e: &git2::Error) -> bool {
    // ref 在、对象没了（悬空 ref）也报 NotFound，但类别是 Odb：那是对象库坏了，不是「没有」
    matches!(
        e.code(),
        ErrorCode::NotFound | ErrorCode::InvalidSpec | ErrorCode::Ambiguous
    ) && e.class() != ErrorClass::Odb
}

/// `git rev-parse --git-dir`，绝对路径。linked worktree 里是 `.git/worktrees/<名字>`，与 CLI 相同。
pub fn git_dir(repo: &str) -> Result<PathBuf> {
    let r = open(repo)?;
    // libgit2 给的是带尾斜杠的正斜杠路径；去掉尾斜杠，后面 join 出来的文件名才干净
    let p = r.path().to_string_lossy();
    Ok(PathBuf::from(p.trim_end_matches(['/', '\\'])))
}

/// `git rev-parse -q --verify <spec>` 的完整 OID；解析不出来时 `None`。支持 `X^{commit}`、
/// `X^1`、`X:path` 这些写法（libgit2 的 revparse 与 CLI 同一套语法）。
pub fn resolve(repo: &str, spec: &str) -> Result<Option<String>> {
    match open(repo)?.revparse_single(spec) {
        Ok(o) => Ok(Some(o.id().to_string())),
        Err(e) if absent(&e) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `git symbolic-ref -q HEAD`：HEAD 指向的 ref 全名；游离 HEAD 时 `None`。
pub fn symbolic_head(repo: &str) -> Result<Option<String>> {
    let r = open(repo)?;
    let head = r.find_reference("HEAD")?;
    Ok(head.symbolic_target().map(str::to_string))
}

/// commit 说明的原始字节（`cat-file commit` 里空行之后的部分），不做任何清理。
pub fn commit_message(repo: &str, rev: &str) -> Result<Vec<u8>> {
    let r = open(repo)?;
    let c = r.revparse_single(rev)?.peel_to_commit()?;
    Ok(c.message_raw_bytes().to_vec())
}

/// `git merge-base <a> <b>`；没有共同祖先时 `None`。
pub fn merge_base(repo: &str, a: &str, b: &str) -> Result<Option<String>> {
    let r = open(repo)?;
    let a = r.revparse_single(a)?.peel_to_commit()?.id();
    let b = r.revparse_single(b)?.peel_to_commit()?.id();
    match r.merge_base(a, b) {
        Ok(o) => Ok(Some(o.to_string())),
        Err(e) if e.code() == ErrorCode::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// `git merge-base --is-ancestor <a> <b>`：`a` 是 `b` 本身或其祖先。
pub fn is_ancestor(repo: &str, a: &str, b: &str) -> Result<bool> {
    let r = open(repo)?;
    let a = r.revparse_single(a)?.peel_to_commit()?.id();
    let b = r.revparse_single(b)?.peel_to_commit()?.id();
    Ok(a == b || r.graph_descendant_of(b, a)?)
}

/// `git ls-tree -r --name-only <rev>`：树里所有非目录条目（文件、符号链接、子模块指针）的路径。
/// 非 UTF-8 的名字按 lossy 处理，与原来 `run_git` 的输出一致。
pub fn tree_paths(repo: &str, rev: &str) -> Result<BTreeSet<String>> {
    let r = open(repo)?;
    let tree = r.revparse_single(rev)?.peel_to_tree()?;
    let mut out = BTreeSet::new();
    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() != Some(ObjectType::Tree) {
            let name = String::from_utf8_lossy(entry.name_bytes());
            out.insert(format!("{dir}{name}"));
        }
        TreeWalkResult::Ok
    })?;
    Ok(out)
}

/// 索引里未合并（冲突态）的路径，即 `git diff --name-only --diff-filter=U`。
pub fn unmerged_paths(repo: &str) -> Result<Vec<String>> {
    let r = open(repo)?;
    let idx = r.index()?;
    let mut out = BTreeSet::new();
    for c in idx.conflicts()? {
        let c = c?;
        if let Some(e) = c.our.or(c.their).or(c.ancestor) {
            out.insert(String::from_utf8_lossy(&e.path).into_owned());
        }
    }
    Ok(out.into_iter().collect())
}

/// `git config --bool --get core.ignoreCase`；没设或读不出来都按 `false`。
pub fn ignore_case(repo: &str) -> bool {
    open(repo)
        .and_then(|r| Ok(r.config()?.get_bool("core.ignoreCase").unwrap_or(false)))
        .unwrap_or(false)
}
