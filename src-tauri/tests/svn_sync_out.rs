//! `sync_out` 编排（spec §8.2 D0–D10）。全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal;
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_out::{
    classify_commit_failure, commit_targets, run_sync_out, run_sync_out_probed, CommitFailure,
    Step, SyncOutExpect, SyncOutOutcome,
};
use app_lib::engine::svn_sync_out_check::{preview_sync_out, DeliverChange, SyncOutPreview};
use std::process::Command;
use svn_fixtures::{hybrid_whitelist_in_external, hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn svn_in(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("svn")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "svn {args:?}：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn repo_url(fx: &SvnFixture) -> String {
    svn_fixtures::file_url(&fx.repo())
}

/// 服务器上 r`rev` 的提交信息。走 `--xml`（恒为 UTF-8）：文本输出按系统代码页转码，
/// ACP 1252 的机器上中文会变成 `?`，拿它断言会把「存坏了」和「显示坏了」混在一起。
fn server_msg(fx: &SvnFixture, rev: u64) -> String {
    let xml = svn_in(
        fx.path(),
        &["log", "--xml", "-r", &rev.to_string(), &repo_url(fx)],
    );
    let start = xml.find("<msg>").map(|i| i + "<msg>".len()).unwrap_or(0);
    let end = xml.find("</msg>").unwrap_or(start);
    xml[start..end]
        .replace("\r\n", "\n")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// 服务器上某个文件在 HEAD 的内容。
fn server_cat(fx: &SvnFixture, rel: &str) -> String {
    svn_in(fx.path(), &["cat", &format!("{}/{rel}", repo_url(fx))])
}

fn write(fx: &SvnFixture, rel: &str, content: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn expect_from(p: &SyncOutPreview, message: &str) -> SyncOutExpect {
    SyncOutExpect {
        orig_branch: p.orig_branch.clone(),
        orig_head: p.orig_head.clone(),
        baseline_oid: p.baseline_oid.clone(),
        deliver_tree: p.deliver_tree.clone(),
        changes: p.changes.clone(),
        confirmed_outside: p.outside_changes.clone(),
        message: message.to_string(),
    }
}

/// 预览 → 「用户确认」→ 执行。
fn deliver(fx: &SvnFixture, message: &str) -> app_lib::error::Result<SyncOutOutcome> {
    let p = preview_sync_out(&fx.wc_str())?;
    run_sync_out(&fx.wc_str(), &expect_from(&p, message))
}

fn deliver_probed(
    fx: &SvnFixture,
    message: &str,
    probe: &dyn Fn(Step, &str),
) -> app_lib::error::Result<SyncOutOutcome> {
    let p = preview_sync_out(&fx.wc_str())?;
    run_sync_out_probed(&fx.wc_str(), &expect_from(&p, message), &probe)
}

/// 一次典型的交付内容：改 A、删 B、在两层新目录里加 New.cs。
fn typical_work(fx: &SvnFixture) {
    write(fx, "Code/A.cs", "a2\n");
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    write(fx, "Code/Deep/Nested/New.cs", "new\n");
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", "work"]);
}

/// 交付彻底收尾的通用断言（spec §8.2 D10、必测 #36）。
fn assert_settled(fx: &SvnFixture) {
    assert!(
        svn_journal::read(&fx.wc_str()).unwrap().is_none(),
        "journal 没删"
    );
    assert!(!svn_lock::is_held(&fx.wc_str()), "锁没放");
    assert_eq!(
        git(fx, &["for-each-ref", "refs/mygitclient/deliver/"]),
        "",
        "交付 ref 没删"
    );
    assert_eq!(git(fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(fx, &["status", "--porcelain"]), "");
    assert!(
        !fx.wc().join(".git/mygitclient-commit-msg.txt").exists(),
        "提交信息文件没删"
    );
}

#[test]
fn a_plain_delivery_goes_all_the_way_to_d10() {
    // 必测 #36（无 Local-Only，走快进）+ #18 + #62 前半
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let before = git(&fx, &["rev-parse", "HEAD"]);

    let out = deliver(&fx, "修复跳跃").unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::Delivered {
            rev: 2,
            branch: "work".into(),
            local_only_left: 0
        }
    );

    // 服务器：一个 revision，五条路径，提交信息原样
    let log = svn_in(fx.path(), &["log", "-v", "-r", "2", &repo_url(&fx)]);
    for line in [
        "M /Code/A.cs",
        "D /Code/B.cs",
        "A /Code/Deep",
        "A /Code/Deep/Nested",
        "A /Code/Deep/Nested/New.cs",
    ] {
        assert!(log.contains(line), "缺 {line}：{log}");
    }
    assert_eq!(server_msg(&fx, 2), "修复跳跃");
    assert_eq!(server_cat(&fx, "Code/A.cs"), "a2\n");

    // 基线推进到新内容，trailer 记 r2；工作分支快进到它（spec §8.2.2）
    let base_msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    assert!(base_msg.contains("@2 "), "{base_msg}");
    assert_eq!(
        git(&fx, &["rev-parse", "work"]),
        git(&fx, &["rev-parse", "svn/base"])
    );
    assert_eq!(git(&fx, &["show", "svn/base:Code/A.cs"]), "a2");
    assert!(git(&fx, &["merge-base", "--is-ancestor", &before, "svn/base"]).is_empty());

    assert_eq!(svn_in(&fx.wc(), &["status", "Code"]), "");
    assert_settled(&fx);
}

#[test]
fn no_false_out_of_sync_after_a_delivery() {
    // 必测 #26：根节点停在旧 revision 是常态，按白名单节点判定不该报
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    deliver(&fx, "work").unwrap();
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
}

#[test]
fn delivering_again_right_after_a_delivery_has_nothing_to_deliver() {
    // Review Focus 5
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    deliver(&fx, "work").unwrap();
    assert_eq!(
        preview_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_nothing_to_deliver"
    );
}

#[test]
fn a_stray_add_in_the_same_new_dir_is_not_committed_along() {
    // 必测 #62 后半：--depth=empty 只提交 target 自身（spec §8.2.6）
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join("Code/Deep2/Nested")).unwrap();
    write(&fx, "Code/Deep2/Stray.txt", "stray\n"); // W-path 内、W-file 外，git 不管
    svn_in(&fx.wc(), &["add", "--parents", "Code/Deep2/Stray.txt"]);
    write(&fx, "Code/Deep2/Nested/New.cs", "new\n");
    git(&fx, &["add", "Code/Deep2/Nested/New.cs"]);
    git(&fx, &["commit", "-qm", "nested"]);

    deliver(&fx, "nested").unwrap();
    let log = svn_in(fx.path(), &["log", "-v", "-r", "2", &repo_url(&fx)]);
    assert!(log.contains("A /Code/Deep2/Nested/New.cs"), "{log}");
    assert!(!log.contains("Stray.txt"), "{log}");
    assert!(svn_in(&fx.wc(), &["status", "Code/Deep2/Stray.txt"]).starts_with('A'));
}

#[test]
fn a_message_that_looks_like_a_path_or_an_option_is_committed_verbatim() {
    // Review Focus 1 / 计划 B14
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "w1"]);
    deliver(&fx, "Code/A.cs").unwrap();
    write(&fx, "Code/A.cs", "a3\n");
    git(&fx, &["commit", "-qam", "w2"]);
    deliver(&fx, "-m 第一行\n第二行").unwrap();
    assert_eq!(server_msg(&fx, 2), "Code/A.cs");
    // 中文必须原样：命令行参数在 ACP 1252 的机器上会被 svn.exe 的 CRT 转成 `?`
    assert_eq!(server_msg(&fx, 3), "-m 第一行\n第二行");
}

#[test]
fn a_colleague_commit_between_d5_and_d6_ends_up_in_the_new_baseline() {
    // Review Focus 4：新基线 = 服务器最新内容，不只是我的提交
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let out = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::Committed {
            fx.colleague_commits("Code/C.cs", "c\n");
        }
    })
    .unwrap();
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 3, .. }),
        "{out:?}"
    );
    assert_eq!(git(&fx, &["show", "svn/base:Code/C.cs"]), "c");
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/C.cs")).unwrap(),
        "c\n"
    );
    assert_eq!(
        git(&fx, &["rev-parse", "work"]),
        git(&fx, &["rev-parse", "svn/base"])
    );
    assert_settled(&fx);
}

#[test]
fn the_baseline_is_published_from_the_very_tree_that_was_checked() {
    // 必测 #73 的快进一半：D7a 核对过的 TREE 就是新基线的 tree，索引对齐不改磁盘
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let captured = std::cell::RefCell::new(None::<String>);
    deliver_probed(&fx, "work", &|s, repo| {
        if s == Step::Captured {
            let j = svn_journal::read(repo).unwrap().unwrap();
            *captured.borrow_mut() = j.capture_tree.clone();
        }
    })
    .unwrap();
    let tree = captured
        .into_inner()
        .expect("Captured 时 journal 里应有 captureTree");
    assert_eq!(git(&fx, &["rev-parse", "svn/base^{tree}"]), tree);
    assert_eq!(
        git(&fx, &["diff", "--cached", "--name-only"]),
        "",
        "索引与 HEAD 一致"
    );
}

#[test]
fn art_outside_the_whitelist_is_neither_updated_nor_merged() {
    // 必测 #15 / #11：D6 只更新白名单路径，美术文件的本地修改原样、不产生 .mine
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd LOCAL\n").unwrap();
    typical_work(&fx);
    deliver_probed(&fx, "work", &|s, _| {
        if s == Step::Committed {
            fx.colleague_commits("Art/x.psd", "psd colleague\n");
        }
    })
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Art/x.psd")).unwrap(),
        "psd LOCAL\n"
    );
    let leftovers: Vec<_> = std::fs::read_dir(fx.wc().join("Art"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers, vec!["x.psd".to_string()], "{leftovers:?}");
}

#[test]
fn a_user_branch_named_deliver_does_not_get_in_the_way() {
    // 必测 #25 的全流程一半
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "deliver"]);
    typical_work(&fx);
    deliver(&fx, "work").unwrap();
    assert!(git(&fx, &["rev-parse", "--verify", "deliver"]).len() == 40);
}

#[test]
fn delivery_works_when_the_whitelist_is_an_external() {
    // 必测 #49 的交付一半：记账取 code 那个 WC
    let fx = hybrid_whitelist_in_external();
    write(&fx, "code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    let out = deliver(&fx, "in external").unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { .. }), "{out:?}");
    assert_eq!(server_cat(&fx, "code-trunk/A.cs"), "a2\n");
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
}

#[test]
fn a_held_lock_refuses_the_delivery_and_leaves_no_journal() {
    // 必测 #41 的交付一半（跨进程那半由 2b 的 svn_lock 测试覆盖）
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let _other = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_in_progress", "{}", err.message);
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

#[test]
fn an_empty_message_is_refused_before_anything() {
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "  \n ")).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_message");
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

#[test]
fn an_empty_commit_target_list_is_refused() {
    // 必测 #1：空 target 的 svn commit 会提交整个工作副本，且不可撤销（spec §8.3）
    let err = commit_targets(&[], &[]).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets");
    let err = commit_targets(&["Code/Deep".into()], &[]).unwrap_err();
    assert_eq!(
        err.code, "sync_out_empty_targets",
        "只有父目录、没有文件也不行"
    );
    let ok = commit_targets(
        &["Code/Deep".into()],
        &[DeliverChange {
            status: "A".into(),
            path: "Code/Deep/N.cs".into(),
        }],
    )
    .unwrap();
    assert_eq!(
        ok,
        vec!["Code/Deep@".to_string(), "Code/Deep/N.cs@".to_string()]
    );
}

#[test]
fn commit_failures_are_rolled_back_only_when_the_server_surely_did_nothing() {
    // 计划 B3，输入是开工前实测 M2 的原文
    let ood = "svn: E155011: Commit failed (details follow):\nsvn: E155011: File 'x' is out of date\nsvn: E160028: File '/Code/Shared.cs' is out of date\n";
    let gone = "svn: E155011: Commit failed (details follow):\nsvn: E160013: File not found: transaction '4-5', path '/Code/A.cs'\n";
    let hook = "svn: E165001: Commit failed (details follow):\nsvn: E165001: Commit blocked by pre-commit hook (exit code 1) with output:\nrejected\n";
    let unreachable = "svn: E170013: Commit failed (details follow):\nsvn: E170013: Unable to connect to a repository at URL 'file:///x'\nsvn: E180001: Unable to open repository 'file:///x'\n";
    assert_eq!(classify_commit_failure(ood, ""), CommitFailure::Rejected);
    assert_eq!(classify_commit_failure(gone, ""), CommitFailure::Rejected);
    assert_eq!(classify_commit_failure(hook, ""), CommitFailure::Rejected);
    assert_eq!(
        classify_commit_failure(unreachable, ""),
        CommitFailure::Unknown
    );
    assert_eq!(
        classify_commit_failure("connection reset", ""),
        CommitFailure::Unknown
    );
    assert_eq!(
        classify_commit_failure(ood, "Committed revision 9.\n"),
        CommitFailure::Unknown,
        "stdout 说提交了就绝不回滚"
    );
}
