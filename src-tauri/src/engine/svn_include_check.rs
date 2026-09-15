//! 纳入集合核对：证明「git 实际跟踪的集合 == 配置声明的集合」。
//!
//! 这是一条**行为断言**，不是针对某一种成因写的特判。它同时覆盖：
//! 工作树里的 `.gitignore` 盖掉了我们的再入规则（spec §6.3.1）、
//! `core.excludesFile`、用户手工改过的 `.git/info/exclude`、规则生成器自己的 bug。
//!
//! 与 §6.8 的基线体检**互不替代**：体检证明「磁盘 == 服务器」，这条证明
//! 「git 跟踪的集合 == 配置声明的集合」（spec §15 第 20 条）。

use crate::engine::git_cli::run_git_code;
use crate::engine::svn_baseline::TempIndex;
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
            let mut chain = vec![std::fs::canonicalize(&abs)?];
            walk(&abs, root, extensions, fold, &mut chain, &mut out)?;
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
///
/// # 必须让 git 自己做布尔解析（`--bool`）
///
/// git 认的真值不止字面量 `true`。2026-09-14 实测 `git config --bool` 的归一化：
///
/// | 配置值 | `--get` 原样返回 | `--bool --get` |
/// |---|---|---|
/// | `1` / `yes` / `on` / `True` | 原样 | `true` |
/// | `0` / `no` / `off` / `false` | 原样 | `false` |
/// | 非法值（如 `banana`） | 原样 | exit 128，stdout 空 |
///
/// 原来这里只认字符串 `"true"`，于是 `core.ignoreCase=1` 时：git 照常把
/// `Code/Loud.CS` 纳入（实测 `ls-files` 列它），期望集合却按大小写敏感算、
/// 不含它 → 核对报「不该被跟踪、实际跟踪了」并硬阻断，而仓库完全正常。
///
/// 非法值落到 `false`：stdout 为空，与「键不存在」同一条路径。git 自己也会
/// 在别处对这个值报错，这里不必再造一种失败形态。
fn ignore_case(repo_root: &str) -> bool {
    // 键不存在时 `git config` 退出 1，走 run_git_code 免得被当成失败
    match run_git_code(repo_root, &["config", "--bool", "--get", "core.ignoreCase"]) {
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

/// 递归遍历 `dir`，把符合扩展名的文件按仓库内相对路径收进 `out`。
///
/// # reparse point（Windows 的 junction 与符号链接）
///
/// **跟随它们，与 git 对齐。** 2026-09-13 实测：`Code/Mirror` 是一条指向仓库外
/// 目录的 junction 时，`git ls-files --others --exclude-standard -- Code/` 会列出
/// `Code/Mirror/Linked.cs` ——git 把 junction 当普通目录走进去了。所以这里也必须
/// 走进去；把它当成「一个文件」反而会让期望集合少一批文件、在一个正常仓库上误报。
///
/// （这与「git 把**符号链接**记成 blob、不递归」那条常识不矛盾：那条对
/// `core.symlinks=true` 下的真符号链接成立，而 Windows 上该项默认是 `false`，
/// junction 走的也不是那条路径。）
///
/// # 环
///
/// `chain` 是当前**祖先链**的规范路径。junction 指回自己的祖先时递归没有出口，
/// 栈溢出会让 Rust 直接 abort——在 Tauri 里就是整个应用凭空消失，连错误横幅都
/// 来不及显示（2026-09-13 独立 review 指出）。`canonicalize` 在 Windows 上会解开
/// reparse point，于是 `Code/Loop` 与 `Code` 得到同一个规范路径，环一眼就能看出来。
///
///
/// 只查祖先链、不做全局已访问集合：两条 junction 指向同一个目标是合法的，
/// git 会把两边的内容都列出来，全局去重会与它分叉。
///
/// 环存在时本函数与 git 仍会给出不同的集合（实测 git 会一路递归到 64 层才放弃，
/// 吐出一堆 `Code/Loop/Loop/…/Top.cs` 并打印 `could not open directory` 警告）。
/// 那种工作树本来就是坏的，**报一次集合不一致是正确行为**；这里要保证的只是
/// 「不崩」，不是「与一个自己都放弃了的遍历逐条对齐」。
fn walk(
    dir: &Path,
    root: &Path,
    extensions: &[String],
    fold: bool,
    chain: &mut Vec<std::path::PathBuf>,
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
        // `is_dir()` 会**跟随** junction，这是对的——见下方 `walk` 的 reparse 说明。
        if p.is_dir() {
            // 环保护：`canonicalize` 在 Windows 上会解开 reparse point，于是
            // `Code/Loop`（指回 `Code` 的 junction）与 `Code` 得到同一个规范路径。
            // 只查**祖先链**而不是全局已访问集合：两条 junction 指向同一个目标是
            // 合法的，git 会把两边的内容都列出来，全局去重会与它分叉。
            let canon = std::fs::canonicalize(&p)?;
            if chain.contains(&canon) {
                continue;
            }
            chain.push(canon);
            walk(&p, root, extensions, fold, chain, out)?;
            chain.pop();
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

/// 核对两个集合，不一致则阻断并指认裁决者。
///
/// # 实际集合来自调用方给的**候选索引**，不是这里现算的
///
/// spec §6.3.1：「核对与录制必须使用同一份候选索引」「不得核对一次后又重新
/// `git add` 另录一棵树」。所以本函数收一个已经 `add_all` 过的 [`TempIndex`]，
/// 核对完之后由调用方在**同一个** `idx` 上 `write_tree`。
///
/// 两次 `git add` 之间磁盘可能变了，于是核对通过的是 A、录进基线的是 B，而两者
/// 都不报错——这正是把索引当参数传进来而不是在函数里自建的理由。
///
/// # 这里曾经是一个「纯读模拟」，2026-09-14 删掉了
///
/// 1a 的实现用 `--cached ∪ --others --exclude-standard − --deleted` 三条只读
/// `ls-files` **模拟** `git add -A`，当时写明是对 spec 的偏离，理由是它连临时索引
/// 文件都不写。`TempIndex` 一到位，那个模拟就成了第二个会分叉的事实源——而 1a 的
/// 独立 review 已经证明「模拟 `git add -A`」很容易写错：最初只有前两条，
/// 漏掉的第三条会让「删了文件还没 stage」被误报成「不该被跟踪、实际跟踪了」。
/// 现在核对读的就是真索引，不存在等价性问题。
pub fn verify_include_set(
    repo_root: &str,
    idx: &TempIndex,
    whitelist: &[String],
    extensions: &[String],
) -> Result<()> {
    let expected = expected_set(repo_root, whitelist, extensions)?;
    let actual = idx.ls_files(repo_root)?;
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
            // 这一侧同样要指认规则——用户想知道的正是「凭什么它进来了」。
            // 这些文件必然已在索引里，所以 `blame_rule` 的 `--no-index` 是前提。
            let who = blame_rule(repo_root, f).unwrap_or_else(|_| "（查不到裁决规则）".into());
            msg.push_str(&format!("  {f}   ← {who}\n"));
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
///
/// # `--no-index` 不能省
///
/// 「多收」方向的文件**必然已经在索引里**，而 `git check-ignore` 默认会先看
/// 索引：已跟踪的文件它一律当成「没有规则管得着」。2026-09-14 实测同一个文件
/// （`Code/Junk.psd`，被 `/Code/**` 排除，用 `git add -f` 强行入索引）：
///
/// | | 输出 | exit |
/// |---|---|---|
/// | 无 `--no-index` | `::<TAB>Code/Junk.psd`（空裁决） | 1 |
/// | 有 `--no-index` | `.git/info/exclude:3:/Code/**<TAB>Code/Junk.psd` | 0 |
///
/// 也就是说，最需要指认规则的那一侧，不带这个参数恰好什么也指认不出来。
/// spec §6.3.1 已写明要带。
fn blame_rule(repo_root: &str, file: &str) -> Result<String> {
    let (_, out) = run_git_code(
        repo_root,
        &[
            "check-ignore",
            "--no-index",
            "-v",
            "--non-matching",
            "--",
            file,
        ],
    )?;
    let line = out.lines().next().unwrap_or("").trim().to_string();
    if line.is_empty() {
        return Ok("（无匹配规则）".to_string());
    }
    Ok(line)
}
