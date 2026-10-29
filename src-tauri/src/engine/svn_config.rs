//! `.git/mygitclient.json` 的读取与校验。
//!
//! 形状对齐 `engine::custom_config`：用户手写，客户端只读。
//!
//! **这个文件是不可信输入。** 每个字段都会被送进 git 或 svn 的命令行，不校验
//! 就等于把命令构造交给了手滑。校验**集中在加载时一次做完**，不要散落到各个
//! 调用点（spec §6.2.1）。

use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 基线分支名是**固定值**，不是可配项。
///
/// 全文所有流程都硬编码了它；字段保留只为向前兼容，值不是它就阻断（spec §6.2.1）。
pub const BASELINE_BRANCH: &str = "svn/base";

/// 本客户端认识的配置格式版本。
pub const SUPPORTED_VERSIONS: [u32; 2] = [1, RESPECT_GITIGNORE_VERSION];

/// 选了「尊重已有的 .gitignore」的仓库写这个版本。
///
/// 「尊重」改变了 W-file 的定义。只认版本 1 的旧客户端打开这种仓库时必须**明确拒绝**，
/// 不能静默按「不尊重」口径工作——那会把正常仓库报成一堆看不懂的漏收
/// （2026-10-28 spec §4）。没选「尊重」的仓库仍写 1，新旧客户端都能打开。
pub const RESPECT_GITIGNORE_VERSION: u32 = 2;

/// `mode` 的唯一合法值。§6.1 的仓库识别据它判定这是不是一个混合仓库。
pub const SVN_GIT_MODE: &str = "svn-git";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub svn_url: String,
    #[serde(default)]
    pub whitelist: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default = "default_baseline_branch")]
    pub baseline_branch: String,
    /// 建库时用户勾了「尊重已有的 .gitignore 规则」：被工作树里的 `.gitignore`
    /// 排除的文件不算 W-file（已在基线里的除外）。判定集中在 `engine::svn_scope`。
    ///
    /// 缺省 false，且为 false 时不写进文件——已经建好的仓库行为与文件内容都不变。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub respect_gitignore: bool,
    /// 建库时被改名停用的 `.gitignore`。只做记录，供以后排查；不进任何命令行。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_gitignores: Vec<DisabledGitignore>,
}

/// 一次改名停用：原路径 → 新路径，都是仓库相对路径、正斜杠。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisabledGitignore {
    pub from: String,
    pub to: String,
}

fn default_version() -> u32 {
    1
}

fn default_baseline_branch() -> String {
    BASELINE_BRANCH.to_string()
}

/// 配置文件的位置：`<仓库根>/.git/mygitclient.json`。
///
/// 放在 `.git/` 内部而不是工作目录，是为了让工作目录**只多出 `.git` 一个条目**
/// （spec §3）——多一个文件就多一个被 TortoiseSVN 误提交上去的东西。
pub fn config_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join("mygitclient.json")
}

/// 读取并校验配置。
///
/// 文件不存在时返回 `not_configured`，**与「配置坏了」（`svn_config`）分开**：
/// §6.1 的分流据此区分「有 `.svn` 但还没初始化」与「配置文件有问题」，两者给用户
/// 的引导完全不同。
pub fn read_config(repo_root: &str) -> Result<SvnConfig> {
    let path = config_path(repo_root);
    if !path.is_file() {
        return Err(GitError::new(
            "not_configured",
            format!("{} 不存在", path.display()),
        ));
    }
    let raw = std::fs::read_to_string(&path)?;
    let mut cfg: SvnConfig = serde_json::from_str(&raw)
        .map_err(|e| GitError::new("svn_config", format!("配置文件不是合法 JSON: {e}")))?;
    // **规范化必须在校验之前。** 反斜杠写法的 `C:\Windows` 只有转成正斜杠之后
    // 才看得出是盘符；顺序反过来，那一类写法会从盘符检查底下溜过去。
    // 而且后续所有比较都基于正斜杠形式，留两种写法会让「白名单里有没有这一条」
    // 在 Windows 上时对时错。
    let mut normalized = Vec::with_capacity(cfg.whitelist.len());
    for entry in &cfg.whitelist {
        normalized.push(normalize_whitelist_entry(entry)?);
    }
    cfg.whitelist = normalized;
    validate(&cfg)?;
    Ok(cfg)
}

/// 把配置写进 `.git/mygitclient.json`。
///
/// **写之前先校验。** 这个文件是 §6.1 仓库识别的判据之一：它一存在，
/// `detect_repo_kind` 就把目录判成 `SvnHybrid`。写进去一份过不了 `validate`
/// 的内容，等于造出一个「识别成混合仓库、但每次读配置都报错」的死局。
pub fn write_config(repo_root: &str, cfg: &SvnConfig) -> Result<()> {
    validate(cfg)?;
    validate_new_extensions(&cfg.extensions)?;
    let path = config_path(repo_root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(cfg)
        .map_err(|e| GitError::new("svn_config", format!("配置序列化失败: {e}")))?;
    std::fs::write(&path, format!("{json}\n"))?;
    Ok(())
}

/// 新写入的扩展名里允许出现的非字母数字字符。
///
/// **白名单，不是黑名单**（第三轮独立 review）：黑名单要逐个列出输入法能打出的每一种
/// 分隔符、句点、全角字母、零宽字符，列不全的那个就是下一次静默失效——上一版就漏了
/// 全角字母，`.ｃｓ` 照样写进了配置。真实工程的扩展名只用 ASCII 字母、数字和这几个符号。
const EXTENSION_PUNCT: &[char] = &['.', '_', '-', '+', '~'];

/// 报错里怎么指出一个字符：可见的 ASCII 直接引出来，其余给码点——零宽字符、
/// 全角空格引出来也看不见。
fn describe_char(c: char) -> String {
    if c.is_ascii_graphic() {
        format!("'{c}'")
    } else if c.is_whitespace() || c.is_control() {
        format!("U+{:04X}（空白或控制字符）", c as u32)
    } else {
        format!("'{c}'（U+{:04X}）", c as u32)
    }
}

/// 挡住**能写出来、但匹配不到任何文件**的扩展名。
///
/// 2026-10-28 现场：向导只按 ASCII 逗号切分，用户用顿号分隔，配置里落下
/// `".cs、.lua、.java"`。它以点开头、不含路径分隔符，[`validate`] 放行，生成的
/// `!/…/**/*.cs、.lua、.java` 什么都匹配不到——白名单整体失效却没有任何报错。
///
/// 规则：
/// - 只许 ASCII 字母、数字与 [`EXTENSION_PUNCT`]
/// - 点分出来的每一段都不能空：挡住 `.lua.` `..cs` `.cs..lua` `.` `..`（多半来自句末或
///   夹在中间打的 `。`）。Windows 上文件名不能以点结尾，`*.lua.` 永远匹配不到
///
/// `[` `]` 顺带被挡：exclude 规则里它是字符类，`*.c[s]` 会收进 `Foo.cs`，而纳入集合
/// 核对按字面量比后缀，两边给出不同答案，核对会报不一致。
///
/// # 为什么只在写新配置时查，不放进 [`validate`]
///
/// `validate` 在每次打开仓库、同步、读日志时都会跑（经 [`read_config`]）。旧版本
/// 已经写下这种扩展名的仓库往往是**半能用**的（`[".cs", ".lua、.java"]` 里 `.cs`
/// 照常受管，上面可能已经有提交和 stash），读配置时拒绝等于把它整个锁死，应用里
/// 没有任何出路。这类扩展名不会让命令作用到预期之外的范围：大多数只是匹配不到文件，
/// exclude 与纳入集合核对两边都为空，仍然一致；方括号那一类会让核对报不一致，但那是
/// 旧版本就有的形态，不是这里新造出来的。所以只在源头（向导组装配置、写配置文件）挡住。
pub fn validate_new_extensions(extensions: &[String]) -> Result<()> {
    const HINT: &str = "请用英文逗号分隔，每条以英文句点开头，例如 .cs, .lua";
    for e in extensions {
        // "*" 是 spec §6.3 明文给出的全纳入写法，单独放行（与 `validate` 一致）
        if e == "*" {
            continue;
        }
        if let Some(c) = e
            .chars()
            .find(|c| !c.is_ascii_alphanumeric() && !EXTENSION_PUNCT.contains(c))
        {
            return Err(GitError::new(
                "svn_config",
                format!(
                    "扩展名 {e:?} 含不允许的字符 {}。扩展名只能由英文字母、数字和 \
                     . _ - + ~ 组成——多半是把多个扩展名写成了一条，或者混进了全角字符，\
                     这样的规则匹配不到任何文件。{HINT}",
                    describe_char(c)
                ),
            ));
        }
        // 以点开头，所以第一段必然为空，从第二段起不许空
        if e.split('.').skip(1).any(str::is_empty) {
            return Err(GitError::new(
                "svn_config",
                format!(
                    "扩展名 {e:?} 有连续的句点或以句点结尾，这样的规则匹配不到任何文件。{HINT}"
                ),
            ));
        }
    }
    Ok(())
}

/// 校验一份**已规范化**的配置。
pub fn validate(cfg: &SvnConfig) -> Result<()> {
    // `version` 与 `mode` 必须校验，理由与 `baselineBranch` 被钉死成 `svn/base`
    // 一模一样：留一个不校验的字段，只会让「文件里写的」与「代码假设的」不一致。
    //
    // 这里与 `engine::custom_config` 的做法**有意不同**——那边的 `version` 明确
    // 写了「解析时不校验其值」，因为它最坏的后果是一个工具栏按钮不对；而这里
    // 一个版本语义差异会直接改变白名单的含义，进而改变哪些文件受保护。
    if !SUPPORTED_VERSIONS.contains(&cfg.version) {
        return Err(GitError::new(
            "svn_config",
            format!(
                "不支持的 version：{}（本客户端只认 {SUPPORTED_VERSIONS:?}）。\
                 请确认这个配置文件是不是给更新版本的客户端写的",
                cfg.version
            ),
        ));
    }
    // 反过来不要求：version 2 而不尊重，语义与 1 相同，放行无害。
    if cfg.respect_gitignore && cfg.version != RESPECT_GITIGNORE_VERSION {
        return Err(GitError::new(
            "svn_config",
            format!(
                "respectGitignore 为 true 时 version 必须是 {RESPECT_GITIGNORE_VERSION}，实际是 {}。\
                 否则只认版本 1 的旧客户端会按「不尊重」的口径打开这个仓库",
                cfg.version
            ),
        ));
    }
    if cfg.mode != SVN_GIT_MODE {
        return Err(GitError::new(
            "svn_config",
            format!(
                "mode 必须是 {SVN_GIT_MODE}，实际是 {:?}。\
                 §6.1 的仓库识别据它判定这是不是一个混合仓库",
                cfg.mode
            ),
        ));
    }
    if cfg.svn_url.trim().is_empty() {
        return Err(GitError::new("svn_config", "svnUrl 不能为空"));
    }
    if cfg.baseline_branch != BASELINE_BRANCH {
        return Err(GitError::new(
            "svn_config",
            format!(
                "baselineBranch 必须是 {BASELINE_BRANCH}，实际是 {}。\
                 全文流程都硬编码了这个名字，改它会让基线与流程不一致",
                cfg.baseline_branch
            ),
        ));
    }
    // spec §6.2.1 说的是「两者皆空则阻断」，但**任一为空同样不可用**：
    // 空白名单会让每一步的空参数保护触发，空扩展名会让 W-file 恒为空集（§6.3.2），
    // 两种都表现为「功能装上了但什么都管不到」。所以这里分别挡。
    if cfg.whitelist.is_empty() {
        return Err(GitError::new("svn_config", "whitelist 不能为空"));
    }
    if cfg.extensions.is_empty() {
        return Err(GitError::new("svn_config", "extensions 不能为空"));
    }
    for e in &cfg.extensions {
        // "*" 是 spec §6.3 明文给出的全纳入写法，单独放行。
        if e == "*" {
            continue;
        }
        if !e.starts_with('.') {
            return Err(GitError::new(
                "svn_config",
                format!("扩展名必须以 . 开头：{e}"),
            ));
        }
        if e.contains('/') || e.contains('\\') || e.contains('*') || e.contains('?') {
            return Err(GitError::new(
                "svn_config",
                format!("扩展名不能含路径分隔符或通配符：{e}"),
            ));
        }
    }
    // 规范化过的条目再过一遍，挡住「直接构造 SvnConfig 绕开 read_config」的调用方。
    for w in &cfg.whitelist {
        normalize_whitelist_entry(w)?;
    }
    Ok(())
}

/// 把一条白名单路径规范化成「仓库内相对路径 + 正斜杠」，并挡掉危险形态。
///
/// 白名单条目同时被当成 **git pathspec** 和 **SVN target**，两套语义各有魔法字符。
/// 一个 `:(exclude)` 或一个 `..` 就能让命令作用到预期之外的范围（spec §6.2.1）。
///
/// **尾部的 `/` 原样保留**：它是「目录（递归）」与「单个文件」的唯一区分
/// （spec §6.3），吃掉它会让 exclude 生成走错分支。
pub fn normalize_whitelist_entry(raw: &str) -> Result<String> {
    let reject = |why: &str| {
        Err(GitError::new(
            "svn_config",
            format!("白名单条目 {raw:?}：{why}"),
        ))
    };

    let s = raw.replace('\\', "/");
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return reject("不能为空");
    }
    // pathspec magic 必须在最前面挡掉：`:(exclude)Assets` 会让 git 反选。
    if trimmed.starts_with(':') {
        return reject("不能以 : 开头（git pathspec magic）");
    }
    // 以 / 开头同时覆盖了绝对路径和 UNC（`//server/share`）。
    if trimmed.starts_with('/') {
        return reject("必须是仓库内相对路径，不能以 / 开头");
    }
    // 盘符：`C:/x`。上面已挡了以 : 开头，这里挡第二个字符是 : 的。
    if trimmed.chars().nth(1) == Some(':') {
        return reject("不能含盘符");
    }
    // ⚠️ **逐段判 `.` 与 `..`，不能只跟整串比。**
    // `trimmed == "."` 这种精确比较挡不住 `"./"`——它不以 `:` / `/` 开头、第二个
    // 字符不是 `:`、不等于 `.` 也不等于 `..`，会被原样放行。而 `./` 当 git
    // pathspec 和当 SVN target **两边都等于「全部」**，正是 §12.2 与
    // `AGENTS.md` §6.2 那次空 pathspec 事故同一类的东西。
    // 顺带也覆盖了 `Assets/../../etc` 这种中间段上跳。
    for seg in trimmed.split('/') {
        if seg == "." || seg == ".." {
            return reject("路径里不能含 . 或 .. 段（`./` 会退化成「整个仓库」）");
        }
    }
    // `-` 开头的条目当 git pathspec 时有 `--` 分隔符保护，但当 **SVN target** 时
    // 会被 svn 当成选项解析（`-rHEAD`、`--version`）。与上面的 pathspec magic
    // 同一类，一并挡掉。
    if trimmed.starts_with('-') {
        return reject("不能以 - 开头（会被 svn 当成选项解析）");
    }
    // `.git` / `.svn` 开头：前者会让客户端把自己的元数据交出去，后者会让 SVN 的
    // 工作副本数据库进 git。
    //
    // **大小写不敏感比较**：NTFS 不区分大小写，`.SVN/wc.db` 会在磁盘上命中真实的
    // `.svn` 目录，精确比较等于这条守卫在 Windows 上根本不生效
    // （`AGENTS.md` §8 把大小写不敏感列为本项目高发缺陷区）。
    //
    // 注意这**只挡条目本身**——深层的 `.svn`（白名单落在 external 根时里面就有
    // 一个）由 §6.3 生成的 exclude 末尾那条兜底规则负责，见 `engine::svn_exclude`。
    let first = trimmed.split('/').next().unwrap_or("");
    if first.eq_ignore_ascii_case(".git") || first.eq_ignore_ascii_case(".svn") {
        return reject("不能以 .git 或 .svn 开头");
    }
    // glob 元字符：与上面的 pathspec magic 同源，但后果更重。
    //
    // 2026-09-13 独立 review 实测：`whitelist:["*"] extensions:[".cs"]` 生成的规则
    // 会把 `Art/boss.fbx`、`Code/Junk.psd` 一起收进 git ——**扩展名过滤被整个绕开，
    // 白名单退化成「全部」**。`Code/*`、`Code/**` 同样。`extensions` 那侧早就挡了
    // `*` / `?`，白名单这侧漏了。
    if let Some(c) = trimmed.chars().find(|c| matches!(c, '*' | '?' | '[' | ']')) {
        return reject(&format!(
            "不能含通配符 {c:?}（白名单条目是路径，不是 pattern；\
             `*` 会让扩展名过滤整个失效、白名单退化成「整个仓库」）"
        ));
    }
    Ok(trimmed.to_string())
}

/// 拿白名单条目和磁盘对一遍：类型（目录 / 文件）与大小写都必须对得上。
///
/// # 为什么这是一道独立的校验，而不是 [`validate`] 的一部分
///
/// [`validate`] 是纯函数，只看字符串本身；这里要读盘，所以收 `repo_root`。
///
/// # 它挡的两件事，都是「配置看着没问题、行为完全不对」
///
/// **① 目录条目漏写尾斜杠。** 尾斜杠是「目录（递归）」与「单个文件」的**唯一**
/// 区分（spec §6.3），而单文件条目**不过扩展名过滤**。于是 `"Assets/Scripts"`
/// （少一个 `/`）会被当成单个文件反选，`!/Assets/Scripts` 把整棵树重新纳入且不受
/// 任何后续规则约束——2026-09-13 实测 `whitelist:["Code"] extensions:[".cs"]` 连
/// `Code/Junk.psd` 都跟踪了。漏一个斜杠是很自然的手误。
///
/// **② 大小写写错。** Windows 上 git 的忽略规则匹配大小写不敏感
/// （`core.ignoreCase=true`），但 **pathspec 匹配是大小写敏感的**。于是 `"code/"`
/// 这种写法下，git 照样按规则跟踪 `Code/Top.cs`，而 `git ls-files -- code/` 返回空
/// ——`engine::svn_include_check` 的核对会在一个工作完全正常的仓库上报
/// 「应当被跟踪、实际没有」。
///
/// # 路径不存在时**不报错**
///
/// 本函数只管「存在但对不上」。「配置指向了不存在的路径」是另一件事：条目可能是
/// 用户刚从磁盘上删掉的目录，在这里硬阻断会把人锁在仓库外面。初始化向导
/// （Phase 1b）该不该额外要求存在，由它自己决定。
///
/// ⚠️ **跳过的只是那一条**。这里原来写的是 `return Ok(())`，那是整函数返回：
/// 第一条不存在的条目会把它**后面所有条目**的校验一并放行。2026-09-14 独立
/// review 给的反例——`["Missing/", "Code"]` 通过，而 `Code` 单独送进来是被拒的
/// （漏尾斜杠 → 整棵树绕开扩展名过滤）。
pub fn validate_whitelist_against_disk(repo_root: &str, whitelist: &[String]) -> Result<()> {
    'entries: for entry in whitelist {
        let wants_dir = entry.ends_with('/');
        let clean = entry.trim_end_matches('/');
        if clean.is_empty() {
            continue;
        }
        let mut cursor = std::path::PathBuf::from(repo_root);
        for seg in clean.split('/') {
            let Some(real) = real_name(&cursor, seg) else {
                // 这一段在磁盘上不存在 → **这一条**不判，继续看下一条
                continue 'entries;
            };
            if real != seg {
                return Err(GitError::new(
                    "svn_config",
                    format!(
                        "白名单条目 {entry:?} 的大小写与磁盘不一致：写的是 {seg:?}，\
                         磁盘上是 {real:?}。git 的忽略规则在 Windows 上大小写不敏感、\
                         但 pathspec 匹配是敏感的，两边会给出不同答案，\
                         纳入集合核对会在一个正常仓库上误报。请改成磁盘上的写法。"
                    ),
                ));
            }
            cursor.push(seg);
        }
        let is_dir = cursor.is_dir();
        if wants_dir && !is_dir {
            return Err(GitError::new(
                "svn_config",
                format!("白名单条目 {entry:?} 以 / 结尾（表示目录），但磁盘上它不是目录。"),
            ));
        }
        if !wants_dir && is_dir {
            return Err(GitError::new(
                "svn_config",
                format!(
                    "白名单条目 {entry:?} 在磁盘上是一个**目录**，但它没有以 / 结尾。\
                     不带尾斜杠的条目会被当成单个文件处理、**不过扩展名过滤**，\
                     结果是整棵子树都进 git。请写成 {entry:?}/ 。"
                ),
            ));
        }
    }
    Ok(())
}

/// 在 `parent` 下找出与 `name` 大小写无关地相等的那一项，返回它在磁盘上的真实写法。
///
/// 不能直接用 `Path::exists()` 判大小写——NTFS 不区分大小写，`code` 对
/// `Code` 返回 true。只能列目录逐项比。
fn real_name(parent: &std::path::Path, name: &str) -> Option<String> {
    let entries = std::fs::read_dir(parent).ok()?;
    for ent in entries.flatten() {
        let real = ent.file_name().to_string_lossy().to_string();
        if real.eq_ignore_ascii_case(name) {
            return Some(real);
        }
    }
    None
}
