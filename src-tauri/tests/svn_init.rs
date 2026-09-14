//! 初始化向导的编排与失败回退（spec §6.5）。
//!
//! **回退是这里的重点。** 初始化会在用户的 SVN 工作副本里创建 `.git`，任何一步
//! 失败都必须删干净——留下半个混合仓库的话，§6.1 会把它识别成 `SvnSuspicious`
//! 并阻断，而用户完全不知道发生了什么。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_baseline::parse_trailer;
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_health::Health;
use app_lib::engine::svn_init::{init_hybrid, rebuild_baseline, InitRequest};
use app_lib::engine::svn_xml::parse_info;
use std::process::Command;
use svn_fixtures::{svn_repo, svn_repo_dirty, svn_repo_for_health, DirtyKind, SvnFixture};

fn svn(dir: &std::path::Path, args: &[&str]) {
    let out = Command::new("svn")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("svn not found");
    assert!(
        out.status.success(),
        "svn {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn req(fx: &SvnFixture) -> InitRequest {
    InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    }
}

/// 服务器当前的 HEAD revision。用来证明初始化一条写命令都没发出去。
fn server_head_rev(fx: &SvnFixture) -> i64 {
    let url = svn_fixtures::file_url(&fx.repo());
    let xml = run_svn(&fx.wc_str(), &["info", "--xml", "--", &url]).unwrap();
    parse_info(&xml).unwrap()[0].revision
}

// ---------------------------------------------------------------------------
// 成功路径
// ---------------------------------------------------------------------------

#[test]
fn a_successful_init_leaves_head_on_work_not_on_the_baseline() {
    // spec §6.5 最后一步：立刻离开基线分支。UI 禁止在基线分支上开发提交。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let head = run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
    assert_eq!(head.trim(), "work");
}

#[test]
fn a_successful_init_records_the_whitelist_content_with_the_right_trailer() {
    let fx = svn_repo();
    let report = init_hybrid(&req(&fx)).unwrap();
    assert_eq!(report.node_rev, 1);

    // 基线树里是 W-file
    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(listed.contains("Code/A.cs"), "{listed}");

    // trailer 的 url 是 **Repository Root**，不是工作副本 URL（spec §6.4.1）
    let msg = run_git(
        &fx.wc_str(),
        &["log", "-1", "--format=%B", "refs/heads/svn/base"],
    )
    .unwrap();
    let t = parse_trailer(&msg).expect("基线必须带 trailer");
    assert_eq!(t.rev, 1);
    assert!(!t.uuid.is_empty());
    assert!(t.url.starts_with("file:///"), "{}", t.url);
}

#[test]
fn a_successful_init_leaves_a_clean_git_status() {
    // spec §6.3.1 要求发布后用 `git read-tree TREE` 对齐真实索引。漏掉这一步的话
    // HEAD 上有文件、索引却是空的，用户一打开就看到每个 W-file 都显示成「已删除」。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let status = run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap();
    assert!(status.trim().is_empty(), "工作区不干净：\n{status}");
}

#[test]
fn a_successful_init_emits_no_svn_write_command() {
    // G8 在本期的延续：整个初始化只读 SVN，服务器侧 revision 必须没变。
    let fx = svn_repo();
    let before = server_head_rev(&fx);
    init_hybrid(&req(&fx)).unwrap();
    assert_eq!(before, server_head_rev(&fx), "初始化动了服务器");
}

#[test]
fn a_successful_init_leaves_no_temp_index_behind() {
    // TempIndex 的 Drop 负责，这里确认它在真实编排里也生效了。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(fx.wc().join(".git"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("mygitclient-") && n.ends_with(".index"))
        .collect();
    assert!(leftovers.is_empty(), "临时索引没清掉：{leftovers:?}");
}

#[test]
fn the_health_check_runs_and_its_result_is_reported() {
    let fx = svn_repo_for_health();
    let report = init_hybrid(&req(&fx)).unwrap();
    assert_eq!(report.health, Health::Verified { rev: 1 });
}

// ---------------------------------------------------------------------------
// 回退
// ---------------------------------------------------------------------------

#[test]
fn a_failure_after_git_init_rolls_the_whole_thing_back() {
    // 用**纳入集合核对不过**来触发：工作副本根放一个 `.gitignore`（`*.cs`），
    // 它的优先级高于我们生成的 `.git/info/exclude`（spec §6.3.1），于是候选索引
    // 一个文件都收不到。
    //
    // ⚠️ 必须放在**根**而不是 `Code/` 底下。放 `Code/.gitignore` 的话它是白名单
    // 目录里一个未版本控制的文件，P1 在 `git init` **之前**就把它拦下了，
    // 走的是「无需回退」那条路——这条测试就什么也没测到。
    //
    // 回退是一条公共路径：录制失败、体检 Lying 走的是同一个 `Err` 分支。
    let fx = svn_repo();
    std::fs::write(fx.wc().join(".gitignore"), "*.cs\n").unwrap();

    let err = init_hybrid(&req(&fx)).expect_err("纳入集合核对必须拦住");
    assert_eq!(err.code, "include_set_mismatch");
    assert!(!fx.wc().join(".git").exists(), "刚建的 .git 没删干净");
    assert!(fx.wc().join("Code/A.cs").exists(), "工作目录文件被删了");
    assert!(fx.wc().join(".svn").is_dir(), ".svn 被动了");
}

#[test]
fn a_preflight_failure_never_creates_a_git_at_all() {
    // P1–P4 全部跑在创建 .git **之前**，所以这一侧根本不需要回退。
    let fx = svn_repo_dirty(DirtyKind::Modified);
    let err = init_hybrid(&req(&fx)).expect_err("脏工作副本必须被拦住");
    assert_eq!(err.code, "svn_preflight_dirty");
    assert!(!fx.wc().join(".git").exists(), "校验没过却建了 .git");
}

#[test]
fn init_refuses_to_touch_a_directory_that_already_has_a_git() {
    // 全项目唯一一处 remove_dir_all，不能依赖上游判对了。
    let fx = svn_repo();
    std::fs::create_dir_all(fx.wc().join(".git")).unwrap();
    std::fs::write(fx.wc().join(".git/PRECIOUS"), "x").unwrap();

    let err = init_hybrid(&req(&fx)).expect_err("已有 .git 必须拒绝");
    assert_eq!(err.code, "svn_unconfigured_git");
    assert!(fx.wc().join(".git/PRECIOUS").exists(), "把别人的 .git 删了");
}

#[test]
fn init_refuses_to_run_twice() {
    // 第二次跑时目录已经是 SvnHybrid。不拦住的话会走到 publish_baseline，
    // 那里 `expected_old = None` 撞上已存在的 ref 才报错——错得太靠后，
    // 而且中途已经把配置和 exclude 覆盖了一遍。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let err = init_hybrid(&req(&fx)).expect_err("第二次必须被拒");
    assert_eq!(err.code, "svn_already_initialized");
    // 第一次的成果原样还在
    assert_eq!(
        run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap()
            .trim(),
        "work"
    );
}

#[test]
fn an_empty_whitelist_is_rejected_before_anything_is_created() {
    // 空白名单有三道拦截（`svn_config::validate`、`run_preflight`、
    // `check_baseline`），最早的一道是配置校验，所以拿到的是 `svn_config`。
    // 这里不钉具体是哪一道——要钉的是「在建 .git 之前就被拒了」。
    let fx = svn_repo();
    let err = init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec![],
        extensions: vec![".cs".to_string()],
    })
    .expect_err("空白名单必须被拒");
    assert_eq!(err.code, "svn_config", "实际：{}", err.message);
    assert!(!fx.wc().join(".git").exists());
}

// ---------------------------------------------------------------------------
// Task 8：重建基线 R1–R5（spec §16）
// ---------------------------------------------------------------------------

/// 一个已初始化、**已经重建过一次**的混合仓库，之后又有人在 TortoiseSVN 里
/// 把它 update 到了 r3。
///
/// ```text
/// init   → 基线 v1@r1，work 也在那个 commit 上，磁盘 v1
/// 外部 update → 磁盘 v2
/// rebuild     → 基线 v2@r2；**work 仍停在 v1 那个 commit**
/// 外部 update → 磁盘 v3
/// ```
///
/// 最终态：磁盘 `v3`、`svn/base` 树是 `v2`、HEAD(`work`) 树是 `v1` —— **三者互不相同**。
///
/// # 为什么非要绕这一圈，以及为什么中间那一步用裸 git
///
/// 第一版 fixture 只做到「init + 一次外部 update」，那时 `work` 与 `svn/base`
/// 指向**同一个 commit**。于是 `git checkout svn/base` 是个空操作，磁盘纹丝不动
/// ——`rebuild_records_the_current_disk_not_the_old_baseline_content` 这条
/// 「反向证据」对它**抓不住**（2026-09-14 变异测试实测：把那条 checkout 插进实现，
/// 只有「HEAD 没动」那条红了）。名为「覆盖 X」却测不到 X 的真实形态属虚假信心
/// （`AGENTS.md` §13 rubric #9）。
///
/// 第二版把中间那次推进写成 `rebuild_baseline(...)`，**还是抓不住**：fixture 调用了
/// 被测函数，变异在搭 fixture 时就先把 HEAD 挪到了 `svn/base`，等测试跑时它已经
/// 自我中和。所以这里改用裸 git 推进基线——**fixture 不能用被测对象搭**，
/// 和 `git_cli` 测试里 `fixture_git` 的道理一样。
fn hybrid_repo_updated_externally() -> SvnFixture {
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();

    let url = svn_fixtures::file_url(&fx.repo());
    svn(fx.path(), &["checkout", &url, "wc2", "--non-interactive"]);
    let wc2 = fx.path().join("wc2");

    for (rev, content) in [("r2", "v2\n"), ("r3", "v3\n")] {
        std::fs::write(wc2.join("Code/A.cs"), content).unwrap();
        svn(&wc2, &["commit", "-m", rev, "--non-interactive"]);
        svn(&fx.wc(), &["update", "--non-interactive"]);
        if rev == "r2" {
            // 中间这次推进让基线走到 v2，而 work 分支留在 v1 —— 三者错开的关键一步
            fixture_advance_baseline(&fx.wc());
        }
    }
    fx
}

/// 用**裸 git 管道命令**把当前磁盘上的 `Code/` 录成一个新的 `svn/base` commit，
/// 全程不碰 HEAD 和真实索引。
///
/// 这是 `rebuild_baseline` 的一个最小复刻，**故意重复**：fixture 必须独立于
/// 被测对象，否则被测对象坏了 fixture 跟着一起坏，断言就失去意义。
fn fixture_advance_baseline(wc: &std::path::Path) {
    let git = |args: &[&str], idx: Option<&std::path::Path>| -> String {
        let mut c = Command::new("git");
        c.current_dir(wc).args(args);
        if let Some(p) = idx {
            c.env("GIT_INDEX_FILE", p);
        }
        let out = c.output().expect("git not found");
        assert!(
            out.status.success(),
            "git {args:?} 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let idx = wc.join(".git/fixture.index");
    let _ = std::fs::remove_file(&idx);
    git(&["add", "-A", "--", "Code"], Some(&idx));
    let tree = git(&["write-tree"], Some(&idx));
    let _ = std::fs::remove_file(&idx);
    let old = git(&["rev-parse", "refs/heads/svn/base"], None);
    let new = git(
        &["commit-tree", &tree, "-p", &old, "-m", "fixture: 推进基线"],
        None,
    );
    git(&["update-ref", "refs/heads/svn/base", &new, &old], None);
}

#[test]
fn rebuild_records_the_current_disk_not_the_old_baseline_content() {
    // **反向证据**：这条证明「不 checkout svn/base」不是偷懒。
    //
    // spec §16 专门写了一段：§9.4 的恢复流程先 checkout 旧基线，是因为它后面紧跟着
    // `svn revert` 把服务器内容重新铺回磁盘。重建**没有 revert**，先 checkout 会把
    // 磁盘换回旧基线内容，随后录下来的就是旧内容却标着新 revision
    // ——正好造出这一节要防的那种撒谎基线。
    //
    // 实现里若混进 `git checkout svn/base`，这条会拿到 v1 的内容配 r2 的 rev。
    let fx = hybrid_repo_updated_externally();
    let report = rebuild_baseline(&fx.wc_str()).unwrap();
    assert_eq!(report.node_rev, 3);
    assert_ne!(report.old_baseline, report.new_baseline);

    let content = run_git(&fx.wc_str(), &["show", "refs/heads/svn/base:Code/A.cs"]).unwrap();
    assert!(content.contains("v3"), "录进去的是旧基线内容：{content}");

    let msg = run_git(
        &fx.wc_str(),
        &["log", "-1", "--format=%B", "refs/heads/svn/base"],
    )
    .unwrap();
    assert_eq!(parse_trailer(&msg).unwrap().rev, 3);
}

#[test]
fn rebuild_never_moves_head_nor_touches_the_index_or_the_worktree() {
    // spec §16：R1–R5 全程不动 HEAD。于是不存在「流程结束忘了切回原分支」，
    // 工作分支上已有的 git 历史也不会被基线的推进带走。
    //
    // ⚠️ 这里的「未提交改动」放在**白名单外**（`Art/scratch.txt`）。放
    // `Code/WIP.cs` 的话它是白名单里一个未版本控制的文件，R1 会直接拒绝重建
    // ——那是 P1 的正确行为，不是 bug，但那样这条测试就跑不到重建本身。
    let fx = hybrid_repo_updated_externally();
    std::fs::write(fx.wc().join("Art/scratch.txt"), "wip").unwrap();

    let head_name = run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
    let head_oid = run_git(&fx.wc_str(), &["rev-parse", "HEAD"]).unwrap();
    let index_before = run_git(&fx.wc_str(), &["ls-files", "-s"]).unwrap();

    rebuild_baseline(&fx.wc_str()).unwrap();

    assert_eq!(
        head_name,
        run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap(),
        "HEAD 换分支了"
    );
    assert_eq!(
        head_oid,
        run_git(&fx.wc_str(), &["rev-parse", "HEAD"]).unwrap(),
        "HEAD 移动了"
    );
    assert_eq!(
        index_before,
        run_git(&fx.wc_str(), &["ls-files", "-s"]).unwrap(),
        "真实索引被动了"
    );
    assert!(
        fx.wc().join("Art/scratch.txt").exists(),
        "白名单外的未提交文件被吃掉了"
    );
    // 磁盘上仍然是 r2 的内容——重建没有把任何东西铺回磁盘
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "v3\n"
    );
}

#[test]
fn the_new_baseline_keeps_the_old_one_as_its_parent() {
    // 基线是一条链，不是一串孤儿 commit：重建后还能从新基线走回旧的，
    // 误操作时 reflog 之外还有一条退路。
    let fx = hybrid_repo_updated_externally();
    let report = rebuild_baseline(&fx.wc_str()).unwrap();
    let parent = run_git(
        &fx.wc_str(),
        &["rev-parse", &format!("{}^", report.new_baseline)],
    )
    .unwrap();
    assert_eq!(parent.trim(), report.old_baseline);
}

#[test]
fn a_failure_before_r4_leaves_the_baseline_pointer_untouched() {
    // R3b 与纳入集合核对都排在 R4（动指针）之前，中止的代价为零。
    // 这里用纳入集合核对触发——在工作副本根放一个 `.gitignore`，
    // 它的优先级高于我们生成的 exclude，候选索引一个文件都收不到。
    let fx = hybrid_repo_updated_externally();
    let before = run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"]).unwrap();
    std::fs::write(fx.wc().join(".gitignore"), "*.cs\n").unwrap();

    let err = rebuild_baseline(&fx.wc_str()).expect_err("核对必须拦住");
    assert_eq!(err.code, "include_set_mismatch");
    assert_eq!(
        before,
        run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"]).unwrap(),
        "指针被动了"
    );
}

#[test]
fn rebuild_refuses_when_the_working_copy_is_dirty() {
    // R1 与 P1 同一套判据：不干净就引导去 TortoiseSVN，**不自动 revert**。
    let fx = hybrid_repo_updated_externally();
    std::fs::write(fx.wc().join("Code/A.cs"), "LOCAL\n").unwrap();
    let err = rebuild_baseline(&fx.wc_str()).expect_err("脏工作副本必须被拒");
    assert_eq!(err.code, "svn_preflight_dirty");
    // 没有自动 revert：用户的改动原样还在
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "LOCAL\n"
    );
}

#[test]
fn rebuild_refuses_on_a_directory_that_was_never_initialized() {
    let fx = svn_repo();
    let err = rebuild_baseline(&fx.wc_str()).expect_err("没建过库就没有基线可重建");
    assert_eq!(err.code, "svn_not_hybrid");
}

#[test]
fn rebuild_emits_no_svn_write_command() {
    // 与初始化同样的约束：整个重建只读 SVN。
    let fx = hybrid_repo_updated_externally();
    let before = server_head_rev(&fx);
    rebuild_baseline(&fx.wc_str()).unwrap();
    assert_eq!(before, server_head_rev(&fx), "重建动了服务器");
}
