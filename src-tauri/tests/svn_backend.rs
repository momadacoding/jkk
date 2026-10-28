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
    // 这条把那层转换的结果钉死在一组字面量上。下面这几个键必须与
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
            "vcs",
            "worktrees",
        ],
        "字段名与前端 interface Capabilities 对不上"
    );
    // `vcs` 是字符串，其余仍然全是 true。分开断言而不是「所有值都是 true」——
    // 后者在加了非布尔字段之后就不成立了。
    assert_eq!(obj["vcs"], serde_json::Value::String("git".into()));
    assert!(obj
        .iter()
        .filter(|(k, _)| k.as_str() != "vcs")
        .all(|(_, v)| v == &serde_json::Value::Bool(true)));
}

// ---------------------------------------------------------------------------
// Phase 1b：四个新命令的返回类型也要把字段名钉住
//
// 同样的理由（`AGENTS.md` §7.3）：`tsc` 只看 `src/engine/svnInit.ts` 里的接口
// 声明，它不知道 Rust 那边真正吐出来的是什么。
// ---------------------------------------------------------------------------

fn keys_of(v: &serde_json::Value) -> Vec<String> {
    let mut k: Vec<String> = v
        .as_object()
        .expect("应当序列化成对象")
        .keys()
        .cloned()
        .collect();
    k.sort();
    k
}

#[test]
fn preflight_serializes_to_camel_case_and_keeps_the_dirty_list_off_the_wire() {
    let json = serde_json::to_value(app_lib::engine::svn_preflight::Preflight {
        node_rev: 7,
        repos_root: "file:///x".into(),
        uuid: "u".into(),
        // 非空，否则「它没被序列化」这件事证明不了——空 Vec 序列化出来也是看不见的
        dirty: vec![app_lib::engine::svn_xml::StatusEntry {
            path: "Code/A.cs".into(),
            item: "modified".into(),
            props: "none".into(),
            wc_locked: false,
        }],
    })
    .unwrap();
    assert_eq!(keys_of(&json), vec!["nodeRev", "reposRoot", "uuid"]);
    assert_eq!(json["nodeRev"], 7);
    // `dirty` 带 `#[serde(skip)]` 是有意的：真实工程上它可能有几千条（白名单底下
    // 每个未版本控制文件都算），而前端一条都不需要。这条断言防的是有人顺手把
    // skip 去掉——那会让每次 preflight 都往 IPC 上搬几千条路径。
    assert!(
        json.get("dirty").is_none(),
        "脏清单不该过 IPC。实际 JSON：{json}"
    );
}

#[test]
fn the_three_health_states_serialize_as_a_discriminated_union() {
    // 前端要按结果分三种显示（spec §6.8.6），所以判别字段必须稳定。
    // serde 默认的外部标签会吐成 `{"Verified":{"rev":1}}`——键名带大写、
    // 还要多剥一层。这里用 `#[serde(tag = "status")]` 压平成前端好读的形状。
    use app_lib::engine::svn_health::Health;

    let v = serde_json::to_value(Health::Verified {
        rev: 12346,
        unverified: vec![],
    })
    .unwrap();
    assert_eq!(v["status"], "verified");
    assert_eq!(v["rev"], 12346);
    // 空数组也必须**出现**：前端无条件读 `h.unverified.length`，
    // 字段缺席会在运行时炸，而 TS 的类型检查看不到 IPC 这一层。
    assert_eq!(v["unverified"], serde_json::json!([]));

    let vu = serde_json::to_value(Health::Verified {
        rev: 7,
        unverified: vec!["Code/A.cs".into()],
    })
    .unwrap();
    assert_eq!(vu["status"], "verified");
    assert_eq!(vu["unverified"][0], "Code/A.cs");

    let l = serde_json::to_value(Health::Lying {
        files: vec!["Code/A.cs".into()],
    })
    .unwrap();
    assert_eq!(l["status"], "lying");
    assert_eq!(l["files"][0], "Code/A.cs");

    let n = serde_json::to_value(Health::NotChecked {
        reason: "断网了".into(),
    })
    .unwrap();
    assert_eq!(n["status"], "notChecked");
    assert_eq!(n["reason"], "断网了");
}

#[test]
fn init_and_rebuild_reports_serialize_to_camel_case() {
    use app_lib::engine::svn_health::Health;
    use app_lib::engine::svn_init::{InitReport, RebuildReport};

    let init = serde_json::to_value(InitReport {
        node_rev: 1,
        baseline: "abc".into(),
        health: Health::Verified {
            rev: 1,
            unverified: vec![],
        },
        pinned: vec![app_lib::engine::svn_externals::PinnedExternal {
            defined_in: "Code".into(),
            local_path: "Code/pinA".into(),
            pinned_rev: 1,
        }],
        timings: vec![app_lib::engine::svn_init::PhaseTiming {
            phase: "预检".into(),
            millis: 12,
        }],
    })
    .unwrap();
    assert_eq!(
        keys_of(&init),
        vec!["baseline", "health", "nodeRev", "pinned", "timings"]
    );
    assert_eq!(keys_of(&init["timings"][0]), vec!["millis", "phase"]);
    assert_eq!(
        keys_of(&init["pinned"][0]),
        vec!["definedIn", "localPath", "pinnedRev"]
    );

    let rebuild = serde_json::to_value(RebuildReport {
        node_rev: 2,
        old_baseline: "a".into(),
        new_baseline: "b".into(),
        health: Health::NotChecked { reason: "x".into() },
    })
    .unwrap();
    assert_eq!(
        keys_of(&rebuild),
        vec!["health", "newBaseline", "nodeRev", "oldBaseline"]
    );
}

#[test]
fn sync_state_serializes_to_camel_case() {
    // 与上面那条同一个理由：`snake_case → camelCase` 没有编译期检查，
    // 写错只会在运行时静默变成 `undefined`。前端 `svnJournal.ts` 的
    // `LockHolder.startedAt` / `UnfinishedSync.startedAt` 全靠这层转换。
    use app_lib::commands::svn::{SyncState, UnfinishedSync};
    use app_lib::engine::svn_journal::{Flow, Phase};
    use app_lib::engine::svn_lock::LockHolder;

    let st = serde_json::to_value(SyncState {
        locked: true,
        holder: Some(LockHolder {
            pid: 4242,
            started_at: 1_757_000_000,
            opid: "abc".into(),
        }),
        unfinished: Some(UnfinishedSync {
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            opid: "abc".into(),
            started_at: 1_757_000_000,
            rolling_back: true,
            expected_start_rev: None,
            commit_message: None,
            committed_rev: None,
            orig_branch: None,
        }),
    })
    .unwrap();

    assert_eq!(keys_of(&st), vec!["holder", "locked", "unfinished"]);
    assert_eq!(keys_of(&st["holder"]), vec!["opid", "pid", "startedAt"]);
    assert_eq!(
        keys_of(&st["unfinished"]),
        vec!["flow", "opid", "phase", "rollingBack", "startedAt"]
    );
    // 值本身也钉住：前端按这两个字符串分流（spec §7.6 的 phase 表）。
    assert_eq!(st["unfinished"]["flow"], "sync_in");
    assert_eq!(st["unfinished"]["phase"], "stashed");
    // `applying-stash` 的横幅按它分方向（PR #49 review #3）
    assert_eq!(st["unfinished"]["rollingBack"], true);

    // 交付停在 `committing` 时横幅要显示这两项（Phase 3a 计划 B4）；为空时不发
    let out = serde_json::to_value(UnfinishedSync {
        flow: Flow::SyncOut,
        phase: Phase::Committing,
        opid: "o".into(),
        started_at: 1,
        rolling_back: false,
        expected_start_rev: Some(12),
        commit_message: Some("修复跳跃".into()),
        committed_rev: None,
        orig_branch: None,
    })
    .unwrap();
    assert_eq!(
        keys_of(&out),
        vec![
            "commitMessage",
            "expectedStartRev",
            "flow",
            "opid",
            "phase",
            "rollingBack",
            "startedAt"
        ]
    );
    assert_eq!(out["flow"], "sync_out");
    assert_eq!(out["phase"], "committing");

    // 提交之后停下的，横幅标题要写「交付已提交（rN）」（2026-10-14 E2E 用户反馈）
    let out = serde_json::to_value(UnfinishedSync {
        flow: Flow::SyncOut,
        phase: Phase::PublishingBaseline,
        opid: "o".into(),
        started_at: 1,
        rolling_back: false,
        expected_start_rev: Some(10),
        commit_message: None,
        committed_rev: Some(11),
        // 横幅要点名是哪个分支：中途 HEAD 停在 svn/base 上，工具栏显示的不是它（2026-10-14 E2E）
        orig_branch: Some("e2e8".into()),
    })
    .unwrap();
    assert_eq!(out["committedRev"], 11);
    assert_eq!(out["origBranch"], "e2e8");
}

#[test]
fn sync_out_shapes_serialize_to_the_exact_names_the_frontend_reads() {
    use app_lib::engine::svn_deliver::LocalOnlyCommit;
    use app_lib::engine::svn_sync_out::{SyncOutExpect, SyncOutOutcome};
    use app_lib::engine::svn_sync_out_check::{DeliverChange, SyncOutPreview};

    let p = SyncOutPreview {
        orig_branch: "work".into(),
        orig_head: "h".into(),
        baseline_oid: "b".into(),
        baseline_rev: 1,
        remote_rev: 1,
        deliver_oid: "d".into(),
        deliver_tree: "t".into(),
        changes: vec![DeliverChange {
            status: "A".into(),
            path: "Code/N.cs".into(),
        }],
        local_only: vec![LocalOnlyCommit {
            oid: "l".into(),
            subject: "mute".into(),
        }],
        outside_changes: vec![],
    };
    let v = serde_json::to_value(&p).unwrap();
    assert_eq!(
        keys_of(&v),
        vec![
            "baselineOid",
            "baselineRev",
            "changes",
            "deliverOid",
            "deliverTree",
            "localOnly",
            "origBranch",
            "origHead",
            "outsideChanges",
            "remoteRev",
        ]
    );
    assert_eq!(v["localOnly"][0]["subject"], "mute");

    let o = serde_json::to_value(SyncOutOutcome::Delivered {
        rev: 2,
        branch: "work".into(),
        local_only_left: 1,
    })
    .unwrap();
    assert_eq!(
        o,
        serde_json::json!({ "kind": "delivered", "rev": 2, "branch": "work", "localOnlyLeft": 1 })
    );
    let o = serde_json::to_value(SyncOutOutcome::RebaseConflict {
        rev: 3,
        files: vec!["a".into()],
    })
    .unwrap();
    assert_eq!(
        o,
        serde_json::json!({ "kind": "rebaseConflict", "rev": 3, "files": ["a"] })
    );
    let o = serde_json::to_value(SyncOutOutcome::Abandoned { committed: true }).unwrap();
    assert_eq!(
        o,
        serde_json::json!({ "kind": "abandoned", "committed": true })
    );

    let e: SyncOutExpect = serde_json::from_value(serde_json::json!({
        "origBranch": "work", "origHead": "h", "baselineOid": "b", "deliverTree": "t",
        "changes": [{ "status": "M", "path": "Code/A.cs" }],
        "confirmedOutside": [], "message": "m"
    }))
    .unwrap();
    assert_eq!(e.changes[0].path, "Code/A.cs");
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

// ═══════════════════════════════════════════════════════════════════════════
// Phase 1c 的五个命令
// ═══════════════════════════════════════════════════════════════════════════

/// 在 fixture 上跑完初始化，得到一个可用的混合仓库。
fn init_hybrid_on(fx: &svn_fixtures::SvnFixture) {
    app_lib::engine::svn_init::init_hybrid(&app_lib::engine::svn_init::InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
}

#[test]
fn capabilities_declare_which_vcs_backs_the_repo() {
    // 前端要据此决定「要不要发那几个 SVN 专属的 invoke」。靠「所有 git 专属能力
    // 都是 false」反推是脆的——将来某个 git 仓库恰好关掉全部可选能力时就会被
    // 当成 SVN，于是对一个普通 git 仓库每次刷新都发几个必然失败的 invoke。
    let dir = TempDir::new().unwrap();
    git_init(dir.path());
    assert_eq!(
        read_capabilities(dir.path().to_string_lossy().to_string())
            .unwrap()
            .vcs,
        "git"
    );

    let fx = svn_repo();
    init_hybrid_on(&fx);
    assert_eq!(read_capabilities(fx.wc_str()).unwrap().vcs, "svn");
}

#[test]
fn svn_read_log_never_touches_the_network() {
    // 打开仓库要先用缓存把图画出来（spec §10.2）。这条命令必须是纯本地读，
    // 缓存还不存在时返回空列表而不是报错——那只是「还没拉过」。
    let fx = svn_repo();
    init_hybrid_on(&fx);
    let got = app_lib::commands::svn::svn_read_log(fx.wc_str()).unwrap();
    assert!(got.revs.is_empty(), "还没拉过，应当是空列表，实际 {got:?}");
    assert!(!got.complete, "「还没拉过」不是「已经拉完」，实际 {got:?}");
}

#[test]
fn svn_fetch_log_populates_the_cache_and_is_idempotent() {
    let fx = svn_repo();
    init_hybrid_on(&fx);
    let first = app_lib::commands::svn::svn_fetch_log(fx.wc_str(), false, 200).unwrap();
    assert!(!first.revs.is_empty(), "拉完应当有历史");

    // 再拉一次不该产生任何差异：SVN 历史不可变，增量区间也已经覆盖到 HEAD。
    let second = app_lib::commands::svn::svn_fetch_log(fx.wc_str(), false, 200).unwrap();
    assert_eq!(first, second);

    // 缓存已落盘，纯本地读现在拿得到同一份
    assert_eq!(
        app_lib::commands::svn::svn_read_log(fx.wc_str()).unwrap(),
        first
    );
}

#[test]
fn svn_fetch_log_includes_the_baseline_anchor_even_when_it_missed_the_whitelist() {
    // C 组实测：baselineRev 通常就是一个没碰过白名单的 revision，所以孤儿 baseline
    // 是**常态**。锚点不在序列里，工作分支在图上就挂不到 SVN 主干。
    let fx = svn_fixtures::svn_repo_baseline_rev_outside_whitelist();
    init_hybrid_on(&fx);
    let baseline = app_lib::commands::svn::svn_sync_status(fx.wc_str())
        .unwrap()
        .baseline_rev;

    // 白名单 URL 的历史里确实没有它
    let got = app_lib::commands::svn::svn_fetch_log(fx.wc_str(), false, 200).unwrap();
    assert!(
        got.revs.iter().any(|e| e.rev == baseline),
        "baseline r{baseline} 必须在序列里（靠锚点补拉），实际 {:?}",
        got.revs.iter().map(|e| e.rev).collect::<Vec<_>>()
    );
}

#[test]
fn svn_sync_status_and_root_revs_are_reachable_through_the_command_layer() {
    let fx = svn_repo();
    init_hybrid_on(&fx);
    let st = app_lib::commands::svn::svn_sync_status(fx.wc_str()).unwrap();
    assert_eq!(st.node_revs.len(), 1);
    let revs = app_lib::commands::svn::svn_root_revs(fx.wc_str()).unwrap();
    assert!(!revs.is_empty());
}

#[test]
fn svn_server_audit_uses_the_current_baseline_revision() {
    let fx = svn_repo();
    init_hybrid_on(&fx);
    let got = app_lib::commands::svn::svn_server_audit(fx.wc_str()).unwrap();
    // 干净仓库：服务器内容与磁盘一致
    let json = serde_json::to_value(&got).unwrap();
    assert_eq!(json["status"], "matched", "实际 {json}");
}

#[test]
fn svn_server_audit_on_a_repo_without_a_baseline_is_not_checked_rather_than_an_error() {
    // 调用方是「打开仓库」那条热路径。没有基线是一个正常状态，不该在那儿
    // 制造一条错误横幅。
    let fx = svn_repo();
    // 故意不建库
    let got = app_lib::commands::svn::svn_server_audit(fx.wc_str());
    match got {
        Ok(v) => {
            let json = serde_json::to_value(&v).unwrap();
            assert_eq!(json["status"], "notChecked", "实际 {json}");
        }
        Err(e) => panic!("不该报错，实际 {e:?}"),
    }
}

#[test]
fn the_new_commands_serialize_with_the_field_names_the_frontend_reads() {
    // 与 capabilities 那条同理：snake_case → camelCase 没有编译期检查。
    // 这里把前端真正会读的键钉死。
    let fx = svn_repo();
    init_hybrid_on(&fx);

    let st = serde_json::to_value(app_lib::commands::svn::svn_sync_status(fx.wc_str()).unwrap())
        .unwrap();
    assert!(st.get("baselineRev").is_some(), "实际 {st}");
    assert!(st.get("nodeRevs").is_some(), "实际 {st}");
    assert_eq!(st["verdict"]["kind"], "inSync", "实际 {st}");

    let revs =
        serde_json::to_value(app_lib::commands::svn::svn_root_revs(fx.wc_str()).unwrap()).unwrap();
    assert!(revs[0].get("wcRoot").is_some(), "实际 {revs}");

    let log = serde_json::to_value(
        app_lib::commands::svn::svn_fetch_log(fx.wc_str(), false, 200).unwrap(),
    )
    .unwrap();
    assert!(log.get("complete").is_some(), "实际 {log}");
    assert!(log["revs"][0].get("rev").is_some(), "实际 {log}");
    assert!(log["revs"][0].get("message").is_some(), "实际 {log}");
}

#[test]
fn every_sync_verdict_variant_serializes_its_fields_in_camel_case() {
    // **上面那条测不到这个。** 干净仓库恒为 `InSync`，而 `InSync` 没有字段；
    // 带字段的三个变体一条都没被序列化过。
    //
    // 2026-09-16 独立 review 发现：枚举上的 `rename_all` 只重命名**变体名**，
    // 不重命名变体**内部的字段**。于是 `Diverged` 实际发出 `node_rev` /
    // `baseline_rev`，而前端 `svnSync.ts` 读的是 `nodeRev` / `baselineRev`：
    //
    // - `forward = v.nodeRev > v.baselineRev` → `undefined > undefined` = false
    //   ⇒ **工作副本前进被说成「被回退到基线之前」**，方向正好反了；
    // - 文案里插值出 `r undefined`。
    //
    // 这三个变体只能手工构造——要在集成测试里真造出 pinned 场景得另起一个
    // fixture，而这里要钉的是序列化形状，不是判定逻辑（那个 `svn_sync.rs` 里有）。
    use app_lib::engine::svn_sync::SyncVerdict;

    let mixed = serde_json::to_value(SyncVerdict::MixedRevision { revs: vec![3, 5] }).unwrap();
    assert_eq!(mixed["kind"], "mixedRevision", "实际 {mixed}");
    assert_eq!(mixed["revs"][1], 5, "实际 {mixed}");

    let diverged = serde_json::to_value(SyncVerdict::Diverged {
        node_rev: 9,
        baseline_rev: 4,
    })
    .unwrap();
    assert_eq!(diverged["kind"], "diverged", "实际 {diverged}");
    assert_eq!(diverged["nodeRev"], 9, "实际 {diverged}");
    assert_eq!(diverged["baselineRev"], 4, "实际 {diverged}");
    assert!(
        diverged.get("node_rev").is_none(),
        "不能同时留着 snake_case 的写法：{diverged}"
    );

    let pinned = serde_json::to_value(SyncVerdict::PinnedReset {
        node_rev: 4,
        baseline_rev: 9,
        external: "Vendor/Lib".to_string(),
    })
    .unwrap();
    assert_eq!(pinned["kind"], "pinnedReset", "实际 {pinned}");
    assert_eq!(pinned["nodeRev"], 4, "实际 {pinned}");
    assert_eq!(pinned["baselineRev"], 9, "实际 {pinned}");
    assert_eq!(pinned["external"], "Vendor/Lib", "实际 {pinned}");
}

#[test]
fn sync_in_types_cross_ipc_in_camel_case() {
    // AGENTS.md §7.3：snake_case ↔ camelCase 没有编译期检查。字段名写错只会在
    // 运行时拿到 undefined——svn_sync.rs 的 SyncVerdict 上真出过一次。
    use app_lib::engine::svn_sync_check::SyncInPreview;
    use app_lib::engine::svn_sync_in::{SyncInExpect, SyncInOutcome};

    let p = serde_json::to_value(SyncInPreview {
        orig_branch: "work".into(),
        orig_head: "h".into(),
        baseline_oid: "b".into(),
        baseline_rev: 1,
        target_rev: 2,
        catch_up_rev: Some(2),
        outside_changes: vec!["Art/x.psd".into()],
    })
    .unwrap();
    for k in [
        "origBranch",
        "origHead",
        "baselineOid",
        "baselineRev",
        "targetRev",
        "catchUpRev",
        "outsideChanges",
    ] {
        assert!(p.get(k).is_some(), "SyncInPreview 缺 `{k}`：{p}");
    }
    // `None` 必须发成 `null` 而不是整个键消失：前端按 `catchUpRev === null` 判断（Task 9）
    let none = serde_json::to_value(SyncInPreview {
        orig_branch: "work".into(),
        orig_head: "h".into(),
        baseline_oid: "b".into(),
        baseline_rev: 1,
        target_rev: 1,
        catch_up_rev: None,
        outside_changes: vec![],
    })
    .unwrap();
    assert_eq!(
        none.get("catchUpRev"),
        Some(&serde_json::Value::Null),
        "{none}"
    );

    let cases = [
        (
            SyncInOutcome::Done { new_rev: 2 },
            r#"{"kind":"done","newRev":2}"#,
        ),
        (SyncInOutcome::Aborted, r#"{"kind":"aborted"}"#),
        (
            SyncInOutcome::RebaseConflict {
                files: vec!["A".into()],
            },
            r#"{"kind":"rebaseConflict","files":["A"]}"#,
        ),
        (
            SyncInOutcome::StashConflict {
                files: vec!["A".into()],
                stash_oid: "o".into(),
            },
            r#"{"kind":"stashConflict","files":["A"],"stashOid":"o"}"#,
        ),
        (
            SyncInOutcome::StashBlocked {
                collisions: vec!["N".into()],
                stash_oid: "o".into(),
            },
            r#"{"kind":"stashBlocked","collisions":["N"],"stashOid":"o"}"#,
        ),
        (
            SyncInOutcome::Released {
                stash_oid: "o".into(),
                rolled_back: true,
            },
            r#"{"kind":"released","stashOid":"o","rolledBack":true}"#,
        ),
        (
            SyncInOutcome::Settled {
                stash_oid: "o".into(),
                rolled_back: false,
            },
            r#"{"kind":"settled","stashOid":"o","rolledBack":false}"#,
        ),
    ];
    for (v, want) in cases {
        assert_eq!(serde_json::to_string(&v).unwrap(), want);
    }

    let e: SyncInExpect = serde_json::from_str(
        r#"{"origBranch":"work","origHead":"h","baselineOid":"b","confirmedOutside":["Art/x.psd"]}"#,
    )
    .unwrap();
    assert_eq!(e.confirmed_outside, vec!["Art/x.psd".to_string()]);
    assert_eq!(e.orig_branch, "work");
}
