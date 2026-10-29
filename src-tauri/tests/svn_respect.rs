//! 「尊重 .gitignore」口径下，建库之后的每个入口都按同一口径判断（2026-10-28 设计 §4）。
//!
//! 反目标第 3 条：选了「尊重」之后，体检 / 更新 / 交付 / 重建里有任何一处还按「不尊重」
//! 的口径判断，表现就是把正常仓库误报成「基线撒谎」或「漏收」。这里每个入口各有一条。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::TempIndex;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_gitignore::display_path;
use app_lib::engine::svn_health::Health;
use app_lib::engine::svn_init::{check_current_baseline, rebuild_baseline};
use app_lib::engine::svn_server_set::{audit_server_set_at_baseline, ServerAudit};
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use app_lib::engine::svn_sync_out_check::preview_sync_out;
use svn_fixtures::{hybrid_with_colleague, svn_repo, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn baseline_files(fx: &SvnFixture) -> Vec<String> {
    git(fx, &["ls-tree", "-r", "--name-only", "svn/base"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// 把一个已建好的仓库切到「尊重」口径——与建库时勾选的效果相同。
fn enable_respect(fx: &SvnFixture) {
    let root = fx.wc_str();
    let mut cfg = read_config(&root).unwrap();
    cfg.respect_gitignore = true;
    cfg.version = 2;
    write_config(&root, &cfg).unwrap();
}

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

/// 「尊重」仓库：`Code/.gitignore` 写 `Gen*.cs`；同事提交了 `Code/Gen1.cs`，
/// 「更新」之后它在 SVN 里、在磁盘上，但不在基线里——用户选了不交给 git 管。
fn respect_with_hidden_server_file() -> SvnFixture {
    let fx = hybrid_with_colleague();
    enable_respect(&fx);
    std::fs::write(fx.wc().join("Code/.gitignore"), "Gen*.cs\n").unwrap();
    fx.colleague_commits("Code/Gen1.cs", "g1\n");
    update(&fx);
    fx
}

/// 「尊重」仓库，建库之后又出现一个 `.gitignore`，挡住了**已在基线里**的 `Code/A.cs`。
fn respect_with_hidden_baseline_file() -> SvnFixture {
    let fx = hybrid_with_colleague();
    enable_respect(&fx);
    std::fs::write(fx.wc().join("Code/.gitignore"), "A.cs\n").unwrap();
    fx
}

#[test]
fn add_paths_bypasses_ignore_rules_and_matches_hash_object() {
    let fx = svn_repo();
    app_lib::engine::svn_init::init_hybrid(&app_lib::engine::svn_init::InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
    std::fs::write(fx.wc().join(".gitignore"), "*.cs\n").unwrap();
    let root = fx.wc_str();
    let idx = TempIndex::empty(&root).unwrap();
    idx.add_all(&root, &["Code/".to_string()]).unwrap();
    assert!(
        idx.ls_files(&root).unwrap().is_empty(),
        "前提：add -A 收不到被挡的文件"
    );

    idx.add_paths(&root, &["Code/A.cs".to_string()]).unwrap();
    let staged = idx.ls_files(&root).unwrap();
    assert!(staged.contains("Code/A.cs"), "实际：{staged:?}");
    // 行尾过滤照常生效：blob 与 `hash-object --path` 一致
    let tree = idx.write_tree(&root).unwrap();
    let blob = git(&fx, &["rev-parse", &format!("{tree}:Code/A.cs")]);
    let want = git(&fx, &["hash-object", "--path", "Code/A.cs", "Code/A.cs"]);
    assert_eq!(blob, want);
    // 空列表是 no-op，绝不能退化成「全部」
    idx.add_paths(&root, &[]).unwrap();
}

#[test]
fn respect_update_records_a_baseline_without_gitignored_server_files() {
    let fx = respect_with_hidden_server_file();
    let files = baseline_files(&fx);
    assert!(
        !files.contains(&"Code/Gen1.cs".to_string()),
        "实际：{files:?}"
    );
    assert!(files.contains(&"Code/A.cs".to_string()), "实际：{files:?}");
    assert!(
        fx.wc().join("Code/Gen1.cs").is_file(),
        "svn update 应当把它取下来"
    );
    // 再预览一次「更新」：纳入集合预检不误报
    preview_sync_in(&fx.wc_str()).unwrap();
}

#[test]
fn respect_delivery_treats_gitignored_files_as_outside_git() {
    let fx = respect_with_hidden_server_file();
    std::fs::write(fx.wc().join("Code/Gen1.cs"), "g LOCAL\n").unwrap();
    std::fs::write(fx.wc().join("Code/A.cs"), "a2\n").unwrap();
    git(&fx, &["commit", "-qam", "work"]);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/Gen1.cs".to_string()]);
}

/// `work` 上提交了新文件 `Code/Foo.cs`（还没交付，基线里没有），之后才出现挡住它的
/// `.gitignore`。口径参照的是基线树（设计 §4），它不算 W-file，交付照样拒绝——但不能再说
/// 「不在白名单里、多半是 `git add -f`」：它就在白名单里。要点名是哪个文件的哪一行
/// （/code-review 第 4 条，用户 2026-10-30 选方案 B：口径不动，只把提示说对）。
#[test]
fn respect_delivery_names_the_gitignore_line_that_hides_a_committed_new_file() {
    let fx = hybrid_with_colleague();
    enable_respect(&fx);
    std::fs::write(fx.wc().join("Code/Foo.cs"), "foo\n").unwrap();
    git(&fx, &["add", "Code/Foo.cs"]);
    git(&fx, &["commit", "-qm", "foo"]);
    std::fs::write(fx.wc().join("Code/.gitignore"), "# 后来加的\nFoo.cs\n").unwrap();

    let err = preview_sync_out(&fx.wc_str()).expect_err("口径不变，必须拒绝");
    assert_eq!(err.code, "sync_out_hidden_by_gitignore", "{}", err.message);
    let gitignore = display_path(&fx.wc_str(), "Code/.gitignore");
    assert!(err.message.contains("Code/Foo.cs"), "{}", err.message);
    assert!(
        err.message
            .contains(&format!("{gitignore} 第 2 行 `Foo.cs`")),
        "{}",
        err.message
    );
    assert!(!err.message.contains("git add -f"), "{}", err.message);
}

/// 真正在白名单外的改动与被 `.gitignore` 挡住的同时出现：两类分开点名，前者沿用原来的说法。
#[test]
fn respect_delivery_still_calls_out_changes_outside_the_whitelist_separately() {
    let fx = hybrid_with_colleague();
    enable_respect(&fx);
    std::fs::write(fx.wc().join("Code/Foo.cs"), "foo\n").unwrap();
    std::fs::create_dir_all(fx.wc().join("Art")).unwrap();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd\n").unwrap();
    git(&fx, &["add", "Code/Foo.cs"]);
    git(&fx, &["add", "-f", "Art/x.psd"]);
    git(&fx, &["commit", "-qm", "foo"]);
    std::fs::write(fx.wc().join("Code/.gitignore"), "Foo.cs\n").unwrap();

    let err = preview_sync_out(&fx.wc_str()).expect_err("必须拒绝");
    assert_eq!(err.code, "sync_out_hidden_by_gitignore", "{}", err.message);
    let (hidden, outside) = err
        .message
        .split_once("不在白名单里")
        .unwrap_or_else(|| panic!("缺「不在白名单里」一段：{}", err.message));
    assert!(hidden.contains("Code/Foo.cs"), "{}", err.message);
    assert!(!hidden.contains("Art/x.psd"), "{}", err.message);
    assert!(outside.contains("Art/x.psd"), "{}", err.message);
}

#[test]
fn respect_server_audit_does_not_flag_gitignored_server_files() {
    let fx = respect_with_hidden_server_file();
    assert_eq!(
        audit_server_set_at_baseline(&fx.wc_str()).unwrap(),
        ServerAudit::Matched
    );
}

#[test]
fn respect_health_filter_drops_gitignored_diffs() {
    let fx = respect_with_hidden_server_file();
    std::fs::write(fx.wc().join("Code/Gen1.cs"), "g LOCAL\n").unwrap();
    match check_current_baseline(&fx.wc_str()).unwrap() {
        Health::Verified { unverified, .. } => {
            assert!(
                !unverified.contains(&"Code/Gen1.cs".to_string()),
                "{unverified:?}"
            )
        }
        other => panic!("应当 Verified，实际 {other:?}"),
    }
}

#[test]
fn respect_rebuild_keeps_baseline_files_a_later_gitignore_hides() {
    let fx = respect_with_hidden_baseline_file();
    rebuild_baseline(&fx.wc_str()).unwrap();
    let files = baseline_files(&fx);
    assert!(
        files.contains(&"Code/A.cs".to_string()),
        "基线悄悄变小了：{files:?}"
    );
}

#[test]
fn respect_manual_health_check_is_not_lying_when_a_later_gitignore_hides_baseline_files() {
    let fx = respect_with_hidden_baseline_file();
    let h = check_current_baseline(&fx.wc_str()).unwrap();
    assert!(matches!(h, Health::Verified { .. }), "实际 {h:?}");
}

#[test]
fn respect_update_precheck_keeps_baseline_files_a_later_gitignore_hides() {
    let fx = respect_with_hidden_baseline_file();
    preview_sync_in(&fx.wc_str()).unwrap();
}
