//! 仓库识别与能力声明。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::commands::repo::{probe_svn_environment, read_capabilities};
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
fn the_read_capabilities_command_returns_what_the_backend_says() {
    // 直接调命令函数本体。这**不能**替代真实窗口验证——Tauri 的
    // snake_case ↔ camelCase 参数名转换发生在 IPC 层，这里碰不到它
    // （`AGENTS.md` §7.3）。它能证明的是命令体本身没有绕过 backend_for
    // 自己拼一套判定，以及错误照常向上抛。
    let fx = svn_repo();
    hybrid(&fx);
    let caps = read_capabilities(fx.wc_str()).unwrap();
    assert_eq!(caps, backend_for(&fx.wc_str()).unwrap().capabilities());
    assert!(!caps.worktrees);

    let dir = TempDir::new().unwrap();
    git_init(dir.path());
    assert!(
        read_capabilities(dir.path().to_string_lossy().to_string())
            .unwrap()
            .worktrees
    );

    // 打不开的仓库要报错，不能悄悄返回一个「全都不支持」的能力集合
    // ——那会让界面变成一块空白，而不是一条错误横幅。
    let blocked = svn_repo();
    let err = read_capabilities(blocked.wc_str()).expect_err("未初始化的工作副本必须报错");
    assert_eq!(err.code, "svn_not_initialized");
}

#[test]
fn capabilities_serializes_to_the_exact_field_names_the_frontend_reads() {
    // `AGENTS.md` §7.3：Rust 与 JS 之间的字段名转换**没有编译期检查**。
    // `tsc` 只看 `src/engine/capabilities.ts` 里的接口声明，它不知道 Rust 那边
    // 真正吐出来的是什么；`#[serde(rename_all = "camelCase")]` 哪天被删掉，
    // JS 会拿到 `pull_requests` → 读 `caps.pullRequests` 得到 `undefined`
    // → falsy → **入口莫名其妙全消失了**，而两侧都编译通过、没有任何报错。
    //
    // 这条把那层转换的结果钉死在一组字面量上。下面这六个键必须与
    // `src/engine/capabilities.ts` 的 `interface Capabilities` 逐字一致。
    let json = serde_json::to_value(app_lib::vcs::Capabilities::all()).unwrap();
    let obj = json.as_object().expect("Capabilities 应当序列化成对象");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "cherryPick",
            "patches",
            "pullRequests",
            "remotes",
            "tags",
            "worktrees",
        ],
        "字段名与前端 interface Capabilities 对不上"
    );
    assert!(obj.values().all(|v| v == &serde_json::Value::Bool(true)));
}

#[test]
fn the_probe_svn_environment_command_reports_one_of_three_states() {
    // ⚠️ 这个命令在 Phase 1a **没有前端调用方**（它是给 1b 的初始化向导准备的），
    // 所以它的 invoke 路径没法在真实窗口里验证。这条至少把命令体和它的
    // 三态契约钉住：前端要靠这三个字符串分流「去装 / 重启应用 / 正常」，
    // 塌缩成两态就等于把「装完了但应用没重启」错报成「没装」（spec §17.1）。
    let state = probe_svn_environment().unwrap();
    assert!(
        ["available", "stale_path", "not_installed"].contains(&state.as_str()),
        "返回值必须是三态之一，实际 {state:?}"
    );
    // 开发机装了 SlikSvn（spec §17），这里应当是 available；不是的话说明
    // 探测逻辑坏了，而不是环境变了——tests/svn_cli.rs 同一前提下是绿的。
    assert_eq!(state, "available");
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
