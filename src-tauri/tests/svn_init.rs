//! 初始化向导的编排与失败回退（spec §6.5）。
//!
//! **回退是这里的重点。** 初始化会在用户的 SVN 工作副本里创建 `.git`，任何一步
//! 失败都必须删干净——留下半个混合仓库的话，§6.1 会把它识别成 `SvnSuspicious`
//! 并阻断，而用户完全不知道发生了什么。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_attributes::has_eol_rule;
use app_lib::engine::svn_baseline::parse_trailer;
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_health::Health;
use app_lib::engine::svn_init::{
    check_current_baseline, init_hybrid, rebuild_baseline, InitRequest,
};
use app_lib::engine::svn_xml::parse_info;
use std::process::Command;
use svn_fixtures::{
    svn_repo, svn_repo_dirty, svn_repo_for_health, svn_repo_obstructed, svn_repo_with_native_eol,
    svn_repo_with_same_repo_nested_external, DirtyKind, NestedExternal, SvnFixture,
};

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
fn after_init_git_status_is_exactly_the_users_existing_local_svn_changes() {
    // spec §6.5.1 末。基线树是**服务器内容**，磁盘上是用户改过的内容，两者故意
    // 不同；`git read-tree` 对齐索引之后，`git status` 显示的恰好就是用户原有的
    // 那些本地 SVN 修改。旧做法下这里永远是空的（强制要求干净），现在它第一眼
    // 就是对的。
    //
    // 这条同时是 `read-tree` **不能带 `-u`** 的守卫：带上会拿基线内容覆盖磁盘，
    // 把用户的改动抹掉——本期最不能犯的错。
    let fx = svn_repo_for_health();
    let wc = fx.wc();
    std::fs::write(wc.join("Code/A.cs"), "MY WORK\n").unwrap();
    std::fs::write(wc.join("Code/New.cs"), "new file\n").unwrap();
    svn(&wc, &["delete", "Code/Deep/D.cs", "--non-interactive"]);

    init_hybrid(&req(&fx)).unwrap();

    let status = run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap();
    let lines: std::collections::BTreeSet<&str> = status
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        lines,
        std::collections::BTreeSet::from([" M Code/A.cs", " D Code/Deep/D.cs", "?? Code/New.cs"]),
        "git 该显示的正是这三处本地改动。实际：\n{status}"
    );
    // 内容必须原样——一个字节都不许被基线覆盖
    assert_eq!(
        std::fs::read_to_string(wc.join("Code/A.cs")).unwrap(),
        "MY WORK\n"
    );
    assert!(
        !wc.join("Code/Deep/D.cs").exists(),
        "被删掉的文件不许被复活"
    );
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
    // `unverified` 空：刚录完那条路径走的是 `ExplainedOrigin::JustRecorded`
    // ——那些 blob 是几毫秒前从 `svn cat -r BASE` 写进去的，由构造保证正确。
    assert_eq!(
        report.health,
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
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
    //
    // 用 obstructed 而不是普通的 `M`：spec §16.1 之后普通的脏**不再阻断**
    // （那条门槛让建库和重建在真实开发节奏下永远做不成），`~` 是唯一仍然
    // 阻断的状态，所以它才是现在唯一能触发「preflight 失败」的形态。
    let fx = svn_repo_obstructed();
    let err = init_hybrid(&req(&fx)).expect_err("obstructed 必须被拦住");
    assert_eq!(err.code, "svn_preflight_obstructed");
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

#[test]
fn init_refuses_a_whitelist_that_open_would_later_reject() {
    // **独立 review 的 BL-1。**
    //
    // `SvnGitBackend::open` 每次打开混合仓库都跑 `validate_whitelist_against_disk`，
    // 而 `init_hybrid` 是唯一**写入**配置的地方。两边不接上的话，向导能建出一个
    // 自己打不开的仓库：
    //
    //   `Code`（漏尾斜杠，很自然的手误）+ 目录里恰好全是匹配扩展名的文件
    //   → 期望集合与候选索引相等 → 纳入集合核对通过 → 初始化成功 → 配置落盘
    //   → 下一次 open_repo 被拒 → 此刻目录已是 SvnHybrid，向导那条分支只认
    //     `svn_not_initialized`，用户在应用内**没有任何出路**。
    //
    // `tests/svn_config.rs` 早就断言了 `["Code"]` 必被那个校验拒绝，
    // 缺的只是把两条路径接上。
    let fx = svn_repo(); // Code/ 底下只有 A.cs 与 B.cs，全部匹配 .cs
    let err = init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code".to_string()], // ← 漏了尾斜杠
        extensions: vec![".cs".to_string()],
    })
    .expect_err("漏尾斜杠的目录条目必须在建库前就被拒");
    assert_eq!(err.code, "svn_config");
    assert!(!fx.wc().join(".git").exists(), "校验没过却建了 .git");
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
fn init_succeeds_on_a_clean_same_repo_nested_external_and_never_tracks_its_dot_svn() {
    // P1/P2 现在会把嵌套 external 一并查过，于是这条路径**第一次真的走得通**，
    // 也第一次把两件事暴露在同一个流程里：
    //
    // ① 嵌套 external 有**自己的 `.svn`**（`Code/Vendor/.svn/wc.db`）。
    //    2026-09-15 实测，裸 `git add -- Code` 会把它整个收进来。一旦 `wc.db`
    //    被 git 跟踪，任何一次 `git checkout` 都会在一个**正在使用的** SVN 工作
    //    副本底下把它换成另一个版本——那不是恢复旧数据，是直接损坏工作副本。
    //    挡住它的是 `generate_exclude` 末尾那行不带前导 `/` 的 `.svn/`。
    // ② 体检覆盖不到 external，所以结论必须是 NotChecked 而**不是** Verified。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    let report = init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .expect("干净的同仓库嵌套 external 必须能建库");

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(
        listed.contains("Code/Vendor/V.cs"),
        "external 里的源码本来就该进基线：{listed}"
    );
    assert!(
        !listed.contains(".svn"),
        "把 external 自己的 .svn 录进基线了，checkout 会损坏工作副本：{listed}"
    );

    // **Phase 1c 把这条的期望从 NotChecked 改成了 Verified。**
    //
    // 1b 时体检从项目根只跑一次 `svn diff`，而 `--old` 那侧是 URL，URL 上没有
    // `Code/Vendor` 这个节点，diff 不下沉进去——一块被 git 收进基线的内容从没被
    // 比对过，所以当时只能降级成 NotChecked，不宣称健康。
    //
    // 现在按 spec §6.6.5 改成**每个记账单位各发一条命令、各用自己的 `url(P)`**，
    // 这块内容第一次真的被服务器验过了。干净就该说干净。
    match report.health {
        Health::Verified { .. } => {}
        other => panic!("嵌套 external 现在应当验得过，实际：{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 并发初始化：回退只许删自己建的 .git
// ---------------------------------------------------------------------------

#[test]
fn two_concurrent_inits_never_delete_the_winner_s_repository() {
    // **这条针对的是 remove_dir_all 那处的所有权问题**，不是一般的并发整洁度。
    //
    // 2026-09-15 实测的交错（用裸 git 复现过每一步）：
    //
    // ```text
    // T1 判 .git 不存在 ─┐
    // T2 判 .git 不存在 ─┘   两条都过，因为「判断」与「创建」是两次系统调用
    // T2 git init            建出 .git
    // T1 git init            **exit 0**，静默 re-init（还会 ignored --initial-branch）
    // T2 publish_baseline    抢到 refs/heads/svn/base
    // T1 publish_baseline    update-ref 带「该 ref 不存在」前置条件 → exit 128
    // T1 rollback            remove_dir_all(.git) ← 删掉的是 T2 的仓库
    // ```
    //
    // 于是 T2 拿到一份「初始化成功」的报告，而磁盘上 .git 已经没了。
    //
    // 2026-09-15 用一个临时探针跑 6 轮，修复前实测命中率约 1/3，长这样：
    //
    // ```text
    // round 2: ok=0 gitdir=false hybrid=SvnUninitialized
    //     Err("git_cli / fatal: not a git repository …")
    //     Err("git_cli / could not lock config file …/.git/config: File exists")
    // ```
    //
    // 两条线程**同时**在同一个 .git 上跑 git init，撞在 config 锁上双双失败，
    // 于是两边都回退，目录被删回 SvnUninitialized。
    //
    // 断言必须是 **`ok == 1`**，不能写成 `ok <= 1`：上面那种「两条都失败」正好让
    // `ok <= 1` 通过，而它恰恰是 bug 的现场。正确实现下这个数恒为 1——抢到的那条
    // 往下走，没抢到的那条在**动任何东西之前**就退出。
    //
    // ⚠️ **这条测试靠撞窗口，轮数是量出来的，不是拍的。** 2026-09-15 把实现改回
    // 修复前的原样（只判存在、`.git` 交给 `git init` 去建）做变异测试：
    //
    // | 轮数 | 变异后的结果 |
    // |---|---|
    // | 6  | 跑 3 次，**2 次**变红（另一次没撞上窗口，假绿） |
    // | 12 | 跑 4 次，**4 次**全红，两条断言都触发过 |
    //
    // 6 轮会漏，所以定 12。修复后同样 12 轮跑 3 次全绿，没有反向的抖动。
    for round in 0..12 {
        let fx = svn_repo();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let b = std::sync::Arc::clone(&barrier);
            let r = req(&fx);
            handles.push(std::thread::spawn(move || {
                b.wait();
                init_hybrid(&r).map_err(|e| format!("{} / {}", e.code, e.message))
            }));
        }
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        let ok = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(ok, 1, "第 {round} 轮：必须恰好一条成功，实际 {results:?}");

        // 落败的那条必须是**被所有权挡下**的，而不是一头撞进 git 管道里才失败。
        // 撞进去意味着它已经在动那个不属于它的仓库了。
        let err = results.iter().find_map(|r| r.as_ref().err()).unwrap();
        assert!(
            err.starts_with("svn_unconfigured_git") || err.starts_with("svn_already_initialized"),
            "第 {round} 轮：落败方应当在占有检查处干净退出，实际：{err}"
        );

        let git = fx.wc().join(".git");
        assert!(
            git.is_dir(),
            "第 {round} 轮：有一条请求报了成功，但 .git 被另一条的回退删掉了"
        );
        // 光有目录不够——被 remove_dir_all 删完再由失败方 git init 重建的残骸
        // 同样是个目录。要求它真的是一个可用的混合仓库。
        assert_eq!(
            app_lib::vcs::detect_repo_kind(&fx.wc_str()),
            app_lib::vcs::RepoKind::SvnHybrid,
            "第 {round} 轮：.git 还在，但已经不是一个完好的混合仓库"
        );
        let head = run_git(&fx.wc_str(), &["symbolic-ref", "--short", "HEAD"]).unwrap();
        assert_eq!(head.trim(), "work", "第 {round} 轮");
    }
}

// ---------------------------------------------------------------------------
// 手动体检：它读的是基线自己的 trailer rev（spec §6.8.5 那张表的最后一行）
// ---------------------------------------------------------------------------

/// 用**裸 git 管道命令**把当前磁盘上的 `Code/` 录成一个新的 `svn/base`，
/// trailer 的 rev 由调用方指定——用来造「基线自称 rN、内容其实是别的 revision」。
///
/// 和 [`fixture_advance_baseline`] 一样故意不调用被测代码：trailer 文本在这里
/// 手写，不走 `svn_baseline::format_trailer`。
fn fixture_record_baseline_claiming(wc: &std::path::Path, rev: i64) {
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
    let xml = run_svn(&wc.to_string_lossy(), &["info", "--xml"]).unwrap();
    let info = &parse_info(&xml).unwrap()[0];

    let idx = wc.join(".git/fixture2.index");
    let _ = std::fs::remove_file(&idx);
    git(&["add", "-A", "--", "Code"], Some(&idx));
    let tree = git(&["write-tree"], Some(&idx));
    let _ = std::fs::remove_file(&idx);

    let old = git(&["rev-parse", "refs/heads/svn/base"], None);
    let msg = format!(
        "svn: r{rev}\n\ngit-svn-id: {}@{rev} {}\n",
        info.repos_root, info.uuid
    );
    let new = git(&["commit-tree", &tree, "-p", &old, "-m", &msg], None);
    git(&["update-ref", "refs/heads/svn/base", &new, &old], None);
}

#[test]
fn the_manual_health_check_verifies_a_good_baseline_despite_ordinary_wip() {
    // 用户就在 work 分支上改代码——这是「体检基线」最常见的调用时机。
    //
    // 旧实现在这里返回 `NotChecked`（前提「磁盘 == 基线树」不成立）。那和
    // spec §16.1 里那条「工作副本必须干净」是同一个病：程序员的工作区 99% 的时间
    // 都是脏的，于是这颗按钮永远只会回答「没法下结论」，等于不存在。
    //
    // §6.8.7 的 `D \ S == ∅` 让它重新可用：这处改动进 `S`，被解释掉；其余文件
    // 与服务器逐一对得上 → **Verified**。
    //
    // 这条测试原本的用意（别把普通 WIP 指控成假基线）没有变松，反而更强了：
    // 从前只要求「别说它坏」，现在要求「能给出一个肯定的答案」。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();

    std::fs::write(fx.wc().join("Code/A.cs"), "work in progress\n").unwrap();

    let got = check_current_baseline(&fx.wc_str()).unwrap();

    assert!(
        matches!(got, Health::Verified { .. }),
        "普通 WIP 不该妨碍体检下结论，实际：{got:?}"
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "work in progress\n",
        "体检是只读的，用户的改动必须原样还在"
    );
}

#[test]
fn the_manual_health_check_still_catches_a_baseline_whose_rev_is_stale() {
    // **反向证据。** 上一条要求「磁盘和基线树不一致时别下结论」，单独看它可以被
    // 「永远返回 NotChecked」满足——那等于把 §6.8 整条检查删掉。
    //
    // 这里造的是它真正要抓的形态：磁盘与基线树**一致**（前提成立），但基线的
    // trailer 自称 r1，而那份内容其实是 r2 的。这正是 §6.8.1「让基线可被服务器
    // 证伪」的目标。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();

    let url = svn_fixtures::file_url(&fx.repo());
    svn(fx.path(), &["checkout", &url, "wc2", "--non-interactive"]);
    let wc2 = fx.path().join("wc2");
    std::fs::write(wc2.join("Code/A.cs"), "v2\n").unwrap();
    svn(&wc2, &["commit", "-m", "r2", "--non-interactive"]);
    svn(&fx.wc(), &["update", "--non-interactive"]);

    // 磁盘现在是 r2 的内容；把基线树对齐到它，但 trailer 仍写 r1
    fixture_record_baseline_claiming(&fx.wc(), 1);

    match check_current_baseline(&fx.wc_str()).unwrap() {
        Health::Lying { files } => {
            assert!(files.contains(&"Code/A.cs".to_string()), "实际：{files:?}")
        }
        other => panic!("基线自称的 revision 已经过时，必须报 Lying，实际：{other:?}"),
    }
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

/// 某个路径在 `svn/base` 树里的 blob 字节。
fn baseline_blob(wc: &str, path: &str) -> Vec<u8> {
    app_lib::engine::git_cli::run_git_bytes(
        wc,
        &["cat-file", "blob", &format!("refs/heads/svn/base:{path}")],
    )
    .unwrap()
}

/// 白名单子树里每个文件的 (相对路径, 字节)。用来证明磁盘没被动过。
fn snapshot_disk(wc: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
        for e in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let p = e.path();
            if p.file_name().is_some_and(|n| n == ".svn") {
                continue;
            }
            if p.is_dir() {
                walk(&p, base, out);
            } else {
                let rel = p
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(&wc.join("Code"), wc, &mut out);
    out.sort();
    out
}

#[test]
fn rebuild_records_server_content_on_a_dirty_working_copy() {
    // **spec §16.1 的核心断言**，也是这一期存在的全部理由。
    //
    // 造齐六种真实的脏状态，重建之后逐条验：基线树里是**服务器 rN 的内容**，
    // 而不是磁盘上用户改过的内容；`A` / `?` 这两种服务器上没有的不在树里。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();
    let wc = fx.wc();

    // M：改过内容
    std::fs::write(wc.join("Code/A.cs"), "LOCAL EDIT\n").unwrap();
    // ?：未版本控制
    std::fs::write(wc.join("Code/Untracked.cs"), "brand new\n").unwrap();
    // A：svn add 了没提交
    std::fs::write(wc.join("Code/Added.cs"), "added\n").unwrap();
    svn(&wc, &["add", "Code/Added.cs", "--non-interactive"]);
    // D：svn delete 了没提交
    svn(&wc, &["delete", "Code/Deep/D.cs", "--non-interactive"]);
    // 非白名单扩展名的脏（`Code/Art.psd`），它既不该进树，也不该进 `S`
    std::fs::write(wc.join("Code/Art.psd"), "LOCAL PSD\n").unwrap();

    let before = snapshot_disk(&wc);
    let git_status_before = run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap();

    let report = rebuild_baseline(&fx.wc_str()).expect("脏工作副本必须能重建");

    assert_eq!(report.node_rev, 1);
    assert_eq!(
        baseline_blob(&fx.wc_str(), "Code/A.cs"),
        b"v1\n",
        "M：基线里必须是服务器 r1 的内容，不是磁盘上那份 LOCAL EDIT"
    );
    assert_eq!(
        baseline_blob(&fx.wc_str(), "Code/Deep/D.cs"),
        b"d1\n",
        "D：本地删掉的文件仍然要以服务器版本留在基线里——本地删除是一处待交付的\
         改动，不是基线的一部分"
    );
    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(
        !listed.contains("Code/Added.cs"),
        "A：服务器 r1 上没有它，不该进基线。实际树：\n{listed}"
    );
    assert!(
        !listed.contains("Code/Untracked.cs"),
        "?：同上。实际树：\n{listed}"
    );
    assert!(
        !listed.contains("Code/Art.psd"),
        "扩展名不在白名单里，本来就不归 git 管。实际树：\n{listed}"
    );

    // **磁盘一个字节都不许动。** 这是整条流程对工作目录只读的证明，
    // 也是它与 `svn revert` 的本质区别（spec §16.1）。
    assert_eq!(before, snapshot_disk(&wc), "重建动了磁盘");
    assert_eq!(
        git_status_before,
        run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap(),
        "重建改变了 git 工作区状态"
    );
}

#[test]
fn recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not() {
    // **同一个文件在基线里长什么样，不该取决于它当时脏不脏。**
    //
    // 两条录制路径不一样：干净文件走 `git add -A`（会跑 autocrlf 等过滤器），
    // 脏文件走 `hash-object --stdin`（默认**不跑**）。2026-09-17 实测，本机
    // `core.autocrlf=true` 时一个 `svn:eol-style=native` 的文件：
    //
    //   磁盘 = "n1\r\n" → git add 得 "n1\n"，hash-object --stdin 得 "n1\r\n"
    //
    // 不对齐的话，在脏工作副本上重建一次就会把行尾整个翻一遍，工作分支上凭空
    // 出现「每一行都改了」的假 diff。修法是给 hash-object 加 `--path`。
    let fx = svn_repo_with_native_eol();
    init_hybrid(&req(&fx)).unwrap();

    // 干净时录的（走 git add -A）
    let clean_nat = baseline_blob(&fx.wc_str(), "Code/Nat.cs");
    let clean_plain = baseline_blob(&fx.wc_str(), "Code/Plain.cs");

    // 改脏之后重建（走 hash-object）
    std::fs::write(fx.wc().join("Code/Nat.cs"), "LOCAL\r\n").unwrap();
    std::fs::write(fx.wc().join("Code/Plain.cs"), "LOCAL\n").unwrap();
    rebuild_baseline(&fx.wc_str()).unwrap();

    assert_eq!(
        baseline_blob(&fx.wc_str(), "Code/Nat.cs"),
        clean_nat,
        "svn:eol-style=native 的文件：两条录制路径产出了不同的 blob"
    );
    assert_eq!(
        baseline_blob(&fx.wc_str(), "Code/Plain.cs"),
        clean_plain,
        "无 eol-style 的文件：两条录制路径产出了不同的 blob"
    );
}

#[test]
fn rebuild_refuses_only_when_a_node_is_obstructed() {
    // R1 与 P1 同一套判据。§16.1 之后那套判据只剩 `~` 一条——普通的本地修改
    // **必须放行**（见 `rebuild_records_server_content_on_a_dirty_working_copy`）。
    //
    // 这条保留下来的核心断言是**不自动修复**：拦住之后磁盘必须原样。删目录、
    // revert 都是破坏性的，本应用一律不替用户做（spec §6.5 P1 / §8.1 #2）。
    let fx = hybrid_repo_updated_externally();
    let obstructed = fx.wc().join("Code/A.cs");
    std::fs::remove_file(&obstructed).unwrap();
    std::fs::create_dir(&obstructed).unwrap();

    let err = rebuild_baseline(&fx.wc_str()).expect_err("obstructed 必须被拒");

    assert_eq!(
        err.code, "svn_preflight_obstructed",
        "实际：{}",
        err.message
    );
    assert!(obstructed.is_dir(), "拦住之后磁盘必须原样，不许替用户动手");
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

// ===========================================================================
// 目录形态的脏状态（2026-09-20 独立 review 的两条阻断项 B1 / B2）
//
// `svn status` 对 unversioned / ignored / deleted 的**目录**只报目录本身。
// 实测（`-v` 也一样不下沉进未版本控制目录）：
//
// ```xml
// <entry path="Code\NewDir"><wc-status item="unversioned" props="none"/></entry>
// ```
//
// 而 git 的 exclude 规则按扩展名**递归**收。两边粒度不一致，就是一个假基线。
// ===========================================================================

#[test]
fn an_unversioned_directory_does_not_smuggle_its_cs_files_into_the_baseline() {
    // **B1。** 这是本期最危险的一条：它静默。四道检查全绿，而基线树里躺着
    // 服务器 r1 上根本不存在的文件，内容是磁盘内容。按 §8.5 / D1，这批文件
    // 第一次交付时会因为「和基线一样」而不在净变更里——静默漏交付。
    //
    // Unity 工程里「新建个文件夹写几个 .cs、还没 svn add」是最常见的状态之一。
    let fx = svn_repo_dirty(DirtyKind::UnversionedDir);
    let report = init_hybrid(&req(&fx)).unwrap();

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(
        !listed.contains("Code/NewDir/New.cs"),
        "未版本控制目录里的 .cs 进了基线树。实际树：\n{listed}"
    );
    assert!(
        !listed.contains("Code/NewDir/Deeper/Deep.cs"),
        "深一层的更容易漏。实际树：\n{listed}"
    );
    assert!(
        listed.contains("Code/A.cs"),
        "服务器上本来就有的必须还在。实际树：\n{listed}"
    );
    assert!(
        matches!(report.health, Health::Verified { .. }),
        "剔干净之后体检该通过，实际：{:?}",
        report.health
    );
    // 磁盘只读：用户的新文件一个都不能少
    assert!(fx.wc().join("Code/NewDir/New.cs").exists());
    assert!(fx.wc().join("Code/NewDir/Deeper/Deep.cs").exists());
}

#[test]
fn an_svn_ignored_directory_is_handled_the_same_way() {
    // `--no-ignore` 让它出现在 status 里，item 是 `ignored` 而不是 `unversioned`。
    // 被 svn:ignore 挡住**不代表 git 不收它**，所以两条分支都得走同一条路。
    let fx = svn_repo_dirty(DirtyKind::IgnoredDir);
    init_hybrid(&req(&fx)).unwrap();

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(
        !listed.contains("Code/Ign/G.cs"),
        "svn:ignore 的目录里的 .cs 进了基线树。实际树：\n{listed}"
    );
}

#[test]
fn a_directory_deleted_with_svn_delete_is_taken_back_from_the_server() {
    // **B2。** 不静默，但它让一个完全合法的 SVN 状态变成死胡同：
    // 子文件没进 `S`，而 `svn diff --summarize` 那一侧**会**报它是 deleted，
    // 于是 D \ S != ∅ → Lying → 初始化整个回退，错误文案还让用户去跑
    // `svn update`（跑一百次也没用）。
    let fx = svn_repo_dirty(DirtyKind::DeletedDir);
    let report = init_hybrid(&req(&fx)).unwrap();

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(
        listed.contains("Code/Deep/D.cs"),
        "被本地删掉的目录里的文件，服务器上还在，必须取回基线。实际树：\n{listed}"
    );
    let blob = run_git(
        &fx.wc_str(),
        &["cat-file", "blob", "refs/heads/svn/base:Code/Deep/D.cs"],
    )
    .unwrap();
    assert_eq!(blob, "d\n", "取回的必须是服务器那一版");
    assert!(
        matches!(report.health, Health::Verified { .. }),
        "取回之后体检该通过，实际：{:?}",
        report.health
    );
    // 磁盘只读：删掉的目录不能被悄悄恢复
    assert!(
        !fx.wc().join("Code/Deep").exists(),
        "重建/初始化不许写磁盘，被删的目录不该回来"
    );
}

#[test]
fn all_three_directory_shapes_at_once_still_record_correctly() {
    // E2E 场景 13 的**完整形态**：三种目录级的脏同时存在。
    //
    // 单独各测一遍不等于测了它们的组合——`read_dirty` 对每个条目分别判断，
    // 一条走 `-v` 那条路（deleted），两条走磁盘遍历那条路（unversioned /
    // ignored），而 `Code` 本身还因为 `svn:ignore` 变成了 `props="modified"`。
    // 手工清单要照着做的就是这个形态，所以这里先把它钉住。
    let fx = svn_repo_dirty(DirtyKind::DeletedDir);
    let wc = fx.wc();

    // ① 未纳入版本控制的目录，两层
    std::fs::create_dir_all(wc.join("Code/NewDir/Deeper")).unwrap();
    std::fs::write(wc.join("Code/NewDir/New.cs"), "class N {}").unwrap();
    std::fs::write(wc.join("Code/NewDir/Deeper/Deep.cs"), "class P {}").unwrap();
    // ② svn:ignore 掉的目录
    std::fs::create_dir_all(wc.join("Code/Ign")).unwrap();
    std::fs::write(wc.join("Code/Ign/G.cs"), "class G {}").unwrap();
    svn(
        &wc,
        &["propset", "svn:ignore", "Ign", "Code", "--non-interactive"],
    );
    // ③ 被 svn delete 的目录由 fixture 提供（Code/Deep）

    let report = init_hybrid(&req(&fx)).unwrap();

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    for gone in [
        "Code/NewDir/New.cs",
        "Code/NewDir/Deeper/Deep.cs",
        "Code/Ign/G.cs",
    ] {
        assert!(
            !listed.contains(gone),
            "`{gone}` 服务器上不存在，不该进基线树。实际树：\n{listed}"
        );
    }
    assert!(
        listed.contains("Code/Deep/D.cs"),
        "被本地删掉的目录里的文件，服务器上还在，必须取回。实际树：\n{listed}"
    );
    assert!(
        matches!(report.health, Health::Verified { .. }),
        "三种形态同时存在时体检仍该通过，实际：{:?}",
        report.health
    );

    // 只读：用户的东西一件都不能动
    assert!(wc.join("Code/NewDir/New.cs").exists());
    assert!(wc.join("Code/Ign/G.cs").exists());
    assert!(!wc.join("Code/Deep").exists(), "被删的目录不许被恢复");
}

// ===========================================================================
// junction（Windows 目录联接）—— 上面那三种形态的第四种，2026-09-21 独立 review
//
// 与它们同一个病根：**svn 按目录报、git 按扩展名递归收**，只是这一次 svn 连
// 「这是个目录」都不算数——2026-09-21 实测：
//
// | 量的是什么 | 结果 |
// |---|---|
// | `svn status --no-ignore -v -- Code`（`Code/Mirror` 是 junction） | 只有 `?  Code\Mirror` 一行 |
// | Rust `symlink_metadata("Code/Mirror").is_dir()` | **`false`**（`is_symlink()` 为 `true`） |
// | `git add -A -- Code/` | 索引里出现 **`Code/Mirror/Linked.cs`** |
//
// 于是 `is_plain_dir` 返回 false ⇒ `expand_dir` 根本不会被调用 ⇒ 一条 `Fix::Drop`
// 都没有 ⇒ 服务器上不存在的文件留在一棵自称「等于服务器 rN」的树里。
//
// 四道检查一道都不响：`?` 不阻断；`expected_set` 的 `walk` **故意跟随 junction**
// （与 git 对齐），两边集合一致；`remove_path` 摘的是 `Code/Mirror` 这一条、不带
// `-r`，对 `Code/Mirror/Linked.cs` 是 no-op；体检比的是 svn 看得见的差异，而 svn
// 看不见它。服务器侧核对只报「服务器有、本地没有」那一个方向，反方向不报。
//
// `svn_dirty.rs` 自己的注释写着「Unity 工程里 junction 并不罕见」。
// ===========================================================================

#[cfg(windows)]
#[test]
fn a_junction_in_the_whitelist_does_not_smuggle_foreign_cs_files_into_the_baseline() {
    // 最容易触发的形态：junction **自己就是**那条未版本控制的条目。
    let fx = svn_repo();
    let outside = tempfile::TempDir::new().unwrap();
    std::fs::write(outside.path().join("Linked.cs"), "class Linked {}").unwrap();
    let link = fx.wc().join("Code").join("Mirror");
    if !svn_fixtures::make_junction(&link, outside.path()) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }

    let report = init_hybrid(&req(&fx)).unwrap();

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(
        !listed.contains("Code/Mirror/Linked.cs"),
        "junction 里的 .cs 服务器上不存在，不该进基线树。实际树：\n{listed}"
    );
    assert!(
        listed.contains("Code/A.cs"),
        "服务器上本来就有的必须还在。实际树：\n{listed}"
    );
    assert!(
        matches!(report.health, Health::Verified { .. }),
        "剔干净之后体检该通过，实际：{:?}",
        report.health
    );
    // 只读：链接和它指向的内容一个字节都不能动
    assert!(link.join("Linked.cs").exists());

    // 拆掉 junction 再让 TempDir 清理，否则 remove_dir_all 会顺着它走出去
    std::fs::remove_dir(&link).unwrap();
}

#[cfg(windows)]
#[test]
fn a_junction_nested_inside_an_unversioned_directory_is_walked_too() {
    // 另一半：外层是普通的未版本控制目录（`is_plain_dir` 为 true，会进
    // `expand_dir`），junction 藏在里面。遍历到它时若直接跳过，同样漏。
    let fx = svn_repo();
    let outside = tempfile::TempDir::new().unwrap();
    std::fs::write(outside.path().join("Linked.cs"), "class Linked {}").unwrap();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code/NewDir")).unwrap();
    std::fs::write(wc.join("Code/NewDir/New.cs"), "class N {}").unwrap();
    let link = wc.join("Code").join("NewDir").join("Mirror");
    if !svn_fixtures::make_junction(&link, outside.path()) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }

    init_hybrid(&req(&fx)).unwrap();

    let listed = run_git(
        &fx.wc_str(),
        &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
    )
    .unwrap();
    for gone in ["Code/NewDir/New.cs", "Code/NewDir/Mirror/Linked.cs"] {
        assert!(
            !listed.contains(gone),
            "`{gone}` 服务器上不存在，不该进基线树。实际树：\n{listed}"
        );
    }

    std::fs::remove_dir(&link).unwrap();
}

#[cfg(windows)]
#[test]
fn a_junction_loop_never_yields_a_baseline_containing_the_loop() {
    // 跟随 junction 就必须自带环保护——遍历转不出来在 Tauri 里就是整个应用凭空
    // 消失（`svn_include_check.rs` 的 `walk` 记着同一条）。
    //
    // ⚠️ **这条不断言具体结局，因为结局不稳定。** 2026-09-21 在同一份 fixture 上
    // 反复跑，多数次是「成功，基线树里只有服务器那两个文件」，也见过一次
    // `include_set_mismatch` 阻断——差别在 `git add` 那一侧：它跟着环一路递归，
    // 撞上 Windows 重解析点 63 层的上限才停（独立探针实测：65 个索引条目，
    // 最后一条是 63 层深的 `Loop/…/New.cs`），而它到底走多深并不稳定。
    //
    // `svn_include_check.rs` 早就写明：环存在时本侧与 git 给出不同集合、**报一次
    // 不一致是正确行为**。所以这里只钉两件**两种结局都必须成立**的事：
    // 会返回（不挂死），以及绝不产出一棵含环形路径的基线。
    //
    // 遍历本身的终止性由 `tests/svn_dirty.rs` 那条直接打在 `read_dirty` 上的
    // 用例钉住——那一条不受 git 行为影响，是确定性的。
    let fx = svn_repo();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code/NewDir")).unwrap();
    std::fs::write(wc.join("Code/NewDir/New.cs"), "class N {}").unwrap();
    let link = wc.join("Code").join("NewDir").join("Loop");
    if !svn_fixtures::make_junction(&link, &wc.join("Code").join("NewDir")) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }

    match init_hybrid(&req(&fx)) {
        Ok(_) => {
            let listed = run_git(
                &fx.wc_str(),
                &["ls-tree", "-r", "--name-only", "refs/heads/svn/base"],
            )
            .unwrap();
            assert!(
                !listed.contains("Loop"),
                "基线树里出现了环形路径。实际树：\n{listed}"
            );
            assert!(
                listed.contains("Code/A.cs"),
                "服务器上本来就有的必须还在。实际树：\n{listed}"
            );
        }
        Err(e) => {
            assert!(!wc.join(".git").exists(), "阻断之后 .git 没删干净：{e:?}");
        }
    }
    assert!(wc.join("Code/NewDir/New.cs").exists(), "用户的文件被删了");

    std::fs::remove_dir(&link).unwrap();
}

// ---------------------------------------------------------------------------
// 行尾归一化（spec §7.3.0）
// ---------------------------------------------------------------------------

#[test]
fn a_new_repo_records_the_baseline_byte_for_byte() {
    // spec §7.3.0：建库时就得装上规则，此后录的基线天然是对的。
    //
    // `svn:eol-style=native` 的文件在 Windows 上磁盘是 CRLF。装规则之前
    // `git add` 会把它归一成 LF 录进基线；装了之后录的必须是磁盘上那些字节。
    //
    // 这不是洁癖：基线与磁盘不同形，意味着任何一次 `git checkout svn/base`
    // 都会把磁盘的行尾翻一遍，那个文件对 svn 立刻变脏。
    let fx = svn_repo_with_native_eol();
    init_hybrid(&req(&fx)).unwrap();

    let disk = std::fs::read(fx.wc().join("Code/Nat.cs")).unwrap();
    assert!(
        disk.windows(2).any(|w| w == b"\r\n"),
        "前提不成立：native 的文件在 Windows 上该是 CRLF"
    );

    assert_eq!(
        baseline_blob(&fx.wc_str(), "Code/Nat.cs"),
        disk,
        "基线里的字节必须与磁盘完全一致"
    );
}

#[test]
fn a_new_repo_has_the_eol_rule_installed() {
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    assert!(
        has_eol_rule(&fx.wc_str()),
        "建库必须装上 EOL 规则，否则录出来的是旧形状的基线"
    );
}

#[test]
fn a_new_repo_is_clean_on_both_sides_right_after_init() {
    // 规则生效之后最容易砸的就是这条：blob 形状变了，索引对齐要是没跟上，
    // 用户一打开就看到满屏 M。既有的
    // `a_successful_init_leaves_a_clean_git_status` 用的是普通 fixture，
    // 那里磁盘本来就是 LF，看不到行尾问题；这条专挑 native eol 的形态，
    // 并且**两侧都查**——svn 那一侧此前没人查过。
    let fx = svn_repo_with_native_eol();
    init_hybrid(&req(&fx)).unwrap();

    let git_dirty = run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap();
    assert!(
        git_dirty.trim().is_empty(),
        "建完库 git 工作区必须干净，实际：\n{git_dirty}"
    );

    // `?  .git` 是**常态**，必须排除：初始化一条 svn 写命令都不发
    // （`a_successful_init_emits_no_svn_write_command` 钉着这条），所以没机会
    // 设 `svn:ignore`，svn 永远会把 `.git` 报成未版本控制。把它当脏会让这条
    // 断言永远红，而真正要看的是**白名单内有没有文件变脏**。
    let svn_out = run_svn(&fx.wc_str(), &["status"]).unwrap();
    let svn_dirty: Vec<&str> = svn_out
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty() && !l.ends_with(".git"))
        .collect();
    assert!(
        svn_dirty.is_empty(),
        "建完库 svn 侧除 .git 外必须干净，实际：\n{}",
        svn_dirty.join("\n")
    );
}
