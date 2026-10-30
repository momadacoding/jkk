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
use crate::engine::svn_dirty::Fix;
use crate::engine::svn_gitignore::{query_verdicts, GitCtx};
use crate::engine::svn_scope::WScope;
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 白名单条目归一成「仓库内相对路径、正斜杠、无尾斜杠」。
pub(crate) fn normalize_roots(whitelist: &[String]) -> Vec<String> {
    whitelist
        .iter()
        .map(|w| w.replace('\\', "/").trim_end_matches('/').to_string())
        .filter(|w| !w.is_empty())
        .collect()
}

/// 按配置遍历磁盘算出**期望**被 git 跟踪的文件集合（W-file，spec §6.3.2）。
///
/// 路径统一为仓库内相对路径 + 正斜杠，与 `git ls-files` 的输出形式对齐。
pub fn expected_set(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
) -> Result<BTreeSet<String>> {
    expected_set_folded(repo_root, whitelist, extensions, ignore_case(repo_root))
}

/// [`expected_set`] 的本体，扩展名是否忽略大小写由调用方给。
///
/// 建库向导第 2 步还没有 `.git`，[`ignore_case`] 在工作副本里问不到 git 的判断——
/// 那时由探针 git 目录给出（`svn_gitignore::detect_conflicts`），必须与建库后的真仓库
/// 一致，否则第 2 步看到的清单与建库时拦下的不同。
pub(crate) fn expected_set_folded(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
    fold: bool,
) -> Result<BTreeSet<String>> {
    let root = Path::new(repo_root);
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

/// **服务器在 rN 上**白名单内的 W-file 集合——基线树该有的文件集合（spec §6.3.1）。
///
/// # 为什么不是「磁盘上的 W-file」
///
/// 初版写的是磁盘，因为那时每个录制入口都要求工作副本干净，两者恒等。§16.1 放松
/// 之后它们会分开，而基线的语义（§6.4）从头到尾说的都是**服务器**：
///
/// ```text
/// 服务器集合 = 磁盘 W-file  −  { 状态为 A / ? 的 }  +  { 状态为 D / ! 的 }
///                               服务器 rN 上还没有       磁盘上没有但服务器有
/// ```
///
/// **求法不走网络**，全部由本地的 `svn status` 推导。走网络的那条独立核对在
/// [`crate::engine::svn_server_set`]，它只出提示、不参与判定——两者不能互相顶替：
/// 这里挡的是「录错了哪些文件」，那里挡的是 `--set-depth=exclude` 这类磁盘上压根
/// 看不见的漏收。
///
/// 工作副本干净时 `fixes` 为空，退化成初版的「磁盘 W-file」，S7 / 恢复⑥ / D7a
/// 三个入口行为逐字不变。
///
/// # 「尊重 .gitignore」
///
/// 磁盘集合先按 [`WScope`] 去掉被工作树 `.gitignore` 排除的文件；`TakeBase` 补进来的
/// 路径同样过 [`WScope::filter`]。两步缺一步，期望集合就会与 `git add -A` 的实际行为分叉。
pub fn server_expected_set(
    repo_root: &str,
    scope: &WScope,
    fixes: &BTreeMap<String, Fix>,
) -> Result<BTreeSet<String>> {
    let mut set = expected_set(repo_root, scope.whitelist(), scope.extensions())?;
    scope.retain_not_ignored(&mut set)?;
    let take_base = scope.filter(
        fixes
            .iter()
            .filter(|(_, f)| **f == Fix::TakeBase)
            .map(|(p, _)| p.as_str()),
    )?;
    for (path, fix) in fixes {
        match fix {
            // 服务器 rN 上没有它。不在集合里时 `remove` 是 no-op——非 W-file
            // （比如一个未版本控制的 `.txt`）本来就不在磁盘集合里。
            Fix::Drop => {
                set.remove(path);
            }
            // 服务器上有。磁盘上还在的（`M` / `R` / `C`）本来就在集合里，
            // 这里真正补进来的是 `D` / `!` ——磁盘上已经没有、`expected_set`
            // 遍历磁盘时看不见的那些。
            //
            // **必须过 W-file 判据**：一个被删掉的 `Code/Notes.txt` 不该因为
            // 「服务器上有」就进基线树，它本来就不归 git 管。
            Fix::TakeBase => {
                if take_base.contains(path) {
                    set.insert(path.clone());
                }
            }
        }
    }
    Ok(set)
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
    scope: &WScope,
    fixes: &BTreeMap<String, Fix>,
) -> Result<()> {
    let expected = server_expected_set(repo_root, scope, fixes)?;
    let actual = idx.ls_files(repo_root)?;
    if expected == actual {
        return Ok(());
    }
    let missing: Vec<String> = expected.difference(&actual).cloned().collect();
    let unexpected: Vec<String> = actual.difference(&expected).cloned().collect();
    Err(GitError::new(
        "include_set_mismatch",
        mismatch_report(
            repo_root,
            "git 实际跟踪的文件集合与配置声明的不一致。",
            &missing,
            &unexpected,
        ),
    ))
}

/// `git add` **之前**的只读预检：按当前的忽略规则，git 将要收进来的文件与配置
/// 声明的是否一致。
///
/// # 为什么需要它
///
/// [`verify_include_set`] 排在 `git add -A` 之后，而 `git add` 要把白名单里每个
/// 文件哈希压缩写进 `.git/objects`。2026-10-27 用户在真实 Unity 工程（扩展名 `*`）
/// 上等了约半小时才看到 `include_set_mismatch`，随后回退删掉整个 `.git`，下次重试
/// 又是半小时。这里只遍历目录（`ls-files -o`），一个对象都不写。
///
/// # 它不替代最终核对
///
/// 两道比的是同一件事，但这一道读的是「git 将会收什么」，最终那道读的是「git
/// 实际收了什么」。磁盘可能在两者之间变化，录进基线的只认后者。这一道只负责
/// **早点失败**。
///
/// # 两侧都要套用与录制相同的脏清单修正
///
/// 最终核对的期望集合按 svn 脏清单修正过（[`server_expected_set`]），实际集合也被
/// 录制改写过：`Fix::Drop` 的路径被 `remove_path` 摘掉，`Fix::TakeBase` 的路径被
/// `replace_blob` **强行**放进索引——不管忽略规则怎么说。这里必须逐项复刻，
/// 否则一个「被 `.gitignore` 挡住的 `?` / `M` 文件」会被预检挡下，而最终核对
/// 本来会放行——误报阻断（2026-10-28 review 指出，`svn_init` 有用例钉住）。
/// `touched` 必须与录制用的是同一个集合（[`crate::engine::svn_dirty::explained_paths`]）。
///
/// # 与 `svn_sync_check::precheck_include_set` 不是一回事
///
/// 那一个走真的 `git add`、不套修正，给同步前的磁盘检查用。名字刻意错开。
pub fn dry_run_include_set(
    repo_root: &str,
    scope: &WScope,
    fixes: &BTreeMap<String, Fix>,
    touched: &BTreeSet<String>,
) -> Result<()> {
    let expected = server_expected_set(repo_root, scope, fixes)?;
    let idx = TempIndex::empty(repo_root)?;
    let mut actual = idx.ls_others(repo_root, scope.whitelist())?;
    // 录制会把这些补进候选索引（`TempIndex::add_paths`），预言里也要有
    actual.extend(scope.hidden_tracked_on_disk()?);
    for (path, fix) in fixes {
        if !touched.contains(path) {
            continue;
        }
        match fix {
            Fix::TakeBase => {
                actual.insert(path.clone());
            }
            Fix::Drop => {
                actual.remove(path);
            }
        }
    }
    if expected == actual {
        return Ok(());
    }
    let missing: Vec<String> = expected.difference(&actual).cloned().collect();
    let unexpected: Vec<String> = actual.difference(&expected).cloned().collect();
    Err(GitError::new(
        "include_set_mismatch",
        mismatch_report(
            repo_root,
            "录制基线前预检没通过：按当前的忽略规则，git 将要收进来的文件与配置声明的不一致。\
             还没开始录制就停下了。",
            &missing,
            &unexpected,
        ),
    ))
}

/// 每组列出的样例文件数。
const SAMPLES_PER_GROUP: usize = 3;
/// 最多展开的组数，其余合成一行。
const MAX_GROUPS: usize = 10;

/// 把两个方向的差异写成「按裁决规则分组」的报告。
///
/// # 为什么按规则分组，而不是逐个文件列
///
/// 原来是按字母序列前 20 个文件、每个后面跟一条规则。2026-10-27 用户在真实
/// Unity 工程上（扩展名 `*`）漏收了几百个 `.exe` / `.pyc`：前 20 条只覆盖了
/// 字母序最前的一小撮，「是哪几条规则、各挡了多少个」恰恰看不出来——而用户
/// 要动手改的正是那几条规则，不是那几百个文件。
///
/// 规则查询失败时**不阻止报告**：差异本身已经确定，规则只是帮用户定位的附加
/// 信息。查不到的路径落进「查不到裁决规则」，用户仍然看得到是哪些文件。
pub(crate) fn mismatch_report(
    repo_root: &str,
    headline: &str,
    missing: &[String],
    unexpected: &[String],
) -> String {
    let all: Vec<&str> = missing
        .iter()
        .chain(unexpected)
        .map(String::as_str)
        .collect();
    // 个别路径让 git fatal 时 `query_verdicts` 自己跳过续查；这里的 `Err` 只剩
    // 「连进程都起不来」——差异照样报，只是不带规则
    let verdicts = query_verdicts(&GitCtx::Repo(repo_root), &all).unwrap_or_else(|e| {
        log::warn!("批量查询裁决规则失败，报告里不带规则：{}", e.message);
        BTreeMap::new()
    });

    let mut msg = format!("{headline}\n");
    let mut blamed_gitignore = false;
    if !missing.is_empty() {
        msg.push_str(&format!(
            "\n应当被跟踪、实际没有：共 {} 个\n",
            missing.len()
        ));
        let groups = group_by(missing, |f| {
            if let Some(repo) = nested_repo(repo_root, f) {
                return format!(
                    "位于嵌套的 git 仓库 {repo} 里（git 只给它记一个子模块指针，不收里面的文件）"
                );
            }
            match verdicts.get(f) {
                Some(v) if v.ignores() => {
                    if v.in_worktree_gitignore() {
                        blamed_gitignore = true;
                    }
                    format!("被 {} 挡掉", v.describe(repo_root))
                }
                Some(v) if !v.pattern.is_empty() => {
                    format!(
                        "没有被忽略规则挡住（git 的裁决：{}），原因不明",
                        v.describe(repo_root)
                    )
                }
                Some(_) => "没有被忽略规则挡住（无匹配规则），原因不明".to_string(),
                None => "查不到裁决规则".to_string(),
            }
        });
        push_groups(&mut msg, groups);
    }
    if !unexpected.is_empty() {
        msg.push_str(&format!(
            "\n不该被跟踪、实际跟踪了：共 {} 个\n",
            unexpected.len()
        ));
        let groups = group_by(unexpected, |f| match verdicts.get(f) {
            // 子模块指针本身。里面的文件已经在「实际没有」那一侧点过名了
            _ if Path::new(repo_root).join(f).join(".git").exists() => {
                "是嵌套的 git 仓库（git 只给它记一个子模块指针）".to_string()
            }
            Some(v) if !v.pattern.is_empty() => format!("由 {} 纳入", v.describe(repo_root)),
            Some(_) => "没有匹配任何规则".to_string(),
            None => "查不到裁决规则".to_string(),
        });
        push_groups(&mut msg, groups);
    }
    if blamed_gitignore {
        msg.push_str(
            "\n工作树里的 .gitignore 优先级高于本应用生成的 .git/info/exclude。\
             把上面点名的 .gitignore 挪走、或删掉对应的行之后再重试。\n",
        );
    }
    msg
}

/// 按 `label` 分组，组按文件数从多到少排；同数按标签排，保证输出稳定。
fn group_by(
    files: &[String],
    mut label: impl FnMut(&str) -> String,
) -> Vec<(String, Vec<&String>)> {
    let mut map: BTreeMap<String, Vec<&String>> = BTreeMap::new();
    for f in files {
        map.entry(label(f)).or_default().push(f);
    }
    let mut groups: Vec<(String, Vec<&String>)> = map.into_iter().collect();
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then_with(|| a.0.cmp(&b.0)));
    groups
}

fn push_groups(msg: &mut String, groups: Vec<(String, Vec<&String>)>) {
    for (label, files) in groups.iter().take(MAX_GROUPS) {
        msg.push_str(&format!("  · {} 个{label}\n", files.len()));
        for f in files.iter().take(SAMPLES_PER_GROUP) {
            msg.push_str(&format!("      {f}\n"));
        }
        if files.len() > SAMPLES_PER_GROUP {
            msg.push_str(&format!(
                "      …还有 {} 个\n",
                files.len() - SAMPLES_PER_GROUP
            ));
        }
    }
    if groups.len() > MAX_GROUPS {
        let rest = &groups[MAX_GROUPS..];
        let n: usize = rest.iter().map(|(_, f)| f.len()).sum();
        msg.push_str(&format!("  …另有 {} 组，共 {n} 个\n", rest.len()));
    }
}

/// `rel` 的某个祖先目录是不是一个嵌套的 git 仓库，是则返回最外层那个目录。
///
/// 只看仓库根**以下**的目录：根自己的 `.git` 是我们的仓库。
fn nested_repo(repo_root: &str, rel: &str) -> Option<String> {
    let root = Path::new(repo_root);
    let mut dir = String::new();
    let segs: Vec<&str> = rel.split('/').collect();
    for seg in &segs[..segs.len().saturating_sub(1)] {
        if !dir.is_empty() {
            dir.push('/');
        }
        dir.push_str(seg);
        if root.join(&dir).join(".git").exists() {
            return Some(dir);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::mismatch_report;
    use crate::engine::git_cli::run_git;
    use crate::engine::svn_gitignore::display_path;

    /// 第二轮 review（2026-10-28）：裁决规则一次批量查，只要有一个路径让
    /// `check-ignore` fatal，原实现整批落进「查不到裁决规则」。
    ///
    /// 真实的触发是白名单里的**目录符号链接**（`pathspec … is beyond a symbolic
    /// link`）。本机建符号链接要管理员权限，这里换成 git 同样会 fatal 的
    /// 「仓库外路径」，只钉「一个 fatal 不连累别的路径」这件事；符号链接本身的
    /// 形态由 `tests/svn_exclude.rs` 里 unix 专属的用例覆盖。
    #[test]
    fn a_path_git_refuses_to_check_does_not_cost_the_others_their_verdicts() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path().to_str().unwrap();
        run_git(root, &["init", "-q"]).unwrap();
        std::fs::create_dir_all(dir.path().join("Code/Tools")).unwrap();
        std::fs::write(dir.path().join("Code/.gitignore"), "*.exe\n").unwrap();

        // fatal 的那条夹在中间：它之前、之后的路径都必须拿到裁决
        let missing = [
            "Code/Tools/a.exe".to_string(),
            "../outside.exe".to_string(),
            "Code/Tools/b.exe".to_string(),
        ];
        let unexpected = ["Code/Keep.cs".to_string()];
        let m = mismatch_report(root, "h", &missing, &unexpected);

        let want = format!(
            "2 个被 {} 第 1 行 `*.exe` 挡掉",
            display_path(root, "Code/.gitignore")
        );
        assert!(m.contains(&want), "fatal 前后的路径都要拿到裁决，实际：{m}");
        assert!(m.contains("1 个查不到裁决规则"), "实际：{m}");
        assert!(
            m.contains("1 个没有匹配任何规则"),
            "另一侧的裁决也不能被连累，实际：{m}"
        );
    }
}
