//! 跟进时拼索引（计划 Task 8；分析 §6.11.1；日志 W22–W26、W28）。
//!
//! 被测对象只碰 `.git/`（索引、对象、一条 ref），与 SVN 无关，所以 fixture 是 `TempDir` 里
//! `git init` 的纯 git 临时仓库（同 `svn_leak.rs`）。NEW 在对象层造（临时索引 + `commit-tree`）：
//! 真实流程里 NEW 也是录出来的，磁盘从头到尾不被 git 写，这正是要断言的形态。

use app_lib::engine::git_cli::{run_git, run_git_code, run_git_env, run_git_stdin_env};
use app_lib::engine::svn_follow_index::{
    prepare_index, prepare_index_with, ComposeReport, PathFacts, StageDecision,
};
use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

/// 路径带空格、中文、`[]`、`%`：不能被当 glob，也不能被转义（Review Focus 2）。
const ODD: &str = "Code/日志 f[1]%20.cs";

const ME_BEFORE: PathFacts = PathFacts {
    author_is_me: Some(true),
    staged_before_mark: Some(true),
};
const ME_AFTER: PathFacts = PathFacts {
    author_is_me: Some(true),
    staged_before_mark: Some(false),
};
const ME_UNKNOWN: PathFacts = PathFacts {
    author_is_me: Some(true),
    staged_before_mark: None,
};
const COLLEAGUE: PathFacts = PathFacts {
    author_is_me: Some(false),
    staged_before_mark: None,
};
/// D4：作者不知道，按「不是我」走。
const NOBODY_KNOWS: PathFacts = PathFacts {
    author_is_me: None,
    staged_before_mark: None,
};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// `(mode, oid)`。
type Entry = (String, String);

struct Repo {
    dir: TempDir,
}

impl Repo {
    fn new() -> Self {
        let r = Self {
            dir: TempDir::new().unwrap(),
        };
        r.git(&["init", "-q", "-b", "master"]);
        r.git(&["config", "user.name", "Test"]);
        r.git(&["config", "user.email", "test@test.com"]);
        // 开发机全局 autocrlf=true；内容要逐字节比较
        r.git(&["config", "core.autocrlf", "false"]);
        r
    }

    /// 写好 `files` 并提交，返回 commit。
    fn seeded(files: &[(&str, &[u8])]) -> (Self, String) {
        let r = Self::new();
        for (p, b) in files {
            r.write(p, b);
        }
        r.git(&["add", "-A"]);
        r.git(&["commit", "-q", "-m", "base"]);
        let old = r.git(&["rev-parse", "HEAD"]);
        (r, old)
    }

    fn s(&self) -> String {
        self.dir.path().to_string_lossy().to_string()
    }

    fn git(&self, args: &[&str]) -> String {
        run_git(&self.s(), args).unwrap().trim().to_string()
    }

    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    fn stage(&self, rel: &str, bytes: &[u8]) {
        self.write(rel, bytes);
        self.git(&["--literal-pathspecs", "add", "--", rel]);
    }

    fn blob(&self, bytes: &[u8]) -> String {
        run_git_stdin_env(
            &self.s(),
            &["hash-object", "-w", "--no-filters", "--stdin"],
            bytes,
            &[],
        )
        .unwrap()
        .trim()
        .to_string()
    }

    /// 对象层造 NEW：`parent` 的树 + `changes`（`None` = 删），不碰工作区与真实索引。
    fn commit_on(&self, parent: &str, changes: &[(&str, Option<&[u8]>)]) -> String {
        self.commit_with_modes(
            parent,
            &changes
                .iter()
                .map(|(p, b)| (*p, "100644", *b))
                .collect::<Vec<_>>(),
        )
    }

    fn commit_with_modes(&self, parent: &str, changes: &[(&str, &str, Option<&[u8]>)]) -> String {
        let wc = self.s();
        let idx = self
            .dir
            .path()
            .join(".git")
            .join(format!("t-{}.index", SEQ.fetch_add(1, Ordering::Relaxed)));
        let idx_s = idx.to_string_lossy().to_string();
        let env = [("GIT_INDEX_FILE", idx_s.as_str())];
        run_git_env(&wc, &["read-tree", parent], &env).unwrap();
        let zero = "0".repeat(parent.len());
        let mut input = Vec::new();
        for (p, mode, body) in changes {
            let line = match body {
                Some(b) => format!("{mode} {}\t{p}", self.blob(b)),
                None => format!("0 {zero}\t{p}"),
            };
            input.extend_from_slice(line.as_bytes());
            input.push(0);
        }
        run_git_stdin_env(&wc, &["update-index", "-z", "--index-info"], &input, &env).unwrap();
        let tree = run_git_env(&wc, &["write-tree"], &env).unwrap();
        std::fs::remove_file(&idx).unwrap();
        self.git(&["commit-tree", tree.trim(), "-p", parent, "-m", "new"])
    }

    /// 真实索引的全部 stage 0 条目。
    fn index(&self) -> BTreeMap<String, Entry> {
        let out = run_git(&self.s(), &["ls-files", "-s", "-z"]).unwrap();
        out.split('\0')
            .filter(|r| !r.is_empty())
            .map(|r| {
                let (meta, path) = r.split_once('\t').unwrap();
                let mut f = meta.split(' ');
                let (mode, oid) = (f.next().unwrap(), f.next().unwrap());
                (path.to_string(), (mode.to_string(), oid.to_string()))
            })
            .collect()
    }

    fn entry(&self, path: &str) -> Option<Entry> {
        self.index().get(path).cloned()
    }

    fn at(&self, rev: &str, path: &str) -> Entry {
        let out = run_git(
            &self.s(),
            &[
                "--literal-pathspecs",
                "ls-tree",
                "-z",
                "--full-tree",
                rev,
                "--",
                path,
            ],
        )
        .unwrap();
        let (meta, _) = out.split_once('\t').unwrap();
        let mut f = meta.split(' ');
        let mode = f.next().unwrap().to_string();
        f.next();
        (mode, f.next().unwrap().to_string())
    }

    /// 暂存区相对 `rev` 的路径（`--name-status`），看「拼出来的索引到底暂存了什么」。
    fn staged_vs(&self, rev: &str) -> Vec<String> {
        let out = self.git(&[
            "diff-index",
            "--cached",
            "--no-renames",
            "--name-status",
            rev,
        ]);
        out.lines().map(str::to_string).collect()
    }

    fn cached_blob(&self, oid: &str) -> Vec<u8> {
        app_lib::engine::git_cli::run_git_bytes(&self.s(), &["cat-file", "blob", oid]).unwrap()
    }

    fn flags(&self) -> BTreeMap<String, String> {
        let out = run_git(&self.s(), &["ls-files", "-v", "-z"]).unwrap();
        out.split('\0')
            .filter_map(|r| r.split_once(' '))
            .map(|(t, p)| (p.to_string(), t.to_string()))
            .collect()
    }

    fn index_path(&self) -> std::path::PathBuf {
        self.dir.path().join(".git").join("index")
    }

    fn lock_path(&self) -> std::path::PathBuf {
        self.dir.path().join(".git").join("index.lock")
    }

    fn index_bytes(&self) -> Vec<u8> {
        std::fs::read(self.index_path()).unwrap()
    }

    /// 工作区所有文件的 (内容, mtime)。`.git` 除外。
    fn disk(&self) -> BTreeMap<String, (Vec<u8>, std::time::SystemTime)> {
        fn walk(
            base: &Path,
            dir: &Path,
            out: &mut BTreeMap<String, (Vec<u8>, std::time::SystemTime)>,
        ) {
            for e in std::fs::read_dir(dir).unwrap() {
                let e = e.unwrap();
                let p = e.path();
                if e.file_name() == ".git" {
                    continue;
                }
                if p.is_dir() {
                    walk(base, &p, out);
                } else {
                    let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
                    let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
                    out.insert(rel, (std::fs::read(&p).unwrap(), mtime));
                }
            }
        }
        let mut m = BTreeMap::new();
        walk(self.dir.path(), self.dir.path(), &mut m);
        m
    }

    /// 模拟编排：CAS 当前分支 → rename。断言完成后不留锁。
    fn publish(&self, old: &str, new: &str, facts: &dyn Fn(&str) -> PathFacts) -> ComposeReport {
        let mut p = prepare_index(&self.s(), "op1", old, new, facts).unwrap();
        self.git(&["update-ref", "refs/heads/master", new, old]);
        p.commit().unwrap();
        assert!(!self.lock_path().exists(), "commit 之后不能留下 index.lock");
        p.report.clone()
    }
}

fn all(f: PathFacts) -> impl Fn(&str) -> PathFacts {
    move |_| f
}

/// 每一行 `line{i}`，共 `n` 行；`edits` 按 1 基行号替换。
fn lines(n: usize, edits: &[(usize, &str)]) -> Vec<u8> {
    let mut out = String::new();
    for i in 1..=n {
        match edits.iter().find(|(l, _)| *l == i) {
            Some((_, s)) => out.push_str(s),
            None => out.push_str(&format!("line{i}")),
        }
        out.push('\n');
    }
    out.into_bytes()
}

// ---- 计划列出的十条 ----

#[test]
fn unstaged_committed_file_takes_new_and_disk_untouched() {
    // W8 / W12：a 没暂存、已提交；b 提交之后又改了一行；c 没暂存也没提交
    let (r, old) = Repo::seeded(&[
        ("a.txt", b"a1\n"),
        ("b.txt", b"b1\n"),
        ("c.txt", b"c1\n"),
        ("gone.txt", b"g\n"),
    ]);
    r.write("a.txt", b"a2\n");
    r.write("b.txt", b"b3\n");
    r.write("c.txt", b"c9\n");
    std::fs::remove_file(r.dir.path().join("gone.txt")).unwrap();
    let new = r.commit_on(
        &old,
        &[
            ("a.txt", Some(b"a2\n")),
            ("b.txt", Some(b"b2\n")),
            ("gone.txt", None),
        ],
    );
    let disk = r.disk();

    let calls = Cell::new(0);
    let report = r.publish(&old, &new, &|_: &str| {
        calls.set(calls.get() + 1);
        COLLEAGUE
    });

    assert_eq!(report, ComposeReport::default(), "没有暂存就没有任何决策");
    assert_eq!(calls.get(), 0, "没有暂存的路径不该去问作者");
    assert!(r.staged_vs("HEAD").is_empty(), "暂存区应等于 NEW");
    // `git()` 去掉了首尾空白，所以第一行开头的空格不见了
    assert_eq!(r.git(&["status", "--porcelain=v1"]), "M b.txt\n M c.txt");
    assert_eq!(r.disk(), disk, "磁盘一个字节都不能动");
}

#[test]
fn intermediate_stage_before_mark_is_discarded_for_own_commit() {
    // U2：暂存中间版 v2，继续改到 v3，TortoiseSVN 提交 v3
    let (r, old) = Repo::seeded(&[("a.txt", b"v1\n")]);
    r.stage("a.txt", b"v2\n");
    r.write("a.txt", b"v3\n");
    let new = r.commit_on(&old, &[("a.txt", Some(b"v3\n"))]);
    let disk = r.disk();

    let report = r.publish(&old, &new, &all(ME_BEFORE));

    assert_eq!(report.decisions["a.txt"], StageDecision::TakeNew);
    assert_eq!(report.discarded, vec!["a.txt".to_string()]);
    assert!(report.uncertain.is_empty() && report.lost.is_empty());
    assert_eq!(r.git(&["status", "--porcelain=v1"]), "");
    assert_eq!(r.disk(), disk);
}

#[test]
fn stage_after_mark_is_kept_and_reported_uncertain() {
    // 提交了 v3，还没跟进就接着改到 v4 并暂存：v4 比 SVN 新，是真实的存档
    for facts in [ME_AFTER, ME_UNKNOWN] {
        let (r, old) = Repo::seeded(&[("a.txt", b"v1\n")]);
        r.stage("a.txt", b"v4\n");
        r.write("a.txt", b"v5\n");
        let staged = r.entry("a.txt").unwrap();
        let new = r.commit_on(&old, &[("a.txt", Some(b"v3\n"))]);
        let disk = r.disk();

        let report = r.publish(&old, &new, &all(facts));

        assert_eq!(report.decisions["a.txt"], StageDecision::KeepStaged);
        assert_eq!(report.uncertain, vec!["a.txt".to_string()]);
        assert!(report.lost.is_empty() && report.discarded.is_empty());
        assert_eq!(r.entry("a.txt"), Some(staged), "暂存内容要原样留下");
        assert_eq!(r.disk(), disk);
    }
}

#[test]
fn colleague_change_merges_with_staged_content() {
    // W13：同事改第 1 行、局部更新拉下来；我暂存的是第 9 行
    for facts in [COLLEAGUE, NOBODY_KNOWS] {
        let (r, old) = Repo::seeded(&[(ODD, &lines(9, &[]))]);
        r.stage(ODD, &lines(9, &[(9, "mine")]));
        r.write(ODD, &lines(9, &[(1, "theirs"), (9, "mine")]));
        let new = r.commit_on(&old, &[(ODD, Some(&lines(9, &[(1, "theirs")])))]);
        let disk = r.disk();

        let report = r.publish(&old, &new, &all(facts));

        let StageDecision::Merged(oid) = report.decisions[ODD].clone() else {
            panic!("应该合并成功：{report:?}");
        };
        assert_eq!(r.entry(ODD).unwrap().1, oid);
        assert_eq!(
            r.cached_blob(&oid),
            lines(9, &[(1, "theirs"), (9, "mine")]),
            "合并结果要同时有同事的和我的"
        );
        assert!(report.uncertain.is_empty() && report.lost.is_empty());
        assert_eq!(r.disk(), disk);
    }
}

#[test]
fn colleague_conflict_takes_new_and_reports_lost() {
    for facts in [COLLEAGUE, NOBODY_KNOWS] {
        let (r, old) = Repo::seeded(&[("a.txt", &lines(5, &[]))]);
        r.stage("a.txt", &lines(5, &[(3, "mine")]));
        let new = r.commit_on(&old, &[("a.txt", Some(&lines(5, &[(3, "theirs")])))]);
        let disk = r.disk();

        let report = r.publish(&old, &new, &all(facts));

        assert_eq!(report.decisions["a.txt"], StageDecision::LostToNew);
        assert_eq!(report.lost, vec!["a.txt".to_string()]);
        assert_eq!(r.entry("a.txt"), Some(r.at(&new, "a.txt")));
        assert_eq!(r.disk(), disk);
    }
}

#[test]
fn staged_delete_and_mode_change_survive_when_new_untouched() {
    // W25：NEW 没碰的路径，read-tree 必须原样留下非内容暂存
    let (r, old) = Repo::seeded(&[
        ("del.txt", b"d\n"),
        ("mode.sh", b"#!\n"),
        ("edit.txt", b"e\n"),
        ("other.txt", b"o\n"),
    ]);
    r.git(&["rm", "-q", "--cached", "del.txt"]);
    r.git(&["update-index", "--chmod=+x", "mode.sh"]);
    r.stage("edit.txt", b"e2\n");
    r.stage("added.txt", b"new\n");
    let new = r.commit_on(&old, &[("other.txt", Some(b"o2\n"))]);
    let disk = r.disk();

    let calls = Cell::new(0);
    let report = r.publish(&old, &new, &|_: &str| {
        calls.set(calls.get() + 1);
        COLLEAGUE
    });

    assert!(report.decisions.is_empty(), "交集为空：{report:?}");
    assert_eq!(calls.get(), 0);
    assert_eq!(
        r.staged_vs("HEAD"),
        vec!["A\tadded.txt", "D\tdel.txt", "M\tedit.txt", "M\tmode.sh"]
    );
    assert_eq!(r.entry("mode.sh").unwrap().0, "100755");
    assert_eq!(r.entry("other.txt"), Some(r.at(&new, "other.txt")));
    assert_eq!(r.disk(), disk);
}

#[test]
fn assume_unchanged_flag_restored_after_new_changes_path() {
    let (r, old) = Repo::seeded(&[
        ("plain.txt", b"p\n"),
        ("staged.txt", b"s\n"),
        ("gone.txt", b"g\n"),
        ("keep.txt", b"k\n"),
    ]);
    r.stage("staged.txt", b"s-mine\n");
    for p in ["plain.txt", "staged.txt", "gone.txt", "keep.txt"] {
        r.git(&["update-index", "--assume-unchanged", p]);
    }
    let new = r.commit_on(
        &old,
        &[
            ("plain.txt", Some(b"p2\n")),
            ("staged.txt", Some(b"s2\n")),
            ("gone.txt", None),
        ],
    );

    // gone.txt 的标志不能去补（索引里已没有它，补了是 fatal）
    let report = r.publish(&old, &new, &all(ME_BEFORE));

    assert_eq!(report.decisions["staged.txt"], StageDecision::TakeNew);
    let flags = r.flags();
    assert_eq!(
        flags["plain.txt"], "h",
        "NEW 改了的路径（read-tree 丢标志）"
    );
    assert_eq!(
        flags["staged.txt"], "h",
        "写过条目的路径（--index-info 丢标志）"
    );
    assert_eq!(flags["keep.txt"], "h");
    assert!(!flags.contains_key("gone.txt"));
    assert_eq!(r.entry("plain.txt"), Some(r.at(&new, "plain.txt")));
}

#[test]
fn conflicted_index_is_refused() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let (b1, b2, b3) = (r.blob(b"1\n"), r.blob(b"2\n"), r.blob(b"3\n"));
    let zero = "0".repeat(40);
    // 先摘掉 stage 0，再写 stage 1–3（`--index-info` 的 `mode oid stage\tpath` 形态）
    let input = format!(
        "0 {zero}\ta.txt\0100644 {b1} 1\ta.txt\0100644 {b2} 2\ta.txt\0100644 {b3} 3\ta.txt\0"
    );
    run_git_stdin_env(
        &r.s(),
        &["update-index", "-z", "--index-info"],
        input.as_bytes(),
        &[],
    )
    .unwrap();
    assert!(
        !r.git(&["ls-files", "-u"]).is_empty(),
        "fixture 要有 stage 1-3"
    );
    let new = r.commit_on(&old, &[("b.txt", Some(b"b\n"))]);
    let before = r.index_bytes();

    let err = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE))
        .err()
        .unwrap();

    assert_eq!(err.code, "svn_follow_index_conflicted");
    assert!(!r.lock_path().exists(), "拒绝时不能留下残锁");
    assert_eq!(r.index_bytes(), before);
    assert_eq!(
        r.git(&["for-each-ref", "refs/mygitclient"]),
        "",
        "拒绝时不钉"
    );
}

#[test]
fn pinned_raw_index_restores_byte_for_byte() {
    // W28：冲突之外的全部状态（i-t-a、标志、stat）都要能逐字节还原
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n"), ("s.txt", b"s\n"), ("h.txt", b"h\n")]);
    r.stage("a.txt", b"a-mine\n");
    r.write("ita.txt", b"later\n");
    r.git(&["add", "-N", "ita.txt"]);
    r.git(&["update-index", "--skip-worktree", "s.txt"]);
    r.git(&["update-index", "--assume-unchanged", "h.txt"]);
    let before = r.index_bytes();
    let flags_before = r.flags();
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);

    let report = r.publish(&old, &new, &all(ME_BEFORE));
    assert_eq!(report.decisions["a.txt"], StageDecision::TakeNew);
    assert_ne!(r.index_bytes(), before, "索引确实换过");

    let pinned = r.cached_blob("refs/mygitclient/follow/op1:index");
    assert_eq!(pinned, before, "钉住的必须是换入之前的原始字节");

    // 按撤销的做法写回去
    std::fs::write(r.index_path(), &pinned).unwrap();
    assert_eq!(r.flags(), flags_before);
    assert_eq!(r.entry("a.txt").unwrap().1, r.blob(b"a-mine\n"));
}

#[test]
fn stat_kept_for_untouched_entries() {
    let (r, old) = Repo::seeded(&[("keep.txt", b"k\n"), ("a.txt", b"a\n")]);
    r.write("a.txt", b"a2\n");
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    let keep_before = stat_of(&r, "keep.txt");
    assert!(mtime_of(&keep_before) != "0:0", "fixture：add 记过 stat");

    r.publish(&old, &new, &all(COLLEAGUE));

    assert_eq!(
        stat_of(&r, "keep.txt"),
        keep_before,
        "没涉及的条目 stat 原样"
    );
    assert!(
        mtime_of(&stat_of(&r, "a.txt")) != "0:0",
        "NEW 换进来且与磁盘相同的条目由 refresh 回填"
    );
}

fn stat_of(r: &Repo, path: &str) -> String {
    r.git(&["ls-files", "--debug", "--", path])
}

fn mtime_of(debug: &str) -> String {
    debug
        .lines()
        .find_map(|l| l.trim().strip_prefix("mtime: "))
        .unwrap()
        .to_string()
}

// ---- 决策表其余的行 ----

#[test]
fn index_already_equal_to_new_is_trivial() {
    // 暂存的正好是提交的版本；问作者没有意义
    let (r, old) = Repo::seeded(&[("a.txt", b"v1\n"), ("d.txt", b"d\n")]);
    r.stage("a.txt", b"v2\n");
    r.git(&["rm", "-q", "--cached", "d.txt"]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"v2\n")), ("d.txt", None)]);

    let calls = Cell::new(0);
    let report = r.publish(&old, &new, &|_: &str| {
        calls.set(calls.get() + 1);
        COLLEAGUE
    });

    assert_eq!(calls.get(), 0);
    assert_eq!(report.decisions["a.txt"], StageDecision::TakeNew);
    assert_eq!(report.decisions["d.txt"], StageDecision::TakeNew);
    assert!(report.discarded.is_empty(), "内容没丢，不算丢弃");
    assert!(r.staged_vs("HEAD").is_empty());
}

#[test]
fn colleague_vs_non_content_stage_goes_to_new() {
    // 保留这些暂存会在暂存区留下对 NEW 改动的反向 diff，下次提交把同事的改动撤掉
    let (r, old) = Repo::seeded(&[
        ("del.txt", b"d\n"),
        ("mode.sh", b"m\n"),
        ("mode2.sh", b"x\n"),
    ]);
    r.git(&["rm", "-q", "--cached", "del.txt"]);
    r.git(&["update-index", "--chmod=+x", "mode.sh"]);
    r.stage("added.txt", b"mine\n");
    r.stage("mode2.sh", b"x-mine\n");
    let new = r.commit_with_modes(
        &old,
        &[
            ("del.txt", "100644", Some(b"d2\n")),
            ("mode.sh", "100644", Some(b"m2\n")),
            ("added.txt", "100644", Some(b"theirs\n")),
            ("mode2.sh", "100755", Some(b"x\n")),
        ],
    );

    let report = r.publish(&old, &new, &all(COLLEAGUE));

    for p in ["del.txt", "mode.sh", "added.txt", "mode2.sh"] {
        assert_eq!(report.decisions[p], StageDecision::LostToNew, "{p}");
        assert_eq!(r.entry(p), Some(r.at(&new, p)), "{p}");
    }
    assert_eq!(report.lost.len(), 4);
}

#[test]
fn binary_staged_against_colleague_is_lost() {
    let (r, old) = Repo::seeded(&[("a.bin", b"a\0\n")]);
    r.stage("a.bin", b"mine\0\n");
    let new = r.commit_on(&old, &[("a.bin", Some(b"theirs\0\n"))]);

    let report = r.publish(&old, &new, &all(COLLEAGUE));

    assert_eq!(report.decisions["a.bin"], StageDecision::LostToNew);
    assert_eq!(report.lost, vec!["a.bin".to_string()]);
    assert_eq!(r.entry("a.bin"), Some(r.at(&new, "a.bin")));
}

#[test]
fn unreadable_blob_fails_instead_of_losing_the_stage() {
    // 实测：merge-file 读不到对象退出 128。这不是「合不了」，不能当冲突把暂存判成 lost
    let (r, old) = Repo::seeded(&[(
        "a.txt", b"a
",
    )]);
    let ghost = "1".repeat(40);
    let input = format!("100644 {ghost}	a.txt ");
    run_git_stdin_env(
        &r.s(),
        &["update-index", "-z", "--index-info"],
        input.as_bytes(),
        &[],
    )
    .unwrap();
    let new = r.commit_on(
        &old,
        &[(
            "a.txt",
            Some(
                b"a2
",
            ),
        )],
    );
    let before = r.index_bytes();

    let err = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE))
        .err()
        .unwrap();

    assert_eq!(err.code, "svn_follow_index_failed");
    assert!(!r.lock_path().exists());
    assert_eq!(r.index_bytes(), before);
}

#[test]
fn own_commit_keeps_staged_delete_when_uncertain() {
    let (r, old) = Repo::seeded(&[("del.txt", b"d\n")]);
    r.git(&["rm", "-q", "--cached", "del.txt"]);
    let new = r.commit_on(&old, &[("del.txt", Some(b"d2\n"))]);

    let report = r.publish(&old, &new, &all(ME_AFTER));

    assert_eq!(report.decisions["del.txt"], StageDecision::KeepStaged);
    assert_eq!(report.uncertain, vec!["del.txt".to_string()]);
    assert_eq!(r.entry("del.txt"), None, "暂存的删除要留下");
    assert_eq!(r.staged_vs("HEAD"), vec!["D\tdel.txt"]);
}

// ---- i-t-a 与 skip-worktree（Review Focus 3）----

fn ita_invisible(r: &Repo, rev: &str) -> String {
    r.git(&[
        "diff-index",
        "--cached",
        "--ita-invisible-in-index",
        "--name-status",
        rev,
    ])
}

#[test]
fn intent_to_add_survives_when_new_untouched() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    r.write("ita.txt", b"later\n");
    r.git(&["add", "-N", "ita.txt"]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);

    let report = r.publish(&old, &new, &all(COLLEAGUE));

    assert!(report.decisions.is_empty());
    assert!(r.entry("ita.txt").is_some(), "i-t-a 条目不能丢");
    assert_eq!(
        ita_invisible(&r, "HEAD"),
        "",
        "它还得是 i-t-a，不是暂存的空文件"
    );
}

#[test]
fn intent_to_add_gives_way_to_new_without_blocking() {
    // i-t-a 没有暂存内容：保留它（作者是我）会让下次提交删掉 NEW 里的文件，所以永远取 NEW。
    // 作者不是我时照计划记 lost，让用户看到这个路径被同事也加了
    for (facts, lost) in [(ME_BEFORE, false), (ME_AFTER, false), (COLLEAGUE, true)] {
        let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
        r.write("ita.txt", b"disk\n");
        r.git(&["add", "-N", "ita.txt"]);
        let new = r.commit_on(&old, &[("ita.txt", Some(b"server\n"))]);
        let disk = r.disk();

        let report = r.publish(&old, &new, &all(facts));

        let want = if lost {
            StageDecision::LostToNew
        } else {
            StageDecision::TakeNew
        };
        assert_eq!(report.decisions["ita.txt"], want);
        assert_eq!(report.lost.is_empty(), !lost);
        assert!(report.uncertain.is_empty(), "i-t-a 没什么可拿不准的");
        assert!(report.discarded.is_empty(), "i-t-a 没有内容可丢弃");
        assert_eq!(r.entry("ita.txt"), Some(r.at(&new, "ita.txt")));
        assert_eq!(r.staged_vs("HEAD"), Vec::<String>::new(), "标志也要清掉");
        assert_eq!(r.disk(), disk);
    }
}

#[test]
fn empty_staged_file_is_not_mistaken_for_intent_to_add() {
    // 暂存的空文件与 i-t-a 的 oid 相同（e69de29），只有标志位能分开
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    r.stage("empty.txt", b"");
    let new = r.commit_on(&old, &[("empty.txt", Some(b"server\n"))]);

    let report = r.publish(&old, &new, &all(ME_AFTER));

    assert_eq!(report.decisions["empty.txt"], StageDecision::KeepStaged);
    assert_eq!(report.uncertain, vec!["empty.txt".to_string()]);
}

#[test]
fn skip_worktree_flag_survives_changes_and_entry_writes() {
    let (r, old) = Repo::seeded(&[
        ("plain.txt", b"p\n"),
        ("staged.txt", b"s\n"),
        ("keep.txt", b"k\n"),
    ]);
    r.stage("staged.txt", b"s-mine\n");
    for p in ["plain.txt", "staged.txt", "keep.txt"] {
        r.git(&["update-index", "--skip-worktree", p]);
    }
    let new = r.commit_on(
        &old,
        &[("plain.txt", Some(b"p2\n")), ("staged.txt", Some(b"s2\n"))],
    );

    r.publish(&old, &new, &all(ME_BEFORE));

    let flags = r.flags();
    for p in ["plain.txt", "staged.txt", "keep.txt"] {
        assert_eq!(flags[p], "S", "{p}");
    }
}

// ---- 目录 / 文件碰撞 ----

#[test]
fn staged_file_colliding_with_directory_in_new_is_reported_not_swallowed() {
    // 实测：read-tree -i -m 退出 0，把暂存的 dfb 无声删掉（换成 dfb/y）
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    r.stage("dfb", b"mine\n");
    let new = r.commit_on(&old, &[("dfb/y", Some(b"y\n"))]);

    let report = r.publish(&old, &new, &all(ME_AFTER));

    assert_eq!(report.decisions["dfb"], StageDecision::LostToNew);
    assert_eq!(report.lost, vec!["dfb".to_string()]);
    assert!(r.entry("dfb").is_none());
    assert_eq!(r.entry("dfb/y"), Some(r.at(&new, "dfb/y")));
    assert!(r.staged_vs("HEAD").is_empty());
}

#[test]
fn staged_file_below_path_new_adds_as_file_is_reported_not_swallowed() {
    // 实测：read-tree -i -m 退出 0，留下 dfa/x、吞掉 NEW 的 dfa，等于暂存了一个反向删除
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    r.stage("dfa/x", b"mine\n");
    let new = r.commit_on(&old, &[("dfa", Some(b"file\n"))]);

    let report = r.publish(&old, &new, &all(COLLEAGUE));

    assert_eq!(report.decisions["dfa/x"], StageDecision::LostToNew);
    assert_eq!(report.lost, vec!["dfa/x".to_string()]);
    assert_eq!(r.entry("dfa"), Some(r.at(&new, "dfa")));
    assert!(r.staged_vs("HEAD").is_empty());
}

#[test]
fn kept_stage_never_overwrites_a_directory_new_brought_in() {
    // OLD 的文件 a 在 NEW 里成了目录 a/b。KeepStaged 写回 a 会把 a/b 顶掉
    let (r, old) = Repo::seeded(&[("a", b"a\n")]);
    r.stage("a", b"a-mine\n");
    let new = r.commit_on(&old, &[("a", None), ("a/b", Some(b"b\n"))]);

    let report = r.publish(&old, &new, &all(ME_AFTER));

    assert_eq!(report.decisions["a"], StageDecision::LostToNew);
    assert_eq!(report.lost, vec!["a".to_string()]);
    assert!(report.uncertain.is_empty());
    assert_eq!(r.entry("a/b"), Some(r.at(&new, "a/b")));
    assert!(r.staged_vs("HEAD").is_empty());
}

// ---- 锁与收尾 ----

#[test]
fn pin_holds_the_staged_blobs_through_gc() {
    // 探针 P6：暂存 blob 只被索引引用。原始索引钉成 blob 挡不住 gc，`git gc --prune=now` 之后
    // 钉住的索引还在，它指着的内容没了
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n"), ("b.txt", b"b\n"), ("c.txt", b"c\n")]);
    r.stage("a.txt", b"only-in-index-a\n");
    r.stage(ODD, b"only-in-index-odd\n");
    r.git(&["rm", "-q", "--cached", "b.txt"]);
    // i-t-a 在 diff-index 里是 e69de29：对象库里未必有这个空 blob，不能让它把钉索引搞失败
    r.write("ita.txt", b"later\n");
    r.git(&["add", "-N", "ita.txt"]);
    let (a, odd) = (r.blob(b"only-in-index-a\n"), r.blob(b"only-in-index-odd\n"));
    let new = r.commit_on(&old, &[("c.txt", Some(b"c2\n"))]);

    r.publish(&old, &new, &all(COLLEAGUE));
    r.git(&["gc", "--prune=now", "-q"]);

    assert_eq!(
        r.cached_blob(&format!("refs/mygitclient/follow/op1:blobs/{a}")),
        b"only-in-index-a\n"
    );
    assert_eq!(
        r.cached_blob(&format!("refs/mygitclient/follow/op1:blobs/{odd}")),
        b"only-in-index-odd\n"
    );
    let (code, _) = run_git_code(&r.s(), &["cat-file", "-e", &a]).unwrap();
    assert_eq!(code, 0, "按 OID 直接取得到");
}

#[test]
fn pin_without_staged_changes_has_no_blobs_directory() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    r.publish(&old, &new, &all(COLLEAGUE));
    assert_eq!(
        r.git(&["ls-tree", "--name-only", "refs/mygitclient/follow/op1"]),
        "index"
    );
}

#[test]
fn locked_callback_runs_with_the_lock_held_and_its_failure_releases_it() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    let lock = r.lock_path();
    let saw = Cell::new(false);
    let p = prepare_index_with(&r.s(), "op1", &old, &new, &all(COLLEAGUE), &mut || {
        saw.set(lock.exists());
        Ok(())
    })
    .unwrap();
    assert!(saw.get(), "回调时锁已经建好");
    p.abandon();

    let err = prepare_index_with(&r.s(), "op2", &old, &new, &all(COLLEAGUE), &mut || {
        Err(app_lib::error::GitError::new("t_nope", "写不了记录"))
    })
    .err()
    .unwrap();
    assert_eq!(err.code, "t_nope");
    assert!(!lock.exists(), "记录写不了就不继续，锁也不留");
}

#[test]
fn abandon_and_drop_remove_lock_and_keep_pin() {
    for drop_only in [false, true] {
        let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
        r.stage("a.txt", b"mine\n");
        let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
        let before = r.index_bytes();

        let p = prepare_index(&r.s(), "op1", &old, &new, &all(ME_BEFORE)).unwrap();
        assert!(r.lock_path().exists(), "拼好之后还持着锁");
        // 持锁期间外部 git 写操作被挡回（W23）
        let (code, _) = run_git_code(&r.s(), &["add", "a.txt"]).unwrap();
        assert_ne!(code, 0, "外部 add 必须撞锁");

        if drop_only {
            drop(p);
        } else {
            p.abandon();
        }

        assert!(!r.lock_path().exists());
        assert_eq!(r.index_bytes(), before, "放弃就是什么都没发生");
        assert_eq!(r.git(&["rev-parse", "HEAD"]), old);
        assert!(!r
            .git(&["rev-parse", "refs/mygitclient/follow/op1"])
            .is_empty());
    }
}

#[test]
fn prepared_index_is_written_into_the_lock_not_the_real_index() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    let before = r.index_bytes();

    let mut p = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE)).unwrap();

    assert_eq!(r.index_bytes(), before, "rename 之前真实索引不变");
    assert_eq!(p.pinned_ref(), "refs/mygitclient/follow/op1");
    p.commit().unwrap();
    // 新索引的条目直接等于 NEW，分支这时还没动（编排里 CAS 在 commit 之前，这里只看索引）
    assert_eq!(r.entry("a.txt"), Some(r.at(&new, "a.txt")));
}

#[test]
fn second_commit_is_an_error_and_does_not_touch_someone_elses_lock() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    let mut p = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE)).unwrap();
    p.commit().unwrap();

    // 别的 git 进程这时拿了锁
    std::fs::write(r.lock_path(), b"someone else").unwrap();
    assert!(p.commit().is_err());
    drop(p);

    assert_eq!(
        std::fs::read(r.lock_path()).unwrap(),
        b"someone else",
        "commit 之后这个路径不再是我们的"
    );
    std::fs::remove_file(r.lock_path()).unwrap();
}

#[test]
fn busy_index_lock_is_left_alone() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    std::fs::write(r.lock_path(), b"held").unwrap();

    let err = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE))
        .err()
        .unwrap();

    assert_eq!(err.code, "svn_index_busy");
    assert_eq!(std::fs::read(r.lock_path()).unwrap(), b"held");
    std::fs::remove_file(r.lock_path()).unwrap();
}

#[test]
fn missing_index_file_is_refused() {
    // 0 字节的 index.lock 当索引读是 fatal，没有 .git/index 时不能走到那一步
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
    std::fs::remove_file(r.index_path()).unwrap();

    let err = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE))
        .err()
        .unwrap();

    assert_eq!(err.code, "svn_follow_index_missing");
    assert!(!r.lock_path().exists());
    assert!(!r.index_path().exists());
}

#[test]
fn failed_compose_leaves_no_lock_and_keeps_index() {
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    let before = r.index_bytes();

    assert!(prepare_index(&r.s(), "op1", &old, "not-a-commit", &all(COLLEAGUE)).is_err());

    assert!(!r.lock_path().exists());
    assert_eq!(r.index_bytes(), before);
}

#[test]
fn identical_old_and_new_changes_nothing() {
    // X 为空：不发空 stdin，索引内容不变
    let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
    r.stage("a.txt", b"mine\n");
    let staged = r.entry("a.txt");

    let report = r.publish(&old, &old, &all(COLLEAGUE));

    assert_eq!(report, ComposeReport::default());
    assert_eq!(r.entry("a.txt"), staged);
}

#[cfg(windows)]
mod windows_rename {
    //! W24：外部进程打开 `.git/index` 时没给 FILE_SHARE_DELETE，rename 报 os error 5 / 32。
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_SHARE_READ: u32 = 1;

    fn hold_index(r: &Repo) -> std::fs::File {
        std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(r.index_path())
            .unwrap()
    }

    #[test]
    fn commit_retries_until_the_holder_lets_go() {
        let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
        let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
        let mut p = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE)).unwrap();
        let holder = hold_index(&r);

        let t = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(holder);
        });
        p.commit().unwrap();
        t.join().unwrap();

        assert!(!r.lock_path().exists());
        assert_eq!(r.entry("a.txt"), Some(r.at(&new, "a.txt")));
    }

    #[test]
    fn commit_failure_keeps_the_lock_for_rollback() {
        // 编排要先把分支 CAS 回去，才能放开外部 git；所以失败时锁必须还在
        let (r, old) = Repo::seeded(&[("a.txt", b"a\n")]);
        let new = r.commit_on(&old, &[("a.txt", Some(b"a2\n"))]);
        let before = r.index_bytes();
        let mut p = prepare_index(&r.s(), "op1", &old, &new, &all(COLLEAGUE)).unwrap();
        let holder = hold_index(&r);

        let started = std::time::Instant::now();
        let err = p.commit().err().unwrap();
        let waited = started.elapsed();
        drop(holder);

        assert_eq!(err.code, "svn_follow_index_rename_failed");
        assert!(
            waited >= std::time::Duration::from_millis(1200),
            "{waited:?}"
        );
        assert!(r.lock_path().exists(), "失败时还持着锁");
        assert_eq!(r.index_bytes(), before);

        p.abandon();
        assert!(!r.lock_path().exists());
    }
}
