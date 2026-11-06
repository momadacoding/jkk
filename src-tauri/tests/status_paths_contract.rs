#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! `read_status_paths`（Stage / Unstage 之后只重读这几个路径）的契约。
//!
//! 契约只有一条：**Staged 与全量读的完全相同，Unstaged 恰好是全量读里这几个路径的那几项**
//! （内容、顺序都相同）。前端拿它就地替换列表里这几个路径的项，所以只要这条成立，
//! 合并后的列表就和全量重读一致（「别的路径受连带影响」由前端另行兜底，见 `statusPatch.ts`）。
//!
//! 场景沿用 `status_contract.rs`：每个场景对「列表里出现的每一个路径」逐个核对，
//! 再核对一次全部路径、一个干净的已跟踪文件、一个不存在的路径。

use app_lib::engine;
use app_lib::engine::diff::{FileChange, StatusData};
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

fn line(f: &FileChange) -> String {
    format!("{} {} +{} -{}", f.status, f.path, f.additions, f.deletions)
}

fn lines(files: &[FileChange]) -> Vec<String> {
    files.iter().map(line).collect()
}

fn full(d: &Path) -> StatusData {
    engine::diff::read_status(d.to_str().unwrap()).unwrap()
}

/// 对一组路径核对契约。
fn assert_contract(d: &Path, paths: &[&str]) {
    let all = full(d);
    let owned: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
    let patch = engine::diff::read_status_paths(d.to_str().unwrap(), &owned).unwrap();
    assert_eq!(
        lines(&patch.staged),
        lines(&all.staged),
        "Staged 应与全量相同：{paths:?}"
    );
    let wanted: Vec<String> = all
        .unstaged
        .iter()
        .filter(|f| paths.contains(&f.path.as_str()))
        .map(line)
        .collect();
    assert_eq!(
        lines(&patch.unstaged),
        wanted,
        "Unstaged 应只有这几个路径：{paths:?}"
    );
}

/// 列表里出现的每一个路径逐个核对，再加全部、干净的、不存在的。
fn assert_contract_everywhere(d: &Path, clean: &str) {
    let all = full(d);
    let listed: BTreeSet<String> = all
        .staged
        .iter()
        .chain(all.unstaged.iter())
        .map(|f| f.path.clone())
        .collect();
    assert!(!listed.is_empty(), "测试前提：场景里要有改动");
    for p in &listed {
        assert_contract(d, &[p.as_str()]);
    }
    let every: Vec<&str> = listed.iter().map(String::as_str).collect();
    assert_contract(d, &every);
    assert_contract(d, &[clean]);
    assert_contract(d, &["no/such/path.txt"]);
}

#[test]
fn mixed_changes_match_the_full_listing_path_by_path() {
    let dir = init_repo();
    let d = dir.path();
    write(d, ".gitignore", "*.log\n");
    write(d, "a.txt", "1\n2\n3\n");
    write(d, "b.txt", "b1\nb2\n");
    write(d, "gone.txt", "g\n");
    write(d, "mv-src.txt", "m\n");
    write(d, "dir/keep.txt", "k\n");
    write(d, "dir/drop.txt", "d1\nd2\nd3\n");
    write(d, "clean.txt", "c\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);

    write(d, "a.txt", "x\n2\n3\n");
    run_git(d, &["add", "a.txt"]);
    write(d, "a.txt", "x\n2\ny\n");
    fs::remove_file(d.join("b.txt")).unwrap();
    run_git(d, &["rm", "-q", "gone.txt"]);
    write(d, "new.txt", "n\n");
    run_git(d, &["add", "new.txt"]);
    fs::remove_file(d.join("new.txt")).unwrap();
    run_git(d, &["mv", "mv-src.txt", "mv-dst.txt"]);
    fs::remove_dir_all(d.join("dir")).unwrap();
    write(d, "top.txt", "t\n");
    write(d, "nested/deep/u.txt", "u1\nu2\n");
    write(d, "中 文.txt", "z\n");
    write(d, "x.log", "ignored\n");
    fs::create_dir_all(d.join("emptydir")).unwrap();

    assert_contract_everywhere(d, "clean.txt");
    // 被忽略的文件点名要也不出现：全量读里没有它
    assert_contract(d, &["x.log"]);
}

/// 真正的用法：先 Stage / Unstage，再按刚操作的路径读。
#[test]
fn after_stage_and_unstage_the_touched_paths_match_the_full_listing() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "1\n2\n");
    write(d, "clean.txt", "c\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "a.txt", "1\n2\n3\n");
    write(d, "newdir/n.txt", "n1\nn2\n");
    write(d, "loose.txt", "l\n");

    // Stage：一个修改、一个新目录里的未跟踪文件
    run_git(d, &["add", "--", "a.txt", "newdir/n.txt"]);
    assert_contract(d, &["a.txt", "newdir/n.txt"]);
    // 暂存之后又改：两边都有
    write(d, "a.txt", "1\n2\n3\n4\n");
    assert_contract(d, &["a.txt"]);

    // Unstage 新文件：它回到「整个目录都是未跟踪」的状态，Unstaged 要重新列出它
    run_git(d, &["reset", "-q", "HEAD", "--", "newdir/n.txt"]);
    let patch = engine::diff::read_status_paths(d.to_str().unwrap(), &["newdir/n.txt".to_string()])
        .unwrap();
    assert_eq!(lines(&patch.unstaged), ["added newdir/n.txt +2 -0"]);
    assert_contract(d, &["newdir/n.txt"]);
    assert_contract_everywhere(d, "clean.txt");
}

/// `git add` 的连带改动：已跟踪的文件 `t` 换成同名目录后暂存 `t/inner.txt`，git 会把 index 里的
/// 文件 `t` 一起删掉（目录与文件不能同名）。`t` 没被点名，局部重读看不到它在 Unstaged 里那一项已经
/// 没了——前端靠「Staged 里没点名的路径也变了」发现这种情况，退回全量重读。这里锁住它的前提：
/// 连带改动一定反映在 Staged 上。
#[test]
fn a_side_effect_on_an_untouched_path_shows_up_in_staged() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "t", "file\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    fs::remove_file(d.join("t")).unwrap();
    write(d, "t/inner.txt", "in\n");
    let before = full(d);
    assert_eq!(lines(&before.staged), Vec::<String>::new());
    assert_eq!(
        lines(&before.unstaged),
        ["deleted t +0 -1", "added t/inner.txt +1 -0"]
    );

    run_git(d, &["add", "--", "t/inner.txt"]);
    let after = full(d);
    // 没点名的 `t` 从 Unstaged 消失、出现在 Staged
    assert_eq!(lines(&after.unstaged), Vec::<String>::new());
    assert_eq!(
        lines(&after.staged),
        ["deleted t +0 -1", "added t/inner.txt +1 -0"]
    );
    assert_contract(d, &["t/inner.txt"]);
}

/// 字面量 pathspec 在 libgit2 里仍按目录前缀匹配：点名 `t` 会把 `t/x` 也带出来。
/// 前端只替换点名路径的项，多带出来的 `t/x` 会和列表里原有的那一项重复（独立 review 第 1 条）。
#[test]
fn a_named_path_does_not_bring_in_entries_under_it() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "t", "file\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    fs::remove_file(d.join("t")).unwrap();
    write(d, "t/x", "x\n");
    run_git(d, &["rm", "-q", "--cached", "t"]);
    let before = full(d);
    assert_eq!(lines(&before.staged), ["deleted t +0 -1"]);
    assert_eq!(lines(&before.unstaged), ["added t/x +1 -0"]);

    // Unstage `t`
    run_git(d, &["reset", "-q", "HEAD", "--", "t"]);
    let patch = engine::diff::read_status_paths(d.to_str().unwrap(), &["t".to_string()]).unwrap();
    assert!(
        patch.unstaged.iter().all(|f| f.path == "t"),
        "只该有 `t` 自己：{:?}",
        lines(&patch.unstaged)
    );
    assert_contract(d, &["t"]);
}

#[test]
fn a_repo_without_commits_matches_the_full_listing() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "staged.txt", "s1\ns2\n");
    run_git(d, &["add", "staged.txt"]);
    write(d, "loose.txt", "l\n");
    assert_contract_everywhere(d, "staged.txt");
}

#[test]
fn conflicts_match_the_full_listing() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "both.txt", "base\n");
    write(d, "del.txt", "base\n");
    write(d, "clean.txt", "c\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    run_git(d, &["checkout", "-q", "-b", "feat"]);
    write(d, "both.txt", "theirs\n");
    run_git(d, &["rm", "-q", "del.txt"]);
    write(d, "feat-only.txt", "f1\nf2\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "feat"]);
    run_git(d, &["checkout", "-q", "main"]);
    write(d, "both.txt", "ours\n");
    write(d, "del.txt", "ours\n");
    run_git(d, &["commit", "-qam", "ours"]);
    assert!(!git(d, &["merge", "-q", "feat"]), "merge 应当冲突");

    assert_contract_everywhere(d, "clean.txt");
}

/// 路径按字面量匹配：读 `f[1].txt` 不能把 `f1.txt` 一起读进来。
/// 否则前端会拿 `f1.txt` 那一项去「替换」，列表里平白多一项（它本来就在，只是没被点名）。
#[test]
fn paths_are_matched_literally() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "base.txt", "b\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "f[1].txt", "bracket\n");
    write(d, "f1.txt", "plain\n");

    let patch =
        engine::diff::read_status_paths(d.to_str().unwrap(), &["f[1].txt".to_string()]).unwrap();
    assert_eq!(lines(&patch.unstaged), ["added f[1].txt +1 -0"]);
    assert_contract_everywhere(d, "base.txt");
}

/// 空路径列表绝不能退化成「全部」（git 与 libgit2 都把空 pathspec 当全部，AGENTS.md §6.2）。
/// 退化的话前端拿到整份 Unstaged，再「追加」到原列表上，每一项都会出现两次。
#[test]
fn an_empty_path_list_reads_no_unstaged_entries() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "1\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "a.txt", "1\n2\n");
    write(d, "s.txt", "s\n");
    run_git(d, &["add", "s.txt"]);
    write(d, "u.txt", "u\n");

    let patch = engine::diff::read_status_paths(d.to_str().unwrap(), &[]).unwrap();
    assert!(patch.unstaged.is_empty(), "{:?}", lines(&patch.unstaged));
    assert_eq!(lines(&patch.staged), lines(&full(d).staged));
}

/// 读不了的路径（悬空的目录联接）点名要也不出现，和全量读一致。
#[test]
fn an_unreadable_path_matches_the_full_listing() {
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
    assert_contract(d, &[".broken"]);
    assert_contract(d, &[".broken", "a.txt"]);
}

/// 前端要按 git 的顺序把新项插回列表，顺序取决于 index 是否不区分大小写。
#[test]
fn the_patch_reports_whether_the_index_ignores_case() {
    let dir = init_repo();
    let d = dir.path();
    for (value, expected) in [("true", true), ("false", false)] {
        run_git(d, &["config", "core.ignorecase", value]);
        let patch = engine::diff::read_status_paths(d.to_str().unwrap(), &[]).unwrap();
        assert_eq!(patch.ignore_case, expected, "core.ignorecase={value}");
    }
}

/// 「链接还在、目标已被删」的悬空状态。与 `status_contract.rs` 同款。
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
