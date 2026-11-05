//! 基线分支 `svn/base` 与移动它的命令的护栏（PR #49 Codex review #2 / #3 留下的两条）。
//!
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::commands::ops::{commit, create_branch, delete_branch, rename_branch, revert};
use app_lib::commands::svn::{hold_for_baseline_move, svn_rebuild_baseline, svn_sync_status};
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
fn an_at_brace_shorthand_does_not_get_around_it() {
    // 2026-10-07 实测（git 2.51.2）：`git branch -D` / `-m` 会展开 `@{-N}`、`@{u}`、
    // `<分支>@{upstream}`。一次更新会先切到 `svn/base` 再切回来，之后 `@{-1}` 就是基线，
    // `git branch -D @{-1}` 输出「Deleted branch svn/base」
    let fx = hybrid_with_colleague();
    let base = git(&fx, &["rev-parse", "svn/base"]);
    git(&fx, &["checkout", "-q", "svn/base"]);
    git(&fx, &["checkout", "-q", "work"]);
    git(&fx, &["branch", "--set-upstream-to=svn/base", "work"]);

    for name in ["@{-1}", "@{u}", "work@{upstream}"] {
        let err = delete_branch(fx.wc_str(), name.into()).unwrap_err();
        assert_eq!(
            err.code, "svn_baseline_branch_protected",
            "{name}：{}",
            err.message
        );
        let err = rename_branch(fx.wc_str(), name.into(), "moved".into()).unwrap_err();
        assert_eq!(
            err.code, "svn_baseline_branch_protected",
            "{name}：{}",
            err.message
        );
    }
    // 新名字一侧同样会被展开
    let err = rename_branch(fx.wc_str(), "work".into(), "@{-1}".into()).unwrap_err();
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

// —— 第 70 条：别的分支不能冒充基线，HEAD 在基线上时不能提交 ——

#[test]
fn a_branch_named_like_the_baseline_cannot_be_created() {
    // 基线丢了的时候新建一个叫 svn/base 的分支，就成了一条没经过体检的「基线」
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "-D", "svn/base"]);
    for name in ["svn/base", "SVN/Base", "refs/heads/svn/base"] {
        let err = create_branch(fx.wc_str(), name.into(), Some("work".into())).unwrap_err();
        assert_eq!(
            err.code, "svn_baseline_branch_protected",
            "{name}：{}",
            err.message
        );
    }
    assert!(run_git(&fx.wc_str(), &["rev-parse", "--verify", "svn/base"]).is_err());
}

#[test]
fn committing_or_reverting_while_head_is_on_the_baseline_is_refused() {
    let fx = hybrid_with_colleague();
    let base = git(&fx, &["rev-parse", "svn/base"]);
    git(&fx, &["checkout", "-q", "svn/base"]);
    std::fs::write(fx.wc().join("Code/Extra.cs"), "class Extra {}\n").unwrap();
    git(&fx, &["add", "Code/Extra.cs"]);

    for amend in [false, true] {
        let err = commit(fx.wc_str(), "on base".into(), amend).unwrap_err();
        assert_eq!(
            err.code, "svn_on_baseline_branch",
            "amend={amend}：{}",
            err.message
        );
    }
    let err = revert(fx.wc_str(), "HEAD".into()).unwrap_err();
    assert_eq!(err.code, "svn_on_baseline_branch", "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), base, "基线被移动了");
}

#[test]
fn committing_on_a_work_branch_is_unaffected() {
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/Extra.cs"), "class Extra {}\n").unwrap();
    git(&fx, &["add", "Code/Extra.cs"]);
    commit(fx.wc_str(), "on work".into(), false).unwrap();
}

#[test]
fn in_a_plain_git_repo_a_branch_called_svn_base_is_just_a_branch() {
    // 与 `isProtectedBaseline` 只在 vcs === 'svn' 时生效一致
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().to_string_lossy().to_string();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
        vec!["commit", "--allow-empty", "-m", "c1"],
    ] {
        run_git(&p, &args).unwrap();
    }
    create_branch(p.clone(), "svn/base".into(), None).unwrap();
    run_git(&p, &["checkout", "-q", "svn/base"]).unwrap();
    std::fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    run_git(&p, &["add", "a.txt"]).unwrap();
    commit(p.clone(), "c2".into(), false).unwrap_or_else(|e| panic!("{}", e.message));
    std::fs::write(dir.path().join("a.txt"), "b\n").unwrap();
    run_git(&p, &["add", "a.txt"]).unwrap();
    commit(p, "c3".into(), true).unwrap_or_else(|e| panic!("{}", e.message));
}

// —— 第 70 条：基线丢了之后「重建基线」从零录一条 ——

#[test]
fn rebuilding_after_the_baseline_was_deleted_records_a_new_one() {
    let fx = hybrid_with_colleague();
    let old = git(&fx, &["rev-parse", "svn/base"]);
    git(&fx, &["branch", "-D", "svn/base"]);

    let report = svn_rebuild_baseline(fx.wc_str()).unwrap_or_else(|e| panic!("{}", e.message));

    assert_eq!(report.old_baseline, "", "没有旧基线时报空串");
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), report.new_baseline);
    // 父提交接回 HEAD 历史里最近的那条基线，工作分支与新基线仍有共同祖先
    assert_eq!(git(&fx, &["rev-parse", "svn/base^"]), old);
    assert_eq!(git(&fx, &["merge-base", "work", "svn/base"]), old);
    // 失同步判定重新有了参照
    svn_sync_status(fx.wc_str()).unwrap_or_else(|e| panic!("{}", e.message));
}

#[test]
fn rebuilding_a_lost_baseline_from_a_branch_with_no_baseline_history_is_refused() {
    // 录成根 commit 的话，工作分支与基线永远没有共同祖先，跟进再也做不了（PR 115 review）。
    // 历史里只有别的 SVN 仓库导入的 git-svn commit 也不能当父提交
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "-D", "svn/base"]);
    git(&fx, &["checkout", "-q", "--orphan", "lonely"]);
    git(
        &fx,
        &[
            "commit",
            "-q",
            "-m",
            "imported\n\ngit-svn-id: file:///elsewhere/trunk@3 00000000-0000-0000-0000-000000000000",
        ],
    );

    let err = svn_rebuild_baseline(fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_recover", "{}", err.message);
    assert!(
        run_git(&fx.wc_str(), &["rev-parse", "--verify", "svn/base"]).is_err(),
        "拒绝了却建出了 svn/base"
    );
}

#[test]
fn rebuilding_with_the_baseline_present_still_moves_it_from_the_old_one() {
    let fx = hybrid_with_colleague();
    let old = git(&fx, &["rev-parse", "svn/base"]);
    let report = svn_rebuild_baseline(fx.wc_str()).unwrap_or_else(|e| panic!("{}", e.message));
    assert_eq!(report.old_baseline, old);
    assert_eq!(git(&fx, &["rev-parse", "svn/base^"]), old);
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
