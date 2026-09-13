//! 由配置生成 `.git/info/exclude`。
//!
//! **由代码生成、绝不让人手写**：这套「先全排除、再逐层反选」的写法极易写错，
//! 而写错的表现是**用户以为文件受 git 保护、实际上没有**——切分支丢改动、
//! stash 存不进去、交付清单里不出现（spec §6.3）。

use crate::error::Result;
use std::io::Write;

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

    let mut lines: Vec<String> = vec![
        "# 由 mygitclient 自动生成，请勿手工编辑——改动会在下次打开仓库时被覆盖。".to_string(),
        "/*".to_string(),
    ];

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

/// 把规则写进 `<仓库根>/.git/info/exclude`，整文件覆盖。
pub fn write_exclude(repo_root: &str, content: &str) -> Result<()> {
    let info = std::path::Path::new(repo_root).join(".git").join("info");
    std::fs::create_dir_all(&info)?;
    let mut f = std::fs::File::create(info.join("exclude"))?;
    f.write_all(content.as_bytes())?;
    Ok(())
}
