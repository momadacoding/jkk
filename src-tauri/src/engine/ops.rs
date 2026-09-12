//! 工作区文件操作。
//!
//! 这段逻辑原本直接写在 `commands/ops.rs` 的命令函数里。下沉到 engine 是为了
//! 让 `vcs::GitBackend` 能调用它——命令层往后只做分发，不放业务逻辑。

use crate::engine::git_cli::{run_git, run_git_code};
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
/// `ls-tree HEAD` 失败有好几种原因，而在 `staged` 分支下把「读不出来」当成
/// 「HEAD 里什么都没有」会让文件被 `git rm -f` 删掉。所以判定必须**正向**：
/// HEAD 是个 symbolic ref，且它指向的那个分支 ref **确实不存在**。
///
/// 判据是 `show-ref --verify --quiet` 的**退出码**，不是「它有没有失败」。
/// 2026-09-13 在临时 fixture 上实测的四种形态：
///
/// | 形态 | `show-ref --verify --quiet` | 结论 |
/// |---|---|---|
/// | 仓库还没有任何提交（ref 文件不存在） | **1** | unborn |
/// | 分支 ref 指向不存在的 OID | **128** `bad ref` | **抛出** |
/// | 分支 ref 内容是垃圾 | 1 | 见下 |
/// | ref 在 packed-refs 里（松散文件不存在） | **0** | 非 unborn |
///
/// 第二行是这条判定存在的全部理由：只看「失败与否」会把它当成 unborn，而此时
/// `git rm -f` 照样退出 0 并删掉文件（实测）。
///
/// 第三行是有意接受的行为：git 自己就把内容损坏的 ref 当成不存在
/// （`warning: ignoring broken ref`），`git checkout HEAD -- <file>` 同样取不到
/// 内容，没有可恢复的目标。实测后果是那一版**暂存内容**丢失，已提交的内容仍在
/// 对象库里、可经 fsck 取回。
///
/// 不能用 `for-each-ref` 判 ref 是否存在：它按路径前缀匹配。HEAD 指向
/// `refs/heads/svn`（spec D10 的基线分支名）而仓库里有 `refs/heads/svn/base` 时，
/// 它会吐出后者，把真 unborn 误判成「ref 存在」。`show-ref --verify` 是精确匹配。
///
/// detached HEAD 下 `symbolic-ref` 退出 1，那种状态必然已经指着某个提交，
/// 返回 `Ok(false)` 正确。
fn head_is_unborn(path: &str) -> Result<bool> {
    let (code, head_ref) = run_git_code(path, &["symbolic-ref", "--quiet", "HEAD"])?;
    if code != 0 {
        return Ok(false);
    }
    let head_ref = head_ref.trim();
    if head_ref.is_empty() {
        return Ok(false);
    }

    let (code, _) = run_git_code(path, &["show-ref", "--verify", "--quiet", head_ref])?;
    match code {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(GitError::new(
            "git_cli",
            format!("git show-ref --verify {head_ref} 退出码 {other}，分支 ref 可能已损坏"),
        )),
    }
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
            // 只有「仓库还没有任何提交」这一种失败是合法的，其余必须抛出。
            // 判不清也算抛出——这里的默认值会直接决定文件是被恢复还是被删掉。
            Err(probe_err) => match head_is_unborn(path) {
                Ok(true) => HashSet::new(),
                Ok(false) => return Err(probe_err),
                // 两条线索都留下：ls-tree 说了症状，show-ref 才指出病根
                // （`not a tree object` 对用户毫无信息量，`bad ref <name>` 有）
                Err(ref_err) => {
                    return Err(GitError::new(
                        "git_cli",
                        format!("{}\n{}", probe_err.message.trim_end(), ref_err.message),
                    ))
                }
            },
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
