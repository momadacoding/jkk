//! 纳入集合核对：证明「git 实际跟踪的集合 == 配置声明的集合」。
//!
//! 这是一条**行为断言**，不是针对某一种成因写的特判。它同时覆盖：
//! 工作树里的 `.gitignore` 盖掉了我们的再入规则（spec §6.3.1）、
//! `core.excludesFile`、用户手工改过的 `.git/info/exclude`、规则生成器自己的 bug。
//!
//! 与 §6.8 的基线体检**互不替代**：体检证明「磁盘 == 服务器」，这条证明
//! 「git 跟踪的集合 == 配置声明的集合」（spec §15 第 20 条）。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::Path;

/// 按配置遍历磁盘算出**期望**被 git 跟踪的文件集合（W-file，spec §6.3.2）。
///
/// 路径统一为仓库内相对路径 + 正斜杠，与 `git ls-files` 的输出形式对齐。
pub fn expected_set(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
) -> Result<BTreeSet<String>> {
    let root = Path::new(repo_root);
    let fold = ignore_case(repo_root);
    let mut out = BTreeSet::new();
    for raw in whitelist {
        let entry = raw.replace('\\', "/");
        let clean = entry.trim_end_matches('/');
        if clean.is_empty() {
            continue;
        }
        let abs = root.join(clean);
        // 白名单条目在磁盘上不存在时**不报错**：本函数的定义就是「磁盘上现在有什么」。
        // 「配置指向了不存在的路径」是另一条独立的校验，属初始化向导（Phase 1b）。
        if abs.is_file() {
            // 单文件条目不过扩展名过滤，与 `svn_exclude` 的第 ③ 段一致
            out.insert(clean.to_string());
            continue;
        }
        if abs.is_dir() {
            walk(&abs, root, extensions, fold, &mut out)?;
        }
    }
    Ok(out)
}

/// git 的忽略规则匹配是否大小写不敏感。
///
/// **读 git 自己的判断，而不是按平台猜。** Windows 上 `git init` 会把
/// `core.ignoreCase` 探测成 `true`，于是 `!/Code/**/*.cs` 连 `Loud.CS` 一起纳入；
/// 期望集合若死板地按大小写敏感算，就会在一个完全正常的仓库上把它报成
/// 「不该被跟踪却跟踪了」——**误报阻断**。同一台机器上仓库落在大小写敏感的卷、
/// 或用户显式设成 `false` 时，行为又反过来，所以只能问 git。
fn ignore_case(repo_root: &str) -> bool {
    // 键不存在时 `git config --get` 退出 1，走 run_git_code 免得被当成失败
    match run_git_code(repo_root, &["config", "--get", "core.ignoreCase"]) {
        Ok((_, out)) => out.trim().eq_ignore_ascii_case("true"),
        Err(_) => false,
    }
}

fn matches_ext(path: &str, extensions: &[String], fold: bool) -> bool {
    if extensions.iter().any(|e| e == "*") {
        return true;
    }
    if fold {
        let lower = path.to_lowercase();
        return extensions
            .iter()
            .any(|e| lower.ends_with(&e.to_lowercase()));
    }
    extensions.iter().any(|e| path.ends_with(e.as_str()))
}

fn walk(
    dir: &Path,
    root: &Path,
    extensions: &[String],
    fold: bool,
    out: &mut BTreeSet<String>,
) -> Result<()> {
    for ent in std::fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        let name = ent.file_name().to_string_lossy().to_string();
        // `.svn` 与 `.git` 在任何深度都不参与——前者是 SVN 的工作副本数据库，
        // 被 git 跟踪之后一次 checkout 就能损坏一个正在使用的工作副本（spec §6.3）。
        if name.eq_ignore_ascii_case(".svn") || name.eq_ignore_ascii_case(".git") {
            continue;
        }
        if p.is_dir() {
            walk(&p, root, extensions, fold, out)?;
        } else {
            let rel = p
                .strip_prefix(root)
                .map_err(|_| GitError::new("include_set", "路径不在仓库内"))?
                .to_string_lossy()
                .replace('\\', "/");
            if matches_ext(&rel, extensions, fold) {
                out.insert(rel);
            }
        }
    }
    Ok(())
}

/// git **实际**会跟踪的集合，限定在白名单路径下（W-path 当 pathspec）。
///
/// # 为什么是纯读，而不是 spec 字面写的「临时索引 + `git add -A`」
///
/// spec §6.3.1 说的是「在临时索引上 `git add -A -- <白名单>` 之后读 `ls-files`」，
/// 意图是「git 会跟踪谁」而**不要弄脏用户的暂存区**。
/// `--cached` ∪ `--others --exclude-standard` 给出同一个集合，却连临时索引文件和
/// 松散对象都不写：
///
/// - `--cached`：已经在索引里的（包括那些「已跟踪但现在被忽略规则命中」的文件
///   ——`git add -A` 同样会保留它们）
/// - `--others --exclude-standard`：还没进索引、也没被忽略规则挡住的，
///   即下一次 `git add -A` 会新收进来的那批。`--exclude-standard` 恰好就是
///   「每层 `.gitignore` + `.git/info/exclude` + 全局 excludesFile」这三源
///
/// 等价性不是推断——`the_actual_set_equals_what_git_add_would_have_produced`
/// 在真实仓库上把两条路径的结果对了一遍。
pub fn actual_set(repo_root: &str, whitelist: &[String]) -> Result<BTreeSet<String>> {
    if whitelist.is_empty() {
        // G6：空列表绝不能退化成「全部」——`git ls-files` 不带 pathspec
        // 会列出整个仓库，比对就变成了拿整仓库对空集。
        return Ok(BTreeSet::new());
    }
    let paths: Vec<String> = whitelist.iter().map(|w| w.replace('\\', "/")).collect();
    let mut set = ls_files(repo_root, &["--cached"], &paths)?;
    set.extend(ls_files(
        repo_root,
        &["--others", "--exclude-standard"],
        &paths,
    )?);
    Ok(set)
}

/// 跑一条 `git ls-files`，**始终带 `-z`**。
///
/// 不带 `-z` 时 git 按 `core.quotePath` 把非 ASCII 路径转义成
/// `"Code/\350\247\222\350\211\262.cs"` 这种带引号的八进制形式，而期望集合是遍历
/// 磁盘得到的原始字符串——两者永远对不上，每个装了中文文件名的工程都会在初始化时
/// 被误报阻断。`-z` 同时关掉引号转义并用 NUL 分隔，顺带也解决了文件名里含换行的
/// 情况。
fn ls_files(repo_root: &str, flags: &[&str], paths: &[String]) -> Result<BTreeSet<String>> {
    let mut args: Vec<&str> = vec!["ls-files", "-z"];
    args.extend_from_slice(flags);
    // `--` 必不可少：白名单条目是用户手写的，没有它一个以 `-` 开头的条目
    // 会被 git 当成选项解析（`AGENTS.md` §6.2）
    args.push("--");
    args.extend(paths.iter().map(String::as_str));
    let out = run_git(repo_root, &args)?;
    Ok(out
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// 核对两个集合，不一致则阻断并指认裁决者。
pub fn verify_include_set(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
) -> Result<()> {
    let expected = expected_set(repo_root, whitelist, extensions)?;
    let actual = actual_set(repo_root, whitelist)?;
    if expected == actual {
        return Ok(());
    }
    let missing: Vec<&String> = expected.difference(&actual).collect();
    let unexpected: Vec<&String> = actual.difference(&expected).collect();

    let mut msg = String::from("git 实际跟踪的文件集合与配置声明的不一致。\n");
    if !missing.is_empty() {
        msg.push_str("\n应当被跟踪、实际没有：\n");
        for f in missing.iter().take(20) {
            // 指认是哪条规则做的决定。git 会直接给出「文件:行号:模式」，
            // 比我们自己猜准得多。
            let who = blame_rule(repo_root, f).unwrap_or_else(|_| "（查不到裁决规则）".into());
            msg.push_str(&format!("  {f}   ← {who}\n"));
        }
        if missing.len() > 20 {
            msg.push_str(&format!("  …还有 {} 条\n", missing.len() - 20));
        }
    }
    if !unexpected.is_empty() {
        msg.push_str("\n不该被跟踪、实际跟踪了：\n");
        for f in unexpected.iter().take(20) {
            msg.push_str(&format!("  {f}\n"));
        }
        if unexpected.len() > 20 {
            msg.push_str(&format!("  …还有 {} 条\n", unexpected.len() - 20));
        }
    }
    Err(GitError::new("include_set_mismatch", msg))
}

/// 问 git「这个文件是被哪条规则决定的」。
///
/// `--non-matching` 让它对**没被忽略**的文件也给出裁决规则（形如
/// `.git/info/exclude:5:!/Code/**/*`），于是两个方向都能指认。
fn blame_rule(repo_root: &str, file: &str) -> Result<String> {
    let (_, out) = run_git_code(
        repo_root,
        &["check-ignore", "-v", "--non-matching", "--", file],
    )?;
    let line = out.lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        return Ok("（无匹配规则）".to_string());
    }
    Ok(line)
}
