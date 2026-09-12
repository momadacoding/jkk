//! 工作区文件操作。
//!
//! 这段逻辑原本直接写在 `commands/ops.rs` 的命令函数里。下沉到 engine 是为了
//! 让 `vcs::GitBackend` 能调用它——命令层往后只做分发，不放业务逻辑。

use crate::engine::git_cli::run_git;
use crate::error::{GitError, Result};
use std::collections::HashSet;

/// 跑一条会输出 NUL 分隔路径列表的 git 命令，收成集合。
///
/// 失败**原样抛出**。这里曾经把任何失败都当成空集，而空集会直接改变下面的
/// 分类结果——见 [`head_is_unborn`] 里记的那次数据丢失。
fn path_set(path: &str, args: &[&str]) -> Result<HashSet<String>> {
    Ok(run_git(path, args)?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// HEAD 是否还没有指向任何提交（`git init` 之后、第一次 commit 之前）。
///
/// 判定必须是**正向**的：HEAD 是个 symbolic ref，且它指向的那个分支 ref 不存在。
///
/// 不能反过来用「`git rev-parse --verify HEAD` 失败」来判。`ls-tree HEAD` 失败有
/// 两种完全不同的原因，而把后一种当成前一种会删用户的文件：
///
/// | 情形 | `ls-tree -r HEAD` | 该怎么办 |
/// |---|---|---|
/// | 仓库还没有任何提交 | 退出 128 `Not a valid object name HEAD` | 当成空集，继续 |
/// | 对象库损坏（tree 对象丢了） | 退出 128 `fatal: not a tree object` | **必须抛出** |
///
/// 2026-09-13 在临时 fixture 上实测第二种：删掉 HEAD 的 tree 对象之后，
/// `ls-tree` 失败而 `git rm -f` 照样退出 0 并把文件删掉。旧实现在 `staged` 分支下
/// 因此把「HEAD 里有、应当恢复」的文件分进了 rm 桶——用户点的是「丢弃改动」，
/// 拿到的是「文件没了」，而且整个调用还返回成功。
///
/// 两种情形下 `symbolic-ref` + `show-ref` 的结果不同（实测）：空仓库里分支 ref
/// 不存在，损坏的仓库里它好好地在。detached HEAD 下 `symbolic-ref` 退出 1，
/// 那种状态必然已经指着某个提交，返回 false 正确。
fn head_is_unborn(path: &str) -> bool {
    let Ok(head_ref) = run_git(path, &["symbolic-ref", "--quiet", "HEAD"]) else {
        return false;
    };
    let head_ref = head_ref.trim();
    !head_ref.is_empty() && run_git(path, &["show-ref", "--verify", "--quiet", head_ref]).is_err()
}

/// 组装一条把「已经做了什么、还没做什么」说清楚的错误。
///
/// discard 的三个阶段**都不可回滚**——`git checkout` 抹掉的工作区内容没有任何
/// 副本可以取回，`git rm -f` 和删文件同理。所以这里能补救的不是撤销，而是让用户
/// 知道仓库停在哪：只把底层那条 git stderr 抛上去的话，界面上看不出前面的恢复
/// 是否已经生效、剩下的文件还在不在。
///
/// `uncertain` 是失败那一步自己的路径列表。git 对多路径是可能部分成功的
/// （恢复了 a、卡在 b），客户端无从知道分界在哪，所以如实说「可能已部分处理」，
/// 不假装精确。逐文件执行的删除阶段没有这个问题，那里 `uncertain` 为空。
fn partial_failure(
    stage: &str,
    cause: &str,
    done: &[&str],
    uncertain: &[&str],
    skipped: &[&str],
) -> GitError {
    let list = |items: &[&str]| {
        if items.is_empty() {
            "（无）".to_string()
        } else {
            items.join("、")
        }
    };
    let mut message = format!(
        "丢弃在「{stage}」这一步失败，已停止，后面的步骤没有执行。\n原因：{cause}\n已完成：{}",
        list(done)
    );
    if !uncertain.is_empty() {
        message.push_str(&format!(
            "\n可能已部分处理（git 未说明卡在哪个文件）：{}",
            list(uncertain)
        ));
    }
    message.push_str(&format!("\n未处理：{}", list(skipped)));
    GitError::new("discard_partial", message)
}

/// 丢弃改动。
///
/// 三类文件走三条完全不同的路，混在一次多选里是常态：
///
/// - **未跟踪**：git 里没有任何副本，只能直接删文件。这一步不可逆，界面必须先确认。
/// - **HEAD 里没有、但已进暂存区**（新加的文件）：`git rm -f` 同时从索引和磁盘去掉。
/// - **其余**：`git checkout` 恢复。
///
/// `staged` 决定恢复到哪一层：Unstaged 区的 Discard 只回退「工作区相对暂存区」的
/// 部分（`git checkout --`），把已暂存的改动一起抹掉是 Staged 区的语义
/// （`git checkout HEAD --`）。
pub fn discard(path: &str, paths: &[String], staged: bool) -> Result<()> {
    // 空 pathspec 会被 git 当成「全部」，那就成了一次误伤整个仓库的 discard
    if paths.is_empty() {
        return Ok(());
    }

    // 失败原样抛出：`ls-files --others` 在空仓库里也是退出 0（实测），它没有
    // 「失败即空集」的合法情形。真失败了还往下走，等于拿一个空集去做分类。
    let mut probe: Vec<&str> = vec!["ls-files", "--others", "--exclude-standard", "-z", "--"];
    probe.extend(paths.iter().map(String::as_str));
    let untracked = path_set(path, &probe)?;

    // `staged == false` 时下面的判断被 `staged &&` 短路，in_head 根本用不到。
    // 那就别跑这条命令——少一次进程，也少一个能出错的面。
    let in_head = if staged {
        let mut head_probe: Vec<&str> = vec!["ls-tree", "-r", "HEAD", "--name-only", "-z", "--"];
        head_probe.extend(paths.iter().map(String::as_str));
        match path_set(path, &head_probe) {
            Ok(set) => set,
            // 只有「仓库还没有任何提交」这一种失败是合法的，其余必须抛出
            Err(_) if head_is_unborn(path) => HashSet::new(),
            Err(err) => return Err(err),
        }
    } else {
        HashSet::new()
    };

    let mut restore: Vec<&str> = Vec::new();
    let mut remove: Vec<&str> = Vec::new();
    let mut unlink: Vec<&str> = Vec::new();
    for p in paths {
        if untracked.contains(p) {
            unlink.push(p);
        } else if staged && !in_head.contains(p) {
            remove.push(p);
        } else {
            restore.push(p);
        }
    }

    // 三个阶段作用在互不相交的路径集上，但**顺序是有意的**：可逆性从高到低，
    // 最不可逆的「直接删文件」放最后。任一步失败就立刻停下，不再往下做更不可逆
    // 的事——代价是用户要再点一次，换来的是出错之后不会继续产生新的不可逆后果。
    // 回滚在这里不存在，所以失败时唯一能补的是 partial_failure 那份交代。
    let mut done: Vec<&str> = Vec::new();

    if !restore.is_empty() {
        let mut args: Vec<&str> = vec!["checkout"];
        if staged {
            args.push("HEAD");
        }
        args.push("--");
        args.extend(restore.iter().copied());
        if let Err(err) = run_git(path, &args) {
            let mut skipped = remove.clone();
            skipped.extend(unlink.iter().copied());
            return Err(partial_failure(
                "恢复文件内容",
                &err.message,
                &done,
                &restore,
                &skipped,
            ));
        }
        done.extend(restore.iter().copied());
    }

    if !remove.is_empty() {
        let mut args: Vec<&str> = vec!["rm", "-f", "--"];
        args.extend(remove.iter().copied());
        if let Err(err) = run_git(path, &args) {
            return Err(partial_failure(
                "移出暂存区并删除",
                &err.message,
                &done,
                &remove,
                &unlink,
            ));
        }
        done.extend(remove.iter().copied());
    }

    // 直接删而不是 `git clean -f`：pathspec 稍有偏差就会波及没被选中的文件，
    // 而这一步不可逆
    let root = std::path::Path::new(path);
    for (i, rel) in unlink.iter().enumerate() {
        let target = root.join(rel);
        let result = if target.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        if let Err(err) = result {
            // 已经不在了就当成功——用户要的就是它消失
            if err.kind() == std::io::ErrorKind::NotFound {
                done.push(rel);
                continue;
            }
            // 这一阶段是逐个文件做的，所以能精确说出卡在哪个文件上，
            // 不需要 uncertain 那一栏
            return Err(partial_failure(
                &format!("删除未跟踪文件「{rel}」"),
                &err.to_string(),
                &done,
                &[],
                &unlink[i..],
            ));
        }
        done.push(rel);
    }

    Ok(())
}
