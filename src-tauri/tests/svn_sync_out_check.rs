//! `sync_out` 的前置检查（spec §8.1）与 D1 / D2 清单。全部在临时 fixture 上跑。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal::{self, Flow, Journal, Phase};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use app_lib::engine::svn_sync_out_check::preview_sync_out;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn svn(dir: &std::path::Path, args: &[&str]) -> String {
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

/// 用 Phase 2c 的「更新」把基线推进到服务器最新。
fn update(fx: &SvnFixture) {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    run_sync_in(
        &fx.wc_str(),
        &SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
}

fn write(fx: &SvnFixture, rel: &str, content: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn the_preview_lists_a_m_d_and_the_local_only_patches() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    write(&fx, "Code/Deep/Nested/New.cs", "new\n");
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "work"]);
    write(&fx, "Code/A.cs", "a2 DEBUG\n");
    git(&fx, &["commit", "-qam", "debug\n\nLocal-Only: true"]);

    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let got: Vec<(String, String)> = p
        .changes
        .iter()
        .map(|c| (c.status.clone(), c.path.clone()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("M".to_string(), "Code/A.cs".to_string()),
            ("D".to_string(), "Code/B.cs".to_string()),
            ("A".to_string(), "Code/Deep/Nested/New.cs".to_string()),
        ]
    );
    assert_eq!(p.local_only.len(), 1);
    assert_eq!(p.local_only[0].subject, "debug");
    assert_eq!(p.orig_branch, "work");
    assert_eq!(p.remote_rev, p.baseline_rev);
    assert!(p.outside_changes.is_empty());
}

#[test]
fn the_preview_changes_nothing_and_takes_no_lock() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    let refs = git(&fx, &["for-each-ref"]);
    // A.cs 在 svn 看来是 M（git 提交了、还没交付），所以比的是「预览前后不变」，不是「为空」
    let svn_before = svn(&fx.wc(), &["status", "Code"]);
    preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(git(&fx, &["for-each-ref"]), refs);
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert_eq!(svn(&fx.wc(), &["status", "Code"]), svn_before);
}

#[test]
fn a_branch_still_on_the_old_baseline_is_refused() {
    // 必测 #57（§8.1 第 0 条、§8.1.2）：分支 B 从旧基线分出来，基线已被别的交付 / 更新推进
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "B"]);
    fx.colleague_commits("Code/ANew.cs", "anew\n");
    update(&fx); // work 与 svn/base 前进到 r2，含 ANew.cs
    git(&fx, &["checkout", "-q", "B"]);
    write(&fx, "Code/BOnly.cs", "b\n");
    git(&fx, &["add", "Code/BOnly.cs"]);
    git(&fx, &["commit", "-qm", "b only"]);

    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_behind_baseline", "{}", err.message);

    // 反向证据：这道门真拦着东西——两点 diff 会把别人刚交付的文件列成删除
    let two_dot = git(
        &fx,
        &["diff", "--name-status", "--no-renames", "svn/base", "B"],
    );
    assert!(two_dot.contains("D\tCode/ANew.cs"), "{two_dot}");
}

#[test]
fn detached_head_and_the_baseline_branch_are_refused() {
    // 必测 #31 的交付一半（§8.1.1）
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    git(&fx, &["checkout", "-q", "--detach"]);
    assert_eq!(
        preview_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_detached_head"
    );
    git(&fx, &["checkout", "-q", "svn/base"]);
    assert_eq!(
        preview_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_on_baseline"
    );
}

#[test]
fn uncommitted_work_inside_the_whitelist_is_refused() {
    // §8.1 第 2 条前半：不自动 stash——不该悄悄排除，也不该悄悄带上
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    write(&fx, "Code/Untracked.cs", "u\n");
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_dirty", "{}", err.message);
    assert!(err.message.contains("Code/Untracked.cs"), "{}", err.message);
}

#[test]
fn an_unfinished_git_operation_is_refused() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    git(&fx, &["revert", "--no-commit", "HEAD"]); // 留下 REVERT_HEAD
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_git_busy", "{}", err.message);
    assert!(err.message.contains("交付"), "{}", err.message);
}

#[test]
fn a_newer_revision_on_the_server_is_refused_even_outside_the_whitelist() {
    // §8.1 第 5 条：比的是 baselineRev。同事只提交了美术也一样要先更新
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    fx.colleague_commits("Art/x.psd", "psd2\n");
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_server_ahead", "{}", err.message);
}

#[test]
fn files_git_does_not_manage_inside_the_whitelist_are_listed_for_confirmation() {
    // §8.1 第 2 条后半 / 必测 #65 前半：Code/x.psd 在白名单目录里、被扩展名排除
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/x.psd", "psd1\n");
    update(&fx);
    write(&fx, "Code/x.psd", "psd LOCAL\n");
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/x.psd".to_string()]);
}

#[test]
fn deleting_the_last_file_of_a_directory_is_not_an_outside_change() {
    // 2026-10-13 E2E 场景 1 实测：`git rm` 删掉目录里最后一个文件时 git 顺手删掉空目录，
    // svn 把目录本身报成 `missing`。目录没有扩展名，被当成「白名单里 git 管不到、你本地改过的
    // 文件」，交付框弹出红色警告——误报，交付之后的 update 会把空目录拉回来
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Solo/S.cs", "solo\n");
    update(&fx);
    git(&fx, &["rm", "-q", "Code/Solo/S.cs"]);
    git(&fx, &["commit", "-qm", "drop solo"]);
    assert!(
        !fx.wc().join("Code/Solo").exists(),
        "前提：git 删掉了空目录"
    );

    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert!(p.outside_changes.is_empty(), "{:?}", p.outside_changes);
    assert_eq!(p.changes.len(), 1);
}

#[test]
fn a_directory_removed_by_hand_still_lists_the_files_git_does_not_manage() {
    // 护栏：跳过目录条目之后，目录里被删掉的非 W-file 仍逐个报出来——它们才是真正的本地改动
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Art/x.psd", "psd\n");
    fx.colleague_commits("Code/Art/y.cs", "y\n");
    update(&fx);
    std::fs::remove_dir_all(fx.wc().join("Code/Art")).unwrap();
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "drop art"]);

    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/Art/x.psd".to_string()]);
}

#[test]
fn a_directory_property_change_is_listed_even_when_a_file_inside_also_changed() {
    // 2026-10-16 review 第 3 条：上面那条只该跳过「目录整个不见了」，不能跳过所有带子条目的目录。
    // 在 TortoiseSVN 里改了目录的 svn:ignore，同时目录里一个 W-file 也改了：属性改动照样要确认
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/Art/y.cs", "y\n");
    update(&fx);
    svn(&fx.wc(), &["propset", "svn:ignore", "*.tmp", "Code/Art"]);
    write(&fx, "Code/Art/y.cs", "y2\n");
    git(&fx, &["commit", "-qam", "touch y"]);

    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/Art".to_string()]);
}

#[test]
fn nothing_to_deliver_says_how_many_local_patches_were_set_aside() {
    // §8.3 + spec §13「D1 / D2 空清单」：提示是否全部改动都被标成了 Local-Only
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a1 DEBUG\n");
    git(&fx, &["commit", "-qam", "debug\n\nLocal-Only: true"]);
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_nothing_to_deliver", "{}", err.message);
    assert!(err.message.contains("1 个本地补丁"), "{}", err.message);
}

#[test]
fn a_change_outside_the_whitelist_is_refused_not_filtered() {
    // 计划 B9：被 `git add -f` 加进来的白名单外文件
    let fx = hybrid_with_colleague();
    write(&fx, "Art/x.psd", "psd LOCAL\n");
    git(&fx, &["add", "-f", "Art/x.psd"]);
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work with art"]);
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_outside_whitelist", "{}", err.message);
    assert!(err.message.contains("Art/x.psd"), "{}", err.message);
}

#[test]
fn an_unfinished_sync_or_a_held_lock_blocks_the_preview() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    {
        let _l = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
        assert_eq!(
            preview_sync_out(&fx.wc_str()).unwrap_err().code,
            "sync_in_progress"
        );
    }
    svn_journal::write(
        &fx.wc_str(),
        &Journal {
            opid: "left".into(),
            flow: Flow::SyncIn,
            phase: Phase::Rebase,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        preview_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_in_unfinished"
    );
}

#[test]
fn a_rename_that_only_changes_case_is_refused() {
    // 里程碑 review：Windows 大小写不敏感，`D Code/B.cs` + `A Code/b.cs` 在磁盘上是同一个文件。
    // D4 先 add 新名、再 delete 旧名，删掉的就是要交付的那个文件
    let fx = hybrid_with_colleague();
    git(&fx, &["mv", "Code/B.cs", "Code/b.cs"]);
    git(&fx, &["commit", "-qm", "case rename"]);
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_unsupported_change", "{}", err.message);
    assert!(
        err.message.contains("Code/B.cs") && err.message.contains("Code/b.cs"),
        "{}",
        err.message
    );
}
