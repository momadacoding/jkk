//! `sync_in` 的未提交改动：S1 压栈、S10 取回（spec §7.3.1、§7.6.1）。
//!
//! # 身份靠 opid，不靠栈位置
//!
//! `stash@{0}` 是栈位置，任何一次新的 stash 都会把它挤走。所以：push 时把 opid
//! 写进 message，事后按 opid 在 reflog 里找；找到之后立刻钉一个
//! `refs/mygitclient/sync/<opid>`，此后一律按不可变 OID 操作。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_include_check::normalize_roots;
use crate::engine::svn_journal::{self, Journal, Phase};
use crate::engine::svn_sync_check::unmerged_paths;
use crate::error::{GitError, Result};
use std::path::Path;

pub const MSG_PREFIX: &str = "mygitclient-sync:";

pub fn pin_ref(opid: &str) -> String {
    format!("refs/mygitclient/sync/{opid}")
}

/// `refs/stash` 当前指向谁；没有 stash 时 `None`。
pub fn read_stash_ref(repo: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(repo, &["rev-parse", "-q", "--verify", "refs/stash"])?;
    match code {
        0 => Ok(Some(out.trim().to_string())),
        // 1 = 不存在。128 是 ref 损坏，不能当成「没有」——同 `svn_baseline::read_baseline`
        1 => Ok(None),
        _ => Err(GitError::new(
            "sync_stash",
            format!("读 refs/stash 失败（rev-parse 退出 {code}）"),
        )),
    }
}

/// 按 opid 在 stash 的 reflog 里找我们那一条（§7.3.1 的确定性判定）。
pub fn find_by_opid(repo: &str, opid: &str) -> Result<Option<String>> {
    if read_stash_ref(repo)?.is_none() {
        return Ok(None); // 没有 refs/stash 时 `log -g` 会以 128 失败
    }
    let out = run_git(repo, &["log", "-g", "--format=%H %gs", "refs/stash"])?;
    // %gs 形如 `On work: mygitclient-sync:<opid>`（2026-09-25 实测）
    let tag = format!(": {MSG_PREFIX}{opid}");
    Ok(out
        .lines()
        .find(|l| l.trim_end().ends_with(&tag))
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string))
}

/// S1 ①–⑤。结束时 `j.phase == Stashed`。
pub fn stash_wip(repo: &str, j: &mut Journal, whitelist: &[String]) -> Result<()> {
    let roots = normalize_roots(whitelist);
    if roots.is_empty() {
        // spec §12.2：`git stash push --` 后面什么都不跟，压的就是整个仓库
        return Err(GitError::new(
            "sync_stash_empty_whitelist",
            "白名单是空的。空 pathspec 会让 stash 压走整个仓库，一律拒绝。",
        ));
    }
    // ① 写在 push 之前（§7.3.1）
    j.stash_ref_before = read_stash_ref(repo)?;
    j.phase = Phase::Stashing;
    svn_journal::write(repo, j)?;

    // ②
    let msg = format!("{MSG_PREFIX}{}", j.opid);
    let mut args: Vec<&str> = vec!["stash", "push", "-u", "-m", &msg, "--"];
    args.extend(roots.iter().map(String::as_str));
    if let Err(e) = run_git(repo, &args) {
        // push 失败时先看它到底落没落地：没落地就什么都没发生，journal 可以删
        if find_by_opid(repo, &j.opid)?.is_none() && read_stash_ref(repo)? == j.stash_ref_before {
            svn_journal::remove(repo)?;
        }
        return Err(e);
    }
    // ③–⑤
    settle_stashing(repo, j)
}

/// §7.3.1 的判定树：phase=stashing 之后 push 落没落地、落在哪。
///
/// 正常路径与崩溃恢复共用——恢复时读到 `stashing` 也走这里。
pub fn settle_stashing(repo: &str, j: &mut Journal) -> Result<()> {
    match find_by_opid(repo, &j.opid)? {
        Some(oid) => {
            // ④ 钉住：之后即使 stash 被外部 drop，对象也不会被 gc
            run_git(repo, &["update-ref", &pin_ref(&j.opid), &oid])?;
            j.stash_oid = Some(oid);
        }
        None => {
            if read_stash_ref(repo)? != j.stash_ref_before {
                return Err(GitError::new(
                    "sync_stash_lost",
                    "stash 栈变了，却找不到本次同步压的那一条——可能被外部工具 \
                     `git stash clear` / `drop` 过。为了不把别人的 WIP 当成你的，这里停下。\
                     请用 `git fsck --unreachable | findstr commit` 找回，或联系开发者。",
                ));
            }
            // push 没落地：白名单内本来就没有可压的改动（计划 A7）
            j.stash_oid = None;
        }
    }
    j.phase = Phase::Stashed; // ⑤
    svn_journal::write(repo, j)
}

#[derive(Debug, Clone, PartialEq)]
pub enum RestoreOutcome {
    Applied,
    /// apply 了，留下冲突标记；stash 保留（spec §13 S10 那一行）。
    Conflicted {
        files: Vec<String>,
    },
    /// 一个字节都没 apply：stash 里的未跟踪文件在磁盘上已经有同名的了（计划 A3）。
    Blocked {
        collisions: Vec<String>,
    },
}

/// S10：按 OID 取回。
pub fn restore_wip(repo: &str, oid: &str) -> Result<RestoreOutcome> {
    let parents = stash_parents(repo, oid)?;
    // 带 `-u` 的 stash 有第三个父提交，里面是未跟踪文件。
    //
    // ⚠️ 必须在 apply **之前**查撞名：2026-09-25 实测，撞名时 git 先把已跟踪
    // 部分写上去、再在未跟踪上失败（退出 1、没有 `UU`）——半套 apply，而再
    // apply 一次又会撞上自己刚写下的改动。
    let mut same_file = Vec::new();
    if let Some(untracked) = parents.get(2) {
        let out = run_git(repo, &["ls-tree", "-r", "--name-only", "-z", untracked])?;
        let mut collisions = Vec::new();
        for p in out.split('\0').filter(|p| !p.is_empty()) {
            // 祖先路径被非目录占着时，对 `p` 取元数据会报错，看起来像「不存在」——
            // 但 git 建不出那一级目录，照样半套 apply（PR #49 review 第三轮 #3，实测）
            if let Some(block) = blocking_ancestor(repo, p) {
                if !collisions.contains(&block) {
                    collisions.push(block);
                }
                continue;
            }
            if Path::new(repo).join(p).symlink_metadata().is_err() {
                continue;
            }
            if is_same_file_everywhere(repo, untracked, p)? {
                same_file.push(p.to_string());
            } else {
                collisions.push(p.to_string());
            }
        }
        // 真撞名时**一个字节都不动**：字节相同的那批也还没删
        if !collisions.is_empty() {
            return Ok(RestoreOutcome::Blocked { collisions });
        }
        // 计划 A17：三处同一个 blob 的，先从磁盘拿掉，让 apply 把同样的字节写回来。
        // 删的是 HEAD 里原样存着的内容，不是用户独有的数据
        for p in &same_file {
            std::fs::remove_file(Path::new(repo).join(p))?;
        }
    }
    let result = match run_git(repo, &["stash", "apply", oid]) {
        Ok(_) => Ok(RestoreOutcome::Applied),
        Err(e) => {
            let files = unmerged_paths(repo)?;
            if files.is_empty() {
                Err(GitError::new(
                    "sync_stash_apply_failed",
                    format!(
                        "取回未提交改动失败了，而且不是冲突。stash 原样保留（{oid}）。git 原话：{}",
                        e.message
                    ),
                ))
            } else {
                Ok(RestoreOutcome::Conflicted { files })
            }
        }
    };
    // apply 没把它们写回来（中途失败）时从 HEAD 取回：HEAD 里就是同一个 blob。
    // `p` 来自 ls-tree，不会是空 pathspec（§12.2）
    for p in &same_file {
        if Path::new(repo).join(p).symlink_metadata().is_err() {
            run_git(repo, &["checkout", "HEAD", "--", p])?;
        }
    }
    result
}

/// 计划 A17：stash 里的这个未跟踪文件，与磁盘上撞名的文件、HEAD 里的同名文件是
/// **同一个 blob**——它不是两份内容在抢一个路径，而是同一个文件出现了三次。
///
/// 实际场景：外部在项目根 `svn update` 带来了服务器新增的文件，git 视它为未跟踪，
/// S1 的 `-u` 把它收进 stash；基线跟上之后它又作为已跟踪文件回到磁盘
/// （2026-09-25 实测，`probe-ext-added.ps1`）。按 A3 一律拦下的话，只要同事新增过
/// 文件，每次接管都会停在 `StashBlocked`。
///
/// 任何一处不同、HEAD 里没有它、或它不是普通文件，都按真撞名处理。
/// `p`（正斜杠、相对仓库根）的某一级祖先在磁盘上存在却不是目录时，返回那一级。
///
/// 符号链接也算挡路：git 不会穿过它写文件。
fn blocking_ancestor(repo: &str, p: &str) -> Option<String> {
    let parts: Vec<&str> = p.split('/').collect();
    for n in 1..parts.len() {
        let prefix = parts[..n].join("/");
        match Path::new(repo).join(&prefix).symlink_metadata() {
            Ok(m) if m.is_dir() => continue,
            Ok(_) => return Some(prefix),
            Err(_) => return None, // 这一级不存在，更深的也不会有东西挡着
        }
    }
    None
}

fn is_same_file_everywhere(repo: &str, untracked: &str, p: &str) -> Result<bool> {
    if !Path::new(repo).join(p).is_file() {
        return Ok(false);
    }
    let (code, head) = run_git_code(repo, &["rev-parse", "-q", "--verify", &format!("HEAD:{p}")])?;
    if code != 0 {
        return Ok(false);
    }
    let stash = run_git(
        repo,
        &["rev-parse", "--verify", &format!("{untracked}:{p}")],
    )?;
    // `--path` 让磁盘内容走与 `git add` 相同的过滤器（svn_baseline.rs 记过这个坑）
    let disk = run_git(repo, &["hash-object", &format!("--path={p}"), "--", p])?;
    Ok(head.trim() == stash.trim() && stash.trim() == disk.trim())
}

/// stash commit 的父提交列表。不是 stash 形状（少于两个父）就报错（§7.6.1 末）。
fn stash_parents(repo: &str, oid: &str) -> Result<Vec<String>> {
    let missing = || {
        GitError::new(
            "sync_stash_missing",
            format!(
                "记录里的 stash {oid} 找不到了，或者它不是一个 stash。\
                 不猜、不退回用 stash@{{0}}（§7.6.1）——请联系开发者。"
            ),
        )
    };
    let (code, out) = run_git_code(repo, &["rev-list", "--parents", "-n", "1", oid])?;
    if code != 0 {
        return Err(missing());
    }
    // 输出：`<oid> <p1> <p2> [<p3>]`
    let parents: Vec<String> = out.split_whitespace().skip(1).map(str::to_string).collect();
    if parents.len() < 2 {
        return Err(missing());
    }
    Ok(parents)
}

/// `oid` 在 stash 栈里的位置（`stash@{n}` 的 n）；不在栈里返回 `None`。
pub fn stash_position(repo: &str, oid: &str) -> Result<Option<usize>> {
    if read_stash_ref(repo)?.is_none() {
        return Ok(None);
    }
    let out = run_git(repo, &["log", "-g", "--format=%H", "refs/stash"])?;
    Ok(out.lines().position(|l| l.trim() == oid))
}

/// 按 OID drop 我们那一条，再删钉住的 ref。
pub fn drop_wip(repo: &str, opid: &str, oid: &str) -> Result<()> {
    if let Some(n) = stash_position(repo, oid)? {
        let dropped = run_git(repo, &["stash", "drop", &format!("stash@{{{n}}}")])?;
        // 输出形如 `Dropped stash@{1} (<oid>)`（2026-09-25 实测）。
        // 定位与 drop 之间若有外部工具又压了一条，位置会错开——核对一下，
        // 丢错了就把它塞回去，宁可报错也不吃掉别人的 WIP。
        if !dropped.contains(oid) {
            if let Some(other) = dropped
                .rsplit_once('(')
                .and_then(|(_, r)| r.split_once(')'))
                .map(|(o, _)| o.trim().to_string())
            {
                let _ = run_git(
                    repo,
                    &["stash", "store", "-m", "mygitclient 误删后放回", &other],
                );
            }
            return Err(GitError::new(
                "sync_stash_drop_raced",
                "删除 stash 的瞬间栈被外部改动了。已把误删的那条放回，本次同步的 stash \
                     仍在，请稍后在横幅上点「继续」重试。",
            ));
        }
    }
    unpin(repo, opid)
}

/// 删掉 `refs/mygitclient/sync/<opid>`；不存在也算成功。
pub fn unpin(repo: &str, opid: &str) -> Result<()> {
    let r = pin_ref(opid);
    let (code, _) = run_git_code(repo, &["rev-parse", "-q", "--verify", &r])?;
    if code == 0 {
        run_git(repo, &["update-ref", "-d", &r])?;
    }
    Ok(())
}
