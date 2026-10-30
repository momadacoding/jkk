//! 由代码生成 `.git/info/attributes`，关掉 git 的行尾转换（spec §7.3.0）。
//!
//! # 为什么混合仓库必须按字节读写
//!
//! 工作目录同时是 SVN 工作副本。`core.autocrlf=true`（多数 Windows 开发机的默认
//! 状态）下，git 每次往磁盘写文件都会把 LF 转成 CRLF，而没设 `svn:eol-style` 的
//! 文件在 SVN 里存的可能是 LF。于是 `git stash` 往返一次，那个文件对 svn 就变脏了
//! ——内容一个字符没改，整文件的行尾全翻了一遍。最坏的下游后果是交付时把一整批
//! 纯行尾变化提交到 SVN。
//!
//! # 为什么是这里，不是 `core.autocrlf=false`，也不是工作树的 `.gitattributes`
//!
//! - 工作树的 `.gitattributes` 会进工作目录，也就会被提交进**用户的 SVN 仓库**。
//!   spec §3 明写「工作目录里只多出 `.git` 一个条目」。
//! - `core.autocrlf=false` 挡不住工作树里的 `.gitattributes`。2026-09-21 实测：
//!   工作树放一个 `* text=auto`，`git check-attr text` 就从 `unset` 变回 `auto`，
//!   归一化又回来了。而 `$GIT_DIR/info/attributes` 的优先级压过工作树，掀不翻。
//!
//! 剩下 `.git/info/attributes` 这一条路，正好是 `svn_exclude.rs` 的邻居，
//! 写法也照抄它。

use crate::error::{GitError, Result};
use std::io::Write;

/// 生成文件的第一行。也是「这个 attributes 是不是我们写的」的唯一判据。
///
/// 「下次打开仓库时被覆盖」与实现不符（它只在建库和行尾迁移时写），`svn_exclude`
/// 的同名文件头已经改掉，这里**有意没改**：[`has_eol_rule`] 按第一行精确比对来判定
/// 规则装没装过，改文字会让所有现存仓库都被判成需要迁移。要改得先让判据兼容旧文字。
pub const GENERATED_HEADER: &str =
    "# 由 mygitclient 自动生成，请勿手工编辑——改动会在下次打开仓库时被覆盖。";

/// 关掉行尾转换的规则本体。
///
/// `-text` 的含义是「这个路径不参与行尾归一化」，**不是**「当成二进制」：
/// `git diff` 照常按文本显示。
pub const EOL_RULE: &str = "* -text";

/// 规则文本。只有一条规则，但仍然走生成函数——判据、注释和文件内容必须长在
/// 一处，否则改了一边忘了另一边时，[`has_eol_rule`] 会对着一个新格式的文件报
/// `false`，表现为「明明装过却一直提示要迁移」。
pub fn generate_attributes() -> String {
    format!("{GENERATED_HEADER}\n{EOL_RULE}\n")
}

fn info_dir(repo_root: &str) -> std::path::PathBuf {
    std::path::Path::new(repo_root).join(".git").join("info")
}

/// 规则是否已经装好。
///
/// # 为什么读文件，而不是问 `git check-attr`
///
/// 这个函数回答的是「**我们的**规则装了没有」，用来判定旧仓库要不要迁移。
/// `git check-attr` 回答的是「这条路径最终的属性是什么」——用户自己在别处
/// 配出同样的效果也会返回 `unset`，那时不该判成「已迁移」，因为基线仍然是旧
/// 设置下录的。两个问题不一样，别拿后者答前者。
pub fn has_eol_rule(repo_root: &str) -> bool {
    match std::fs::read_to_string(info_dir(repo_root).join("attributes")) {
        Ok(s) => {
            s.lines().next() == Some(GENERATED_HEADER) && s.lines().any(|l| l.trim() == EOL_RULE)
        }
        Err(_) => false,
    }
}

/// 把规则写进 `<仓库根>/.git/info/attributes`，返回**写之前**的原始字节。
///
/// 返回值是给调用方回滚用的（`migrate_eol` 要在重建基线失败时还原原状）。
/// `None` 表示这个文件原本不存在。
///
/// # 为什么不是直接 `File::create` + `write_all`
///
/// `File::create` 先截断再写。写到一半失败（磁盘满、进程被杀、断电）留下的是
/// 半个规则文件——而半个规则比没有规则更难发现：`has_eol_rule` 会报 `false`，
/// 于是界面提示迁移，迁移又会因为「规则已存在」的判断路径走岔。写临时文件再
/// `rename` 把这一步变成原子的：要么还是旧内容，要么已经是完整的新内容。
///
/// # 用户原有的 attributes 不会被静默吃掉
///
/// 首次接管时，如果那里已经有一个**不是本工具生成的、且含真实规则的**文件，
/// 先备份成 `attributes.bak`。「含真实规则」= 至少有一行既不是空行也不是 `#`
/// 注释。备份已存在则不再覆盖——那说明我们早就接管过一次，`attributes.bak`
/// 里才是用户真正的原件。
pub fn write_attributes(repo_root: &str) -> Result<Option<Vec<u8>>> {
    let info = info_dir(repo_root);
    std::fs::create_dir_all(&info)?;
    let target = info.join("attributes");

    let previous = std::fs::read(&target).ok();

    if let Some(bytes) = previous.as_deref() {
        // 用户的 attributes 可能不是 UTF-8（比如 GBK 注释）。`from_utf8_lossy`
        // 只用于**判断**要不要备份，备份写回去的是原始字节，不会被替换字符污染。
        let existing = String::from_utf8_lossy(bytes);
        let ours = existing.lines().next() == Some(GENERATED_HEADER);
        let has_rules = existing
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        let backup = info.join("attributes.bak");
        if !ours && has_rules && !backup.exists() {
            std::fs::write(&backup, bytes)?;
        }
    }

    // 临时文件必须与目标**同目录**：跨卷 rename 在 Windows 上报
    // ERROR_NOT_SAME_DEVICE，而 %TEMP% 完全可能在另一个盘上。
    //
    // 名字带 pid + 纳秒：固定名字在两条并发请求下会互相截断对方写到一半的内容
    // （Windows 上还可能直接撞上共享冲突）。与 `TempIndex` 的 `unique_index_path`
    // 同一个理由（2026-09-25 PR #45 review）。
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = info.join(format!(
        "attributes.mygitclient.{}.{nanos}.tmp",
        std::process::id()
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(generate_attributes().as_bytes())?;
        // rename 是原子的，但「新内容已经落盘」不是它保证的。
        // 少了这一步，断电后可能 rename 生效了而内容还是空的。
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        // rename 失败时把临时文件清掉，别在 .git/info 里留垃圾
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换 .git/info/attributes 失败: {e}"))
    })?;
    Ok(previous)
}

/// 把 `.git/info/attributes` 还原成 `previous` 描述的样子。
///
/// `None` 表示原本不存在 —— 此时**删掉**文件，而不是留一个空文件：留空文件会让
/// `.git/info` 里多出一个来路不明的条目，下一个人无从判断它是谁写的。
///
/// # 只有当文件仍是**我们写的那一份**时才回滚
///
/// 2026-09-25 PR #45 review 的并发场景（详见 `svn_init::MigrationLock`）：
/// 落败的那条请求按自己记下的 `previous = None` 把文件删掉，而那一份其实是
/// **胜者刚写好的规则**。胜者返回成功，规则却没了。
///
/// `MigrationLock` 已经让本应用不再制造这种交错，这里再加一道：回滚前确认
/// 文件内容仍是我们生成的那份，不是就原样留着。锁防的是本应用自己，这一道
/// 连外部改动也一并让开——回滚的语义本来就是「撤销我写的东西」，不是
/// 「把这个路径恢复成我记得的样子」。
pub fn restore_attributes(repo_root: &str, previous: Option<&[u8]>) -> Result<()> {
    let target = info_dir(repo_root).join("attributes");
    let current = std::fs::read(&target).ok();
    if current.as_deref() != Some(generate_attributes().as_bytes()) {
        // 已经不是我们写的那份了（别人改过，或根本没写成功）。不碰。
        return Ok(());
    }
    match previous {
        Some(bytes) => std::fs::write(&target, bytes)?,
        None => std::fs::remove_file(&target)?,
    }
    Ok(())
}
