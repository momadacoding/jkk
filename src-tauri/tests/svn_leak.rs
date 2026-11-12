#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! Local-Only 泄漏按 hunk 判定（计划 Task 7；分析 §6.5 F3、§6.11.2；日志 W21）。
//!
//! 被测对象只读对象（B = Local-Only commit 的父提交、L、NEW），与 SVN 无关，所以 fixture
//! 是 `TempDir` 里 `git init` 的纯 git 临时仓库，B / L / NEW 都在对象层造（临时索引 +
//! `commit-tree`，参照 `svn_replay.rs`），不经过工作区。跑完随 `TempDir` 删除。

use app_lib::engine::git_cli::{run_git, run_git_env, run_git_stdin_env};
use app_lib::engine::svn_leak::{
    leak_message, leak_message_en, local_only_leaks, local_only_leaks_on_disk, Leak, LeakVerdict,
};
use app_lib::engine::svn_replay::{commits_between, ReplayCodes, ReplayCommit};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

const CODES: ReplayCodes = ReplayCodes {
    conflict: "t_conflict",
    too_old: "t_too_old",
    too_old_message: "git 太旧",
    failed: "t_failed",
};

/// 路径带空格与中文：判定要按仓库相对路径逐字节对上 `changed_paths`。
const F: &str = "Code/日志 f.cs";
const OTHER: &str = "Code/other.cs";
const BIN: &str = "Art/x.bin";

const LOG: &str = "log(\"enter\")";
const LOG_OFF: &str = "-- log(\"enter\")";

struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let r = Self { dir };
        r.git(&["init", "-q"]);
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "user.email", "test@test.com"]);
        r
    }

    fn s(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    fn git(&self, args: &[&str]) -> String {
        run_git(&self.s(), args).unwrap().trim().to_string()
    }
}

enum Ch<'a> {
    Put(&'a str, &'a [u8]),
    Del(&'a str),
}

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// 在对象层造一个 commit：`parent` 的树（没有就是空树）+ `changes`。不碰工作区与真实索引。
fn commit_on(r: &Repo, parent: Option<&str>, changes: &[Ch], msg: &str) -> String {
    let wc = r.s();
    // 临时索引用绝对路径（W19），放在 .git 下，不进工作区
    let idx = r
        .dir
        .path()
        .join(".git")
        .join(format!("t-{}.index", SEQ.fetch_add(1, Ordering::Relaxed)));
    let idx_s = idx.to_string_lossy().to_string();
    let env = [("GIT_INDEX_FILE", idx_s.as_str())];
    match parent {
        Some(p) => run_git_env(&wc, &["read-tree", p], &env).unwrap(),
        None => run_git_env(&wc, &["read-tree", "--empty"], &env).unwrap(),
    };
    for ch in changes {
        match ch {
            Ch::Put(p, body) => {
                // --no-filters：内容要逐字节进对象，不受 autocrlf 影响
                let oid = run_git_stdin_env(
                    &wc,
                    &["hash-object", "-w", "--no-filters", "--stdin"],
                    body,
                    &[],
                )
                .unwrap()
                .trim()
                .to_string();
                run_git_env(
                    &wc,
                    &[
                        "update-index",
                        "--add",
                        "--cacheinfo",
                        &format!("100644,{oid},{p}"),
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
    if let Some(p) = parent {
        args.push("-p");
        args.push(p);
    }
    args.push("-m");
    args.push(msg);
    r.git(&args)
}

const LOCAL_ONLY_MSG: &str = "L: 注释掉 log\n\nLocal-Only: true";

/// y4.sh 的文件：40 行，第 5、30 行是 `log("enter")`，第 6 行是 `work(1)`，其余是 `lineN`。
/// `repl` 替换某些行；`shift` 在最前面插 3 行。
fn y4(repl: &[(usize, &str)], shift: bool) -> String {
    let mut s = String::new();
    if shift {
        s.push_str("hdr1\nhdr2\nhdr3\n");
    }
    for n in 1..=40 {
        let base = match n {
            5 | 30 => LOG.to_string(),
            6 => "work(1)".to_string(),
            _ => format!("line{n}"),
        };
        let v = repl
            .iter()
            .find(|(k, _)| *k == n)
            .map(|(_, v)| v.to_string())
            .unwrap_or(base);
        s.push_str(&v);
        s.push('\n');
    }
    s
}

/// 20 行（第 n 行是 n），`repl` 替换、`del` 删除、`after` 在某行之后插一行，`head` 在最前面插几行。
fn numbered(
    repl: &[(usize, &str)],
    del: &[usize],
    after: &[(usize, &str)],
    head: &[&str],
) -> String {
    let mut s = String::new();
    for h in head {
        s.push_str(h);
        s.push('\n');
    }
    for n in 1..=20 {
        if !del.contains(&n) {
            match repl.iter().find(|(k, _)| *k == n) {
                Some((_, v)) => s.push_str(&format!("{v}\n")),
                None => s.push_str(&format!("{n}\n")),
            }
        }
        for (_, v) in after.iter().filter(|(k, _)| *k == n) {
            s.push_str(&format!("{v}\n"));
        }
    }
    s
}

fn set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

/// (old_start, lines, verdict)
type Hunks = Vec<(u32, u32, LeakVerdict)>;
/// (场景名, NEW 里该文件的内容, 期望的 hunk 判定)
type Cases<'a> = Vec<(&'a str, String, Hunks)>;

/// 把结果压成 (路径, [(old_start, lines, verdict)])，便于整体断言。
fn shape(leaks: &[Leak]) -> Vec<(String, Hunks)> {
    leaks
        .iter()
        .map(|l| {
            (
                l.path.clone(),
                l.hunks
                    .iter()
                    .map(|h| (h.old_start, h.lines, h.verdict))
                    .collect(),
            )
        })
        .collect()
}

/// y4 的分支形状：B（旧基线）← L（Local-Only，注释第 5、30 行的 log）← Fe（功能，第 6 行改成
/// work(2)）。返回 (仓库, B, L, `commits_between(B, Fe)`)。
fn y4_branch() -> (Repo, String, String, Vec<ReplayCommit>) {
    let r = Repo::new();
    let b_text = y4(&[], false);
    let l_text = y4(&[(5, LOG_OFF), (30, LOG_OFF)], false);
    let fe_text = y4(&[(5, LOG_OFF), (6, "work(2)"), (30, LOG_OFF)], false);
    let b = commit_on(
        &r,
        None,
        &[Ch::Put(F, b_text.as_bytes()), Ch::Put(OTHER, b"o\n")],
        "B",
    );
    let l = commit_on(
        &r,
        Some(&b),
        &[Ch::Put(F, l_text.as_bytes())],
        LOCAL_ONLY_MSG,
    );
    let fe = commit_on(
        &r,
        Some(&l),
        &[Ch::Put(F, fe_text.as_bytes())],
        "F: line6 work(2)",
    );
    let commits = commits_between(&r.s(), &b, &fe, &CODES).unwrap();
    assert_eq!(commits.len(), 2);
    assert!(commits[0].local_only && !commits[1].local_only);
    (r, b, l, commits)
}

#[test]
fn leak_by_hunk_matches_w21_scenarios() {
    use LeakVerdict::Present;
    let (r, b, l, commits) = y4_branch();
    let w2 = (6, "work(2)");
    let c5 = (5, LOG_OFF);
    let c30 = (30, LOG_OFF);
    // (场景, NEW 里 f 的内容, 期望的 hunk 判定)。L 的两个 hunk 都是 `@@ -5 +5 @@`、`@@ -30 +30 @@`
    let cases: Cases = vec![
        (
            "只提交了功能改动（line6 与 line5 相邻）",
            y4(&[w2], false),
            vec![],
        ),
        ("泄漏 line5", y4(&[w2, c5], false), vec![(5, 1, Present)]),
        (
            "全部泄漏",
            y4(&[w2, c5, c30], false),
            vec![(5, 1, Present), (30, 1, Present)],
        ),
        (
            "只泄漏 line30",
            y4(&[w2, c30], false),
            vec![(30, 1, Present)],
        ),
        (
            "前面插入 3 行后泄漏 line5",
            y4(&[w2, c5], true),
            vec![(5, 1, Present)],
        ),
        (
            "前面插入 3 行后泄漏 line30（line5 不泄漏）",
            y4(&[w2, c30], true),
            vec![(30, 1, Present)],
        ),
        ("前面插入 3 行后只提交了功能改动", y4(&[w2], true), vec![]),
    ];
    for (name, new_text, want) in cases {
        let new = commit_on(&r, Some(&b), &[Ch::Put(F, new_text.as_bytes())], name);
        let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new).unwrap();
        if want.is_empty() {
            assert!(leaks.is_empty(), "{name}：期望没有泄漏，实际 {leaks:?}");
            continue;
        }
        assert_eq!(shape(&leaks), vec![(F.to_string(), want)], "{name}");
        let leak = &leaks[0];
        assert_eq!(leak.branch, "main", "{name}");
        assert_eq!(leak.commit, l, "{name}：点名的是 Local-Only commit");
        assert_eq!(leak.subject, "L: 注释掉 log", "{name}");
    }
}

#[test]
fn overlapping_earlier_hunk_that_changes_line_count_is_unknown() {
    // NEW 把第 4–5 行（2 行）改成 3 行，起点早于 L 的第 5 行：映射位置拿不准
    let (r, b, _, commits) = y4_branch();
    let mut text = y4(&[(6, "work(2)")], false);
    text = text.replacen(
        &format!("line4\n{LOG}\n"),
        &format!("LINE4\nx\n{LOG_OFF}\n"),
        1,
    );
    let new = commit_on(&r, Some(&b), &[Ch::Put(F, text.as_bytes())], "NEW");
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new).unwrap();
    assert_eq!(
        shape(&leaks),
        vec![(F.to_string(), vec![(5, 1, LeakVerdict::Unknown)])]
    );
}

#[test]
fn pure_insertion_and_deletion_hunks_are_mapped() {
    use LeakVerdict::{Present, Unknown};
    let r = Repo::new();
    let path = "Code/n.cs";
    let b = commit_on(
        &r,
        None,
        &[Ch::Put(path, numbered(&[], &[], &[], &[]).as_bytes())],
        "B",
    );
    // L：第 10 行之后插一行 DEBUG（`@@ -10,0 +11 @@`），删掉第 15 行（`@@ -15 +14,0 @@`）
    let l_text = numbered(&[], &[15], &[(10, "DEBUG")], &[]);
    let l = commit_on(
        &r,
        Some(&b),
        &[Ch::Put(path, l_text.as_bytes())],
        LOCAL_ONLY_MSG,
    );
    let commits = commits_between(&r.s(), &b, &l, &CODES).unwrap();
    let cases: Cases = vec![
        (
            "前面插 2 行后两处都带上了",
            numbered(&[], &[15], &[(10, "DEBUG")], &["h1", "h2"]),
            vec![(10, 0, Present), (15, 1, Present)],
        ),
        (
            "前面插 2 行、第 15 行被改成别的而不是删掉：删除拿不准，插入没带上",
            numbered(&[(15, "15x")], &[], &[], &["h1", "h2"]),
            vec![(15, 1, Unknown)],
        ),
        (
            "NEW 改的是第 12 行：都不重叠",
            numbered(&[(12, "12x")], &[], &[], &[]),
            vec![],
        ),
    ];
    for (name, new_text, want) in cases {
        let new = commit_on(&r, Some(&b), &[Ch::Put(path, new_text.as_bytes())], name);
        let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[path]), &new).unwrap();
        if want.is_empty() {
            assert!(leaks.is_empty(), "{name}：实际 {leaks:?}");
        } else {
            assert_eq!(shape(&leaks), vec![(path.to_string(), want)], "{name}");
        }
    }
}

#[test]
fn non_local_only_commits_are_ignored() {
    // 功能 commit Fe 的改动原样进了 NEW：这是正常提交，不是泄漏
    let (r, b, _, commits) = y4_branch();
    let new = commit_on(
        &r,
        Some(&b),
        &[Ch::Put(F, y4(&[(6, "work(2)")], false).as_bytes())],
        "NEW",
    );
    let only_feature: Vec<ReplayCommit> =
        commits.iter().filter(|c| !c.local_only).cloned().collect();
    assert_eq!(only_feature.len(), 1);
    let leaks = local_only_leaks(&r.s(), "main", &only_feature, &set(&[F]), &new).unwrap();
    assert!(leaks.is_empty(), "{leaks:?}");

    // 对照：同一个 NEW 再带上 L 的 line5，就要报出来——说明上面是因为 Fe 不是 Local-Only
    let new2 = commit_on(
        &r,
        Some(&b),
        &[Ch::Put(
            F,
            y4(&[(5, LOG_OFF), (6, "work(2)")], false).as_bytes(),
        )],
        "NEW2",
    );
    assert!(
        local_only_leaks(&r.s(), "main", &only_feature, &set(&[F]), &new2)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new2)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn paths_outside_changed_paths_are_ignored() {
    // L 的 line5 进了 NEW，但这次外部变更集合里没有 f（调用方只让看 OTHER）
    let (r, b, _, commits) = y4_branch();
    let new = commit_on(
        &r,
        Some(&b),
        &[
            Ch::Put(F, y4(&[(5, LOG_OFF)], false).as_bytes()),
            Ch::Put(OTHER, b"o2\n"),
        ],
        "NEW",
    );
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[OTHER]), &new).unwrap();
    assert!(leaks.is_empty(), "{leaks:?}");
    let empty = local_only_leaks(&r.s(), "main", &commits, &BTreeSet::new(), &new).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn binary_change_on_both_sides_is_unknown() {
    let r = Repo::new();
    let b = commit_on(&r, None, &[Ch::Put(BIN, b"\x00\x01\x02bin")], "B");
    let l = commit_on(
        &r,
        Some(&b),
        &[Ch::Put(BIN, b"\x00\x01\x03bin")],
        LOCAL_ONLY_MSG,
    );
    let commits = commits_between(&r.s(), &b, &l, &CODES).unwrap();

    // 两侧都改了：按 hunk 比不了，整文件拿不准（old_start = 0、lines = 0 表示整个文件）
    let new = commit_on(&r, Some(&b), &[Ch::Put(BIN, b"\x00\x01\x03bin")], "NEW");
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[BIN]), &new).unwrap();
    assert_eq!(
        shape(&leaks),
        vec![(BIN.to_string(), vec![(0, 0, LeakVerdict::Unknown)])]
    );

    // 对照：NEW 里这个文件没动过（与 B 相同）→ 不可能泄漏
    let untouched = commit_on(&r, Some(&b), &[Ch::Put(OTHER, b"o\n")], "NEW2");
    let leaks =
        local_only_leaks(&r.s(), "main", &commits, &set(&[BIN, OTHER]), &untouched).unwrap();
    assert!(leaks.is_empty(), "{leaks:?}");
}

#[test]
fn deleted_in_both_is_present() {
    // L 删掉 f（本地不要这个文件），外部提交也把它删了：泄漏的是删除本身
    let (r, b, _, _) = y4_branch();
    let l = commit_on(&r, Some(&b), &[Ch::Del(F)], LOCAL_ONLY_MSG);
    let commits = commits_between(&r.s(), &b, &l, &CODES).unwrap();
    let new = commit_on(&r, Some(&b), &[Ch::Del(F)], "NEW");
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new).unwrap();
    assert_eq!(
        shape(&leaks),
        vec![(F.to_string(), vec![(0, 0, LeakVerdict::Present)])]
    );

    // 对照：L 删了，NEW 只是改了内容 → 删除没有上服务器
    let modified = commit_on(
        &r,
        Some(&b),
        &[Ch::Put(F, y4(&[(6, "work(2)")], false).as_bytes())],
        "NEW2",
    );
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &modified).unwrap();
    assert!(leaks.is_empty(), "{leaks:?}");
}

#[test]
fn path_deleted_in_new_is_absent() {
    // L 改了 f，NEW 把 f 删了：L 的内容不在 NEW 里
    let (r, b, _, commits) = y4_branch();
    let new = commit_on(&r, Some(&b), &[Ch::Del(F)], "NEW");
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new).unwrap();
    assert!(leaks.is_empty(), "{leaks:?}");
}

#[test]
fn unsubmitted_feature_below_local_only_maps_through_its_reverse_diff() {
    // 真实分支的形态：L 的父提交不是旧基线，而是一个还没提交到 SVN 的功能 commit（最前面插了
    // 3 行）。于是 B→NEW 带着这 3 行的反向删除，偏移是负的：B 的第 8 行对应 NEW 的第 5 行
    let r = Repo::new();
    let o = commit_on(&r, None, &[Ch::Put(F, y4(&[], false).as_bytes())], "O");
    let fe = commit_on(
        &r,
        Some(&o),
        &[Ch::Put(F, y4(&[], true).as_bytes())],
        "F: 文件头",
    );
    let l = commit_on(
        &r,
        Some(&fe),
        &[Ch::Put(
            F,
            y4(&[(5, LOG_OFF), (30, LOG_OFF)], true).as_bytes(),
        )],
        LOCAL_ONLY_MSG,
    );
    let commits = commits_between(&r.s(), &o, &l, &CODES).unwrap();
    assert_eq!(commits.len(), 2);

    // 外部只提交了 line5（NEW 接在旧基线上，没有那 3 行文件头）
    let new = commit_on(
        &r,
        Some(&o),
        &[Ch::Put(F, y4(&[(5, LOG_OFF)], false).as_bytes())],
        "NEW",
    );
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new).unwrap();
    assert_eq!(
        shape(&leaks),
        vec![(F.to_string(), vec![(8, 1, LeakVerdict::Present)])],
        "坐标是 B 的行号；第 30 行（B 的第 33 行）没有上去"
    );
    assert_eq!(leaks[0].commit, l);
}

#[test]
fn file_added_by_local_only_and_by_new_with_same_content_is_present() {
    // L 新增一个本地配置文件，外部提交也加了同样内容的同名文件
    let (r, b, _, _) = y4_branch();
    let cfg = "Code/local.cfg";
    let l = commit_on(&r, Some(&b), &[Ch::Put(cfg, b"debug=1\n")], LOCAL_ONLY_MSG);
    let commits = commits_between(&r.s(), &b, &l, &CODES).unwrap();
    let new = commit_on(&r, Some(&b), &[Ch::Put(cfg, b"debug=1\n")], "NEW");
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[cfg]), &new).unwrap();
    assert_eq!(
        shape(&leaks),
        vec![(cfg.to_string(), vec![(0, 0, LeakVerdict::Present)])]
    );
}

// —— Review 2：hunk 不逐字相等时的漏报 ——

/// 10 行，第 n 行是 `ln`。
fn ten() -> String {
    (1..=10).map(|i| format!("l{i}\n")).collect()
}

/// 只有一个文件、一个 Local-Only commit 的判定。
fn judge(b: &str, l: &str, n: &str) -> Hunks {
    let r = Repo::new();
    let base = commit_on(&r, None, &[Ch::Put(F, b.as_bytes())], "B");
    let lc = commit_on(&r, Some(&base), &[Ch::Put(F, l.as_bytes())], LOCAL_ONLY_MSG);
    let new = commit_on(&r, Some(&base), &[Ch::Put(F, n.as_bytes())], "NEW");
    let commits = commits_between(&r.s(), &base, &lc, &CODES).unwrap();
    let leaks = local_only_leaks(&r.s(), "main", &commits, &set(&[F]), &new).unwrap();
    let hunks = match shape(&leaks).into_iter().next() {
        Some((_, hunks)) => hunks,
        None => Vec::new(),
    };
    // C1h Task 4：Check-commit 走的真实路径（N 是磁盘文件）与按对象判定必须同一结论。
    // 走 `local_only_leaks_on_disk` 而不是某个只有测试在调的函数（Review 2 第 1 条）
    let (dr, _) = disk_repo(b, l, Some(n));
    let on_disk = local_only_leaks_on_disk(&dr.s(), &[abs(&dr, F)]).unwrap();
    let on_disk = match shape(&on_disk).into_iter().next() {
        Some((_, hunks)) => hunks,
        None => Vec::new(),
    };
    assert_eq!(on_disk, hunks, "磁盘路径与 local_only_leaks 结论不一致");
    hunks
}

#[test]
fn disk_path_matches_local_only_leaks_on_w21_cases() {
    let b = y4(&[], false);
    let l = y4(&[(5, LOG_OFF), (30, LOG_OFF)], false);
    let w2 = (6, "work(2)");
    let c5 = (5, LOG_OFF);
    let c30 = (30, LOG_OFF);
    use LeakVerdict::Present;
    // `judge` 内部断言了磁盘路径与 local_only_leaks 结论一致；这里再钉住结论本身
    let cases: Vec<(&str, String, Hunks)> = vec![
        ("只提交了功能改动", y4(&[w2], false), vec![]),
        ("泄漏 line5", y4(&[w2, c5], false), vec![(5, 1, Present)]),
        (
            "全部泄漏",
            y4(&[w2, c5, c30], false),
            vec![(5, 1, Present), (30, 1, Present)],
        ),
        (
            "前面插入 3 行后泄漏 line30",
            y4(&[w2, c30], true),
            vec![(30, 1, Present)],
        ),
    ];
    for (name, n, want) in cases {
        assert_eq!(judge(&b, &l, &n), want, "{name}");
    }
}

/// 磁盘路径的边界形状。每一条都与按对象判定（`local_only_leaks`）对照过语义。
fn disk_shape(b: Option<&[u8]>, l: Option<&[u8]>, n: Option<&[u8]>) -> Hunks {
    let (r, _) = disk_repo_bytes(b, l, n);
    let leaks = local_only_leaks_on_disk(&r.s(), &[abs(&r, F)]).unwrap();
    match shape(&leaks).into_iter().next() {
        Some((_, hunks)) => hunks,
        None => Vec::new(),
    }
}

#[test]
fn disk_equal_to_base_is_not_a_leak() {
    // 文本
    assert!(disk_shape(Some(b"a\n"), Some(b"x\n"), Some(b"a\n")).is_empty());
    // 二进制：按行比不了，但 N 与 B 完全相同就是没碰（`judge_path` 的 `n == b`）。
    // 少了这一步，Local-Only 改过、这次又原样提交的二进制文件会被判 Unknown 挡下（Review 2 第 1 条）
    assert!(disk_shape(Some(b"a\0"), Some(b"b\0"), Some(b"a\0")).is_empty());
    // L 新增、磁盘上也没有
    assert!(disk_shape(None, Some(b"log\n"), None).is_empty());
}

#[test]
fn disk_edge_shapes() {
    use LeakVerdict::{Present, Unknown};
    // L 删了文件、磁盘上也没有：删除泄漏
    assert_eq!(disk_shape(Some(b"a\n"), None, None), vec![(0, 0, Present)]);
    // L 删了、磁盘上只是改了：删除没上去
    assert!(disk_shape(Some(b"a\n"), None, Some(b"b\n")).is_empty());
    // 二进制（含 NUL）且都不同：比不了
    assert_eq!(
        disk_shape(Some(b"a\0"), Some(b"b\0"), Some(b"c\0")),
        vec![(0, 0, Unknown)]
    );
    // L 新增的文件：B 当空文件
    assert_eq!(
        disk_shape(None, Some(b"log\n"), Some(b"log\n")),
        vec![(0, 0, Present)]
    );
}

// —— C1h Task 4：Check-commit 拿磁盘上这一刻的内容当 N ——

/// B（`svn/base`）← L（Local-Only）← HEAD 在 main 上，工作区按 `disk` 写。返回 (仓库, L)。
fn disk_repo(b: &str, l: &str, disk: Option<&str>) -> (Repo, String) {
    disk_repo_bytes(
        Some(b.as_bytes()),
        Some(l.as_bytes()),
        disk.map(str::as_bytes),
    )
}

/// `None` 表示这一侧没有这个文件。
fn disk_repo_bytes(b: Option<&[u8]>, l: Option<&[u8]>, disk: Option<&[u8]>) -> (Repo, String) {
    let r = Repo::new();
    let mut base_changes = vec![Ch::Put(OTHER, b"o\n")];
    if let Some(b) = b {
        base_changes.push(Ch::Put(F, b));
    }
    let base = commit_on(&r, None, &base_changes, "B");
    let change = match l {
        Some(l) => Ch::Put(F, l),
        None => Ch::Del(F),
    };
    let lc = commit_on(&r, Some(&base), &[change], LOCAL_ONLY_MSG);
    r.git(&["update-ref", "refs/heads/svn/base", &base]);
    r.git(&["update-ref", "refs/heads/main", &lc]);
    r.git(&["symbolic-ref", "HEAD", "refs/heads/main"]);
    let f = r.dir.path().join(F);
    std::fs::create_dir_all(f.parent().unwrap()).unwrap();
    std::fs::write(r.dir.path().join(OTHER), "o changed\n").unwrap();
    if let Some(d) = disk {
        std::fs::write(&f, d).unwrap();
    }
    (r, lc)
}

/// TortoiseSVN 给的绝对路径：正斜杠（H2）。
fn abs(r: &Repo, rel: &str) -> String {
    format!("{}/{rel}", r.s().replace('\\', "/"))
}

#[test]
fn restore_after_commit_with_patch_removed_is_not_a_leak() {
    // H5 的形状：20 行；L 改第 2 行；提交前用户删掉了第 2 行的补丁，只留第 18 行的功能改动
    let b: String = (1..=20).map(|i| format!("line {i:02}\r\n")).collect();
    let l = b.replace("line 02\r\n", "line 02 DEBUG_LOCAL_PATCH\r\n");
    let disk = b.replace("line 18\r\n", "line 18 FEATURE\r\n");
    let (r, _) = disk_repo(&b, &l, Some(&disk));
    let leaks = local_only_leaks_on_disk(&r.s(), &[abs(&r, F)]).unwrap();
    assert!(leaks.is_empty(), "{leaks:?}");
}

#[test]
fn whole_file_commit_with_patch_is_present() {
    let b: String = (1..=20).map(|i| format!("line {i:02}\r\n")).collect();
    let l = b.replace("line 02\r\n", "line 02 DEBUG_LOCAL_PATCH\r\n");
    let disk = l.replace("line 18\r\n", "line 18 FEATURE\r\n");
    let (r, lc) = disk_repo(&b, &l, Some(&disk));
    let leaks = local_only_leaks_on_disk(&r.s(), &[abs(&r, F)]).unwrap();
    assert_eq!(
        shape(&leaks),
        vec![(F.to_string(), vec![(2, 1, LeakVerdict::Present)])]
    );
    assert_eq!(
        (leaks[0].commit.as_str(), leaks[0].branch.as_str()),
        (lc.as_str(), "main")
    );
    assert_eq!(leaks[0].subject, "L: 注释掉 log");
}

#[test]
fn disk_check_ignores_paths_not_in_the_commit_or_not_touched_by_local_only() {
    let b = ten();
    let l = b.replace("l4\n", "log\n");
    let (r, _) = disk_repo(&b, &l, Some(&l));
    // F 带着补丁，但这次没勾 F，只勾了 OTHER（L 没碰它）
    assert!(local_only_leaks_on_disk(&r.s(), &[abs(&r, OTHER)])
        .unwrap()
        .is_empty());
    // 仓库外的路径
    assert!(
        local_only_leaks_on_disk(&r.s(), &["Z:/elsewhere/x.cs".to_string()])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn disk_check_matches_paths_case_insensitively() {
    let b = ten();
    let l = b.replace("l4\n", "log\n");
    let (r, _) = disk_repo(&b, &l, Some(&l));
    let upper = abs(&r, F).to_uppercase().replace("日志 F.CS", "日志 f.cs");
    assert_eq!(local_only_leaks_on_disk(&r.s(), &[upper]).unwrap().len(), 1);
}

#[test]
fn disk_check_without_svn_base_is_an_error() {
    let (r, _) = disk_repo(&ten(), &ten().replace("l4\n", "log\n"), None);
    r.git(&["update-ref", "-d", "refs/heads/svn/base"]);
    assert!(local_only_leaks_on_disk(&r.s(), &[abs(&r, F)]).is_err());
}

#[test]
fn leak_message_names_paths_subjects_and_the_way_out() {
    let b: String = (1..=20).map(|i| format!("line {i:02}\r\n")).collect();
    let l = b.replace("line 02\r\n", "line 02 DEBUG_LOCAL_PATCH\r\n");
    let (r, _) = disk_repo(&b, &l, Some(&l));
    let msg = leak_message(&local_only_leaks_on_disk(&r.s(), &[abs(&r, F)]).unwrap());
    assert!(msg.contains(F), "{msg}");
    assert!(msg.contains("L: 注释掉 log"), "{msg}");
    assert!(msg.contains("第 2 行"), "{msg}");
    assert_eq!(
        msg.lines().last(),
        Some("取消勾选这些文件，或用「提交后恢复」删掉这些改动再提交。")
    );
}

/// 线程代码页装不下中文时发的英文版（2026-11-11 使用报告）：内容与中文版逐条对应。
#[test]
fn leak_message_en_names_the_same_paths_lines_and_subjects() {
    let b: String = (1..=20).map(|i| format!("line {i:02}\r\n")).collect();
    let l = b.replace("line 02\r\n", "line 02 DEBUG_LOCAL_PATCH\r\n");
    let (r, _) = disk_repo(&b, &l, Some(&l));
    let msg = leak_message_en(&local_only_leaks_on_disk(&r.s(), &[abs(&r, F)]).unwrap());
    assert_eq!(
        msg,
        format!(
            "jkk: these files contain Local-Only patches; committing would upload them to the server:\n  \
             {F}: line 2 onward, from commit \"L: 注释掉 log\"\n\
             Uncheck these files, or use \"Restore after commit\" to drop these changes before committing."
        )
    );
}

#[test]
fn leak_inserted_at_the_same_point_as_a_feature_line_is_present() {
    // 探针 a：同一点插入，NEW 的 hunk 是 feature + log，映射位置上是 feature
    let b = ten();
    let l = b.replace("l4\n", "l4\nlog\n");
    let n = b.replace("l4\n", "l4\nfeature\nlog\n");
    assert_eq!(judge(&b, &l, &n), vec![(4, 0, LeakVerdict::Present)]);
}

#[test]
fn leak_below_a_line_added_in_the_same_hunk_is_present() {
    // 探针 b：`l5→//l5` 对 `feature` + `//l5`
    let b = ten();
    let l = b.replace("l5\n", "//l5\n");
    let n = b.replace("l5\n", "feature\n//l5\n");
    assert_eq!(judge(&b, &l, &n), vec![(5, 1, LeakVerdict::Present)]);
}

#[test]
fn half_a_hunk_on_the_server_is_unknown() {
    // 探针 c：一个 hunk 改 l5、l6，只有 //l6 上了服务器
    let b = ten();
    let l = b.replace("l5\nl6\n", "//l5\n//l6\n");
    let n = b.replace("l6\n", "//l6\n");
    assert_eq!(judge(&b, &l, &n), vec![(5, 2, LeakVerdict::Unknown)]);
}

#[test]
fn same_hunk_with_none_of_the_local_lines_is_absent() {
    // 反例：NEW 改的是同一行，内容完全是别的
    let b = ten();
    let l = b.replace("l5\n", "//l5\n");
    assert_eq!(judge(&b, &l, &b.replace("l5\n", "l5-feature\n")), vec![]);
    assert_eq!(
        judge(&b, &l, &b.replace("l5\n", "feature1\nfeature2\n")),
        vec![]
    );
    // 同一点插入、内容不同
    let l = b.replace("l4\n", "l4\nlog\n");
    assert_eq!(judge(&b, &l, &b.replace("l4\n", "l4\nfeature\n")), vec![]);
}

#[test]
fn repeated_local_lines_are_counted_not_just_seen() {
    // L 的 `+` 行有两条相同的，NEW 只带上去一条：是半个 hunk，不是全部
    let b = ten();
    let l = b.replace("l5\n", "x\nx\n");
    let n = b.replace("l5\n", "x\nfeature\nfeature2\n");
    assert_eq!(judge(&b, &l, &n), vec![(5, 1, LeakVerdict::Unknown)]);
}

#[test]
fn insertion_right_below_a_changed_line_is_compared() {
    // L 在 l4 之后插 log，提交时 l4 也改了：B→NEW 是一个 `@@ -4 +4,2 @@`，在 B 里止于插入点。
    // 「在前面」的 hunk 不能挡住「碰到边界算重叠」
    let b = ten();
    let l = b.replace("l4\n", "l4\nlog\n");
    let n = b.replace("l4\n", "l4-feature\nlog\n");
    assert_eq!(judge(&b, &l, &n), vec![(4, 0, LeakVerdict::Present)]);
    // 反例：同一个形状，插进来的不是 log
    assert_eq!(
        judge(&b, &l, &b.replace("l4\n", "l4-feature\nother\n")),
        vec![]
    );
}

#[test]
fn line_inserted_right_above_a_local_hunk_still_shifts_the_mapping() {
    // NEW 紧贴 h 上方纯插入：既要算偏移，也要算重叠。第一条 l5 原样留着、L 的行插在它上面
    let b = ten();
    let l = b.replace("l5\n", "//l5\n");
    assert_eq!(
        judge(&b, &l, &b.replace("l5\n", "//l5\nl5\n")),
        vec![(5, 1, LeakVerdict::Present)]
    );
    assert_eq!(judge(&b, &l, &b.replace("l5\n", "feature\nl5\n")), vec![]);
}

#[test]
fn pure_deletion_next_to_an_insertion_is_not_a_leak() {
    // L 删 l5。NEW 只在 l5 上方 / 下方插一行：这几行还在，不重叠
    let b = ten();
    let l = b.replace("l5\n", "");
    assert_eq!(judge(&b, &l, &b.replace("l5\n", "x\nl5\n")), vec![]);
    assert_eq!(judge(&b, &l, &b.replace("l5\n", "l5\nx\n")), vec![]);
}

#[test]
fn trailing_newline_is_not_part_of_the_comparison() {
    // L 的行在文件末尾没有换行，上了服务器之后后面又接了内容
    let b = "l1\nl2\n";
    let l = "l1\nl2\nlog";
    let n = "l1\nl2\nfeature\nlog\n";
    assert_eq!(judge(b, l, n), vec![(2, 0, LeakVerdict::Present)]);
}
