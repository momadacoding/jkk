//! §16 R2a：把 `svn status` 的条目分成「录基线时要怎么处理」。
//!
//! # 这一层为什么单独存在
//!
//! 基线的语义是「白名单内的文件在 revision N 时**服务器上的样子**」（§6.4）。
//! 工作副本干净时磁盘就等于那个样子；放松了「必须干净」之后（§16.1），每个脏
//! 条目都要单独回答一个问题：**服务器 rN 上这个位置是什么？**
//!
//! | 磁盘上的状态 | 服务器 rN 上是什么 | 怎么办 |
//! |---|---|---|
//! | 改过 / 替换过 / 冲突中 | 一份旧内容 | 取回 BASE |
//! | 本地删了 / 文件丢了 | 还在 | 取回 BASE |
//! | 本地新增（已 add 或未版本控制） | **没有** | 从树里剔除 |
//! | 只改了属性 | 同一份内容 | 不用动 |
//!
//! ⚠️ **这张表是按文件说的，而 svn 会按目录报。** 两边粒度对不上时整棵子树都会
//! 掉进缝里——见 [`read_dirty`] 文档里那张实测表，那是 2026-09-20 两条阻断项的
//! 根因。
//!
//! 判错一条就是一个假基线，而假基线看起来完全正常——这是整套设计里最贵的那类
//! 错误（§6.8.1）。所以这里对没见过的 item **报错而不是跳过**。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_scope::WScope;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_info_schedules, parse_status_entries, StatusEntry};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 读出 `targets` 覆盖范围内所有**非 pristine** 的条目。**不做任何阻断判定。**
///
/// 三个参数缺一不可（spec §6.5）：
///
/// ```text
/// svn status --no-ignore --ignore-externals --xml -- <target>
///            ↑ 不带它会漏掉 SVN-ignored 文件，而其中匹配扩展名的那些 git 照收
///                        ↑ 不带它 external 内部的节点会混进来（§6.6.4）
/// ```
///
/// `targets` 必须是白名单根**加上**其底下每一条嵌套 external 的根。只传白名单根
/// 的话，external 里的改动在这条命令下只显示成一条 `item="external"`，而下面正好
/// 把它跳过——等于完全没查（spec §6.5.5 / §6.6.5）。
///
/// # ⚠️ svn 是按**目录**报的，而 git 是按**文件**递归收的
///
/// 2026-09-20 实测（独立 review 的两条阻断项）。`svn status` 对目录级的脏状态
/// 只报目录本身：
///
/// | 形态 | 不带 `-v` | 带 `-v` | 目录还在磁盘上吗 |
/// |---|---|---|---|
/// | 未版本控制的目录 | 只报目录 | **只报目录**（不下沉） | 在 |
/// | `svn:ignore` 的目录 | 只报目录 | **只报目录**（不下沉） | 在 |
/// | `svn delete` 的目录 | 只报目录 | **目录 + 每个子文件都报 `deleted`** | 不在 |
/// | `svn add` 未提交的目录 | 目录 + 子文件都报 | 同左 | 在 |
/// | **junction / 符号链接**（2026-09-21 补） | 只报链接本身 | **只报链接本身** | 在 |
///
/// 而 git 侧的 exclude 规则是 `!/<白名单>/**/*.<扩展名>`——**按扩展名递归**。
/// 两边粒度不一致，中间那些文件就会掉进缝里：
///
/// - 未版本控制 / ignored 的目录 → 里面的 `.cs` 被 `git add` 收进基线树，而
///   分类层只认得那一条目录路径 ⇒ 服务器上不存在的文件混进一个自称「等于
///   服务器 rN」的树，**四道检查全绿**（假基线，且静默）；
/// - `svn delete` 的目录 → 子文件没被取回，而 `svn diff --summarize` 那一侧
///   **会**报它们是 deleted ⇒ `D \ S != ∅` ⇒ 初始化 / 重建永远失败，
///   错误文案还把原因指向「漏跑了 svn update」。
///
/// 所以这里做两件事：**加 `-v`**（把被删目录的子文件捞出来），以及对
/// 未版本控制 / ignored 的目录**自己走一遍磁盘**（`-v` 帮不上忙）。
///
/// # junction 是同一个病的第四种形态（2026-09-21，独立 review）
///
/// 表里最后那一行的代价和前两行一样，只是更隐蔽：**Windows 的目录联接在 Rust
/// 的 `symlink_metadata().is_dir()` 下是 `false`**，于是下面那道「是不是目录」
/// 的判断连 `expand_dir` 都不会调用，而 `git add` 照样走进去。2026-09-21 实测
/// （`Code/Mirror` 是指向工作副本外的 junction）：
///
/// ```text
/// svn status --no-ignore -v -- Code   →   ?  Code\Mirror      （就这一行）
/// symlink_metadata("Code/Mirror")     →   is_dir()=false, is_symlink()=true
/// git add -A -- Code/                 →   索引里出现 Code/Mirror/Linked.cs
/// ```
///
/// 所以磁盘遍历这一侧**必须跟随 junction**，与 `git` 和
/// [`crate::engine::svn_include_check`] 的 `walk` 对齐——那边早就是跟随的
/// （2026-09-13 实测并有测试钉住），两侧不一致正是这条缺口的成因。
/// 跟随就必须自带环保护，见 [`expand_dir`]。
///
/// # 为什么读与判分开
///
/// 两个调用方要的东西不一样：初始化 / 重建要在 `~` 上**阻断**，而打开仓库时的
/// 服务器侧核对（[`crate::engine::svn_server_set`]）只是想知道哪些文件是本地删的，
/// 不该因为一处 obstructed 就让仓库打不开。判定留给各自的调用点。
pub fn read_dirty(repo_root: &str, targets: &[String]) -> Result<Vec<StatusEntry>> {
    let mut dirty: Vec<StatusEntry> = Vec::new();
    for target in targets {
        let arg = peg_safe(target);
        let xml = run_svn(
            repo_root,
            &[
                "status",
                // `-v` 是为了**被 `svn delete` 掉的目录**：不带它，`svn status`
                // 只报目录本身，里面那些服务器上还在的文件一条都拿不到。
                // 代价是所有干净条目也一并返回，下面按 `normal` + `props=none`
                // 滤掉；解析一遍的开销远小于漏掉整棵子树。
                "-v",
                "--no-ignore",
                "--ignore-externals",
                "--xml",
                "--",
                &arg,
            ],
        )?;
        for e in parse_status_entries(&xml)? {
            // `item="external"` 即使带了 `--ignore-externals` 也照样出现（实测），
            // 它不是脏状态。本设计的核心场景就是「代码在 external 里」，把它判脏
            // 会让向导在目标仓库上永远过不去。
            if e.item == "external" {
                continue;
            }
            // `normal` + `props == "none"` 才是真干净。只改了属性的节点是
            // `item="normal" props="modified"`（2026-09-14 实测），它**要**进清单：
            // 本函数的定义是「非 pristine」。[`classify`] 再把它判成「不用动」。
            //
            // 复制来的干净文件也是 `normal` + `none`，但它**不**等于服务器内容：服务器
            // rN 上根本没有这个路径（2026-11-06 实测）。丢掉它就等于宣称它干净，
            // 于是按磁盘内容录进基线。
            if e.item == "normal" && e.props == "none" && !e.copied {
                continue;
            }
            // 未版本控制 / ignored 的**目录**：svn 连 `-v` 都不下沉进去，
            // 必须自己走一遍磁盘，否则里面的 W-file 会被 git 收进基线树而
            // 分类层完全不知道（见本函数文档里那张表）。
            //
            // 目录条目本身**保留**在清单里：这个函数的契约是「如实反映 svn
            // 说了什么」，删掉它等于替下游做判断；而它不是 W-file，
            // `explained_paths` 自然会把它滤掉。
            if matches!(e.item.as_str(), "unversioned" | "ignored")
                && is_dir_on_disk(repo_root, &e.path)
            {
                expand_dir(repo_root, &e, &mut dirty)?;
            }
            dirty.push(e);
        }
    }
    // 同一条路径不该来自两个 target（`--ignore-externals` 让白名单根那次查询不会
    // 下沉进 external），但去重的代价近乎为零，而重复会让 `S` 多出条目。
    dirty.sort_by(|a, b| a.path.cmp(&b.path));
    dirty.dedup_by(|a, b| a.path == b.path);
    fill_schedules(repo_root, &mut dirty)?;
    Ok(dirty)
}

/// 一次 `svn info` 最多带多少个路径。Windows 命令行上限约 32K 字符，路径按 200 字符
/// 估也留足了余量。
const INFO_BATCH: usize = 50;

/// 给 `missing` + `revision="-1"` 的条目补上 `svn info` 的 schedule（见 [`classify`]）。
///
/// 只读、不联网（`svn info` 对工作副本路径读的是 wc.db）。这类条目通常没有，没有时
/// 不起进程——空参数列表的 `svn info` 会去查当前目录，不能交给它。
///
/// # 非 ASCII 路径不能当参数传
///
/// svn.exe 按系统 ANSI 代码页取命令行参数，ACP 1252 上中文变成 `?`、目标找不到（见
/// `svn_health` 的实测记录；本函数的 fixture 测试也在开发机上撞到了）。这类路径改问它
/// **最深的纯 ASCII 祖先目录**，带 `-R`：输出的 XML 是 UTF-8，与代码页无关，再按路径
/// 挑出要的那几条。祖先至少是 `read_dirty` 的 target 本身——target 是参数，若含
/// 非 ASCII 字符，前面的 `svn status` 已经失败了。
fn fill_schedules(repo_root: &str, dirty: &mut [StatusEntry]) -> Result<()> {
    let want: Vec<usize> = dirty
        .iter()
        .enumerate()
        .filter(|(_, e)| e.item == "missing" && e.revision == Some(-1))
        .map(|(i, _)| i)
        .collect();
    let (ascii, wide): (Vec<usize>, Vec<usize>) =
        want.iter().partition(|&&i| dirty[i].path.is_ascii());

    let mut found: BTreeMap<String, Option<String>> = BTreeMap::new();
    for chunk in ascii.chunks(INFO_BATCH) {
        let targets: Vec<String> = chunk.iter().map(|&i| peg_safe(&dirty[i].path)).collect();
        let mut args: Vec<&str> = vec!["info", "--xml", "--"];
        args.extend(targets.iter().map(String::as_str));
        found.extend(info_schedules(repo_root, &args, chunk.len())?);
    }
    let ancestors: BTreeSet<String> = wide
        .iter()
        .map(|&i| ascii_ancestor(&dirty[i].path))
        .collect();
    for anc in &ancestors {
        let target = peg_safe(anc);
        let args = ["info", "-R", "--xml", "--", target.as_str()];
        found.extend(info_schedules(repo_root, &args, wide.len())?);
    }

    for i in want {
        // 没对上的留 `None`：classify 把它当成「说不清」并停下，而不是猜
        dirty[i].schedule = found.get(&dirty[i].path).cloned().flatten();
    }
    Ok(())
}

/// 跑一次 `svn info --xml` 并取出 schedule。失败时说清是哪一步：这个错误会一路冒到
/// 初始化 / 跟进 / 体检的界面上，svn 的原话单独看不出它和录基线有什么关系。
fn info_schedules(
    repo_root: &str,
    args: &[&str],
    count: usize,
) -> Result<Vec<(String, Option<String>)>> {
    let wrap = |e: GitError| {
        GitError::new(
            "svn_dirty_info_failed",
            format!(
                "有 {count} 个文件从磁盘上丢了、svn 记录它们还没提交过。为了分清它们是新加的\
                 还是顶替了服务器上的文件，要用 svn info 查一下，这一步失败了：{}",
                e.message
            ),
        )
    };
    let xml = run_svn(repo_root, args).map_err(wrap)?;
    parse_info_schedules(&xml).map_err(wrap)
}

/// 路径里从头开始、每一段都是纯 ASCII 的最长前缀（不含最后一段——那是要问的文件自己）。
/// 一段都没有时退到 `.`。
fn ascii_ancestor(path: &str) -> String {
    let segs: Vec<&str> = path.split('/').collect();
    let parents = &segs[..segs.len().saturating_sub(1)];
    let n = parents.iter().take_while(|s| s.is_ascii()).count();
    if n == 0 {
        ".".to_string()
    } else {
        parents[..n].join("/")
    }
}

/// 这个相对路径在磁盘上**是不是一个目录**——junction / 符号链接**跟随**。
///
/// # 这里曾经用 `symlink_metadata`，那是错的
///
/// 原来的写法是 `symlink_metadata(..).is_dir()`，理由写的是「不跟随链接，免得
/// 一条指回自己祖先的 junction 让遍历转不出来」。环的顾虑没错，**但把它放在这道
/// 门上是把整扇门关死了**：2026-09-21 实测，Windows 目录联接在
/// `symlink_metadata().is_dir()` 下就是 `false`，于是一条未版本控制的 junction
/// 连 [`expand_dir`] 都进不去，而 `git add` 照样走进去把里面的 `.cs` 收进基线树
/// ——服务器上没有的文件进了一棵自称「等于服务器 rN」的树，且四道检查全绿。
///
/// 环该在遍历里用祖先链解决（[`expand_dir`] 里就是这么做的，与
/// [`crate::engine::svn_include_check`] 的 `walk` 同一套），不该靠「干脆不进去」。
///
/// 目标不存在的悬空链接这里是 `false`：进不去也没什么可摊平的，而 `git add`
/// 同样收不到它。
fn is_dir_on_disk(repo_root: &str, rel: &str) -> bool {
    Path::new(repo_root).join(rel).is_dir()
}

/// 把一个未版本控制 / ignored 的目录，摊平成它底下每一个**文件**的条目。
///
/// 合成出来的条目沿用目录的 `item`（`unversioned` 或 `ignored`），因为服务器 rN
/// 上这整棵子树都不存在——每一个都该被 [`Fix::Drop`]。`props` 填 `none`：
/// 未纳入版本控制的路径没有 svn 属性可言，而 [`classify`] 也只看 `item`。
///
/// # 出错就报错，不跳过
///
/// 读不动某个子目录（权限、被占用）时返回 `Err`。悄悄跳过等于宣称「这里没有
/// W-file」，而那正好是本函数要消灭的那种静默假基线。调用方各自决定怎么处理：
/// 初始化 / 重建会中止，服务器侧核对退成 `NotChecked`。
///
/// # junction / 符号链接：**跟随**，并用祖先链防环
///
/// 与 `git` 以及 [`crate::engine::svn_include_check`] 的 `walk` 对齐。这里原来
/// 是 `if ft.is_symlink() { continue }`，理由写的是「链接目标要么在这棵树里、
/// 要么在外面不归白名单管」——**那句话对 git 不成立**：2026-09-21 实测，
/// `git add -A -- Code/` 会把 `Code/NewDir/Mirror/Linked.cs` 收进索引，哪怕
/// `Mirror` 指到工作副本外面去。不跟随就漏掉这些条目的 [`Fix::Drop`]，于是它们
/// 留在基线树里冒充服务器内容。
///
/// `chain` 是当前**祖先链**的规范路径，与 `walk` 同一套：`canonicalize` 在
/// Windows 上会解开 reparse point，于是一条指回自己祖先的 junction 与那个祖先
/// 得到同一个规范路径，环一眼就能看出来。只查祖先链、不做全局已访问集合——
/// 两条 junction 指向同一个目标是合法的，git 会把两边的内容都列出来。
fn expand_dir(repo_root: &str, dir: &StatusEntry, out: &mut Vec<StatusEntry>) -> Result<()> {
    // 起点自己也要进祖先链，否则 `Code/NewDir/Loop -> Code/NewDir` 这种
    // 「指回自己」的环在第一层就转不出来。
    let mut chain = vec![canon(repo_root, &dir.path)?];
    walk_expand(repo_root, &dir.path, &dir.item, &mut chain, out)
}

/// 一个相对路径的规范形式，失败映射成与遍历同一个错误码。
fn canon(repo_root: &str, rel: &str) -> Result<std::path::PathBuf> {
    std::fs::canonicalize(Path::new(repo_root).join(rel)).map_err(|e| {
        GitError::new(
            "svn_dirty_walk_failed",
            format!(
                "取不到 `{rel}` 的规范路径：{e}。跟随目录联接时要靠它发现环，\
                 取不到就没法保证遍历会停下来。"
            ),
        )
    })
}

fn walk_expand(
    repo_root: &str,
    rel: &str,
    item: &str,
    chain: &mut Vec<std::path::PathBuf>,
    out: &mut Vec<StatusEntry>,
) -> Result<()> {
    let abs = Path::new(repo_root).join(rel);
    let rd = std::fs::read_dir(&abs).map_err(|e| {
        GitError::new(
            "svn_dirty_walk_failed",
            format!(
                "读不动 `{rel}`：{e}。它是一个未纳入版本控制的目录，\
                 里面可能有会被 git 收进基线的源码文件，没读完就不能下结论。"
            ),
        )
    })?;
    for ent in rd {
        let ent = ent.map_err(|e| {
            GitError::new(
                "svn_dirty_walk_failed",
                format!("枚举 `{rel}` 的内容时失败：{e}"),
            )
        })?;
        let ft = ent.file_type().map_err(|e| {
            GitError::new(
                "svn_dirty_walk_failed",
                format!("读不到 `{rel}` 里某一项的类型：{e}"),
            )
        })?;
        let name = ent.file_name().to_string_lossy().into_owned();
        // `.svn` / `.git` 不该出现在未版本控制的目录里，但真出现了也绝不能
        // 跟进去——那是另一套元数据，不是源码。
        if name == ".svn" || name == ".git" {
            continue;
        }
        let child = format!("{rel}/{name}");
        // `Path::is_dir()` **跟随**链接，这是对的——见本函数文档的 junction 一节。
        // 悬空链接两个分支都不落，与改动前一致：`git add` 同样收不到它。
        if ent.path().is_dir() {
            let c = canon(repo_root, &child)?;
            if chain.contains(&c) {
                continue;
            }
            chain.push(c);
            walk_expand(repo_root, &child, item, chain, out)?;
            chain.pop();
        } else if ft.is_file() {
            out.push(StatusEntry {
                path: child,
                item: item.to_string(),
                props: "none".to_string(),
                wc_locked: false,
                // 未纳入版本控制的路径 svn 不给 revision，也不可能是复制来的
                revision: None,
                copied: false,
                schedule: None,
            });
        }
    }
    Ok(())
}

/// 把分类结果收敛成体检判据里的 `S`（spec §6.8.7）。
///
/// 只保留 **W-file**：不归 git 管的路径（白名单目录里一个未版本控制的 `.txt`、
/// 一个改脏的 `.psd`）在基线树里本来就不存在，录制时对它们动手是 no-op。
/// 把它们记进 `S` 不会让任何东西变对，却会让 `D \ S` 变小——判据被悄悄削弱，
/// 而不会有任何测试因此变红（§16.1「S 必须恰好」）。
///
/// 抽成一个函数是因为**三个地方都要算它**：录制时（决定改写哪些路径）、录制后
/// 的体检、以及用户手动点体检。三份各写一遍，迟早有一份会和另外两份不一致，
/// 而不一致的方向若是「多算了」，判据就被削弱了且没有信号。
///
/// 「尊重 .gitignore」时，被它排除的脏文件同样不在树里，不进 `S`（[`WScope`]）。
pub fn explained_paths(fixes: &BTreeMap<String, Fix>, scope: &WScope) -> Result<BTreeSet<String>> {
    scope.filter(fixes.keys().map(String::as_str))
}

/// 录基线时要对这个路径做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// 把树里的内容换成 `svn cat -r BASE` 的字节。
    TakeBase,
    /// 从树里摘掉——服务器 rN 上没有这个文件。
    Drop,
}

/// 把脏清单分类。返回的**只有需要动的路径**，干净条目不出现。
///
/// 返回值同时就是体检判据里的 `S`（§6.8.7），所以它必须**恰好**是会被改写的那批
/// 路径：多一条，`D \ S` 就小一分，判据被悄悄削弱，而不会有任何测试变红。
/// `BTreeMap` 保证有序且去重。
///
/// # 「服务器上有没有」不能只看 item
///
/// 2026-11-06 在 fixture 上实测，下面几种 item 看着像「服务器上有」，其实没有，
/// 而 `svn cat -r BASE` 要么报 E200009（录制中止），要么**成功返回复制源的内容**
/// （静默录出假基线）：
///
/// | 工作副本里做了什么 | item | revision | copied |
/// |---|---|---|---|
/// | `svn add` 后又从磁盘删掉 | `missing` | `-1` | — |
/// | `svn cp` / `svn mv` 出来的目录里的干净文件 | `normal` | 没有 | `true` |
/// | 同上，改过的文件 | `modified` | 没有 | `true` |
/// | 同上，从磁盘删掉的文件 / 整个子目录 | `missing` | `-1` | — （标记消失） |
/// | `svn rm X; svn cp Y X` | `replaced` | 没有 | `true` |
///
/// 所以先按「所在的复制子树」判（见 [`copy_root`]），再落到按 item 的映射。
/// 最后一行是另一回事：服务器上**有** `X` 的旧内容，但离线取到的是 `Y`，只能停下。
///
/// # `missing` + `revision="-1"` 要看 `schedule`
///
/// 不在复制子树里的这种条目有三种来源，`svn status` 分不开（独立 review 实测），
/// 只有 `svn info` 的 `<schedule>` 分得开，由 [`read_dirty`] 补进 [`StatusEntry::schedule`]：
///
/// | 工作副本里做了什么 | schedule | 服务器 rN 上 | 怎么办 |
/// |---|---|---|---|
/// | `svn add` 后又从磁盘删掉 | `add` | 没有 | Drop |
/// | `svn rm X` 后同名 `svn add`（或 `svn cp`），再从磁盘删掉 | `replace` | **有**旧内容 | 停下 |
/// | 其他（没查到 / 没见过的值） | — | 说不清 | 停下 |
///
/// 只有正面证据（`add`）才摘：误摘的代价是基线静默少一个服务器上的文件，比报错贵。
/// 也绝不落回 TakeBase——`revision="-1"` 的节点 `svn cat -r BASE` 要么报 E200009，
/// 要么在复制子树里返回复制源的内容。
pub fn classify(entries: &[StatusEntry]) -> Result<BTreeMap<String, Fix>> {
    let mut out: BTreeMap<String, Fix> = BTreeMap::new();
    // 路径（已归一）→ 条目。用来顺着祖先链找每个节点所在复制子树的根
    let by_path: BTreeMap<String, &StatusEntry> = entries
        .iter()
        .map(|e| (e.path.replace('\\', "/"), e))
        .collect();

    for e in entries {
        let path = e.path.replace('\\', "/");
        if path.trim().is_empty() {
            return Err(GitError::new(
                "svn_dirty_empty_path",
                "脏清单里出现了空路径。它会变成一条空 pathspec 的 git 调用，\
                 而本仓库已经因为空 pathspec 误伤过一次（AGENTS.md §6.2）。",
            ));
        }

        // 只对受版本控制的节点问「服务器上有没有」。未版本控制 / ignored 本来就是 Drop，
        // external 标记与 obstructed 交给下面原有的分支。
        if matches!(
            e.item.as_str(),
            "modified" | "replaced" | "conflicted" | "deleted" | "missing" | "normal" | "added"
        ) {
            if let Some((root, root_item)) = copy_root(&path, &by_path) {
                match root_item {
                    // 复制根是新加的：整棵子树服务器 rN 上都没有
                    "added" => {
                        out.insert(path, Fix::Drop);
                        continue;
                    }
                    "replaced" => return Err(copied_over_server_content(&path, &root)),
                    _ => return Err(copy_origin_unknown(&path, &root, root_item)),
                }
            }
            if e.item == "missing" && e.revision == Some(-1) {
                match e.schedule.as_deref() {
                    Some("add") => {
                        out.insert(path, Fix::Drop);
                        continue;
                    }
                    Some("replace") => return Err(replaced_then_missing(&path)),
                    other => return Err(missing_uncommitted_unknown(&path, other)),
                }
            }
            // 不带历史的替换（事故分析 6.2）：带历史的已经在上面按复制子树处理了
            if e.item == "replaced" {
                return Err(replaced_without_history(&path));
            }
        }

        let fix = match e.item.as_str() {
            // 磁盘上的不是服务器内容。`deleted` / `missing` 是磁盘上干脆没有——
            // 它们仍然要取回来，因为基线记的是**服务器**上的样子，本地的删除是
            // 一处待交付的改动，不是基线的一部分。
            // `replaced` 走不到这里：带历史的按复制子树判，不带历史的上面已经停下
            "modified" | "conflicted" | "deleted" | "missing" => Fix::TakeBase,

            // 服务器 rN 上根本没有它们。`ignored` 与 `unversioned` 都算：
            // 被 `svn:ignore` 挡住不代表 git 不收它（正是 `--no-ignore` 存在的
            // 理由，§6.5 P1），而 git 收了它就会进基线树。
            "added" | "unversioned" | "ignored" => Fix::Drop,

            // 干净，或者只改了属性（`item="normal" props="modified"`）。
            //
            // 属性变更**故意**落在这里：内容等于服务器内容，换不换字节都一样，
            // 而让它进结果就等于让它进 `S`，判据随之被削弱（§16.1「S 必须恰好」）。
            "normal" => continue,

            // `--ignore-externals` 也拦不住这条标记（实测）。它不是脏状态，
            // external 内部的节点由 `check_targets` 作为独立 target 各查一次。
            "external" => continue,

            // P1 在 §16.1 之后唯一仍然阻断的状态，不该走到这一层。
            "obstructed" => {
                return Err(GitError::new(
                    "svn_dirty_obstructed",
                    format!(
                        "`{path}` 处在 obstructed 状态（svn status 的 `~`）：\
                         那个位置上本该是文件的成了目录、或者反过来。\
                         svn 自己都说不清它是什么，也就取不到它在服务器上的内容。\
                         请先在 SVN 客户端里把这个路径理顺，本应用不会替你动它。"
                    ),
                ))
            }

            other => {
                return Err(GitError::new(
                    "svn_dirty_unknown_item",
                    format!(
                        "不认识的 svn status item：`{other}`（路径 `{path}`）。\
                         默认当成「不用动」会把这个文件按**磁盘内容**录进基线，\
                         而基线声称自己是服务器 rN 的内容——那正是本期要消灭的\
                         那种假基线，且没有任何信号。所以这里停下来。"
                    ),
                ))
            }
        };

        out.insert(path, fix);
    }

    Ok(out)
}

/// `path` 所在复制子树的根（路径与 item）：从 `path` 自己往上走，带 `copied` 的记为
/// 候选，取链上**最上层**那个。
///
/// # 链怎么走
///
/// - 带 `copied` 的节点：记为候选，继续往上。取最上层而不是最近的：复制出来的目录里
///   再 `svn cp` 一份，决定性的是最外层那次复制
/// - `missing` + `revision="-1"`：**穿过去**。复制子树里从磁盘删掉的文件、整个子目录
///   都会丢掉 copied 标记、只剩这个形状（实测），它们不能把链截断——否则
///   `C/sub/x.cs` 找不到 `C`，而 `svn cat -r BASE` 会返回复制源的内容
/// - 其他节点，或者清单里没有（干净的、未复制的节点被 [`read_dirty`] 滤掉了）：**停**。
///   这是 external 的边界：带 external 的目录被复制后，external 里的节点没有 copied
///   标记、服务器上（external 的源）也确实有，不能被外面那次复制吞进去（独立 review 实测）
///
/// 被穿过的**祖先**若带着 schedule，它本身就可能是复制根：整个复制目录从磁盘删掉后
/// （`svn mv E M; rm -rf M` 很常见），根是 `missing -1` + `add`、子节点是 `missing -1` +
/// `normal`（独立 review 实测）。`add` 记成 added 的根，`replace` 记成 replaced 的根。
/// 只看祖先、不看自己：自己是 `missing -1` 的情况由 [`classify`] 按 schedule 单独判，
/// 报错文案更准。
///
/// 按路径段走，不按字符串前缀：`Code/B` 是复制来的，不代表 `Code/Bar.cs` 也是。
fn copy_root<'a>(
    path: &str,
    by_path: &BTreeMap<String, &'a StatusEntry>,
) -> Option<(String, &'a str)> {
    let mut root: Option<(String, &'a str)> = None;
    let mut cur = path;
    loop {
        match by_path.get(cur) {
            Some(e) if e.copied => root = Some((cur.to_string(), e.item.as_str())),
            Some(e) if e.item == "missing" && e.revision == Some(-1) => {
                if cur != path {
                    match e.schedule.as_deref() {
                        Some("add") => root = Some((cur.to_string(), "added")),
                        Some("replace") => root = Some((cur.to_string(), "replaced")),
                        _ => {}
                    }
                }
            }
            _ => return root,
        }
        cur = &cur[..cur.rfind('/')?];
    }
}

fn under(path: &str, root: &str) -> String {
    if root == path {
        format!("`{path}`")
    } else {
        format!("`{path}`（它在 `{root}` 底下）")
    }
}

/// 复制根**顶替了服务器上已有的路径**（`svn rm X; svn cp Y X`）。
///
/// 服务器 rN 上这个位置有旧内容，但 `svn cat -r BASE` 给的是复制源 `Y` 的内容（实测），
/// 离线拿不到旧内容；子树里哪些路径服务器上也有，同样判断不了。按「服务器上没有」
/// 摘掉会让基线少文件，按 BASE 取会录进复制源的内容——两条都是假基线，所以停下。
fn copied_over_server_content(path: &str, root: &str) -> GitError {
    GitError::new(
        "svn_dirty_replaced_with_history",
        format!(
            "{}是复制过来、顶替了服务器上原有内容的（svn status 里 `{root}` 是带复制历史的 \
             replaced，典型操作是先 `svn rm` 再 `svn cp` / `svn mv` 到同一个位置）。\
             服务器上这个位置的旧内容在本地取不到，svn 只给得出复制源的内容，\
             录进基线就会把复制源当成服务器内容。\n\
             请先在 TortoiseSVN 里把这次替换提交，或者对 `{root}` 做「还原」，然后再试。\
             本应用什么都没改。",
            under(path, root)
        ),
    )
}

/// 链上最上层的复制节点既不是 added 也不是 replaced：真正的复制起点不在本次检查的
/// 范围里（典型是白名单根本身就在一个复制出来的目录里，独立 review 实测），判断不了
/// 服务器上有没有这些路径。
fn copy_origin_unknown(path: &str, root: &str, root_item: &str) -> GitError {
    GitError::new(
        "svn_dirty_copy_origin_unknown",
        format!(
            "{}是复制过来的，但往上找到的 `{root}`（svn status 里是 `{root_item}`）也是复制来的，\
             真正的复制起点在白名单之外，看不到。服务器上这些路径有没有、是什么，在本地判断不了。\n\
             请先在 TortoiseSVN 里把这次复制提交，然后再试。本应用什么都没改。",
            under(path, root)
        ),
    )
}

/// `svn rm X` 后在同一位置新加（或复制）了文件，之后又从磁盘删掉：服务器 rN 上 `X`
/// 有旧内容，本地却只剩一个「待替换、文件丢失」的记录，拿不到旧内容。
/// 不带历史的 replaced（`svn rm X` 后在同一位置 `svn add`；目录同理）。事故分析 6.2 选定的做法：
/// 离线取不到服务器上的旧内容（`svn cat -r BASE` 报 E200009，2026-11-05 实测），也不为它破例联网，
/// 明确停下并说清怎么办。
///
/// 目录也停（PR 116 遗留）：`svn rm D; mkdir D; svn add D` 之后，D 底下重新加的同名文件 schedule 是
/// `add`，单看它会被摘出基线，可服务器上旧的 D 里有它。停在 D 上，子节点怎么分类就不重要了。
fn replaced_without_history(path: &str) -> GitError {
    GitError::new(
        "svn_dirty_replaced_without_history",
        format!(
            "`{path}` 在 svn 里被替换过：先删掉，又在同一位置重新添加（TortoiseSVN 里显示为 R）。\
             服务器上这个位置有旧内容，但本地取不到它（svn 只记着「替换」这件事，旧内容要联网才拿得到），\
             录进基线就会少录或者录错。\n\
             请先在 TortoiseSVN 里把这次替换提交，或者对 `{path}` 做「还原」，然后再试。\
             工作副本不变的话，重试还会停在这里。本应用什么都没改。"
        ),
    )
}

fn replaced_then_missing(path: &str) -> GitError {
    GitError::new(
        "svn_dirty_replaced_missing",
        format!(
            "`{path}` 在 svn 里先删掉、又在同一位置新加了文件（替换），之后又从磁盘上删掉了。\
             服务器上这个位置有旧内容，但本地取不到它。\n\
             对 `{path}` 做「还原」可以恢复服务器上的那一版；如果新加的文件是有意删掉的，\
             还原之后再在 TortoiseSVN 里删除它即可。本应用什么都没改。"
        ),
    )
}

/// `missing` + `revision="-1"`，但 `svn info` 没说清它是新加的还是替换的。
fn missing_uncommitted_unknown(path: &str, schedule: Option<&str>) -> GitError {
    let said = match schedule {
        Some(s) => format!("svn info 说它的 schedule 是 `{s}`"),
        None => "没查到它的 schedule".to_string(),
    };
    GitError::new(
        "svn_dirty_missing_unresolved",
        format!(
            "`{path}` 从磁盘上丢了，svn 记录它还没提交过，但{said}，判断不了服务器上有没有它。\
             摘掉可能让基线少一个服务器上的文件，取回又取不到，所以停下。\n\
             请在 TortoiseSVN 里对它做「还原」，或者确认是新加的文件后撤销添加，然后再试。\
             本应用什么都没改。"
        ),
    )
}
