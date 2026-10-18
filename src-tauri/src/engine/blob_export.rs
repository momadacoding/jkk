//! 把 git 里的某个版本导出成临时文件，交给外部 diff / merge 工具。

use crate::engine::git_cli::run_git_bytes;
use crate::error::{GitError, Result};
use std::path::Path;

/// 按工作区形态导出一个 blob：经过 autocrlf / eol、`ident`、`working-tree-encoding`、
/// LFS smudge 等过滤器，和用户 checkout 出来的文件一致。
///
/// 这是对「读取走 libgit2」的有意偏离：libgit2 跑不了 LFS 这类外部过滤器。
/// 必须用 `run_git_bytes` 按字节原样写盘，`run_git` 的 `from_utf8_lossy` 会改坏 UTF-16、GBK。
/// git 失败时不写任何文件——拿空文件顶替，用户会以为那一侧是空的。
pub fn export_filtered(repo: &str, rel_path: &str, oid: &str, dest: &Path) -> Result<()> {
    if rel_path.is_empty() {
        return Err(GitError::new("external_tool_export", "导出路径为空"));
    }
    let path_arg = format!("--path={rel_path}");
    let bytes = run_git_bytes(repo, &["cat-file", "--filters", &path_arg, oid]).map_err(|e| {
        GitError::new(
            "external_tool_export",
            format!("导出 {rel_path} 失败：{}", e.message.trim()),
        )
    })?;
    std::fs::write(dest, bytes)?;
    Ok(())
}

/// 临时文件名：保留原扩展名，外部工具才能按类型选语法高亮和比较规则。
/// `Code/P.cs` + `LOCAL` → `P.LOCAL.cs`；`Makefile` → `Makefile.LOCAL`；`.gitignore` → `.gitignore.LOCAL`。
pub fn side_file_name(rel_path: &str, side: &str) -> String {
    let name = rel_path.rsplit(['/', '\\']).next().unwrap_or(rel_path);
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}.{side}.{ext}"),
        _ => format!("{name}.{side}"),
    }
}

/// 交给外部工具的真实文件必须是普通文件。
///
/// 用 `symlink_metadata` 而不是 `metadata`：后者会跟随链接，看到的是链接指向的文件，
/// 外部工具存盘时就顺着链接写到别处去了。
pub fn ensure_regular_file(abs: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(abs)?;
    if meta.file_type().is_file() {
        Ok(())
    } else {
        Err(GitError::new(
            "external_tool_not_regular",
            format!(
                "{} 不是普通文件（符号链接或目录），不能交给外部工具",
                abs.display()
            ),
        ))
    }
}
