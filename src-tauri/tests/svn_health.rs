//! §6.8 基线体检：让基线可被服务器证伪。
//!
//! 场景编号沿用 spec §6.8.2 的 T1–T7，2026-09-14 在临时 fixture 上逐条复现过。
//!
//! 这里最重要的不是 T2/T4 那几条「抓得住」，而是 T3 / T3b 两条「不误报」——
//! 误报会让一个完全正常的仓库**永远建不起来**，而漏报至少还有别的信号兜着。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_basemap::BaseMap;
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_health::{
    check_baseline, check_baseline_map, plan_diffs, DiffScope, ExplainedOrigin, Health, HealthMap,
    MAX_DIFF_CMDLINE,
};
use app_lib::engine::svn_nodes::{compute_map, read_node_status};
use app_lib::engine::svn_scope::WScope;
use app_lib::engine::svn_tombstone::fault::{self, Point};
use app_lib::engine::svn_tombstone::{confirm_absent, encode_url_path, server_head_rev};
use app_lib::engine::svn_wc::{enumerate_wcs, WcIdentity};
use std::collections::{BTreeMap, BTreeSet};
use svn_fixtures::{
    file_url, svn_cmd, svn_repo_for_health, svn_repo_for_health_at_r2, svn_repo_three_revisions,
    svn_repo_with_cross_repo_nested_external, svn_repo_with_same_repo_nested_external,
    NestedExternal, SvnFixture,
};

fn wl(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn cs() -> Vec<String> {
    vec![".cs".to_string()]
}

/// §6.8.7 的 `S = ∅`。
///
/// 本文件里每一条都跑在**干净**工作副本上，所以什么都不需要解释，判据退化成
/// §6.8.2 的原判据 `D == ∅`。`S ≠ ∅` 的那两组（不误报 / 有牙齿）在
/// `svn_health_dirty.rs` 里成对出现。
fn nothing_explained() -> std::collections::BTreeSet<String> {
    std::collections::BTreeSet::new()
}

fn repo_url(fx: &SvnFixture) -> String {
    file_url(&fx.repo())
}

/// 跑一次体检，白名单固定 `Code/`、扩展名固定 `.cs`。
fn check(fx: &SvnFixture, rev: u64) -> Health {
    check_baseline(
        &fx.wc_str(),
        &repo_url(fx),
        rev,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap()
}

fn lying_files(h: &Health) -> Vec<String> {
    match h {
        Health::Lying { files } => files.clone(),
        other => panic!("期望 Lying，实际 {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T1–T7
// ---------------------------------------------------------------------------

#[test]
fn t1_a_clean_working_copy_at_the_right_revision_is_verified() {
    let fx = svn_repo_for_health();
    assert_eq!(
        check(&fx, 1),
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
}

#[test]
fn t2_a_locally_modified_source_file_is_reported() {
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "LOCAL\n").unwrap();
    assert_eq!(lying_files(&check(&fx, 1)), vec!["Code/A.cs"]);
}

#[test]
fn t3_a_modified_art_file_outside_the_whitelist_does_not_trigger_a_false_alarm() {
    // 只传白名单根。这条是「不误报」那一侧，比 T2 更重要——误报会让一个
    // 正常仓库永远建不起来。
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Art/boss.fbx"), "CHANGED\n").unwrap();
    assert_eq!(
        check(&fx, 1),
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
}

#[test]
fn t3b_a_modified_art_file_inside_the_whitelist_does_not_trigger_a_false_alarm() {
    // 与 T3 不同：`Code/Art.psd` **就在白名单根底下**，2026-09-14 实测它
    // 确实会出现在 `svn diff --summarize` 的输出里。靠扩展名过滤剔除
    // （spec §6.8.4：「否则恢复流程有意保留的 Code/Art.psd 本地修改会让体检误报」）。
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/Art.psd"), "CHANGED\n").unwrap();
    assert_eq!(
        check(&fx, 1),
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
}

#[test]
fn t4_an_outdated_baseline_rev_is_caught() {
    // 磁盘已是 r2，仍拿 baselineRev=1 比 → 必须报 Lying。这正是要抓的那类。
    let fx = svn_repo_for_health_at_r2();
    let files = lying_files(&check(&fx, 1));
    assert!(files.contains(&"Code/A.cs".to_string()), "{files:?}");
}

#[test]
fn t5_the_same_moment_with_the_correct_rev_is_verified() {
    // 与 T4 成对：同一个磁盘状态、只换 rev，结论必须翻过来。
    // 少了这一条，T4 也可能是因为别的原因报的差异。
    let fx = svn_repo_for_health_at_r2();
    assert_eq!(
        check(&fx, 2),
        Health::Verified {
            rev: 2,
            unverified: vec![]
        }
    );
}

#[test]
fn t6_multiple_whitelist_roots_are_accepted() {
    let fx = svn_repo_for_health();
    let got = check_baseline(
        &fx.wc_str(),
        &repo_url(&fx),
        1,
        &WScope::plain(&wl(&["Code/", "Art/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    assert_eq!(
        got,
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
}

#[test]
fn t7_a_file_deleted_on_the_server_is_caught() {
    // 覆盖 §9.4.1 那类漏检：服务器删了文件，本地基线还记着它。
    let fx = svn_repo_for_health_at_r2();
    let files = lying_files(&check(&fx, 1));
    assert!(files.contains(&"Code/Deep/D.cs".to_string()), "{files:?}");
}

// ---------------------------------------------------------------------------
// §6.8.4 的过滤
// ---------------------------------------------------------------------------

#[test]
fn directory_entries_and_property_only_changes_are_filtered_out() {
    // 2026-09-14 实测：T4 那次输出里除了两个文件，还有两条
    // `item="none" props="modified" kind="dir"`（`Code` 与 `Code/Deep`）。
    // 不剔除的话，差异清单里会混进目录，用户看到「Code 这个目录和服务器不符」
    // 完全不知道该怎么办；而且纯属性变更根本不是内容差异。
    let fx = svn_repo_for_health_at_r2();
    let files = lying_files(&check(&fx, 1));
    assert!(!files.contains(&"Code".to_string()), "{files:?}");
    assert!(!files.contains(&"Code/Deep".to_string()), "{files:?}");
    assert_eq!(files.len(), 2, "只该剩两个文件：{files:?}");
}

#[test]
fn the_reported_paths_are_repo_relative_not_urls() {
    // 输出里的 path 是 URL（实测 `file:///…/repo/Code/A.cs`）。不剥前缀的话，
    // 它既对不上白名单也对不上 git 的输出，扩展名过滤更无从谈起。
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "LOCAL\n").unwrap();
    let files = lying_files(&check(&fx, 1));
    assert!(!files[0].contains("://"), "还是 URL：{files:?}");
    assert_eq!(files[0], "Code/A.cs");
}

// ---------------------------------------------------------------------------
// §6.8.6：三种结果必须分开
// ---------------------------------------------------------------------------

#[test]
fn a_command_failure_is_not_reported_as_a_lying_baseline() {
    // spec §6.8.6：把「没能验证」当成「验证不通过」会让离线场景寸步难行，
    // 当成「验证通过」则等于这条检查不存在。
    let fx = svn_repo_for_health();
    let got = check_baseline(
        &fx.wc_str(),
        "file:///definitely/not/here",
        1,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    assert!(matches!(got, Health::NotChecked { .. }), "实际：{got:?}");
}

#[test]
fn a_revision_the_server_does_not_have_is_not_checked_either() {
    // 2026-09-14 实测：rev 大于服务器 HEAD 时 `svn diff` 退出 1，
    // 报 `E160006: No such revision 999`。这同样是「没能验证」，不是基线撒谎
    // ——最可能的成因是 svnUrl 配错了或者指向了另一个仓库。
    let fx = svn_repo_for_health();
    let got = check(&fx, 999);
    assert!(matches!(got, Health::NotChecked { .. }), "实际：{got:?}");
}

#[test]
fn a_failing_command_still_emits_partial_xml_and_must_not_be_parsed_as_success() {
    // 这条守的是一个具体的写法错误。2026-09-14 实测：URL 不可达时 svn **仍然**
    // 往 stdout 打出 `<?xml …?>` 和 `<diff><paths>`，错误只在 stderr 和退出码里。
    // 「解析得出来就算成功」的实现会在断网时得到一个空的 paths 列表，
    // 于是**报告基线健康**——这条检查等于不存在。
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/A.cs"), "LOCAL\n").unwrap();
    // 同一个工作副本，同一处真实差异，只把 URL 指歪
    assert!(matches!(
        check_baseline(
            &fx.wc_str(),
            "file:///definitely/not/here",
            1,
            &WScope::plain(&wl(&["Code/"]), &cs()),
            &nothing_explained(),
            ExplainedOrigin::Unknown,
        )
        .unwrap(),
        Health::NotChecked { .. }
    ));
    // 对照：URL 正确时它确实报得出来，证明上面那条不是因为别的原因静默了
    assert_eq!(lying_files(&check(&fx, 1)), vec!["Code/A.cs"]);
}

#[test]
fn an_empty_whitelist_is_an_error_not_a_clean_bill_of_health() {
    // 不传 target 时 `svn diff` 会拿**整个工作副本**跟服务器比。对一个 Unity
    // 工程那是几个 GB 的美术资产，而且任何一处正常的本地改动都会报成基线撒谎。
    // 返回 Verified 更糟——等于宣布一个空白名单的基线是健康的。
    let fx = svn_repo_for_health();
    let err = check_baseline(
        &fx.wc_str(),
        &repo_url(&fx),
        1,
        &WScope::plain(&[], &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .expect_err("空白名单必须报错");
    assert_eq!(err.code, "svn_health_empty_whitelist");
}

#[test]
fn a_switched_whitelist_subtree_is_not_checked_rather_than_lying() {
    // **独立 review 的 BL-3。** `svn switch` 一个子目录是 SVN 的常规操作，而且
    // 四道前置门槛一个都拦不住它（P1 零条目、P2 单值、P3 干净、P4 同 UUID，
    // XML 里连 switched 属性都没有）。
    //
    // 此时体检的 `--old=<工作副本根 URL>@N` 指向的是 trunk 的内容，磁盘上是分支的
    // 内容，逐条比下去会报出一串 modified —— 一个**完全合法**的工作副本被判成
    // 「基线撒谎」，而 spec §6.8.6 下初始化时 Lying 会触发整个回退，给出的引导
    // （「跑一次完整 svn update」）还永远无效。
    //
    // 正确的结论是「没能验证」：基线本身没问题（它录的是磁盘内容配 nodeRevs），
    // 只是这条检查的基准 URL 不适用。
    use svn_fixtures::svn_repo_with_switched_subtree;
    let fx = svn_repo_with_switched_subtree();
    let wc_url = run_svn(&fx.wc_str(), &["info", "--show-item", "url"]).unwrap();
    let got = check_baseline(
        &fx.wc_str(),
        wc_url.trim(),
        3,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    match got {
        Health::NotChecked { reason } => {
            assert!(reason.contains("switch"), "理由要点出成因：{reason}")
        }
        other => panic!("必须是 NotChecked，实际：{other:?}"),
    }
}

#[test]
fn a_single_file_whitelist_entry_is_not_filtered_out_by_extension() {
    // **独立 review 的 NB-3。** 单文件白名单条目**不过**扩展名过滤
    // （`svn_exclude` 的第 ③ 段与 `expected_set` 都是这么做的，spec §6.3）。
    // 体检这边若照样过滤，就是一个假阴性：那个文件进了基线，服务器上改了它，
    // 体检却报 Verified。
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/Art.psd"), "CHANGED\n").unwrap();
    let got = check_baseline(
        &fx.wc_str(),
        &repo_url(&fx),
        1,
        // 单文件条目；扩展名只有 .cs，psd 不在里面
        &WScope::plain(&wl(&["Code/Art.psd"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    assert_eq!(lying_files(&got), vec!["Code/Art.psd"]);
}

#[test]
fn a_clean_nested_external_is_verified_not_merely_unchecked() {
    // **Phase 1c 把这条的期望从 NotChecked 改成了 Verified。**
    //
    // 1b 的做法是从项目根跑**一次** `svn diff`，而 `--old` 那侧是 URL，URL 上没有
    // `Code/Vendor` 这个节点（它只是 `Code` 上的一条 `svn:externals` 属性），
    // diff 不下沉进去、`<paths>` 是空的；而 `git add -- Code` 把
    // `Code/Vendor/V.cs` 照收进基线。当时只能降级成 NotChecked，不宣称健康。
    //
    // 现在按 spec §6.6.5 的要求，**每个记账单位各发一条命令、各用自己的 url(P)**，
    // 嵌套 external 那块内容第一次真的被服务器验过了 —— 干净就该说干净。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        2,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    assert!(
        matches!(got, Health::Verified { .. }),
        "干净的嵌套 external 现在应当能验过，实际：{got:?}"
    );
}

#[test]
fn one_unit_failing_does_not_silently_verify_the_others() {
    // 按单位分开发命令之后出现的**新失败形态**：某一个单位的 URL 不可达。
    //
    // 「其余的都干净，所以 Verified」是这条检查最坏的结论——`Verified` 不是
    // 「没发现问题」，是一句主动的保证，§6.6 的失同步判定与 §9 的恢复流程都
    // 建立在它之上。一个单位没验成，整体就只能是 NotChecked。
    //
    // 造法：跨仓库嵌套 external，然后把 external 指向的那个仓库从磁盘上删掉。
    // 工作副本仍在（`svn info` 是本地读，照样列得出这个单位），但它的 URL 已经
    // 指向一个不存在的仓库。
    let fx = svn_repo_with_cross_repo_nested_external();
    std::fs::remove_dir_all(fx.path().join("repo2")).unwrap();

    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        2,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    match got {
        Health::NotChecked { reason } => assert!(
            reason.contains("Vendor"),
            "理由要点名是哪个单位没验成：{reason}"
        ),
        other => panic!("一个单位没验成时不能宣称健康，实际：{other:?}"),
    }
}

#[test]
fn a_lying_baseline_inside_a_nested_external_is_caught() {
    // **这条是上一条的反向证据，少了它上一条毫无意义**：
    // 「见到 external 就直接返回 Verified」同样能让上一条变绿，而那会把嵌套
    // external 变成一块无人盯防的盲区 —— 正是 1b 那道降级守卫要防的东西。
    //
    // Dirty 形态：external 里有一个未提交的改动，磁盘与服务器不符。
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Dirty);
    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        2,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    let files = lying_files(&got);
    assert!(
        files.iter().any(|f| f.contains("Vendor")),
        "差异文件要落在那条 external 里，实际 {files:?}"
    );
}

#[test]
fn an_svn_url_pointing_at_a_subpath_is_not_checked_rather_than_lying() {
    // **这条是 2026-09-14 写测试时发现的一个真实误报路径。**
    //
    // svnUrl 配成仓库里的子路径（比如误填了白名单根的 URL）时，`--old` 那侧指向
    // 一个不存在的 `<svnUrl>/Code`，白名单里每个文件都被报成差异。而且输出里的
    // URL 前缀**照样剥得掉**（它就是 `<svnUrl>/<target>/…`），所以靠「剥不掉前缀」
    // 拦不住——拿到的是一份看起来完全合理的 Lying 清单。
    //
    // 按 spec §6.8.6，初始化时 Lying 会触发**整个回退**。把一处配置错误变成
    // 「基线撒谎」是这条检查最坏的失败形态，所以先核对 svnUrl 与工作副本根的 URL。
    let fx = svn_repo_for_health();
    let bogus = format!("{}/Code", repo_url(&fx));
    let got = check_baseline(
        &fx.wc_str(),
        &bogus,
        1,
        &WScope::plain(&wl(&["Code/"]), &cs()),
        &nothing_explained(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    match got {
        Health::NotChecked { reason } => {
            assert!(reason.contains("svnUrl"), "理由要说得清：{reason}")
        }
        other => panic!("实际：{other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 计划 Task 4：按映射分组体检、墓碑
//
// 体检比的是磁盘与服务器（前提：磁盘 == 基线树），所以这里不需要 git 仓库：映射照录制
// 的口径（`read_node_status` + `compute_map`）从工作副本算出来，S 由测试给出。
// ---------------------------------------------------------------------------

fn code_scope() -> WScope {
    WScope::plain(&wl(&["Code/"]), &cs())
}

fn units(fx: &SvnFixture) -> Vec<WcIdentity> {
    enumerate_wcs(&fx.wc_str(), &wl(&["Code/"])).unwrap()
}

/// 照录制的口径算映射：键 = 磁盘上带服务器 revision 的 `.cs` 文件（即基线树），墓碑由测试给。
fn map_of(fx: &SvnFixture, gone: BTreeMap<String, u64>) -> BaseMap {
    let wc = fx.wc_str();
    let us = units(fx);
    let nodes = read_node_status(&wc, &wl(&["Code/"]), &us).unwrap();
    let files: BTreeSet<String> = nodes
        .iter()
        .filter(|n| {
            n.node_rev.is_some() && n.path.ends_with(".cs") && fx.wc().join(&n.path).is_file()
        })
        .map(|n| n.path.clone())
        .collect();
    compute_map(&us[0].repos_root, &us[0].uuid, &files, &nodes, gone).unwrap()
}

fn check_map(fx: &SvnFixture, map: &BaseMap, origin: ExplainedOrigin) -> HealthMap {
    check_baseline_map(
        &fx.wc_str(),
        &repo_url(fx),
        map,
        &code_scope(),
        &nothing_explained(),
        origin,
    )
    .unwrap()
}

fn verified(rev: u64, gone: &[(&str, u64)]) -> HealthMap {
    HealthMap::Verified {
        health: Health::Verified {
            rev,
            unverified: vec![],
        },
        new_gone: gone.iter().map(|&(p, r)| (p.to_string(), r)).collect(),
    }
}

fn health_of(h: HealthMap) -> Health {
    match h {
        HealthMap::Verified { health, .. } => health,
        other => panic!("期望 Verified 分支，实际 {other:?}"),
    }
}

/// 三 revision 仓库停在 r3，再只提交 `Code/B.cs` 到 r4、不 update——「只提交不更新」。
fn commit_b_without_update(content: &str) -> SvnFixture {
    let fx = svn_repo_three_revisions();
    std::fs::write(fx.wc().join("Code/B.cs"), content).unwrap();
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
    fx
}

fn commit_rm_b() -> SvnFixture {
    let fx = svn_repo_three_revisions();
    svn_cmd(
        &fx.wc(),
        &["delete", "--non-interactive", "--", "Code/B.cs@"],
    );
    svn_cmd(&fx.wc(), &["commit", "-m", "r4 rm", "--non-interactive"]);
    fx
}

#[test]
fn grouped_check_verifies_commit_without_update_baseline() {
    let fx = commit_b_without_update("v4\n");
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(map.floor, 3);
    assert_eq!(
        map.revs,
        BTreeMap::from([("Code/B.cs".to_string(), 4)]),
        "前提：B 单列在 r4"
    );

    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::Unknown),
        verified(3, &[])
    );
    // 对照：只拿 floor 一个值去比（改造前的唯一形态），B 就是差异——分组才让它站得住
    assert_eq!(lying_files(&check(&fx, 3)), vec!["Code/B.cs".to_string()]);
    // B 那组真的发了一条带尾随 PATH 的命令，而不是被 floor 组的剔除顺手放过
    let plan = plan_diffs(&units(&fx), &map, &BTreeSet::new(), &BTreeSet::new()).unwrap();
    assert_eq!(plan.commands.len(), 2, "{plan:?}");
    let g = &plan.commands[1];
    assert_eq!(g.rev, 4);
    assert_eq!(g.args.last().map(String::as_str), Some("B.cs"));
    assert_eq!(
        g.scope,
        DiffScope::Only(BTreeSet::from(["Code/B.cs".to_string()]))
    );
    assert!(plan.commands[0]
        .args
        .contains(&"--ignore-properties".to_string()));
}

#[test]
fn grouped_check_flags_lying_file_in_newer_group() {
    // 「手动改基线树里一个高 revision 组的文件」：体检的前提是磁盘 == 基线树，
    // 所以改磁盘上的 B 就是改基线树里的 B
    let fx = commit_b_without_update("v4\n");
    let map = map_of(&fx, BTreeMap::new());
    std::fs::write(fx.wc().join("Code/B.cs"), "TAMPERED\n").unwrap();
    assert_eq!(
        lying_files(&health_of(check_map(&fx, &map, ExplainedOrigin::Unknown))),
        vec!["Code/B.cs".to_string()]
    );
    // floor 组里的文件同样有牙齿（剔除只剔单列的文件）
    std::fs::write(fx.wc().join("Code/B.cs"), "v4\n").unwrap();
    std::fs::write(fx.wc().join("Code/A.cs"), "TAMPERED\n").unwrap();
    assert_eq!(
        lying_files(&health_of(check_map(&fx, &map, ExplainedOrigin::Unknown))),
        vec!["Code/A.cs".to_string()]
    );
}

#[test]
fn committed_deletion_becomes_tombstone_when_just_recorded() {
    let fx = commit_rm_b();
    let map = map_of(&fx, BTreeMap::new());
    assert!(map.is_single() && map.floor == 3, "前提：{map:?}");

    // 刚录的基线：floor 上有、工作副本里没有的 B 向服务器确认已删除 → 墓碑，值 = 当时的 HEAD
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        verified(3, &[("Code/B.cs", 4)])
    );
    // 来历不明的基线不产生墓碑：没有墓碑解释的 D 就是差异（与改造前相同）
    assert_eq!(
        lying_files(&health_of(check_map(&fx, &map, ExplainedOrigin::Unknown))),
        vec!["Code/B.cs".to_string()]
    );
    // 已经记了墓碑：D 由墓碑解释，墓碑本身经服务器核对成立
    let tomb = map_of(&fx, BTreeMap::from([("Code/B.cs".to_string(), 4)]));
    assert_eq!(
        check_map(&fx, &tomb, ExplainedOrigin::Unknown),
        verified(3, &[])
    );
    assert_eq!(
        check_map(&fx, &tomb, ExplainedOrigin::JustRecorded),
        verified(3, &[]),
        "已有墓碑不再重复产生"
    );
}

#[test]
fn excluded_file_is_refused_not_tombstoned() {
    // E17：`--set-depth=exclude` 的文件从 status 消失、整单元 diff 也不报（Task 4 实测），
    // 服务器上却还在——那是漏收，不是删除
    let fx = svn_repo_for_health();
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--set-depth=exclude",
            "--non-interactive",
            "--",
            "Code/Deep/D.cs@",
        ],
    );
    assert!(
        !fx.wc().join("Code/Deep/D.cs").exists(),
        "前提：文件已被排除"
    );
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        HealthMap::Excluded {
            files: vec!["Code/Deep/D.cs".to_string()]
        }
    );
    // 来历不明时不扫被排除的节点：与改造前一样看不见，交给服务器侧核对
    assert_eq!(
        health_of(check_map(&fx, &map, ExplainedOrigin::Unknown)),
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
}

#[test]
fn excluded_directory_is_refused_file_by_file() {
    let fx = svn_repo_for_health();
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--set-depth=exclude",
            "--non-interactive",
            "--",
            "Code/Deep@",
        ],
    );
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        HealthMap::Excluded {
            files: vec!["Code/Deep/D.cs".to_string()]
        },
        "被排除的目录要点名到里面的代码文件"
    );
}

#[test]
fn excluded_non_code_file_does_not_refuse_recording() {
    // 不误报的一侧：排除掉的是美术文件（扩展名不在白名单里），基线本来就不收它
    let fx = svn_repo_for_health();
    svn_cmd(
        &fx.wc(),
        &[
            "update",
            "--set-depth=exclude",
            "--non-interactive",
            "--",
            "Code/Art.psd@",
        ],
    );
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        verified(1, &[])
    );
}

#[test]
fn tombstone_that_reappeared_on_server_is_lying() {
    // 墓碑说「服务器在 d 上没有 B」。B 在 r4 删掉、r5 又被同事加回来：
    // 记成 `B → 5` 的墓碑是假话，必须被服务器证伪；`B → 4` 仍然成立
    let fx = commit_rm_b();
    let seed = fx.path().join("seed");
    svn_cmd(&seed, &["update", "--non-interactive"]);
    std::fs::write(seed.join("Code/B.cs"), "back\n").unwrap();
    svn_cmd(&seed, &["add", "Code/B.cs", "--non-interactive"]);
    svn_cmd(&seed, &["commit", "-m", "r5 back", "--non-interactive"]);

    let lie = map_of(&fx, BTreeMap::from([("Code/B.cs".to_string(), 5)]));
    assert_eq!(
        lying_files(&health_of(check_map(&fx, &lie, ExplainedOrigin::Unknown))),
        vec!["Code/B.cs".to_string()]
    );
    let truth = map_of(&fx, BTreeMap::from([("Code/B.cs".to_string(), 4)]));
    assert_eq!(
        check_map(&fx, &truth, ExplainedOrigin::Unknown),
        verified(3, &[])
    );
}

#[test]
fn single_rev_map_uses_exactly_the_legacy_command() {
    // 单值映射：每个单元只有一条 diff，参数与改造前 `check_baseline` 发的逐字相同
    // （不带 `--ignore-properties`，不带尾随 PATH）
    let fx = svn_repo_for_health();
    let us = units(&fx);
    let map = BaseMap::single(us[0].repos_root.clone(), us[0].uuid.clone(), 1);
    let plan = plan_diffs(&us, &map, &BTreeSet::new(), &BTreeSet::new()).unwrap();
    let url = us[0].url.trim_end_matches('/');
    assert_eq!(plan.commands.len(), 1, "{plan:?}");
    assert_eq!(
        plan.commands[0].args,
        vec![
            "diff".to_string(),
            format!("--old={url}@1"),
            "--new=Code".to_string(),
            "--summarize".to_string(),
            "--xml".to_string(),
        ]
    );
    assert_eq!(plan.commands[0].scope, DiffScope::Floor);
    assert!(plan.unaddressable.is_empty());

    // 两个单元（嵌套 external）：仍是每单元一条，各用自己的 URL
    let fx2 = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    let us2 = units(&fx2);
    let plan2 = plan_diffs(
        &us2,
        &BaseMap::single(us2[0].repos_root.clone(), us2[0].uuid.clone(), 2),
        &BTreeSet::new(),
        &BTreeSet::new(),
    )
    .unwrap();
    assert_eq!(plan2.commands.len(), us2.len());
    for (c, u) in plan2.commands.iter().zip(&us2) {
        assert_eq!(c.args.len(), 5, "{c:?}");
        assert_eq!(
            c.args[1],
            format!("--old={}@2", u.url.trim_end_matches('/'))
        );
    }

    // 单值包装与按映射体检对同一个工作副本给出同一个结论
    assert_eq!(
        health_of(check_map(&fx, &map, ExplainedOrigin::Unknown)),
        check(&fx, 1)
    );
}

#[test]
fn newer_group_paths_are_batched_under_the_command_line_limit() {
    let u = WcIdentity {
        wc_root: "Code".to_string(),
        url: "file:///C:/r/Code".to_string(),
        repos_root: "file:///C:/r".to_string(),
        wc_root_abspath: None,
        uuid: "u".to_string(),
    };
    let mut map = BaseMap::single("file:///C:/r", "u", 10);
    let long = "x".repeat(200);
    for i in 0..400 {
        map.revs.insert(format!("Code/{long}/{i}.cs"), 12);
    }
    map.revs.insert("Code/skip.cs".to_string(), 12);
    let skip = BTreeSet::from(["Code/skip.cs".to_string()]);
    let plan = plan_diffs(&[u], &map, &skip, &BTreeSet::new()).unwrap();
    let groups: Vec<_> = plan.commands.iter().skip(1).collect();
    assert!(groups.len() > 1, "要分批：{}", groups.len());
    let mut seen = BTreeSet::new();
    for c in &groups {
        let len: usize =
            "svn --non-interactive".len() + c.args.iter().map(|a| a.len() + 3).sum::<usize>();
        assert!(len <= MAX_DIFF_CMDLINE, "{len}");
        assert_eq!(c.rev, 12);
        let DiffScope::Only(set) = &c.scope else {
            panic!("{c:?}")
        };
        // scope 里的每个文件都恰好对应一个尾随 PATH
        assert_eq!(
            set.len(),
            c.args.iter().skip_while(|a| *a != "--").count() - 1
        );
        seen.extend(set.iter().cloned());
    }
    assert_eq!(seen.len(), 400);
    assert!(!seen.contains("Code/skip.cs"), "剔除的 PATH 不能发出去");
}

#[test]
fn non_ascii_path_in_newer_group_is_checked_by_whole_unit_diff() {
    // svn.exe 按系统代码页取命令行参数，表示不了的字符变成 `?`（Task 4 实测 E155010）。
    // 这类文件改走整单元 diff、只看它自己；结果路径是 URL 编码的，要解码才对得上
    let fx = svn_repo_three_revisions();
    let name = "Code/中 文 %x@y.cs";
    std::fs::write(fx.wc().join(name), "new\n").unwrap();
    // 不在命令行上写这个路径：`add --force` 递归收进未版本化文件
    svn_cmd(
        &fx.wc(),
        &["add", "--force", "--non-interactive", "--", "Code@"],
    );
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "r4 add", "--non-interactive", "--", "Code@"],
    );
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(map.revs, BTreeMap::from([(name.to_string(), 4)]), "前提");
    let plan = plan_diffs(&units(&fx), &map, &BTreeSet::new(), &BTreeSet::new()).unwrap();
    assert!(
        plan.commands
            .iter()
            .all(|c| c.args.iter().all(|a| !a.contains('中'))),
        "非 ASCII 路径不能出现在尾随参数里：{plan:?}"
    );

    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::Unknown),
        verified(3, &[])
    );
    std::fs::write(fx.wc().join(name), "TAMPERED\n").unwrap();
    assert_eq!(
        lying_files(&health_of(check_map(&fx, &map, ExplainedOrigin::Unknown))),
        vec![name.to_string()],
        "报出来的是解码后的仓库相对路径"
    );
}

#[test]
fn unversioned_path_in_newer_group_is_skipped_and_reported_unverified() {
    // 旧映射里单列的 B 后来被 `svn rm --keep-local` 并提交：磁盘上还在、已不受版本控制。
    // 带着它发尾随 PATH 会让整条命令 E155010，所以发之前剔掉，并如实报「没比过」
    let fx = commit_b_without_update("v4\n");
    let map = map_of(&fx, BTreeMap::new());
    svn_cmd(
        &fx.wc(),
        &[
            "delete",
            "--keep-local",
            "--non-interactive",
            "--",
            "Code/B.cs@",
        ],
    );
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "r5 untrack", "--non-interactive"],
    );
    assert!(fx.wc().join("Code/B.cs").is_file(), "前提：文件还在磁盘上");
    assert_eq!(
        health_of(check_map(&fx, &map, ExplainedOrigin::Unknown)),
        Health::Verified {
            rev: 3,
            unverified: vec!["Code/B.cs".to_string()]
        }
    );
}

#[test]
fn newer_file_inside_a_nested_external_is_checked_against_that_unit() {
    // 映射的键是仓库相对路径，按最长的 wc_root 前缀归属单元：
    // `Code/Vendor/V.cs` 归 `Code/Vendor`，拿 vendor 的 URL 去比
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    std::fs::write(fx.wc().join("Code/Vendor/V.cs"), "v-next\n").unwrap();
    svn_cmd(
        &fx.wc().join("Code/Vendor"),
        &["commit", "-m", "vendor next", "--non-interactive"],
    );
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(
        map.revs.keys().collect::<Vec<_>>(),
        vec!["Code/Vendor/V.cs"],
        "{map:?}"
    );
    let us = units(&fx);
    let plan = plan_diffs(&us, &map, &BTreeSet::new(), &BTreeSet::new()).unwrap();
    let g = plan.commands.last().unwrap();
    assert_eq!(g.wc_root, "Code/Vendor");
    assert_eq!(g.args.last().map(String::as_str), Some("V.cs"));
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::Unknown),
        verified(map.floor, &[])
    );
}

#[test]
fn confirm_absent_matches_by_url_and_tolerates_missing_targets() {
    // E16：多 target 时存在的照常输出、不存在的每个一条 W170000，rc=1。
    // 按 `<url>` 对回，路径里的空格 / 中文 / `%` / `@` 要编码后才能当 URL
    let fx = svn_repo_three_revisions();
    let name = "Code/中 文 %x@y.cs";
    std::fs::write(fx.wc().join(name), "n\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &["add", "--force", "--non-interactive", "--", "Code@"],
    );
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "r4", "--non-interactive", "--", "Code@"],
    );
    let root = units(&fx)[0].repos_root.clone();
    let at = |p: &str, r: u64| format!("{root}/{}@{r}", encode_url_path(p));
    let got = confirm_absent(
        &fx.wc_str(),
        &[
            ("A".to_string(), at("Code/A.cs", 4)),
            ("CN".to_string(), at(name, 4)),
            ("CN3".to_string(), at(name, 3)),
            ("NO".to_string(), at("Code/nope.cs", 4)),
        ],
    )
    .unwrap();
    assert_eq!(
        got,
        BTreeMap::from([
            ("A".to_string(), true),
            ("CN".to_string(), true),
            ("CN3".to_string(), false),
            ("NO".to_string(), false),
        ])
    );
    assert!(confirm_absent(&fx.wc_str(), &[]).unwrap().is_empty());
    assert_eq!(server_head_rev(&fx.wc_str(), &root).unwrap(), 4);
}

#[test]
fn confirm_absent_refuses_a_revision_the_server_does_not_have() {
    // E160006 是「revision 不存在」，与「文件不存在」是两回事：不能当成不存在去立墓碑
    let fx = svn_repo_three_revisions();
    let root = units(&fx)[0].repos_root.clone();
    let err = confirm_absent(
        &fx.wc_str(),
        &[("A".to_string(), format!("{root}/Code/A.cs@99"))],
    )
    .unwrap_err();
    assert_eq!(err.code, "svn_tombstone_no_such_rev", "{err:?}");
}

// ---------------------------------------------------------------------------
// Review 1 修复（Task 1–5）
// ---------------------------------------------------------------------------

fn unconfirmed_files(h: HealthMap) -> Vec<String> {
    match h {
        HealthMap::Unconfirmed { files, reason } => {
            assert!(!reason.is_empty(), "要说清楚哪一步没确认成");
            files
        }
        other => panic!("期望 Unconfirmed，实际 {other:?}"),
    }
}

#[test]
fn deletion_that_cannot_be_confirmed_refuses_recording_instead_of_not_checked() {
    // Review 1 第 1 条：floor 组的 D 已知、却问不到服务器时，改造前那份状态是 Lying（拒绝），
    // 不能降级成 NotChecked——录制接受 NotChecked，发布出去的基线就静默丢了 B、也没有墓碑
    let fx = commit_rm_b();
    let map = map_of(&fx, BTreeMap::new());
    for p in [Point::ServerHead, Point::ConfirmAbsent] {
        let _armed = fault::arm(p);
        assert_eq!(
            unconfirmed_files(check_map(&fx, &map, ExplainedOrigin::JustRecorded)),
            vec!["Code/B.cs".to_string()],
            "{p:?}"
        );
    }
    // 对照：撤掉注入后照常成墓碑
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        verified(3, &[("Code/B.cs", 4)])
    );
}

/// 三 revision 仓库：r4 提交删除 B，r5 只提交 A、不 update。映射因此是混合的，floor 组之后
/// 还有一条 A 的 revision 组，D（B）在第一条 diff 里就知道了。
fn commit_rm_b_then_a() -> SvnFixture {
    let fx = commit_rm_b();
    std::fs::write(fx.wc().join("Code/A.cs"), "a5\n").unwrap();
    svn_cmd(
        &fx.wc(),
        &[
            "commit",
            "-m",
            "r5",
            "--non-interactive",
            "--",
            "Code/A.cs@",
        ],
    );
    fx
}

#[test]
fn diff_failure_after_the_comparison_started_refuses_recording() {
    // Review 2 第 6 条：floor 组已经报出 B，后面的 revision 组失败时降成 NotChecked，录制放行，
    // 发布的基线就静默丢了 B
    let fx = commit_rm_b_then_a();
    let map = map_of(&fx, BTreeMap::new());
    assert!(!map.revs.is_empty(), "前提：有 revision 组");
    {
        let _armed = fault::arm(Point::LaterDiff);
        assert_eq!(
            unconfirmed_files(check_map(&fx, &map, ExplainedOrigin::JustRecorded)),
            vec!["Code/B.cs".to_string()]
        );
        // 不产生墓碑的体检照旧：只是「没验成」
        assert!(matches!(
            health_of(check_map(&fx, &map, ExplainedOrigin::Unknown)),
            Health::NotChecked { .. }
        ));
    }
    assert!(matches!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        HealthMap::Verified { .. }
    ));
}

#[test]
fn first_diff_failing_is_still_not_checked_while_recording() {
    // 离线时一条都没比成，什么都不知道：录制照旧放行（遗留风险见日志 Review 1）
    let fx = commit_rm_b_then_a();
    let map = map_of(&fx, BTreeMap::new());
    let _armed = fault::arm(Point::FirstDiff);
    assert!(matches!(
        health_of(check_map(&fx, &map, ExplainedOrigin::JustRecorded)),
        Health::NotChecked { .. }
    ));
}

#[test]
fn unreadable_diff_path_refuses_recording() {
    // svn 的输出读不懂时不知道它说的是哪个文件，同样不能放行
    let fx = commit_rm_b();
    let map = map_of(&fx, BTreeMap::new());
    let _armed = fault::arm(Point::DiffPath);
    assert!(matches!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        HealthMap::Unconfirmed { .. }
    ));
    assert!(matches!(
        health_of(check_map(&fx, &map, ExplainedOrigin::Unknown)),
        Health::NotChecked { .. }
    ));
}

#[test]
fn existing_tombstone_check_failure_refuses_recording_with_the_known_deletions() {
    // 比对做完了，B 已知，核对旧墓碑失败时同样不能放行
    let fx = commit_rm_b();
    let gone = BTreeMap::from([("Code/Gone.cs".to_string(), 4)]);
    let map = map_of(&fx, gone);
    assert!(!map.gone.is_empty(), "前提：有旧墓碑");
    let _armed = fault::arm(Point::ConfirmAbsent);
    assert_eq!(
        unconfirmed_files(check_map(&fx, &map, ExplainedOrigin::JustRecorded)),
        vec!["Code/B.cs".to_string()]
    );
}

#[test]
fn excluded_node_scan_failure_refuses_recording() {
    // 找不出被排除的节点，就不知道有没有漏收（E17）：同样拒绝，不放行成 NotChecked
    let fx = svn_repo_for_health();
    let map = map_of(&fx, BTreeMap::new());
    let _armed = fault::arm(Point::ExcludedScan);
    assert!(unconfirmed_files(check_map(&fx, &map, ExplainedOrigin::JustRecorded)).is_empty());
    // 来历不明的体检不扫被排除的节点，碰不到这个注入点
    assert_eq!(
        health_of(check_map(&fx, &map, ExplainedOrigin::Unknown)),
        Health::Verified {
            rev: 1,
            unverified: vec![]
        }
    );
}

/// 把 `rel`（含非 ASCII 字符的目录）设成 depth=exclude。
///
/// svn.exe 按系统代码页取命令行参数（Task 4「计划没写的坑」），中文路径传不进去。改为以
/// 那个目录为 cwd 跑 `svn update --set-depth=exclude .`（cwd 走宽字符 API）：wc.db 里记下
/// exclude，删目录那一步因为 cwd 占着它而失败、留在 work queue 里，再从 wc 根 cleanup 补做。
/// 实测（SlikSvn 1.14.2）稳定。
fn exclude_by_cwd(fx: &SvnFixture, rel: &str) {
    let _ = std::process::Command::new("svn")
        .current_dir(fx.wc().join(rel))
        .args(["update", "--set-depth=exclude", "--non-interactive", "."])
        .output()
        .unwrap();
    svn_cmd(&fx.wc(), &["cleanup", "--non-interactive"]);
    assert!(!fx.wc().join(rel).exists(), "前提：`{rel}` 已被排除");
    let info = run_svn(&fx.wc_str(), &["info", "-R", "--xml", "--", "Code@"]).unwrap();
    assert!(info.contains("<depth>exclude</depth>"), "{info}");
}

/// svn_repo_for_health 再加一个中文目录（r2）并更新、再把它排除。`files` 是目录里的文件名。
fn with_excluded_cn_dir(files: &[&str]) -> SvnFixture {
    let fx = svn_repo_for_health();
    let dir = fx.wc().join("Code/中文目录");
    std::fs::create_dir_all(&dir).unwrap();
    for f in files {
        std::fs::write(dir.join(f), "x\n").unwrap();
    }
    svn_cmd(
        &fx.wc(),
        &["add", "--force", "--non-interactive", "--", "Code@"],
    );
    svn_cmd(&fx.wc(), &["commit", "-m", "r2", "--non-interactive"]);
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
    exclude_by_cwd(&fx, "Code/中文目录");
    fx
}

#[test]
fn non_ascii_excluded_directory_with_only_non_code_files_does_not_refuse() {
    // Review 1 第 3 条（D8）：中文目录被排除，里面只有美术文件。改造前点不了名的目录一律算漏收，
    // 中文目录很常见，会永久拦住初始化 / 重建；现在列出它在 floor 上的内容、按 W-file 筛
    let fx = with_excluded_cn_dir(&["a.png"]);
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(map.floor, 2, "前提：{map:?}");
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        verified(2, &[])
    );
    // 列不出来就不知道里面有没有代码文件：拒绝，点名那个目录
    let _armed = fault::arm(Point::ListAtFloor);
    assert_eq!(
        unconfirmed_files(check_map(&fx, &map, ExplainedOrigin::JustRecorded)),
        vec!["Code/中文目录".to_string()]
    );
}

#[test]
fn non_ascii_excluded_directory_absent_at_the_floor_does_not_refuse() {
    // 目录 r2 才有，A 退回 r1 把 floor 拖到 1：floor 上没有这个节点，基线不欠它什么。
    // `svn ls` 在这个形态下 rc=1、W160013（实测），不能当成「列不出来」去拒绝；
    // 点得了名的节点同一形态是 rc=0、无输出，两边结论要一致
    let fx = with_excluded_cn_dir(&["b.cs"]);
    svn_cmd(
        &fx.wc(),
        &["update", "-r1", "--non-interactive", "--", "Code/A.cs@"],
    );
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(map.floor, 1, "前提：{map:?}");
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        verified(1, &[])
    );
}

#[test]
fn non_ascii_excluded_directory_with_a_code_file_is_refused_file_by_file() {
    let fx = with_excluded_cn_dir(&["a.png", "b.cs"]);
    let map = map_of(&fx, BTreeMap::new());
    assert_eq!(
        check_map(&fx, &map, ExplainedOrigin::JustRecorded),
        HealthMap::Excluded {
            files: vec!["Code/中文目录/b.cs".to_string()]
        }
    );
}
