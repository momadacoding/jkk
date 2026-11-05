//! `read_diff` 的单文件 scope：`staged:<路径>` / `unstaged:<路径>`（2026-11-05 排查报告 6.13 方案 C）。
//!
//! 「本地改动」右侧只显示选中的那一个文件，原来却按整个区读（`unstaged` 要把整个工作区 stat 一遍、
//! 再读出每个改动文件的内容）。契约：**单文件 scope 读出来的，恰好是整区读出来的结果里这个路径的那一项**
//! ——hunk、行号、二进制标记、冲突文件补上的行都算在内。

use app_lib::engine;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success()
}

fn run_git(dir: &Path, args: &[&str]) {
    assert!(git(dir, args), "git {args:?} 失败");
}

fn init_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    run_git(d, &["init", "-q", "-b", "main"]);
    run_git(d, &["config", "user.email", "t@t.com"]);
    run_git(d, &["config", "user.name", "T"]);
    run_git(d, &["config", "core.autocrlf", "false"]);
    dir
}

fn write(d: &Path, rel: &str, body: &str) {
    let p = d.join(rel);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(p, body).unwrap();
}

/// 整区读出来的每一项，按路径取出。
fn section(d: &Path, scope: &str) -> Vec<serde_json::Value> {
    let data = engine::diff::read_diff(d.to_str().unwrap(), scope.to_string()).unwrap();
    serde_json::to_value(&data.files)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn file_scope(d: &Path, section: &str, path: &str) -> Vec<serde_json::Value> {
    self::section(d, &format!("{section}:{path}"))
}

/// 两个区里出现的每一个路径都核对一遍，再加一个干净的和一个不存在的。
fn assert_contract_everywhere(d: &Path, clean: &str) {
    for sec in ["staged", "unstaged"] {
        let whole = section(d, sec);
        let listed: BTreeSet<String> = whole
            .iter()
            .map(|f| f["path"].as_str().unwrap().to_string())
            .collect();
        for p in listed
            .iter()
            .map(String::as_str)
            .chain([clean, "no/such.txt"])
        {
            let wanted: Vec<_> = whole.iter().filter(|f| f["path"] == p).cloned().collect();
            assert_eq!(file_scope(d, sec, p), wanted, "{sec}:{p}");
        }
    }
}

#[test]
fn a_single_file_scope_is_exactly_that_file_of_the_whole_section() {
    let dir = init_repo();
    let d = dir.path();
    write(d, ".gitignore", "*.log\n");
    write(d, "a.txt", "1\n2\n3\n");
    write(d, "b.txt", "b1\nb2\n");
    write(d, "gone.txt", "g\n");
    write(d, "mv-src.txt", "m\n");
    write(d, "bin.dat", "x");
    write(d, "clean.txt", "c\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);

    write(d, "a.txt", "x\n2\n3\n");
    run_git(d, &["add", "a.txt"]);
    write(d, "a.txt", "x\n2\ny\n");
    fs::remove_file(d.join("b.txt")).unwrap();
    run_git(d, &["rm", "-q", "gone.txt"]);
    run_git(d, &["mv", "mv-src.txt", "mv-dst.txt"]);
    fs::write(d.join("bin.dat"), [0u8, 1, 2, 0, 3]).unwrap();
    write(d, "nested/deep/u.txt", "u1\nu2\n");
    write(d, "中 文.txt", "z\n");
    write(d, "f[1].txt", "bracket\n");
    write(d, "f1.txt", "plain\n");
    write(d, "x.log", "ignored\n");

    assert_contract_everywhere(d, "clean.txt");
    // 字面量匹配：`f[1].txt` 不把 `f1.txt` 一起读进来
    let paths: Vec<_> = file_scope(d, "unstaged", "f[1].txt")
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, ["f[1].txt"]);
}

/// 冲突文件：Unstaged 里要带上 HEAD 对比工作区补出来的行（`fill_conflicted`），Staged 里不出现。
#[test]
fn a_conflicted_file_keeps_its_filled_in_lines() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "both.txt", "base\n");
    write(d, "clean.txt", "c\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    run_git(d, &["checkout", "-q", "-b", "feat"]);
    write(d, "both.txt", "theirs\n");
    write(d, "feat-only.txt", "f\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "feat"]);
    run_git(d, &["checkout", "-q", "main"]);
    write(d, "both.txt", "ours\n");
    run_git(d, &["commit", "-qam", "ours"]);
    assert!(!git(d, &["merge", "-q", "feat"]), "merge 应当冲突");

    let one = file_scope(d, "unstaged", "both.txt");
    assert_eq!(one.len(), 1);
    assert_eq!(one[0]["status"], "conflicted");
    assert!(
        !one[0]["hunks"].as_array().unwrap().is_empty(),
        "冲突文件要补上行"
    );
    assert!(file_scope(d, "staged", "both.txt").is_empty());
    assert_contract_everywhere(d, "clean.txt");
}

/// 读不了的路径（悬空的目录联接）：单读它不报错，也不出现；别的文件照常。
#[test]
fn an_unreadable_path_reads_as_empty() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "1\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "a.txt", "1\n2\n");
    if !make_dangling_link(&d.join(".broken")) {
        eprintln!("跳过：本机无法创建目录联接");
        return;
    }
    assert!(file_scope(d, "unstaged", ".broken").is_empty());
    assert_contract_everywhere(d, "no-such-clean.txt");
}

/// 字面量 pathspec 在 libgit2 里仍按目录前缀匹配：已跟踪的文件 `assets` 换成了未跟踪的同名目录，
/// 选中 `assets` 那一项时，不能把目录下每个文件的内容都读出来（第二轮独立 review 第 6 条）。
/// 结果只能有 `assets` 自己。
#[test]
fn a_file_replaced_by_a_directory_reads_only_that_file() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "assets", "file\n");
    write(d, "clean.txt", "c\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    fs::remove_file(d.join("assets")).unwrap();
    for i in 0..20 {
        write(d, &format!("assets/f{i}.txt"), "x\n");
    }

    let paths: Vec<_> = file_scope(d, "unstaged", "assets")
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, ["assets"]);
    assert_contract_everywhere(d, "clean.txt");
}

/// 空路径绝不能退化成「整个区」（libgit2 把空 pathspec 当全部，AGENTS.md §6.2）：要报错。
/// 退化成整区的话，这条快路径就悄悄变回了它要替换掉的那次全量读。
#[test]
fn an_empty_path_is_rejected() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "1\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "a.txt", "1\n2\n");
    for scope in ["staged:", "unstaged:"] {
        let err = engine::diff::read_diff(d.to_str().unwrap(), scope.to_string()).expect_err(scope);
        assert_eq!(err.code, "bad_diff_scope", "{scope}");
    }
}

#[cfg(windows)]
fn make_dangling_link(link: &Path) -> bool {
    let target = link.with_file_name("junction-target");
    if fs::create_dir_all(&target).is_err() {
        return false;
    }
    let ok = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(&target)
        .stdout(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok && fs::remove_dir_all(&target).is_ok()
}

#[cfg(not(windows))]
fn make_dangling_link(link: &Path) -> bool {
    std::os::unix::fs::symlink(link.with_file_name("nowhere-at-all"), link).is_ok()
}
