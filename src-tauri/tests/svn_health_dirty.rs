//! §6.8.7 的判据 `D \ S == ∅`：工作副本不干净时的基线体检。
//!
//! # 这个文件必须成对存在，缺一半就是虚假信心
//!
//! - 只写「不误报」：一个永远返回 `Verified` 的实现照样全绿，等于把 §6.8 整条
//!   检查删掉。
//! - 只写「有牙齿」：一个永远返回 `Lying` 的实现照样全绿，而那会让每个正常仓库
//!   在有本地修改时都被指控撒谎——比不检查更糟。
//!
//! 所以两组断言写在同一个文件里，谁也别想单独被删掉（`AGENTS.md` §13 rubric #9）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_dirty::{classify, explained_paths, read_dirty};
use app_lib::engine::svn_health::{check_baseline, ExplainedOrigin, Health};
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use std::collections::BTreeSet;
use svn_fixtures::{file_url, svn_cmd, svn_repo_for_health, SvnFixture};

fn wl() -> Vec<String> {
    vec!["Code/".to_string()]
}
fn cs() -> Vec<String> {
    vec![".cs".to_string()]
}
fn req(fx: &SvnFixture) -> InitRequest {
    InitRequest {
        repo_root: fx.wc_str(),
        whitelist: wl(),
        extensions: cs(),
    }
}

/// 此刻的 `S`：白名单内被 svn 判为脏的那些 W-file。
///
/// 与 `svn_init::record_server_tree` 用的是同一套函数，所以这里算出来的就是
/// 产品代码会算出来的那一份，不是测试自己另编的一份。
fn explained_now(fx: &SvnFixture) -> BTreeSet<String> {
    let dirty = read_dirty(&fx.wc_str(), &wl()).unwrap();
    let fixes = classify(&dirty).unwrap();
    explained_paths(&fixes, &wl(), &cs())
}

// ---------------------------------------------------------------------------
// (a) 不误报：本地改动不该被判成基线撒谎
// ---------------------------------------------------------------------------

#[test]
fn local_modifications_are_explained_and_do_not_accuse_the_baseline() {
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();
    let wc = fx.wc();

    // 程序员手上最常见的四种状态，一次摆齐
    std::fs::write(wc.join("Code/A.cs"), "LOCAL EDIT\n").unwrap();
    std::fs::write(wc.join("Code/Untracked.cs"), "new\n").unwrap();
    std::fs::write(wc.join("Code/Added.cs"), "added\n").unwrap();
    svn_cmd(&wc, &["add", "Code/Added.cs", "--non-interactive"]);
    svn_cmd(&wc, &["delete", "Code/Deep/D.cs", "--non-interactive"]);

    let s = explained_now(&fx);
    assert!(
        s.contains("Code/A.cs") && s.contains("Code/Deep/D.cs"),
        "前提：这些路径确实进了 S。实际 S = {s:?}"
    );

    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        1,
        &wl(),
        &cs(),
        &s,
        ExplainedOrigin::Unknown,
    )
    .unwrap();

    assert!(
        matches!(got, Health::Verified { .. }),
        "本地改动每一处都能被 S 解释，`D \\ S` 应当是空集。实际：{got:?}"
    );
}

// ---------------------------------------------------------------------------
// (b) 有牙齿：基线声称 r2、磁盘其实停在 r1
// ---------------------------------------------------------------------------

#[test]
fn a_baseline_claiming_a_revision_the_disk_never_reached_is_caught() {
    // **这才是判据存在的意义。** 它抓的正是 `svn status` 这类「相对本地 BASE」的
    // 间接判据**结构上看不见**的那一类缺陷：工作副本相对自己的 BASE 确实没改过，
    // 所以 status 干干净净；错的是基线自称的那个 revision。
    //
    // 真实成因：漏跑了一次 update、恢复流程没走完、或者基线本来就录错了。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();

    // 另一个 checkout 把服务器推到 r2，wc1 **故意不 update**，仍停在 r1
    let url = file_url(&fx.repo());
    svn_cmd(fx.path(), &["checkout", &url, "wc2", "--non-interactive"]);
    let wc2 = fx.path().join("wc2");
    std::fs::write(wc2.join("Code/A.cs"), "v2-on-server\n").unwrap();
    svn_cmd(&wc2, &["commit", "-m", "r2", "--non-interactive"]);

    // 前提：svn status 什么都看不见
    let s = explained_now(&fx);
    assert!(
        s.is_empty(),
        "前提：工作副本相对自己的 BASE 是干净的，S 必须为空。实际 S = {s:?}"
    );

    // 基线**自称** r2，而磁盘上是 r1 的内容
    let got = check_baseline(
        &fx.wc_str(),
        &url,
        2,
        &wl(),
        &cs(),
        &s,
        ExplainedOrigin::Unknown,
    )
    .unwrap();

    match got {
        Health::Lying { files } => assert!(
            files.iter().any(|f| f == "Code/A.cs"),
            "必须指名是哪个文件对不上。实际：{files:?}"
        ),
        other => {
            panic!("基线声称 r2 而磁盘停在 r1，这是假基线的典型形态，必须被抓住。实际：{other:?}")
        }
    }
}

#[test]
fn a_file_that_got_dirty_after_s_was_captured_still_blocks() {
    // §16.1 竞态表第一行：R2a 读完脏清单之后，用户在外部编辑器里又改脏了一个
    // **当时还干净**的文件。那个文件按磁盘内容进了树，却不在 `S` 里。
    //
    // 这里直接喂一份**过期的 `S`**（清单读取时刻的那一份），验证判据会中止。
    // 方向必须是「误拦」而不是「静默录错」——重点一次按钮 vs Phase 3 丢改动。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();

    let stale_s = explained_now(&fx); // 此刻干净 → 空集
    assert!(stale_s.is_empty(), "前提：抓拍时工作副本是干净的");

    // 抓拍之后才发生的改动
    std::fs::write(fx.wc().join("Code/A.cs"), "edited after the snapshot\n").unwrap();

    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        1,
        &wl(),
        &cs(),
        &stale_s,
        ExplainedOrigin::Unknown,
    )
    .unwrap();

    match got {
        Health::Lying { files } => {
            assert!(files.iter().any(|f| f == "Code/A.cs"), "实际：{files:?}")
        }
        other => panic!("解释不了的差异必须中止（误拦是安全方向）。实际：{other:?}"),
    }
}

#[test]
fn the_criterion_degenerates_to_the_original_one_on_a_clean_working_copy() {
    // `S = ∅` 时 `D \ S == ∅` 就是 §6.8.2 的 `D == ∅`。这条钉住「推广不是替换」——
    // S7 / 恢复⑥ / D7a 三个既有入口的行为依赖它逐字不变。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();

    assert!(explained_now(&fx).is_empty(), "前提：工作副本是干净的");

    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        1,
        &wl(),
        &cs(),
        &BTreeSet::new(),
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    assert!(matches!(got, Health::Verified { .. }), "实际：{got:?}");
}

#[test]
fn a_dirty_file_outside_the_whitelist_extensions_never_enters_s() {
    // `Code/Art.psd` 改脏了。它既不该进 `D`（体检只看 W-file，§6.8.4），
    // 也不该进 `S`——进了 `S` 就是在无谓地放大它，而 `S` 越大判据越弱（§16.1）。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();
    std::fs::write(fx.wc().join("Code/Art.psd"), "LOCAL PSD\n").unwrap();

    let s = explained_now(&fx);

    assert!(
        s.is_empty(),
        "非白名单扩展名的脏文件不该进 S。实际 S = {s:?}"
    );
    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        1,
        &wl(),
        &cs(),
        &s,
        ExplainedOrigin::Unknown,
    )
    .unwrap();
    assert!(
        matches!(got, Health::Verified { .. }),
        "美术文件的本地修改本来就不归体检管（§6.8.4 的不误报）。实际：{got:?}"
    );
}

// ---------------------------------------------------------------------------
// (c) 覆盖面必须如实报出去（2026-09-21 独立 review）
//
// 上面 (a) 那条只断言「没被冤枉」，而 `Verified` 是一句**主动的保证**。
// `S` 里那批路径一条都没被验证过，判据把它们整批豁免了——不报出去，前端就会
// 无条件显示「磁盘内容与服务器上该 revision 一致」，而那句话对它们是假的。
// ---------------------------------------------------------------------------

/// 摆出 (a) 里那四种本地状态，返回 `(fixture, S)`。
fn dirty_with_four_shapes() -> (SvnFixture, BTreeSet<String>) {
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();
    let wc = fx.wc();
    std::fs::write(wc.join("Code/A.cs"), "LOCAL EDIT\n").unwrap();
    std::fs::write(wc.join("Code/Untracked.cs"), "new\n").unwrap();
    std::fs::write(wc.join("Code/Added.cs"), "added\n").unwrap();
    svn_cmd(&wc, &["add", "Code/Added.cs", "--non-interactive"]);
    svn_cmd(&wc, &["delete", "Code/Deep/D.cs", "--non-interactive"]);
    let s = explained_now(&fx);
    assert!(!s.is_empty(), "前提：S 非空");
    (fx, s)
}

#[test]
fn a_manual_check_reports_exactly_which_paths_it_could_not_cover() {
    let (fx, s) = dirty_with_four_shapes();

    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        1,
        &wl(),
        &cs(),
        &s,
        // 手动体检：基线是谁、什么时候录的都不知道
        ExplainedOrigin::Unknown,
    )
    .unwrap();

    match got {
        Health::Verified { rev, unverified } => {
            assert_eq!(rev, 1);
            // **恰好是 S**，不是它的子集也不是超集：少报一条就是继续撒谎，
            // 多报一条会让用户去查一个其实验过的文件。
            assert_eq!(
                unverified.iter().cloned().collect::<BTreeSet<String>>(),
                s,
                "未覆盖清单必须恰好等于 S"
            );
            assert!(
                unverified.contains(&"Code/A.cs".to_string()),
                "用户正在改的那个文件必须在清单里：{unverified:?}"
            );
        }
        other => panic!("本地改动都能被 S 解释，应当 Verified。实际：{other:?}"),
    }
}

#[test]
fn right_after_recording_there_is_nothing_to_disclose() {
    // 另一半，缺了它这组就只剩「永远报一堆未覆盖」也能全绿。
    //
    // 初始化 / 重建那条路径上，`S` 里的 blob 是几毫秒前由 R2b 从
    // `svn cat -r BASE` 写进去的，而 P2 要求 `nodeRevs` 单值、`rev` 取的就是
    // 那个值 ⇒ `BASE == rev` ⇒ 那些 blob 就是服务器 `rev` 上的内容。
    // 由构造保证正确，没有什么要向用户声明的。
    let (fx, s) = dirty_with_four_shapes();

    let got = check_baseline(
        &fx.wc_str(),
        &file_url(&fx.repo()),
        1,
        &wl(),
        &cs(),
        &s,
        ExplainedOrigin::JustRecorded,
    )
    .unwrap();

    match got {
        Health::Verified { unverified, .. } => assert!(
            unverified.is_empty(),
            "刚录完的路径由构造保证正确，不该报成未覆盖：{unverified:?}"
        ),
        other => panic!("应当 Verified。实际：{other:?}"),
    }
}

#[test]
fn a_clean_working_copy_has_nothing_to_disclose_either() {
    // `S = ∅` 时两种 origin 都必须给出空清单——否则干净仓库上会平白多出一句
    // 「有 0 个文件未覆盖」。
    let fx = svn_repo_for_health();
    init_hybrid(&req(&fx)).unwrap();
    assert!(explained_now(&fx).is_empty(), "前提：工作副本是干净的");

    for origin in [ExplainedOrigin::Unknown, ExplainedOrigin::JustRecorded] {
        let got = check_baseline(
            &fx.wc_str(),
            &file_url(&fx.repo()),
            1,
            &wl(),
            &cs(),
            &BTreeSet::new(),
            origin,
        )
        .unwrap();
        match got {
            Health::Verified { unverified, .. } => {
                assert!(
                    unverified.is_empty(),
                    "{origin:?} 下应当为空：{unverified:?}"
                )
            }
            other => panic!("{origin:?} 下应当 Verified。实际：{other:?}"),
        }
    }
}
