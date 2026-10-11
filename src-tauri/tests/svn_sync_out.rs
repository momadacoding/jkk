//! `sync_out` 编排（spec §8.2 D0–D10）。全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal;
use app_lib::engine::svn_journal::Phase;
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_out::{
    abort_sync_out, classify_commit_failure, commit_targets, continue_sync_out, run_sync_out,
    run_sync_out_probed, CommitFailure, Step, SyncOutExpect, SyncOutOutcome,
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
    // rev 是**我的**提交（r2），不是 D6 更新到的 r3——用户拿它去服务器 log 里找自己的提交
    // （里程碑 review）。新基线照样是 r3，下面那几行断言它
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }),
        "{out:?}"
    );
    assert!(
        git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@3 "),
        "新基线记的是 D6 之后的 r3"
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

// —— Task 6：Local-Only 全流程 ——

fn thirty_lines(fx: &SvnFixture) {
    let body: String = (1..=30).map(|n| format!("{n}\n")).collect();
    std::fs::write(fx.wc().join("Code/A.cs"), body).unwrap();
    git(fx, &["commit", "-qam", "thirty lines"]);
}

fn edit_line(fx: &SvnFixture, line: usize, to: &str, message: &str) {
    let p = fx.wc().join("Code/A.cs");
    let text = std::fs::read_to_string(&p).unwrap();
    let out: String = text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i + 1 == line {
                format!("{to}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    std::fs::write(&p, out).unwrap();
    git(fx, &["commit", "-qam", message]);
}

fn line(text: &str, n: usize) -> &str {
    text.lines().nth(n - 1).unwrap()
}

#[test]
fn a_local_only_patch_in_the_middle_is_never_delivered_and_survives() {
    // 必测 #19（T1 + T3）+ #36 的 rebase 一半：同一文件里既有补丁又有待交付改动
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    edit_line(&fx, 20, "twenty", "C2");
    let orig_head = git(&fx, &["rev-parse", "HEAD"]);

    let out = deliver(&fx, "jump fix").unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::Delivered {
            rev: 2,
            branch: "work".into(),
            local_only_left: 1
        }
    );

    let server = server_cat(&fx, "Code/A.cs");
    assert_eq!(
        (line(&server, 1), line(&server, 10), line(&server, 20)),
        ("one", "10", "twenty")
    );
    let base = git(&fx, &["show", "svn/base:Code/A.cs"]);
    assert_eq!(line(&base, 10), "10", "基线里是干净原文");
    let disk = std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap();
    assert_eq!(line(&disk, 10), "ten-LOCAL", "磁盘上补丁还在");
    assert_eq!(line(&disk, 20), "twenty");

    let left = git(&fx, &["log", "--format=%s", "svn/base..work"]);
    assert_eq!(left, "mute log", "svn/base..work 只剩那个补丁");
    assert_ne!(
        git(&fx, &["rev-parse", "work"]),
        orig_head,
        "origHead 变了（spec §8.2.4 末）"
    );
    assert_settled(&fx);
}

#[test]
fn a_local_only_suffix_takes_the_fast_path_and_is_rebased_back_on_top() {
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    let out = deliver(&fx, "c1").unwrap();
    assert!(
        matches!(
            out,
            SyncOutOutcome::Delivered {
                local_only_left: 1,
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(line(&server_cat(&fx, "Code/A.cs"), 10), "10");
    assert_eq!(
        git(&fx, &["rev-parse", "work~1"]),
        git(&fx, &["rev-parse", "svn/base"])
    );
    assert_settled(&fx);
}

#[test]
fn an_overlapping_local_only_aborts_before_anything_is_touched() {
    // 必测 #20（T2）：SVN 侧一个字节没动、没有交付 ref、主工作副本磁盘全程未变
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    edit_line(&fx, 11, "eleven", "fix jump");
    let disk = std::fs::read(fx.wc().join("Code/A.cs")).unwrap();
    let head = git(&fx, &["rev-parse", "HEAD"]);

    assert_eq!(
        preview_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_local_only_conflict"
    );
    // 执行入口自己也拦得住（锁内复核重算 D0.5），用一份手写的 expect 直接调
    let forged = SyncOutExpect {
        orig_branch: "work".into(),
        orig_head: head.clone(),
        baseline_oid: git(&fx, &["rev-parse", "svn/base"]),
        deliver_tree: "0".repeat(40),
        changes: vec![DeliverChange {
            status: "M".into(),
            path: "Code/A.cs".into(),
        }],
        confirmed_outside: vec![],
        message: "x".into(),
    };
    assert_eq!(
        run_sync_out(&fx.wc_str(), &forged).unwrap_err().code,
        "sync_out_local_only_conflict"
    );
    assert_eq!(std::fs::read(fx.wc().join("Code/A.cs")).unwrap(), disk);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(svn_in(fx.path(), &["log", "-q", &repo_url(&fx)]).contains("r1 "));
    assert!(!svn_in(fx.path(), &["log", "-q", &repo_url(&fx)]).contains("r2 "));
}

#[test]
fn a_merge_inside_the_delivery_is_kept_and_the_local_patch_is_replayed_on_top() {
    // 必测 #72②：merge 已在交付树内，其后只有连续单父的 Local-Only，独有文件保留
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    write(&fx, "Code/Side.cs", "side\n");
    git(&fx, &["add", "Code/Side.cs"]);
    git(&fx, &["commit", "-qm", "side"]);
    git(&fx, &["checkout", "-q", "work"]);
    git(&fx, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");

    let out = deliver(&fx, "with merge").unwrap();
    assert!(
        matches!(
            out,
            SyncOutOutcome::Delivered {
                local_only_left: 1,
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(server_cat(&fx, "Code/Side.cs"), "side\n");
    assert_eq!(git(&fx, &["show", "svn/base:Code/Side.cs"]), "side");
    assert!(fx.wc().join("Code/Side.cs").exists());
    assert_eq!(
        line(
            &std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
            10
        ),
        "ten-LOCAL"
    );
}

#[test]
fn a_local_patch_that_clashes_with_a_colleague_stops_in_a_resolvable_rebase() {
    // 必测 #69 的 D8d 一半（停下的状态）；继续的一半在 Task 8
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    let out = deliver_probed(&fx, "c1", &|s, _| {
        if s == Step::Committed {
            // 同事在我们 D5 之后改了第 10 行：D6 拉下来，D8d 重放补丁时冲突
            let theirs: String = (1..=30)
                .map(|n| match n {
                    1 => "one\n".to_string(),
                    10 => "ten-colleague\n".to_string(),
                    _ => format!("{n}\n"),
                })
                .collect();
            fx.colleague_commits("Code/A.cs", &theirs);
        }
    })
    .unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::RebaseConflict {
            rev: 2, // 我的提交；同事的是 r3

            files: vec!["Code/A.cs".into()]
        }
    );
    let j = svn_journal::read(&fx.wc_str())
        .unwrap()
        .expect("journal 保留");
    assert_eq!(
        j.phase,
        app_lib::engine::svn_journal::Phase::RestoringBranch
    );
    assert!(
        !svn_lock::is_held(&fx.wc_str()),
        "冲突出口放锁，用户要编辑、暂存文件"
    );
    assert!(
        git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@3 "),
        "基线已推进"
    );
    assert!(server_cat(&fx, "Code/A.cs").contains("ten-colleague"));
}

// —— Task 7：早退与 D5 失败 ——

/// 白名单当前的 svn 状态文本，用来断言「SVN 侧回到了交付前」。
fn svn_status(fx: &SvnFixture) -> String {
    svn_in(&fx.wc(), &["status", "Code"])
}

fn server_head(fx: &SvnFixture) -> String {
    svn_in(
        fx.path(),
        &["info", "--show-item", "revision", &repo_url(fx)],
    )
    .trim()
    .to_string()
}

/// 早退之后的通用断言：回到原分支原位置、没有交付 ref、没有 journal、锁已放、SVN 原样。
fn assert_untouched(fx: &SvnFixture, head: &str, status: &str, server: &str) {
    assert_eq!(git(fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(fx, &["for-each-ref", "refs/mygitclient/deliver/"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert_eq!(svn_status(fx), status, "svn 登记没回到交付前");
    assert_eq!(server_head(fx), server, "服务器上多了 revision");
    assert_eq!(git(fx, &["status", "--porcelain"]), "");
}

#[test]
fn a_commit_made_while_the_dialog_was_open_is_caught_before_anything_moves() {
    // Review Focus 2
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    write(&fx, "Code/A.cs", "a3\n");
    git(&fx, &["commit", "-qam", "one more"]);
    let (head, status, server) = (
        git(&fx, &["rev-parse", "HEAD"]),
        svn_status(&fx),
        server_head(&fx),
    );
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_out_state_changed", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn a_file_git_does_not_manage_changed_after_the_preview_needs_reconfirmation() {
    // 计划 B1：用户确认时没看到的白名单外修改，不能被执行时悄悄带进 D6
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/x.psd", "psd1\n");
    let p = app_lib::engine::svn_sync_check::preview_sync_in(&fx.wc_str()).unwrap();
    app_lib::engine::svn_sync_in::run_sync_in(
        &fx.wc_str(),
        &app_lib::engine::svn_sync_in::SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert!(p.outside_changes.is_empty());
    std::fs::write(fx.wc().join("Code/x.psd"), "psd LOCAL\n").unwrap();
    let (head, status, server) = (
        git(&fx, &["rev-parse", "HEAD"]),
        svn_status(&fx),
        server_head(&fx),
    );
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_out_needs_reconfirm", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn the_server_moving_after_the_preview_is_caught_at_d35() {
    // spec §8.4 / §13「D3.5 复查失败」：保留提交信息由前端负责（Task 11）
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    fx.colleague_commits("Art/x.psd", "psd2\n");
    let (head, status, server) = (
        git(&fx, &["rev-parse", "HEAD"]),
        svn_status(&fx),
        server_head(&fx),
    );
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_out_server_ahead", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn switching_back_in_another_tool_after_confirming_is_caught_at_d49() {
    // 必测 #30（spec §8.2.5）
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let (head, status, server) = (
        git(&fx, &["rev-parse", "HEAD"]),
        svn_status(&fx),
        server_head(&fx),
    );
    let err = deliver_probed(&fx, "x", &|s, repo| {
        if s == Step::Staged {
            run_git(repo, &["checkout", "-q", "work"]).unwrap(); // 用户在 Rider 里切了回去
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "sync_out_disk_changed", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn an_out_of_date_commit_is_rolled_back_precisely() {
    // 必测 #3 + #29 的全流程版：交付含新增（两层新目录）+ 删除 + 修改，同事抢先改了 A.cs
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let (head, status) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx));
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::BeforeCommit {
            fx.colleague_commits("Code/A.cs", "colleague\n");
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "sync_out_commit_rejected", "{}", err.message);
    assert!(
        err.message.contains("E155011"),
        "原样展示 svn stderr：{}",
        err.message
    );
    assert_untouched(&fx, &head, &status, "2"); // r2 是同事的
    assert_eq!(server_msg(&fx, 2), "colleague Code/A.cs");
}

#[test]
fn a_hook_rejection_is_rolled_back_and_not_retried() {
    // spec §8.6 表「服务端钩子拒绝」/ §8.7 绝不自动重试
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    install_rejecting_pre_commit_hook(&fx);
    let (head, status, server) = (
        git(&fx, &["rev-parse", "HEAD"]),
        svn_status(&fx),
        server_head(&fx),
    );
    let err = deliver(&fx, "mine").unwrap_err();
    assert_eq!(err.code, "sync_out_commit_rejected", "{}", err.message);
    assert!(err.message.contains("rejected by hook"), "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

fn install_rejecting_pre_commit_hook(fx: &SvnFixture) {
    let hooks = fx.repo().join("hooks");
    if cfg!(windows) {
        std::fs::write(
            hooks.join("pre-commit.bat"),
            "@echo off\r\necho rejected by hook 1>&2\r\nexit 1\r\n",
        )
        .unwrap();
    } else {
        let p = hooks.join("pre-commit");
        std::fs::write(&p, "#!/bin/sh\necho rejected by hook >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[test]
fn an_unknown_commit_result_leaves_everything_where_it_is() {
    // 计划 B3 / B4：仓库在 D5 时不可达（E170013 + E180001），既不回滚也不推进
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let away = fx.path().join("repo-away");
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::BeforeCommit {
            std::fs::rename(fx.repo(), &away).unwrap();
        }
    })
    .unwrap_err();
    std::fs::rename(&away, fx.repo()).unwrap();
    assert_eq!(err.code, "sync_out_commit_unknown", "{}", err.message);
    assert!(err.message.contains("mygitclient:opid="), "{}", err.message);
    // 基线 r1：提交若落地，第一个可能的位置是 r2，不能说成「r2 之后」（最终 review 第 5 条）
    assert!(err.message.contains("从 r2 起"), "{}", err.message);
    let j = svn_journal::read(&fx.wc_str())
        .unwrap()
        .expect("journal 保留");
    assert_eq!(j.phase, app_lib::engine::svn_journal::Phase::Committing);
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert!(
        svn_status(&fx).contains("A       Code\\Deep")
            || svn_status(&fx).contains("A       Code/Deep")
    );
    assert_eq!(server_head(&fx), "1");
}

// —— Task 8：继续 / 放弃 ——

/// 在 `step` 处模拟崩溃：panic 展开时锁的 Drop 照常释放（等同于进程死后操作系统放锁），
/// journal 停在那一步写下的 phase。
fn crash_at(fx: &SvnFixture, step: Step) {
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let e = expect_from(&p, "crash");
    let wc = fx.wc_str();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_sync_out_probed(&wc, &e, &move |s: Step, _: &str| {
            if s == step {
                panic!("模拟崩溃于 {s:?}");
            }
        })
    }));
    assert!(r.is_err(), "应在 {step:?} 崩溃");
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

fn phase(fx: &SvnFixture) -> Phase {
    svn_journal::read(&fx.wc_str())
        .unwrap()
        .expect("journal 应保留")
        .phase
}

#[test]
fn every_crash_before_the_commit_is_cleaned_up_by_either_button() {
    // 必测 #16 的 D5 之前部分 + 计划 B8：继续与放弃都走同一条清理路径
    for (step, want, use_continue) in [
        (Step::Built, Phase::DeliverBuilt, false),
        (Step::CheckedOut, Phase::CheckedOut, true),
        (Step::Staged, Phase::SvnStaged, false),
        (Step::BeforeCommit, Phase::SvnStaged, true),
    ] {
        let fx = hybrid_with_colleague();
        typical_work(&fx);
        let (head, status, server) = (
            git(&fx, &["rev-parse", "HEAD"]),
            svn_status(&fx),
            server_head(&fx),
        );
        crash_at(&fx, step);
        assert_eq!(phase(&fx), want, "{step:?}");
        let out = if use_continue {
            continue_sync_out(&fx.wc_str())
        } else {
            abort_sync_out(&fx.wc_str())
        }
        .unwrap_or_else(|e| panic!("{step:?}：{}", e.message));
        assert_eq!(
            out,
            SyncOutOutcome::Abandoned { committed: false },
            "{step:?}"
        );
        assert_untouched(&fx, &head, &status, &server);
    }
}

#[test]
fn a_temp_worktree_named_in_a_foreign_journal_is_left_alone() {
    // 里程碑 review 第 9 条的回归：本期从不写 tempWorktree；字段若出现（手改 / 外来的
    // journal），收尾不得对那里 `worktree remove --force`——那会连未提交的改动一起删掉
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Built);
    let wt = fx.path().join("someone-elses-worktree");
    let wt_str = wt.to_string_lossy().to_string();
    git(&fx, &["worktree", "add", "-q", "-b", "elsewhere", &wt_str]);
    std::fs::write(wt.join("unsaved.txt"), "work in progress\n").unwrap();
    let mut j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    j.temp_worktree = Some(wt_str.clone());
    svn_journal::write(&fx.wc_str(), &j).unwrap();

    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap(),
        SyncOutOutcome::Abandoned { committed: false }
    );
    assert!(
        wt.join("unsaved.txt").exists(),
        "别人的 worktree 连同未提交改动必须还在"
    );
    assert!(git(&fx, &["worktree", "list"]).contains("someone-elses-worktree"));
}

#[test]
fn a_commit_in_an_unknown_state_cannot_be_continued_or_abandoned_yet() {
    // 计划 B4：查证在 3b
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let away = fx.path().join("repo-away");
    let _ = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::BeforeCommit {
            std::fs::rename(fx.repo(), &away).unwrap();
        }
    });
    std::fs::rename(&away, fx.repo()).unwrap();
    let before = svn_journal::read(&fx.wc_str()).unwrap();
    assert_eq!(
        continue_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_commit_unknown"
    );
    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_commit_unknown"
    );
    assert_eq!(
        svn_journal::read(&fx.wc_str()).unwrap(),
        before,
        "journal 原样"
    );
}

#[test]
fn every_crash_after_the_commit_is_finished_by_continue_without_committing_again() {
    // 必测 #16 的 D5 之后部分 + #73 的「只续对应阶段、不重做 SVN commit」
    for (step, want) in [
        (Step::Committed, Phase::PostCommit),
        (Step::Updated, Phase::PostCommit),
        (Step::Captured, Phase::PublishingBaseline),
        (Step::Published, Phase::RestoringBranch),
    ] {
        let fx = hybrid_with_colleague();
        typical_work(&fx);
        crash_at(&fx, step);
        assert_eq!(phase(&fx), want, "{step:?}");
        let out =
            continue_sync_out(&fx.wc_str()).unwrap_or_else(|e| panic!("{step:?}：{}", e.message));
        assert_eq!(
            out,
            SyncOutOutcome::Delivered {
                rev: 2,
                branch: "work".into(),
                local_only_left: 0
            },
            "{step:?}"
        );
        assert_eq!(server_head(&fx), "2", "{step:?}：服务器上只能有一次提交");
        assert_settled(&fx);
    }
}

#[test]
fn abandoning_after_the_commit_leaves_a_state_that_update_takes_over() {
    // 计划 B7：放弃 = 不再替用户做后面的事；先 D6 拉齐，避免留下 mixed-revision
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let old_base = git(&fx, &["rev-parse", "svn/base"]);
    crash_at(&fx, Step::Committed);
    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap(),
        SyncOutOutcome::Abandoned { committed: true }
    );
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(
        git(&fx, &["rev-parse", "svn/base"]),
        old_base,
        "基线没发布就不发布"
    );
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/deliver/"]), "");
    match check_sync(&fx.wc_str()).unwrap().verdict {
        SyncVerdict::Diverged {
            node_rev,
            baseline_rev,
        } => assert_eq!((node_rev, baseline_rev), (2, 1)),
        v => panic!("应是整体前进，实际 {v:?}"),
    }
    // 「更新」接管（2c 的 A16）：已交付的 commit 在 rebase 时变空、被丢掉
    let p = app_lib::engine::svn_sync_check::preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.catch_up_rev, Some(2));
    app_lib::engine::svn_sync_in::run_sync_in(
        &fx.wc_str(),
        &app_lib::engine::svn_sync_in::SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
    assert_eq!(
        git(&fx, &["rev-parse", "work"]),
        git(&fx, &["rev-parse", "svn/base"])
    );
}

#[test]
fn a_baseline_moved_by_someone_else_is_not_overwritten_on_continue() {
    // 必测 #73：注入旧基线 OID 改变，CAS 拒绝覆盖
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Captured);
    git(&fx, &["update-ref", "refs/heads/svn/base", "work"]); // 外部工具挪了基线
    let err = continue_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_moved", "{}", err.message);
    assert_eq!(phase(&fx), Phase::PublishingBaseline);
    assert_eq!(server_head(&fx), "2");
}

#[test]
fn a_d8d_conflict_is_continued_after_the_user_resolves_it() {
    // 必测 #69 的继续一半
    let fx = hybrid_with_colleague();
    make_d8d_clash(&fx);
    let err = continue_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(
        err.code,
        "sync_out_unresolved", // review 第 6 条：不借用「更新」流程的码
        "没解决不能继续"
    );
    // 最终 review 第 4 条：报错里点名的按钮必须是这个 phase 横幅上真有的
    assert!(
        err.message.contains("「我已解决，继续」"),
        "{}",
        err.message
    );
    let resolved: String = (1..=30)
        .map(|n| match n {
            1 => "one\n".to_string(),
            10 => "ten-LOCAL\n".to_string(),
            _ => format!("{n}\n"),
        })
        .collect();
    std::fs::write(fx.wc().join("Code/A.cs"), resolved).unwrap();
    git(&fx, &["add", "Code/A.cs"]);
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(
        out,
        // 我的提交是 r2，同事的是 r3（里程碑 review 第 5 条：报自己的 revision）
        SyncOutOutcome::Delivered {
            rev: 2,
            branch: "work".into(),
            local_only_left: 1
        }
    );
    assert_settled(&fx);
}

#[test]
fn a_d8d_conflict_can_be_abandoned() {
    let fx = hybrid_with_colleague();
    let head = make_d8d_clash(&fx);
    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap(),
        SyncOutOutcome::Abandoned { committed: true }
    );
    assert_eq!(
        git(&fx, &["rev-parse", "HEAD"]),
        head,
        "rebase 已中止，分支回到交付前的位置"
    );
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert!(
        git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@3 "),
        "基线留在新位置"
    );
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

/// 造出 D8d 的 rebase 冲突（与 Task 6 的同一场景），返回交付前的 HEAD。
fn make_d8d_clash(fx: &SvnFixture) -> String {
    thirty_lines(fx);
    edit_line(fx, 1, "one", "C1");
    edit_line(fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    let head = git(fx, &["rev-parse", "HEAD"]);
    let out = deliver_probed(fx, "c1", &|s, _| {
        if s == Step::Committed {
            let theirs: String = (1..=30)
                .map(|n| match n {
                    1 => "one\n".to_string(),
                    10 => "ten-colleague\n".to_string(),
                    _ => format!("{n}\n"),
                })
                .collect();
            fx.colleague_commits("Code/A.cs", &theirs);
        }
    })
    .unwrap();
    assert!(
        matches!(out, SyncOutOutcome::RebaseConflict { .. }),
        "{out:?}"
    );
    head
}

#[test]
fn the_delivery_buttons_do_not_touch_an_update_journal_and_vice_versa() {
    let fx = hybrid_with_colleague();
    svn_journal::write(
        &fx.wc_str(),
        &app_lib::engine::svn_journal::Journal {
            opid: "in".into(),
            flow: app_lib::engine::svn_journal::Flow::SyncIn,
            phase: Phase::Rebase,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        continue_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_wrong_flow"
    );
    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_wrong_flow"
    );

    let fx2 = hybrid_with_colleague();
    typical_work(&fx2);
    crash_at(&fx2, Step::Built);
    let err = app_lib::engine::svn_sync_in::continue_sync(&fx2.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_not_supported", "{}", err.message);
}

#[cfg(windows)]
#[test]
fn a_file_held_open_at_checkout_is_named_and_nothing_is_committed() {
    // Review Focus 3：Unity 以 share=Read 打开文件时，D3.8 的 checkout 退出 0 却没写进去（2c 的 A15）
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "deliverable"]);
    write(&fx, "Code/A.cs", "a2 LOCAL\n");
    git(&fx, &["commit", "-qam", "patch\n\nLocal-Only: true"]); // 交付树里的 A.cs 与磁盘不同
    let (head, status, server) = (
        git(&fx, &["rev-parse", "HEAD"]),
        svn_status(&fx),
        server_head(&fx),
    );

    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1) // FILE_SHARE_READ
        .open(fx.wc().join("Code/A.cs"))
        .unwrap();
    let err = deliver(&fx, "x").unwrap_err();
    assert_eq!(err.code, "sync_in_checkout_failed", "{}", err.message);
    assert!(
        err.message.contains("Code/A.cs"),
        "要指名被占用的文件：{}",
        err.message
    );
    drop(held);

    // 收尾在占用期间若没走完，journal 会保留；放开文件之后点「放弃」必须能收干净
    if svn_journal::read(&fx.wc_str()).unwrap().is_some() {
        abort_sync_out(&fx.wc_str()).unwrap();
    }
    assert_untouched(&fx, &head, &status, &server);
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a2 LOCAL\n"
    );
}

// —— Task 9：D6 / D7a 失败（提交已在服务器上）——

/// 用「更新」把同事提交的文件拉到 wc1（基线随之推进）。
fn update_from_server(fx: &SvnFixture) {
    let p = app_lib::engine::svn_sync_check::preview_sync_in(&fx.wc_str()).unwrap();
    app_lib::engine::svn_sync_in::run_sync_in(
        &fx.wc_str(),
        &app_lib::engine::svn_sync_in::SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
}

#[test]
fn a_failed_d6_is_retried_without_committing_again() {
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let away = fx.path().join("repo-away");
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::Committed {
            std::fs::rename(fx.repo(), &away).unwrap();
        }
    })
    .unwrap_err();
    std::fs::rename(&away, fx.repo()).unwrap();
    assert_eq!(err.code, "sync_out_update_failed", "{}", err.message);
    assert!(err.message.contains("「继续交付」"), "{}", err.message);
    assert!(
        err.message.contains("提交已经成功（r2）"),
        "{}",
        err.message
    );
    assert_eq!(phase(&fx), Phase::PostCommit);

    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }),
        "{out:?}"
    );
    assert_eq!(server_head(&fx), "2", "不重复提交");
    assert_settled(&fx);
}

#[test]
fn a_d6_conflict_on_a_file_git_does_not_manage_is_handed_to_tortoisesvn() {
    // 必测 #65 后半（spec §8.2.1 末、§15 第 25 条）
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/x.psd", "psd1\n");
    update_from_server(&fx);
    std::fs::write(fx.wc().join("Code/x.psd"), "psd LOCAL\n").unwrap();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(
        p.outside_changes,
        vec!["Code/x.psd".to_string()],
        "确认框要列出它"
    );

    let err = run_sync_out_probed(
        &fx.wc_str(),
        &expect_from(&p, "mine"),
        &|s: Step, _: &str| {
            if s == Step::Committed {
                fx.colleague_commits("Code/x.psd", "psd colleague\n");
            }
        },
    )
    .unwrap_err();
    assert_eq!(err.code, "sync_out_update_conflict", "{}", err.message);
    assert!(err.message.contains("「继续交付」"), "{}", err.message);
    assert!(
        err.message.contains("Code/x.psd") && err.message.contains("TortoiseSVN"),
        "{}",
        err.message
    );
    assert_eq!(phase(&fx), Phase::PostCommit);

    // 用户在 TortoiseSVN 里解决（这里用命令行模拟），再点继续。`mine-full` 是「保留我的」；
    // 计划原写 `working`——文本文件上 svn 已把冲突标记写进工作文件，`working` 会原样保留标记
    svn_in(
        &fx.wc(),
        &["resolve", "--accept", "mine-full", "Code/x.psd"],
    );
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { .. }), "{out:?}");
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/x.psd")).unwrap(),
        "psd LOCAL\n"
    );
}

#[test]
fn a_new_gitignore_from_the_server_stops_d7a_and_continue_picks_up_after_the_fix() {
    // 必测 #70 的 D7a 一半：保持已提交状态、不重复 D5；修正规则后从原录制阶段重新核对
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::Committed {
            fx.colleague_commits("Code/.gitignore", "*.cs\n");
            fx.colleague_commits("Code/New2.cs", "n2\n");
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "include_set_mismatch", "{}", err.message);
    assert!(err.message.contains("Code/New2.cs"), "{}", err.message);
    assert_eq!(phase(&fx), Phase::PostCommit);
    let base_before = git(&fx, &["rev-parse", "svn/base"]);
    assert!(
        git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@1 "),
        "基线没动"
    );

    // 修正规则：本地把 .gitignore 改空（它在 W-path 内、不是 W-file，svn 视为本地修改）
    std::fs::write(fx.wc().join("Code/.gitignore"), "").unwrap();
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    // rev 是我的提交 r2（里程碑 review 第 5 条）；新基线记的是 D6 之后的 r4
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }),
        "{out:?}"
    );
    assert!(git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@4 "));
    assert_ne!(git(&fx, &["rev-parse", "svn/base"]), base_before);
    assert_eq!(git(&fx, &["show", "svn/base:Code/New2.cs"]), "n2");
    assert_eq!(
        server_head(&fx),
        "4",
        "r2 是我们的，r3 / r4 是同事的，没有第二次交付"
    );
}

// —— Task 8–10 里程碑 review 的回归 ——

/// 在 `step` 处模拟崩溃，崩溃前先在 D5 之后插一笔同事的提交（`Code/Col.cs`），
/// 让 D6 有东西可拉、新基线与原分支不同。
fn crash_after_colleague(fx: &SvnFixture, step: Step) {
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let e = expect_from(&p, "mine");
    let wc = fx.wc_str();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_sync_out_probed(&wc, &e, &|s: Step, _: &str| {
            if s == Step::Committed {
                fx.colleague_commits("Code/Col.cs", "col\n");
            }
            if s == step {
                panic!("模拟崩溃于 {s:?}");
            }
        })
    }));
    assert!(r.is_err(), "应在 {step:?} 崩溃");
}

fn deliver_ref_name(fx: &SvnFixture) -> String {
    git(
        fx,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/mygitclient/deliver/",
        ],
    )
}

#[test]
fn resuming_after_the_user_left_the_deliver_tree_never_runs_svn_update() {
    // review 第 1 条：D6 之前必须先确认 HEAD 还在交付树上，否则 svn update 会把同事的改动
    // 合进一条本流程从没打算碰的分支
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_after_colleague(&fx, Step::Committed);
    assert_eq!(phase(&fx), Phase::PostCommit);
    git(&fx, &["checkout", "-q", "work"]); // 用户在 IDE 里切回了分支

    for (name, r) in [
        ("继续", continue_sync_out(&fx.wc_str())),
        ("放弃", abort_sync_out(&fx.wc_str())),
    ] {
        let err = r.unwrap_err();
        assert_eq!(
            err.code, "sync_out_state_changed",
            "{name}：{}",
            err.message
        );
        assert!(
            !fx.wc().join("Code/Col.cs").exists(),
            "{name}：不能先跑 svn update"
        );
        assert_eq!(phase(&fx), Phase::PostCommit, "{name}");
    }

    // 照报错说的切回交付树，就能接着走
    git(&fx, &["checkout", "-q", "--detach", &deliver_ref_name(&fx)]);
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }),
        "{out:?}"
    );
    assert!(fx.wc().join("Code/Col.cs").exists());
    assert_eq!(
        server_head(&fx),
        "3",
        "r2 是我的，r3 是同事的，没有第二次交付"
    );
    assert_settled(&fx);
}

#[test]
fn a_commit_made_on_the_detached_deliver_tree_is_not_thrown_away_by_abort() {
    // review 第 7 条：`checkout -f` 之前只看 W-file 的 svn 状态，看不见 detached 上的新提交
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Committed);
    write(&fx, "Code/A.cs", "a3\n");
    git(&fx, &["commit", "-qam", "made while paused"]);
    let mine = git(&fx, &["rev-parse", "HEAD"]);

    let err = abort_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_state_changed", "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), mine, "提交不能被切走");
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a3\n"
    );
    assert_eq!(phase(&fx), Phase::PostCommit);
}

#[test]
fn abandoning_on_the_original_branch_settles_checkout_residue() {
    // review 第 2 条：D8d 切回原分支时 Unity 占着文件（退出 0、HEAD 已切、字节还是新基线的），
    // 「放弃」不能把这种磁盘当作干净的报「已停止收尾」
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    write(&fx, "Code/A.cs", "a2 LOCAL\n");
    git(&fx, &["commit", "-qam", "patch\n\nLocal-Only: true"]);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    crash_after_colleague(&fx, Step::Published);
    assert_eq!(phase(&fx), Phase::RestoringBranch);
    // 模拟 A15 的残留：HEAD 已切到原分支，磁盘还是新基线的字节，该删的文件没删掉
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, "Code/Col.cs", "col\n");

    // 不是任何基线里的字节：不知道是谁的，不覆盖
    write(&fx, "Code/A.cs", "foreign\n");
    let err = abort_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_unsafe_to_return", "{}", err.message);
    assert!(err.message.contains("Code/A.cs"), "{}", err.message);
    // 交付横幅上没有「放弃」，只有「停止收尾」（最终 review 第 4 条）
    assert!(err.message.contains("「停止收尾」"), "{}", err.message);
    assert!(!err.message.contains("「放弃」"), "{}", err.message);
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "foreign\n"
    );

    write(&fx, "Code/A.cs", "a2\n");
    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap(),
        SyncOutOutcome::Abandoned { committed: true }
    );
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a2 LOCAL\n",
        "本地补丁要回到磁盘上"
    );
    assert!(!fx.wc().join("Code/Col.cs").exists());
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

#[test]
fn stopping_after_the_branch_already_reached_the_new_baseline_reports_it_delivered() {
    // 最终 review 第 7 条：快进的 update-ref 已经做完、只差切回原分支时崩溃，再点「停止收尾」。
    // 原分支已经在新基线上，「分支回到了交付前的位置、点更新」两句都是假话
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_after_colleague(&fx, Step::Published);
    assert_eq!(phase(&fx), Phase::RestoringBranch);
    let new = git(&fx, &["rev-parse", "svn/base"]);
    let before = git(&fx, &["rev-parse", "work"]);
    git(&fx, &["update-ref", "refs/heads/work", &new, &before]); // D8d 的 CAS 已经落地

    let out = abort_sync_out(&fx.wc_str()).unwrap();
    assert!(
        matches!(
            out,
            SyncOutOutcome::Delivered {
                rev: 2,
                local_only_left: 0,
                ..
            }
        ),
        "{out:?}"
    );
    assert_eq!(git(&fx, &["rev-parse", "work"]), new);
    assert_eq!(git(&fx, &["branch", "--show-current"]), "work");
    assert_settled(&fx);
}

#[test]
fn continuing_after_the_user_switched_back_by_hand_does_not_move_the_branch_under_head() {
    // review 第 3 条：HEAD 已在原分支上时，`update-ref` 会把当前分支挪到新基线、索引和磁盘
    // 却留在旧内容上——git 看来就是「你撤掉了同事的改动」
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_after_colleague(&fx, Step::Published);
    assert_eq!(phase(&fx), Phase::RestoringBranch);
    git(&fx, &["checkout", "-q", "work"]); // 磁盘回到交付前，Col.cs 被删掉

    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }),
        "{out:?}"
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/Col.cs")).unwrap(),
        "col\n"
    );
    assert_settled(&fx);
    assert_eq!(
        git(&fx, &["rev-parse", "work"]),
        git(&fx, &["rev-parse", "svn/base"])
    );
}

#[test]
fn continuing_never_switches_away_from_commits_made_during_the_interruption() {
    // 最终 review 第 2 条：没有本地补丁时 restore_branch 只认「HEAD 已在原分支上」，
    // HEAD 在别处（detached 做了提交、或切到了别的分支）时照样 checkout 原分支——
    // 那些提交只剩 reflog 能找回。停止收尾那一侧（里程碑 review 第 7 条）早就拦了
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_after_colleague(&fx, Step::Published);
    assert_eq!(phase(&fx), Phase::RestoringBranch);
    git(&fx, &["checkout", "-q", "--detach"]);
    write(&fx, "Code/A.cs", "made while the app was down\n");
    git(&fx, &["commit", "-qam", "during"]);
    let made = git(&fx, &["rev-parse", "HEAD"]);
    let work_before = git(&fx, &["rev-parse", "work"]);

    let err = continue_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_state_changed", "{}", err.message);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), made, "HEAD 不能被切走");
    assert_eq!(
        git(&fx, &["rev-parse", "work"]),
        work_before,
        "原分支也不能动"
    );
    assert_eq!(
        phase(&fx),
        Phase::RestoringBranch,
        "记录留着，回到原位后还能继续"
    );
}

#[test]
fn someone_elses_rebase_after_the_commit_is_named_not_reported_as_corruption() {
    // review 第 5 条：post-commit 还没有 newBaselineOid，拿它去认 rebase 会报成 journal 损坏
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Committed);
    git(&fx, &["checkout", "-q", "-b", "side", "svn/base"]);
    write(&fx, "Code/A.cs", "side\n");
    git(&fx, &["commit", "-qam", "side"]);
    git(&fx, &["checkout", "-q", "work"]);
    assert!(
        run_git(&fx.wc_str(), &["rebase", "side"]).is_err(),
        "要造出一个停在冲突上的 rebase"
    );

    for (name, r) in [
        ("继续", continue_sync_out(&fx.wc_str())),
        ("放弃", abort_sync_out(&fx.wc_str())),
    ] {
        let err = r.unwrap_err();
        assert_eq!(
            err.code, "sync_out_state_changed",
            "{name}：{}",
            err.message
        );
        assert!(err.message.contains("rebase"), "{name}：{}", err.message);
    }
    assert!(
        fx.wc().join(".git/rebase-merge").exists(),
        "别人的 rebase 不碰"
    );
    assert_eq!(phase(&fx), Phase::PostCommit);
}

#[test]
fn abandoning_offline_says_it_needs_the_network_too() {
    // review 第 4 条：放弃也要先 D6（否则留下 mixed-revision，之后的「更新」会把同事的改动
    // 当成你的 WIP）。离线时它会失败，报错不能只叫人去点「继续」
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Committed);
    let away = fx.path().join("repo-away");
    std::fs::rename(fx.repo(), &away).unwrap();
    let err = abort_sync_out(&fx.wc_str()).unwrap_err();
    std::fs::rename(&away, fx.repo()).unwrap();
    assert_eq!(err.code, "sync_out_update_failed", "{}", err.message);
    assert!(err.message.contains("停止收尾"), "{}", err.message);
    assert!(err.message.contains("mixed-revision"), "{}", err.message);
    assert_eq!(phase(&fx), Phase::PostCommit);

    assert_eq!(
        abort_sync_out(&fx.wc_str()).unwrap(),
        SyncOutOutcome::Abandoned { committed: true }
    );
}

#[test]
fn a_crash_before_the_new_baseline_commit_is_recaptured_on_continue() {
    // review 第 10 条：`publishing-baseline` 但还没有 newBaselineOid = commit 没建，从 D7 重录
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Captured);
    let mut j = svn_journal::read(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(j.phase, Phase::PublishingBaseline);
    j.new_baseline_oid = None;
    svn_journal::write(&fx.wc_str(), &j).unwrap();
    let old_base = git(&fx, &["rev-parse", "svn/base"]);

    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(
        matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }),
        "{out:?}"
    );
    assert_ne!(git(&fx, &["rev-parse", "svn/base"]), old_base);
    assert!(git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@2 "));
    assert_eq!(server_head(&fx), "2");
    assert_settled(&fx);
}

#[test]
fn an_unknown_phase_in_a_delivery_journal_is_not_blamed_on_update() {
    // review 第 10 条：交付记录里的 phase 认不出，不能说成「更新」流程留下的
    let fx = hybrid_with_colleague();
    svn_journal::write(
        &fx.wc_str(),
        &app_lib::engine::svn_journal::Journal {
            opid: "out".into(),
            flow: app_lib::engine::svn_journal::Flow::SyncOut,
            phase: Phase::Rebase,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    for r in [
        continue_sync_out(&fx.wc_str()),
        abort_sync_out(&fx.wc_str()),
    ] {
        let err = r.unwrap_err();
        assert_eq!(err.code, "sync_out_unknown_phase", "{}", err.message);
        assert!(err.message.contains("rebase"), "{}", err.message);
        assert!(!err.message.contains("「更新」流程"), "{}", err.message);
    }
}
