#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! diff 面板的开关（`DiffViewOptions`）：上下文行数、整个文件、忽略空白。
//!
//! 「忽略换行符」在前端做（`src/engine/diffEol.ts`），它依赖 libgit2 把行尾的 `\r` 留在行内容里，
//! 这条假设也在这里用真实 CRLF 文件钉住。

use app_lib::engine;
use app_lib::engine::diff::{
    DiffData, DiffViewOptions, FULL_FILE_CONTEXT, WIDE_CONTEXT_FILE_LIMIT,
};
use app_lib::engine::file_history;
use std::fs;
use std::path::Path;
use std::process::Command;
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
    run_git(d, &["config", "user.email", "t@t.com"]);
    run_git(d, &["config", "user.name", "T"]);
    // 不让 git 改写换行符：CRLF 的用例要的就是字节原样进仓库
    run_git(d, &["config", "core.autocrlf", "false"]);
    dir
}

/// 1..=20 每行一个数字，`changed` 那一行换成别的内容。
fn numbered(changed: Option<usize>) -> String {
    (1..=20)
        .map(|i| {
            if Some(i) == changed {
                format!("changed {i}\n")
            } else {
                format!("line {i}\n")
            }
        })
        .collect()
}

fn commit_all(d: &Path, msg: &str) -> String {
    run_git(d, &["add", "-A"]);
    run_git(d, &["commit", "-q", "-m", msg]);
    git_out(d, &["rev-parse", "HEAD"])
}

fn read(d: &Path, scope: &str, view: DiffViewOptions) -> DiffData {
    engine::diff::read_diff_with(d.to_str().unwrap(), scope.to_string(), &view).unwrap()
}

fn ctx(n: u32) -> DiffViewOptions {
    DiffViewOptions {
        context_lines: Some(n),
        ..Default::default()
    }
}

fn ws() -> DiffViewOptions {
    DiffViewOptions {
        ignore_whitespace: true,
        ..Default::default()
    }
}

/// 第 10 行改动的仓库：一个提交（改动已提交）+ 工作区再改第 10 行 + 暂存同样的改动。
struct Repo {
    dir: TempDir,
    base: String,
    head: String,
}

fn changed_line_repo() -> Repo {
    let dir = init_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), numbered(None)).unwrap();
    let base = commit_all(d, "base");
    fs::write(d.join("a.txt"), numbered(Some(10))).unwrap();
    let head = commit_all(d, "change line 10");
    // Staged 和 Unstaged 各有一份同样形状的改动：第 5 行进暂存区，第 15 行留在工作区
    let mut staged = numbered(Some(10)).replace("line 5\n", "staged 5\n");
    fs::write(d.join("a.txt"), &staged).unwrap();
    run_git(d, &["add", "a.txt"]);
    staged = staged.replace("line 15\n", "unstaged 15\n");
    fs::write(d.join("a.txt"), staged).unwrap();
    Repo { dir, base, head }
}

/// 每个会读 diff 的 scope 都要吃到选项：只改了一处的那几个 scope 下，单个 hunk 的总行数。
fn single_hunk_len(r: &Repo, scope: &str, view: DiffViewOptions) -> usize {
    let data = read(r.dir.path(), scope, view);
    assert_eq!(data.files.len(), 1, "{scope}");
    assert_eq!(data.files[0].hunks.len(), 1, "{scope}");
    data.files[0].hunks[0].lines.len()
}

fn every_scope(r: &Repo) -> Vec<String> {
    vec![
        format!("commit:{}", r.head),
        format!("range:{}..{}", r.base, r.head),
        "staged".into(),
        "staged:a.txt".into(),
        "unstaged".into(),
        "unstaged:a.txt".into(),
    ]
}

#[test]
fn default_context_is_three_lines_on_every_scope() {
    let r = changed_line_repo();
    for scope in every_scope(&r) {
        // 3 行上下文 + 一删一增 + 3 行上下文
        assert_eq!(
            single_hunk_len(&r, &scope, DiffViewOptions::default()),
            8,
            "{scope}"
        );
    }
}

#[test]
fn context_lines_reach_every_scope() {
    let r = changed_line_repo();
    for scope in every_scope(&r) {
        assert_eq!(single_hunk_len(&r, &scope, ctx(1)), 4, "{scope}");
        assert_eq!(single_hunk_len(&r, &scope, ctx(0)), 2, "{scope}");
    }
}

#[test]
fn full_file_context_shows_every_line_from_the_first() {
    let r = changed_line_repo();
    for scope in every_scope(&r) {
        let data = read(r.dir.path(), &scope, ctx(FULL_FILE_CONTEXT));
        let hunk = &data.files[0].hunks[0];
        // 20 行原文，改动那一行多出一条删除
        assert_eq!(hunk.lines.len(), 21, "{scope}");
        assert_eq!((hunk.old_start, hunk.new_start), (1, 1), "{scope}");
    }
}

#[test]
fn context_beyond_the_limit_is_clamped_instead_of_wrapping_around() {
    // u32::MAX 直接给 xdiff 的话在 Windows 上会变成 -1（`long` 是 32 位）
    let r = changed_line_repo();
    let scope = format!("commit:{}", r.head);
    assert_eq!(single_hunk_len(&r, &scope, ctx(u32::MAX)), 21);
}

/// 只改缩进和行内空格的仓库，提交了、也在工作区与暂存区各有一份。
fn whitespace_repo() -> Repo {
    let dir = init_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "fn main() {\n    call(a, b);\n}\n").unwrap();
    let base = commit_all(d, "base");
    fs::write(d.join("a.txt"), "fn main() {\n\tcall(a,  b);\n}\n").unwrap();
    let head = commit_all(d, "reindent");
    fs::write(d.join("a.txt"), "fn  main() {\n\tcall(a,  b);\n}\n").unwrap();
    run_git(d, &["add", "a.txt"]);
    fs::write(d.join("a.txt"), "fn  main() {\n\tcall(a,  b); \n}\n").unwrap();
    Repo { dir, base, head }
}

#[test]
fn ignore_whitespace_hides_whitespace_only_changes_on_every_scope() {
    let r = whitespace_repo();
    for scope in every_scope(&r) {
        let plain = read(r.dir.path(), &scope, DiffViewOptions::default());
        assert_eq!(plain.files[0].hunks.len(), 1, "{scope}：不忽略时应当有改动");
        let data = read(r.dir.path(), &scope, ws());
        // 文件仍在列表里（文件树要它），只是没有可显示的行
        assert_eq!(data.files.len(), 1, "{scope}");
        assert!(data.files[0].hunks.is_empty(), "{scope}");
    }
}

#[test]
fn ignore_whitespace_keeps_real_changes() {
    let dir = init_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "a\n    b\nc\n").unwrap();
    commit_all(d, "base");
    fs::write(d.join("a.txt"), "a\n\tb\nC\n").unwrap();
    let data = read(d, "unstaged", ws());
    let lines: Vec<(char, String)> = data.files[0].hunks[0]
        .lines
        .iter()
        .map(|l| (l.origin, l.content.clone()))
        .collect();
    // 缩进那一行成了上下文，真改的那一行照样是一删一增
    assert!(lines.contains(&('-', "c".into())), "{lines:?}");
    assert!(lines.contains(&('+', "C".into())), "{lines:?}");
    assert!(
        !lines.iter().any(|(o, c)| *o != ' ' && c.contains('b')),
        "{lines:?}"
    );
}

#[test]
fn commit_file_diff_takes_the_options_too() {
    let r = changed_line_repo();
    let p = r.dir.path().to_str().unwrap();
    let data = file_history::commit_file_diff_with(p, &r.head, "a.txt", None, &ctx(0)).unwrap();
    assert_eq!(data.files[0].hunks[0].lines.len(), 2);

    let w = whitespace_repo();
    let p = w.dir.path().to_str().unwrap();
    let data = file_history::commit_file_diff_with(p, &w.head, "a.txt", None, &ws()).unwrap();
    assert!(data.files[0].hunks.is_empty());
}

#[test]
fn options_deserialize_from_what_the_frontend_sends() {
    // 前端 `backendDiffOptions` 的形状；缺字段一律按默认
    let full: DiffViewOptions =
        serde_json::from_str(r#"{"contextLines":5,"ignoreWhitespace":true}"#).unwrap();
    assert_eq!(full.context_lines, Some(5));
    assert!(full.ignore_whitespace);
    let empty: DiffViewOptions = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, DiffViewOptions::default());
}

/// 前端「忽略换行符」的前提：LF 改成 CRLF 后，libgit2 给的行内容里留着 `\r`，两边能区分开。
#[test]
fn crlf_changes_keep_the_carriage_return_in_line_content() {
    let dir = init_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "one\ntwo\n").unwrap();
    commit_all(d, "lf");
    fs::write(d.join("a.txt"), "one\r\ntwo\r\n").unwrap();
    let data = read(d, "unstaged", DiffViewOptions::default());
    let lines: Vec<(char, String)> = data.files[0].hunks[0]
        .lines
        .iter()
        .map(|l| (l.origin, l.content.clone()))
        .collect();
    assert_eq!(
        lines,
        vec![
            ('-', "one".into()),
            ('-', "two".into()),
            ('+', "one\r".into()),
            ('+', "two\r".into()),
        ]
    );
}

/// 前端「忽略换行符」不藏的那一种：文件末尾补上换行，libgit2 给的一删一增行内容完全相同
/// （`\n` 被去掉、`\ No newline` 标记行被滤掉），前端靠「原样相同」把它和 CRLF 区分开。
#[test]
fn adding_the_final_newline_gives_identical_line_content() {
    let dir = init_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "one\ntwo").unwrap();
    commit_all(d, "no final newline");
    fs::write(d.join("a.txt"), "one\ntwo\n").unwrap();
    let data = read(d, "unstaged", DiffViewOptions::default());
    let lines: Vec<(char, String)> = data.files[0].hunks[0]
        .lines
        .iter()
        .map(|l| (l.origin, l.content.clone()))
        .collect();
    assert_eq!(
        lines,
        vec![
            (' ', "one".into()),
            ('-', "two".into()),
            ('+', "two".into()),
        ]
    );
}

/// 一个提交改 `n` 个文件，每个文件都是 20 行里改了第 10 行。
fn many_files_commit(n: usize) -> (TempDir, String) {
    let dir = init_repo();
    let d = dir.path();
    for i in 0..n {
        fs::write(d.join(format!("f{i}.txt")), numbered(None)).unwrap();
    }
    commit_all(d, "base");
    for i in 0..n {
        fs::write(d.join(format!("f{i}.txt")), numbered(Some(10))).unwrap();
    }
    let head = commit_all(d, "change all");
    (dir, head)
}

#[test]
fn wide_context_falls_back_to_default_when_a_commit_has_too_many_files() {
    let (dir, head) = many_files_commit(WIDE_CONTEXT_FILE_LIMIT + 1);
    let scope = format!("commit:{head}");
    let data = read(dir.path(), &scope, ctx(FULL_FILE_CONTEXT));
    assert!(data.context_narrowed);
    assert_eq!(data.files.len(), WIDE_CONTEXT_FILE_LIMIT + 1);
    // 按默认 3 行读：3 + 一删一增 + 3
    assert!(data.files.iter().all(|f| f.hunks[0].lines.len() == 8));
    // 告诉前端的字段只在为真时出现
    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["contextNarrowed"], true);
    // 默认上下文时文件再多也不算「退回」
    let data = read(dir.path(), &scope, DiffViewOptions::default());
    assert!(!data.context_narrowed);
}

#[test]
fn wide_context_applies_up_to_the_limit() {
    let (dir, head) = many_files_commit(WIDE_CONTEXT_FILE_LIMIT);
    let data = read(
        dir.path(),
        &format!("commit:{head}"),
        ctx(FULL_FILE_CONTEXT),
    );
    assert!(!data.context_narrowed);
    assert!(data.files.iter().all(|f| f.hunks[0].lines.len() == 21));
    let json = serde_json::to_value(&data).unwrap();
    assert!(json.get("contextNarrowed").is_none());
}

#[test]
fn single_file_scopes_are_never_narrowed() {
    // 「本地改动」只读选中的那一个文件，不管工作区里改了多少个
    let (dir, _) = many_files_commit(WIDE_CONTEXT_FILE_LIMIT + 1);
    let d = dir.path();
    for i in 0..=WIDE_CONTEXT_FILE_LIMIT {
        fs::write(d.join(format!("f{i}.txt")), numbered(Some(3))).unwrap();
    }
    let data = read(d, "unstaged:f0.txt", ctx(FULL_FILE_CONTEXT));
    assert!(!data.context_narrowed);
    assert_eq!(data.files[0].hunks[0].lines.len(), 22);
    let whole = read(d, "unstaged", ctx(FULL_FILE_CONTEXT));
    assert!(whole.context_narrowed, "整区读超过上限时同样退回");
}
