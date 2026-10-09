//! D0.5：从 HEAD 摘掉 Local-Only commit 构造交付树（spec §8.2.3；计划 B2）。
//! 全部在临时 fixture 上跑。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_deliver::{deliver_ref, pin, plan_deliver, unpin};
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

/// `Code/A.cs` 写成 30 行（第 n 行是 n），并作为普通 commit 提交。之后各测试改不同的行。
fn thirty_lines(fx: &SvnFixture) {
    let body: String = (1..=30).map(|n| format!("{n}\n")).collect();
    std::fs::write(fx.wc().join("Code/A.cs"), body).unwrap();
    git(fx, &["commit", "-qam", "thirty lines"]);
}

fn edit_line(fx: &SvnFixture, line: usize, to: &str, message: &str) {
    let p = fx.wc().join("Code/A.cs");
    let text = std::fs::read_to_string(&p).unwrap();
    let out: String = text
        .lines()
        .enumerate()
        .map(|(i, l)| {
            if i + 1 == line {
                format!("{to}\n")
            } else {
                format!("{l}\n")
            }
        })
        .collect();
    std::fs::write(&p, out).unwrap();
    git(fx, &["commit", "-qam", message]);
}

fn base(fx: &SvnFixture) -> String {
    git(fx, &["rev-parse", "svn/base"])
}

fn head(fx: &SvnFixture) -> String {
    git(fx, &["rev-parse", "HEAD"])
}

fn show(fx: &SvnFixture, rev: &str, path: &str) -> String {
    git(fx, &["show", &format!("{rev}:{path}")])
}

#[test]
fn without_local_only_the_head_is_delivered_as_is() {
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert!(p.fast);
    assert_eq!(p.deliver_oid, head(&fx));
    assert!(p.local_only.is_empty());
}

#[test]
fn a_local_only_suffix_is_cut_off_without_replaying_anything() {
    // 快路径：只是给一个已有 commit 起名字（spec §8.2.3）
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    let c1 = head(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert!(p.fast);
    assert_eq!(p.deliver_oid, c1, "交付的就是补丁之前那个已有 commit");
    assert_eq!(p.local_only.len(), 1);
    assert_eq!(p.local_only[0].subject, "L");
}

#[test]
fn a_local_only_in_the_middle_is_replayed_out_without_touching_the_disk() {
    // T1 + T6：不重叠时重放成功；磁盘、HEAD、索引、ref 全没动
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    edit_line(&fx, 20, "twenty", "C2");
    let before_head = head(&fx);
    let before_disk = std::fs::read(fx.wc().join("Code/A.cs")).unwrap();
    let before_refs = git(&fx, &["for-each-ref"]);

    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert!(!p.fast);
    let a = show(&fx, &p.deliver_oid, "Code/A.cs");
    assert!(a.contains("one") && a.contains("twenty"), "{a}");
    assert!(!a.contains("ten-LOCAL"), "本地补丁进了交付树：{a}");
    assert_eq!(
        git(&fx, &["rev-parse", &format!("{}^{{tree}}", p.deliver_oid)]),
        p.deliver_tree
    );

    assert_eq!(head(&fx), before_head);
    assert_eq!(
        std::fs::read(fx.wc().join("Code/A.cs")).unwrap(),
        before_disk
    );
    assert_eq!(
        git(&fx, &["for-each-ref"]),
        before_refs,
        "预览不许留下任何 ref"
    );
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
}

#[test]
fn replayed_commits_keep_their_author_and_message() {
    // 计划 B15：与 cherry-pick 一致，git log 里看得出是谁写的
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    let p = fx.wc().join("Code/A.cs");
    let text = std::fs::read_to_string(&p)
        .unwrap()
        .replace("20\n", "twenty\n");
    std::fs::write(&p, text).unwrap();
    git(
        &fx,
        &[
            "-c",
            "user.name=Someone Else",
            "-c",
            "user.email=else@x",
            "commit",
            "-qam",
            "C2\n\nbody line",
        ],
    );
    let plan = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert_eq!(
        git(&fx, &["log", "-1", "--format=%an <%ae>", &plan.deliver_oid]),
        "Someone Else <else@x>"
    );
    assert_eq!(
        git(&fx, &["log", "-1", "--format=%B", &plan.deliver_oid]),
        "C2\n\nbody line"
    );
}

#[test]
fn an_overlapping_local_only_aborts_and_names_both_commits() {
    // T2：紧挨着的 hunk 冲突，必须中止（spec §8.2.3「绝不自动解冲突」）
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    edit_line(&fx, 11, "eleven", "fix jump");
    let before = head(&fx);
    let err = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_out_local_only_conflict", "{}", err.message);
    assert!(
        err.message.contains("mute log") && err.message.contains("fix jump"),
        "{}",
        err.message
    );
    assert!(err.message.contains("Code/A.cs"), "{}", err.message);
    assert_eq!(head(&fx), before);
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
}

#[test]
fn the_slow_path_refuses_any_merge_but_the_fast_path_does_not_care() {
    // 必测 #24 / #72①：慢路径 + merge → 前置拒绝；把补丁移到顶端（快路径）就能交付
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    std::fs::write(fx.wc().join("Code/Side.cs"), "side\n").unwrap();
    git(&fx, &["add", "Code/Side.cs"]);
    git(&fx, &["commit", "-qm", "side"]);
    git(&fx, &["checkout", "-q", "work"]);
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    git(&fx, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    edit_line(&fx, 20, "twenty", "C2");

    let err = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_out_merge_in_slow_path", "{}", err.message);
    assert!(err.message.contains("merge side"), "{}", err.message);

    // 同样的历史，但没有夹在中间的补丁：快路径，merge 原样在交付树里
    let fx2 = hybrid_with_colleague();
    thirty_lines(&fx2);
    git(&fx2, &["checkout", "-q", "-b", "side"]);
    std::fs::write(fx2.wc().join("Code/Side.cs"), "side\n").unwrap();
    git(&fx2, &["add", "Code/Side.cs"]);
    git(&fx2, &["commit", "-qm", "side"]);
    git(&fx2, &["checkout", "-q", "work"]);
    git(
        &fx2,
        &["merge", "-q", "--no-ff", "-m", "merge side", "side"],
    );
    let merged = head(&fx2);
    edit_line(&fx2, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    let p = plan_deliver(&fx2.wc_str(), &base(&fx2), &head(&fx2)).unwrap();
    assert!(p.fast);
    assert_eq!(p.deliver_oid, merged);
}

#[test]
fn a_local_only_suffix_that_contains_a_merge_is_refused() {
    // 必测 #72③ / §8.1 第 7 条：后缀会在 D8d 被 rebase 重放，rebase 会压平 merge
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    edit_line(&fx, 5, "five-LOCAL", "side patch\n\nLocal-Only: true");
    git(&fx, &["checkout", "-q", "work"]);
    edit_line(&fx, 25, "twenty-five", "C");
    git(
        &fx,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "merge local\n\nLocal-Only: true",
            "side",
        ],
    );
    let err = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_out_local_only_merge", "{}", err.message);
}

#[test]
fn when_everything_is_local_only_the_baseline_itself_is_the_delivery() {
    let fx = hybrid_with_colleague();
    edit_line_on_fresh_file(&fx);
    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert_eq!(
        p.deliver_oid,
        base(&fx),
        "D1 会是空清单，由前置检查报「没有可交付的改动」"
    );
    assert_eq!(p.local_only.len(), 1);
}

fn edit_line_on_fresh_file(fx: &SvnFixture) {
    std::fs::write(fx.wc().join("Code/A.cs"), "a1 LOCAL\n").unwrap();
    git(fx, &["commit", "-qam", "L\n\nLocal-Only: true"]);
}

#[test]
fn the_deliver_ref_never_collides_with_a_user_branch_named_deliver() {
    // 必测 #25：放进 refs/mygitclient/，用户叫 deliver 的分支照常存在
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "deliver"]);
    let r = pin(&fx.wc_str(), "op-1", &head(&fx)).unwrap();
    assert_eq!(r, deliver_ref("op-1"));
    assert_eq!(r, "refs/mygitclient/deliver/op-1");
    assert_eq!(git(&fx, &["rev-parse", "deliver"]), head(&fx));
}

#[test]
fn pin_refuses_to_overwrite_and_unpin_is_idempotent_and_fenced() {
    let fx = hybrid_with_colleague();
    let r = pin(&fx.wc_str(), "op-2", &head(&fx)).unwrap();
    assert!(
        pin(&fx.wc_str(), "op-2", &base(&fx)).is_err(),
        "同名残留不能被覆盖"
    );
    unpin(&fx.wc_str(), &r).unwrap();
    unpin(&fx.wc_str(), &r).unwrap(); // §8.2.4：崩溃重开会重跑收尾，第二次不算失败
    for bad in [
        "refs/heads/work",
        "refs/mygitclient/deliver/",
        "refs/mygitclient/sync/x",
    ] {
        let err = unpin(&fx.wc_str(), bad).unwrap_err();
        assert_eq!(
            err.code, "sync_out_bad_deliver_ref",
            "{bad}：{}",
            err.message
        );
    }
    assert_eq!(
        git(&fx, &["rev-parse", "work"]),
        head(&fx),
        "别的 ref 一律不碰"
    );
}
