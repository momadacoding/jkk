//! `sync_out` 编排（spec §8.2 D0–D10）、早退收尾（§8.2.4）、D5 失败分类（§8.6；计划 B3），
//! 以及中断后的继续 / 放弃（§7.6；计划 B4 / B7 / B8，在 Task 8 加）。
//!
//! # 为什么不走 `trait Vcs`、锁、watcher
//!
//! 与 `svn_sync_in` 完全相同：`SvnGitBackend` 的写方法都挂着 `assert_not_syncing`，
//! 本流程全程持锁，走那一层会被自己挡回来；锁在入口拿、返回时由 Drop 释放
//! （D8d 冲突出口也是一次返回）；watcher 由命令层挂起并在结束后补发 `repo-changed`。
//!
//! # 确认在流程之外（计划 B1）
//!
//! 用户对着预览确认（D3）。执行时锁内把前置检查、D0.5、D1 / D2 全部重算，与
//! [`SyncOutExpect`] 比对，不一致就停下让用户重新确认。

use crate::engine::git_cli::{run_git, run_git_code, run_git_env};
use crate::engine::graph::parse_local_only;
use crate::engine::svn_baseline::{
    create_baseline_commit, move_baseline_ref, parse_trailer, read_baseline, swap_index_to_tree,
    TempIndex, Trailer,
};
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_deliver::{deliver_ref, pin, unpin};
use crate::engine::svn_include_check::normalize_roots;
use crate::engine::svn_init::record_server_tree_into;
use crate::engine::svn_journal::{self, Flow, Journal, Phase};
use crate::engine::svn_lock;
use crate::engine::svn_preflight::{read_node_rev, run_preflight, svn_targets};
use crate::engine::svn_stage::{snapshot, stage, unstage};
use crate::engine::svn_sync_check::{
    assert_linear, current_branch, list_paths, rev_parse, unfinished_error, unmerged_paths,
};
use crate::engine::svn_sync_in::{
    checkout_failed, checkout_verified, dirty_w_files, is_baseline_bytes, is_our_rebase, need,
    new_opid, no_unfinished, rebase_in_progress, settle_residue, tracked_changes,
    untracked_leftovers,
};
use crate::engine::svn_sync_out_check::{check_sync_out, git_dirty, DeliverChange, SyncOutPreview};
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::parse_update_status;
use crate::error::{GitError, Result};
use std::path::Path;

/// D5 的提交信息文件，相对仓库根（svn 的 cwd）。
///
/// 提交信息**不走 `-m`**：svn.exe 的 C 运行库按系统 ANSI 代码页解码命令行，ACP 1252
/// 的机器上中文变成 `?`，甚至整条参数错位——2026-10-10 实测 `-m 修复跳跃` 在服务器上
/// 存成了别的参数的值。`-F` + `--encoding UTF-8` 与代码页无关。用相对路径：仓库的
/// 绝对路径本身也可能含非 ASCII。放在 `.git/` 里：svn 不会把它当成要交付的文件。
const MESSAGE_FILE: &str = ".git/mygitclient-commit-msg.txt";

/// 用户在确认框里看到并同意的预览事实（计划 B1）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutExpect {
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    /// 比 tree 不比 commit：慢路径每次重放出来的 commit OID 不同（提交时间），树相同。
    pub deliver_tree: String,
    pub changes: Vec<DeliverChange>,
    pub confirmed_outside: Vec<String>,
    pub message: String,
}

/// 一次交付 / 继续 / 放弃的结局。D8d 的 rebase 冲突**不是错误**，是正常出口。
///
/// `rename_all_fields` 不能省：只有 `rename_all` 时变体内部字段原样发出（`local_only_left`），
/// 前端读 `localOnlyLeft` 拿到 undefined——`SyncVerdict` 上踩过同一个坑。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SyncOutOutcome {
    /// D10 完成。`local_only_left > 0` 时界面不得提示「可删除分支」（spec §8.2.3 D9）。
    Delivered {
        rev: u64,
        branch: String,
        local_only_left: usize,
    },
    /// D8d 的 rebase 冲突：提交已在服务器上、基线已推进，只剩本地补丁没落回去。
    /// journal 停在 `restoring-branch`，锁已释放（spec §13）。
    RebaseConflict { rev: u64, files: Vec<String> },
    /// 继续 / 放弃把一次中断的交付清理掉了。`committed` 为真时提交已在服务器上。
    Abandoned { committed: bool },
}

/// **测试缝**：在流程的几个固定点插一段动作（外部工具改磁盘、同事提交、模拟崩溃）。
/// 生产路径用 [`run_sync_out`]，等价于什么都不插。
pub trait Probe {
    fn at(&self, _step: Step, _repo: &str) {}
}

impl<F: Fn(Step, &str)> Probe for F {
    fn at(&self, step: Step, repo: &str) {
        self(step, repo)
    }
}

/// [`Probe`] 的插入点。每一个都在对应 phase 落盘**之后**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// D0.5 完成，`deliver-built`。
    Built,
    /// D3.8 完成，`checked-out`。
    CheckedOut,
    /// D4 完成，`svn-staged`，D4.9 之前。
    Staged,
    /// D4.9 通过，D5 之前（phase 仍是 `svn-staged`）。
    BeforeCommit,
    /// D5 成功，`post-commit`。
    Committed,
    /// D6 完成，D7 之前（`post-commit`）。
    Updated,
    /// D8a 完成：新基线 commit 已建，指针未动（`publishing-baseline`）。
    Captured,
    /// D8c 完成，`restoring-branch`，D8d 之前。
    Published,
}

/// 一次完整的交付。
pub fn run_sync_out(repo: &str, expect: &SyncOutExpect) -> Result<SyncOutOutcome> {
    run_sync_out_probed(repo, expect, &|_: Step, _: &str| {})
}

/// [`run_sync_out`] 加测试缝。**只给集成测试用**。
pub fn run_sync_out_probed(
    repo: &str,
    expect: &SyncOutExpect,
    probe: &dyn Probe,
) -> Result<SyncOutOutcome> {
    let message = expect.message.trim().to_string();
    if message.is_empty() {
        return Err(GitError::new(
            "sync_out_empty_message",
            "提交信息不能为空。",
        ));
    }
    let opid = new_opid();
    let _lock = svn_lock::acquire(repo, &opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    // 锁内复核：前置检查、D0.5、D1 / D2 全部重算。其中的服务器 revision 查询发生在
    // 用户确认之后，就是 D3.5（计划 B13）
    let p = check_sync_out(repo)?;
    verify_expect(&p, expect)?;
    let cfg = read_config(repo)?;
    let targets = svn_targets(repo, &cfg.whitelist)?;

    // D0：任何修改之前落盘
    let mut j = Journal {
        opid: opid.clone(),
        flow: Flow::SyncOut,
        phase: Phase::Prepare,
        started_at: now_secs(),
        orig_branch: Some(p.orig_branch.clone()),
        orig_head: Some(p.orig_head.clone()),
        baseline_rev_before: Some(p.baseline_rev),
        baseline_oid_before: Some(p.baseline_oid.clone()),
        commit_message: Some(message.clone()),
        expected_start_rev: Some(p.remote_rev),
        local_only_commits: p.local_only.iter().map(|c| c.oid.clone()).collect(),
        svn_added: paths_with(&p.changes, "A"),
        svn_deleted: paths_with(&p.changes, "D"),
        svn_modified: paths_with(&p.changes, "M"),
        ..Default::default()
    };
    svn_journal::write(repo, &j)?;

    // D0.5：ref 名先落盘再建 ref（§7.6 写前）。交付树已在复核时算好（对象层，计划 B2）
    j.phase = Phase::BuildingDeliver;
    j.deliver_ref = Some(deliver_ref(&opid));
    j.deliver_oid = Some(p.deliver_oid.clone());
    svn_journal::write(repo, &j)?;
    if let Err(e) = pin(repo, &opid, &p.deliver_oid) {
        return early_exit(repo, &cfg, &j, e);
    }
    j.phase = Phase::DeliverBuilt;
    svn_journal::write(repo, &j)?;
    probe.at(Step::Built, repo);

    // D3.8：全流程第一次动主工作副本，在用户确认之后
    j.phase = Phase::CheckedOut;
    svn_journal::write(repo, &j)?;
    if let Err(e) = checkout_detached(repo, &cfg, &p.deliver_oid) {
        return early_exit(repo, &cfg, &j, e);
    }
    probe.at(Step::CheckedOut, repo);

    // D4：快照先落盘，再发第一条 svn 命令（§7.6 写前；计划 B6）
    let before = match snapshot(repo, &targets) {
        Ok(b) => b,
        Err(e) => return early_exit(repo, &cfg, &j, e),
    };
    j.phase = Phase::SvnStaged;
    j.svn_status_before = before;
    svn_journal::write(repo, &j)?;
    match stage(repo, &targets, &p.changes, &j.svn_status_before) {
        Ok(rec) => {
            j.svn_delta = rec.delta;
            j.new_parent_dirs = rec.new_parent_dirs;
            svn_journal::write(repo, &j)?;
        }
        Err(e) => return rollback_exit(repo, &cfg, &j, &targets, e),
    }
    probe.at(Step::Staged, repo);

    // D4.9：确认之后磁盘仍可能被外部工具改动（spec §8.2.5）
    if let Err(e) = assert_still_delivering(repo, &cfg, &p.deliver_oid) {
        return rollback_exit(repo, &cfg, &j, &targets, e);
    }
    probe.at(Step::BeforeCommit, repo);

    // D5
    let commit_targets = match commit_targets(&j.new_parent_dirs, &p.changes) {
        Ok(t) => t,
        Err(e) => return rollback_exit(repo, &cfg, &j, &targets, e),
    };
    let msg_file = Path::new(repo).join(MESSAGE_FILE);
    if let Err(e) = std::fs::write(&msg_file, message.as_bytes()) {
        return rollback_exit(repo, &cfg, &j, &targets, e.into());
    }
    j.phase = Phase::Committing;
    svn_journal::write(repo, &j)?;
    let opid_prop = format!("mygitclient:opid={opid}");
    let mut args: Vec<&str> = vec![
        "commit",
        "--non-interactive",
        "--depth=empty",
        // 计划 B14 原为 `-m` + `--force-log`。改走文件见 MESSAGE_FILE；`--force-log` 仍留着：
        // 用户 `svn add --force .` 会把 `.git` 本身登记进去，那时 svn 拒绝受控文件当 -F
        "--force-log",
        "--with-revprop",
        &opid_prop,
        "-F",
        MESSAGE_FILE,
        "--encoding",
        "UTF-8",
        "--",
    ];
    args.extend(commit_targets.iter().map(String::as_str));
    let ran = run_svn_code(repo, &args);
    // 留着也无害（下次覆盖写），删不掉不影响结局
    let _ = std::fs::remove_file(&msg_file);
    let out = match ran {
        Ok(o) => o,
        // svn 根本没起来：什么都没发出去，服务器确定没变
        Err(e) => {
            j.phase = Phase::SvnStaged;
            svn_journal::write(repo, &j)?;
            return rollback_exit(repo, &cfg, &j, &targets, e);
        }
    };
    if out.code != 0 {
        return match classify_commit_failure(&out.stderr, &out.stdout) {
            CommitFailure::Rejected => {
                // 先把 phase 退回 svn-staged 再撤：撤到一半崩了，重开时走撤销而不是「状态未知」
                j.phase = Phase::SvnStaged;
                svn_journal::write(repo, &j)?;
                rollback_exit(repo, &cfg, &j, &targets, commit_rejected(&out.stderr))
            }
            CommitFailure::Unknown => Err(commit_unknown(&j, &out.stderr)),
        };
    }
    j.committed_rev = parse_committed_rev(&out.stdout);
    j.phase = Phase::PostCommit;
    svn_journal::write(repo, &j)?;
    probe.at(Step::Committed, repo);

    after_commit(repo, &cfg, &mut j, probe)
}

/// D6–D10。继续 `post-commit` 时也从这里进（Task 8）——那时调用方先确认过 HEAD 还在交付树上。
pub(crate) fn after_commit(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    probe: &dyn Probe,
) -> Result<SyncOutOutcome> {
    update_whitelist(repo, cfg, j, Then::Continue)?; // D6
    probe.at(Step::Updated, repo);
    capture_delivered(repo, cfg, j)?; // D7、D7a、D8a
    probe.at(Step::Captured, repo);
    publish_delivered(repo, cfg, j)?; // D8b、D8c
    probe.at(Step::Published, repo);
    restore_branch(repo, cfg, j) // D8d、D9、D10
}

fn verify_expect(p: &SyncOutPreview, e: &SyncOutExpect) -> Result<()> {
    if p.orig_branch != e.orig_branch
        || p.orig_head != e.orig_head
        || p.baseline_oid != e.baseline_oid
    {
        return Err(state_changed(
            "预览之后分支或基线变了（比如在 IDE 里又提交了一次）",
        ));
    }
    if p.deliver_tree != e.deliver_tree
        || p.changes != e.changes
        || p.outside_changes != e.confirmed_outside
    {
        return Err(GitError::new(
            "sync_out_needs_reconfirm",
            "要交付的内容和你确认时看到的不一样了（白名单里 git 管不到的文件也算）。\
             什么都没有改动，请重新点「交付」再确认一次。",
        ));
    }
    Ok(())
}

/// D3.8：detached 切到交付树，事后核对真的切干净了（计划 A15 同一手法：退出码证明不了什么）。
fn checkout_detached(repo: &str, cfg: &SvnConfig, oid: &str) -> Result<()> {
    let from = rev_parse(repo, "HEAD")?;
    let ran = run_git(repo, &["checkout", "--detach", oid]);
    let on_target = current_branch(repo)?.is_none() && rev_parse(repo, "HEAD")? == oid;
    let mut stale = tracked_changes(repo, cfg)?;
    stale.extend(untracked_leftovers(repo, cfg, &[from])?);
    if on_target && stale.is_empty() {
        return Ok(());
    }
    Err(checkout_failed(
        repo,
        &oid[..oid.len().min(10)],
        &stale,
        ran,
    ))
}

/// D4.9（spec §8.2.5）：HEAD 仍是交付树，白名单里 git 没有任何改动。
fn assert_still_delivering(repo: &str, cfg: &SvnConfig, deliver: &str) -> Result<()> {
    let head = rev_parse(repo, "HEAD")?;
    let dirty = git_dirty(repo, cfg)?;
    if head == deliver && current_branch(repo)?.is_none() && dirty.is_empty() {
        return Ok(());
    }
    let mut what = Vec::new();
    if head != deliver {
        what.push(format!(
            "HEAD 现在是 {}，应为交付树 {}",
            &head[..head.len().min(10)],
            &deliver[..deliver.len().min(10)]
        ));
    }
    if !dirty.is_empty() {
        what.push(format!("白名单里有改动：\n{}", list_paths(&dirty)));
    }
    Err(GitError::new(
        "sync_out_disk_changed",
        format!(
            "工作副本在你确认之后被外部工具改动过（{}）。再提交的话，交上去的会是磁盘上的这些字节，\
             而不是你确认过的内容，所以这里中止。",
            what.join("；")
        ),
    ))
}

/// D5 的 target：新增的父目录 + 显式文件清单（spec §8.2.6）。
///
/// **空了就拒绝**（spec §8.3）：空 target 的 `svn commit` 会提交整个工作副本，且不可撤销。
/// 只有父目录、没有文件也拒绝——那说明清单在别处丢了。
pub fn commit_targets(
    new_parent_dirs: &[String],
    changes: &[DeliverChange],
) -> Result<Vec<String>> {
    if changes.is_empty() {
        return Err(GitError::new(
            "sync_out_empty_targets",
            "拒绝发出没有文件的 svn commit：空目标会提交整个工作副本，且不可撤销。",
        ));
    }
    Ok(new_parent_dirs
        .iter()
        .chain(changes.iter().map(|c| &c.path))
        .map(|p| peg_safe(p))
        .collect())
}

/// D5 非零退出时，服务器是不是**确定**没变（计划 B3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFailure {
    /// 被服务器明确拒绝：撤销 D4 登记、收尾。
    Rejected,
    /// 拿不准：不回滚、不推进，phase 停在 `committing`（计划 B4）。
    Unknown,
}

/// 「服务器确定什么都没做」的错误码。开工前实测 M2：out-of-date 是 E155011 + E160028，
/// 同事删了文件是 E155011 + E160013，钩子拒绝是 E165001；其余来自 spec §8.6 的表。
const REJECTED_CODES: [&str; 8] = [
    "E155011", "E160028", "E160013", "E160024", "E165001", "E170001", "E200009", "E200007",
];

pub fn classify_commit_failure(stderr: &str, stdout: &str) -> CommitFailure {
    if stdout.contains("Committed revision") {
        return CommitFailure::Unknown; // svn 说提交了：绝不回滚
    }
    let codes = error_codes(stderr);
    if !codes.is_empty() && codes.iter().all(|c| REJECTED_CODES.contains(&c.as_str())) {
        CommitFailure::Rejected
    } else {
        CommitFailure::Unknown
    }
}

/// stderr 里所有 `E` + 6 位数字的错误码。
fn error_codes(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for i in 0..b.len() {
        let starts = b[i] == b'E' && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if starts
            && i + 7 <= b.len()
            && b[i + 1..i + 7].iter().all(u8::is_ascii_digit)
            && (i + 7 == b.len() || !b[i + 7].is_ascii_digit())
        {
            out.push(s[i..i + 7].to_string());
        }
    }
    out
}

fn parse_committed_rev(stdout: &str) -> Option<u64> {
    let rest = &stdout[stdout.find("Committed revision ")? + "Committed revision ".len()..];
    rest.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn commit_rejected(stderr: &str) -> GitError {
    GitError::new(
        "sync_out_commit_rejected",
        format!(
            "SVN 拒绝了这次提交，服务器上什么都没有发生。svn 原话：\n{}\n\
             常见原因：有人刚提交了你也改过的文件（先「更新」再交付）、凭据或权限不对、\
             服务器的钩子不接受。",
            stderr.trim()
        ),
    )
}

fn commit_unknown(j: &Journal, stderr: &str) -> GitError {
    let start = j.expected_start_rev.unwrap_or(0);
    GitError::new(
        "sync_out_commit_unknown",
        format!(
            "提交已经发出，但没有拿到服务器的明确答复：\n{}\n\
             它可能已经成功，也可能没有。本版本还不能自动查证，所以什么都不再动：\
             不撤销登记，也不推进基线。请在 TortoiseSVN 的 Show log 里看从 r{} 起有没有\
             提交信息为「{}」的 revision（它带着属性 mygitclient:opid={}）。记录已保留。",
            stderr.trim(),
            start + 1,
            j.commit_message.as_deref().unwrap_or(""),
            j.opid
        ),
    )
}

/// D6 是替谁跑的：报错里告诉用户之后点哪个按钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Then {
    Continue,
    Stop,
}

/// 「停止收尾」为什么也要先 D6（Task 8–10 里程碑 review 第 4 条）。
const STOP_NEEDS_UPDATE: &str =
    "「停止收尾」也要先把白名单更新到最新：不更新就切回分支，工作副本会停在 \
     mixed-revision，之后的「更新」会把同事的改动当成你的未提交改动。";

/// D6（spec §8.2.1）：只更新白名单路径。冲突只看白名单 target 之内。
fn update_whitelist(repo: &str, cfg: &SvnConfig, j: &Journal, then: Then) -> Result<()> {
    let targets = svn_targets(repo, &cfg.whitelist)?;
    let pegged: Vec<String> = targets.iter().map(|t| peg_safe(t)).collect();
    let mut args: Vec<&str> = vec!["update", "--accept", "postpone", "--non-interactive", "--"];
    args.extend(pegged.iter().map(String::as_str));
    let committed = committed_text(j);
    let up = run_svn_code(repo, &args)?;
    if up.code != 0 {
        let retry = match then {
            Then::Continue => "网络恢复后在顶部横幅上点「继续交付」，不会重复提交。".to_string(),
            Then::Stop => format!(
                "{STOP_NEEDS_UPDATE}网络恢复后再点「继续交付」或「停止收尾」都可以，不会重复提交。"
            ),
        };
        return Err(GitError::new(
            "sync_out_update_failed",
            format!(
                "{committed}，但把白名单更新到最新时失败了（常见原因：VPN 断了）。基线还没推进，\
                 记录已保留。{retry}svn 原话：{}",
                up.stderr.trim()
            ),
        ));
    }
    // 成败不看退出码：--accept postpone 遇到冲突也退出 0（2c 计划 A4，实测）
    let mut conflicts = Vec::new();
    for t in &pegged {
        let xml = run_svn(repo, &["status", "--xml", "--non-interactive", "--", t])?;
        conflicts.extend(
            parse_update_status(&xml)?
                .entries
                .into_iter()
                .filter(|e| e.is_conflicted())
                .map(|e| e.path),
        );
    }
    if !conflicts.is_empty() {
        let button = match then {
            Then::Continue => "继续交付",
            Then::Stop => "停止收尾",
        };
        return Err(GitError::new(
            "sync_out_update_conflict",
            format!(
                "{committed}，但更新白名单时有冲突：\n{}\n这些是 git 管不到的文件（被扩展名排除的，\
                 或目录级的树冲突），本应用没法替你解决。请用 TortoiseSVN 处理（Resolve），\
                 然后在顶部横幅上点「{button}」。基线还没推进，不会重复提交。",
                list_paths(&conflicts)
            ),
        ));
    }
    Ok(())
}

fn committed_text(j: &Journal) -> String {
    match j.committed_rev {
        Some(r) => format!("提交已经成功（r{r}）"),
        None => "提交已经成功".to_string(),
    }
}

/// D7 + D7a + D8a。D7a 的纳入集合核对不过时停在 `post-commit`（spec §13）。
fn capture_delivered(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    // 从 HEAD 树起步录（§6.3.1）——HEAD 必须还是交付树，否则录的就不是它
    if current_branch(repo)?.is_some() || rev_parse(repo, "HEAD")? != deliver {
        return Err(state_changed("HEAD 已经不在交付树上了"));
    }
    let pre = run_preflight(repo, &cfg.whitelist)?; // D7：nodeRevs 单值
    let idx = TempIndex::from_head(repo)?;
    let (tree, _explained) = record_server_tree_into(repo, cfg, &pre, &idx)?; // D7a
    let now = read_node_rev(repo, &cfg.whitelist)?;
    if now != pre.node_rev {
        return Err(GitError::new(
            "sync_out_revision_moved",
            format!(
                "{}，但录新基线的过程中工作副本被外部改动了（r{} → r{now}）。基线没有推进，\
                 记录已保留；外部操作结束后在顶部横幅上点「继续交付」。",
                committed_text(j),
                pre.node_rev
            ),
        ));
    }
    // 创建 commit 前记录 captureTree（§7.6）
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree.clone());
    j.new_baseline_oid = None;
    svn_journal::write(repo, j)?;
    let trailer = Trailer {
        url: pre.repos_root,
        rev: pre.node_rev,
        uuid: pre.uuid,
    };
    // 父提交是交付树，不是旧基线（spec §8.2.2；§7.6 末「不能套用 S7 的旧基线父」）
    let commit = create_baseline_commit(repo, &tree, Some(&deliver), &trailer)?;
    // 移动 ref 前记录 newBaselineOid（§7.6）
    j.new_baseline_oid = Some(commit);
    svn_journal::write(repo, j)
}

/// D8b + D8c。重开时也从这里续（`publishing-baseline` 且已有 `newBaselineOid`）。
pub(crate) fn publish_delivered(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    match read_baseline(repo)?.as_deref() {
        Some(cur) if cur == new => {} // 已发布（崩溃恢复时会走到）
        Some(cur) if cur == old => move_baseline_ref(repo, &new, Some(&old))?,
        other => {
            return Err(GitError::new(
                "svn_baseline_moved",
                format!(
                    "基线分支现在是 {other:?}，既不是交付前的 {old}，也不是本次录好的 {new}。\
                     有别的工具动过它，这里停下，不覆盖。提交已在服务器上，记录已保留。"
                ),
            ))
        }
    }
    // D8c：索引从交付树换成新基线树（CAS），磁盘不动——磁盘已经是 D6 之后的候选快照
    let (code, _) = run_git_code(repo, &["diff-index", "--cached", "--quiet", &new])?;
    if code != 0 {
        swap_index_to_tree(repo, &deliver, &new)?;
    }
    // 树 == 索引 == 磁盘，git 不写任何文件
    if current_branch(repo)?.as_deref() != Some(BASELINE_BRANCH) {
        checkout_verified(repo, cfg, BASELINE_BRANCH, false)?;
    }
    j.phase = Phase::RestoringBranch;
    svn_journal::write(repo, j)
}

/// D8d（spec §8.2.2）+ D9 / D10。
pub(crate) fn restore_branch(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let new = published(repo, j)?;
    let rev = delivered_rev(repo, j, &new)?;
    let orig_ref = format!("refs/heads/{orig}");

    if j.local_only_commits.is_empty() {
        // 只有两个合法起点：刚发布完停在新基线上，或用户已手工切回原分支（下面处理）。
        // 别处一律不动——中断期间在那里做过的提交，切走之后只剩 reflog 找得回
        // （最终 review 第 2 条；停止收尾那一侧由 require_left_in_place 拦）
        let cur = current_branch(repo)?;
        let on_new_base =
            cur.as_deref() == Some(BASELINE_BRANCH) && rev_parse(repo, "HEAD")? == new;
        if !on_new_base && cur.as_deref() != Some(orig.as_str()) {
            let head = rev_parse(repo, "HEAD")?;
            return Err(state_changed(&format!(
                "提交已在服务器上（r{rev}），基线已推进，但 HEAD 现在在 {}，既不在新基线上也不在原分支 \
                 `{orig}` 上。中断期间若在那里做过提交，切走就会丢掉它们，所以不动。先处理好它们，再 \
                 `git checkout {BASELINE_BRANCH}` 回到新基线后点「我已解决，继续」",
                cur.unwrap_or_else(|| head[..head.len().min(10)].to_string()),
            )));
        }
        // 计划 B10：快进 = 直接把分支指针移到新基线（CAS），再从基线切过去——两者是同一个
        // commit，git 不写任何文件。spec 字面的 checkout + merge --ff-only 会多两次全量重编译
        let tip = rev_parse(repo, &orig_ref)?;
        if tip == orig_head && current_branch(repo)?.as_deref() == Some(orig.as_str()) {
            // 中断期间用户手工切回了原分支（Task 8–10 里程碑 review 第 3 条）。下面的
            // update-ref 会把**当前**分支挪走、索引和磁盘却留在旧内容上——git 看来就是撤掉了
            // 同事的改动。先回到基线分支（verified），磁盘随之写成新基线，再走原路
            if !tracked_changes(repo, cfg)?.is_empty() {
                return Err(state_changed(&format!(
                    "提交已在服务器上（r{rev}），基线已推进，但分支 `{orig}` 已被切回、而且有未提交的\
                     改动。先提交或撤掉它们再点「我已解决，继续」"
                )));
            }
            checkout_verified(repo, cfg, BASELINE_BRANCH, false)?;
        }
        if tip == orig_head {
            let (code, _) = run_git_code(repo, &["merge-base", "--is-ancestor", &orig_head, &new])?;
            if code != 0 {
                return Err(GitError::new(
                    "sync_out_not_fast_forward",
                    format!(
                        "提交已在服务器上（r{rev}），基线已推进，但分支 `{orig}` 不能快进到新基线——\
                         前面某一步的假设不成立了。这里停下，不自作主张 merge。记录已保留。"
                    ),
                ));
            }
            run_git(repo, &["update-ref", &orig_ref, &new, &orig_head])?;
        } else if tip != new {
            return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
        }
        if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
            checkout_verified(repo, cfg, &orig, false)?;
        }
    } else {
        match current_branch(repo)?.as_deref() {
            // 磁盘带上本地补丁，D6 拉下来的别人的改动暂时退回——rebase 之后两者都在
            Some(b) if b == BASELINE_BRANCH => checkout_verified(repo, cfg, &orig, false)?,
            Some(b) if b == orig => {}
            other => {
                return Err(state_changed(&format!(
                    "HEAD 在 {other:?} 上，既不是基线分支也不是原分支 `{orig}`"
                )))
            }
        }
        if rev_parse(repo, "HEAD")? != orig_head {
            return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
        }
        // 计划 B11：rebase 真正要重放的区间从 merge-base 开始
        let mb = run_git(repo, &["merge-base", &new, &orig_head])?
            .trim()
            .to_string();
        assert_linear(repo, &mb, &orig_head)?;
        // 与 sync_in 的 S9 同一组 -c：用户配置不能改变 rebase 的语义（2c 独立 review C1）
        if let Err(e) = run_git(
            repo,
            &[
                "-c",
                "rebase.autoStash=false",
                "-c",
                "rebase.updateRefs=false",
                "rebase",
                "--empty=drop",
                &new,
            ],
        ) {
            return rebase_conflict_or_error(repo, rev, e);
        }
    }
    finish(repo, j, rev)
}

pub(crate) fn rebase_conflict_or_error(
    repo: &str,
    rev: u64,
    e: GitError,
) -> Result<SyncOutOutcome> {
    let files = unmerged_paths(repo)?;
    if rebase_in_progress(repo)? && !files.is_empty() {
        return Ok(SyncOutOutcome::RebaseConflict { rev, files });
    }
    Err(GitError::new(
        "sync_out_rebase_failed",
        format!(
            "提交已在服务器上（r{rev}），基线已推进，但把本地补丁接到新基线上失败了，而且不是冲突。\
             记录已保留：按 git 说的处理好之后点「我已解决，继续」。git 原话：{}",
            e.message.trim()
        ),
    ))
}

/// D10：断言成功路径自己的不变量（spec §8.2.4 末尾的表），再复用 ③ 与 ⑤。
pub(crate) fn finish(repo: &str, j: &Journal, rev: u64) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
        return Err(state_changed(&format!(
            "收尾时 HEAD 不在原分支 `{orig}` 上"
        )));
    }
    let range = format!("refs/heads/{BASELINE_BRANCH}..refs/heads/{orig}");
    let out = run_git(repo, &["log", "--format=%H%x00%s%x00%B%x1e", &range])?;
    let mut left = 0usize;
    let mut undelivered = Vec::new();
    for rec in out
        .split('\u{1e}')
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
    {
        let mut f = rec.splitn(3, '\0');
        let (oid, subject, body) = (
            f.next().unwrap_or(""),
            f.next().unwrap_or(""),
            f.next().unwrap_or(""),
        );
        if parse_local_only(body) {
            left += 1;
        } else {
            undelivered.push(format!("{} {subject}", &oid[..oid.len().min(10)]));
        }
    }
    if !undelivered.is_empty() {
        return Err(GitError::new(
            "sync_out_undelivered_left",
            format!(
                "提交已在服务器上（r{rev}），基线已推进，但分支 `{orig}` 上还剩下没标「仅本地」的提交：\n{}\n\
                 它们该交付却没进交付树。这里停下，记录已保留，请把记录导出给开发者看。",
                list_paths(&undelivered)
            ),
        ));
    }
    // ③：成功路径上 deliverRef 还在；⑤：删 journal（锁由调用方的守卫释放）
    if let Some(r) = &j.deliver_ref {
        unpin(repo, r)?;
    }
    svn_journal::remove(repo)?;
    Ok(SyncOutOutcome::Delivered {
        rev,
        branch: orig,
        local_only_left: left,
    })
}

/// spec §8.2.4：D5 之前所有出口共用的收尾。每一步幂等（崩溃恢复会重跑）。
pub(crate) fn cleanup_before_svn(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<()> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    // ①：本期不建临时 worktree（计划 B2），所以这一步什么都不做。`tempWorktree` 字段
    // 若出现，只可能来自手改或别的版本写的 journal——对着那里的路径
    // `worktree remove --force` 会连同未提交的改动一起删掉别人的 worktree（里程碑 review），
    // 不替用户做这件事
    // ②：只有 D3.8 之后才会真的切
    match current_branch(repo)? {
        Some(b) if b == orig => {}
        None if Some(rev_parse(repo, "HEAD")?) == j.deliver_oid => {
            back_from_deliver(repo, cfg, j, &orig, &orig_head)?;
        }
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是原分支 `{orig}` 也不是交付树"
            )))
        }
    }
    // ③
    if let Some(r) = &j.deliver_ref {
        unpin(repo, r)?;
    }
    // ④：什么都不该发生过
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    // ⑤
    svn_journal::remove(repo)
}

/// 从交付树切回原分支。普通切换被挡住时（多半是 D3.8 没切干净：Unity 占着文件，
/// 磁盘上新旧两种字节混着，计划 A15），只要每个不一致的文件都是交付树或原分支里
/// 原样存着的字节（或已不在），`-f` 就不会丢任何东西；否则原样报错、不覆盖。
fn back_from_deliver(
    repo: &str,
    cfg: &SvnConfig,
    j: &Journal,
    orig: &str,
    orig_head: &str,
) -> Result<()> {
    let Err(first) = checkout_verified(repo, cfg, orig, false) else {
        return Ok(());
    };
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    let bases = [deliver.clone(), orig_head.to_string()];
    let mut paths = tracked_changes(repo, cfg)?;
    paths.extend(changed_between(repo, cfg, &deliver, orig_head)?);
    for p in &paths {
        if !is_baseline_bytes(repo, &bases, p)? {
            return Err(first);
        }
    }
    checkout_verified(repo, cfg, orig, true)
}

/// `a` 与 `b` 之间白名单内有差异的路径。
fn changed_between(repo: &str, cfg: &SvnConfig, a: &str, b: &str) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(Vec::new()); // 空 pathspec 会变成「全部」（§12.2）
    }
    let mut args: Vec<&str> = vec!["diff", "--name-only", "-z", a, b, "--"];
    args.extend(roots.iter().map(String::as_str));
    Ok(run_git(repo, &args)?
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect())
}

/// D4 之前的出口：收尾，并把结果接在原因后面（原因永远在前）。
fn early_exit(repo: &str, cfg: &SvnConfig, j: &Journal, cause: GitError) -> Result<SyncOutOutcome> {
    let tail = match cleanup_before_svn(repo, cfg, j) {
        Ok(()) => format!(
            "已回到分支 `{}`，SVN 侧没有任何改动。",
            j.orig_branch.as_deref().unwrap_or("")
        ),
        Err(e) => format!(
            "另外：收尾没有完成（{}）。记录已保留，处理好之后在顶部横幅上点「清理」。",
            e.message
        ),
    };
    Err(GitError::new(
        &cause.code,
        format!("{}\n\n{tail}", cause.message),
    ))
}

/// D4 之后、D5 落地之前的出口：先按快照撤销登记（spec §8.5），再收尾。
fn rollback_exit(
    repo: &str,
    cfg: &SvnConfig,
    j: &Journal,
    targets: &[String],
    cause: GitError,
) -> Result<SyncOutOutcome> {
    if let Err(e) = unstage(repo, targets, &j.svn_status_before) {
        return Err(GitError::new(
            &cause.code,
            format!(
                "{}\n\n另外：撤销 SVN 登记没有完成（{}）。记录已保留，处理好之后在顶部横幅上点「清理」。",
                cause.message, e.message
            ),
        ));
    }
    early_exit(repo, cfg, j, cause)
}

fn published(repo: &str, j: &Journal) -> Result<String> {
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    if read_baseline(repo)?.as_deref() != Some(new.as_str()) {
        return Err(GitError::new(
            "svn_baseline_moved",
            "基线分支已经不是本次交付发布的那个了，有别的工具动过它。这里停下，记录已保留。",
        ));
    }
    Ok(new)
}

/// 报给用户的是**他自己的**提交（D5 的 `Committed revision`），不是 D6 更新到的
/// revision——两者之间同事可能又提交过，拿后者去服务器 log 里找会找到别人的提交
/// （里程碑 review）。D5 的输出没读出来（极少）时退回新基线的 revision。
fn delivered_rev(repo: &str, j: &Journal, new_baseline: &str) -> Result<u64> {
    match j.committed_rev {
        Some(r) => Ok(r),
        None => trailer_rev(repo, new_baseline),
    }
}

fn trailer_rev(repo: &str, commit: &str) -> Result<u64> {
    let msg = run_git(repo, &["log", "-1", "--format=%B", commit])?;
    parse_trailer(&msg)
        .map(|t| t.rev)
        .ok_or_else(|| GitError::new("svn_baseline_trailer", "新基线上读不到 git-svn-id。"))
}

pub(crate) fn state_changed(what: &str) -> GitError {
    GitError::new(
        "sync_out_state_changed",
        format!(
            "{what}。这里停下，不覆盖任何东西；请重新点「交付」（或处理好之后点横幅上的按钮）。"
        ),
    )
}

fn paths_with(changes: &[DeliverChange], status: &str) -> Vec<String> {
    changes
        .iter()
        .filter(|c| c.status == status)
        .map(|c| c.path.clone())
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// —— 继续 / 放弃（spec §7.6「提供继续 / 放弃两个入口」；计划 B4 / B7 / B8）——

/// 从中断处接着走。
pub fn continue_sync_out(repo: &str) -> Result<SyncOutOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    match j.phase {
        // B8：往 D5 走需要用户重新看过清单；崩溃之后什么都可能变了
        Phase::Prepare
        | Phase::BuildingDeliver
        | Phase::DeliverBuilt
        | Phase::CheckedOut
        | Phase::SvnStaged => abandon_before_commit(repo, &cfg, j),
        Phase::Committing => Err(commit_unknown(j, "（上一次在提交途中中断）")),
        Phase::PostCommit => {
            // D6 之前：HEAD 必须还在交付树上（里程碑 review 第 1 条）
            check_rebase(repo, j)?;
            require_left_in_place(repo, j)?;
            after_commit(repo, &cfg, j, &|_: Step, _: &str| {})
        }
        Phase::PublishingBaseline => {
            check_rebase(repo, j)?;
            require_left_in_place(repo, j)?;
            if j.new_baseline_oid.is_none() {
                // 没有新 OID = commit 还没建：从 D7 重录（§7.6）
                capture_delivered(repo, &cfg, j)?;
            }
            publish_delivered(repo, &cfg, j)?;
            restore_branch(repo, &cfg, j)
        }
        Phase::RestoringBranch => continue_restoring(repo, &cfg, j),
        p => Err(unknown_phase(p)),
    }
}

/// 放弃这次交付。
pub fn abort_sync_out(repo: &str) -> Result<SyncOutOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    match j.phase {
        Phase::Prepare
        | Phase::BuildingDeliver
        | Phase::DeliverBuilt
        | Phase::CheckedOut
        | Phase::SvnStaged => abandon_before_commit(repo, &cfg, j),
        Phase::Committing => Err(commit_unknown(j, "（上一次在提交途中中断）")),
        Phase::PostCommit | Phase::PublishingBaseline | Phase::RestoringBranch => {
            stop_after_commit(repo, &cfg, j)
        }
        p => Err(unknown_phase(p)),
    }
}

/// 拿锁并在锁内重读 journal（两次读之间可能被另一个进程推进过）。
fn take_over(repo: &str) -> Result<(svn_lock::SyncLock, Journal)> {
    let first = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    if first.flow != Flow::SyncOut {
        return Err(wrong_flow());
    }
    let lock = svn_lock::acquire(repo, &first.opid)?;
    let j = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    if j.opid != first.opid || j.flow != Flow::SyncOut {
        return Err(state_changed("记录在拿锁的那一瞬间被换掉了"));
    }
    Ok((lock, j))
}

/// D5 之前的 phase：撤销登记（若有）并走 §8.2.4 收尾。
fn abandon_before_commit(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<SyncOutOutcome> {
    if j.phase == Phase::SvnStaged {
        let targets = svn_targets(repo, &cfg.whitelist)?;
        unstage(repo, &targets, &j.svn_status_before)?;
    }
    cleanup_before_svn(repo, cfg, j)?;
    Ok(SyncOutOutcome::Abandoned { committed: false })
}

/// 计划 B7：提交已在服务器上，放弃 = 不再替用户做后面的事。
fn stop_after_commit(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    // A14：冲突期间锁是放开的，不是我们发起的 rebase 不碰。先于 D6 判：别人的 rebase
    // 进行中时 HEAD 也不在交付树上，svn update 不能跑
    if check_rebase(repo, j)? {
        run_git(repo, &["rebase", "--abort"])?;
    }
    if j.phase == Phase::PostCommit {
        // 只提交、没更新的工作副本是 mixed-revision（实测 M2-E6），「更新」与「重建基线」都不接。
        // 先把白名单拉齐，留下一个「整体前进」的状态，让「更新」能接管（2c 的 A16）。
        // D6 之前 HEAD 必须还在交付树上（里程碑 review 第 1 条）
        require_left_in_place(repo, j)?;
        update_whitelist(repo, cfg, j, Then::Stop)?;
    }
    if current_branch(repo)?.as_deref() == Some(orig.as_str()) {
        // D8d 切回原分支那次 checkout、或刚才的 rebase --abort 可能退出 0 却没写干净
        // （2c 的 A15；里程碑 review 第 2 条）。不核对就报「已停止收尾」，磁盘上会是
        // 撤掉了本地补丁的内容
        let mut bases: Vec<String> = j.new_baseline_oid.iter().cloned().collect();
        bases.push(need(&j.deliver_oid, "deliverOid")?);
        settle_residue(
            repo,
            cfg,
            &bases,
            &orig,
            "sync_out_unsafe_to_return",
            "「停止收尾」",
        )?;
    } else {
        // `-f` 会把 HEAD 从这里切走：中断期间在 detached HEAD 上做的提交、切去的别的分支，
        // 都不在下面 W-file 的检查范围里（里程碑 review 第 7 条）
        require_left_in_place(repo, j)?;
        // 能 -f 的前提：W-file 在 svn 看来全部干净 = 磁盘就是服务器内容，能取回（2c 的 A5）
        let dirty = dirty_w_files(repo, cfg)?;
        if !dirty.is_empty() {
            return Err(GitError::new(
                "sync_out_unsafe_to_return",
                format!(
                    "要回到分支 `{orig}` 就得覆盖工作区里的文件，而下面这些文件的内容既不是 git 写的、\
                     也不是服务器上的，不知道是谁改的，不能覆盖：\n{}\n记录已保留。请先确认这些改动\
                     （需要的话自己备份），再点「停止收尾」。",
                    list_paths(&dirty)
                ),
            ));
        }
        checkout_verified(repo, cfg, &orig, true)?;
    }
    // 原分支已经接在本次发布的基线上（D8d 的快进 update-ref 做完、或 rebase 走完只差 D10）：
    // 交付其实已经完成，报「已停止、分支回到交付前、请更新」是假话（最终 review 第 7 条）
    if let Some(new) = &j.new_baseline_oid {
        let base_ref = format!("refs/heads/{BASELINE_BRANCH}");
        if rev_parse(repo, &base_ref)? == *new && is_ancestor(repo, new)? {
            return finish(repo, j, delivered_rev(repo, j, new)?);
        }
    }
    if let Some(r) = &j.deliver_ref {
        unpin(repo, r)?;
    }
    svn_journal::remove(repo)?;
    Ok(SyncOutOutcome::Abandoned { committed: true })
}

/// `restoring-branch` 的继续：按 HEAD、原分支 OID 与 git 的 rebase 状态续 D8d 或进 D10。
/// 不回到 D6 / D8c——那会覆盖用户正在解决的冲突（§7.6）。
fn continue_restoring(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let new = published(repo, j)?;
    let rev = delivered_rev(repo, j, &new)?;
    if check_rebase(repo, j)? {
        let files = unmerged_paths(repo)?;
        if !files.is_empty() {
            return Err(unresolved(&files));
        }
        // GIT_EDITOR=true：非交互地接受 git 给的提交信息（2c 实测）
        if let Err(e) = run_git_env(
            repo,
            &[
                "-c",
                "rebase.autoStash=false",
                "-c",
                "rebase.updateRefs=false",
                "rebase",
                "--continue",
            ],
            &[("GIT_EDITOR", "true")],
        ) {
            return rebase_conflict_or_error(repo, rev, e);
        }
        return finish(repo, j, rev);
    }
    let on_orig = current_branch(repo)?.as_deref() == Some(orig.as_str());
    if on_orig && rev_parse(repo, "HEAD")? != orig_head {
        // D8d 已经做完（快进或 rebase 走完了），只差 D10
        if !is_ancestor(repo, &new)? {
            return Err(state_changed(
                "原分支还没有落到新基线上（rebase 可能在外面被中止了）；要停在这里，请点「停止收尾」",
            ));
        }
        return finish(repo, j, rev);
    }
    restore_branch(repo, cfg, j)
}

fn wrong_flow() -> GitError {
    GitError::new(
        "sync_out_wrong_flow",
        "这条记录是「更新」流程留下的，请用横幅上更新那一组按钮处理。",
    )
}

/// 交付记录里出现了交付流程不会写的 phase（手改、或别的版本写的记录）。
fn unknown_phase(p: Phase) -> GitError {
    let name = serde_json::to_value(p)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{p:?}"));
    GitError::new(
        "sync_out_unknown_phase",
        format!(
            "交付记录停在「{name}」这一步，这个版本认不出它（多半是别的版本写的记录）。\
             这里什么都不动，记录已保留；请不要手工去动分支指针。"
        ),
    )
}

/// 有没有进行中的 rebase；有的话它必须是本次 D8d 发起的那一个（A14），返回 `true`。
///
/// D8d 只发生在 `restoring-branch`、且那时新基线已经发布——更早的 phase 里出现的 rebase
/// 一定不是我们的，不能拿还没有的 `newBaselineOid` 去认（那会被报成 journal 损坏，
/// 里程碑 review 第 5 条）。
fn check_rebase(repo: &str, j: &Journal) -> Result<bool> {
    if !rebase_in_progress(repo)? {
        return Ok(false);
    }
    if j.phase == Phase::RestoringBranch {
        let onto = published(repo, j)?;
        if is_our_rebase(repo, j, &onto)? {
            return Ok(true);
        }
    }
    Err(state_changed(
        "有一个进行中的 rebase 不是本次交付发起的。它的冲突和进度归它自己，这里不碰；\
         先在 git 里把它做完或中止",
    ))
}

/// 提交之后、回到原分支之前，HEAD 必须还在本流程放它的位置：交付树（detached），或
/// D8c 之后的新基线分支。否则 D6 的 svn update 会合进一条从没打算碰的分支，`checkout -f`
/// 会把中断期间在 detached HEAD 上做的提交切丢（里程碑 review 第 1 / 7 条）。
fn require_left_in_place(repo: &str, j: &Journal) -> Result<()> {
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    let head = rev_parse(repo, "HEAD")?;
    let branch = current_branch(repo)?;
    let on_deliver = branch.is_none() && head == deliver;
    let on_new_base = j.phase != Phase::PostCommit
        && branch.as_deref() == Some(BASELINE_BRANCH)
        && j.new_baseline_oid.as_deref() == Some(head.as_str());
    if on_deliver || on_new_base {
        return Ok(());
    }
    let target = j
        .deliver_ref
        .clone()
        .unwrap_or_else(|| deliver[..deliver.len().min(10)].to_string());
    Err(state_changed(&format!(
        "HEAD 已经不在本次交付放它的位置上了（应在交付树 {} 上，现在在 {}）。中断期间若在这里做过\
         提交，切走就会丢掉它们，所以不动。只是切了分支的话，先 `git checkout --detach {target}` \
         回到交付树",
        &deliver[..deliver.len().min(10)],
        branch.unwrap_or_else(|| head[..head.len().min(10)].to_string()),
    )))
}

fn unresolved(files: &[String]) -> GitError {
    GitError::new(
        "sync_out_unresolved",
        format!(
            "还有冲突没解决：\n{}\n改好之后把它们暂存（git add），再点「我已解决，继续」。",
            list_paths(files)
        ),
    )
}

/// `a` 在不在 HEAD 的历史里。
fn is_ancestor(repo: &str, a: &str) -> Result<bool> {
    match run_git_code(repo, &["merge-base", "--is-ancestor", a, "HEAD"])?.0 {
        0 => Ok(true),
        1 => Ok(false),
        code => Err(state_changed(&format!(
            "判断 {a} 是否在 HEAD 的历史里失败了（merge-base 退出 {code}）"
        ))),
    }
}
