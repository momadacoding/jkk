#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 「jkk 与 TortoiseSVN 看法不一致」的判定（设计 2026-11-25 §6.1、§8）。
//!
//! 全部在 `hybrid_with_colleague` 建的临时混合仓库里做：白名单 `Code/`、扩展名 `.cs`，
//! 停在 `work` 分支，`Code/A.cs` = `a1`、`Code/B.cs` = `b1`。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_mismatch::{find_baseline_mismatch, BaselineMismatch};
use std::path::Path;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, svn_repo};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "author")
        .env("GIT_AUTHOR_EMAIL", "author@example.invalid")
        .env("GIT_COMMITTER_NAME", "committer")
        .env("GIT_COMMITTER_EMAIL", "committer@example.invalid")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn mismatch(wc: &Path) -> BaselineMismatch {
    find_baseline_mismatch(&wc.to_string_lossy()).unwrap()
}

/// 在 `work` 上提交一笔（`--no-verify`：混合仓库的钩子不是这里要测的）。
fn commit_on_work(wc: &Path, rel: &str, content: &str, message_from: &str) {
    std::fs::write(wc.join(rel), content).unwrap();
    git(wc, &["add", "--", rel]);
    let msg = git(wc, &["log", "-1", "--format=%B", message_from]);
    let file = wc.join(".git").join("TEST_MSG");
    std::fs::write(&file, msg).unwrap();
    git(
        wc,
        &["commit", "-q", "--no-verify", "-F", &file.to_string_lossy()],
    );
}

#[test]
fn clean_repo_reports_nothing() {
    let fx = hybrid_with_colleague();
    let m = mismatch(&fx.wc());
    assert_eq!(m.files, Vec::<String>::new());
    assert_eq!(m.total, 0);
    assert!(!m.truncated);
}

#[test]
fn a_baseline_that_recorded_wrong_content_is_reported() {
    // 「有人手工往 svn/base 上提交过」：基线里的 A.cs 不是 SVN 手上的内容。基线的提交信息
    // 原样照抄（带 trailer），这样它在别处看来仍是一条合法基线，只是内容错了
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    commit_on_work(&wc, "Code/A.cs", "WRONG\n", "svn/base");
    let head = git(&wc, &["rev-parse", "HEAD"]);
    git(&wc, &["update-ref", "refs/heads/svn/base", head.trim()]);
    // 磁盘放回 SVN 的内容：TortoiseSVN 说没改，jkk 的「本地改动」里却有它
    svn_cmd(&wc, &["revert", "Code/A.cs"]);
    assert_eq!(
        std::fs::read_to_string(wc.join("Code/A.cs")).unwrap(),
        "a1\n"
    );

    let m = mismatch(&wc);
    assert_eq!(m.files, vec!["Code/A.cs".to_string()]);
    assert_eq!(m.total, 1);
}

#[test]
fn a_local_only_patch_reverted_in_svn_is_not_reported() {
    // 用户在分支上提交了 B.cs 的补丁，又在 TortoiseSVN 里 revert 掉：同样是「git 说改了、
    // SVN 说没改」，但那是分支提交碰过的文件，有「放回补丁」提示管，不是基线错了
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    commit_on_work(&wc, "Code/B.cs", "b-local\n", "HEAD");
    svn_cmd(&wc, &["revert", "Code/B.cs"]);
    assert_eq!(
        std::fs::read_to_string(wc.join("Code/B.cs")).unwrap(),
        "b1\n"
    );
    assert!(
        git(&wc, &["diff", "--name-only", "HEAD"]).contains("Code/B.cs"),
        "前提：git 说它改了"
    );

    assert_eq!(mismatch(&wc).files, Vec::<String>::new());
}

#[test]
fn a_file_the_user_edited_is_not_reported() {
    // 用户自己改的文件：SVN 也说 modified。这是最常见的情形，报了就是每次改文件都弹横幅
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    std::fs::write(wc.join("Code/A.cs"), "a1 edited\n").unwrap();

    assert_eq!(mismatch(&wc).files, Vec::<String>::new());
}

#[test]
fn a_file_added_only_in_git_is_not_reported() {
    // 只在 git 里新增的文件：SVN 说 unversioned，不是「SVN 说干净」
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    std::fs::write(wc.join("Code/New.cs"), "new\n").unwrap();
    git(&wc, &["add", "--", "Code/New.cs"]);

    assert_eq!(mismatch(&wc).files, Vec::<String>::new());
}

#[test]
fn only_the_wrong_file_is_reported_among_user_edits() {
    // 基线记错一个文件，同时用户在改另一个：只报记错的那个
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    commit_on_work(&wc, "Code/A.cs", "WRONG\n", "svn/base");
    let head = git(&wc, &["rev-parse", "HEAD"]);
    git(&wc, &["update-ref", "refs/heads/svn/base", head.trim()]);
    svn_cmd(&wc, &["revert", "Code/A.cs"]);
    std::fs::write(wc.join("Code/B.cs"), "b1 edited\n").unwrap();

    assert_eq!(mismatch(&wc).files, vec!["Code/A.cs".to_string()]);
}

#[test]
fn files_updated_by_svn_before_follow_are_not_reported() {
    // 2026-11-25 独立 review 阻断项：TortoiseSVN update 下来、jkk 还没跟进的文件同样是「git 说
    // 改了、SVN 说没改、分支没碰过」。前端的前提门槛看的失同步结论在 update 之后要几秒才跟上，
    // 那段时间门槛放行，所以判定自己必须分得开：SVN 手上的 revision 比基线记的新，丢掉
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    fx.colleague_commits("Code/A.cs", "a2\n");
    svn_cmd(&wc, &["update", "--non-interactive"]);
    let verdict = app_lib::engine::svn_sync::check_sync(&wc.to_string_lossy())
        .unwrap()
        .verdict;
    assert!(
        matches!(
            verdict,
            app_lib::engine::svn_sync::SyncVerdict::ExternallyAdvanced { .. }
        ),
        "前提：此刻顶部是「跟进」横幅，实际：{verdict:?}"
    );
    assert!(
        git(&wc, &["diff", "--name-only", "HEAD"]).contains("Code/A.cs"),
        "前提：git 说它改了"
    );

    assert_eq!(mismatch(&wc).files, Vec::<String>::new());
}

#[test]
fn a_wrong_baseline_is_still_reported_after_an_unrelated_svn_update() {
    // 上一条的反面：update 只带来了 B.cs 的新内容，A.cs 的 revision 仍与基线记的对得上，
    // 基线把 A.cs 记错了照样要报
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    commit_on_work(&wc, "Code/A.cs", "WRONG\n", "svn/base");
    let head = git(&wc, &["rev-parse", "HEAD"]);
    git(&wc, &["update-ref", "refs/heads/svn/base", head.trim()]);
    svn_cmd(&wc, &["revert", "Code/A.cs"]);
    fx.colleague_commits("Code/B.cs", "b2\n");
    svn_cmd(&wc, &["update", "--non-interactive"]);

    assert_eq!(mismatch(&wc).files, vec!["Code/A.cs".to_string()]);
}

#[test]
fn a_file_with_only_a_property_change_is_not_reported() {
    // spec §6.1 第 4 条：只改了属性的是 item=normal、props=modified，不是「SVN 说完全干净」。
    // 基线把它记错了也不报：体检对 SVN 认为改过的文件本来就证明不了什么
    let fx = hybrid_with_colleague();
    let wc = fx.wc();
    commit_on_work(&wc, "Code/A.cs", "WRONG\n", "svn/base");
    let head = git(&wc, &["rev-parse", "HEAD"]);
    git(&wc, &["update-ref", "refs/heads/svn/base", head.trim()]);
    svn_cmd(&wc, &["revert", "Code/A.cs"]);
    svn_cmd(&wc, &["propset", "x:test", "1", "Code/A.cs"]);

    assert_eq!(mismatch(&wc).files, Vec::<String>::new());
}

#[test]
fn refuses_a_directory_that_was_never_initialized() {
    let fx = svn_repo();
    let err = find_baseline_mismatch(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "svn_not_hybrid");
}

#[test]
fn serializes_with_camel_case_fields() {
    // Tauri 返回值的字段名没有编译期检查（commands/svn.rs 模块头）
    let v = serde_json::to_value(BaselineMismatch {
        files: vec!["Code/A.cs".into()],
        total: 1,
        truncated: false,
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "files": ["Code/A.cs"], "total": 1, "truncated": false })
    );
}
