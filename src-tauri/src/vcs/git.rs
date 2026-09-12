//! git 后端。方法体逐条委托给 `engine::*`，不搬动 engine 里的任何代码。
//!
//! 之所以是薄委托而不是把 engine 的实现搬进来：`src-tauri/tests/` 下多个测试
//! 文件直接调 `app_lib::engine::graph::load` 这类路径，搬动会连带改测试，
//! 而 Phase 0 的验收标准就是现有测试一行不改还能全绿。

use super::Vcs;
use crate::engine;
use crate::engine::diff::{DiffData, StatusData};
use crate::engine::graph::GraphData;
use crate::engine::repo::RepoInfo;
use crate::engine::stash::StashEntry;
use crate::error::Result;

pub struct GitBackend {
    /// 工作目录根。原样保存调用方给的写法，不做规范化——
    /// `engine::*` 收到的一直是这个原始字符串，改写法会动到行为。
    root: String,
}

impl GitBackend {
    pub fn new(path: &str) -> Self {
        Self {
            root: path.to_string(),
        }
    }
}

impl Vcs for GitBackend {
    fn open(&self) -> Result<RepoInfo> {
        engine::repo::open(&self.root)
    }

    fn graph(&self) -> Result<GraphData> {
        engine::graph::load(&self.root)
    }

    fn status(&self) -> Result<StatusData> {
        engine::diff::read_status(&self.root)
    }

    fn diff(&self, scope: &str) -> Result<DiffData> {
        // engine 侧收 String，这里做一次转换。trait 用 &str 是为了调用方省事，
        // 多出来的这次分配相对一次 diff 计算可以忽略。
        engine::diff::read_diff(&self.root, scope.to_string())
    }

    fn stage(&self, paths: &[String]) -> Result<()> {
        let mut args: Vec<&str> = vec!["add"];
        args.extend(paths.iter().map(String::as_str));
        engine::git_cli::run_git(&self.root, &args)?;
        Ok(())
    }

    fn unstage(&self, paths: &[String]) -> Result<()> {
        let mut args: Vec<&str> = vec!["reset", "HEAD"];
        args.extend(paths.iter().map(String::as_str));
        engine::git_cli::run_git(&self.root, &args)?;
        Ok(())
    }

    fn discard(&self, paths: &[String], staged: bool) -> Result<()> {
        engine::ops::discard(&self.root, paths, staged)
    }

    fn commit(&self, message: &str, amend: bool) -> Result<()> {
        if amend {
            engine::git_cli::run_git(&self.root, &["commit", "--amend", "-m", message])?;
        } else {
            engine::git_cli::run_git(&self.root, &["commit", "-m", message])?;
        }
        Ok(())
    }

    fn create_branch(&self, name: &str, start_point: Option<&str>) -> Result<()> {
        let mut args = vec!["branch", name];
        if let Some(start) = start_point {
            args.push(start);
        }
        engine::git_cli::run_git(&self.root, &args)?;
        Ok(())
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        engine::git_cli::run_git(&self.root, &["branch", "-D", name])?;
        Ok(())
    }

    fn rename_branch(&self, old: &str, new: &str) -> Result<()> {
        engine::git_cli::run_git(&self.root, &["branch", "-m", old, new])?;
        Ok(())
    }

    fn checkout(&self, rev: &str) -> Result<()> {
        engine::git_cli::run_git(&self.root, &["checkout", rev])?;
        Ok(())
    }

    fn list_stashes(&self) -> Result<Vec<StashEntry>> {
        engine::stash::list(&self.root)
    }

    fn stash_push(&self, message: Option<&str>, include_untracked: bool) -> Result<()> {
        engine::stash::push(&self.root, message, include_untracked)
    }

    fn stash_apply(&self, index: usize) -> Result<()> {
        engine::stash::apply(&self.root, index)
    }

    fn stash_pop(&self, index: usize) -> Result<()> {
        engine::stash::pop(&self.root, index)
    }

    fn stash_drop(&self, index: usize) -> Result<()> {
        engine::stash::drop_entry(&self.root, index)
    }
}
