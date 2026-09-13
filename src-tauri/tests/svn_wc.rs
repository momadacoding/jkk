//! 白名单覆盖到的工作副本枚举与 P4（spec §6.5 / §6.5.5）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_wc::{check_single_repository, enumerate_wcs};
use svn_fixtures::{
    svn_repo, svn_repo_with_at_sign_in_path, svn_repo_with_cross_repo_nested_external,
};

#[test]
fn a_whitelist_path_containing_an_at_sign_is_not_parsed_as_a_peg_revision() {
    // `Code/Team@Home` 是完全合法的目录名，也过得了配置校验（`@` 不是通配符），
    // 但 SVN 把 target 里最后一个 `@` 之后的部分当 peg revision 解析，**`--`
    // 挡不住**。2026-09-14 实测：`svn info` 报 `E205000: Syntax error parsing
    // peg revision 'Home'`，`svn status` 报 `E200009: a peg revision is not
    // allowed here`——两条命令 `enumerate_wcs` 都要发，所以整个白名单条目不可用。
    let fx = svn_repo_with_at_sign_in_path();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/Team@Home/".to_string()]).unwrap();
    assert_eq!(wcs.len(), 1, "实际枚举到 {wcs:?}");
    // 内部键必须是**原始**路径：那个 `@` 后缀只属于命令行，混进 wc_root 会让
    // 去重比对失效，也会原样显示给用户
    assert_eq!(wcs[0].wc_root, "Code/Team@Home");
    assert!(
        wcs[0].url.ends_with("Code/Team@Home"),
        "实际 url={}",
        wcs[0].url
    );
}

#[test]
fn a_plain_working_copy_yields_exactly_one_wc() {
    let fx = svn_repo();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert_eq!(wcs.len(), 1, "实际枚举到 {wcs:?}");
    assert_eq!(wcs[0].wc_root, "Code");
    assert!(!wcs[0].uuid.is_empty());
    check_single_repository(&wcs).unwrap();
}

#[test]
fn a_nested_cross_repo_external_is_enumerated_and_rejected() {
    // spec §6.5.5：这正是 P4 初版漏掉的形态。
    let fx = svn_repo_with_cross_repo_nested_external();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert_eq!(
        wcs.len(),
        2,
        "应当枚举到 Code 与 Code/Vendor 两个 WC，实际 {wcs:?}"
    );
    let err = check_single_repository(&wcs).expect_err("跨仓库必须被 P4 拒绝");
    assert_eq!(err.code, "svn_cross_repo");
    // 错误信息必须能指出是哪一条，否则用户无从下手
    assert!(err.message.contains("Vendor"), "实际：{}", err.message);
}

#[test]
fn checking_only_the_whitelist_root_would_have_let_it_through() {
    // **反向证据**：证明 §6.5.5 的递归枚举不是过度设计。
    // 只查白名单根拿到的是 repo1 的 UUID，单值，P4 会干净通过——
    // 这条如果哪天开始失败，说明有人改了枚举顺序或 P4 的判据，要重新想。
    let fx = svn_repo_with_cross_repo_nested_external();
    let all = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    let root_only: Vec<_> = all
        .iter()
        .filter(|w| w.wc_root == "Code")
        .cloned()
        .collect();
    assert_eq!(root_only.len(), 1);
    check_single_repository(&root_only).expect("只查根的旧写法会放行——这正是要修的缺陷");
}

#[test]
fn the_two_nested_wcs_really_belong_to_different_repositories() {
    // 上一条的前提：两个 WC 的 UUID 确实不同。如果 fixture 造错了（比如
    // external 指回了同一个仓库），P4 那条断言会因为「本来就该通过」而
    // 假绿——这条把 fixture 自身的正确性也钉住。
    let fx = svn_repo_with_cross_repo_nested_external();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert_ne!(wcs[0].uuid, wcs[1].uuid, "fixture 没造出跨仓库的形态");
}

#[test]
fn an_empty_whitelist_enumerates_nothing() {
    // G6 / spec §12.2：空列表是 no-op，不能退化成「整个工作副本」。
    let fx = svn_repo();
    let wcs = enumerate_wcs(&fx.wc_str(), &[]).unwrap();
    assert!(wcs.is_empty());
    // 空列表进 P4 也必须是通过而不是 panic
    check_single_repository(&wcs).unwrap();
}

#[test]
fn a_whitelist_entry_that_is_a_file_still_resolves_to_its_working_copy() {
    // 白名单可以是单个文件（spec §6.3）。`svn info` 对文件同样给出
    // repository root 与 uuid，所以 P4 对这种条目照样成立。
    let fx = svn_repo();
    let wcs = enumerate_wcs(&fx.wc_str(), &["Code/A.cs".to_string()]).unwrap();
    assert_eq!(wcs.len(), 1);
    assert!(!wcs[0].uuid.is_empty());
}
