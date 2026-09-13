//! 仓库识别与能力声明。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
use app_lib::vcs::{backend_for, detect_repo_kind, RepoKind};
use std::fs;
use std::process::Command;
use svn_fixtures::svn_repo;
use tempfile::TempDir;

fn git_init(dir: &std::path::Path) {
    for args in [
        vec!["init"],
        vec!["config", "user.email", "t@t"],
        vec!["config", "user.name", "T"],
    ] {
        let out = Command::new("git")
            .current_dir(dir)
            .args(&args)
            .output()
            .expect("git not found");
        assert!(
            out.status.success(),
            "git {args:?} 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn write_config(dir: &std::path::Path, url: &str) {
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(
        dir.join(".git/mygitclient.json"),
        format!(
            r#"{{"version":1,"mode":"svn-git","svnUrl":"{url}",
                 "whitelist":["Code/"],"extensions":[".cs"],
                 "baselineBranch":"svn/base"}}"#
        ),
    )
    .unwrap();
}

/// 把一个 SVN 工作副本布置成完整的混合仓库：`.svn` + `.git` + 配置 + exclude。
fn hybrid(fx: &svn_fixtures::SvnFixture) {
    git_init(&fx.wc());
    write_config(&fx.wc(), "file:///whatever");
    let w = vec!["Code/".to_string()];
    let e = vec![".cs".to_string()];
    write_exclude(&fx.wc_str(), &generate_exclude(&w, &e)).unwrap();
}

#[test]
fn a_plain_git_repo_is_detected_as_git() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());
    assert_eq!(
        detect_repo_kind(dir.path().to_str().unwrap()),
        RepoKind::Git
    );
}

#[test]
fn a_directory_with_neither_marker_still_yields_a_backend() {
    // 本 Task 给 backend_for 新开了三条 Err 分支。既有行为是「任何目录都能拿到
    // 一个后端，出不出错交给下游 git2 去说」——「打开文件夹」那条路径依赖它。
    // 没有这条测试，新加的分支哪天多吃了一种目录也不会有人发现。
    let dir = TempDir::new().unwrap();
    assert_eq!(
        detect_repo_kind(dir.path().to_str().unwrap()),
        RepoKind::Git
    );
    assert!(backend_for(dir.path().to_str().unwrap()).is_ok());
}

#[test]
fn an_svn_working_copy_without_git_is_uninitialized() {
    let fx = svn_repo();
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnUninitialized);
}

#[test]
fn svn_plus_git_without_config_is_suspicious() {
    // spec §6.1：可能是别人留下的 .git，阻断并提示，不要猜。
    let fx = svn_repo();
    git_init(&fx.wc());
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnSuspicious);
}

#[test]
fn svn_plus_git_plus_config_is_hybrid() {
    let fx = svn_repo();
    hybrid(&fx);
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnHybrid);
}

#[test]
fn detection_checks_svn_before_git() {
    // 混合仓库两者都有，先查 .git 就会永远判成普通 git 仓库（spec §6.1）。
    let fx = svn_repo();
    hybrid(&fx);
    assert_ne!(detect_repo_kind(&fx.wc_str()), RepoKind::Git);
}

#[test]
fn the_two_blocking_states_carry_different_error_codes() {
    // 「有 .svn 无 .git」和「.svn + 无配置的 .git」都拒绝打开，但**引导完全相反**：
    // 前者是「来走初始化向导」，后者是「先查清这个 .git 哪来的，别直接用」。
    // 塌缩成同一个错误码就等于把两条相反的指引合并了（spec §6.1）。
    // `Box<dyn Vcs>` 没有 Debug，`expect_err` 用不了，走 `.err()`
    let fx = svn_repo();
    let uninit = backend_for(&fx.wc_str())
        .err()
        .expect("未初始化的工作副本必须阻断");
    assert_eq!(uninit.code, "svn_not_initialized");

    git_init(&fx.wc());
    let suspicious = backend_for(&fx.wc_str()).err().expect("可疑仓库必须阻断");
    assert_eq!(suspicious.code, "svn_unconfigured_git");

    assert_ne!(uninit.code, suspicious.code);
}

#[test]
fn a_hybrid_repo_with_a_broken_config_fails_at_open() {
    // 配置坏了不能当成「可疑」，也不能半个坏配置流到下游——SvnGitBackend::open
    // 就地校验。错误码沿用 Task 3 的 svn_config，前端的引导是「去修配置」。
    let fx = svn_repo();
    git_init(&fx.wc());
    fs::write(
        fx.wc().join(".git/mygitclient.json"),
        r#"{"version":1,"mode":"svn-git","svnUrl":"file:///x",
            "whitelist":["../etc"],"extensions":[".cs"],"baselineBranch":"svn/base"}"#,
    )
    .unwrap();
    // 判定层看得到配置文件，所以是 Hybrid；拦截发生在 open()
    assert_eq!(detect_repo_kind(&fx.wc_str()), RepoKind::SvnHybrid);
    let err = backend_for(&fx.wc_str())
        .err()
        .expect("坏配置必须在打开时就失败");
    assert_eq!(err.code, "svn_config");
}

#[test]
fn a_git_backend_advertises_every_capability() {
    let dir = TempDir::new().unwrap();
    git_init(dir.path());
    let caps = backend_for(dir.path().to_str().unwrap())
        .unwrap()
        .capabilities();
    assert!(caps.worktrees && caps.tags && caps.remotes);
    assert!(caps.pull_requests && caps.cherry_pick && caps.patches);
}

#[test]
fn an_svn_backend_hides_the_git_only_entries() {
    // Phase 0 的交接明确警告过：这些命令**没有进接缝**，仍会直连 git。
    // 光靠 trait 拦不住它们，必须靠 capabilities + 前端不渲染入口。
    let fx = svn_repo();
    hybrid(&fx);
    let caps = backend_for(&fx.wc_str()).unwrap().capabilities();
    assert!(!caps.worktrees, "SVN 模式下 worktree 整块隐藏");
    assert!(!caps.tags);
    assert!(!caps.remotes);
    assert!(!caps.pull_requests);
    assert!(!caps.cherry_pick);
    assert!(!caps.patches);
}

#[test]
fn an_svn_backend_still_delegates_git_reads() {
    // §16 Phase 1 验收：「能打开 SVN 仓库，本地 git 的暂存/分支/stash 可用」。
    let fx = svn_repo();
    hybrid(&fx);
    fs::write(fx.wc().join("Code/A.cs"), "changed\n").unwrap();

    let backend = backend_for(&fx.wc_str()).unwrap();
    let st = backend.status().unwrap();
    assert!(
        st.unstaged.iter().any(|f| f.path.contains("A.cs")),
        "白名单内的改动必须读得出来，实际：{:?}",
        st.unstaged
    );
    // 同一次读取还要证明 exclude 真的在生效——否则 SVN 的工作副本数据库会
    // 摆在「未跟踪文件」列表里等着用户一键 stage 进去。
    assert!(
        !st.unstaged.iter().any(|f| f.path.contains(".svn")),
        "SVN 元数据不该出现在状态列表里，实际：{:?}",
        st.unstaged
    );
}
