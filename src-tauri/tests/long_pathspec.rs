#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 一次操作的路径多到拼不进一条命令行（Windows 上限 32767 个 UTF-16 字符）。
//!
//! 2 万文件的 SVN 混合 fixture 上实测（2026-11-09）：区头 Stage All 一次交下 631 个路径，
//! `git add -- <路径…>` 直接失败，报 `The filename or extension is too long. (os error 206)`。
//! 这里每个路径约 80 个字符、一次 900 个，合计约 7 万字符，是上限的两倍多。
//!
//! 只在 Windows 上会真正撞墙：Linux / macOS 的 ARG_MAX 是兆字节级，这些用例在那边
//! 改动前后都能过。批里混进几个特殊文件名，确认换了传参方式之后，`--` 与
//! `--literal-pathspecs` 守住的那几条（第 85 条）仍然成立，非 ASCII 路径也照常收得到。

use app_lib::commands::{diff, ops};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const MANY: usize = 900;

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-b", "main"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);
    // core.quotePath 默认会把中文转义成八进制，断言里比的是原样的路径
    run_git(dir.path(), &["config", "core.quotePath", "false"]);
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

/// 约 80 个字符的相对路径，正斜杠，和 status 交给前端的形态一致。
fn long_path(prefix: &str, i: usize) -> String {
    format!(
        "{prefix}/deeply/nested/directory/for/command/line/limit/file_{i:04}_padding_padding.txt"
    )
}

fn write(dir: &TempDir, rel: &str, content: &str) {
    let full = dir.path().join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

fn read(dir: &TempDir, rel: &str) -> String {
    fs::read_to_string(dir.path().join(rel)).unwrap()
}

fn staged_paths(dir: &TempDir) -> Vec<String> {
    diff::read_status(p(dir))
        .unwrap()
        .staged
        .into_iter()
        .map(|f| f.path)
        .collect()
}

/// 特殊文件名：连字符开头（会被当选项）、方括号（会被当 glob 匹配到 `f1.txt`）、中文加空格。
const SPECIAL: [&str; 3] = ["-dash.txt", "f[1].txt", "中文 文件.txt"];

#[test]
fn stage_and_unstage_take_more_paths_than_one_command_line_holds() {
    let dir = repo();
    // f1.txt 是 `f[1].txt` 当 glob 时会匹配到的那个文件：先提交，再改，不在这次的选择里
    write(&dir, "f1.txt", "one\n");
    run_git(dir.path(), &["add", "f1.txt"]);
    run_git(dir.path(), &["commit", "-m", "init"]);
    write(&dir, "f1.txt", "edited\n");

    let mut paths: Vec<String> = (0..MANY).map(|i| long_path("new", i)).collect();
    for name in SPECIAL {
        paths.push(name.to_string());
    }
    for rel in &paths {
        write(&dir, rel, "x\n");
    }
    let total: usize = paths.iter().map(|s| s.encode_utf16().count() + 1).sum();
    assert!(
        total > 32767 * 2,
        "路径总长 {total} 必须远超 Windows 命令行上限"
    );

    ops::stage(p(&dir), paths.clone()).unwrap();
    let staged = staged_paths(&dir);
    assert_eq!(
        staged.len(),
        paths.len(),
        "选中的每个路径都应暂存，且只暂存它们"
    );
    for name in SPECIAL {
        assert!(staged.iter().any(|s| s == name), "{name} 应已暂存");
    }
    assert!(
        !staged.iter().any(|s| s == "f1.txt"),
        "f1.txt 没被选中，不能因为 `f[1].txt` 的 glob 被一起暂存"
    );

    // 撤销方向：f1.txt 先单独暂存，撤掉整批时它必须留在暂存区
    ops::stage(p(&dir), vec!["f1.txt".into()]).unwrap();
    ops::unstage(p(&dir), paths).unwrap();
    assert_eq!(
        staged_paths(&dir),
        vec!["f1.txt".to_string()],
        "整批应撤出暂存区，没选中的 f1.txt 留下"
    );
}

/// 路径从 stdin 读之后，`reset HEAD` 后面要有 `--` 收尾，否则 git 要猜 `HEAD` 是不是文件名。
/// 独立 review 实测（2026-11-09）：少了它，还没提交的仓库里报 `unknown revision`，
/// Unstage 用不了；改动前的 `reset HEAD -- <路径>` 在这里是好的。
#[test]
fn unstage_works_before_the_first_commit() {
    let dir = repo();
    write(&dir, "a.txt", "a\n");
    write(&dir, "b.txt", "b\n");
    ops::stage(p(&dir), vec!["a.txt".into(), "b.txt".into()]).unwrap();

    ops::unstage(p(&dir), vec!["a.txt".into()]).unwrap();
    assert_eq!(staged_paths(&dir), vec!["b.txt".to_string()]);
}

/// 同上：根目录有个叫 `HEAD` 的文件时，没有 `--` 的 `reset HEAD` 报
/// `both revision and filename`，选什么都撤不掉。`checkout HEAD`（Staged 区的 Discard）同一形态。
#[test]
fn unstage_and_staged_discard_work_with_a_file_named_head() {
    let dir = repo();
    write(&dir, "HEAD", "not a ref\n");
    write(&dir, "a.txt", "original\n");
    run_git(dir.path(), &["add", "-A"]);
    run_git(dir.path(), &["commit", "-m", "init"]);

    write(&dir, "a.txt", "edited\n");
    ops::stage(p(&dir), vec!["a.txt".into()]).unwrap();
    ops::unstage(p(&dir), vec!["a.txt".into()]).unwrap();
    assert!(staged_paths(&dir).is_empty(), "a.txt 应撤出暂存区");

    ops::stage(p(&dir), vec!["a.txt".into()]).unwrap();
    ops::discard(p(&dir), vec!["a.txt".into()], true).unwrap();
    assert_eq!(read(&dir, "a.txt"), "original\n", "a.txt 应回到 HEAD");
    assert_eq!(
        read(&dir, "HEAD"),
        "not a ref\n",
        "叫 HEAD 的文件没被选中，不能动"
    );
}

#[test]
fn discarding_unstaged_takes_more_paths_than_one_command_line_holds() {
    let dir = repo();
    let tracked: Vec<String> = (0..MANY).map(|i| long_path("tracked", i)).collect();
    for rel in &tracked {
        write(&dir, rel, "original\n");
    }
    write(&dir, "keep.txt", "original\n");
    run_git(dir.path(), &["add", "-A"]);
    run_git(dir.path(), &["commit", "-m", "init"]);

    for rel in &tracked {
        write(&dir, rel, "messed up\n");
    }
    write(&dir, "keep.txt", "not selected\n");
    let untracked: Vec<String> = (0..MANY).map(|i| long_path("junk", i)).collect();
    for rel in &untracked {
        write(&dir, rel, "junk\n");
    }

    let mut selection = tracked.clone();
    selection.extend(untracked.iter().cloned());
    ops::discard(p(&dir), selection, false).unwrap();

    for rel in &tracked {
        assert_eq!(read(&dir, rel), "original\n", "{rel} 应恢复");
    }
    for rel in &untracked {
        assert!(
            !dir.path().join(rel).exists(),
            "{rel} 是未跟踪文件，应被删除"
        );
    }
    assert_eq!(
        read(&dir, "keep.txt"),
        "not selected\n",
        "没选中的文件不能动"
    );
}

#[test]
fn discarding_staged_takes_more_paths_than_one_command_line_holds() {
    let dir = repo();
    let tracked: Vec<String> = (0..MANY / 2).map(|i| long_path("tracked", i)).collect();
    for rel in &tracked {
        write(&dir, rel, "original\n");
    }
    run_git(dir.path(), &["add", "-A"]);
    run_git(dir.path(), &["commit", "-m", "init"]);

    // 三类混在一起：已跟踪且改动已暂存（checkout HEAD）、新加进暂存区（rm -f）、未跟踪（直接删）
    for rel in &tracked {
        write(&dir, rel, "staged edit\n");
    }
    let added: Vec<String> = (0..MANY / 2).map(|i| long_path("added", i)).collect();
    for rel in &added {
        write(&dir, rel, "new\n");
    }
    run_git(dir.path(), &["add", "-A"]);
    let untracked: Vec<String> = (0..MANY / 2).map(|i| long_path("junk", i)).collect();
    for rel in &untracked {
        write(&dir, rel, "junk\n");
    }

    let mut selection = tracked.clone();
    selection.extend(added.iter().cloned());
    selection.extend(untracked.iter().cloned());
    ops::discard(p(&dir), selection, true).unwrap();

    for rel in &tracked {
        assert_eq!(read(&dir, rel), "original\n", "{rel} 应回到 HEAD");
    }
    for rel in added.iter().chain(&untracked) {
        assert!(!dir.path().join(rel).exists(), "{rel} 应从磁盘删除");
    }
    assert!(staged_paths(&dir).is_empty(), "暂存区应回到 HEAD");
}
