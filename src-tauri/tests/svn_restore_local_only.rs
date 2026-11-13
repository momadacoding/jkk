#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进后把 Local-Only 补丁放回磁盘（使用报告 9.1；计划 `2026-11-12-restore-local-only-patches.md`）。
//!
//! 全部在临时 fixture 上跑。形状取自 2026-11-11 的真实现场：Local-Only commit 加一行，之后一个
//! 普通 commit 在 git 里删掉它，这个文件随 TortoiseSVN 提交上了 SVN；跟进后普通 commit 变空被丢，
//! Local-Only commit 照常重放，分支比磁盘（= SVN）多出那一行。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::{follow, FollowOutcome, FollowReport};
use app_lib::engine::svn_follow_mark::write_mark;
use app_lib::engine::svn_journal::{self, Flow, Journal, Phase};
use app_lib::engine::svn_restore_local_only::{
    dismiss_pending_restores, pending_list, pending_restores, restore_local_only,
};
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const L_MSG: &str = "add @avatarclass\n\nLocal-Only: true";
const CLASS: &str = "---@class Avatar\n";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args)
        .unwrap_or_else(|e| panic!("git {args:?}: {}", e.message))
        .trim()
        .to_string()
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}
fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn commit_all(fx: &SvnFixture, msg: &str) {
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", msg]);
}
fn tortoise_commit(fx: &SvnFixture, rel: &str) {
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", rel],
    );
}
/// wc1 里提交的作者就是系统用户；设成「我」，F2 ① 才认得出我的 SVN 提交。
fn i_commit(fx: &SvnFixture) {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(["info", "--show-item", "last-changed-author", "--", A])
        .output()
        .unwrap();
    assert!(out.status.success());
    let me = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some(me);
    write_config(&fx.wc_str(), &cfg).unwrap();
}
fn done(o: FollowOutcome) -> FollowReport {
    match o {
        FollowOutcome::Done { report } => *report,
        other => panic!("应当 Done：{other:?}"),
    }
}
fn modified_paths(fx: &SvnFixture) -> Vec<String> {
    git(fx, &["diff", "--name-only"])
        .lines()
        .map(str::to_string)
        .collect()
}

fn base_text() -> String {
    (1..=6).map(|i| format!("l{i}\n")).collect()
}
fn feature_text() -> String {
    base_text().replace("l6\n", "l6\nfeature()\n")
}

/// 真实现场的形状，返回跟进报告。之后：磁盘 = SVN = `feature_text()`，分支 = `CLASS` + 它。
fn report_case(fx: &SvnFixture) -> FollowReport {
    report_case_for(fx, &[A])
}

/// 同 [`report_case`]，几个文件一起。
///
/// 跟进会自动放回（SVN 上最后改它的是我），这里再把磁盘改回 SVN 的版本：按钮测的是「磁盘缺补丁」
/// 的现场（自动放回没写成、或者用户又去掉了补丁）。
fn report_case_for(fx: &SvnFixture, files: &[&str]) -> FollowReport {
    let rep = followed_case_for(fx, files);
    for f in files {
        write(fx, f, &feature_text());
    }
    rep
}

/// 真实现场走到跟进完成为止，不碰之后的磁盘。
fn followed_case_for(fx: &SvnFixture, files: &[&str]) -> FollowReport {
    i_commit(fx);
    for f in files {
        write(fx, f, &base_text());
        tortoise_commit(fx, f);
    }
    done(follow(&fx.wc_str(), "op0").unwrap());
    for f in files {
        write(fx, f, &format!("{CLASS}{}", base_text()));
    }
    commit_all(fx, L_MSG);
    // 「小婉问答」：在 git 里删掉这一行，另加功能，再整个文件用 TortoiseSVN 提交
    for f in files {
        write(fx, f, &feature_text());
    }
    commit_all(fx, "小婉问答");
    write_mark(&fx.wc_str()).unwrap();
    for f in files {
        tortoise_commit(fx, f);
    }
    done(follow(&fx.wc_str(), "op1").unwrap())
}

#[test]
fn follow_puts_the_patch_back_when_i_was_the_last_to_commit_the_file() {
    // 计划 2026-11-12-auto-restore-local-only 的 A：跟进完成后在同一把锁里放回
    let fx = hybrid_with_colleague();
    let rep = followed_case_for(&fx, &[A]);
    assert_eq!(
        rep.drift,
        vec![A.to_string()],
        "前提：报「分支与磁盘对不上」"
    );
    assert_eq!(rep.restorable, vec![A.to_string()]);

    let restore = rep.restore.expect("跟进结果里有放回结果");
    assert_eq!(restore.restored, vec![A.to_string()], "{restore:?}");
    assert!(restore.skipped.is_empty(), "{restore:?}");
    let commits = &restore.commits[A];
    assert_eq!(commits.len(), 1, "{commits:?}");
    assert!(
        commits[0].ends_with(" add @avatarclass"),
        "写明是哪个补丁：{commits:?}"
    );
    assert_eq!(read(&fx, A), format!("{CLASS}{}", feature_text()));
    assert!(modified_paths(&fx).is_empty(), "磁盘 = 索引 = HEAD");
    assert_eq!(git(&fx, &["diff", "--cached", "--name-only"]), "");
    assert!(
        pending_restores(&fx.wc_str()).unwrap().is_empty(),
        "放回了就不挂横幅"
    );
}

#[test]
fn report_case_lists_the_file_as_restorable_and_the_button_puts_the_patch_back() {
    let fx = hybrid_with_colleague();
    let rep = report_case(&fx);
    assert_eq!(rep.restorable, vec![A.to_string()]);
    assert_eq!(read(&fx, A), feature_text(), "前提：磁盘缺补丁");

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    assert_eq!(out.restored, vec![A.to_string()], "{out:?}");
    assert!(out.skipped.is_empty(), "{out:?}");
    assert_eq!(read(&fx, A), format!("{CLASS}{}", feature_text()));
    assert!(modified_paths(&fx).is_empty(), "磁盘 = 索引 = HEAD");
    assert_eq!(git(&fx, &["diff", "--cached", "--name-only"]), "");
}

#[test]
fn user_edit_on_disk_after_the_report_is_not_overwritten() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    let mine = format!("{}my edit\n", feature_text());
    write(&fx, A, &mine);

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    assert!(out.restored.is_empty(), "{out:?}");
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert_eq!(out.skipped[0].path, A);
    assert_eq!(read(&fx, A), mine, "磁盘上用户的编辑原样保留");
}

#[test]
fn staged_version_is_not_overwritten() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    write(&fx, A, "staged\n");
    git(&fx, &["add", A]);
    write(&fx, A, &feature_text());

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    assert!(out.restored.is_empty(), "{out:?}");
    assert_eq!(git(&fx, &["show", ":Code/A.cs"]), "staged", "暂存原样保留");
    assert_eq!(read(&fx, A), feature_text());
}

#[test]
fn a_normal_commit_touching_the_file_makes_it_not_restorable() {
    // 分支比 SVN 多出的不全是 Local-Only：放回去等于把没交给 SVN 的普通改动也写进磁盘
    let fx = hybrid_with_colleague();
    report_case(&fx);
    write(&fx, A, &format!("{CLASS}{}normal\n", feature_text()));
    commit_all(&fx, "普通改动");
    write(&fx, A, &feature_text());

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    assert!(out.restored.is_empty(), "{out:?}");
    assert_eq!(read(&fx, A), feature_text());
}

#[test]
fn file_already_matching_the_branch_is_skipped_with_its_own_reason() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    // 第二次点：磁盘已经是分支的版本，不再等于 SVN。原因要说清，不能报成「可能有你自己的编辑」
    let out = restore_local_only(&fx.wc_str(), "op3", &[A.to_string()]).unwrap();

    assert!(out.restored.is_empty(), "{out:?}");
    assert_eq!(out.skipped[0].reason, "磁盘上已经是分支的版本，不用放回。");
    assert_eq!(read(&fx, A), format!("{CLASS}{}", feature_text()));
}

/// PR 159 review 第 2 条：没装换行规则的旧仓库，`checkout-index` 按 autocrlf 写出 CRLF，SVN 里整个
/// 文件都变了。按钮直接拒绝，什么都不写。
#[test]
fn refuses_in_a_repo_without_the_eol_rule() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    std::fs::write(fx.wc().join(".git/info/attributes"), "").unwrap();

    let e = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap_err();

    assert_eq!(e.code, "svn_restore_needs_eol_migration", "{}", e.message);
    assert_eq!(read(&fx, A), feature_text());
}

/// 第 1 条：skip-worktree 的条目 `checkout-index` 拒绝写；用户有意设的，跳过并说明。
#[test]
fn skip_worktree_entry_is_skipped() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    git(&fx, &["update-index", "--skip-worktree", A]);

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    assert!(out.restored.is_empty(), "{out:?}");
    assert!(out.skipped[0].reason.contains("skip-worktree"), "{out:?}");
    assert_eq!(read(&fx, A), feature_text());
}

/// 第 3 条：分支落后基线时 `base..HEAD` 只剩 Local-Only commit，但写回 HEAD 的版本会撤掉基线
/// 后来的 SVN 改动。
#[test]
fn branch_behind_the_baseline_is_not_restored() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    // 基线往前走一步、改 A：HEAD 不再是它的后代
    let tree = git(&fx, &["write-tree"]);
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let ahead = git(
        &fx,
        &["commit-tree", &tree, "-p", &base, "-m", "SVN 后来的改动"],
    );
    git(&fx, &["update-ref", "refs/heads/svn/base", &ahead]);

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap();

    assert!(out.restored.is_empty(), "{out:?}");
    assert!(out.skipped[0].reason.contains("落后"), "{out:?}");
    assert_eq!(read(&fx, A), feature_text());
}

/// 第 1 条：一个文件被别的程序独占打开、写不进去时，其余的照样写成，各自报实际结果；
/// 不能整体报错，也不能把写成的那个说成失败。
#[cfg(windows)]
#[test]
fn a_locked_file_does_not_hide_the_ones_that_were_restored() {
    use std::os::windows::fs::OpenOptionsExt;
    const B: &str = "Code/B.cs";
    let fx = hybrid_with_colleague();
    let rep = report_case_for(&fx, &[A, B]);
    assert_eq!(rep.restorable, vec![A.to_string(), B.to_string()]);
    // share_mode(0)：不许别人读写删，`checkout-index` 换不掉它，`hash-object` 也读不了
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(fx.wc().join(B))
        .unwrap();

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string(), B.to_string()]).unwrap();
    drop(held);

    assert_eq!(out.restored, vec![A.to_string()], "{out:?}");
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert_eq!(out.skipped[0].path, B);
    assert_eq!(read(&fx, A), format!("{CLASS}{}", feature_text()));
    assert_eq!(read(&fx, B), feature_text(), "被锁住的没被改");
}

/// 同上，但锁只挡写、不挡读：写之前的核对通过，`checkout-index` 写到 B 时失败、退出非零。
/// 这才走到「写了一半」的路径。
#[cfg(windows)]
#[test]
fn a_write_failure_midway_reports_each_file_by_what_is_on_disk() {
    use std::os::windows::fs::OpenOptionsExt;
    const B: &str = "Code/B.cs";
    const FILE_SHARE_READ: u32 = 1;
    let fx = hybrid_with_colleague();
    report_case_for(&fx, &[A, B]);
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(fx.wc().join(B))
        .unwrap();

    let out = restore_local_only(&fx.wc_str(), "op2", &[A.to_string(), B.to_string()]).unwrap();
    drop(held);

    assert_eq!(out.restored, vec![A.to_string()], "{out:?}");
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert_eq!(out.skipped[0].path, B);
    assert!(
        out.skipped[0]
            .reason
            .starts_with("没写进去，磁盘上还是 SVN 的版本"),
        "{out:?}"
    );
    assert_eq!(read(&fx, B), feature_text());

    // 再点一次：A 已经放回，B 这回能写了
    let again = restore_local_only(&fx.wc_str(), "op3", &[A.to_string(), B.to_string()]).unwrap();
    assert_eq!(again.restored, vec![B.to_string()], "{again:?}");
    assert_eq!(
        again.skipped[0].reason,
        "磁盘上已经是分支的版本，不用放回。"
    );
}

#[test]
fn empty_list_touches_nothing() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    let out = restore_local_only(&fx.wc_str(), "op2", &[]).unwrap();
    assert!(out.restored.is_empty() && out.skipped.is_empty(), "{out:?}");
    assert_eq!(read(&fx, A), feature_text());
}

#[test]
fn path_outside_the_branch_is_skipped_not_an_error() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    let out = restore_local_only(
        &fx.wc_str(),
        "op2",
        &["Code/none.cs".to_string(), A.to_string()],
    )
    .unwrap();
    assert_eq!(out.restored, vec![A.to_string()], "{out:?}");
    assert_eq!(out.skipped.len(), 1, "{out:?}");
    assert!(!fx.wc().join("Code/none.cs").exists());
}

#[test]
fn refuses_while_a_sync_is_unfinished() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    svn_journal::write(
        &fx.wc_str(),
        &Journal {
            opid: "x".into(),
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            ..Default::default()
        },
    )
    .unwrap();

    let e = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap_err();

    assert_eq!(e.code, "svn_restore_unfinished_sync", "{}", e.message);
    assert_eq!(read(&fx, A), feature_text());
}

#[test]
fn refuses_while_another_git_holds_the_index() {
    let fx = hybrid_with_colleague();
    report_case(&fx);
    let lock = fx.wc().join(".git/index.lock");
    std::fs::write(&lock, b"").unwrap();

    let e = restore_local_only(&fx.wc_str(), "op2", &[A.to_string()]).unwrap_err();
    std::fs::remove_file(&lock).unwrap();

    assert_eq!(e.code, "svn_index_busy", "{}", e.message);
    assert_eq!(read(&fx, A), feature_text());
}

/// 路径带空格与 `%`：写回走 `--stdin -z`，不经命令行拆参。不用中文：本机 ACP 1252，svn.exe 收不了
/// 中文参数（同 `svn_follow.rs` 的 `follow_handles_paths_with_spaces_and_percent`）。
#[test]
fn path_with_spaces_and_percent_is_restored() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    let p = "Code/avatar members 100%.cs";
    write(&fx, p, &base_text());
    svn_cmd(&fx.wc(), &["add", "-q", "--", p]);
    tortoise_commit(&fx, p);
    done(follow(&fx.wc_str(), "op0").unwrap());
    write(&fx, p, &format!("{CLASS}{}", base_text()));
    commit_all(&fx, L_MSG);
    write(&fx, p, &feature_text());
    commit_all(&fx, "小婉问答");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, p);
    let rep = done(follow(&fx.wc_str(), "op1").unwrap());
    assert_eq!(rep.restorable, vec![p.to_string()], "{rep:?}");
    assert_eq!(
        rep.restore.map(|r| r.restored),
        Some(vec![p.to_string()]),
        "自动放回"
    );
    assert_eq!(read(&fx, p), format!("{CLASS}{}", feature_text()));

    // 按钮同样走 `--stdin -z`
    write(&fx, p, &feature_text());
    let out = restore_local_only(&fx.wc_str(), "op2", &[p.to_string()]).unwrap();

    assert_eq!(out.restored, vec![p.to_string()], "{out:?}");
    assert_eq!(read(&fx, p), format!("{CLASS}{}", feature_text()));
}

// —— 待放回清单（计划的 C） ——

/// 跟进时 B 被别的程序挡着写（锁只放行读）：A 照样放回，B 记进待放回清单；锁放开后横幅能放回它，
/// 放回之后清单清空。
#[cfg(windows)]
#[test]
fn a_file_held_open_during_follow_stays_pending_until_the_banner_restores_it() {
    use std::os::windows::fs::OpenOptionsExt;
    const B: &str = "Code/B.cs";
    const FILE_SHARE_READ: u32 = 1;
    let fx = hybrid_with_colleague();
    // 先把现场造到最后一次跟进之前：followed_case_for 内部会跟进，这里自己展开
    i_commit(&fx);
    for f in [A, B] {
        write(&fx, f, &base_text());
        tortoise_commit(&fx, f);
    }
    done(follow(&fx.wc_str(), "op0").unwrap());
    for f in [A, B] {
        write(&fx, f, &format!("{CLASS}{}", base_text()));
    }
    commit_all(&fx, L_MSG);
    for f in [A, B] {
        write(&fx, f, &feature_text());
    }
    commit_all(&fx, "小婉问答");
    write_mark(&fx.wc_str()).unwrap();
    for f in [A, B] {
        tortoise_commit(&fx, f);
    }
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(fx.wc().join(B))
        .unwrap();

    let rep = done(follow(&fx.wc_str(), "op1").unwrap());

    let restore = rep.restore.expect("有放回结果");
    assert_eq!(restore.restored, vec![A.to_string()], "{restore:?}");
    assert_eq!(restore.skipped.len(), 1, "{restore:?}");
    assert_eq!(restore.skipped[0].path, B);
    assert!(
        restore.commits.contains_key(B),
        "没放回的也写明补丁：{restore:?}"
    );
    // 锁还在时横幅照样列出：核对只读，读得了
    let pending = pending_restores(&fx.wc_str()).unwrap();
    assert_eq!(
        pending.iter().map(|p| p.path.as_str()).collect::<Vec<_>>(),
        vec![B]
    );
    drop(held);

    let out = restore_local_only(&fx.wc_str(), "op2", &[B.to_string()]).unwrap();

    assert_eq!(out.restored, vec![B.to_string()], "{out:?}");
    assert_eq!(read(&fx, B), format!("{CLASS}{}", feature_text()));
    assert!(pending_restores(&fx.wc_str()).unwrap().is_empty());
}

/// 「不放回」：从清单删掉，磁盘不动；空列表什么都不做。
#[test]
fn dismissing_a_pending_restore_leaves_the_disk_alone() {
    let fx = hybrid_with_colleague();
    let held = not_mine_case(&fx);
    let pending = pending_restores(&fx.wc_str()).unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");

    dismiss_pending_restores(&fx.wc_str(), "op3", &[]).unwrap();
    assert_eq!(
        pending_restores(&fx.wc_str()).unwrap().len(),
        1,
        "空列表不删"
    );

    dismiss_pending_restores(&fx.wc_str(), "op3", &[A.to_string()]).unwrap();

    assert!(pending_restores(&fx.wc_str()).unwrap().is_empty());
    assert_eq!(read(&fx, A), held, "磁盘不动");
}

/// 不是我最后提交的：跟进不写磁盘，原因写进放回结果，横幅列出文件和补丁，按钮能放回。
#[test]
fn a_file_last_committed_by_someone_else_is_left_to_the_banner() {
    let fx = hybrid_with_colleague();
    let held = not_mine_case(&fx);

    let pending = pending_restores(&fx.wc_str()).unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].path, A);
    assert!(
        pending[0]
            .commits
            .iter()
            .any(|c| c.ends_with(" add @avatarclass")),
        "{pending:?}"
    );

    let out = restore_local_only(&fx.wc_str(), "op3", &[A.to_string()]).unwrap();

    assert_eq!(out.restored, vec![A.to_string()], "{out:?}");
    assert_eq!(read(&fx, A), format!("{CLASS}{held}"));
    assert!(
        pending_restores(&fx.wc_str()).unwrap().is_empty(),
        "放回后清单清空"
    );
}

/// 跟进之后磁盘上有了用户自己的编辑：横幅不再列它，下一次跟进把它从清单里修剪掉。
#[test]
fn an_edit_after_follow_drops_the_file_from_the_banner() {
    let fx = hybrid_with_colleague();
    let held = not_mine_case(&fx);
    write(&fx, A, &format!("{held}my edit\n"));

    // 横幅现场重验不过，不再列；清单文件里的条目还在（横幅只读、不修剪）
    assert!(pending_restores(&fx.wc_str()).unwrap().is_empty());
    assert_eq!(pending_list(&fx.wc_str()), vec![A.to_string()]);
}

/// 下一次真正完成的跟进把「再等也不会满足」的条目从清单里删掉，即使这次没有新的可放回文件
/// （review M1、M2）。
#[test]
fn the_next_completed_follow_prunes_entries_that_can_never_be_restored() {
    const C: &str = "Code/C.cs";
    let fx = hybrid_with_colleague();
    let held = not_mine_case(&fx);
    write(&fx, A, &format!("{held}my edit\n"));
    assert_eq!(pending_list(&fx.wc_str()), vec![A.to_string()], "前提");
    // 同事改了一个无关的文件：这次跟进能完成，但没有可放回的文件
    fx.colleague_commits(C, "c\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", C]);

    let rep = done(follow(&fx.wc_str(), "op4").unwrap());

    assert!(rep.restorable.is_empty(), "{rep:?}");
    assert!(
        pending_list(&fx.wc_str()).is_empty(),
        "磁盘有编辑的条目被修剪掉"
    );
    assert_eq!(read(&fx, A), format!("{held}my edit\n"), "编辑原样保留");
}

/// 暂时满足不了的（暂存了别的版本）留在清单里，修剪不删。
#[test]
fn pruning_keeps_entries_that_may_become_restorable_later() {
    const C: &str = "Code/C.cs";
    let fx = hybrid_with_colleague();
    not_mine_case(&fx);
    // 暂存一个别的版本：放回条件不满足，但取消暂存之后又满足
    let held = read(&fx, A);
    write(&fx, A, "staged\n");
    git(&fx, &["add", A]);
    write(&fx, A, &held);
    fx.colleague_commits(C, "c\n");
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", C]);

    done(follow(&fx.wc_str(), "op4").unwrap());

    assert_eq!(pending_list(&fx.wc_str()), vec![A.to_string()]);
}

/// 有意去掉补丁、还没提交时，`svn update` 带来同事对这个文件的改动（同 `svn_follow_auto.rs` 的
/// 同名场景）。返回跟进后的磁盘内容（同事的版本，没有补丁）。
fn not_mine_case(fx: &SvnFixture) -> String {
    i_commit(fx);
    write(fx, A, &base_text());
    tortoise_commit(fx, A);
    done(follow(&fx.wc_str(), "op0").unwrap());
    write(fx, A, &format!("{CLASS}{}", base_text()));
    commit_all(fx, L_MSG);
    // 提交 SVN 前去掉补丁
    write(fx, A, &base_text());
    // 改最后一行：补丁加在第一行前面，挨着改会重放冲突
    let theirs = base_text().replace("l6\n", "l6 colleague\n");
    fx.colleague_commits(A, &theirs);
    set_author(fx, youngest(fx), "colleague");
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", A]);
    assert_eq!(
        read(fx, A),
        theirs,
        "前提：update 带来同事的改动，补丁仍不在磁盘上"
    );

    let rep = done(follow(&fx.wc_str(), "op1").unwrap());

    assert_eq!(rep.restorable, vec![A.to_string()], "{rep:?}");
    let restore = rep.restore.expect("有放回结果");
    assert!(restore.restored.is_empty(), "{restore:?}");
    assert_eq!(restore.skipped.len(), 1, "{restore:?}");
    assert!(restore.skipped[0].reason.contains("不是你"), "{restore:?}");
    assert!(restore.commits.contains_key(A), "{restore:?}");
    assert_eq!(read(fx, A), theirs, "不自动写");
    theirs
}

fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

/// file:// 下作者都是系统用户，同事的 revision 要在 update 之前改 revprop（同 `svn_follow_auto.rs`）。
fn set_author(fx: &SvnFixture, rev: u64, who: &str) {
    let f = fx.path().join(format!("author-{rev}.txt"));
    std::fs::write(&f, who).unwrap();
    let out = Command::new("svnadmin")
        .args([
            "setrevprop",
            &fx.repo().to_string_lossy(),
            "-r",
            &rev.to_string(),
            "svn:author",
            &f.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
