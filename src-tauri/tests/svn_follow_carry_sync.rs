#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进带回发布之后的判定（计划 `2026-11-25-follow-perf-pr2.md`，2026-11-24 分析方案 4）。
//!
//! 前端拿到 `sync` 就不再调 `svn_sync_status`，所以它必须与紧接着跑一次 `check_sync` **逐项相同**：
//! 整次跟进、部分跟进、点「跟进」冲突（基线发布、分支落后）三种都比。没发布基线、没有锁内复核读过
//! 节点时不带，前端照原来自己读。现场的搭法同 `svn_follow_auto.rs`。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::repo;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::{
    follow, follow_auto_with_sync, follow_with_sync, AutoFollowOutcome, FollowOutcome, Followed,
    SyncAfterFollow,
};
use app_lib::engine::svn_init::rebuild_baseline;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args)
        .unwrap_or_else(|e| panic!("git {args:?}: {}", e.message))
        .trim()
        .to_string()
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}
fn commit_all(fx: &SvnFixture, msg: &str) {
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", msg]);
}
fn tortoise_commit(fx: &SvnFixture, rel: &str) {
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", rel],
    );
}
fn set_me(fx: &SvnFixture, who: &str) {
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some(who.to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();
}
/// 「我」取 wc1 里提交的作者（同 `svn_follow.rs` 的 `i_commit`）。
fn i_commit(fx: &SvnFixture) {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(["info", "--show-item", "last-changed-author", "--", A])
        .output()
        .unwrap();
    assert!(out.status.success());
    set_me(fx, String::from_utf8_lossy(&out.stdout).trim());
}

/// 带回的判定与紧接着的 `check_sync` 逐项相同，HEAD 与此刻 `open_repo` 读到的相同。
fn assert_same_as_fresh(fx: &SvnFixture, sync: &SyncAfterFollow) {
    assert_eq!(sync.status, check_sync(&fx.wc_str()).unwrap());
    assert_eq!(sync.head, repo::open(&fx.wc_str()).unwrap());
}

#[test]
fn a_whole_auto_follow_carries_the_verdict_a_fresh_check_would_give() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, A);

    let (out, sync) = follow_auto_with_sync(&fx.wc_str(), "auto1").unwrap();

    assert!(
        matches!(out, AutoFollowOutcome::Done { .. }),
        "应当自动跟进完成：{out:?}"
    );
    let sync = sync.expect("跟进完成时带回判定");
    assert!(
        matches!(sync.status.verdict, SyncVerdict::InSync),
        "{:?}",
        sync.status.verdict
    );
    assert!(!sync.status.head_behind_base);
    assert_same_as_fresh(&fx, &sync);
}

#[test]
fn a_partial_auto_follow_carries_the_files_it_left_behind() {
    // B：我在分支上改了第 1 行，同事改了第 8 行、我局部 update 了，磁盘上还有没提交的一行，重放结果
    // 与磁盘对不上，留在旧基线上（同 `svn_follow_auto.rs` 的 `overlap_scene`）。A 照常跟进
    let fx = hybrid_with_colleague();
    let lines = |ls: &[&str]| ls.iter().map(|l| format!("{l}\n")).collect::<String>();
    set_me(&fx, "me");
    write(&fx, B, &lines(&["1", "2", "3", "4", "5", "6", "7", "8"]));
    tortoise_commit(&fx, B);
    assert!(matches!(
        follow(&fx.wc_str(), "seed").unwrap(),
        FollowOutcome::Done { .. }
    ));
    fx.colleague_commits(B, &lines(&["1", "2", "3", "4", "5", "6", "7", "colleague"]));
    write(&fx, B, &lines(&["mine", "2", "3", "4", "5", "6", "7", "8"]));
    commit_all(&fx, "我的改动");
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", B]);
    write(
        &fx,
        B,
        &lines(&[
            "mine",
            "2",
            "3",
            "4",
            "5",
            "6",
            "7",
            "colleague",
            "uncommitted",
        ]),
    );
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let (out, sync) = follow_auto_with_sync(&fx.wc_str(), "auto1").unwrap();

    let AutoFollowOutcome::Done { report } = out else {
        panic!("应当部分跟进完成：{out:?}");
    };
    assert_eq!(
        report
            .held_back
            .iter()
            .map(|h| h.path.as_str())
            .collect::<Vec<_>>(),
        [B]
    );
    let sync = sync.expect("部分跟进完成时也带回判定");
    match &sync.status.verdict {
        SyncVerdict::ExternallyAdvanced { changed, .. } => assert_eq!(
            changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            [B],
            "横幅只剩留下的文件"
        ),
        other => panic!("留下的文件仍是 SVN 侧的新内容：{other:?}"),
    }
    assert_same_as_fresh(&fx, &sync);
}

#[test]
fn a_conflicting_button_follow_carries_the_lagging_branch() {
    // 没有快照 ⇒ 先后拿不准 ⇒ 正常重放，冲突（同 `svn_follow.rs` 的
    // `own_commit_without_mark_replays_normally_and_conflicts`）：基线发布了，分支没动
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);

    let (out, sync) = follow_with_sync(&fx.wc_str(), "manual").unwrap();

    assert!(
        matches!(out, FollowOutcome::NeedsManual { .. }),
        "应当冲突：{out:?}"
    );
    let sync = sync.expect("基线发布了就带回判定");
    assert!(sync.status.head_behind_base, "分支落后基线");
    assert!(!sync.status.behind_paths.is_empty());
    assert_same_as_fresh(&fx, &sync);

    // 再点一次走分支跟进入口，重放照样冲突：准备完就返回，不拿锁、不复核，没有要带的
    let (again, sync) = follow_with_sync(&fx.wc_str(), "manual2").unwrap();
    assert!(
        matches!(again, FollowOutcome::NeedsManual { .. }),
        "{again:?}"
    );
    assert!(sync.is_none(), "冲突的分支跟进入口什么都没发布：{sync:?}");
}

#[test]
fn a_branch_follow_entry_that_completes_carries_nothing() {
    // 基线已前进、分支没跟上（同 `svn_follow.rs` 的 `branch_follow_entry_moves_lagging_branch`）：分支跟进
    // 入口没有录制，锁内复核不重读节点（`Recheck::Valid { nodes: None }`），完成了也不带，前端照原来读
    let fx = hybrid_with_colleague();
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    assert!(check_sync(&fx.wc_str()).unwrap().head_behind_base);

    let (out, sync) = follow_with_sync(&fx.wc_str(), "manual").unwrap();

    assert!(
        matches!(out, FollowOutcome::Done { .. }),
        "应当走分支跟进入口完成：{out:?}"
    );
    assert!(sync.is_none(), "{sync:?}");
    assert!(
        !check_sync(&fx.wc_str()).unwrap().head_behind_base,
        "跟上了"
    );
}

#[test]
fn nothing_published_carries_nothing() {
    let fx = hybrid_with_colleague();
    let (out, sync) = follow_auto_with_sync(&fx.wc_str(), "auto1").unwrap();
    assert!(matches!(out, AutoFollowOutcome::Skipped { .. }), "{out:?}");
    assert!(sync.is_none());

    let (out, sync) = follow_with_sync(&fx.wc_str(), "manual").unwrap();
    assert!(matches!(out, FollowOutcome::Refused { .. }), "{out:?}");
    assert!(sync.is_none());
}

#[test]
fn the_carried_verdict_sits_beside_the_outcome_in_json() {
    // 前端按 `kind` 认结局（`svnFollow.ts`）：包一层之后结局的字段必须还在顶层，`sync` 的字段是 camelCase
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let followed = Followed::from(follow_auto_with_sync(&fx.wc_str(), "auto1").unwrap());

    let v = serde_json::to_value(&followed).unwrap();

    assert_eq!(v["kind"], "done", "{v}");
    assert!(v["report"]["newFloor"].is_u64(), "{v}");
    assert_eq!(v["sync"]["status"]["verdict"]["kind"], "inSync", "{v}");
    assert!(v["sync"]["status"]["headBehindBase"].is_boolean(), "{v}");
    assert_eq!(
        v["sync"]["head"]["headHash"],
        git(&fx, &["rev-parse", "HEAD"]),
        "{v}"
    );
    assert!(v["sync"]["head"]["headParents"].is_array(), "{v}");

    // 没有判定时不发 `sync`，前端照原来读
    let skipped = Followed::from(follow_auto_with_sync(&fx.wc_str(), "auto2").unwrap());
    let v = serde_json::to_value(&skipped).unwrap();
    assert_eq!(v["kind"], "skipped", "{v}");
    assert!(v.get("sync").is_none(), "{v}");
}
