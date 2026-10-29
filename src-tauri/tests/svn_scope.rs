//! `WScope`：全项目唯一的 W-file 口径（2026-10-28 spec §4）。
//!
//! 「尊重 .gitignore」时 W-file = 白名单 ∧ 扩展名 ∧ 没被工作树里的 `.gitignore` 排除，
//! 但**已经在基线树里的文件**不会因为后来的 `.gitignore` 被移出去。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_scope::WScope;
use std::collections::BTreeSet;
use std::path::Path;
use svn_fixtures::{svn_repo, SvnFixture};

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn set(v: &[&str]) -> BTreeSet<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// 白名单 `Code/`、扩展名 `.cs` 建好库（基线里有 `Code/A.cs`、`Code/B.cs`）。
fn hybrid() -> SvnFixture {
    let fx = svn_repo();
    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: strs(&["Code/"]),
        extensions: strs(&[".cs"]),
    })
    .unwrap();
    fx
}

/// 把一个已建好的仓库切到「尊重」口径——与建库时勾选的效果相同。
fn enable_respect(wc: &Path) {
    let root = wc.to_str().unwrap();
    let mut cfg = read_config(root).unwrap();
    cfg.respect_gitignore = true;
    cfg.version = 2;
    write_config(root, &cfg).unwrap();
}

fn respect_scope(fx: &SvnFixture) -> WScope {
    let root = fx.wc_str();
    enable_respect(&fx.wc());
    WScope::for_repo(&root, &read_config(&root).unwrap()).unwrap()
}

#[test]
fn plain_scope_ignores_gitignore_entirely() {
    let fx = hybrid();
    std::fs::write(fx.wc().join(".gitignore"), "*.cs\n").unwrap();
    let plain = WScope::plain(&strs(&["Code/"]), &strs(&[".cs"]));
    assert_eq!(
        plain.filter(["Code/A.cs", "Code/n.txt"]).unwrap(),
        set(&["Code/A.cs"])
    );
    // 不尊重的仓库，`for_repo` 与 `plain` 同口径
    let root = fx.wc_str();
    let scope = WScope::for_repo(&root, &read_config(&root).unwrap()).unwrap();
    assert!(!scope.respects_gitignore());
    assert_eq!(scope.filter(["Code/A.cs"]).unwrap(), set(&["Code/A.cs"]));
}

#[test]
fn respect_scope_excludes_files_hidden_by_a_worktree_gitignore() {
    let fx = hybrid();
    std::fs::write(fx.wc().join("Code/.gitignore"), "Gen*.cs\n").unwrap();
    std::fs::write(fx.wc().join("Code/Gen1.cs"), "g\n").unwrap();
    let scope = respect_scope(&fx);
    assert_eq!(
        scope.filter(["Code/Gen1.cs", "Code/A.cs"]).unwrap(),
        set(&["Code/A.cs"])
    );
}

#[test]
fn respect_scope_keeps_files_already_in_the_baseline() {
    // git 本身的语义：已跟踪的文件不受 .gitignore 影响。参照统一是「当前基线树」。
    let fx = hybrid();
    std::fs::write(fx.wc().join(".gitignore"), "*.cs\n").unwrap();
    let scope = respect_scope(&fx);
    assert_eq!(
        scope.filter(["Code/A.cs", "Code/B.cs"]).unwrap(),
        set(&["Code/A.cs", "Code/B.cs"])
    );
    assert_eq!(
        scope.hidden_tracked_on_disk().unwrap(),
        strs(&["Code/A.cs", "Code/B.cs"])
    );
}

#[test]
fn hidden_tracked_on_disk_skips_files_deleted_from_disk() {
    // `update-index --add` 对磁盘上不存在的路径会报错；被删的文件由脏清单修正负责
    let fx = hybrid();
    std::fs::write(fx.wc().join(".gitignore"), "*.cs\n").unwrap();
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    let scope = respect_scope(&fx);
    assert_eq!(
        scope.hidden_tracked_on_disk().unwrap(),
        strs(&["Code/A.cs"])
    );
}

#[test]
fn scope_excludes_a_missing_file_under_an_ignored_directory() {
    // Review Focus 第 1 条：目录规则挡住整个目录，而其中某个服务器上的文件在本地不存在
    // （svn `!`）。服务器集合核对、体检过滤问到的正是这种路径。
    let fx = hybrid();
    std::fs::write(fx.wc().join("Code/.gitignore"), "Gen/\n").unwrap();
    assert!(
        !fx.wc().join("Code/Gen").exists(),
        "前提：目录在磁盘上不存在"
    );
    let scope = respect_scope(&fx);
    assert_eq!(
        scope.filter(["Code/Gen/X.cs", "Code/A.cs"]).unwrap(),
        set(&["Code/A.cs"])
    );
}

#[test]
fn respect_scope_never_admits_non_w_files() {
    // 「尊重」只做减法：`!` 反选不能把扩展名不匹配的文件加进来
    let fx = hybrid();
    std::fs::write(fx.wc().join("Code/.gitignore"), "!*.txt\n").unwrap();
    std::fs::write(fx.wc().join("Code/n.txt"), "n\n").unwrap();
    let scope = respect_scope(&fx);
    assert_eq!(
        scope.filter(["Code/n.txt", "Code/A.cs"]).unwrap(),
        set(&["Code/A.cs"])
    );
}

#[test]
fn rules_outside_worktree_gitignores_do_not_count_as_respected_exclusions() {
    // 只有工作树里的 `.gitignore` 算「已有的规则」。本应用自己的 info/exclude 挡的本来就
    // 不是 W-file，全局 excludesFile 压不过 info/exclude 的反选（PR #90 实测）。
    let fx = hybrid();
    std::fs::write(fx.wc().join("Code/New.cs"), "n\n").unwrap();
    let scope = respect_scope(&fx);
    assert_eq!(
        scope.filter(["Code/New.cs"]).unwrap(),
        set(&["Code/New.cs"])
    );
}

#[test]
fn empty_input_gives_an_empty_set() {
    let fx = hybrid();
    let scope = respect_scope(&fx);
    assert!(scope.filter(std::iter::empty()).unwrap().is_empty());
}
