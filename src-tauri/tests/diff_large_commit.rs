#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 超大提交的 diff（第 73 条）。
//!
//! 时间几乎全花在读 blob 上（5 万个文件约 22 秒，见 `2026-11-23-large-commit-diff-decisions.md`），
//! 所以文件超过 `SUMMARY_FILE_LIMIT` 个时 `read_diff` 只给列表、不读内容，
//! 点到哪个文件再由 `read_diff_file` 读那一个。
//!
//! fixture 用 `git fast-import` 造：一千多个文件逐个写盘再 `git add` 在 Windows 上要几十秒，
//! fast-import 直接写一个 pack，一两秒就好，对象也和真实仓库一样是打包的。

use app_lib::engine::diff::{
    read_diff_file, read_diff_with, DiffData, DiffViewOptions, SUMMARY_FILE_LIMIT,
};
use app_lib::engine::file_history;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success();
    assert!(ok, "git {args:?} 失败");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(out.status.success(), "git {args:?} 失败");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn init_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    run_git(d, &["init", "-q", "-b", "main"]);
    run_git(d, &["config", "user.email", "t@t"]);
    run_git(d, &["config", "user.name", "t"]);
    run_git(d, &["config", "core.autocrlf", "false"]);
    dir
}

/// 拼 `git fast-import` 的输入流。
#[derive(Default)]
struct Stream {
    buf: Vec<u8>,
    mark: usize,
}

impl Stream {
    fn blob(&mut self, content: &str) -> usize {
        self.mark += 1;
        write!(
            self.buf,
            "blob\nmark :{}\ndata {}\n{content}\n",
            self.mark,
            content.len()
        )
        .unwrap();
        self.mark
    }

    /// `ops` 是 fast-import 的文件命令（`M 100644 :1 a.txt`、`D dir` 等）。
    fn commit(&mut self, msg: &str, ops: &[String]) {
        self.mark += 1;
        write!(
            self.buf,
            "commit refs/heads/main\nmark :{}\ncommitter t <t@t> 1700000000 +0000\ndata {}\n{msg}\n",
            self.mark,
            msg.len()
        )
        .unwrap();
        for op in ops {
            writeln!(self.buf, "{op}").unwrap();
        }
        self.buf.push(b'\n');
    }

    fn import(self, dir: &Path) {
        let mut child = Command::new("git")
            .current_dir(dir)
            .args(["fast-import", "--quiet"])
            .stdin(Stdio::piped())
            .spawn()
            .expect("git not found");
        child.stdin.take().unwrap().write_all(&self.buf).unwrap();
        assert!(child.wait().unwrap().success(), "fast-import 失败");
    }
}

/// `n` 个内容各不相同的文件，放在 `dir/F00000.txt` 这样的路径下。返回 (mark, 路径) 列表。
fn files(s: &mut Stream, dir: &str, n: usize) -> Vec<(usize, String)> {
    (0..n)
        .map(|i| {
            let mark = s.blob(&format!("line {i}\nsecond {i}"));
            (mark, format!("{dir}/F{i:05}.txt"))
        })
        .collect()
}

fn add_ops(list: &[(usize, String)]) -> Vec<String> {
    list.iter()
        .map(|(m, p)| format!("M 100644 :{m} {p}"))
        .collect()
}

/// 只有一个根提交，里面 `n` 个新增文件。返回仓库和提交 id。
fn root_with(n: usize) -> (TempDir, String) {
    let dir = init_repo();
    let mut s = Stream::default();
    let list = files(&mut s, "big", n);
    s.commit("root", &add_ops(&list));
    s.import(dir.path());
    let head = git_out(dir.path(), &["rev-parse", "main"]);
    (dir, head)
}

fn p(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

fn read(dir: &Path, scope: &str) -> DiffData {
    read_diff_with(p(dir), scope.to_string(), &DiffViewOptions::default()).unwrap()
}

#[test]
fn a_commit_over_the_limit_lists_files_without_reading_contents() {
    let (dir, head) = root_with(SUMMARY_FILE_LIMIT + 1);
    let data = read(dir.path(), &format!("commit:{head}"));
    assert!(data.summary_only);
    assert_eq!(data.files.len(), SUMMARY_FILE_LIMIT + 1);
    assert!(data.files.iter().all(|f| f.hunks.is_empty()));
    assert!(data.files.iter().all(|f| f.status == "added"));
    assert_eq!(data.files[7].path, "big/F00007.txt");
    // 只给列表时不涉及上下文，「按默认上下文读了」的说明不该出现
    assert!(!data.context_narrowed);
}

#[test]
fn a_commit_at_the_limit_is_read_in_full_as_before() {
    let (dir, head) = root_with(SUMMARY_FILE_LIMIT);
    let data = read(dir.path(), &format!("commit:{head}"));
    assert!(!data.summary_only);
    assert_eq!(data.files.len(), SUMMARY_FILE_LIMIT);
    assert!(data.files.iter().all(|f| !f.hunks.is_empty()));
}

#[test]
fn summary_only_is_not_serialized_for_a_normal_commit() {
    let (dir, head) = root_with(3);
    let json = serde_json::to_string(&read(dir.path(), &format!("commit:{head}"))).unwrap();
    assert!(!json.contains("summaryOnly"), "{json}");
    let (dir, head) = root_with(SUMMARY_FILE_LIMIT + 1);
    let json = serde_json::to_string(&read(dir.path(), &format!("commit:{head}"))).unwrap();
    assert!(json.contains("\"summaryOnly\":true"));
}

#[test]
fn reading_one_file_of_a_large_commit_matches_the_single_file_history_read() {
    let (dir, head) = root_with(SUMMARY_FILE_LIMIT + 1);
    let scope = format!("commit:{head}");
    let file = "big/F00007.txt";
    for view in [
        DiffViewOptions::default(),
        DiffViewOptions {
            context_lines: Some(1),
            ignore_whitespace: true,
        },
    ] {
        let one = read_diff_file(p(dir.path()), &scope, file, None, &view).unwrap();
        let reference =
            file_history::commit_file_diff_with(p(dir.path()), &head, file, None, &view).unwrap();
        assert_eq!(one.files.len(), 1);
        assert_eq!(one.files[0].path, file);
        assert_eq!(one.files[0].status, "added");
        assert_eq!(
            format!("{:?}", one.files[0].hunks),
            format!("{:?}", reference.files[0].hunks)
        );
        assert!(!one.summary_only);
    }
}

/// 两次提交：第一次 `n` 个文件加一个空文件 `E.txt` 放在 `a/`，第二次整体挪到 `b/`，并改了 `b/F00003.txt`。
fn moved(n: usize) -> (TempDir, String, String) {
    let dir = init_repo();
    let mut s = Stream::default();
    let mut list = files(&mut s, "a", n);
    let empty = s.blob("");
    list.push((empty, "a/E.txt".to_string()));
    s.commit("first", &add_ops(&list));
    let changed = s.blob("line 3\nchanged");
    let mut ops = vec!["D a".to_string()];
    for (m, path) in &list {
        let to = path.replacen("a/", "b/", 1);
        let m = if to == "b/F00003.txt" { changed } else { *m };
        ops.push(format!("M 100644 :{m} {to}"));
    }
    s.commit("move", &ops);
    s.import(dir.path());
    let first = git_out(dir.path(), &["rev-parse", "main~1"]);
    let second = git_out(dir.path(), &["rev-parse", "main"]);
    (dir, first, second)
}

#[test]
fn a_large_move_still_pairs_exact_renames_without_reading_contents() {
    // 改名前后共 2·(LIMIT/2+1) 个 delta，超过上限；配对后剩 LIMIT/2+1 条
    let n = SUMMARY_FILE_LIMIT / 2 + 1;
    let (dir, first, second) = moved(n);
    for scope in [
        format!("commit:{second}"),
        format!("range:{first}..{second}"),
    ] {
        let data = read(dir.path(), &scope);
        assert!(data.summary_only, "{scope}");
        let f = data
            .files
            .iter()
            .find(|f| f.path == "b/F00005.txt")
            .unwrap();
        assert_eq!(f.status, "renamed");
        assert_eq!(f.old_path.as_deref(), Some("a/F00005.txt"));
        // 内容也改了的那个不读内容就配不成对，摊成一删一增
        let added = data
            .files
            .iter()
            .find(|f| f.path == "b/F00003.txt")
            .unwrap();
        assert_eq!(added.status, "added");
        assert!(data
            .files
            .iter()
            .any(|f| f.path == "a/F00003.txt" && f.status == "deleted"));
        // 空文件内容都一样，配成哪一对都是猜，也摊成一删一增
        for (path, status) in [("b/E.txt", "added"), ("a/E.txt", "deleted")] {
            assert!(
                data.files
                    .iter()
                    .any(|f| f.path == path && f.status == status),
                "{path}"
            );
        }
        // n-1 条改名，加上两对一删一增
        assert_eq!(data.files.len(), n + 3);
    }
}

#[test]
fn diff_renames_false_turns_off_pairing_for_large_commits_too() {
    // 没超上限时 `find_similar(None)` 按 `diff.renames` 走；超了也要一致，不能小提交摊开、大提交配上
    let n = SUMMARY_FILE_LIMIT / 2 + 1;
    let (dir, _first, second) = moved(n);
    run_git(dir.path(), &["config", "diff.renames", "false"]);
    let data = read(dir.path(), &format!("commit:{second}"));
    assert!(data.summary_only);
    assert!(data.files.iter().all(|f| f.status != "renamed"));
    assert_eq!(data.files.len(), 2 * (n + 1));
}

#[test]
fn reading_one_renamed_file_with_its_old_path_keeps_the_rename() {
    let n = SUMMARY_FILE_LIMIT / 2 + 1;
    let (dir, first, second) = moved(n);
    for scope in [
        format!("commit:{second}"),
        format!("range:{first}..{second}"),
    ] {
        let one = read_diff_file(
            p(dir.path()),
            &scope,
            "b/F00005.txt",
            Some("a/F00005.txt"),
            &DiffViewOptions::default(),
        )
        .unwrap();
        assert_eq!(one.files.len(), 1, "{scope}");
        assert_eq!(one.files[0].status, "renamed");
        assert_eq!(one.files[0].old_path.as_deref(), Some("a/F00005.txt"));
        assert!(one.files[0].hunks.is_empty());

        // 改了内容的那个在列表里是新增，单独读也按新增读，与列表一致
        let one = read_diff_file(
            p(dir.path()),
            &scope,
            "b/F00003.txt",
            None,
            &DiffViewOptions::default(),
        )
        .unwrap();
        assert_eq!(one.files[0].status, "added");
        assert!(!one.files[0].hunks.is_empty());
    }
}

#[test]
fn reading_one_file_is_an_exact_match_not_a_directory_prefix() {
    let (dir, head) = root_with(SUMMARY_FILE_LIMIT + 1);
    // 字面量 pathspec 在 libgit2 里按目录前缀匹配：点名 `big` 会带出目录下所有文件
    let one = read_diff_file(
        p(dir.path()),
        &format!("commit:{head}"),
        "big",
        None,
        &DiffViewOptions::default(),
    )
    .unwrap();
    assert!(one.files.is_empty());
}

#[test]
fn reading_one_file_rejects_an_empty_path_and_non_commit_scopes() {
    let (dir, head) = root_with(2);
    // 空 pathspec 在 libgit2 里是「全部」：不拦的话就成了读整个提交
    let e = read_diff_file(
        p(dir.path()),
        &format!("commit:{head}"),
        "",
        None,
        &DiffViewOptions::default(),
    )
    .unwrap_err();
    assert_eq!(e.code, "bad_diff_scope");
    // 说的是没给文件，不能说 scope 解析不了：scope 本身是好的
    assert!(e.message.contains("文件"), "{}", e.message);
    for scope in [
        "workdir",
        "staged",
        "unstaged:big/F00000.txt",
        "range:abc",
        "",
    ] {
        let e = read_diff_file(
            p(dir.path()),
            scope,
            "big/F00000.txt",
            None,
            &DiffViewOptions::default(),
        )
        .unwrap_err();
        assert_eq!(e.code, "bad_diff_scope", "{scope}");
    }
}

#[test]
fn an_empty_old_path_is_treated_as_none() {
    let (dir, head) = root_with(SUMMARY_FILE_LIMIT + 1);
    let one = read_diff_file(
        p(dir.path()),
        &format!("commit:{head}"),
        "big/F00001.txt",
        Some(""),
        &DiffViewOptions::default(),
    )
    .unwrap();
    assert_eq!(one.files.len(), 1);
}

/// 5 万文件的验收目标值（第 73 条）：选中提交到列表出来、点开一个文件，各自 1 秒以内。
/// `cargo test --release --test diff_large_commit -- --ignored --nocapture perf_50k`
#[test]
#[ignore]
fn perf_50k() {
    let (dir, head) = root_with(50_000);
    let scope = format!("commit:{head}");
    let t = Instant::now();
    let data = read(dir.path(), &scope);
    let list = t.elapsed();
    let json = serde_json::to_string(&data).unwrap();
    let t = Instant::now();
    let one = read_diff_file(
        p(dir.path()),
        &scope,
        "big/F49999.txt",
        None,
        &DiffViewOptions::default(),
    )
    .unwrap();
    let file = t.elapsed();
    assert!(data.summary_only);
    assert_eq!(data.files.len(), 50_000);
    assert_eq!(one.files.len(), 1);
    eprintln!(
        "5 万新增：列表 {list:?}（{} 字节），单个文件 {file:?}",
        json.len()
    );

    let (dir, first, second) = moved(50_000);
    let t = Instant::now();
    let data = read(dir.path(), &format!("range:{first}..{second}"));
    let mv = t.elapsed();
    eprintln!("5 万挪目录：列表 {mv:?}（{} 项）", data.files.len());
    assert_eq!(data.files.len(), 50_003);

    assert!(list.as_secs_f64() < 1.0, "列表 {list:?}");
    assert!(file.as_secs_f64() < 1.0, "单个文件 {file:?}");
    assert!(mv.as_secs_f64() < 1.0, "挪目录 {mv:?}");
}
