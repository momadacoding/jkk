//! §6.6 失同步判定。
//!
//! 每条测试都先在一个**干净**仓库上跑完初始化（P1/P2/P3 都得过），再由测试自己
//! 制造外部变更——「有人用 TortoiseSVN 在外面动了」按定义发生在本应用之外。
//! 直接拿一个已经失同步的 fixture 去初始化是造不出来的：向导会在前置校验就拦下。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::create_baseline_commit_with;
use app_lib::engine::svn_basemap::read_base_map;
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_sync::{self, ExternalChange, SyncVerdict};
use svn_fixtures::{
    svn_cmd, svn_repo, svn_repo_pinned_external_updated_to_head, svn_repo_three_revisions,
    svn_repo_with_same_repo_nested_external, NestedExternal, SvnFixture,
};

fn init(fx: &SvnFixture) {
    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
}

#[test]
fn a_freshly_initialized_repo_is_in_sync() {
    let fx = svn_repo();
    init(&fx);
    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert_eq!(st.verdict, SyncVerdict::InSync, "实际 {st:?}");
    assert_eq!(st.node_revs.len(), 1);
    assert_eq!(st.node_revs[0], st.baseline_rev);
}

// —— 文件级判定（计划 Task 3，分析 §6.2 第六轮）——
//
// 逐个基线文件 f 比较 R(f) 与 status 里的 n(f)（节点 revision）、c(f)（最后修改 revision）：
// n < R 回退；c > R 内容变了；其余视同相等。

fn seed(fx: &SvnFixture) -> std::path::PathBuf {
    let seed = fx.path().join("seed");
    svn_cmd(&seed, &["update", "--non-interactive"]);
    seed
}

/// 把 revision `rev` 的作者改成 `who`。file:// 访问时 `--username` 不改作者，只能直接改 revprop。
fn set_author(fx: &SvnFixture, rev: u64, who: &str) {
    let f = fx.path().join(format!("author-{rev}.txt"));
    std::fs::write(&f, who).unwrap();
    let out = std::process::Command::new("svnadmin")
        .args([
            "setrevprop",
            &fx.repo().to_string_lossy(),
            "-r",
            &rev.to_string(),
            "svn:author",
            &f.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn advanced(st: &svn_sync::SyncStatus) -> (Vec<ExternalChange>, Vec<ExternalChange>, Vec<String>) {
    match &st.verdict {
        SyncVerdict::ExternallyAdvanced {
            changed,
            added,
            removed,
        } => (changed.clone(), added.clone(), removed.clone()),
        other => panic!("应当判成 externallyAdvanced，实际 {other:?}（{st:?}）"),
    }
}

#[test]
fn commit_without_update_is_externally_advanced_not_mixed() {
    // 用户每天的形态：TortoiseSVN 里只提交、不更新。B 推到 r4，其余原地停在 r3
    let fx = svn_repo_three_revisions();
    init(&fx);
    std::fs::write(fx.wc().join("Code/B.cs"), "v4\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &[
            "commit",
            "-m",
            "r4",
            "--non-interactive",
            "--",
            "Code/B.cs@",
        ],
    );

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    let (changed, added, removed) = advanced(&st);
    assert_eq!(changed.len(), 1, "{changed:?}");
    assert_eq!((changed[0].path.as_str(), changed[0].rev), ("Code/B.cs", 4));
    assert!(
        changed[0].author.as_deref().is_some_and(|a| !a.is_empty()),
        "{changed:?}"
    );
    assert!(added.is_empty() && removed.is_empty());
    assert_eq!(st.baseline_rev, 3);
    assert_eq!(st.node_revs, vec![3, 4], "文件节点 revision 的并集");
    assert_eq!(
        (st.base.floor, st.base.newer, st.base.gone),
        (3, 0, 0),
        "基线本身仍是单值"
    );
}

#[test]
fn partial_update_without_content_change_stays_in_sync() {
    // E12：同事只提交了白名单外（扩展名不符）的文件，局部 update 把 Code 下所有节点推到 r4，
    // 但 .cs 一个字节都没变——不该挂横幅
    let fx = svn_repo_three_revisions();
    init(&fx);
    let seed = seed(&fx);
    std::fs::write(seed.join("Code/notes.txt"), "n\n").unwrap();
    svn_cmd(&seed, &["add", "Code/notes.txt", "--non-interactive"]);
    svn_cmd(&seed, &["commit", "-m", "r4 notes", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code@"]);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert_eq!(st.verdict, SyncVerdict::InSync, "{st:?}");
    assert_eq!(st.node_revs, vec![4], "节点 revision 确实被推高了");
    assert_eq!(st.baseline_rev, 3);
}

#[test]
fn partial_update_with_colleague_change_lists_colleague_as_author() {
    let fx = svn_repo_three_revisions();
    init(&fx);
    let seed = seed(&fx);
    std::fs::write(seed.join("Code/B.cs"), "colleague\n").unwrap();
    svn_cmd(
        &seed,
        &["commit", "-m", "r4 colleague", "--non-interactive"],
    );
    // ASCII：svnadmin setrevprop 按本机代码页读文件，中文会被转码成乱码（fixture 的限制，与产品无关）
    set_author(&fx, 4, "colleague-zhang");
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code@"]);

    let (changed, added, removed) = advanced(&svn_sync::check_sync(&fx.wc_str()).unwrap());
    assert_eq!(
        changed,
        vec![ExternalChange {
            path: "Code/B.cs".into(),
            rev: 4,
            author: Some("colleague-zhang".into()),
        }],
        "A 的节点 revision 也被推到 r4，但内容没变，不列"
    );
    assert!(added.is_empty() && removed.is_empty());
}

#[test]
fn colleague_added_w_file_is_reported_as_added_and_non_w_file_is_not() {
    let fx = svn_repo_three_revisions();
    init(&fx);
    let seed = seed(&fx);
    std::fs::create_dir_all(seed.join("Code/Deep")).unwrap();
    std::fs::write(seed.join("Code/Deep/N.cs"), "n\n").unwrap();
    std::fs::write(seed.join("Code/readme.txt"), "r\n").unwrap();
    svn_cmd(
        &seed,
        &["add", "Code/Deep", "Code/readme.txt", "--non-interactive"],
    );
    svn_cmd(&seed, &["commit", "-m", "r4 new", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code@"]);

    let (changed, added, removed) = advanced(&svn_sync::check_sync(&fx.wc_str()).unwrap());
    assert!(changed.is_empty() && removed.is_empty());
    // 目录 Code/Deep 本身也有 revision，扩展名不符的 readme.txt 不是 W-file：都不算
    assert_eq!(
        added.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        vec!["Code/Deep/N.cs"]
    );
    assert_eq!(added[0].rev, 4);
}

#[test]
fn copied_replacement_is_a_local_change_not_an_external_advance() {
    // Review 1 第 4 条：`svn rm B; svn cp <url>/G@4 B` 之后 B 是 `item=replaced copied=true`，
    // 没有 revision，`<commit>` 却是**复制源**的（r4、同事）。把它当 c(B) 会误报「SVN 侧有新内容」、
    // 作者还是错的；而这只是一个还没提交的本地改动
    let fx = svn_repo_three_revisions();
    init(&fx);
    let seed = seed(&fx);
    std::fs::write(seed.join("Code/G.cs"), "g4\n").unwrap();
    svn_cmd(&seed, &["add", "--non-interactive", "--", "Code/G.cs@"]);
    svn_cmd(&seed, &["commit", "-m", "r4", "--non-interactive"]);
    set_author(&fx, 4, "colleague");
    svn_cmd(
        &fx.wc(),
        &["delete", "--non-interactive", "--", "Code/B.cs@"],
    );
    let src = format!("{}/Code/G.cs@4", svn_fixtures::file_url(&fx.repo()));
    svn_cmd(&fx.wc(), &["copy", "--non-interactive", &src, "Code/B.cs"]);

    // 前提：真是 E13 那个形状
    let wc = fx.wc_str();
    let wl = vec!["Code/".to_string()];
    let wcs = app_lib::engine::svn_wc::enumerate_wcs(&wc, &wl).unwrap();
    let nodes = app_lib::engine::svn_nodes::read_node_status(&wc, &wl, &wcs).unwrap();
    let b = nodes.iter().find(|n| n.path == "Code/B.cs").unwrap();
    assert_eq!(
        (b.item.as_str(), b.copied, b.node_rev, b.commit_rev),
        ("replaced", true, None, Some(4)),
        "{b:?}"
    );

    assert_eq!(
        svn_sync::check_sync(&wc).unwrap().verdict,
        SyncVerdict::InSync
    );
}

#[test]
fn committed_deletion_is_reported_as_removed() {
    // E7 / 分析 §1.4 盲区二：提交删除后节点整个消失，其余 revision 都不变，
    // 旧的 nodeRevs 判据判成 InSync，基线里却还留着服务器上已经不存在的文件
    let fx = svn_repo_three_revisions();
    init(&fx);
    svn_cmd(
        &fx.wc(),
        &["delete", "--non-interactive", "--", "Code/B.cs@"],
    );
    svn_cmd(&fx.wc(), &["commit", "-m", "r4 rm", "--non-interactive"]);

    let (changed, added, removed) = advanced(&svn_sync::check_sync(&fx.wc_str()).unwrap());
    assert_eq!(removed, vec!["Code/B.cs".to_string()]);
    assert!(changed.is_empty() && added.is_empty());
}

#[test]
fn tombstoned_path_that_reappears_is_reported_as_added() {
    // 墓碑：基线树里没有、映射里记着「r4 已删除」。同事重新加回来、本机局部 update 之后，
    // 它是外部新增，而不是「墓碑对得上」
    let fx = svn_repo_three_revisions();
    init(&fx);
    svn_cmd(
        &fx.wc(),
        &["delete", "--non-interactive", "--", "Code/B.cs@"],
    );
    svn_cmd(&fx.wc(), &["commit", "-m", "r4 rm", "--non-interactive"]); // r4
    publish_baseline_without(&fx, "Code/B.cs", 4);
    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert_eq!(
        st.verdict,
        SyncVerdict::InSync,
        "墓碑对应的文件不在：相等，{st:?}"
    );
    assert_eq!((st.base.floor, st.base.gone), (3, 1));

    let seed = seed(&fx);
    std::fs::write(seed.join("Code/B.cs"), "back\n").unwrap();
    svn_cmd(&seed, &["add", "Code/B.cs", "--non-interactive"]);
    svn_cmd(&seed, &["commit", "-m", "r5 back", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", "Code@"]);

    let (changed, added, removed) = advanced(&svn_sync::check_sync(&fx.wc_str()).unwrap());
    assert_eq!(
        added
            .iter()
            .map(|c| (c.path.as_str(), c.rev))
            .collect::<Vec<_>>(),
        vec![("Code/B.cs", 5)]
    );
    assert!(changed.is_empty() && removed.is_empty());
}

/// 把 svn/base 换成「去掉 `path`、带墓碑 `path → rev`」的混合基线（Task 5 之前只能手工造）。
fn publish_baseline_without(fx: &SvnFixture, path: &str, rev: u64) {
    let repo = fx.wc_str();
    let old = run_git(&repo, &["rev-parse", "svn/base"]).unwrap();
    let old = old.trim();
    let mut map = read_base_map(&repo, old).unwrap();
    let idx = fx.path().join("tomb-index");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .current_dir(fx.wc())
            .env("GIT_INDEX_FILE", &idx)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["read-tree", old]);
    git(&["update-index", "--force-remove", "--", path]);
    let tree = git(&["write-tree"]);
    map.gone.insert(path.to_string(), rev);
    let oid = create_baseline_commit_with(&repo, &tree, Some(old), &map, &[], None).unwrap();
    run_git(&repo, &["update-ref", "refs/heads/svn/base", &oid, old]).unwrap();
}

#[test]
fn rolled_back_file_is_diverged() {
    // 外部只把一个文件退回旧 revision——工作副本根纹丝不动（spec §6.6.1）
    let fx = svn_repo_three_revisions();
    init(&fx);
    svn_cmd(
        &fx.wc(),
        &["update", "-r1", "--non-interactive", "--", "Code/A.cs@"],
    );

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert_eq!(
        st.verdict,
        SyncVerdict::Diverged {
            node_rev: 1,
            baseline_rev: 3
        },
        "{st:?}"
    );
}

#[test]
fn rollback_wins_over_external_advance() {
    // 同时有回退和前进：回退是真正的异常，跟进处理不了，必须报回退
    let fx = svn_repo_three_revisions();
    init(&fx);
    std::fs::write(fx.wc().join("Code/B.cs"), "v4\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &[
            "commit",
            "-m",
            "r4",
            "--non-interactive",
            "--",
            "Code/B.cs@",
        ],
    );
    svn_cmd(
        &fx.wc(),
        &["update", "-r1", "--non-interactive", "--", "Code/A.cs@"],
    );
    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert!(
        matches!(st.verdict, SyncVerdict::Diverged { node_rev: 1, .. }),
        "{st:?}"
    );
}

#[cfg(windows)]
#[test]
fn locked_working_copy_reports_wc_locked() {
    // E15 的陈旧锁：update 中途被一个独占打开的文件打断（造法同 svn_repo_interrupted_update）。
    // status 照样 rc=0，只有目录 entry 带 wc-locked；此时任何判定都可能看到半更新的状态
    use std::os::windows::fs::OpenOptionsExt;
    let fx = svn_repo_three_revisions();
    init(&fx);
    let seed = seed(&fx);
    std::fs::write(seed.join("Code/A.cs"), "v4\n").unwrap();
    svn_cmd(&seed, &["commit", "-m", "r4", "--non-interactive"]);
    {
        let _hold = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(fx.wc().join("Code/A.cs"))
            .unwrap();
        let ok = std::process::Command::new("svn")
            .current_dir(fx.wc())
            .args(["update", "--non-interactive"])
            .output()
            .unwrap()
            .status
            .success();
        assert!(!ok, "前提：被独占的文件让 update 半途失败");
    }

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    match &st.verdict {
        SyncVerdict::WcLocked { dirs } => {
            assert!(dirs.contains(&"Code".to_string()), "{dirs:?}")
        }
        other => panic!("应当判成 wcLocked，实际 {other:?}"),
    }
}

#[test]
fn changes_in_two_externals_are_both_reported() {
    // Review Focus 1：白名单落在两个工作副本里（Code 与嵌套 external Code/Vendor）。
    // status 要逐个 wc 跑并合并，两个单元各有一个文件被外部提交，两个都要列出来
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    init(&fx);
    std::fs::write(fx.wc().join("Code/A.cs"), "a2\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "A", "--non-interactive", "--", "Code/A.cs@"],
    );
    let vendor = fx.wc().join("Code/Vendor");
    std::fs::write(vendor.join("V.cs"), "v2\n").unwrap();
    svn_cmd(
        &vendor,
        &["commit", "-m", "V", "--non-interactive", "--", "V.cs@"],
    );

    let (changed, _, removed) = advanced(&svn_sync::check_sync(&fx.wc_str()).unwrap());
    assert_eq!(
        changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
        vec!["Code/A.cs", "Code/Vendor/V.cs"]
    );
    assert!(
        removed.is_empty(),
        "external 根的标记条目不能把另一个 wc 的文件判成消失"
    );
}

#[test]
fn whole_root_update_with_a_changed_file_is_externally_advanced() {
    // 原 A16 的「整体前进」：现在是 ExternallyAdvanced 的特例（分析 §6.2 末条）
    let fx = svn_repo_three_revisions();
    init(&fx);
    let seed = seed(&fx);
    std::fs::write(seed.join("Code/B.cs"), "v4\n").unwrap();
    svn_cmd(&seed, &["commit", "-m", "r4 外部提交", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    let (changed, _, _) = advanced(&st);
    assert_eq!(
        changed
            .iter()
            .map(|c| (c.path.as_str(), c.rev))
            .collect::<Vec<_>>(),
        vec![("Code/B.cs", 4)]
    );
    assert_eq!(st.node_revs, vec![4]);
}

#[test]
fn a_reset_to_the_externals_peg_is_distinguished_from_a_plain_divergence() {
    // spec §6.6.2：N 恰好等于该 external 的 peg revision ⇒ 有人在项目根跑了递归
    // `svn update`，把白名单拽回了 peg。
    //
    // 这时的出路是**一条 `svn update <白名单目录>`**，不是 §9.4 的完整恢复流程。
    // 判错的代价很具体：`Diverged` 会引导用户去「重建基线」，而那会把被钉死的
    // **旧内容**录成一个标着新 revision 的基线 —— 自己造出 §9.2 的基线撒谎。
    let fx = svn_repo_pinned_external_updated_to_head();
    init(&fx);

    // 有人在项目根跑了递归 update：SVN 按 externals 定义把 Code 拽回 peg
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    match &st.verdict {
        SyncVerdict::PinnedReset {
            node_rev,
            baseline_rev,
            external,
        } => {
            assert!(node_rev < baseline_rev, "被拽回旧 revision");
            assert!(!external.is_empty(), "要点名是哪条 external");
        }
        other => panic!(
            "应当判成 pinned reset，实际 {other:?}（node_revs={:?}）",
            st.node_revs
        ),
    }
}

#[test]
fn node_revs_unions_every_working_copy_including_nested_externals() {
    // spec §6.6.5：不变量是所有白名单所属工作副本的 nodeRevs **并集**是单元素集合。
    // 只查白名单根自己那个 WC，会漏掉底下嵌套 external 那块
    // 「git 管着、SVN 侧没人盯」的盲区。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    init(&fx);

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    // 干净的嵌套 external：两个 WC 都在同一个 revision 上，所以并集仍是单值。
    // 这条验的是**它真的去查了第二个 WC**，靠下面那条证伪。
    assert_eq!(st.verdict, SyncVerdict::InSync, "实际 {st:?}");

    // 把嵌套 external 单独退回旧 revision —— 只有并集才看得见
    svn_cmd(
        &fx.wc(),
        &["update", "-r1", "--non-interactive", "--", "Code/Vendor@"],
    );
    let after = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert!(
        !matches!(after.verdict, SyncVerdict::InSync),
        "嵌套 external 停在别的 revision 必须被看见，实际 {after:?}"
    );
}

#[test]
fn root_revs_covers_every_working_copy_and_is_cheap() {
    // §6.6.3 的 O(1) 提示档：每个 wc(P) 各一条。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    init(&fx);
    let got = svn_sync::read_root_revs(&fx.wc_str()).unwrap();
    assert!(
        got.len() >= 2,
        "嵌套 external 是独立工作副本，必须各占一条，实际 {got:?}"
    );
    assert!(got.iter().all(|r| r.rev > 0));
}

#[test]
fn root_revs_changes_when_someone_updates_outside() {
    // 第一档的判据是「与**上次观测值**比」，所以这个值必须真的会变。
    let fx = svn_repo_three_revisions();
    init(&fx);
    let before = svn_sync::read_root_revs(&fx.wc_str()).unwrap();

    let seed = fx.path().join("seed");
    svn_cmd(&seed, &["update", "--non-interactive"]);
    std::fs::write(seed.join("Code/B.cs"), "v4\n").unwrap();
    svn_cmd(&seed, &["commit", "-m", "r4", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let after = svn_sync::read_root_revs(&fx.wc_str()).unwrap();
    assert_ne!(before, after, "外部 update 之后 rootRev 必须变");
}

#[test]
fn a_repo_without_a_baseline_branch_is_a_loud_error() {
    // 没有 svn/base 时 baselineRev 无从谈起。返回 0 或者 InSync 都是撒谎 ——
    // 前者会让「N != 0」把每个仓库都判成失同步，后者会宣称一个不存在的基线是好的。
    let fx = svn_repo();
    // 故意不跑 init_hybrid
    let err = svn_sync::check_sync(&fx.wc_str()).expect_err("没有基线必须报错");
    assert_eq!(err.code, "svn_no_baseline", "实际 {err:?}");
}
