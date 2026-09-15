//! §6.3.1 的服务器侧证伪：`svn ls -R` 的清单 vs 磁盘上的 W-file。
//!
//! # 这块机制是为一个**四道检查全绿的盲区**造的
//!
//! `svn update --set-depth=exclude Code/Deep` 之后（2026-09-15 实测）：
//!
//! | 检查 | 结果 |
//! |---|---|
//! | P1 白名单内 status | 零条目 |
//! | P2 `nodeRevs` | 单值 |
//! | P3 工作副本状态 | 干净 |
//! | §6.8 基线体检 | **Verified** |
//!
//! 而基线里静默少了服务器上确实存在的 `Code/Deep/D.cs`。`svn info` 对父目录
//! 照样报 `depth=infinity`，靠读 depth 也检测不出来。唯一看得见它的入口是
//! `svn ls -R <url>@N`。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_health::{check_baseline, Health};
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use app_lib::engine::svn_server_set::{self, ServerAudit};
use app_lib::engine::svn_sync::{self, SyncVerdict};
use svn_fixtures::{
    file_url, svn_cmd, svn_repo_for_health, svn_repo_whitelist_added_in_r2, SvnFixture,
};

fn init(fx: &SvnFixture) {
    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();
}

/// 当前基线的 revision。fixture 只提交过一次，所以恒为 1。
const REV: u64 = 1;

#[test]
fn a_normal_repo_matches() {
    let fx = svn_repo_for_health();
    init(&fx);
    let got = svn_server_set::audit_server_set(&fx.wc_str(), REV).unwrap();
    assert!(matches!(got, ServerAudit::Matched), "实际 {got:?}");
}

#[test]
fn art_files_on_the_server_are_not_reported_as_missing() {
    // fixture 的服务器上有 `Code/Art.psd`（白名单内但扩展名不符）与
    // `Art/boss.fbx`（白名单外）。两者**本来就不该**进 git。
    //
    // 不做同一套过滤的话，每个真实 Unity 工程一打开就会报出几千条「漏收」——
    // 那种噪音等于把这条检查关掉。
    let fx = svn_repo_for_health();
    init(&fx);
    match svn_server_set::audit_server_set(&fx.wc_str(), REV).unwrap() {
        ServerAudit::Matched => {}
        ServerAudit::Missing { files, .. } => panic!("美术资产被误报成漏收：{files:?}"),
        other => panic!("实际 {other:?}"),
    }
}

#[test]
fn a_depth_excluded_subtree_is_reported_as_missing() {
    let fx = svn_repo_for_health();
    init(&fx);
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
    assert!(
        !fx.wc().join("Code/Deep").exists(),
        "前提：depth-exclude 之后磁盘上那棵子树应当消失"
    );

    let got = svn_server_set::audit_server_set(&fx.wc_str(), REV).unwrap();
    match got {
        ServerAudit::Missing { files, .. } => assert!(
            files
                .iter()
                .any(|f| f.replace('\\', "/").ends_with("Code/Deep/D.cs")),
            "要点名漏掉的是哪个文件：{files:?}"
        ),
        other => panic!("应当报漏收，实际 {other:?}"),
    }
}

#[test]
fn the_existing_checks_really_do_miss_a_depth_excluded_subtree() {
    // 这条**不测新代码**，它把「为什么需要这块新机制」钉成回归测试。
    // 少了它，将来有人「优化」掉这次网络调用时，不会有任何测试变红。
    let fx = svn_repo_for_health();
    init(&fx);
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

    let st = svn_sync::check_sync(&fx.wc_str()).unwrap();
    assert_eq!(
        st.verdict,
        SyncVerdict::InSync,
        "失同步判据看不见 depth-exclude —— 这正是 NB-2 的成因"
    );

    let health = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        REV,
        &["Code/".to_string()],
        &[".cs".to_string()],
    )
    .unwrap();
    assert!(
        matches!(health, Health::Verified { .. }),
        "体检也看不见 —— 实际 {health:?}"
    );
}

#[test]
fn an_unreachable_url_is_not_checked_rather_than_matched() {
    // **本模块最危险的失败形态。**
    //
    // 实测 P1：路径不存在时 svn **exit 1**，但 stdout 仍是一份结构完整的空
    // `<lists><list></list></lists>`。忽略退出码的写法会拿到空的服务器清单，
    // 而「服务器有、磁盘没有」的差集在空集上**恒为空** ⇒ 报告「没有漏收」。
    // 那就是用修 NB-2 的代码重新造一个 NB-2。
    let fx = svn_repo_for_health();
    init(&fx);
    // 把仓库本体从磁盘删掉：工作副本还在（svn info 是本地读），但 URL 已不可达
    std::fs::remove_dir_all(fx.repo()).unwrap();

    let got = svn_server_set::audit_server_set(&fx.wc_str(), REV).unwrap();
    assert!(
        matches!(got, ServerAudit::NotChecked { .. }),
        "URL 不可达必须是「没验成」，不能是「没漏收」，实际 {got:?}"
    );
}

#[test]
fn a_path_missing_at_that_revision_is_not_checked_rather_than_matched() {
    // **这条才是实测 P1 那个真正危险的形态，也是本模块唯一靠退出码兜住的一条。**
    //
    // 仓库存在、只是 `Code/` 在 r1 上还没建出来：svn **exit 1**，而 stdout 仍是
    // 一份**结构完整的空** `<lists><list></list></lists>` —— `parse_list` 会成功
    // 解析出一个空清单。忽略退出码的写法就会拿到空的服务器集合，差集恒为空，
    // 于是报告「没有漏收」。
    //
    // 上面 `an_unreachable_url_is_not_checked_rather_than_matched` 那条测不出这个：
    // 把整个仓库删掉时 XML 是**坏的**，解析失败已经兜住了，退出码那道门根本没被
    // 考验到（2026-09-15 变异测试证实：换成忽略退出码，那条照样绿）。
    let fx = svn_repo_whitelist_added_in_r2();
    init(&fx); // 基线 r2
    let got = svn_server_set::audit_server_set(&fx.wc_str(), 1).unwrap();
    assert!(
        matches!(got, ServerAudit::NotChecked { .. }),
        "路径在该 revision 上不存在时必须是「没验成」，不能是「没漏收」，实际 {got:?}"
    );
}

#[test]
fn a_revision_the_server_does_not_have_is_not_checked_either() {
    // 实测 P2：revision 不存在时 svn exit 1，而且 XML 被**截断在半路**。
    // 解析层会报错，但这里要确认它落成 NotChecked 而不是冒泡成 Err ——
    // 这条检查是锦上添花，不该让打开仓库失败。
    let fx = svn_repo_for_health();
    init(&fx);
    let got = svn_server_set::audit_server_set(&fx.wc_str(), 999).unwrap();
    assert!(
        matches!(got, ServerAudit::NotChecked { .. }),
        "实际 {got:?}"
    );
}

#[test]
fn a_file_only_on_disk_is_not_reported() {
    // 反方向（磁盘有、服务器没有）是「本地新增还没 svn add」，完全正常。
    // 报了就是噪音，而噪音会让用户学会忽略这条提示。
    let fx = svn_repo_for_health();
    init(&fx);
    std::fs::write(fx.wc().join("Code/New.cs"), "n\n").unwrap();

    let got = svn_server_set::audit_server_set(&fx.wc_str(), REV).unwrap();
    assert!(
        matches!(got, ServerAudit::Matched),
        "本地新增不该被报成漏收，实际 {got:?}"
    );
}
