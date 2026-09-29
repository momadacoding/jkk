//! 基线分支 `svn/base` 与移动它的命令的护栏（PR #49 Codex review #2 / #3 留下的两条）。
//!
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::commands::ops::{delete_branch, rename_branch};
use app_lib::commands::svn::hold_for_baseline_move;
use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal::{self, Flow, Journal, Phase};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

// —— 基线分支不能被删、不能被改名 ——

#[test]
fn the_baseline_branch_cannot_be_deleted() {
    // 删掉之后失同步检测、体检、重建全部失去参照；未收尾的同步也就继续不了、放弃不了。
    // 界面的「删除」菜单项已经置灰，后端不能只靠界面
    let fx = hybrid_with_colleague();
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let err = delete_branch(fx.wc_str(), "svn/base".into()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_branch_protected", "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base);
}

#[test]
fn the_baseline_branch_cannot_be_renamed_away_or_onto() {
    let fx = hybrid_with_colleague();
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let err = rename_branch(fx.wc_str(), "svn/base".into(), "old-base".into()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_branch_protected", "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base);

    // 反方向：把别的分支改名成 `svn/base`。已有 `svn/base` 时 git 自己也会拒，
    // 但报的是一句英文；基线不在的时候就真被顶替了
    let err = rename_branch(fx.wc_str(), "work".into(), "svn/base".into()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_branch_protected", "{}", err.message);
}

#[test]
fn a_differently_cased_name_does_not_get_around_it() {
    // 2026-09-30 在临时仓库实测（Windows / NTFS，ref 为散装文件）：
    // `git branch -D SVN/BASE` 输出「Deleted branch SVN/BASE」，删掉的就是 `svn/base`；
    // `git branch -m SVN/BASE x` 同理把它改了名。比较必须不区分大小写
    let fx = hybrid_with_colleague();
    let base = git(&fx, &["rev-parse", "svn/base"]);
    let err = delete_branch(fx.wc_str(), "SVN/BASE".into()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_branch_protected", "{}", err.message);
    let err = rename_branch(fx.wc_str(), "Svn/Base".into(), "x".into()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_branch_protected", "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base);
}

#[test]
fn other_branches_are_still_renamed_and_deleted() {
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "tmp", "work"]);
    rename_branch(fx.wc_str(), "tmp".into(), "tmp2".into()).unwrap();
    delete_branch(fx.wc_str(), "tmp2".into()).unwrap();
    // 名字里带 `svn/base` 的普通分支不受影响（精确匹配，同 `capabilities.ts::isBaselineBranch`）
    git(&fx, &["branch", "svn/base-backup", "work"]);
    delete_branch(fx.wc_str(), "svn/base-backup".into()).unwrap();
}

// —— 移动基线的命令全程持同步锁 ——

fn expect(fx: &SvnFixture) -> SyncInExpect {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    SyncInExpect {
        orig_branch: p.orig_branch,
        orig_head: p.orig_head,
        baseline_oid: p.baseline_oid,
        confirmed_outside: p.outside_changes,
    }
}

#[test]
fn a_sync_cannot_start_while_the_baseline_is_being_moved() {
    // Codex review #3：重建 / 行尾迁移原先只在开头看一眼锁，做的时候不持锁。
    // 期间开始的一次同步会收起 WIP、写 journal，然后被重建移走的基线挡在半路
    let fx = hybrid_with_colleague();
    let e = expect(&fx);
    let head = git(&fx, &["rev-parse", "HEAD"]);

    let lock = hold_for_baseline_move(&fx.wc_str()).unwrap();
    assert!(svn_lock::is_held(&fx.wc_str()));
    let err = run_sync_in(&fx.wc_str(), &e).unwrap_err();
    assert_eq!(err.code, "sync_in_progress", "{}", err.message);
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);

    drop(lock);
    assert!(!svn_lock::is_held(&fx.wc_str()), "用完就放");
}

#[test]
fn moving_the_baseline_is_refused_while_a_sync_holds_the_lock_or_is_unfinished() {
    let fx = hybrid_with_colleague();
    {
        let _other = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
        let err = hold_for_baseline_move(&fx.wc_str()).unwrap_err();
        assert_eq!(err.code, "sync_in_progress", "{}", err.message);
    }
    // 冲突出口放开了锁、留着 journal
    let j = Journal {
        opid: "op-left".into(),
        flow: Flow::SyncIn,
        phase: Phase::Rebase,
        started_at: 1,
        ..Default::default()
    };
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    let err = hold_for_baseline_move(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_in_unfinished", "{}", err.message);
    assert!(!svn_lock::is_held(&fx.wc_str()), "被拒时不留着锁");
}
