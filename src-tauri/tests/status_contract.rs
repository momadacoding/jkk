//! `read_status` 的完整输出快照。
//!
//! 这组测试先在**原实现**（`diff_index_to_workdir` + `statuses()` 两遍扫描）上跑通，
//! 再换成只扫一遍的实现。断言的是整份列表——路径、状态、增删行数、顺序都算在内，
//! 换实现时任何一处行为漂移都会在这里暴露，而不是只核对某一项在不在。

use app_lib::engine;
use app_lib::engine::diff::StatusData;
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
    // 行数断言不受本机 autocrlf 影响
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

/// 一项一行：`<状态> <路径> +<增> -<删>`，按 `read_status` 给出的顺序。
fn dump(status: &StatusData) -> (Vec<String>, Vec<String>) {
    let lines = |files: &[engine::diff::FileChange]| {
        files
            .iter()
            .map(|f| format!("{} {} +{} -{}", f.status, f.path, f.additions, f.deletions))
            .collect::<Vec<_>>()
    };
    (lines(&status.staged), lines(&status.unstaged))
}

fn read(d: &Path) -> (Vec<String>, Vec<String>) {
    dump(&engine::diff::read_status(d.to_str().unwrap()).unwrap())
}

/// 日常会碰到的各种改动混在一起：两边都改、只暂存、只在工作区、`git mv`、
/// 暂存后又在工作区删掉、整个目录删掉、深层未跟踪目录、被忽略的文件、空目录、中文路径。
#[test]
fn mixed_changes_are_listed_exactly() {
    let dir = init_repo();
    let d = dir.path();
    write(d, ".gitignore", "*.log\n");
    write(d, "a.txt", "1\n2\n3\n");
    write(d, "b.txt", "b1\nb2\n");
    write(d, "gone.txt", "g\n");
    write(d, "mv-src.txt", "m\n");
    write(d, "dir/keep.txt", "k\n");
    write(d, "dir/drop.txt", "d1\nd2\nd3\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);

    // a.txt：第 1 行改了并暂存，第 3 行又在工作区改了 → 两边各一项
    write(d, "a.txt", "x\n2\n3\n");
    run_git(d, &["add", "a.txt"]);
    write(d, "a.txt", "x\n2\ny\n");
    // 只在工作区删掉
    fs::remove_file(d.join("b.txt")).unwrap();
    // 暂存的删除
    run_git(d, &["rm", "-q", "gone.txt"]);
    // 暂存的新文件，随后又在工作区删掉（AD）
    write(d, "new.txt", "n\n");
    run_git(d, &["add", "new.txt"]);
    fs::remove_file(d.join("new.txt")).unwrap();
    // git mv：不做改名检测，列成一删一增
    run_git(d, &["mv", "mv-src.txt", "mv-dst.txt"]);
    // 整个已跟踪目录删掉
    fs::remove_dir_all(d.join("dir")).unwrap();
    // 未跟踪：顶层、深层目录、中文带空格
    write(d, "top.txt", "t\n");
    write(d, "nested/deep/u.txt", "u1\nu2\n");
    write(d, "中 文.txt", "z\n");
    // 不该出现的：被忽略的文件、空目录
    write(d, "x.log", "ignored\n");
    fs::create_dir_all(d.join("emptydir")).unwrap();

    let (staged, unstaged) = read(d);
    assert_eq!(
        staged,
        [
            "modified a.txt +1 -1",
            "deleted gone.txt +0 -1",
            "added mv-dst.txt +1 -0",
            "deleted mv-src.txt +0 -1",
            "added new.txt +1 -0",
        ]
    );
    assert_eq!(
        unstaged,
        [
            "modified a.txt +1 -1",
            "deleted b.txt +0 -2",
            "deleted dir/drop.txt +0 -3",
            "deleted dir/keep.txt +0 -1",
            "added nested/deep/u.txt +2 -0",
            "deleted new.txt +0 -1",
            "added top.txt +1 -0",
            "added 中 文.txt +1 -0",
        ]
    );
}

/// 还没有任何提交：没有 HEAD，Staged 要和空树比。
#[test]
fn a_repo_without_commits_lists_staged_and_untracked() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "staged.txt", "s1\ns2\n");
    run_git(d, &["add", "staged.txt"]);
    write(d, "loose.txt", "l\n");

    assert_eq!(
        read(d),
        (
            vec!["added staged.txt +2 -0".to_string()],
            vec!["added loose.txt +1 -0".to_string()],
        )
    );
}

/// 干净的工作区两边都是空的。
#[test]
fn a_clean_repo_has_no_changes() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "a\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);

    assert_eq!(read(d), (Vec::<String>::new(), Vec::<String>::new()));
}

/// 合并冲突中：冲突文件只在 Unstaged 出现一次（`conflicted`），不进 Staged；
/// 同一次合并里自动合进来的新文件照常在 Staged。UU 和 DU（对方删、我方改）都要覆盖。
#[test]
fn conflicts_are_listed_once_and_only_in_unstaged() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "both.txt", "base\n");
    write(d, "del.txt", "base\n");
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

    let (staged, unstaged) = read(d);
    assert_eq!(staged, ["added feat-only.txt +2 -0"]);
    assert_eq!(
        unstaged,
        ["conflicted both.txt +0 -0", "conflicted del.txt +0 -0"]
    );
}

/// 工作区里读不了的路径（悬空的目录联接 / 符号链接）不能出现，也不能拖垮其余文件的行数。
#[test]
fn an_unreadable_path_is_dropped_and_the_rest_keep_their_line_counts() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "1\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "a.txt", "1\n2\n");
    write(d, "zz-untracked.txt", "u\n");
    if !make_dangling_link(&d.join(".broken")) {
        eprintln!("跳过：本机无法创建目录联接");
        return;
    }

    assert_eq!(
        read(d),
        (
            Vec::<String>::new(),
            vec![
                "modified a.txt +1 -0".to_string(),
                "added zz-untracked.txt +1 -0".to_string(),
            ],
        )
    );
}

/// `git add -N`（intent-to-add）：index 里有一条空 blob 的占位。
///
/// libgit2 把它算成「Staged 侧新增一个空文件 + Unstaged 侧修改」，和 git 命令行
/// （只在工作区一侧显示 ` A`）不一样。这里锁住的是原实现的现状，不是理想行为。
#[test]
fn intent_to_add_is_listed_on_both_sides_as_libgit2_reports_it() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "a.txt", "a\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    write(d, "ita.txt", "i1\ni2\n");
    run_git(d, &["add", "-N", "ita.txt"]);

    assert_eq!(
        read(d),
        (
            vec!["added ita.txt +0 -0".to_string()],
            vec!["modified ita.txt +2 -0".to_string()],
        )
    );
}

/// 已跟踪的文件被换成同名目录：status 带 `INCLUDE_TYPECHANGE`，
/// 拆不拆成一删一增要和原实现一致。
#[test]
fn a_file_replaced_by_a_directory_matches_the_original_listing() {
    let dir = init_repo();
    let d = dir.path();
    write(d, "t", "file\n");
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    fs::remove_file(d.join("t")).unwrap();
    write(d, "t/inner.txt", "in\n");

    assert_eq!(
        read(d),
        (
            Vec::<String>::new(),
            vec![
                "deleted t +0 -1".to_string(),
                "added t/inner.txt +1 -0".to_string(),
            ],
        )
    );
}

/// 「链接还在、目标已被删」的悬空状态。与 `fixtures.rs` 同款。
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
