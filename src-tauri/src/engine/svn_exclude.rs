//! 由配置生成 `.git/info/exclude`。
//!
//! **由代码生成、绝不让人手写**：这套「先全排除、再逐层反选」的写法极易写错，
//! 而写错的表现是**用户以为文件受 git 保护、实际上没有**——切分支丢改动、
//! stash 存不进去、交付清单里不出现（spec §6.3）。

use crate::error::{GitError, Result};
use std::io::Write;

/// 生成文件的第一行。也是「这个 exclude 是不是我们写的」的唯一判据
/// ——[`write_exclude`] 靠它决定要不要先备份用户原有的内容。
///
/// 只在建库时写一次（[`write_exclude`] 唯一的调用点在 `git init` 之后），改
/// `mygitclient.json`、重建基线都不会让它重新生成。打开仓库时重新生成也不对——
/// 纳入集合与基线绑定，换一套规则而基线不变，新纳入的 SVN 文件会被 git 当成
/// 未跟踪的新文件。
///
/// 文件头只陈述这件事，不给出路：应用里目前没有改白名单 / 扩展名的入口，初始化向导
/// 也不接受已经是混合仓库的目录（第二轮独立 review）。写「需要重新初始化」会把人
/// 引向手工删 `.git`，丢掉提交和 stash。
///
/// 旧版本的文字（「改动会在下次打开仓库时被覆盖」）与实现不符，已改掉。不需要
/// 兼容旧文字：`write_exclude` 只在刚 `git init` 的仓库上跑，那里只有 git 的模板。
pub const GENERATED_HEADER: &str =
    "# 由 mygitclient 建库时生成，请勿手工编辑。之后修改 mygitclient.json 不会让这里的规则重新生成。";

/// 生成规则文本。
///
/// 输出形如（白名单 `Assets/Scripts/`，扩展名 `.cs` / `.meta`）：
///
/// ```gitignore
/// /*
/// !/Assets/
/// /Assets/*
/// !/Assets/Scripts/
/// /Assets/Scripts/**
/// !/Assets/Scripts/**/
/// !/Assets/Scripts/**/*.cs
/// !/Assets/Scripts/**/*.meta
/// .svn/
/// ```
///
/// # 为什么分三段发，而不是一个条目接一个条目地发
///
/// git 的裁决规则是「**最后一条**匹配的模式说了算」，所以规则的**先后顺序**
/// 本身就是语义。按条目顺序流水线式输出时，后一个条目为打开自己的父目录发出的
/// `/Code/*` 会落在前一个条目的 `!/Code/**/*.cs` 之后，把前者已经纳入的文件又
/// 剔出去。2026-09-13 实测：
///
/// | 白名单 | 流水线式输出的实际结果 |
/// |---|---|
/// | `["Code/", "Code/Sub/"]` | `Code/Top.cs` 消失 |
/// | `["Code/", "Code/Junk.psd"]` | 只剩 `Code/Junk.psd`，整个目录条目全没了 |
///
/// 而且**没有任何报错**——用户以为整个 `Code/` 都受 git 保护，实际没有。
/// 所以这里固定分三段：
///
/// 1. **打开父目录**：所有条目的非末段前缀去重、按深度升序，每层 `!/P/` + `/P/*`。
///    去重保证同一层只发一次，深度升序保证外层先于内层。
/// 2. **目录条目的叶子规则**：`/P/**` 紧跟着按扩展名反选，成对出现，互不干扰。
/// 3. **单文件条目**：必须排在最后。它不过扩展名过滤，若排在目录条目之前，
///    会被目录条目的 `/P/**` 重新盖掉。
pub fn generate_exclude(whitelist: &[String], extensions: &[String]) -> String {
    // 先拆成 (段列表, 是否目录)，顺手丢掉空条目
    let mut entries: Vec<(Vec<String>, bool)> = Vec::new();
    for raw in whitelist {
        let entry = raw.replace('\\', "/");
        let is_dir = entry.ends_with('/');
        let clean = entry.trim_end_matches('/');
        if clean.is_empty() {
            continue;
        }
        entries.push((clean.split('/').map(str::to_string).collect(), is_dir));
    }

    let mut lines: Vec<String> = vec![GENERATED_HEADER.to_string(), "/*".to_string()];

    // ① 逐层打开父目录。每一层都要「先反选目录本身、再排除其内容」，
    // 否则下一层的反选规则够不着——git 的硬规则是**父目录被排除后，
    // 其中的文件无法再被重新纳入**（spec §6.3）。
    let mut opens: Vec<String> = Vec::new();
    for (segs, _) in &entries {
        let mut prefix = String::new();
        for seg in &segs[..segs.len() - 1] {
            prefix.push('/');
            prefix.push_str(seg);
            if !opens.contains(&prefix) {
                opens.push(prefix.clone());
            }
        }
    }
    // 按深度升序：`/A` 必须排在 `/A/B` 之前，否则内层反选够不着。
    // sort_by_key 是稳定排序，同深度的条目保持白名单里的书写顺序。
    opens.sort_by_key(|p| p.matches('/').count());
    for p in &opens {
        lines.push(format!("!{p}/"));
        lines.push(format!("{p}/*"));
    }

    // ② 目录条目
    for (segs, is_dir) in &entries {
        if !is_dir {
            continue;
        }
        let prefix = format!("/{}", segs.join("/"));
        lines.push(format!("!{prefix}/"));
        lines.push(format!("{prefix}/**"));
        // ⚠️ 这一行不能省：`/x/**` 会把中间**目录**也排除掉，git 就不再
        // 往里递归，后面按扩展名的反选对深层文件永远不会生效。
        // 2026-09-12 实测：缺它时 Sub/Mid.cs 与 Sub/Deep/Low.cs 静默丢失。
        lines.push(format!("!{prefix}/**/"));
        for ext in extensions {
            if ext == "*" {
                lines.push(format!("!{prefix}/**/*"));
            } else {
                lines.push(format!("!{prefix}/**/*{ext}"));
            }
        }
    }

    // ③ 单文件条目：直接反选它本身，不展开通配、不过扩展名过滤
    for (segs, is_dir) in &entries {
        if *is_dir {
            continue;
        }
        lines.push(format!("!/{}", segs.join("/")));
    }

    // ⚠️ **必须是最后一行。** `git check-ignore` 的裁决规则是「最后一条匹配的
    // 模式说了算」，而上面那些 `!/x/**/` 把白名单内部的**所有**子目录重新纳入了，
    // `.svn` 也在其中。白名单落在 external 根时那里有真实的 `.svn`；一旦 `wc.db`
    // 被 git 跟踪，任何一次 `git checkout` 都会在一个**正在使用的** SVN 工作副本
    // 底下把它换成另一个版本——那不是「恢复旧数据」，是直接损坏工作副本。
    // 不带前导 `/` 使它在任意深度生效（spec §6.3）。
    lines.push(".svn/".to_string());

    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// 把规则写进 `<仓库根>/.git/info/exclude`。
///
/// # 为什么不是直接 `File::create` + `write_all`
///
/// `File::create` **先截断再写**。写到一半失败（磁盘满、进程被杀、断电）留下的
/// 是一个不完整的 exclude——最坏的形态是**缺了末尾那行 `.svn/`**：之后一次
/// `git add -A` 会把 SVN 的 `wc.db` 收进 git，再一次 `git checkout` 就会在一个
/// **正在使用的**工作副本底下把它换成另一个版本。写临时文件再 `rename` 把这一步
/// 变成原子的：要么还是旧内容，要么已经是完整的新内容，没有中间态。
///
/// # 用户原有的 exclude 不会被静默吃掉
///
/// 首次写入时如果那里已经有一个**不是本工具生成的、且含真实规则的**文件，
/// 先把它备份成 `exclude.bak`。`.git/info/exclude` 是 git 给用户自己用的地方，
/// 无条件整文件覆盖等于删掉用户手写的规则且不留痕迹。
///
/// 「含真实规则」= 至少有一行既不是空行也不是 `#` 注释。`git init` 会在那里写
/// 一份**纯注释**的模板（`# git ls-files --others --exclude-from=...`），按「非空」
/// 判定的话每个新仓库都会多出一个什么也没保存的 `exclude.bak`。
///
/// 备份已存在则不再覆盖——那说明我们早就接管过一次，`exclude.bak` 里才是用户
/// 真正的原件。
pub fn write_exclude(repo_root: &str, content: &str) -> Result<()> {
    let info = std::path::Path::new(repo_root).join(".git").join("info");
    std::fs::create_dir_all(&info)?;
    let target = info.join("exclude");

    if let Ok(existing) = std::fs::read_to_string(&target) {
        let ours = existing.lines().next() == Some(GENERATED_HEADER);
        let has_rules = existing
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        let backup = info.join("exclude.bak");
        if !ours && has_rules && !backup.exists() {
            std::fs::write(&backup, &existing)?;
        }
    }

    // 临时文件要和目标**同目录**：跨卷时 rename 会失败（Windows 上
    // ERROR_NOT_SAME_DEVICE），而 `%TEMP%` 完全可能在另一个盘上。
    let tmp = info.join("exclude.mygitclient.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        // 显式 sync：rename 是原子的，但「新内容已经落盘」不是它保证的。
        // 少了这一步，断电后可能 rename 生效了而内容还是空的。
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        // rename 失败时把临时文件清掉，别在 .git/info 里留垃圾
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换 .git/info/exclude 失败: {e}"))
    })?;
    Ok(())
}
