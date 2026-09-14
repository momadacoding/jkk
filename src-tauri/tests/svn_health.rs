//! §6.8 基线体检：让基线可被服务器证伪。
//!
//! 场景编号沿用 spec §6.8.2 的 T1–T7，2026-09-14 在临时 fixture 上逐条复现过。
//!
//! 这里最重要的不是 T2/T4 那几条「抓得住」，而是 T3 / T3b 两条「不误报」——
//! 误报会让一个完全正常的仓库**永远建不起来**，而漏报至少还有别的信号兜着。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_health::{check_baseline, Health};
use svn_fixtures::{file_url, svn_repo_for_health, svn_repo_for_health_at_r2, SvnFixture};

fn wl(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn cs() -> Vec<String> {
    vec![".cs".to_string()]
}

fn repo_url(fx: &SvnFixture) -> String {
    file_url(&fx.repo())
}

/// 跑一次体检，白名单固定 `Code/`、扩展名固定 `.cs`。
fn check(fx: &SvnFixture, rev: u64) -> Health {
    check_baseline(&fx.wc_str(), &repo_url(fx), rev, &wl(&["Code/"]), &cs()).unwrap()
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
    assert_eq!(check(&fx, 1), Health::Verified { rev: 1 });
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
    assert_eq!(check(&fx, 1), Health::Verified { rev: 1 });
}

#[test]
fn t3b_a_modified_art_file_inside_the_whitelist_does_not_trigger_a_false_alarm() {
    // 与 T3 不同：`Code/Art.psd` **就在白名单根底下**，2026-09-14 实测它
    // 确实会出现在 `svn diff --summarize` 的输出里。靠扩展名过滤剔除
    // （spec §6.8.4：「否则恢复流程有意保留的 Code/Art.psd 本地修改会让体检误报」）。
    let fx = svn_repo_for_health();
    std::fs::write(fx.wc().join("Code/Art.psd"), "CHANGED\n").unwrap();
    assert_eq!(check(&fx, 1), Health::Verified { rev: 1 });
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
    assert_eq!(check(&fx, 2), Health::Verified { rev: 2 });
}

#[test]
fn t6_multiple_whitelist_roots_are_accepted() {
    let fx = svn_repo_for_health();
    let got = check_baseline(
        &fx.wc_str(),
        &repo_url(&fx),
        1,
        &wl(&["Code/", "Art/"]),
        &cs(),
    )
    .unwrap();
    assert_eq!(got, Health::Verified { rev: 1 });
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
        &wl(&["Code/"]),
        &cs(),
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
            &wl(&["Code/"]),
            &cs()
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
    let err =
        check_baseline(&fx.wc_str(), &repo_url(&fx), 1, &[], &cs()).expect_err("空白名单必须报错");
    assert_eq!(err.code, "svn_health_empty_whitelist");
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
    let got = check_baseline(&fx.wc_str(), &bogus, 1, &wl(&["Code/"]), &cs()).unwrap();
    match got {
        Health::NotChecked { reason } => {
            assert!(reason.contains("svnUrl"), "理由要说得清：{reason}")
        }
        other => panic!("实际：{other:?}"),
    }
}
