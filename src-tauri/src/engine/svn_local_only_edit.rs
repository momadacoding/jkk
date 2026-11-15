//! 事后改 Local-Only 标记：取消、加上、把一部分文件拆成普通 commit（2026-11-11 使用报告 9.2）。
//!
//! 设计与前置条件见 `docs/superpowers/plans/2026-11-15-local-only-toggle-decisions.md`。要点：
//!
//! - **分支末端的树不变。** 取消 / 加上只改 commit C 的说明；拆分把 C 换成 C1（C^ 加上选中的文件）
//!   与 C2（C 的树）。C 之后的 commit 原样搬树、只换父提交，不需要合并、不会冲突。
//! - **只动一个 ref。** 新 commit 都只是对象，最后对当前分支做一次 CAS（`update-ref <分支> <新> <旧>`）。
//!   之前任何一步失败，没有 ref 动过，工作区、索引一个字节都没碰过。
//! - **别的分支不动。** 它们上面的旧 C 保持原样，结果里列出来，由界面告诉用户。
//!
//! 持同步锁：跟进、更新、交付都会移动分支，跟进的崩溃恢复按 journal 里记的 OID 走。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::git_read::is_ancestor;
use crate::engine::graph::parse_local_only;
use crate::engine::svn_config::BASELINE_BRANCH;
use crate::engine::svn_journal;
use crate::engine::svn_lock;
use crate::engine::svn_replay::{
    changed_paths, tree_without, write_commit, ReplayCodes, ReplayCommit,
};
use crate::engine::svn_sync_check::{check_git_idle, current_branch, rev_parse};
use crate::error::{GitError, Result};
use std::collections::BTreeSet;

/// 改写的结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditOutcome {
    pub branch: String,
    /// 改写前的分支末端：reflog 里也有，`git reset --keep <它>` 能回去。
    pub old_tip: String,
    pub tip: String,
    /// 被改的那个 commit 改写之后的 OID（拆分时是留下的 Local-Only 那一个）。界面据此选中它
    /// （独立 review 第 4 条）。
    pub commit: String,
    /// 别的本地分支上也有这个 commit：它们没动，留着旧的。
    pub other_branches: Vec<String>,
}

const CODES: ReplayCodes = ReplayCodes {
    conflict: "local_only_edit_failed",
    too_old: "local_only_edit_failed",
    too_old_message: "git 版本太旧。",
    failed: "local_only_edit_failed",
};

const TRAILER: &str = "Local-Only: true";

/// 取消（`on = false`）或加上（`on = true`）commit `target` 的 Local-Only 标记。
pub fn set_local_only(repo: &str, opid: &str, target: &str, on: bool) -> Result<EditOutcome> {
    let _lock = svn_lock::acquire(repo, opid)?;
    let ctx = prepare(repo, target)?;
    let c = &ctx.chain[0];
    if c.local_only == on {
        return Err(GitError::new(
            "local_only_edit_no_change",
            if on {
                "这个 commit 已经标着「仅本地」了。"
            } else {
                "这个 commit 没有标「仅本地」。"
            },
        ));
    }
    let first = c.rewrite(repo, &c.tree, &c.parents[0], &with_trailer(&c.body, on))?;
    let verb = if on {
        "标为仅本地"
    } else {
        "取消仅本地"
    };
    finish(repo, &ctx, first, verb)
}

/// 把 Local-Only commit `target` 里的 `paths` 拆成一个普通 commit，放在它前面；其余的文件留在
/// Local-Only commit 里。`paths` 是仓库相对路径，反斜杠按正斜杠认。
pub fn split_local_only(
    repo: &str,
    opid: &str,
    target: &str,
    paths: &[String],
) -> Result<EditOutcome> {
    let _lock = svn_lock::acquire(repo, opid)?;
    let ctx = prepare(repo, target)?;
    let c = &ctx.chain[0];
    if !c.local_only {
        return Err(GitError::new(
            "local_only_split_not_local_only",
            "这个 commit 没有标「仅本地」，不用拆。",
        ));
    }
    let changed = changed_paths(repo, &c.parents[0], &c.oid)?;
    if let Some((dir, file)) = dir_file_pair(&changed) {
        // 同一个位置上目录换成了同名文件（或反过来）：按文件拆要在临时索引里同时放下目录与文件，
        // `update-index` 拒绝；手动跟进用的换条目逻辑也只认文件（独立 review 第 3 条）
        return Err(GitError::new(
            "local_only_split_dir_file",
            format!(
                "这个 commit 在 {dir} 上把目录换成了同名文件（或反过来，涉及 {file}），没法按文件拆。\
                 要让它整个上服务器，用「取消仅本地」。"
            ),
        ));
    }
    let chosen: BTreeSet<String> = paths.iter().map(|p| p.replace('\\', "/")).collect();
    let bad: Vec<&String> = chosen.iter().filter(|p| !changed.contains(*p)).collect();
    if chosen.is_empty() || !bad.is_empty() || chosen.len() == changed.len() {
        let why = if chosen.is_empty() {
            "没有选中任何文件。".to_string()
        } else if !bad.is_empty() {
            format!(
                "这些文件这个 commit 没有改过：{}",
                bad.iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            )
        } else {
            "选中了全部文件：那等于取消「仅本地」，请用那一项。".to_string()
        };
        return Err(GitError::new("local_only_split_bad_selection", why));
    }
    // C1：C 去掉没选中的文件（换回 C^ 的条目），说明去掉标记；C2：C 的树，说明原样
    let rest: BTreeSet<String> = changed.difference(&chosen).cloned().collect();
    let c1_tree = tree_without(repo, &c.as_replay(), &rest, &CODES)?;
    let c1 = c.rewrite(repo, &c1_tree, &c.parents[0], &with_trailer(&c.body, false))?;
    let c2 = c.rewrite(repo, &c.tree, &c1, &c.body)?;
    finish(repo, &ctx, c2, "拆分仅本地")
}

/// 拆分弹框里列的文件：commit 相对它第一个父提交改过的路径，与 [`split_local_only`] 校验选中项时
/// 用的是同一份（不认改名）。只读，不拿锁。
pub fn local_only_files(repo: &str, commit: &str) -> Result<Vec<String>> {
    let oid = rev_parse(repo, &format!("{commit}^{{commit}}"))?;
    let parent = rev_parse(repo, &format!("{oid}~1"))?;
    Ok(changed_paths(repo, &parent, &oid)?.into_iter().collect())
}

/// 改写链上的一个 commit：一次 `git log` 读出的全部元数据。
struct ChainCommit {
    oid: String,
    parents: Vec<String>,
    tree: String,
    name: String,
    email: String,
    /// `--date=raw`。
    date: String,
    body: String,
    local_only: bool,
}

impl ChainCommit {
    /// 作者取自自己，树、父提交、说明按给的。
    fn rewrite(&self, repo: &str, tree: &str, parent: &str, message: &str) -> Result<String> {
        write_commit(
            repo,
            tree,
            parent,
            &self.name,
            &self.email,
            &self.date,
            message,
        )
    }

    fn as_replay(&self) -> ReplayCommit {
        ReplayCommit {
            oid: self.oid.clone(),
            parents: self.parents.clone(),
            subject: self.body.lines().next().unwrap_or("").to_string(),
            body: self.body.clone(),
            local_only: self.local_only,
        }
    }
}

struct Prepared {
    branch: String,
    head: String,
    /// `target` 到 HEAD，从旧到新；第一个就是 `target`。
    chain: Vec<ChainCommit>,
}

/// 前置条件，任何一条不满足都在动任何东西之前拒绝。调用方持同步锁。
fn prepare(repo: &str, target: &str) -> Result<Prepared> {
    if svn_journal::read(repo)?.is_some() {
        return Err(GitError::new(
            "local_only_edit_unfinished_sync",
            "有一次 SVN 同步还没做完。请先在顶部横幅上处理，再改「仅本地」标记。",
        ));
    }
    check_git_idle(repo, "local_only_edit_git_busy", "改「仅本地」标记")?;
    let branch = match current_branch(repo)? {
        Some(b) if b != BASELINE_BRANCH => b,
        _ => {
            return Err(GitError::new(
                "local_only_edit_not_on_branch",
                "HEAD 不在工作分支上。请先切到这个 commit 所在的分支。",
            ))
        }
    };
    let head = rev_parse(repo, "HEAD^{commit}")?;
    let target = rev_parse(repo, &format!("{target}^{{commit}}"))?;
    if !is_ancestor(repo, &target, &head)? {
        return Err(GitError::new(
            "local_only_edit_not_on_branch",
            format!("这个 commit 不在当前分支 {branch} 上。请先切到它所在的分支。"),
        ));
    }
    let base = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}^{{commit}}"))?;
    if is_ancestor(repo, &target, &base)? {
        return Err(GitError::new(
            "local_only_edit_in_baseline",
            "这个 commit 已经在基线（svn/base）里了，改它的标记没有意义。",
        ));
    }
    // `target^..HEAD`；根提交不可能不在基线里，到不了这里
    let parent = rev_parse(repo, &format!("{target}~1"))?;
    let chain = load_chain(repo, &parent, &head)?;
    // 从旧到新：第一个必须就是 target。target 之前若有从别处合进来的 commit，`target^..HEAD` 会多列，
    // 那也是非线性
    let linear = chain.first().is_some_and(|c| c.oid == target)
        && chain.iter().all(|c| c.parents.len() == 1)
        && chain.windows(2).all(|w| w[1].parents[0] == w[0].oid);
    if !linear {
        return Err(GitError::new(
            "local_only_edit_not_linear",
            "这个 commit 之后的历史里有合并（merge），没法只改这一个 commit 的标记。",
        ));
    }
    Ok(Prepared {
        branch,
        head,
        chain,
    })
}

/// `base..head` 从旧到新，连同树与作者一次读出：之后每个 commit 只需要一次 `commit-tree`
/// （原来每个还要各一次 `rev-parse ^{tree}` 与 `log -1`，几百个 commit 时持锁要几十秒，独立 review 第 5 条）。
fn load_chain(repo: &str, base: &str, head: &str) -> Result<Vec<ChainCommit>> {
    let out = run_git(
        repo,
        &[
            "log",
            "--reverse",
            "--topo-order",
            "--date=raw",
            "--format=%H%x00%P%x00%T%x00%an%x00%ae%x00%ad%x00%B%x1e",
            &format!("{base}..{head}"),
        ],
    )?;
    let mut list = Vec::new();
    for rec in out.split('\u{1e}') {
        let rec = rec.trim_start_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let f: Vec<&str> = rec.splitn(7, '\0').collect();
        let [oid, parents, tree, name, email, date, body] = f[..] else {
            return Err(GitError::new(
                "local_only_edit_failed",
                format!("读不懂 git log 的输出：{rec:?}"),
            ));
        };
        list.push(ChainCommit {
            oid: oid.to_string(),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            tree: tree.to_string(),
            name: name.to_string(),
            email: email.to_string(),
            date: date.to_string(),
            body: body.trim_end_matches('\n').to_string(),
            local_only: parse_local_only(body),
        });
    }
    Ok(list)
}

/// 把 `chain[1..]` 原样搬树接到 `first` 上，CAS 移动分支。
fn finish(repo: &str, ctx: &Prepared, first: String, verb: &str) -> Result<EditOutcome> {
    let mut tip = first.clone();
    for d in &ctx.chain[1..] {
        tip = d.rewrite(repo, &d.tree, &tip, &d.body)?;
    }
    let refname = format!("refs/heads/{}", ctx.branch);
    let target = &ctx.chain[0].oid;
    let reason = format!("mygitclient: {verb} {}", &target[..target.len().min(10)]);
    // CAS：读完 HEAD 到这里之间分支被别人挪了，就什么都不改
    match run_git_code(
        repo,
        &["update-ref", "-m", &reason, &refname, &tip, &ctx.head],
    )? {
        (0, _) => {}
        (_, out) => {
            return Err(GitError::new(
                "local_only_edit_branch_moved",
                format!(
                    "分支 {} 在改写过程中被移动了，什么都没改。请再试一次。{}",
                    ctx.branch,
                    out.trim()
                ),
            ))
        }
    }
    // 分支已经移动了：从这里起什么失败都不能再报成错误，否则用户以为没改成、再点一次（独立 review 第 1 条）
    let other_branches = other_branches(repo, target, &ctx.branch).unwrap_or_else(|e| {
        log::warn!("改写已完成，但列不出别的分支：{}", e.message);
        Vec::new()
    });
    Ok(EditOutcome {
        branch: ctx.branch.clone(),
        old_tip: ctx.head.clone(),
        tip,
        commit: first,
        other_branches,
    })
}

/// 别的本地分支里还含着旧 `target` 的（不含基线分支）。
fn other_branches(repo: &str, target: &str, branch: &str) -> Result<Vec<String>> {
    let out = run_git(
        repo,
        &[
            "for-each-ref",
            "--contains",
            target,
            "--format=%(refname)",
            "refs/heads/",
        ],
    )?;
    Ok(out
        .lines()
        .filter_map(|l| l.trim().strip_prefix("refs/heads/"))
        .filter(|b| *b != branch && *b != BASELINE_BRANCH)
        .map(str::to_string)
        .collect())
}

/// 改过的路径里，有没有一个是另一个的上级目录（同一个位置上目录与文件互换）。有就交回一对。
fn dir_file_pair(changed: &BTreeSet<String>) -> Option<(String, String)> {
    changed.iter().find_map(|p| {
        let prefix = format!("{p}/");
        changed
            .range(prefix.clone()..)
            .next()
            .filter(|q| q.starts_with(&prefix))
            .map(|q| (p.clone(), q.clone()))
    })
}

/// 加上或去掉 `Local-Only: true`。
///
/// 加：已经有了就原样；没有就在末尾单独成段（同提交框的 `buildCommitMessage`：不单独成段的话
/// `git interpret-trailers` 不认它）。去：删掉所有被 `parse_local_only` 认作标记的行；按段处理，
/// 删空的段整段去掉，段与段之间仍是一个空行（独立 review 第 7 条：原来逐行删，标记在中间时会留下
/// 两个连续空行，交付时原样进 SVN）。
pub fn with_trailer(message: &str, on: bool) -> String {
    let trimmed = message.trim_end();
    if on {
        if parse_local_only(trimmed) {
            return trimmed.to_string();
        }
        return format!("{trimmed}\n\n{TRAILER}");
    }
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in trimmed.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(current.join("\n"));
                current.clear();
            }
            continue;
        }
        if !parse_local_only(line) {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(current.join("\n"));
    }
    paragraphs.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::{dir_file_pair, with_trailer};
    use std::collections::BTreeSet;

    #[test]
    fn adds_the_trailer_as_its_own_paragraph() {
        assert_eq!(with_trailer("subject", true), "subject\n\nLocal-Only: true");
        assert_eq!(
            with_trailer("subject\n\nbody\n\n", true),
            "subject\n\nbody\n\nLocal-Only: true"
        );
    }

    #[test]
    fn adding_twice_changes_nothing() {
        let once = with_trailer("s", true);
        assert_eq!(with_trailer(&once, true), once);
        // 大小写不同的写法也算已经有了（判定同 parse_local_only）
        assert_eq!(
            with_trailer("s\n\nlocal-only: TRUE", true),
            "s\n\nlocal-only: TRUE"
        );
    }

    #[test]
    fn removes_only_the_trailer_line() {
        assert_eq!(with_trailer("s\n\nLocal-Only: true", false), "s");
        assert_eq!(
            with_trailer("s\n\nbody\n\nSigned-off-by: x\nLocal-Only: true\n", false),
            "s\n\nbody\n\nSigned-off-by: x"
        );
        // 没有标记：原样（去掉末尾空白）
        assert_eq!(with_trailer("s\n\nbody\n", false), "s\n\nbody");
    }

    #[test]
    fn a_trailer_in_the_middle_leaves_a_single_blank_line() {
        assert_eq!(
            with_trailer("s\n\nLocal-Only: true\n\nmore details", false),
            "s\n\nmore details"
        );
        assert_eq!(
            with_trailer("s\r\n\r\nLocal-Only: true\r\n\r\nmore", false),
            "s\n\nmore"
        );
    }

    #[test]
    fn local_only_false_is_not_treated_as_the_marker() {
        assert_eq!(
            with_trailer("s\n\nLocal-Only: false", false),
            "s\n\nLocal-Only: false"
        );
    }

    #[test]
    fn finds_a_directory_replaced_by_a_file_of_the_same_name() {
        let set = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        assert_eq!(
            dir_file_pair(&set(&["A/B", "A/B/x.lua", "C.lua"])),
            Some(("A/B".to_string(), "A/B/x.lua".to_string()))
        );
        // 只是名字前缀相同，不是上下级
        assert_eq!(dir_file_pair(&set(&["A/B", "A/B.meta", "A/Bc/x"])), None);
        assert_eq!(dir_file_pair(&set(&[])), None);
    }
}
