//! 跟进编排 F1–F6 与崩溃恢复（计划 Task 10；分析 §6.5、§6.11.1、§6.11.3）。
//!
//! 全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事；「TortoiseSVN 提交」就是在 wc1 里直接
//! `svn commit` 一个文件。`file://` 下作者都是系统用户，要区分「我」与同事时改 revprop。
//!
//! 崩溃与竞态用 `svn_follow::fault` 注入：`crash_at` 模拟进程死掉（同步锁随 unwind 释放，
//! 对应操作系统回收句柄；`.git/index.lock` 留在磁盘上），`call_at` 在固定位置插进一段外部操作。

#[path = "common/long_ref.rs"]
mod long_ref;
#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::commands::svn::svn_branches_behind;
use app_lib::engine::git_cli::{run_git, run_git_env};
use app_lib::engine::svn_baseline::TempIndex;
use app_lib::engine::svn_basemap::read_base_map;
use app_lib::engine::svn_cli::with_scan_cache;
use app_lib::engine::svn_config::{read_config, write_config};
use app_lib::engine::svn_follow::fault::{self, Crashed, Point};
use app_lib::engine::svn_follow::{
    follow, git_version_ok, manual_follow, resume_follow, FollowOutcome, FollowReport,
};
use app_lib::engine::svn_follow_mark::{mark_path, read_mark, write_mark};
use app_lib::engine::svn_init::{rebuild_baseline, record_mixed};
use app_lib::engine::svn_journal::{self, Flow, FollowStep, Journal, Phase};
use app_lib::engine::svn_leak::LeakVerdict;
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_in::{abort_sync, continue_sync, SyncInOutcome};
use app_lib::engine::svn_sync_out::continue_sync_out;
use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::process::Command;
use svn_fixtures::{
    hybrid_whitelist_in_external, hybrid_with_colleague, svn_cmd, svn_repo, SvnFixture,
};

const A: &str = "Code/A.cs";
const B: &str = "Code/B.cs";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args)
        .unwrap_or_else(|e| panic!("git {args:?}: {}", e.message))
        .trim()
        .to_string()
}
fn oid(fx: &SvnFixture, rev: &str) -> String {
    git(fx, &["rev-parse", rev])
}
fn write(fx: &SvnFixture, rel: &str, s: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, s).unwrap();
}
fn read(fx: &SvnFixture, rel: &str) -> String {
    std::fs::read_to_string(fx.wc().join(rel)).unwrap()
}
fn commit_all(fx: &SvnFixture, msg: &str) -> String {
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", msg]);
    oid(fx, "HEAD")
}
fn show(fx: &SvnFixture, spec: &str) -> String {
    git(fx, &["show", spec])
}

/// TortoiseSVN 只提交这一个文件，不 update。返回新 revision。
fn tortoise_commit(fx: &SvnFixture, rel: &str) -> u64 {
    svn_cmd(
        &fx.wc(),
        &["commit", "-m", "tortoise", "--non-interactive", "--", rel],
    );
    youngest(fx)
}
fn youngest(fx: &SvnFixture) -> u64 {
    let out = Command::new("svnlook")
        .args(["youngest", &fx.repo().to_string_lossy()])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}
/// file:// 访问时 `--username` 不改作者，只能直接改 revprop（ASCII：svnadmin 按本机代码页读文件）。
fn set_author(fx: &SvnFixture, rev: u64, who: &str) {
    let f = fx.path().join(format!("author-{rev}.txt"));
    std::fs::write(&f, who).unwrap();
    let out = Command::new("svnadmin")
        .args([
            "setrevprop",
            &fx.repo().to_string_lossy(),
            "-r",
            &rev.to_string(),
            "svn:author",
            &f.to_string_lossy(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn svn_update(fx: &SvnFixture, rel: &str) {
    svn_cmd(&fx.wc(), &["update", "--non-interactive", "--", rel]);
}
fn set_me(fx: &SvnFixture, who: &str) {
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some(who.to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();
}
/// wc1 里提交的作者：file:// 下就是系统用户。`<commit author>` 读的是 wc.db，提交之后再改 revprop
/// 也不会刷新（同事的 revision 要在 update 之前改），所以「我」直接取这个名字。
fn committer(fx: &SvnFixture) -> String {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(["info", "--show-item", "last-changed-author", "--", A])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn i_commit(fx: &SvnFixture) -> String {
    let me = committer(fx);
    set_me(fx, &me);
    me
}

fn run(fx: &SvnFixture) -> FollowOutcome {
    follow(&fx.wc_str(), "op1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
fn done(o: FollowOutcome) -> FollowReport {
    match o {
        FollowOutcome::Done { report } => *report,
        other => panic!("应当 Done：{other:?}"),
    }
}
fn refused(o: FollowOutcome) -> (String, String) {
    match o {
        FollowOutcome::Refused { code, message } => (code, message),
        other => panic!("应当 Refused：{other:?}"),
    }
}

fn index_lock(fx: &SvnFixture) -> std::path::PathBuf {
    fx.wc().join(".git/index.lock")
}
fn assert_no_residue(fx: &SvnFixture) {
    assert!(!index_lock(fx).exists(), "不该留下 index.lock");
    assert!(
        svn_journal::read(&fx.wc_str()).unwrap().is_none(),
        "不该留下 journal"
    );
    assert!(!svn_lock::is_held(&fx.wc_str()), "不该还拿着同步锁");
}
fn staged_paths(fx: &SvnFixture) -> Vec<String> {
    git(fx, &["diff", "--cached", "--name-only"])
        .lines()
        .map(str::to_string)
        .collect()
}
fn modified_paths(fx: &SvnFixture) -> Vec<String> {
    git(fx, &["diff", "--name-only"])
        .lines()
        .map(str::to_string)
        .collect()
}

/// 工作副本全部文件（不含 `.git`、`.svn`）的内容与 mtime。
fn wc_snapshot(fx: &SvnFixture) -> BTreeMap<String, (Vec<u8>, std::time::SystemTime)> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, (Vec<u8>, std::time::SystemTime)>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name == ".git" || name == ".svn" {
                continue;
            }
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let mtime = e.metadata().unwrap().modified().unwrap();
                out.insert(rel, (std::fs::read(&p).unwrap(), mtime));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&fx.wc(), &fx.wc(), &mut out);
    out
}

// —— 主路径 ——

#[test]
fn follow_after_tortoise_commit_leaves_only_uncommitted_changes() {
    // U2 / G1：改了 A、B，只把 A 提交到 SVN。跟进之后「本地改动」只剩 B
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    assert_eq!(
        modified_paths(&fx),
        vec![A, B],
        "前提：两个文件都算本地改动"
    );
    let r = tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(modified_paths(&fx), vec![B]);
    assert!(staged_paths(&fx).is_empty(), "不能凭空多出暂存");
    let base = oid(&fx, "svn/base");
    assert_ne!(base, old_base);
    assert_eq!(oid(&fx, &format!("{base}^")), old_base, "父提交是旧基线");
    assert_eq!(oid(&fx, "HEAD"), base, "work 上没有 commit，直接到新基线");
    assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a2");
    assert_eq!(show(&fx, "svn/base:Code/B.cs"), "b1", "没提交的不进基线");
    let map = read_base_map(&fx.wc_str(), "svn/base").unwrap();
    assert_eq!(map.floor, 1);
    assert_eq!(map.revs.get(A), Some(&r));
    assert_eq!(rep.new_floor, 1);
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(rep.changes[0].path, A);
    assert_eq!(rep.changes[0].rev, r);
    assert_eq!(read(&fx, B), "b2\n", "磁盘不动");
    assert_no_residue(&fx);
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
}

#[test]
fn follow_after_partial_update_puts_colleague_changes_in_baseline() {
    // U7：同事改了 B，我对这一个文件局部 update，自己的 A 没提交
    let fx = hybrid_with_colleague();
    set_me(&fx, "me");
    fx.colleague_commits(B, "b-colleague\n");
    let r = youngest(&fx);
    set_author(&fx, r, "colleague");
    write(&fx, A, "a-mine\n");
    svn_update(&fx, B);
    assert_eq!(read(&fx, B), "b-colleague\n");

    let rep = done(run(&fx));

    assert_eq!(modified_paths(&fx), vec![A], "同事的改动不算本地改动");
    assert_eq!(show(&fx, "svn/base:Code/B.cs"), "b-colleague");
    assert_eq!(rep.changes[0].author.as_deref(), Some("colleague"));
    let msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    assert!(msg.contains(&format!("跟进 r{r}：colleague")), "{msg}");
}

#[test]
fn follow_drops_intermediate_commits_of_own_svn_commit() {
    // U3：git 里提交过中间版本 a2，之后磁盘改成 a3 用 TortoiseSVN 提交
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    let c = commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    write_mark(&fx.wc_str()).unwrap();
    let r = tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(
        oid(&fx, "HEAD"),
        oid(&fx, "svn/base"),
        "commit 变空，分支到新基线"
    );
    assert_eq!(rep.dropped, vec![c.clone()]);
    assert_eq!(rep.rewritten, vec![(c, vec![A.to_string()])]);
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a3");
    assert!(modified_paths(&fx).is_empty());
    assert_eq!(
        read_base_map(&fx.wc_str(), "svn/base").unwrap().rev_of(A),
        r
    );
    assert!(read_mark(&fx.wc_str()).unwrap().is_none(), "收尾清掉快照");
    assert!(!mark_path(&fx.wc_str()).exists());
}

#[test]
fn follow_keeps_other_paths_of_a_partly_committed_commit() {
    // 分析 §6.5：C 改了 a 和 e，只有 a 被 TortoiseSVN 提交，C' 只剩 e 的改动
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    let c = commit_all(&fx, "改了两个文件");
    write(&fx, A, "a3\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.rewritten, vec![(c, vec![A.to_string()])]);
    assert!(rep.dropped.is_empty());
    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"));
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a3");
    assert_eq!(show(&fx, "HEAD:Code/B.cs"), "b2");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn own_commit_without_mark_replays_normally_and_conflicts() {
    // 没有快照 ⇒ 先后拿不准 ⇒ P 为空，正常重放（这里因此冲突，不会静默丢掉 a2）
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);

    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));
}

#[test]
fn commit_after_the_mark_is_not_superseded() {
    // 快照之后才有的 commit 不是快照 HEAD 的祖先：拿不准，正常重放
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, "a2\n");
    commit_all(&fx, "快照之后的 commit");
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);

    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));
}

#[test]
fn colleague_commit_is_not_superseded_even_before_the_mark() {
    let fx = hybrid_with_colleague();
    set_me(&fx, "me");
    write(&fx, A, "a2\n");
    commit_all(&fx, "我的 commit");
    write(&fx, A, "a3\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A); // 作者是系统用户，不是配置里的「me」

    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));
}

// —— 快照比 SVN 提交新（review 探针 P2）——

/// 把失焦快照的 JSON 改一下，造旧格式或坏掉的 `baseOid`。
fn edit_mark(fx: &SvnFixture, f: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>)) {
    let path = mark_path(&fx.wc_str());
    let mut v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    f(v.as_object_mut().unwrap());
    std::fs::write(path, serde_json::to_vec(&v).unwrap()).unwrap();
}

#[test]
fn mark_records_the_baseline_it_was_taken_on() {
    let fx = hybrid_with_colleague();
    write_mark(&fx.wc_str()).unwrap();
    let mark = read_mark(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(
        mark.base_oid.as_deref(),
        Some(oid(&fx, "svn/base").as_str())
    );
}

#[test]
fn commit_made_after_the_baseline_was_published_is_not_superseded() {
    // P2a：基线已发布、分支落后，这时再提交的内容从没进过 SVN。快照比 SVN 提交新，
    // 「早于快照」推不出「早于这批 SVN 提交」
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    write(&fx, A, "a3-never-in-svn\n");
    let c = commit_all(&fx, "SVN 提交之后的新工作");
    write_mark(&fx.wc_str()).unwrap();

    let out = run(&fx);

    assert!(
        matches!(out, FollowOutcome::NeedsManual { .. }),
        "没进 SVN 的 commit 不能被换掉：{out:?}"
    );
    assert_eq!(oid(&fx, "HEAD"), c);
}

#[test]
fn stage_made_after_the_baseline_was_published_is_kept() {
    // P2b：同一形态的暂存
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    // 只提交 W：commit_all 的 `add -A` 会把磁盘上的 a2 也带进 commit，A 就不在 OLD→NEW 里了
    write(&fx, "Code/W.cs", "w\n");
    git(&fx, &["add", "Code/W.cs"]);
    git(&fx, &["commit", "-qm", "work"]);
    write(&fx, A, "a3-staged\n");
    git(&fx, &["add", A]);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, "a4-disk\n");

    let rep = done(run(&fx));

    assert_eq!(show(&fx, ":Code/A.cs"), "a3-staged");
    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
    assert!(rep.stage.discarded.is_empty());
}

#[test]
fn switching_to_a_lagging_branch_does_not_wipe_its_commit() {
    // P2c：feature 因冲突落后，切过去、InSync、失焦写快照、点跟进。C 从没进过 SVN
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, A, "a-feature-never-in-svn\n");
    let c = commit_all(&fx, "feature 上的真实工作");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    assert_eq!(oid(&fx, "feature"), c, "前提：feature 冲突，留在原地");
    git(&fx, &["checkout", "-q", "feature"]);
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
    write_mark(&fx.wc_str()).unwrap();

    let out = follow(&fx.wc_str(), "op2").unwrap();

    assert!(
        matches!(
            out,
            FollowOutcome::NeedsManual {
                stale_disk: false,
                ..
            }
        ),
        "应当走冲突出口（重放冲突先于磁盘核对）：{out:?}"
    );
    assert_eq!(oid(&fx, "HEAD"), c);
    assert_eq!(show(&fx, "HEAD:Code/A.cs"), "a-feature-never-in-svn");
}

#[test]
fn only_paths_that_reached_the_baseline_after_the_mark_are_superseded() {
    // C 同时改了 A、B。B 在快照之前就进了基线，A 在快照之后才提交：只有 A 能算中间版本，
    // B 的改动没进过 SVN，要照常重放（这里因此冲突，不能静默丢掉 b3）
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, B);
    rebuild_baseline(&fx.wc_str()).unwrap();
    write(&fx, A, "a2\n");
    write(&fx, B, "b3-never-in-svn\n");
    let c = commit_all(&fx, "两个文件");
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, "a3\n");
    tortoise_commit(&fx, A);

    let out = run(&fx);

    match out {
        FollowOutcome::NeedsManual { at, files, .. } => {
            assert_eq!(at, c);
            assert_eq!(files, vec![B.to_string()], "A 已换成 SVN 版本，只剩 B");
        }
        other => panic!("{other:?}"),
    }
}

/// `value` 为 `None` 时删掉字段，即旧格式快照。
fn set_mark_base(fx: &SvnFixture, value: Option<String>) {
    edit_mark(fx, |m| match value {
        Some(v) => {
            m.insert("baseOid".to_string(), v.into());
        }
        None => {
            m.remove("baseOid");
        }
    });
}

#[test]
fn mark_without_a_trustworthy_baseline_is_uncertain() {
    // 旧格式没有 baseOid；读不到的、不是 NEW 祖先的同样分不清快照与 SVN 提交谁先谁后。
    // commit 正常重放（这里因此冲突），暂存留着
    type Base = fn(&SvnFixture) -> Option<String>;
    let cases: [(&str, Base); 3] = [
        ("旧格式", |_| None),
        ("读不到", |_| Some("1".repeat(40))),
        ("不是祖先", |fx| {
            let tree = oid(fx, "HEAD^{tree}");
            Some(git(fx, &["commit-tree", "-m", "别处", &tree]))
        }),
    ];
    for (name, base) in cases {
        let fx = hybrid_with_colleague();
        i_commit(&fx);
        write(&fx, A, "a2\n");
        commit_all(&fx, "中间版本");
        write(&fx, A, "a3\n");
        write_mark(&fx.wc_str()).unwrap();
        set_mark_base(&fx, base(&fx));
        tortoise_commit(&fx, A);
        assert!(
            matches!(run(&fx), FollowOutcome::NeedsManual { .. }),
            "{name}：commit 应当正常重放"
        );

        let fx = hybrid_with_colleague();
        i_commit(&fx);
        write(&fx, A, "a-mid\n");
        git(&fx, &["add", A]);
        write(&fx, A, "a-final\n");
        write_mark(&fx.wc_str()).unwrap();
        set_mark_base(&fx, base(&fx));
        tortoise_commit(&fx, A);
        let rep = done(run(&fx));
        assert_eq!(rep.stage.uncertain, vec![A.to_string()], "{name}");
        assert_eq!(show(&fx, ":Code/A.cs"), "a-mid", "{name}");
    }
}

#[test]
fn local_only_commit_survives_restore_after_commit() {
    // P1：L 加 log，磁盘另有功能 hunk，TortoiseSVN「提交后恢复」只提交功能 hunk。L 不是中间版本，
    // 换成 SVN 版本等于把 log 补丁静默扔掉，还没有告警
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    let base: String = (1..=12).map(|i| format!("l{i}\n")).collect();
    write(&fx, A, &base);
    tortoise_commit(&fx, A);
    done(run(&fx));
    let with_log = base.replace("l2\n", "l2\nlog()\n");
    write(&fx, A, &with_log);
    commit_all(&fx, L_MSG);
    let full = with_log.replace("l10\n", "l10\nfeature()\n");
    write(&fx, A, &full);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, &base.replace("l10\n", "l10\nfeature()\n"));
    tortoise_commit(&fx, A);
    write(&fx, A, &full);

    let rep = done(follow(&fx.wc_str(), "op2").unwrap());

    assert!(rep.dropped.is_empty(), "{rep:?}");
    assert!(rep.rewritten.is_empty(), "{rep:?}");
    assert!(rep.leaks.is_empty(), "{rep:?}");
    let head = show(&fx, "HEAD:Code/A.cs");
    assert!(
        head.contains("log()") && head.contains("feature()"),
        "{head}"
    );
    assert_eq!(
        git(&fx, &["log", "-1", "--format=%s", "HEAD"]),
        "本地补丁",
        "L 留在分支上"
    );
    assert!(modified_paths(&fx).is_empty());
}

#[test]
fn local_only_commit_that_conflicts_goes_to_the_manual_exit_instead_of_being_replaced() {
    // 泄漏不再靠 F2 ① 收掉：L 与 SVN 版本打架时走冲突出口
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a-log\n");
    commit_all(&fx, L_MSG);
    write(&fx, A, "a-svn\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);

    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));
}

#[test]
fn follow_keeps_stash_applicable() {
    // U4：跟进不动 stash，之后照样能 apply
    let fx = hybrid_with_colleague();
    write(&fx, B, "b-stashed\n");
    git(&fx, &["stash", "push", "-m", "wip"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let stash = oid(&fx, "refs/stash");

    done(run(&fx));

    assert_eq!(oid(&fx, "refs/stash"), stash);
    git(&fx, &["stash", "apply"]);
    assert_eq!(read(&fx, B), "b-stashed\n");
}

#[test]
fn follow_moves_feature_branch_by_object_replay() {
    // U5：feature 不是当前分支，对象层重放过去，不检出、不碰磁盘
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, "Code/F.cs", "f\n");
    let c = commit_all(&fx, "feature 的提交");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let snapshot = wc_snapshot(&fx);

    let rep = done(run(&fx));

    assert_eq!(wc_snapshot(&fx), snapshot);
    assert_eq!(rep.moved_branches, vec!["feature".to_string()]);
    assert_eq!(oid(&fx, "feature^"), oid(&fx, "svn/base"));
    assert_ne!(oid(&fx, "feature"), c);
    assert_eq!(show(&fx, "feature:Code/F.cs"), "f");
    assert_eq!(show(&fx, "feature:Code/A.cs"), "a2");
    assert_eq!(
        git(&fx, &["log", "-1", "--format=%s", "feature"]),
        "feature 的提交"
    );
}

#[test]
fn follow_moves_branch_without_own_commits() {
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "plain"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.moved_branches, vec!["plain".to_string()]);
    assert_eq!(oid(&fx, "plain"), oid(&fx, "svn/base"));
}

#[test]
fn branches_sharing_a_commit_share_its_replay() {
    // R20：不然 commit_like 取当前时间，会得到 OID 不同的重复 commit
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "-b", "one"]);
    write(&fx, "Code/S.cs", "shared\n");
    commit_all(&fx, "共享的 commit");
    git(&fx, &["checkout", "-q", "-b", "two"]);
    write(&fx, "Code/T.cs", "t\n");
    commit_all(&fx, "two 自己的");
    git(&fx, &["checkout", "-q", "one"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(
        rep.moved_branches,
        vec!["two".to_string(), "work".to_string()]
    );
    assert_eq!(
        oid(&fx, "two^"),
        oid(&fx, "one"),
        "共享的 commit 只重放一次"
    );
}

#[test]
fn follow_marks_conflicting_branch_behind_without_blocking() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, A, "a-feature\n");
    let tip = commit_all(&fx, "feature 改了 A");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "feature"), tip, "冲突的分支不动");
    assert_eq!(rep.behind_branches.len(), 1);
    assert_eq!(rep.behind_branches[0].0, "feature");
    assert!(rep.behind_branches[0].1.contains("冲突"));
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"), "当前分支照常跟进");
}

#[test]
fn branch_with_merge_is_marked_behind() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    write(&fx, "Code/S.cs", "s\n");
    commit_all(&fx, "side");
    git(&fx, &["checkout", "-q", "feature"]);
    write(&fx, "Code/F.cs", "f\n");
    commit_all(&fx, "feature");
    git(&fx, &["merge", "-q", "--no-ff", "-m", "merge", "side"]);
    let tip = oid(&fx, "feature");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "feature"), tip);
    let reasons: BTreeMap<_, _> = rep.behind_branches.iter().cloned().collect();
    assert!(reasons["feature"].contains("merge"), "{reasons:?}");
}

#[test]
fn replay_failure_on_another_branch_marks_it_behind_without_blocking() {
    // 不是冲突，是重放本身报错：我提交的路径在 feature 上是目录，只能按文件换（W19）。
    // 这条分支的问题不能让当前分支跟进不了
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, "Code/X.cs/y.cs", "y\n");
    let tip = commit_all(&fx, "feature 上它是目录");
    write_mark(&fx.wc_str()).unwrap();
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, "Code/X.cs", "x\n");
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", "Code/X.cs"]);
    tortoise_commit(&fx, "Code/X.cs");

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "feature"), tip);
    assert_eq!(rep.behind_branches[0].0, "feature");
    assert!(rep.behind_branches[0].1.contains("不是文件"), "{rep:?}");
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
}

#[test]
fn branches_in_other_worktrees_or_with_upstream_are_skipped() {
    // §6.11.3：挡不住外部工具加 upstream、worktree，只能跳过
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "tracked"]);
    git(&fx, &["config", "branch.tracked.remote", "."]);
    git(&fx, &["config", "branch.tracked.merge", "refs/heads/work"]);
    git(&fx, &["branch", "there"]);
    let there = fx.path().join("wt-there");
    git(
        &fx,
        &["worktree", "add", "-q", &there.to_string_lossy(), "there"],
    );
    let (tracked, at_there) = (oid(&fx, "tracked"), oid(&fx, "there"));
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "tracked"), tracked);
    assert_eq!(oid(&fx, "there"), at_there);
    assert!(rep.moved_branches.is_empty());
    assert!(rep.behind_branches.is_empty());
}

#[test]
fn branch_based_on_an_older_baseline_is_left_alone() {
    // §6.11.3：范围是包含旧基线的分支。基于更早基线的本来就落后，这次不动
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "older"]);
    let at = oid(&fx, "older");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    assert_eq!(oid(&fx, "older"), oid(&fx, "svn/base"), "第一次一并移走");
    git(&fx, &["branch", "-f", "older", &at]);
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, B);

    let rep = done(follow(&fx.wc_str(), "op2").unwrap());

    assert_eq!(oid(&fx, "older"), at);
    assert!(rep.moved_branches.is_empty());
}

#[test]
fn lagging_current_branch_does_not_drag_branches_older_than_the_old_baseline() {
    // §6.11.3 的范围是包含旧 svn/base 的分支。当前分支落后时 merge-base 比旧基线更早，拿它判
    // 范围会把只含更早基线的分支也移走
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "older"]);
    let at = oid(&fx, "older");
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    git(&fx, &["branch", "fresh", "svn/base"]);
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, B);

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "older"), at, "只含更早基线的分支不动");
    assert_eq!(oid(&fx, "fresh"), oid(&fx, "svn/base"), "包含旧基线的跟上");
    assert_eq!(rep.moved_branches, vec!["fresh".to_string()]);
    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"), "当前分支照常跟上");
}

#[test]
fn branch_entry_scopes_other_branches_by_the_previous_baseline() {
    // 分支跟进入口没有「这一次的旧基线」，等价的是 svn/base 的父提交：被中断的那次跟进本该移动
    // 的就是包含它的分支
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "older"]);
    let at = oid(&fx, "older");
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    git(&fx, &["branch", "fresh", "svn/base"]);
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, B);
    rebuild_baseline(&fx.wc_str()).unwrap();

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "older"), at);
    assert_eq!(oid(&fx, "fresh"), oid(&fx, "svn/base"));
    assert_eq!(rep.moved_branches, vec!["fresh".to_string()]);
}

#[test]
fn branch_already_on_the_new_baseline_is_left_alone_by_the_branch_entry() {
    // 分支跟进入口上 NEW 是现成的，别的分支可能已经在它上面了，重放只会造出重复 commit
    let fx = hybrid_with_colleague();
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    git(&fx, &["branch", "ahead", "svn/base"]);
    let wc = fx.wc_str();
    let ahead = run_git(
        &wc,
        &["commit-tree", "-m", "ahead", "-p", "ahead", "ahead^{tree}"],
    )
    .unwrap();
    git(&fx, &["update-ref", "refs/heads/ahead", ahead.trim()]);
    let at = oid(&fx, "ahead");

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "ahead"), at);
    assert!(!rep.moved_branches.contains(&"ahead".to_string()));
}

#[test]
fn branch_checked_out_after_the_rename_is_not_moved() {
    // 发布顺序改了之后其他分支在 rename 之后才移，这时 index.lock 已经放开（日志 Task 10）
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "plain"]);
    let at = oid(&fx, "plain");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let there = fx.path().join("wt-late").to_string_lossy().to_string();
    let _hook = fault::call_at(Point::IndexSwapped, move || {
        run_git(&wc, &["worktree", "add", "-q", &there, "plain"]).unwrap();
    });

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "plain"), at, "已检出的分支不能被移走");
    assert_eq!(rep.behind_branches[0].0, "plain");
    assert!(rep.behind_branches[0].1.contains("检出"));
    assert!(rep.moved_branches.is_empty());
}

#[test]
fn branch_moved_after_the_rename_is_not_overwritten() {
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "plain"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let moved = run_git(
        &wc,
        &["commit-tree", "-m", "外部", "-p", "plain", "plain^{tree}"],
    )
    .unwrap()
    .trim()
    .to_string();
    let target = moved.clone();
    let _hook = fault::call_at(Point::IndexSwapped, move || {
        run_git(&wc, &["update-ref", "refs/heads/plain", &target]).unwrap();
    });

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "plain"), moved);
    assert_eq!(rep.behind_branches[0].0, "plain");
    assert!(rep.moved_branches.is_empty());
}

#[test]
fn follow_after_external_no_ff_merge_uses_all_committed_shortcut() {
    // U5：merge 回来之后全部提交到 SVN。区间里有 merge commit，但不需要线性
    let fx = hybrid_with_colleague();
    let base = oid(&fx, "svn/base");
    git(&fx, &["checkout", "-q", "-b", "topic"]);
    write(&fx, A, "a2\n");
    commit_all(&fx, "topic");
    git(&fx, &["checkout", "-q", "work"]);
    git(
        &fx,
        &["merge", "-q", "--no-ff", "-m", "merge topic", "topic"],
    );
    let head = oid(&fx, "HEAD");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
    assert_ne!(oid(&fx, "HEAD"), head);
    assert_eq!(oid(&fx, "svn/base^"), base);
    assert_eq!(rep.dropped.len(), 2, "merge 与 topic 的 commit 都收掉了");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
    assert!(
        rep.moved_branches.contains(&"topic".to_string()),
        "topic 同样全部提交了：{rep:?}"
    );
}

#[test]
fn follow_refuses_a_merge_that_is_not_fully_committed() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "-b", "topic"]);
    write(&fx, "Code/T.cs", "t\n");
    commit_all(&fx, "topic");
    git(&fx, &["checkout", "-q", "work"]);
    git(
        &fx,
        &["merge", "-q", "--no-ff", "-m", "merge topic", "topic"],
    );
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let (head, base) = (oid(&fx, "HEAD"), oid(&fx, "svn/base"));

    let (code, _) = refused(run(&fx));

    assert_eq!(code, "svn_follow_merge_in_range");
    assert_eq!(oid(&fx, "HEAD"), head);
    assert_eq!(oid(&fx, "svn/base"), base, "什么都不发布");
    assert_no_residue(&fx);
}

#[test]
fn follow_never_writes_working_tree() {
    let fx = hybrid_with_colleague();
    set_me(&fx, "me");
    // 磁盘上什么形态都有：提交过的、暂存的、未暂存的、未跟踪的、白名单外的
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, B, "b-disk\n");
    write(&fx, "Code/Untracked.cs", "u\n");
    write(&fx, "Art/y.psd", "psd\n");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    fx.colleague_commits("Code/C.cs", "c\n");
    svn_update(&fx, "Code/C.cs");
    let before = wc_snapshot(&fx);

    done(run(&fx));

    let after = wc_snapshot(&fx);
    assert_eq!(after.len(), before.len());
    for (path, was) in &before {
        assert_eq!(after.get(path), Some(was), "{path} 被写过");
    }
}

#[test]
fn follow_works_when_whitelist_is_inside_an_external() {
    // Review Focus 1：往 external 提交只写它自己的 wc.db，键是仓库相对路径
    let fx = hybrid_whitelist_in_external();
    write(&fx, "code/A.cs", "a2\n");
    svn_cmd(
        &fx.wc().join("code"),
        &["commit", "-m", "x", "--non-interactive", "--", "A.cs"],
    );

    done(run(&fx));

    assert_eq!(show(&fx, "svn/base:code/A.cs"), "a2");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn follow_handles_paths_with_spaces_and_percent() {
    // Review Focus 2。中文路径过不了开发机代码页下的 svn 命令行（Task 4 实测），这里用空格、%、[]
    let fx = hybrid_with_colleague();
    let odd = "Code/sp ace %41[x].cs";
    write(&fx, odd, "o1\n");
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", odd]);
    let r = tortoise_commit(&fx, odd);

    let rep = done(run(&fx));

    assert_eq!(rep.changes[0].path, odd);
    assert_eq!(
        read_base_map(&fx.wc_str(), "svn/base").unwrap().rev_of(odd),
        r
    );
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn follow_records_a_tombstone_for_a_committed_deletion() {
    let fx = hybrid_with_colleague();
    svn_cmd(&fx.wc(), &["delete", "--non-interactive", "--", B]);
    let r = tortoise_commit(&fx, B);

    let rep = done(run(&fx));

    assert_eq!(rep.removed, vec![B.to_string()]);
    assert!(git(&fx, &["ls-tree", "-r", "--name-only", "svn/base"])
        .lines()
        .all(|l| l != B));
    let map = read_base_map(&fx.wc_str(), "svn/base").unwrap();
    assert_eq!(map.gone.get(B), Some(&r));
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
    let msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    assert!(msg.contains("1 个文件已从 SVN 删除"), "{msg}");
}

// —— 暂存（分析 §6.11.1）——

#[test]
fn stage_older_than_the_mark_is_discarded_for_own_commit() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a-mid\n");
    git(&fx, &["add", A]);
    write(&fx, A, "a-final\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.stage.discarded, vec![A.to_string()]);
    assert!(staged_paths(&fx).is_empty());
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
    let pin = "refs/mygitclient/follow/op1:index";
    assert!(!git(&fx, &["rev-parse", pin]).is_empty(), "旧索引钉住了");
}

#[test]
fn lost_stage_stays_recoverable_after_gc() {
    // P6：暂存内容只在索引里，没有任何 commit 指着它。钉住原始索引不等于钉住它引用的 blob
    let fx = hybrid_with_colleague();
    set_me(&fx, "somebody-else");
    write(&fx, A, "a-staged-only-here\n");
    git(&fx, &["add", A]);
    let blob = git(&fx, &["rev-parse", ":Code/A.cs"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.stage.lost, vec![A.to_string()]);
    let pin = rep.pinned_ref.expect("报告要点名钉住的 ref");
    assert_eq!(pin, "refs/mygitclient/follow/op1");
    git(&fx, &["gc", "--prune=now", "-q"]);
    assert_eq!(
        git(&fx, &["cat-file", "blob", &format!("{pin}:blobs/{blob}")]),
        "a-staged-only-here"
    );
    assert_eq!(git(&fx, &["cat-file", "-t", &blob]), "blob");
}

#[test]
fn stage_after_the_mark_is_kept_and_reported_uncertain() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, "a-final\n");
    tortoise_commit(&fx, A);
    write(&fx, A, "a-after\n");
    git(&fx, &["add", A]);

    let rep = done(run(&fx));

    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
    assert_eq!(show(&fx, ":Code/A.cs"), "a-after");
}

#[test]
fn stage_without_mark_is_kept_and_reported_uncertain() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a-final\n");
    tortoise_commit(&fx, A);
    write(&fx, A, "a-after\n");
    git(&fx, &["add", A]);

    let rep = done(run(&fx));

    assert_eq!(rep.stage.uncertain, vec![A.to_string()]);
}

#[test]
fn staged_delete_matches_the_mark_by_zero_oid() {
    // 暂存删除在快照里记全 0 oid，当前索引里没有这条路径时要按全 0 比
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["rm", "-q", "--cached", A]);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, A, "a-final\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.stage.discarded, vec![A.to_string()]);
    assert_eq!(show(&fx, ":Code/A.cs"), "a-final");
}

#[test]
fn colleague_change_merges_with_staged_content() {
    // W13：局部 update 拉下同事改动，暂存内容合进去
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "1\n2\n3\n4\n5\n6\n7\n8\n");
    tortoise_commit(&fx, A);
    done(follow(&fx.wc_str(), "seed").unwrap());

    fx.colleague_commits(A, "1\n2\n3\n4\n5\n6\n7\ncolleague\n");
    let r = youngest(&fx);
    set_author(&fx, r, "colleague");
    write(&fx, A, "mine\n2\n3\n4\n5\n6\n7\n8\n");
    git(&fx, &["add", A]);
    svn_update(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.stage.decisions.len(), 1);
    assert_eq!(show(&fx, ":Code/A.cs"), "mine\n2\n3\n4\n5\n6\n7\ncolleague");
    assert!(rep.stage.lost.is_empty());
}

// —— 拒绝：什么都不动 ——

/// 全部 ref 与索引的指纹。拒绝时一个字节都不能变。
fn state(fx: &SvnFixture) -> (String, Vec<u8>) {
    (
        git(fx, &["for-each-ref"]),
        std::fs::read(fx.wc().join(".git/index")).unwrap(),
    )
}

#[test]
fn follow_refuses_when_svn_conflicted() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a-colleague\n");
    write(&fx, A, "a-mine\n");
    svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive"],
    );
    let before = state(&fx);

    let (code, _) = refused(run(&fx));

    assert_eq!(code, "svn_preflight_conflicted");
    assert_eq!(state(&fx), before);
    assert_no_residue(&fx);
}

#[cfg(windows)]
#[test]
fn follow_refuses_when_wc_locked() {
    // 只对子目录跑的 update 半途失败：E15，分不出活锁与陈旧锁
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    fx.colleague_commits(A, "a-colleague\n");
    {
        let _hold = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(fx.wc().join(A))
            .unwrap();
        let ok = Command::new("svn")
            .current_dir(fx.wc())
            .args(["update", "--non-interactive", "--", "Code@"])
            .output()
            .unwrap()
            .status
            .success();
        assert!(!ok, "前提：被独占的文件让 update 半途失败");
    }
    let before = state(&fx);

    let (code, _) = refused(run(&fx));

    assert_eq!(code, "svn_follow_wc_locked");
    assert_eq!(state(&fx), before);
}

#[test]
fn follow_refuses_when_there_is_nothing_to_follow() {
    let fx = hybrid_with_colleague();
    let before = state(&fx);

    let (code, _) = refused(run(&fx));

    assert_eq!(code, "svn_follow_nothing_to_do");
    assert_eq!(state(&fx), before);
}

#[test]
fn follow_refuses_detached_head_and_the_baseline_branch() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    git(&fx, &["checkout", "-q", "--detach"]);
    assert_eq!(refused(run(&fx)).0, "svn_follow_detached_head");
    git(&fx, &["checkout", "-q", "svn/base"]);
    assert_eq!(refused(run(&fx)).0, "svn_follow_on_baseline");
}

#[test]
fn follow_refuses_an_unfinished_journal_and_a_held_lock() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    {
        let _held = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
        assert_eq!(refused(run(&fx)).0, "sync_in_progress");
    }
    svn_journal::write(
        &fx.wc_str(),
        &Journal {
            opid: "x".to_string(),
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(refused(run(&fx)).0, "svn_follow_unfinished");
}

#[test]
fn follow_refuses_while_git_is_busy() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    std::fs::write(fx.wc().join(".git/MERGE_HEAD"), oid(&fx, "HEAD")).unwrap();
    assert_eq!(refused(run(&fx)).0, "svn_follow_git_busy");
    std::fs::remove_file(fx.wc().join(".git/MERGE_HEAD")).unwrap();

    std::fs::write(index_lock(&fx), "别人的").unwrap();
    assert_eq!(refused(run(&fx)).0, "svn_index_busy");
    assert_eq!(std::fs::read(index_lock(&fx)).unwrap(), "别人的".as_bytes());
}

#[test]
fn git_older_than_2_43_is_refused_before_anything_is_recorded() {
    // D9：merge-file --object-id 要 2.43。在基线发布之后才报就晚了
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    tortoise_commit(&fx, A);
    let before = state(&fx);
    let objects = git(&fx, &["count-objects"]);
    let _old = fault::pretend_git_version("git version 2.42.3.windows.1");

    let (code, msg) = refused(run(&fx));

    assert_eq!(code, "svn_follow_git_too_old");
    assert!(msg.contains("2.43"), "{msg}");
    assert_eq!(state(&fx), before);
    assert_eq!(git(&fx, &["count-objects"]), objects, "F1 还没开始");
}

#[test]
fn git_version_text_is_parsed_with_the_minimum_at_2_43() {
    for ok in [
        "git version 2.43.0",
        "git version 2.51.2.windows.1",
        "git version 3.0.0",
        "git version 2.43.0-rc1",
    ] {
        assert!(git_version_ok(ok), "{ok}");
    }
    for bad in [
        "git version 2.42.3.windows.1",
        "git version 2.9.0",
        "git version 1.99.0",
        "",
        "not git",
    ] {
        assert!(!git_version_ok(bad), "{bad}");
    }
}

#[test]
fn follow_not_on_a_hybrid_repo_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (code, _) = refused(follow(&dir.path().to_string_lossy(), "x").unwrap());
    assert_eq!(code, "svn_not_hybrid");
}

// —— F3 ——

const L_MSG: &str = "本地补丁\n\nLocal-Only: true";

#[test]
fn local_only_leak_on_current_branch_is_reported_and_branch_still_moves() {
    // 手动跟进不拦（§6.12 条件 8 只管自动跟进），但必须告诉用户
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-log\n");
    let l = commit_all(&fx, L_MSG);
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(rep.leaks.len(), 1);
    assert_eq!(rep.leaks[0].commit, l);
    assert_eq!(rep.leaks[0].branch, "work");
    assert_eq!(rep.leaks[0].path, A);
    assert_eq!(rep.leaks[0].hunks[0].verdict, LeakVerdict::Present);
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
}

#[test]
fn local_only_leak_on_other_branch_keeps_it_behind() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, A, "a-log\n");
    let l = commit_all(&fx, L_MSG);
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, A, "a-log\n");
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "feature"), l, "命中 F3 的分支不动");
    assert_eq!(rep.leaks[0].branch, "feature");
    assert_eq!(rep.behind_branches[0].0, "feature");
    assert!(rep.behind_branches[0].1.contains("Local-Only"));
}

#[test]
fn all_committed_shortcut_does_not_swallow_a_local_only_commit() {
    // Local-Only 被带上服务器时内容也对得上，不能走捷径静默收掉（Task 6 的 all_committed）
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-log\n");
    commit_all(&fx, L_MSG);
    tortoise_commit(&fx, A);

    let rep = done(run(&fx));

    assert!(!rep.leaks.is_empty());
}

// —— 当前分支冲突（手动跟进归 Task 13）——

#[test]
fn follow_conflict_publishes_only_baseline() {
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    git(&fx, &["branch", "plain"]);
    write(&fx, A, "a-git\n");
    let c = commit_all(&fx, "git 里的版本");
    write(&fx, A, "a-svn\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);
    let index = std::fs::read(fx.wc().join(".git/index")).unwrap();

    let out = run(&fx);

    match out {
        FollowOutcome::NeedsManual {
            at,
            files,
            baseline_published,
            leaks,
            stale_disk,
        } => {
            assert_eq!(at, c);
            assert_eq!(files, vec![A.to_string()]);
            assert!(baseline_published);
            assert!(leaks.is_empty());
            assert!(!stale_disk, "是重放冲突");
        }
        other => panic!("{other:?}"),
    }
    assert_ne!(oid(&fx, "svn/base"), old_base, "基线发布了");
    assert_eq!(oid(&fx, "HEAD"), c, "分支不动");
    assert_eq!(oid(&fx, "plain"), old_base, "其他分支也不动");
    assert_eq!(std::fs::read(fx.wc().join(".git/index")).unwrap(), index);
    assert!(
        read_mark(&fx.wc_str()).unwrap().is_some(),
        "没跟进完，快照留着"
    );
    assert_no_residue(&fx);

    // 再点一次走分支跟进入口，同样的冲突、同样的出口
    match follow(&fx.wc_str(), "op2").unwrap() {
        FollowOutcome::NeedsManual {
            baseline_published, ..
        } => assert!(baseline_published),
        other => panic!("{other:?}"),
    }
}

// —— 分支跟进入口 ——

#[test]
fn sync_status_says_when_the_branch_lags_the_baseline() {
    let fx = hybrid_with_colleague();
    let behind = |fx: &SvnFixture| check_sync(&fx.wc_str()).unwrap().head_behind_base;
    assert!(!behind(&fx), "分支就在基线上");
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    assert!(!behind(&fx), "分支在基线之前");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    assert!(!behind(&fx), "基线还没动，判定是 ExternallyAdvanced");
    rebuild_baseline(&fx.wc_str()).unwrap();
    assert!(behind(&fx), "基线发布了，分支还没跟上");

    // 跟进入口只对附着的分支开放：detached 给了按钮也只会被拒绝
    git(&fx, &["checkout", "-q", "--detach"]);
    assert!(!behind(&fx));
    git(&fx, &["checkout", "-q", "work"]);

    done(run(&fx));
    assert!(!behind(&fx), "跟上了");

    // 分叉点不是基线 commit：跟进会报 svn_follow_branch_unrelated，不给入口
    let msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    let tree = oid(&fx, "svn/base^{tree}");
    let x = git(
        &fx,
        &["commit-tree", "-m", "普通 commit", "-p", "svn/base", &tree],
    );
    let base = git(&fx, &["commit-tree", "-m", &msg, "-p", &x, &tree]);
    let side = git(&fx, &["commit-tree", "-m", "侧枝", "-p", &x, &tree]);
    git(&fx, &["update-ref", "refs/heads/svn/base", &base]);
    git(&fx, &["update-ref", "refs/heads/side", &side]);
    git(&fx, &["symbolic-ref", "HEAD", "refs/heads/side"]);
    assert!(!behind(&fx));

    // 无关历史没有 merge-base
    let lone = git(&fx, &["commit-tree", "-m", "无关", &tree]);
    git(&fx, &["update-ref", "refs/heads/side", &lone]);
    assert!(!behind(&fx));
}

#[test]
fn branch_follow_entry_moves_lagging_branch() {
    // 基线已前进、分支没跟上：这里用重建基线造出来（F5 ① 之后崩溃、用户放弃都是同一形态）
    let fx = hybrid_with_colleague();
    let me = i_commit(&fx);
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, A, "a2\n");
    let r = tortoise_commit(&fx, A);
    let old_base = oid(&fx, "svn/base");
    rebuild_baseline(&fx.wc_str()).unwrap();
    let new_base = oid(&fx, "svn/base");
    assert_ne!(new_base, old_base);
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
    assert_eq!(
        git(&fx, &["merge-base", "HEAD", "svn/base"]),
        old_base,
        "前提：分支落后"
    );

    let rep = done(run(&fx));

    assert_eq!(oid(&fx, "svn/base"), new_base, "不录新基线");
    assert_eq!(oid(&fx, "HEAD^"), new_base);
    assert_eq!(show(&fx, "HEAD:Code/W.cs"), "w");
    assert_eq!(staged_paths(&fx), vec![B]);
    assert!(modified_paths(&fx).is_empty());
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(rep.changes[0].path, A);
    assert_eq!(rep.changes[0].rev, r);
    assert_eq!(rep.changes[0].author.as_deref(), Some(me.as_str()));
    assert_eq!(rep.new_floor, 1);
}

#[test]
fn branch_entry_sends_a_stale_disk_to_manual_follow() {
    // Review 3 第 2 条：「仅切换」或外部工具切到落后的分支，git 把旧内容写回磁盘（SVN 看着已修改）。
    // 对象层跟进只移 HEAD、不写磁盘，「本地改动」就成了把 SVN 的内容改回去，commit -a 造出回退
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    // 跟进会顺带移动包含旧基线的分支，落后的分支要在它之后才有
    git(&fx, &["checkout", "-q", "-b", "old", &old_base]);
    assert_eq!(read(&fx, A), "a1\n", "前提：磁盘写回了旧内容");
    let status = check_sync(&fx.wc_str()).unwrap();
    assert!(matches!(status.verdict, SyncVerdict::InSync));
    assert!(status.head_behind_base);
    let head = oid(&fx, "HEAD");
    let index = std::fs::read(fx.wc().join(".git/index")).unwrap();

    match run(&fx) {
        FollowOutcome::NeedsManual {
            at,
            files,
            baseline_published,
            stale_disk,
            ..
        } => {
            assert_eq!(at, head);
            assert_eq!(files, vec![A.to_string()]);
            assert!(baseline_published, "分支跟进入口的基线本来就在");
            assert!(stale_disk);
        }
        other => panic!("应当改走手动跟进：{other:?}"),
    }
    assert_eq!(oid(&fx, "HEAD"), head, "HEAD 不动");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "old");
    assert_eq!(std::fs::read(fx.wc().join(".git/index")).unwrap(), index);
    assert_eq!(read(&fx, A), "a1\n");
    assert_no_residue(&fx);

    // 手动跟进的 rebase 会把新内容写回磁盘，结束时干净
    assert!(matches!(manual(&fx), SyncInOutcome::Done { .. }));
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
    assert_eq!(read(&fx, A), "a2\n");
    assert_eq!(porcelain(&fx), "");
}

#[test]
fn a_stale_disk_on_a_lagging_branch_also_stops_a_recording_follow() {
    // 同一个现场再来一次外部提交（判定 ExternallyAdvanced）：基线照常发布，分支交给手动跟进。
    // 只核对分支落后的那一段：这次 svn update 进来的 B 磁盘上是新内容，不在名单里
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    let published = oid(&fx, "svn/base");
    git(&fx, &["checkout", "-q", "-b", "old", &old_base]);
    fx.colleague_commits(B, "b2\n");
    svn_update(&fx, B);
    let head = oid(&fx, "HEAD");

    match run(&fx) {
        FollowOutcome::NeedsManual {
            files, stale_disk, ..
        } => {
            assert_eq!(files, vec![A.to_string()]);
            assert!(stale_disk);
        }
        other => panic!("应当改走手动跟进：{other:?}"),
    }
    assert_ne!(oid(&fx, "svn/base"), published, "基线发布了");
    assert_eq!(oid(&fx, "HEAD"), head, "HEAD 不动");
    assert_no_residue(&fx);

    assert!(matches!(manual(&fx), SyncInOutcome::Done { .. }));
    assert_eq!((read(&fx, A), read(&fx, B)), ("a2\n".into(), "b2\n".into()));
    assert_eq!(porcelain(&fx), "");
}

#[test]
fn a_file_the_lagging_branch_does_not_have_counts_as_stale_when_it_is_gone_from_disk() {
    // 基线新增的文件，旧分支里没有：切过去时 git 把它从磁盘上删掉。只移 HEAD 的话它显示成「已删除」
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    let c = "Code/C.cs";
    write(&fx, c, "c\n");
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", c]);
    tortoise_commit(&fx, c);
    done(run(&fx));
    git(&fx, &["checkout", "-q", "-b", "old", &old_base]);
    assert!(!fx.wc().join(c).exists(), "前提：git 删掉了它");
    let head = oid(&fx, "HEAD");

    match run(&fx) {
        FollowOutcome::NeedsManual {
            files, stale_disk, ..
        } => {
            assert_eq!(files, vec![c.to_string()]);
            assert!(stale_disk);
        }
        other => panic!("应当改走手动跟进：{other:?}"),
    }
    assert_eq!(oid(&fx, "HEAD"), head);

    assert!(matches!(manual(&fx), SyncInOutcome::Done { .. }));
    assert_eq!(read(&fx, c), "c\n");
    assert_eq!(porcelain(&fx), "");
}

#[test]
fn branch_entry_refuses_when_svn_conflicted() {
    // 这个入口不录制，R1 那道检查要自己补。冲突放在非 W-file 上：判定看不见它，仍是 InSync
    let fx = hybrid_with_colleague();
    let x = "Code/notes.txt";
    write(&fx, x, "x1\n");
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", x]);
    tortoise_commit(&fx, x);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    fx.colleague_commits(x, "theirs\n");
    write(&fx, x, "mine\n");
    svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive"],
    );
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
    assert_ne!(
        git(&fx, &["merge-base", "HEAD", "svn/base"]),
        oid(&fx, "svn/base")
    );
    let before = state(&fx);

    let (code, _) = refused(run(&fx));

    assert_eq!(code, "svn_preflight_conflicted");
    assert_eq!(state(&fx), before);
}

#[test]
fn lagging_branch_still_supersedes_paths_from_the_earlier_baseline() {
    // A 是上一个基线带进来的，这次的外部变更只有 B。只看 svn/base..NEW 的话 F2 ① 看不到 A，
    // 中间版本 a2 会撞上 SVN 的 a3，变成一次不必要的冲突
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a2\n");
    let c = commit_all(&fx, "中间版本");
    write(&fx, A, "a3\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, B);

    let rep = done(run(&fx));

    assert_eq!(rep.dropped, vec![c]);
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[test]
fn new_external_commit_while_branch_lags_replays_onto_the_new_baseline() {
    // 上一次只发布了基线，分支还在更早的基线上，这时又来一个外部提交：走普通入口
    let fx = hybrid_with_colleague();
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, B);

    done(run(&fx));

    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"));
    assert_eq!(show(&fx, "HEAD:Code/W.cs"), "w");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

// —— 准备阶段与发布之间的变化 ——

#[test]
fn a_commit_during_preparation_is_picked_up_by_a_retry() {
    // F4：HEAD 变了就回到 F1，最多重来一次
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let mut once = true;
    let _hook = fault::call_at(Point::Prepared, move || {
        if once {
            once = false;
            std::fs::write(Path::new(&wc).join("Code/Race.cs"), "race\n").unwrap();
            run_git(&wc, &["add", "--", "Code/Race.cs"]).unwrap();
            run_git(&wc, &["commit", "-qm", "准备期间的提交"]).unwrap();
        }
    });

    done(run(&fx));

    assert_eq!(show(&fx, "HEAD:Code/Race.cs"), "race");
    assert_eq!(oid(&fx, "HEAD^"), oid(&fx, "svn/base"));
    assert_no_residue(&fx);
}

#[test]
fn state_that_keeps_changing_is_refused_after_one_retry() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let base = oid(&fx, "svn/base");
    let wc = fx.wc_str();
    let mut n = 0;
    let _hook = fault::call_at(Point::Prepared, move || {
        n += 1;
        run_git(
            &wc,
            &["commit", "-q", "--allow-empty", "-m", &format!("抢跑 {n}")],
        )
        .unwrap();
    });

    let (code, _) = refused(run(&fx));

    assert_eq!(code, "svn_follow_state_changed");
    assert_eq!(oid(&fx, "svn/base"), base);
    assert_no_residue(&fx);
}

#[test]
fn a_mapping_change_during_preparation_is_detected_in_the_lock() {
    // R3c 的第三道：录完之后又提交了一次
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    write(&fx, B, "b2\n");
    tortoise_commit(&fx, A);
    let wc = std::path::PathBuf::from(fx.wc_str());
    let mut once = true;
    let _hook = fault::call_at(Point::Prepared, move || {
        if once {
            once = false;
            svn_cmd(&wc, &["commit", "-m", "x", "--non-interactive", "--", B]);
        }
    });

    done(run(&fx));

    assert_eq!(show(&fx, "svn/base:Code/B.cs"), "b2", "重来时录进去了");
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

// —— 发布期间的竞态（Review Focus 4）——

#[test]
fn checkout_b_before_the_cas_is_caught_by_the_second_check_and_rolls_back() {
    // checkout -b 不受 index.lock 阻挡（W23；带 -q 时反而会撞锁，见日志 Task 10，这里不能加）。
    // 落在第一次核对与 CAS 之间：新分支建在 OLD，CAS 照样成功，第二次核对读到的 HEAD 不是
    // NEW_TIP，把分支移回去，索引不换。落在 CAS 之后的形态见下一条
    let fx = hybrid_with_colleague();
    let old_head = oid(&fx, "HEAD");
    let old_base = oid(&fx, "svn/base");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let index = std::fs::read(fx.wc().join(".git/index")).unwrap();
    let wc = fx.wc_str();
    let _hook = fault::call_at(Point::HeadChecked, move || {
        run_git(&wc, &["checkout", "-b", "race"]).unwrap();
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();

    assert_eq!(e.code, "svn_follow_head_moved", "{}", e.message);
    assert_eq!(oid(&fx, "work"), old_head, "分支 CAS 回去了");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "race");
    assert_ne!(oid(&fx, "svn/base"), old_base, "基线保持前进");
    assert_eq!(
        std::fs::read(fx.wc().join(".git/index")).unwrap(),
        index,
        "索引没换"
    );
    assert_no_residue(&fx);

    // 交给分支跟进入口：现在 HEAD 在 race 上
    drop(_hook);
    done(follow(&fx.wc_str(), "op2").unwrap());
    assert_eq!(oid(&fx, "race"), oid(&fx, "svn/base"));
    assert_eq!(oid(&fx, "work"), oid(&fx, "svn/base"), "work 成了其他分支");
    assert_eq!(staged_paths(&fx), vec![B]);
    assert!(modified_paths(&fx).is_empty());
}

#[test]
fn checkout_b_after_the_cas_goes_forward_and_keeps_the_stage() {
    // 真实窗口：CAS 与第二次核对之间。新分支建在 NEW_TIP 上，这时把 work 移回去、放弃索引就是
    // HEAD=race@NEW_TIP、索引是 OLD 内容、没有 index.lock：反向暂存，再跟进还报 nothing_to_do
    let fx = hybrid_with_colleague();
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let _hook = fault::call_at(Point::BranchMoved, move || {
        run_git(&wc, &["checkout", "-b", "race"]).unwrap();
    });

    done(run(&fx));

    let base = oid(&fx, "svn/base");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "race");
    assert_eq!(oid(&fx, "race"), base);
    assert_eq!(oid(&fx, "work"), base, "work 留在新位置");
    assert_eq!(staged_paths(&fx), vec![B], "没有反向暂存");
    assert_eq!(show(&fx, ":Code/B.cs"), "b-staged");
    assert_no_residue(&fx);
}

#[test]
fn head_pointed_at_another_commit_after_the_cas_rolls_back() {
    // symbolic-ref 同样绕得过 index.lock，这时 HEAD 不在 NEW_TIP 上，换入的索引就错了
    let fx = hybrid_with_colleague();
    let old_head = oid(&fx, "HEAD");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    git(&fx, &["branch", "other"]);
    let index = std::fs::read(fx.wc().join(".git/index")).unwrap();
    let wc = fx.wc_str();
    let _hook = fault::call_at(Point::BranchMoved, move || {
        run_git(&wc, &["symbolic-ref", "HEAD", "refs/heads/other"]).unwrap();
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();

    assert_eq!(e.code, "svn_follow_head_moved", "{}", e.message);
    assert_eq!(oid(&fx, "work"), old_head, "分支 CAS 回去了");
    assert_eq!(
        std::fs::read(fx.wc().join(".git/index")).unwrap(),
        index,
        "索引没换"
    );
    assert_no_residue(&fx);
}

#[test]
fn head_moved_before_the_branch_cas_leaves_the_branch_alone() {
    let fx = hybrid_with_colleague();
    let old_head = oid(&fx, "HEAD");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let _hook = fault::call_at(Point::IndexPrepared, move || {
        run_git(&wc, &["checkout", "-b", "race"]).unwrap();
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();

    assert_eq!(e.code, "svn_follow_head_moved");
    assert_eq!(oid(&fx, "work"), old_head);
    assert_no_residue(&fx);
}

#[test]
fn branch_moved_under_the_cas_rolls_back_cleanly() {
    // update-ref 不需要 index.lock（W23），外部能在 CAS 之前把分支挪走
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let moved = run_git(
        &wc,
        &["commit-tree", "-m", "外部", "-p", "HEAD", "HEAD^{tree}"],
    )
    .unwrap()
    .trim()
    .to_string();
    let target = moved.clone();
    let _hook = fault::call_at(Point::HeadChecked, move || {
        run_git(&wc, &["update-ref", "refs/heads/work", &target]).unwrap();
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();

    assert_eq!(e.code, "svn_follow_branch_moved", "{}", e.message);
    assert_eq!(oid(&fx, "work"), moved, "不覆盖外部的移动");
    assert_no_residue(&fx);
}

#[test]
fn prepare_failure_after_the_baseline_keeps_the_baseline_and_nothing_else() {
    // 发布顺序：基线先走。别的 git 进程恰好拿着索引锁时分支与索引都不能动
    let fx = hybrid_with_colleague();
    let old_head = oid(&fx, "HEAD");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let lock = index_lock(&fx);
    let foreign = lock.clone();
    let hook = fault::call_at(Point::BeforePrepare, move || {
        std::fs::write(&foreign, "别人的").unwrap();
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();
    drop(hook);

    assert_eq!(e.code, "svn_index_busy");
    assert_eq!(std::fs::read(&lock).unwrap(), "别人的".as_bytes());
    assert_eq!(oid(&fx, "HEAD"), old_head);
    assert_ne!(oid(&fx, "svn/base"), old_head);
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());

    std::fs::remove_file(&lock).unwrap();
    done(follow(&fx.wc_str(), "op2").unwrap());
    assert_eq!(oid(&fx, "HEAD"), oid(&fx, "svn/base"));
    assert!(git(&fx, &["status", "--porcelain"]).is_empty());
}

#[cfg(windows)]
#[test]
fn index_rename_failure_moves_the_branch_back_before_letting_go() {
    // W24：外部进程开着 .git/index 又没给 FILE_SHARE_DELETE
    use std::cell::RefCell;
    use std::os::windows::fs::OpenOptionsExt;
    use std::rc::Rc;
    let fx = hybrid_with_colleague();
    let old_head = oid(&fx, "HEAD");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let index = fx.wc().join(".git/index");
    let bytes = std::fs::read(&index).unwrap();
    let held = Rc::new(RefCell::new(None));
    let (slot, path) = (held.clone(), index.clone());
    let hook = fault::call_at(Point::BeforeSwap, move || {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .unwrap();
        *slot.borrow_mut() = Some(f);
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();
    drop(hook);
    held.borrow_mut().take();

    assert_eq!(e.code, "svn_follow_index_rename_failed", "{}", e.message);
    assert_eq!(oid(&fx, "work"), old_head);
    assert_eq!(std::fs::read(&index).unwrap(), bytes);
    assert_no_residue(&fx);

    done(follow(&fx.wc_str(), "op2").unwrap());
    assert_eq!(staged_paths(&fx), vec![B]);
    assert!(modified_paths(&fx).is_empty());
}

#[cfg(windows)]
#[test]
fn rename_failure_with_head_on_a_new_branch_keeps_the_lock_for_continue() {
    // 第二次核对之后才 checkout -b，rename 又失败。race 建在 NEW_TIP 上移不回去，这时放开锁就是
    // 反向暂存，只能留着锁和记录，等「继续」补做
    use std::cell::RefCell;
    use std::os::windows::fs::OpenOptionsExt;
    use std::rc::Rc;
    let fx = hybrid_with_colleague();
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let wc = fx.wc_str();
    let index = fx.wc().join(".git/index");
    let held = Rc::new(RefCell::new(None));
    let (slot, path) = (held.clone(), index.clone());
    let hook = fault::call_at(Point::BeforeSwap, move || {
        run_git(&wc, &["checkout", "-b", "race"]).unwrap();
        let f = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .unwrap();
        *slot.borrow_mut() = Some(f);
    });

    let e = follow(&fx.wc_str(), "op1").unwrap_err();
    drop(hook);
    held.borrow_mut().take();

    assert_eq!(e.code, "svn_follow_index_rename_failed", "{}", e.message);
    assert!(e.message.contains("继续"), "{}", e.message);
    assert!(index_lock(&fx).exists(), "锁留着挡住外部 git");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_some());

    done(resume_follow(&fx.wc_str()).unwrap());
    assert_eq!(staged_paths(&fx), vec![B]);
    assert_no_residue(&fx);
}

// —— 崩溃恢复（Review Focus 5）——

struct Scene {
    fx: SvnFixture,
    old_head: String,
}

/// 什么都有的现场：work 上有 commit、暂存里有存档、其他分支一条有 commit 一条没有。
fn crash_scene() -> Scene {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["branch", "plain"]);
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, "Code/F.cs", "f\n");
    commit_all(&fx, "feature");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, B, "b-disk\n");
    write(&fx, A, "a2\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);
    let old_head = oid(&fx, "HEAD");
    Scene { fx, old_head }
}

fn crash_at(s: &Scene, p: Point) {
    let armed = fault::crash_at(p);
    let wc = s.fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| follow(&wc, "op1")));
    drop(armed);
    let payload = match r {
        Err(p) => p,
        Ok(o) => panic!("没有走到崩溃点 {p:?}：{:?}", o.map_err(|e| e.message)),
    };
    assert!(payload.is::<Crashed>(), "{p:?} 处是别的 panic");
}

/// Review Focus 5：不能留下「分支已移、索引是旧的」并且 index.lock 不存在的状态。
fn assert_no_reverse_stage(s: &Scene, at: Point) {
    let fx = &s.fx;
    if oid(fx, "work") != s.old_head && !index_lock(fx).exists() {
        assert_eq!(
            staged_paths(fx),
            vec![B],
            "{at:?}：分支移了、索引还是旧的，外部提交会回退"
        );
    }
}

fn assert_finished(s: &Scene, at: Point) {
    let fx = &s.fx;
    let base = oid(fx, "svn/base");
    assert_no_residue(fx);
    assert_eq!(git(fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(oid(fx, "HEAD^"), base, "{at:?}");
    assert_eq!(show(fx, "HEAD:Code/W.cs"), "w", "{at:?}");
    assert_eq!(show(fx, "svn/base:Code/A.cs"), "a2", "{at:?}");
    assert_eq!(staged_paths(fx), vec![B], "{at:?}");
    assert_eq!(show(fx, ":Code/B.cs"), "b-staged", "{at:?}");
    assert_eq!(modified_paths(fx), vec![B], "{at:?}");
    assert_eq!(read(fx, B), "b-disk\n", "{at:?}");
    assert_eq!(oid(fx, "feature^"), base, "{at:?}");
    assert_eq!(oid(fx, "plain"), base, "{at:?}");
    assert!(read_mark(&fx.wc_str()).unwrap().is_none(), "{at:?}");
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
}

#[test]
fn resume_after_crash_between_each_step() {
    for at in [
        Point::Journaled,
        Point::BaseMoved,
        Point::BeforePrepare,
        Point::IndexLocked,
        Point::IndexPrepared,
        Point::HeadChecked,
        Point::BranchMoved,
        Point::BeforeSwap,
        Point::IndexSwapped,
        Point::OtherMoved,
        Point::BeforeFinish,
    ] {
        let s = crash_scene();
        crash_at(&s, at);
        assert!(
            svn_journal::read(&s.fx.wc_str()).unwrap().is_some(),
            "{at:?}：崩溃之后 journal 要在"
        );
        assert_no_reverse_stage(&s, at);

        let out = resume_follow(&s.fx.wc_str())
            .unwrap_or_else(|e| panic!("{at:?}：{}: {}", e.code, e.message));

        assert!(matches!(out, FollowOutcome::Done { .. }), "{at:?}：{out:?}");
        assert_finished(&s, at);
    }
}

#[test]
fn journal_tracks_the_step_before_it_runs() {
    let cases = [
        (Point::Journaled, FollowStep::Baseline),
        (Point::BaseMoved, FollowStep::Baseline),
        (Point::BeforePrepare, FollowStep::PrepareIndex),
        (Point::IndexPrepared, FollowStep::MoveBranch),
        (Point::BranchMoved, FollowStep::MoveBranch),
        (Point::BeforeSwap, FollowStep::SwapIndex),
        (Point::IndexSwapped, FollowStep::SwapIndex),
        (Point::OtherMoved, FollowStep::MoveOthers),
    ];
    // 一个现场只能崩一次，每个点换新现场
    for (at, step) in cases {
        let s = crash_scene();
        crash_at(&s, at);
        let j = svn_journal::read(&s.fx.wc_str()).unwrap().unwrap();
        assert_eq!(j.flow, Flow::Follow);
        assert_eq!(j.phase, Phase::FollowPublishing);
        let f = j.follow_steps.as_ref().unwrap();
        assert_eq!(f.step, step, "{at:?}");
        assert_eq!(j.orig_branch.as_deref(), Some("work"));
        assert_eq!(j.orig_head.as_deref(), Some(s.old_head.as_str()));
        assert!(j.baseline_oid_before.is_some() && j.new_baseline_oid.is_some());
        assert_eq!(f.others.len(), 2, "全部旧 / 新 OID 都记下了");
        // index.lock 从拼好到换入一直在
        let expect_lock = matches!(
            at,
            Point::IndexPrepared | Point::BranchMoved | Point::BeforeSwap
        );
        assert_eq!(index_lock(&s.fx).exists(), expect_lock, "{at:?}");
    }
}

#[test]
fn half_built_index_lock_is_deleted_not_renamed() {
    // 日志 Task 8 遗留第 2 条：停在 prepare 时锁文件可能是半成品。rename 进去就毁了索引
    let s = crash_scene();
    crash_at(&s, Point::IndexLocked);
    assert!(
        svn_journal::read(&s.fx.wc_str())
            .unwrap()
            .unwrap()
            .follow_steps
            .unwrap()
            .lock_created,
        "前提：journal 记下了锁是我方建的"
    );
    // unwind 跑了 IndexLock 的 Drop，真实崩溃不会：把半成品放回去
    let lock = index_lock(&s.fx);
    std::fs::write(&lock, "half built").unwrap();

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_finished(&s, Point::IndexLocked);
}

#[test]
fn lock_we_never_recorded_is_left_alone_at_the_prepare_step() {
    // 活着的 git 等编辑器时 index.lock 在、句柄已关，删除照样成功（review 探针 P7），所以「删不掉
    // 才算别人的」不成立。没记过「锁是我方建的」就不是我们的，不能删
    let s = crash_scene();
    crash_at(&s, Point::BeforePrepare);
    let lock = index_lock(&s.fx);
    std::fs::write(&lock, "别人的").unwrap();

    let e = resume_follow(&s.fx.wc_str()).unwrap_err();

    assert_eq!(e.code, "svn_index_busy", "{}", e.message);
    assert_eq!(std::fs::read(&lock).unwrap(), "别人的".as_bytes());
    assert!(svn_journal::read(&s.fx.wc_str()).unwrap().is_some());

    // 对方结束之后再点「继续」
    std::fs::remove_file(&lock).unwrap();
    let out = resume_follow(&s.fx.wc_str()).unwrap();
    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_finished(&s, Point::BeforePrepare);
}

#[test]
fn someone_elses_lock_after_the_rename_is_left_alone() {
    // 崩在 rename 之后：那个路径上是别的 git 进程的锁，不是我们的，不能 rename、不能删
    let s = crash_scene();
    crash_at(&s, Point::IndexSwapped);
    let lock = index_lock(&s.fx);
    std::fs::write(&lock, "别人的").unwrap();

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_eq!(std::fs::read(&lock).unwrap(), "别人的".as_bytes());
    std::fs::remove_file(&lock).unwrap();
    assert_finished(&s, Point::IndexSwapped);
}

#[test]
fn lock_deleted_by_hand_after_the_branch_moved_is_rolled_back() {
    // 用户照 git 的提示手工删了 index.lock。此刻分支已移、索引是旧的，要移回去再重新跟进
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    assert_ne!(oid(&s.fx, "work"), s.old_head, "前提：分支已移");
    std::fs::remove_file(index_lock(&s.fx)).unwrap();

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_finished(&s, Point::BranchMoved);
}

#[test]
fn lock_deleted_by_hand_before_the_rename_is_rolled_back_too() {
    let s = crash_scene();
    crash_at(&s, Point::BeforeSwap);
    std::fs::remove_file(index_lock(&s.fx)).unwrap();

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_finished(&s, Point::BeforeSwap);
}

#[test]
fn stat_refresh_after_the_lock_was_deleted_does_not_look_like_a_rename() {
    // 崩在 rename 之前，用户手工删了锁，别的 git 又刷新了 stat：索引字节变了，内容还是旧的。
    // 按字节比会判成 rename 发生过，留下分支已移、索引是旧的
    let s = crash_scene();
    crash_at(&s, Point::BeforeSwap);
    let index = s.fx.wc().join(".git/index");
    let before = std::fs::read(&index).unwrap();
    std::fs::remove_file(index_lock(&s.fx)).unwrap();
    // 内容不变、mtime 变：refresh 只改写这一条的 stat。A、B 磁盘与索引不同，退出码不是 0
    write(&s.fx, "Code/W.cs", "w\n");
    let wc = s.fx.wc_str();
    let _ = run_git(&wc, &["update-index", "-q", "--really-refresh"]);
    let _ = run_git(&wc, &["status", "--porcelain"]);
    assert_ne!(std::fs::read(&index).unwrap(), before, "前提：字节变了");

    let out = resume_follow(&wc).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_finished(&s, Point::BeforeSwap);
}

#[test]
fn checkout_b_while_crashed_goes_forward() {
    // 重开前的真实形态：race 建在 NEW_TIP 上。往回走就是 P3b 的反向暂存
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    git(&s.fx, &["checkout", "-b", "race"]);
    let tip = oid(&s.fx, "race");

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_no_residue(&s.fx);
    assert_eq!(git(&s.fx, &["symbolic-ref", "--short", "HEAD"]), "race");
    assert_eq!(oid(&s.fx, "race"), tip);
    assert_eq!(oid(&s.fx, "work"), tip, "work 留在新位置");
    assert_eq!(staged_paths(&s.fx), vec![B]);
    assert_eq!(show(&s.fx, ":Code/B.cs"), "b-staged");
}

#[cfg(windows)]
#[test]
fn rename_failure_during_resume_with_head_on_a_new_branch_keeps_the_lock() {
    // 恢复时同样不能回滚：race 在 NEW_TIP 上，放开锁就是反向暂存
    use std::cell::RefCell;
    use std::os::windows::fs::OpenOptionsExt;
    use std::rc::Rc;
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    git(&s.fx, &["checkout", "-b", "race"]);
    let index = s.fx.wc().join(".git/index");
    let held = Rc::new(RefCell::new(None));
    let (slot, path) = (held.clone(), index.clone());
    let hook = fault::call_at(Point::BeforeSwap, move || {
        let f = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0x1 | 0x2)
            .open(&path)
            .unwrap();
        *slot.borrow_mut() = Some(f);
    });

    let e = resume_follow(&s.fx.wc_str()).unwrap_err();
    drop(hook);
    held.borrow_mut().take();

    assert_eq!(e.code, "svn_follow_index_rename_failed", "{}", e.message);
    assert!(index_lock(&s.fx).exists(), "锁留着挡住外部 git");
    assert!(svn_journal::read(&s.fx.wc_str()).unwrap().is_some());

    let out = resume_follow(&s.fx.wc_str()).unwrap();
    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_eq!(staged_paths(&s.fx), vec![B]);
    assert_no_residue(&s.fx);
}

#[test]
fn checkout_b_and_lock_deleted_by_hand_recompose_the_index() {
    // race 移不回去，拼好的索引又被手工删了：只能照 OLD → NEW_TIP 重新拼一份
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    git(&s.fx, &["checkout", "-b", "race"]);
    std::fs::remove_file(index_lock(&s.fx)).unwrap();

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_no_residue(&s.fx);
    assert_eq!(staged_paths(&s.fx), vec![B], "没有反向暂存");
    assert_eq!(show(&s.fx, ":Code/B.cs"), "b-staged");
}

#[test]
fn head_on_another_commit_while_crashed_rolls_back_and_follows_that_branch() {
    // symbolic-ref 绕得过 index.lock：HEAD 指向 OLD 上的分支，拼好的索引对不上它
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    git(&s.fx, &["branch", "race", &s.old_head]);
    git(&s.fx, &["symbolic-ref", "HEAD", "refs/heads/race"]);

    let out = resume_follow(&s.fx.wc_str()).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_no_residue(&s.fx);
    let base = oid(&s.fx, "svn/base");
    assert_eq!(oid(&s.fx, "race^"), base);
    assert_eq!(oid(&s.fx, "work^"), base);
    assert_eq!(staged_paths(&s.fx), vec![B]);
}

#[test]
fn branch_moved_elsewhere_while_crashed_stops_without_clobbering_it() {
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    let wc = s.fx.wc_str();
    std::fs::remove_file(index_lock(&s.fx)).unwrap();
    let elsewhere = git(&s.fx, &["rev-parse", "plain"]);
    run_git(&wc, &["update-ref", "refs/heads/work", &elsewhere]).unwrap();

    let _ = resume_follow(&wc);

    assert!(
        svn_journal::read(&wc).unwrap().is_none(),
        "记录不能卡死：C1a 没有「放弃」入口"
    );
    assert!(git(&s.fx, &["merge-base", "--is-ancestor", &elsewhere, "work"]).is_empty());
}

#[test]
fn second_crash_during_resume_is_still_recoverable() {
    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    let armed = fault::crash_at(Point::IndexSwapped);
    let wc = s.fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| resume_follow(&wc)));
    drop(armed);
    assert!(r.is_err(), "前提：恢复途中又崩了一次");

    let out = resume_follow(&wc).unwrap();

    assert!(matches!(out, FollowOutcome::Done { .. }), "{out:?}");
    assert_finished(&s, Point::IndexSwapped);
}

#[test]
fn report_survives_the_crash() {
    // 泄漏告警只在这次跟进里看得见
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-log\n");
    commit_all(&fx, L_MSG);
    tortoise_commit(&fx, A);
    let armed = fault::crash_at(Point::IndexSwapped);
    let wc = fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| follow(&wc, "op1")));
    drop(armed);
    assert!(r.is_err());

    let rep = done(resume_follow(&wc).unwrap());

    assert_eq!(rep.leaks.len(), 1);
    assert_eq!(rep.changes[0].path, A);
}

#[test]
fn stage_report_survives_the_crash() {
    // 丢了哪些暂存只有这一次知道，重开之后还要告诉用户
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a-mid\n");
    git(&fx, &["add", A]);
    write(&fx, A, "a-final\n");
    write_mark(&fx.wc_str()).unwrap();
    tortoise_commit(&fx, A);
    let armed = fault::crash_at(Point::IndexSwapped);
    let wc = fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| follow(&wc, "op1")));
    drop(armed);
    assert!(r.is_err());

    let rep = done(resume_follow(&wc).unwrap());

    assert_eq!(rep.stage.discarded, vec![A.to_string()]);
    assert_eq!(
        rep.pinned_ref.as_deref(),
        Some("refs/mygitclient/follow/op1")
    );
}

#[test]
fn resume_without_a_record_is_an_error() {
    let fx = hybrid_with_colleague();
    let e = resume_follow(&fx.wc_str()).unwrap_err();
    assert_eq!(e.code, "sync_no_unfinished");
}

#[test]
fn resume_refuses_another_flows_record_and_the_other_flows_refuse_this_one() {
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    svn_journal::write(
        &wc,
        &Journal {
            opid: "x".to_string(),
            flow: Flow::SyncIn,
            phase: Phase::Stashed,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        resume_follow(&wc).unwrap_err().code,
        "svn_follow_wrong_flow"
    );

    let s = crash_scene();
    crash_at(&s, Point::BranchMoved);
    let e = continue_sync(&s.fx.wc_str()).unwrap_err();
    assert_eq!(e.code, "sync_follow_not_supported");
    let e = continue_sync_out(&s.fx.wc_str()).unwrap_err();
    assert_eq!(e.code, "sync_out_wrong_flow");
    assert!(e.message.contains("跟进"), "{}", e.message);
}

#[test]
fn unreadable_index_during_resume_rolls_back_instead_of_wedging_the_record() {
    // 恢复没有放弃入口，一条 `?` 出去的读错误就会让 journal 永远卡住，挡死更新与交付
    let s = crash_scene();
    crash_at(&s, Point::BeforeSwap);
    std::fs::remove_file(index_lock(&s.fx)).unwrap();
    std::fs::remove_file(s.fx.wc().join(".git/index")).unwrap();

    let _ = resume_follow(&s.fx.wc_str());

    assert!(svn_journal::read(&s.fx.wc_str()).unwrap().is_none());
    assert_eq!(oid(&s.fx, "work"), s.old_head, "分支移回去了");
}

// —— 收尾 ——

#[test]
fn pins_keep_the_latest_twenty() {
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let tree = git(&fx, &["rev-parse", "HEAD^{tree}"]);
    for i in 0..25 {
        let date = format!("{} +0000", 1_700_000_000 + i);
        let c = run_git_env(
            &wc,
            &["commit-tree", "-m", "pin", &tree],
            &[("GIT_COMMITTER_DATE", date.as_str())],
        )
        .unwrap();
        git(
            &fx,
            &[
                "update-ref",
                &format!("refs/mygitclient/follow/old{i:02}"),
                c.trim(),
            ],
        );
    }
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    done(run(&fx));

    let left = git(
        &fx,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/mygitclient/follow/",
        ],
    );
    let left: Vec<&str> = left.lines().collect();
    assert_eq!(left.len(), 20);
    assert!(left.contains(&"refs/mygitclient/follow/op1"), "这次的留着");
    assert!(
        left.contains(&"refs/mygitclient/follow/old24"),
        "最新的留着"
    );
    assert!(!left.contains(&"refs/mygitclient/follow/old05"), "老的删掉");
}

#[test]
fn baseline_summary_lists_revisions_and_authors_within_a_limit() {
    let fx = hybrid_with_colleague();
    let me = i_commit(&fx);
    write(&fx, A, "a2\n");
    let mine = tortoise_commit(&fx, A);
    let mut last = mine;
    for i in 0..14 {
        let rel = format!("Code/N{i:02}.cs");
        fx.colleague_commits(&rel, "n\n");
        last = youngest(&fx);
        set_author(&fx, last, &format!("user{}", i % 7));
        svn_update(&fx, &rel);
    }

    done(run(&fx));

    let msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    assert!(
        msg.contains(&format!(
            "跟进 r{mine}–r{last}：user0、user1、user2、user3、user4 等 7 人"
        )),
        "{msg}"
    );
    assert!(msg.contains(&format!("自己的提交 r{mine}")), "{msg}");
    assert!(msg.contains(&format!("r{mine} {me}：1 个文件")), "{msg}");
    assert!(msg.contains("另有 5 个 revision 未列出"), "{msg}");
    assert!(msg.lines().count() < 30 + 15, "控制长度：{msg}");
}

#[test]
fn anonymous_commit_is_named_in_the_summary() {
    let fx = hybrid_with_colleague();
    fx.colleague_commits(B, "b2\n");
    let r = youngest(&fx);
    set_author(&fx, r, "");
    svn_update(&fx, B);

    done(run(&fx));

    let msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    assert!(msg.contains(&format!("跟进 r{r}：（匿名）")), "{msg}");
}

// —— 手动跟进（计划 Task 13；分析 §6.5）——

fn manual(fx: &SvnFixture) -> SyncInOutcome {
    manual_follow(&fx.wc_str(), "mf1").unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
fn manual_err(fx: &SvnFixture) -> String {
    manual_follow(&fx.wc_str(), "mf1").unwrap_err().code
}
fn resume(fx: &SvnFixture) -> SyncInOutcome {
    continue_sync(&fx.wc_str()).unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}
fn journal(fx: &SvnFixture) -> Journal {
    svn_journal::read(&fx.wc_str())
        .unwrap()
        .expect("应当有记录")
}
/// `git()` 会 trim，把第一行的 ` M` 吃掉；这里要看前两列。
fn porcelain(fx: &SvnFixture) -> String {
    run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap()
}
fn stash_list(fx: &SvnFixture) -> String {
    git(fx, &["stash", "list"])
}
fn resolve_a(fx: &SvnFixture, content: &str) {
    write(fx, A, content);
    git(fx, &["add", A]);
}
/// W10-R2：git 里的中间版本与提交到 SVN 的版本不同，跟进只发布了基线。返回那个 commit。
fn conflict_scene(fx: &SvnFixture) -> String {
    write(fx, A, "a-git\n");
    let c = commit_all(fx, "git 里的版本");
    write(fx, A, "a-svn\n");
    tortoise_commit(fx, A);
    assert!(matches!(
        run(fx),
        FollowOutcome::NeedsManual {
            baseline_published: true,
            ..
        }
    ));
    c
}
fn expect_rebase_conflict(fx: &SvnFixture) {
    match manual(fx) {
        SyncInOutcome::RebaseConflict { files } => assert_eq!(files, vec![A.to_string()]),
        other => panic!("应当停在 rebase 冲突：{other:?}"),
    }
}
/// 暂存与未暂存各一份，再加一个未跟踪文件。
fn wip(fx: &SvnFixture) {
    write(fx, B, "b-staged\n");
    git(fx, &["add", B]);
    write(fx, B, "b-disk\n");
    write(fx, "Code/new.cs", "mine\n");
}
fn assert_wip_kept(fx: &SvnFixture) {
    assert_eq!(show(fx, ":Code/B.cs"), "b-staged", "暂存内容还在索引里");
    assert_eq!(read(fx, B), "b-disk\n", "未暂存的内容还在磁盘上");
    let st = porcelain(fx);
    assert!(st.contains("MM Code/B.cs"), "{st}");
    assert!(st.contains("?? Code/new.cs"), "{st}");
    assert_eq!(read(fx, "Code/new.cs"), "mine\n");
}

#[test]
fn manual_follow_resolves_w10_r2_via_rebase_conflict_exit() {
    let fx = hybrid_with_colleague();
    let c = conflict_scene(&fx);
    let base = oid(&fx, "svn/base");

    expect_rebase_conflict(&fx);

    let j = journal(&fx);
    assert_eq!(j.flow, Flow::FollowManual);
    assert_eq!(j.phase, Phase::Rebase);
    assert_eq!(j.orig_branch.as_deref(), Some("work"));
    assert_eq!(j.orig_head.as_deref(), Some(c.as_str()));
    assert_eq!(j.new_baseline_oid.as_deref(), Some(base.as_str()));
    assert!(!svn_lock::is_held(&fx.wc_str()), "冲突出口要放开锁");

    resolve_a(&fx, "a-svn\n");
    assert!(matches!(resume(&fx), SyncInOutcome::Done { .. }));

    assert_eq!(
        oid(&fx, "HEAD"),
        base,
        "解成 SVN 版本：这一笔被 --empty=drop 丢掉"
    );
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(oid(&fx, "svn/base"), base);
    assert_eq!(porcelain(&fx), "");
    assert_eq!(read(&fx, A), "a-svn\n");
    assert_eq!(stash_list(&fx), "");
    assert_no_residue(&fx);
    assert!(matches!(
        check_sync(&fx.wc_str()).unwrap().verdict,
        SyncVerdict::InSync
    ));
}

#[test]
fn manual_follow_restores_staging_with_index_when_possible() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    wip(&fx);

    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-svn\n");

    assert!(matches!(resume(&fx), SyncInOutcome::Done { .. }));
    assert_wip_kept(&fx);
    assert_eq!(stash_list(&fx), "");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_abort_returns_to_original_branch() {
    let fx = hybrid_with_colleague();
    let c = conflict_scene(&fx);
    let base = oid(&fx, "svn/base");
    wip(&fx);

    expect_rebase_conflict(&fx);
    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);

    assert_eq!(oid(&fx, "HEAD"), c, "回到原位");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(oid(&fx, "svn/base"), base, "基线保持前进：它是真实的");
    assert_eq!(read(&fx, A), "a-svn\n", "磁盘上的已提交内容放回来了");
    assert_wip_kept(&fx);
    assert!(!fx.wc().join(".git/rebase-merge").exists());
    assert_eq!(stash_list(&fx), "");
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/sync"]), "");
    assert_no_residue(&fx);

    // 回到了跟进前的样子：再点一次跟进，还是同一个出口
    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));
}

#[test]
fn manual_follow_refuses_when_head_contains_base() {
    let fx = hybrid_with_colleague();
    let before = state(&fx);
    assert_eq!(manual_err(&fx), "svn_follow_manual_not_needed");
    assert_eq!(state(&fx), before, "分支就在基线上");

    write(&fx, A, "a2\n");
    commit_all(&fx, "基线之后的提交");
    let before = state(&fx);
    assert_eq!(manual_err(&fx), "svn_follow_manual_not_needed");
    assert_eq!(state(&fx), before, "分支在基线之前");
    assert_eq!(stash_list(&fx), "");
    assert_no_residue(&fx);
}

/// 同事的改动落在暂存补丁的上下文里：`--index` 打不上（探针 P1），不带 `--index` 合得上。
/// 暂存区里是 B 的 `staged`；`disk` 不为空时之后磁盘继续改成它，再由 svn update 合进同事的改动。
fn stage_lost_scene(fx: &SvnFixture, staged: &str, disk: Option<&str>) {
    write(fx, B, "1\n2\n3\n4\n5\n6\n7\n8\n");
    tortoise_commit(fx, B);
    done(follow(&fx.wc_str(), "seed").unwrap());
    write(fx, A, "a-git\n");
    commit_all(fx, "git 里的版本");
    fx.colleague_commits(B, "1\n2\n3\n4\n5-colleague\n6\n7\n8\n");
    write(fx, B, staged);
    git(fx, &["add", B]);
    if let Some(d) = disk {
        write(fx, B, d);
    }
    svn_update(fx, B);
    write(fx, A, "a-svn\n");
    tortoise_commit(fx, A);
    assert!(matches!(run(fx), FollowOutcome::NeedsManual { .. }));

    expect_rebase_conflict(fx);
    resolve_a(fx, "a-svn\n");
}

/// `StageLost` 的三个字段。其他结局直接失败。
fn stage_lost(o: SyncInOutcome) -> (Option<u64>, Option<String>, Vec<String>) {
    match o {
        SyncInOutcome::StageLost {
            new_rev,
            stash_oid,
            files,
        } => (new_rev, stash_oid, files),
        other => panic!("应当标出暂存没保住：{other:?}"),
    }
}

#[test]
fn manual_follow_apply_index_fallback_with_stage_a_subset_of_disk_keeps_the_backup() {
    // 暂存的就是磁盘上的改动，只是磁盘多了 svn update 合进来的同事那一行：暂存版本仍与磁盘不同
    let fx = hybrid_with_colleague();
    stage_lost_scene(&fx, "1\n2-mine\n3\n4\n5\n6\n7\n8\n", None);

    let (new_rev, stash, files) = stage_lost(resume(&fx));
    assert!(new_rev.is_some());
    assert_eq!(files, vec![B.to_string()], "暂存版本缺同事那一行");
    assert_eq!(read(&fx, B), "1\n2-mine\n3\n4\n5-colleague\n6\n7\n8\n");
    assert_eq!(staged_paths(&fx), Vec::<String>::new(), "没有半成品暂存");
    assert_eq!(modified_paths(&fx), vec![B.to_string()]);
    assert_eq!(git(&fx, &["ls-files", "-u"]), "", "没有冲突残留");
    let stash = stash.expect("备份留着");
    assert!(stash_list(&fx).contains("mygitclient-sync:mf1"), "备份留着");
    assert_eq!(oid(&fx, "refs/mygitclient/sync/mf1"), stash, "钉子也留着");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_stage_lost_keeps_a_stage_that_differs_from_disk() {
    // Review 3 第 1 条（U2）：暂存区是方案甲，磁盘继续改成方案乙。`--index` 打不上、退回普通
    // apply 之后，方案甲只在 stash 里。原先照常 drop 并拆钉子，方案甲就只剩悬空对象，gc 后消失
    let fx = hybrid_with_colleague();
    let plan_a = "1\n2-plan-a\n3\n4\n5\n6\n7\n8\n";
    stage_lost_scene(&fx, plan_a, Some("1\n2-plan-b\n3\n4\n5\n6\n7\n8\n"));

    let (new_rev, stash, files) = stage_lost(resume(&fx));
    assert!(new_rev.is_some());
    assert_eq!(files, vec![B.to_string()], "点名暂存与磁盘不同的路径");
    assert_eq!(read(&fx, B), "1\n2-plan-b\n3\n4\n5-colleague\n6\n7\n8\n");
    let stash = stash.expect("备份留着");
    assert!(stash_list(&fx).contains("mygitclient-sync:mf1"), "备份留着");
    assert_eq!(oid(&fx, "refs/mygitclient/sync/mf1"), stash, "钉子也留着");
    assert_no_residue(&fx);

    // 能取回方案甲：提示里给的两条命令
    assert_eq!(show(&fx, &format!("{stash}^2:{B}")), plan_a.trim_end());
    git(
        &fx,
        &[
            "restore",
            &format!("--source={stash}^2"),
            "--staged",
            "--",
            B,
        ],
    );
    assert_eq!(show(&fx, &format!(":{B}")), plan_a.trim_end());
    assert_eq!(
        read(&fx, B),
        "1\n2-plan-b\n3\n4\n5-colleague\n6\n7\n8\n",
        "磁盘不动"
    );
}

#[test]
fn manual_follow_stash_conflict_exit_settles_and_keeps_the_backup() {
    // 冲突没解成 SVN 版本：stash 里的 A 是 SVN 版本，放回时同一行又冲突一次
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-merged\n");

    let stash = match resume(&fx) {
        SyncInOutcome::StashConflict { files, stash_oid } => {
            assert_eq!(files, vec![A.to_string()]);
            stash_oid
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(journal(&fx).phase, Phase::ApplyingStash);
    assert_eq!(journal(&fx).flow, Flow::FollowManual);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_in_unresolved"
    );

    resolve_a(&fx, "a-final\n");
    assert_eq!(
        resume(&fx),
        SyncInOutcome::Settled {
            stash_oid: stash,
            rolled_back: false,
        }
    );
    assert!(stash_list(&fx).contains("mygitclient-sync:mf1"), "备份留着");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_abort_at_stash_conflict_releases_like_sync_in() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-merged\n");
    let SyncInOutcome::StashConflict { stash_oid, .. } = resume(&fx) else {
        panic!("应当停在放回冲突");
    };
    let tip = oid(&fx, "HEAD");

    assert_eq!(
        abort_sync(&fx.wc_str()).unwrap(),
        SyncInOutcome::Released {
            stash_oid,
            rolled_back: false,
        }
    );
    assert_eq!(oid(&fx, "HEAD"), tip, "磁盘上有冲突标记，不回滚");
    assert!(stash_list(&fx).contains("mygitclient-sync:mf1"));
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_stash_blocked_exit_resumes_after_the_file_moves() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    write(&fx, "Code/new.cs", "mine\n");
    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-svn\n");
    write(&fx, "Code/new.cs", "别的内容\n");

    match resume(&fx) {
        SyncInOutcome::StashBlocked { collisions, .. } => {
            assert_eq!(collisions, vec!["Code/new.cs".to_string()]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(read(&fx, "Code/new.cs"), "别的内容\n", "一个字节都没放");

    std::fs::remove_file(fx.wc().join("Code/new.cs")).unwrap();
    assert!(matches!(resume(&fx), SyncInOutcome::Done { .. }));
    assert_eq!(read(&fx, "Code/new.cs"), "mine\n");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_takes_over_an_untracked_copy_of_a_baseline_file() {
    // A17：局部 update 拉下的新文件在 git 看来是未跟踪的，基线里又有它，同一个 blob 出现三次
    let fx = hybrid_with_colleague();
    write(&fx, A, "a-git\n");
    commit_all(&fx, "git 里的版本");
    fx.colleague_commits("Code/C.cs", "c\n");
    svn_update(&fx, "Code");
    write(&fx, A, "a-svn\n");
    tortoise_commit(&fx, A);
    assert!(matches!(run(&fx), FollowOutcome::NeedsManual { .. }));

    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-svn\n");

    assert!(matches!(resume(&fx), SyncInOutcome::Done { .. }));
    assert_eq!(read(&fx, "Code/C.cs"), "c\n");
    assert_eq!(porcelain(&fx), "");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_neither_updates_svn_nor_visits_the_baseline() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    fx.colleague_commits(B, "b-colleague\n"); // 服务器上有更新的，wc1 不该拉下来
    let rev = || {
        let out = Command::new("svn")
            .current_dir(fx.wc())
            .args(["info", "--show-item", "revision", "--", B])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let before = rev();

    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-svn\n");
    assert!(matches!(resume(&fx), SyncInOutcome::Done { .. }));

    assert_eq!(rev(), before);
    assert_eq!(read(&fx, B), "b1\n");
    let moves = git(&fx, &["reflog", "--format=%gs", "HEAD"]);
    assert!(!moves.contains("to svn/base"), "{moves}");
}

#[test]
fn manual_follow_on_a_clean_lagging_branch_needs_no_stash() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    git(&fx, &["checkout", "-q", "-b", "lag", "HEAD~1"]);
    write(&fx, "Code/L.cs", "l\n");
    let l = commit_all(&fx, "落后分支自己的提交");
    let base = oid(&fx, "svn/base");

    assert!(matches!(manual(&fx), SyncInOutcome::Done { .. }));

    assert_ne!(oid(&fx, "HEAD"), l);
    assert_eq!(oid(&fx, "HEAD~1"), base);
    assert_eq!(read(&fx, A), "a2\n");
    assert_eq!(read(&fx, "Code/L.cs"), "l\n");
    assert_eq!(stash_list(&fx), "");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_clears_the_mark_when_the_branch_landed_and_keeps_it_on_abort() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    write_mark(&fx.wc_str()).unwrap();
    expect_rebase_conflict(&fx);
    assert!(read_mark(&fx.wc_str()).unwrap().is_some(), "还没落到基线上");
    abort_sync(&fx.wc_str()).unwrap();
    assert!(read_mark(&fx.wc_str()).unwrap().is_some(), "放弃之后还要用");

    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-svn\n");
    resume(&fx);
    assert!(
        read_mark(&fx.wc_str()).unwrap().is_none(),
        "分支已落到基线上"
    );
}

// —— 手动跟进：分派不能串味 ——

#[test]
fn manual_follow_record_stays_out_of_the_other_flows() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    expect_rebase_conflict(&fx);
    let wc = fx.wc_str();

    assert_eq!(
        resume_follow(&wc).unwrap_err().code,
        "svn_follow_wrong_flow"
    );
    let e = continue_sync_out(&wc).unwrap_err();
    assert_eq!(e.code, "sync_out_wrong_flow");
    assert!(e.message.contains("手动跟进"), "{}", e.message);
    assert_eq!(refused(run(&fx)).0, "svn_follow_unfinished");
    assert_eq!(manual_err(&fx), "svn_follow_unfinished");
    assert_eq!(journal(&fx).phase, Phase::Rebase, "都没有动这条记录");
}

#[test]
fn manual_follow_is_never_started_by_continue_or_resume() {
    // 只有显式调用才会动磁盘：普通跟进的冲突出口不留记录，「继续」无从接手
    let fx = hybrid_with_colleague();
    let c = conflict_scene(&fx);
    assert_eq!(oid(&fx, "HEAD"), c);
    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_no_unfinished"
    );
    assert_eq!(
        resume_follow(&fx.wc_str()).unwrap_err().code,
        "sync_no_unfinished"
    );
    assert_eq!(read(&fx, A), "a-svn\n");
}

// —— 手动跟进：崩溃恢复 ——

fn crash_after_stash(fx: &SvnFixture) {
    let armed = fault::crash_at(Point::ManualStashed);
    let wc = fx.wc_str();
    let r = catch_unwind(AssertUnwindSafe(|| manual_follow(&wc, "mf1")));
    drop(armed);
    match r {
        Err(p) => assert!(p.is::<Crashed>(), "是别的 panic"),
        Ok(o) => panic!("没有走到崩溃点：{:?}", o.map_err(|e| e.message)),
    }
}

#[test]
fn manual_follow_resumes_from_stashed_without_leaving_the_branch() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    wip(&fx);
    crash_after_stash(&fx);
    assert_eq!(journal(&fx).phase, Phase::Stashed);

    match resume(&fx) {
        SyncInOutcome::RebaseConflict { files } => assert_eq!(files, vec![A.to_string()]),
        other => panic!("{other:?}"),
    }
    let moves = git(&fx, &["reflog", "--format=%gs", "HEAD"]);
    assert!(!moves.contains("to svn/base"), "恢复不能走 S3：{moves}");

    resolve_a(&fx, "a-svn\n");
    assert!(matches!(resume(&fx), SyncInOutcome::Done { .. }));
    assert_wip_kept(&fx);
}

#[test]
fn manual_follow_settles_a_crash_in_the_middle_of_the_stash() {
    // 崩在 push 落地之后、记下 OID 之前：phase 还是 stashing
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    wip(&fx);
    crash_after_stash(&fx);
    let mut j = journal(&fx);
    let oid_was = j.stash_oid.take();
    j.phase = Phase::Stashing;
    svn_journal::write(&fx.wc_str(), &j).unwrap();

    assert!(matches!(resume(&fx), SyncInOutcome::RebaseConflict { .. }));
    assert_eq!(journal(&fx).stash_oid, oid_was, "按 opid 找回来");
}

#[test]
fn stage_lost_survives_a_crash_after_the_apply_in_both_directions() {
    // 崩在放回之后、删记录之前：这一次已经不会再 apply，只有 journal 记得暂存没保住。
    // 暂存没保住时不再 drop（Review 3 第 1 条），这里是 stash 被外部删掉了：收尾是 Finished，
    // 不是 Settled，也给不出备份
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    expect_rebase_conflict(&fx);
    resolve_a(&fx, "a-svn\n");
    resume(&fx);
    let record = |rolling_back: bool| Journal {
        opid: "mf2".to_string(),
        flow: Flow::FollowManual,
        phase: Phase::ApplyingStash,
        started_at: 1,
        orig_branch: Some("work".to_string()),
        orig_head: Some(oid(&fx, "HEAD")),
        new_baseline_oid: Some(oid(&fx, "svn/base")),
        stash_oid: Some("0".repeat(40)),
        stash_applied: true,
        stage_lost: true,
        rolling_back,
        ..Default::default()
    };

    svn_journal::write(&fx.wc_str(), &record(false)).unwrap();
    let (new_rev, stash, files) = stage_lost(resume(&fx));
    assert!(new_rev.is_some(), "往前走完");
    assert_eq!((stash, files), (None, Vec::new()));
    assert_no_residue(&fx);

    svn_journal::write(&fx.wc_str(), &record(true)).unwrap();
    assert_eq!(
        resume(&fx),
        SyncInOutcome::StageLost {
            new_rev: None,
            stash_oid: None,
            files: Vec::new(),
        }
    );
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_abort_after_a_crash_restores_everything() {
    let fx = hybrid_with_colleague();
    let c = conflict_scene(&fx);
    wip(&fx);
    crash_after_stash(&fx);

    assert_eq!(abort_sync(&fx.wc_str()).unwrap(), SyncInOutcome::Aborted);

    assert_eq!(oid(&fx, "HEAD"), c);
    assert_wip_kept(&fx);
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_resume_stops_when_head_was_moved_to_the_baseline() {
    // 手动跟进从不去基线，HEAD 在那里只可能是外部动的。靠 baselineOidBefore 是分支的旧基线挡住
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    crash_after_stash(&fx);
    git(&fx, &["checkout", "-q", "svn/base"]);

    let e = continue_sync(&fx.wc_str()).unwrap_err();

    assert_eq!(e.code, "sync_in_state_changed");
    assert_eq!(journal(&fx).phase, Phase::Stashed, "记录留着");
}

#[test]
fn manual_follow_resume_stops_on_new_changes_after_the_crash() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    crash_after_stash(&fx);
    write(&fx, B, "中断之后才写的\n");

    assert_eq!(
        continue_sync(&fx.wc_str()).unwrap_err().code,
        "sync_in_state_changed"
    );
    assert_eq!(read(&fx, B), "中断之后才写的\n");
}

// —— 手动跟进：前置条件。全部在动手之前挡住 ——

#[test]
fn manual_follow_refuses_tracked_changes_outside_the_whitelist() {
    // stash 只收白名单，rebase 又要求工作区干净：放过去就是 stash 之后才失败（探针 P3）
    let fx = hybrid_with_colleague();
    write(&fx, "Art/t.txt", "t1\n");
    git(&fx, &["add", "-f", "Art/t.txt"]);
    git(&fx, &["commit", "-qm", "白名单外的文件"]);
    conflict_scene(&fx);

    write(&fx, "Art/t.txt", "t2\n");
    let before = state(&fx);
    assert_eq!(manual_err(&fx), "svn_follow_manual_outside_changes");
    git(&fx, &["add", "-f", "Art/t.txt"]);
    assert_eq!(manual_err(&fx), "svn_follow_manual_outside_changes");

    assert_eq!(state(&fx).0, before.0, "ref 一个都没动");
    assert_eq!(stash_list(&fx), "");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_refuses_when_the_working_copy_moved_again() {
    // 基线又落后了：先让「跟进」把它发布上去，别把分支重放到一个过时的基线上
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    write(&fx, A, "a-svn2\n");
    tortoise_commit(&fx, A);
    let before = state(&fx);

    assert_eq!(manual_err(&fx), "svn_follow_manual_stale");
    assert_eq!(state(&fx), before);
    assert_eq!(stash_list(&fx), "");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_refuses_a_merge_on_the_branch() {
    let fx = hybrid_with_colleague();
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    git(&fx, &["checkout", "-q", "-b", "old", "HEAD~1"]);
    git(&fx, &["checkout", "-q", "-b", "topic"]);
    write(&fx, "Code/T.cs", "t\n");
    commit_all(&fx, "topic");
    git(&fx, &["checkout", "-q", "old"]);
    write(&fx, "Code/O.cs", "o\n");
    commit_all(&fx, "old");
    git(
        &fx,
        &["merge", "-q", "--no-ff", "-m", "merge topic", "topic"],
    );
    let before = state(&fx);

    assert_eq!(manual_err(&fx), "svn_follow_merge_in_range");
    assert_eq!(state(&fx), before);
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_refuses_detached_head_and_the_baseline_branch() {
    let fx = hybrid_with_colleague();
    conflict_scene(&fx);
    write(&fx, A, "a-git\n"); // 工作区干净才切得走
    git(&fx, &["checkout", "-q", "--detach"]);
    assert_eq!(manual_err(&fx), "svn_follow_detached_head");
    git(&fx, &["checkout", "-q", "-f", "svn/base"]);
    assert_eq!(manual_err(&fx), "svn_follow_on_baseline");
    assert_no_residue(&fx);
}

#[test]
fn manual_follow_refuses_when_svn_conflicted() {
    // 非 W-file 上的冲突判定看不见，要靠 R1。同 `branch_entry_refuses_when_svn_conflicted`
    let fx = hybrid_with_colleague();
    let x = "Code/notes.txt";
    write(&fx, x, "x1\n");
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", x]);
    tortoise_commit(&fx, x);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    rebuild_baseline(&fx.wc_str()).unwrap();
    fx.colleague_commits(x, "theirs\n");
    write(&fx, x, "mine\n");
    svn_cmd(
        &fx.wc(),
        &["update", "--accept", "postpone", "--non-interactive"],
    );
    let before = state(&fx);

    assert_eq!(manual_err(&fx), "svn_preflight_conflicted");
    assert_eq!(state(&fx), before);
    assert_eq!(stash_list(&fx), "");
}

// —— NeedsManual 带 F3 ——

#[test]
fn conflict_exit_reports_local_only_leaks_on_both_entries() {
    // L 的两行只有一行上了服务器：重放冲突，泄漏判定是「拿不准」
    let fx = hybrid_with_colleague();
    write(&fx, A, "log\nx\n");
    let l = commit_all(&fx, L_MSG);
    write(&fx, A, "log\ny\n");
    tortoise_commit(&fx, A);

    for opid in ["op1", "op2"] {
        match follow(&fx.wc_str(), opid).unwrap() {
            FollowOutcome::NeedsManual { leaks, .. } => {
                assert_eq!(leaks.len(), 1, "{opid}");
                assert_eq!(leaks[0].commit, l);
                assert_eq!(leaks[0].branch, "work");
                assert_eq!(leaks[0].path, A);
                assert_eq!(leaks[0].hunks[0].verdict, LeakVerdict::Unknown);
            }
            other => panic!("{other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// 落后基线的分支（计划 Task 11，分析 §6.11.3）
// ---------------------------------------------------------------------------

fn behind(fx: &SvnFixture) -> Vec<String> {
    svn_branches_behind(fx.wc_str()).unwrap_or_else(|e| panic!("{}: {}", e.code, e.message))
}

#[test]
fn branch_left_behind_by_a_conflicting_follow_is_listed() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["checkout", "-q", "-b", "feature"]);
    write(&fx, A, "a-feature\n");
    commit_all(&fx, "feature 的改动");
    git(&fx, &["checkout", "-q", "work"]);
    assert_eq!(behind(&fx), Vec::<String>::new(), "跟进之前谁都不落后");

    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    let report = done(run(&fx));

    assert_eq!(
        report.behind_branches.len(),
        1,
        "前提：feature 冲突，留在原地"
    );
    assert_eq!(behind(&fx), vec!["feature".to_string()]);
}

#[test]
fn branches_on_the_current_baseline_are_not_behind() {
    // 重放过去的分支、没有自己 commit 的分支、基线分支本身，以及当前分支
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    git(&fx, &["branch", "empty"]);
    git(&fx, &["checkout", "-q", "-b", "moved"]);
    write(&fx, B, "b-moved\n");
    commit_all(&fx, "moved 的改动");
    git(&fx, &["checkout", "-q", "work"]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    let report = done(run(&fx));

    assert!(report.behind_branches.is_empty(), "{report:?}");
    assert_eq!(behind(&fx), Vec::<String>::new());
}

#[test]
fn branch_with_no_commits_of_its_own_on_an_old_baseline_is_behind() {
    // 配了 upstream 的分支跟进不动它，tip 就停在旧基线上：tip 本身是 svn/base 的祖先，
    // 不能因为「分叉点就是它自己」被漏掉
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    git(&fx, &["branch", "stay", &old_base]);
    git(&fx, &["config", "branch.stay.remote", "."]);
    git(&fx, &["config", "branch.stay.merge", "refs/heads/work"]);
    i_commit(&fx);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    assert_eq!(oid(&fx, "stay"), old_base, "前提：有 upstream，跟进不动它");

    assert_eq!(behind(&fx), vec!["stay".to_string()]);
}

#[test]
fn loose_branch_behind_a_long_ref_is_still_listed() {
    // 超长 ref 让 libgit2 的全量遍历提前结束时，只有 loose 的分支不能从「落后基线」里消失
    let fx = hybrid_with_colleague();
    let old_base = oid(&fx, "svn/base");
    git(&fx, &["branch", "stay", &old_base]);
    git(&fx, &["config", "branch.stay.remote", "."]);
    git(&fx, &["config", "branch.stay.merge", "refs/heads/work"]);
    i_commit(&fx);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));
    long_ref::plant_long_loose_ref(&fx.wc(), &old_base);

    assert_eq!(behind(&fx), vec!["stay".to_string()]);
}

#[test]
fn branch_with_unrelated_history_is_not_behind() {
    // 没有共同祖先：它不含任何基线，「落后」无从谈起，手动跟进也接不了
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "--orphan", "island"]);
    git(&fx, &["commit", "-qm", "另一段历史"]);
    git(&fx, &["checkout", "-q", "work"]);

    assert_eq!(behind(&fx), Vec::<String>::new());
}

#[test]
fn branches_behind_lists_every_lagging_branch_including_non_ascii_names() {
    let fx = hybrid_with_colleague();
    i_commit(&fx);
    write(&fx, A, "a-shared\n");
    let shared = commit_all(&fx, "几条分支共有");
    git(&fx, &["branch", "zeta", &shared]);
    git(&fx, &["branch", "功能/甲", &shared]);
    git(&fx, &["branch", "alpha", &shared]);
    // 从基线另开一条分支再跟进：work 带着同一个 commit，和上面三条一样重放冲突、留在原地
    let base = oid(&fx, "svn/base");
    git(&fx, &["checkout", "-q", "-b", "next", &base]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    done(run(&fx));

    let mut got = behind(&fx);
    got.sort();
    assert_eq!(
        got,
        vec![
            "alpha".to_string(),
            "work".to_string(),
            "zeta".to_string(),
            "功能/甲".to_string()
        ]
    );
}

#[test]
fn branches_behind_is_empty_outside_a_hybrid_repo() {
    // 普通 git 仓库：有条分支落后于叫 svn/base 的分支，但这个名字在这里没有含义
    let plain = tempfile::TempDir::new().unwrap();
    let path = plain.path().to_str().unwrap();
    run_git(path, &["init", "-q", "-b", "main"]).unwrap();
    let commit = ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q"];
    run_git(
        path,
        &[&commit[..], &["--allow-empty", "-m", "c1"]].concat(),
    )
    .unwrap();
    run_git(path, &["branch", "old"]).unwrap();
    run_git(
        path,
        &[&commit[..], &["--allow-empty", "-m", "c2"]].concat(),
    )
    .unwrap();
    run_git(path, &["branch", "svn/base"]).unwrap();
    assert_eq!(
        svn_branches_behind(path.to_string()).unwrap(),
        Vec::<String>::new()
    );

    // 还没建库的 SVN 工作副本：不能因为查一下就报错，也不能造出 `.git`
    let svn = svn_repo();
    assert_eq!(
        svn_branches_behind(svn.wc_str()).unwrap(),
        Vec::<String>::new()
    );
    assert!(!svn.wc().join(".git").exists());
}

#[test]
fn branches_behind_is_empty_when_the_baseline_ref_is_gone() {
    let fx = hybrid_with_colleague();
    git(&fx, &["update-ref", "-d", "refs/heads/svn/base"]);

    assert_eq!(behind(&fx), Vec::<String>::new());
}

// —— 录制的候选索引沿用 stat（性能，日志「性能 P1」）——

#[test]
fn follow_records_the_same_tree_an_empty_index_would() {
    // 候选索引改成从旧基线树起步、沿用真实索引的 stat 之后，录出来的必须仍是
    // 「从空索引 add 磁盘、再换回服务器内容」那棵树：磁盘上什么形态都放一份
    let fx = hybrid_with_colleague();
    set_me(&fx, &committer(&fx));
    write(&fx, "Code/W.cs", "w\n");
    commit_all(&fx, "work 上的 commit");
    write(&fx, B, "b-staged\n");
    git(&fx, &["add", B]);
    write(&fx, B, "b-disk\n");
    write(&fx, "Code/Untracked.cs", "u\n");
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);
    fx.colleague_commits("Code/C.cs", "c\n");
    svn_update(&fx, "Code/C.cs");

    done(run(&fx));

    let cfg = read_config(&fx.wc_str()).unwrap();
    let fresh = record_mixed(&fx.wc_str(), &cfg, &TempIndex::empty(&fx.wc_str()).unwrap()).unwrap();
    assert_eq!(oid(&fx, "svn/base^{tree}"), fresh.tree);
}

#[test]
fn a_recording_from_the_old_base_keeps_its_file_modes() {
    // 与空索引不等价的一处，有意保留（独立 review 2026-11-01）：filemode=false 时 `git add` 沿用
    // 已有条目的 mode。旧基线里的 100755（S7 从用户 commit 带进来的）跟进后仍是 100755；
    // 从空索引录会归一成 100644，基线就随跟进来回翻 mode
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    git(&fx, &["config", "core.filemode", "false"]);
    let scratch = fx.wc().join(".git").join("mode-test-index");
    let scratch = scratch.to_string_lossy().into_owned();
    let env = [("GIT_INDEX_FILE", scratch.as_str())];
    run_git_env(&wc, &["read-tree", "svn/base^{tree}"], &env).unwrap();
    run_git_env(&wc, &["update-index", "--chmod=+x", A], &env).unwrap();
    let old_base = run_git_env(&wc, &["write-tree"], &env)
        .unwrap()
        .trim()
        .to_string();
    let cfg = read_config(&wc).unwrap();

    let kept = record_mixed(
        &wc,
        &cfg,
        &TempIndex::from_tree_keeping_stat(&wc, &old_base).unwrap(),
    )
    .unwrap();
    let fresh = record_mixed(&wc, &cfg, &TempIndex::empty(&wc).unwrap()).unwrap();

    let mode = |tree: &str| git(&fx, &["ls-tree", tree, A])[..6].to_string();
    assert_eq!(mode(&kept.tree), "100755");
    assert_eq!(mode(&fresh.tree), "100644");
}

#[test]
fn a_stale_scan_cache_cannot_record_a_lying_baseline() {
    // 准备阶段缓存了 svn 扫描（性能 P2）。缓存之后 TortoiseSVN 又提交了一次：用陈旧的
    // 节点状态录，映射会把新内容标成旧 revision。R3c 重读与体检必须把它拦下
    let fx = hybrid_with_colleague();
    let base = oid(&fx, "svn/base");
    let cfg = read_config(&fx.wc_str()).unwrap();
    let err = with_scan_cache(|| {
        check_sync(&fx.wc_str()).unwrap();
        write(&fx, A, "a2\n");
        tortoise_commit(&fx, A);
        record_mixed(&fx.wc_str(), &cfg, &TempIndex::empty(&fx.wc_str()).unwrap())
    })
    .expect_err("陈旧的扫描结果不能录出基线");
    assert!(
        ["svn_rebuild_revision_moved", "svn_baseline_lying"].contains(&err.code.as_str()),
        "应当被 R3c 或体检拦下，实际 {}: {}",
        err.code,
        err.message
    );
    assert_eq!(oid(&fx, "svn/base"), base);
}

#[test]
fn follow_falls_back_to_an_empty_index_when_the_old_base_has_files_outside_the_whitelist() {
    // 旧基线里有现在已不算 W 的文件（比如扩展名清单收窄过）：从旧基线起步时 `add -A`
    // 会把这个已跟踪的文件留下，纳入集合核对不过。这时要退回空索引照旧录一次，
    // 而不是让跟进失败
    let fx = hybrid_with_colleague();
    write(&fx, "Code/readme.txt", "r\n");
    svn_cmd(
        &fx.wc(),
        &["add", "--non-interactive", "--", "Code/readme.txt"],
    );
    tortoise_commit(&fx, "Code/readme.txt");
    svn_update(&fx, ".");
    // 把 readme.txt 塞进基线树，work 跟着指过去
    let base = oid(&fx, "svn/base");
    let blob = git(&fx, &["hash-object", "-w", "Code/readme.txt"]);
    let tmp = fx.wc().join(".git").join("probe-index");
    let env = [("GIT_INDEX_FILE", tmp.to_str().unwrap())];
    let wc = fx.wc_str();
    run_git_env(&wc, &["read-tree", &base], &env).unwrap();
    run_git_env(
        &wc,
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("100644,{blob},Code/readme.txt"),
        ],
        &env,
    )
    .unwrap();
    let tree = run_git_env(&wc, &["write-tree"], &env).unwrap();
    std::fs::remove_file(&tmp).unwrap();
    let msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    let msg_file = fx.wc().join(".git").join("probe-msg");
    std::fs::write(&msg_file, &msg).unwrap();
    let wide = git(
        &fx,
        &[
            "commit-tree",
            tree.trim(),
            "-p",
            &base,
            "-F",
            msg_file.to_str().unwrap(),
        ],
    );
    std::fs::remove_file(&msg_file).unwrap();
    git(&fx, &["update-ref", "refs/heads/svn/base", &wide]);
    git(&fx, &["update-ref", "refs/heads/work", &wide]);
    git(&fx, &["read-tree", &wide]);
    write(&fx, A, "a2\n");
    tortoise_commit(&fx, A);

    done(run(&fx));

    assert_eq!(show(&fx, "svn/base:Code/A.cs"), "a2");
    let files = git(&fx, &["ls-tree", "-r", "--name-only", "svn/base"]);
    assert!(
        !files.lines().any(|l| l == "Code/readme.txt"),
        "退回空索引重录，不是 W 的文件不进新基线：{files}"
    );
}
