//! 对象层重放引擎（计划 Task 6；分析 §6.5 F2、§6.11.3；日志 W17–W20）。
//!
//! 全部在临时 fixture 上跑。被重放的 commit 与「新基线」都直接在对象层造（临时索引 +
//! `commit-tree`），不经过工作区：重放本身只看对象，造法与它无关；而且这样能精确摆出
//! W10-R2、W14 那种「同一行先后两个版本」的形态。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::{run_git, run_git_env, run_git_stdin_env};
use app_lib::engine::svn_replay::{
    all_committed, commits_between, no_superseded, replay_onto, ReplayCodes, ReplayCommit,
    ReplayOpts, ReplayOutcome,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

const CODES: ReplayCodes = ReplayCodes {
    conflict: "t_conflict",
    too_old: "t_too_old",
    too_old_message: "git 太旧",
    failed: "t_failed",
};

const A: &str = "Code/A.cs";
const E: &str = "Code/E.cs";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn svn_base(fx: &SvnFixture) -> String {
    git(fx, &["rev-parse", "svn/base"])
}

fn tree_of(fx: &SvnFixture, rev: &str) -> String {
    git(fx, &["rev-parse", &format!("{rev}^{{tree}}")])
}

fn parent_of(fx: &SvnFixture, rev: &str) -> String {
    git(fx, &["rev-parse", &format!("{rev}^")])
}

fn show(fx: &SvnFixture, rev: &str, path: &str) -> String {
    run_git(&fx.wc_str(), &["show", &format!("{rev}:{path}")]).unwrap()
}

fn subject(fx: &SvnFixture, rev: &str) -> String {
    git(fx, &["log", "-1", "--format=%s", rev])
}

enum Ch<'a> {
    /// 普通文件（100644）
    Put(&'a str, &'a str),
    /// 可执行文件（100755）
    Exec(&'a str, &'a str),
    Del(&'a str),
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// 在对象层造一个 commit：`parents[0]` 的树 + `changes`。不碰工作区与真实索引。
fn commit_on(fx: &SvnFixture, parents: &[&str], changes: &[Ch], msg: &str) -> String {
    let wc = fx.wc_str();
    // 临时索引放在 fixture 根下（wc 外），绝对路径（W19）
    let idx = fx
        .path()
        .join(format!("t-{}.index", SEQ.fetch_add(1, Ordering::Relaxed)));
    let idx_s = idx.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", idx_s.as_str())];
    run_git_env(&wc, &["read-tree", parents[0]], &env).unwrap();
    for ch in changes {
        match ch {
            Ch::Put(p, body) | Ch::Exec(p, body) => {
                let mode = if matches!(ch, Ch::Exec(..)) {
                    "100755"
                } else {
                    "100644"
                };
                let oid =
                    run_git_stdin_env(&wc, &["hash-object", "-w", "--stdin"], body.as_bytes(), &[])
                        .unwrap()
                        .trim()
                        .to_string();
                run_git_env(
                    &wc,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("{mode},{oid},{p}"),
                    ],
                    &env,
                )
                .unwrap();
            }
            Ch::Del(p) => {
                run_git_env(&wc, &["update-index", "--force-remove", "--", p], &env).unwrap();
            }
        }
    }
    let tree = run_git_env(&wc, &["write-tree"], &env)
        .unwrap()
        .trim()
        .to_string();
    std::fs::remove_file(&idx).unwrap();
    let mut args = vec!["commit-tree", tree.as_str()];
    for p in parents {
        args.push("-p");
        args.push(p);
    }
    args.push("-m");
    args.push(msg);
    git(fx, &args)
}

/// 30 行（第 n 行是 n），`repl` 替换某些行，`after` 在某行之后插入一行。
fn lines(repl: &[(usize, &str)], after: &[(usize, &str)]) -> String {
    let mut s = String::new();
    for n in 1..=30 {
        match repl.iter().find(|(k, _)| *k == n) {
            Some((_, v)) => s.push_str(&format!("{v}\n")),
            None => s.push_str(&format!("{n}\n")),
        }
        for (_, v) in after.iter().filter(|(k, _)| *k == n) {
            s.push_str(&format!("{v}\n"));
        }
    }
    s
}

fn opts<'a>(
    drop_empty: bool,
    superseded: &'a dyn Fn(&ReplayCommit) -> BTreeSet<String>,
) -> ReplayOpts<'a> {
    ReplayOpts {
        drop_empty,
        superseded,
        codes: CODES,
    }
}

/// `Done` 的四个字段：(tip, map, dropped, rewritten)。
type Done = (
    String,
    BTreeMap<String, String>,
    Vec<String>,
    Vec<(String, Vec<String>)>,
);

fn done(o: ReplayOutcome) -> Done {
    match o {
        ReplayOutcome::Done {
            tip,
            map,
            dropped,
            rewritten,
        } => (tip, map, dropped, rewritten),
        other => panic!("期望 Done，实际 {other:?}"),
    }
}

/// 旧基线换成「A 是 30 行」的 commit，方便按行摆冲突。返回它的 OID。
fn thirty_line_base(fx: &SvnFixture) -> String {
    commit_on(
        fx,
        &[&svn_base(fx)],
        &[Ch::Put(A, &lines(&[], &[]))],
        "old base",
    )
}

#[test]
fn replay_drops_commit_whose_content_reached_svn() {
    // W4：C1 改 a 后被 TortoiseSVN 原样提交，C2 改 e 没提交
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let c1 = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "C1 改 a");
    let c2 = commit_on(&fx, &[&c1], &[Ch::Put(E, "e\n")], "C2 加 e");
    let new = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "NEW");

    let commits = commits_between(&wc, &o, &c2, &CODES).unwrap();
    assert_eq!(
        commits.iter().map(|c| c.oid.as_str()).collect::<Vec<_>>(),
        vec![c1.as_str(), c2.as_str()],
        "从旧到新"
    );
    let mut memo = BTreeMap::new();
    let (tip, map, dropped, rewritten) =
        done(replay_onto(&wc, &new, &commits, &opts(true, &no_superseded), &mut memo).unwrap());
    assert_eq!(dropped, vec![c1.clone()]);
    assert!(rewritten.is_empty());
    assert_eq!(
        map.get(&c1),
        Some(&new),
        "丢弃的 commit 映射到它父提交的新 OID"
    );
    assert_eq!(map.get(&c2), Some(&tip));
    assert_eq!(parent_of(&fx, &tip), new);
    assert_eq!(subject(&fx, &tip), "C2 加 e");
    assert_eq!(show(&fx, &tip, A), "a2\n");
    assert_eq!(show(&fx, &tip, E), "e\n");
    assert_eq!(memo, map, "memo 记下了全部映射");
}

#[test]
fn drop_empty_compares_with_current_parent_not_onto() {
    // W18 的两个反例：判空的基准是「上一个重放结果」，不是 onto
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let new = commit_on(&fx, &[&o], &[Ch::Put("Code/B.cs", "b2\n")], "NEW 改 b");

    // ① 改回去的 commit：结果树 == onto 的树，但不是空的，必须保留
    let c1 = commit_on(&fx, &[&o], &[Ch::Put(A, "x\n")], "C1 a=x");
    let c2 = commit_on(&fx, &[&c1], &[Ch::Put(A, "a1\n")], "C2 a 改回去");
    let commits = commits_between(&wc, &o, &c2, &CODES).unwrap();
    let (tip, _, dropped, _) = done(
        replay_onto(
            &wc,
            &new,
            &commits,
            &opts(true, &no_superseded),
            &mut BTreeMap::new(),
        )
        .unwrap(),
    );
    assert!(dropped.is_empty(), "改回去的 commit 被误丢：{dropped:?}");
    assert_eq!(tree_of(&fx, &tip), tree_of(&fx, &new));
    assert_eq!(subject(&fx, &tip), "C2 a 改回去");

    // ② 真正变空、但结果树 ≠ onto 的树：必须丢弃
    let d1 = commit_on(&fx, &[&o], &[Ch::Put(A, "x\n")], "D1 a=x");
    let d2 = commit_on(&fx, &[&d1], &[Ch::Put("Code/B.cs", "b2\n")], "D2 b=b2");
    let commits = commits_between(&wc, &o, &d2, &CODES).unwrap();
    let (tip, map, dropped, _) = done(
        replay_onto(
            &wc,
            &new,
            &commits,
            &opts(true, &no_superseded),
            &mut BTreeMap::new(),
        )
        .unwrap(),
    );
    assert_eq!(dropped, vec![d2.clone()]);
    assert_eq!(subject(&fx, &tip), "D1 a=x");
    assert_eq!(map.get(&d2), Some(&tip));
    assert_ne!(tree_of(&fx, &tip), tree_of(&fx, &new));
}

#[test]
fn originally_empty_commit_is_kept_and_drop_empty_off_keeps_everything() {
    // 原本就空的 commit 保留（与 git rebase 默认一致）；drop_empty = false（交付）一个不丢
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let new = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "NEW");
    let empty = commit_on(&fx, &[&o], &[], "原本就空");
    let c1 = commit_on(&fx, &[&empty], &[Ch::Put(A, "a2\n")], "C1 已进 SVN");
    let commits = commits_between(&wc, &o, &c1, &CODES).unwrap();

    let (tip, _, dropped, _) = done(
        replay_onto(
            &wc,
            &new,
            &commits,
            &opts(true, &no_superseded),
            &mut BTreeMap::new(),
        )
        .unwrap(),
    );
    assert_eq!(dropped, vec![c1.clone()]);
    assert_eq!(subject(&fx, &tip), "原本就空");
    assert_eq!(parent_of(&fx, &tip), new);

    let (tip, _, dropped, _) = done(
        replay_onto(
            &wc,
            &new,
            &commits,
            &opts(false, &no_superseded),
            &mut BTreeMap::new(),
        )
        .unwrap(),
    );
    assert!(dropped.is_empty());
    assert_eq!(subject(&fx, &tip), "C1 已进 SVN");
    assert_eq!(subject(&fx, &parent_of(&fx, &tip)), "原本就空");
}

#[test]
fn superseded_path_resolves_w10_r2_conflict() {
    // W10-R2：C1 改第 2 行，之后工作区继续改同一行，TortoiseSVN 提交的是最终版
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = thirty_line_base(&fx);
    let c1 = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(A, &lines(&[(2, "two-C1")], &[]))],
        "C1",
    );
    let new = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(A, &lines(&[(2, "two-FINAL")], &[]))],
        "NEW",
    );
    let commits = commits_between(&wc, &o, &c1, &CODES).unwrap();

    // 不认定 SVN 版本是自己更晚的版本：冲突，是 Outcome 而不是 Err
    match replay_onto(
        &wc,
        &new,
        &commits,
        &opts(true, &no_superseded),
        &mut BTreeMap::new(),
    )
    .unwrap()
    {
        ReplayOutcome::Conflict { at, files } => {
            assert_eq!(at, c1);
            assert_eq!(files, vec![A.to_string()]);
        }
        other => panic!("期望冲突，实际 {other:?}"),
    }

    let p = |_: &ReplayCommit| BTreeSet::from([A.to_string()]);
    let (tip, map, dropped, rewritten) =
        done(replay_onto(&wc, &new, &commits, &opts(true, &p), &mut BTreeMap::new()).unwrap());
    assert_eq!(tip, new, "C' 重放后变空丢弃，tip == NEW");
    assert_eq!(dropped, vec![c1.clone()]);
    assert_eq!(map.get(&c1), Some(&new));
    assert_eq!(rewritten, vec![(c1.clone(), vec![A.to_string()])]);
}

#[test]
fn superseded_path_fixes_w14_duplicate_foo() {
    // W14：C 在第 10 行后加 foo()；工作区挪到第 20 行后，TortoiseSVN 提交的是挪过的版本
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = thirty_line_base(&fx);
    let c = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(A, &lines(&[], &[(10, "foo()")]))],
        "C foo",
    );
    let new = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(A, &lines(&[], &[(20, "foo()")]))],
        "NEW",
    );
    let commits = commits_between(&wc, &o, &c, &CODES).unwrap();

    // 先钉住病症：不换路径时干净合并，但 foo() 出现两次
    let (tip, _, dropped, _) = done(
        replay_onto(
            &wc,
            &new,
            &commits,
            &opts(true, &no_superseded),
            &mut BTreeMap::new(),
        )
        .unwrap(),
    );
    assert!(dropped.is_empty());
    assert_eq!(show(&fx, &tip, A).matches("foo()").count(), 2);

    let p = |_: &ReplayCommit| BTreeSet::from([A.to_string()]);
    let (tip, _, dropped, _) =
        done(replay_onto(&wc, &new, &commits, &opts(true, &p), &mut BTreeMap::new()).unwrap());
    assert_eq!(tip, new);
    assert_eq!(dropped, vec![c]);
    assert_eq!(show(&fx, &tip, A).matches("foo()").count(), 1);
}

#[test]
fn superseded_keeps_other_paths_of_same_commit() {
    // C 同时改 a 和 e，只有 a 被 TortoiseSVN 提交（而且提交的是更晚的版本）。C' 只剩 e。
    // 路径带空格、中文和 `[1]`：pathspec 若被当成 glob，`角色 [1].cs` 会连带匹配到
    // `角色 1.cs`，把 e 也换成旧版本（Review Focus 2）
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let a = "Code/角色 [1].cs";
    let e = "Code/角色 1.cs";
    let o = commit_on(
        &fx,
        &[&svn_base(&fx)],
        &[Ch::Put(a, "a0\n"), Ch::Put(e, "e0\n")],
        "O",
    );
    let c = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(a, "a-mid\n"), Ch::Put(e, "e1\n")],
        "C 改 a 与 e",
    );
    let new = commit_on(&fx, &[&o], &[Ch::Put(a, "a-final\n")], "NEW");
    let commits = commits_between(&wc, &o, &c, &CODES).unwrap();

    let p = |_: &ReplayCommit| BTreeSet::from([a.to_string()]);
    let (tip, _, dropped, rewritten) =
        done(replay_onto(&wc, &new, &commits, &opts(true, &p), &mut BTreeMap::new()).unwrap());
    assert!(dropped.is_empty(), "C 还有 e 的改动，不能丢");
    assert_eq!(rewritten, vec![(c.clone(), vec![a.to_string()])]);
    assert_eq!(parent_of(&fx, &tip), new);
    assert_eq!(subject(&fx, &tip), "C 改 a 与 e");
    assert_eq!(show(&fx, &tip, a), "a-final\n");
    assert_eq!(show(&fx, &tip, e), "e1\n");
}

#[test]
fn superseded_path_absent_in_parent_is_deleted_from_c_prime() {
    // C 新增 n，TortoiseSVN 提交的是更晚的版本：C^ 没有 n → C' 里写 mode 0 删掉
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let n = "Code/N.cs";
    let c = commit_on(&fx, &[&o], &[Ch::Put(n, "n-mid\n")], "C 加 n");
    let new = commit_on(&fx, &[&o], &[Ch::Put(n, "n-final\n")], "NEW");
    let commits = commits_between(&wc, &o, &c, &CODES).unwrap();
    // 不换：add/add 冲突
    assert!(matches!(
        replay_onto(
            &wc,
            &new,
            &commits,
            &opts(true, &no_superseded),
            &mut BTreeMap::new()
        )
        .unwrap(),
        ReplayOutcome::Conflict { .. }
    ));
    let p = |_: &ReplayCommit| BTreeSet::from([n.to_string()]);
    let (tip, _, dropped, rewritten) =
        done(replay_onto(&wc, &new, &commits, &opts(true, &p), &mut BTreeMap::new()).unwrap());
    assert_eq!(tip, new);
    assert_eq!(dropped, vec![c.clone()]);
    assert_eq!(rewritten, vec![(c, vec![n.to_string()])]);
}

#[test]
fn superseded_directory_path_is_an_internal_error() {
    // W19：P 只能放文件路径，喂目录会写坏索引——直接报错，不猜
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let c = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "C");
    let new = commit_on(&fx, &[&o], &[Ch::Put("Code/B.cs", "b2\n")], "NEW");
    let commits = commits_between(&wc, &o, &c, &CODES).unwrap();
    let p = |_: &ReplayCommit| BTreeSet::from(["Code".to_string()]);
    let err = replay_onto(&wc, &new, &commits, &opts(true, &p), &mut BTreeMap::new()).unwrap_err();
    assert_eq!(err.code, "t_failed", "{}", err.message);
    assert!(err.message.contains("Code"), "{}", err.message);
}

#[test]
fn shared_commit_replayed_once_across_branches() {
    // R20：两条分支共享 D（已进 SVN）与 S；同一个原始 commit 只重放一次
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let new = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "NEW");
    let d = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "D 已进 SVN");
    let s = commit_on(&fx, &[&d], &[Ch::Put("Code/G.cs", "g\n")], "S 共享");
    let x1 = commit_on(&fx, &[&s], &[Ch::Put("Code/X.cs", "x\n")], "X1");
    let y1 = commit_on(&fx, &[&s], &[Ch::Put("Code/Y.cs", "y\n")], "Y1");

    let mut memo = BTreeMap::new();
    let xs = commits_between(&wc, &o, &x1, &CODES).unwrap();
    let (x_tip, _, x_dropped, _) =
        done(replay_onto(&wc, &new, &xs, &opts(true, &no_superseded), &mut memo).unwrap());
    assert_eq!(x_dropped, vec![d.clone()]);
    let s_new = memo.get(&s).cloned().expect("memo 记下 S");
    assert_eq!(memo.get(&d), Some(&new));
    assert_eq!(parent_of(&fx, &x_tip), s_new);

    let ys = commits_between(&wc, &o, &y1, &CODES).unwrap();
    let (y_tip, y_map, y_dropped, _) =
        done(replay_onto(&wc, &new, &ys, &opts(true, &no_superseded), &mut memo).unwrap());
    assert_eq!(y_dropped, vec![d.clone()], "memo 命中的丢弃照样报告");
    assert_eq!(parent_of(&fx, &y_tip), s_new, "两条分支共享同一个 S'");
    assert_eq!(y_map.get(&s), Some(&s_new));

    // 证明 memo 真被查了：把 S 预先映射到一个别的 commit，Y1' 就挂在它上面
    let sentinel = commit_on(&fx, &[&new], &[Ch::Put("Code/G.cs", "g\n")], "sentinel");
    let mut memo2 = BTreeMap::from([(d.clone(), new.clone()), (s.clone(), sentinel.clone())]);
    let (y_tip2, _, _, _) =
        done(replay_onto(&wc, &new, &ys, &opts(true, &no_superseded), &mut memo2).unwrap());
    assert_eq!(parent_of(&fx, &y_tip2), sentinel);
    assert_eq!(subject(&fx, &y_tip2), "Y1");
}

#[test]
fn replay_refuses_merge_commits() {
    // 重放要求线性（调用方先 assert_linear）；漏过来的 merge 报失败码，不静默压平
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let l = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "L");
    let r = commit_on(&fx, &[&o], &[Ch::Put(E, "e\n")], "R");
    let m = commit_on(&fx, &[&l, &r], &[Ch::Put(E, "e\n")], "M");
    let new = commit_on(&fx, &[&o], &[Ch::Put("Code/B.cs", "b2\n")], "NEW");
    let commits = commits_between(&wc, &o, &m, &CODES).unwrap();
    let err = replay_onto(
        &wc,
        &new,
        &commits,
        &opts(true, &no_superseded),
        &mut BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(err.code, "t_failed", "{}", err.message);
}

#[test]
fn all_committed_true_through_no_ff_merge_with_delete_add_mode_rename() {
    // W20 / U5：外部工具 `--no-ff` 合回来、内容全部提交到 SVN 之后，走捷径
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    // 主线：删 B
    let m1 = commit_on(&fx, &[&o], &[Ch::Del("Code/B.cs")], "删 B");
    // 侧线：A 改名为 Moved，新增 N，再把 N 改成可执行
    let s1 = commit_on(
        &fx,
        &[&o],
        &[
            Ch::Del(A),
            Ch::Put("Code/Moved.cs", "a1\n"),
            Ch::Put("Code/N.cs", "n\n"),
        ],
        "改名 + 新增",
    );
    let s2 = commit_on(&fx, &[&s1], &[Ch::Exec("Code/N.cs", "n\n")], "N 可执行");
    let head = commit_on(
        &fx,
        &[&m1, &s2],
        &[
            Ch::Del(A),
            Ch::Put("Code/Moved.cs", "a1\n"),
            Ch::Exec("Code/N.cs", "n\n"),
        ],
        "Merge side",
    );
    // NEW：上面全部进了 SVN，外加同事改的一个无关文件
    let new = commit_on(
        &fx,
        &[&o],
        &[
            Ch::Del("Code/B.cs"),
            Ch::Del(A),
            Ch::Put("Code/Moved.cs", "a1\n"),
            Ch::Exec("Code/N.cs", "n\n"),
            Ch::Put("Code/Colleague.cs", "c\n"),
        ],
        "NEW",
    );
    assert!(all_committed(&wc, &o, &head, &new).unwrap());
    // 少提交一样（删除没进 SVN）就不成立
    let partial = commit_on(
        &fx,
        &[&o],
        &[
            Ch::Del(A),
            Ch::Put("Code/Moved.cs", "a1\n"),
            Ch::Exec("Code/N.cs", "n\n"),
        ],
        "NEW 漏了删除",
    );
    assert!(!all_committed(&wc, &o, &head, &partial).unwrap());
    // 分支上没有自己的 commit：trivially 成立
    assert!(all_committed(&wc, &o, &o, &new).unwrap());
}

#[test]
fn all_committed_false_when_mode_differs() {
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let head = commit_on(&fx, &[&o], &[Ch::Exec(A, "a2\n")], "A 改内容并可执行");
    let new_644 = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "NEW 内容同、mode 不同");
    let new_755 = commit_on(&fx, &[&o], &[Ch::Exec(A, "a2\n")], "NEW 完全相同");
    assert!(!all_committed(&wc, &o, &head, &new_644).unwrap());
    assert!(all_committed(&wc, &o, &head, &new_755).unwrap());
}

#[test]
fn all_committed_false_when_local_only_in_range() {
    // 内容全对上了，但区间里有 Local-Only commit：它被一起提交上去了（泄漏），不能走捷径
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    let o = svn_base(&fx);
    let c1 = commit_on(&fx, &[&o], &[Ch::Put(A, "a2\n")], "C1");
    let l = commit_on(
        &fx,
        &[&c1],
        &[Ch::Put("Code/B.cs", "b-local\n")],
        "mute log\n\nLocal-Only: true",
    );
    let new = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(A, "a2\n"), Ch::Put("Code/B.cs", "b-local\n")],
        "NEW",
    );
    assert!(!all_committed(&wc, &o, &l, &new).unwrap());
    assert!(
        all_committed(&wc, &o, &c1, &new).unwrap(),
        "对照：没有 Local-Only 时成立"
    );
}

#[test]
fn replay_cjk_path_conflict_is_reported_verbatim() {
    let fx = hybrid_with_colleague();
    let wc = fx.wc_str();
    // 钉成 git 的默认值：开发机的全局配置常把它关掉，会把转义的问题藏起来
    git(&fx, &["config", "core.quotePath", "true"]);
    let p = "Code/角色 配置.cs";
    let o = commit_on(&fx, &[&svn_base(&fx)], &[Ch::Put(p, &lines(&[], &[]))], "O");
    let c = commit_on(&fx, &[&o], &[Ch::Put(p, &lines(&[(5, "mine")], &[]))], "C");
    let new = commit_on(
        &fx,
        &[&o],
        &[Ch::Put(p, &lines(&[(5, "theirs")], &[]))],
        "NEW",
    );
    let commits = commits_between(&wc, &o, &c, &CODES).unwrap();
    match replay_onto(
        &wc,
        &new,
        &commits,
        &opts(true, &no_superseded),
        &mut BTreeMap::new(),
    )
    .unwrap()
    {
        ReplayOutcome::Conflict { at, files } => {
            assert_eq!(at, c);
            assert_eq!(files, vec![p.to_string()]);
        }
        other => panic!("期望冲突，实际 {other:?}"),
    }
}
