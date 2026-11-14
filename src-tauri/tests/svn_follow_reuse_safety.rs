#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进沿用上次取到的 blob 时不能用错（独立 review 第 1、2 条）：同一个路径换成了别的 SVN 节点
//! （目录被复制回旧版本、`svn switch` 到分支），最后修改的 revision 可能比上次录的还早，内容却已
//! 不同。只比 revision 的判据在这两个测试上都录出了错的内容（2026-11-14 实测）。
//!
//! 本地改过的文件不进体检（属于「已解释」的 S），录错了没人报，所以用真实 svn 把这两种形态固定
//! 下来：上一次跟进时文件已经改着（缓存里有它），这一次新基线里必须是工作副本 BASE 的那一版。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_follow::{follow, FollowOutcome};
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const M: &str = "Code/M.cs";
/// 同事先提交 V1，再改第一行成 V2；我只改最后一行，update / switch 都能干净合并。
const V1: &str = "l1\nl2\nl3\nl4\nl5\n";
const V2: &str = "L1\nl2\nl3\nl4\nl5\n";

fn show(fx: &SvnFixture, spec: &str) -> String {
    run_git(&fx.wc_str(), &["show", spec])
        .unwrap_or_else(|e| panic!("git show {spec}: {}", e.message))
}

fn run(fx: &SvnFixture) -> FollowOutcome {
    follow(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}

fn follow_done(fx: &SvnFixture) {
    let out = run(fx);
    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
}

fn update(fx: &SvnFixture) {
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);
}

/// 本地只改最后一行。
fn edit_mine(fx: &SvnFixture) {
    let p = fx.wc().join(M);
    let text = std::fs::read_to_string(&p).unwrap().replace("l5", "mine5");
    std::fs::write(p, text).unwrap();
}

/// 同事提交 M 的 V1（r2）、V2（r3）。我 update 到 V2、改着最后一行跟进一次：M 进了缓存。
fn baseline_with_dirty_v2(fx: &SvnFixture) {
    fx.colleague_commits(M, V1);
    fx.colleague_commits(M, V2);
    update(fx);
    edit_mine(fx);
    follow_done(fx);
    assert_eq!(show(fx, &format!("svn/base:{M}")), V2, "前提");
}

#[test]
fn a_file_inside_a_directory_replaced_by_an_older_copy_takes_the_new_base() {
    let fx = hybrid_with_colleague();
    baseline_with_dirty_v2(&fx);
    // 同事把整个 Code 目录回退成 r2 的副本：M.cs 是 V1，最后修改 revision 是复制源里的 r2
    let wc2 = fx.wc2();
    svn_cmd(&wc2, &["update", "--non-interactive"]);
    svn_cmd(&wc2, &["rm", "Code", "--non-interactive"]);
    svn_cmd(&wc2, &["cp", "^/Code@2", "Code", "--non-interactive"]);
    svn_cmd(&wc2, &["commit", "-m", "revert Code", "--non-interactive"]);
    // 单是这次回退，判定看不出来（最后修改 revision 都没超过基线，另一个已知盲区）；
    // 再来一个新文件，让跟进真的走到录制
    fx.colleague_commits("Code/C.cs", "c1\n");
    // 目录被替换时本地改着会成树冲突：我先撤掉改动、update，再改回来
    svn_cmd(&fx.wc(), &["revert", "--", M]);
    update(&fx);
    assert_eq!(std::fs::read_to_string(fx.wc().join(M)).unwrap(), V1);
    edit_mine(&fx);

    follow_done(&fx);

    assert_eq!(show(&fx, &format!("svn/base:{M}")), V1, "BASE 是 r2 的副本");
}

#[test]
fn a_switched_directory_takes_the_branch_base() {
    let fx = hybrid_with_colleague();
    baseline_with_dirty_v2(&fx);
    // 分支从 r2 的 Code 拉出来：分支上的 M.cs 是 V1，最后修改在 r2
    let wc2 = fx.wc2();
    svn_cmd(
        &wc2,
        &[
            "cp",
            "^/Code@2",
            "^/branch-code",
            "-m",
            "branch",
            "--non-interactive",
        ],
    );
    // 分支上再加一个文件：单是 switch，判定看不出来（同上一个测试），让跟进真的走到录制
    fx.colleague_commits("branch-code/D.cs", "d1\n");
    svn_cmd(
        &fx.wc(),
        &["switch", "^/branch-code", "Code", "--non-interactive"],
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join(M)).unwrap(),
        V1.replace("l5", "mine5"),
        "前提：本地改动合并到了分支版本上"
    );

    let out = run(&fx);
    // 切到分支之后跟进拒绝也可以；只要做了，基线里就必须是分支 BASE 的内容
    if let FollowOutcome::Done { .. } = out {
        assert_eq!(
            show(&fx, &format!("svn/base:{M}")),
            V1,
            "BASE 是分支上的版本"
        );
    } else {
        panic!("switch 之后应当能跟进：{out:?}");
    }
}
