#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 文件历史 / Blame / 单文件 diff（计划 2026-11-08-file-history-blame）。
//! 全部在自建的临时 fixture 仓库里跑，不碰任何真实仓库（AGENTS.md §8）。

use app_lib::engine::file_history;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn rev(dir: &std::path::Path, spec: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", spec])
        .output()
        .expect("git not found");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn init() -> TempDir {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    git(d, &["init", "-q", "-b", "main"]);
    git(d, &["config", "user.email", "t@t.com"]);
    git(d, &["config", "user.name", "Tester"]);
    git(d, &["config", "core.autocrlf", "false"]);
    dir
}

/// 改名后的路径带空格和中文：Windows 上最容易出事的形态（AGENTS §8）。
const RENAMED: &str = "dir sp/中文.txt";

/// add a → edit a → 改名到 `RENAMED` → append，另有一个只改 other.txt 的提交夹在中间。
fn renamed_repo() -> TempDir {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("a.txt"), "l1\nl2\nl3\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "add a"]);
    fs::write(d.join("a.txt"), "l1\nL2\nl3\n").unwrap();
    git(d, &["commit", "-qam", "edit a"]);
    fs::write(d.join("other.txt"), "o\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "unrelated"]);
    fs::create_dir_all(d.join("dir sp")).unwrap();
    git(d, &["mv", "a.txt", RENAMED]);
    git(d, &["commit", "-qm", "rename"]);
    fs::write(d.join(RENAMED), "l1\nL2\nl3\nl4\n").unwrap();
    git(d, &["commit", "-qam", "append"]);
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

fn head() -> Vec<String> {
    vec!["HEAD".to_string()]
}

// ---------------------------------------------------------------------------
// file_history
// ---------------------------------------------------------------------------

#[test]
fn history_follows_renames_and_skips_unrelated_commits() {
    let dir = renamed_repo();
    let h = file_history::file_history(&p(&dir), RENAMED, &head()).unwrap();
    let subjects: Vec<&str> = h.entries.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, ["append", "rename", "edit a", "add a"]);
    assert!(!h.truncated);

    // 每一条带的是**那次提交里**这个文件的路径：改名之前的提交要用旧路径去 Blame / 读 diff
    let paths: Vec<&str> = h.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, [RENAMED, RENAMED, "a.txt", "a.txt"]);
    let statuses: Vec<&str> = h.entries.iter().map(|e| e.status.as_str()).collect();
    assert_eq!(statuses, ["modified", "renamed", "modified", "added"]);
    assert_eq!(h.entries[1].old_path.as_deref(), Some("a.txt"));
    assert_eq!(h.entries[0].old_path, None);

    let top = &h.entries[0];
    assert_eq!(top.id, rev(dir.path(), "HEAD"));
    assert_eq!(top.parents, [rev(dir.path(), "HEAD~1")]);
    assert_eq!(top.author, "Tester");
    assert!(top.time > 0);
    // 根提交没有父
    assert!(h.entries[3].parents.is_empty());
}

#[test]
fn history_of_a_deleted_file_starts_with_the_deletion() {
    let dir = renamed_repo();
    let d = dir.path();
    git(d, &["rm", "-q", RENAMED]);
    git(d, &["commit", "-qm", "delete"]);
    let h = file_history::file_history(&p(&dir), RENAMED, &head()).unwrap();
    assert_eq!(h.entries[0].subject, "delete");
    assert_eq!(h.entries[0].status, "deleted");
    assert_eq!(h.entries.len(), 5);
}

#[test]
fn history_of_a_path_that_never_existed_is_empty_not_an_error() {
    let dir = renamed_repo();
    let h = file_history::file_history(&p(&dir), "nope.txt", &head()).unwrap();
    assert!(h.entries.is_empty());
}

#[test]
fn history_unions_head_with_a_commit_on_another_branch() {
    // 在别的分支上的提交里打开历史：只从 HEAD 走会是空的（决策 D2）
    let dir = renamed_repo();
    let d = dir.path();
    git(d, &["checkout", "-q", "-b", "side"]);
    fs::write(d.join("side.txt"), "s1\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "side add"]);
    let side = rev(d, "HEAD");
    git(d, &["checkout", "-q", "main"]);

    let only_head = file_history::file_history(&p(&dir), "side.txt", &head()).unwrap();
    assert!(only_head.entries.is_empty());
    let both =
        file_history::file_history(&p(&dir), "side.txt", &["HEAD".to_string(), side.clone()])
            .unwrap();
    assert_eq!(both.entries.len(), 1);
    assert_eq!(both.entries[0].id, side);
}

#[test]
fn history_is_capped_and_marked_truncated() {
    let dir = init();
    let d = dir.path();
    for i in 0..5 {
        fs::write(d.join("f.txt"), format!("{i}\n")).unwrap();
        git(d, &["add", "."]);
        git(d, &["commit", "-qm", &format!("c{i}")]);
    }
    let h = file_history::file_history_limited(&p(&dir), "f.txt", &head(), 3).unwrap();
    assert_eq!(h.entries.len(), 3);
    assert!(h.truncated);
    let all = file_history::file_history_limited(&p(&dir), "f.txt", &head(), 5).unwrap();
    assert_eq!(all.entries.len(), 5);
    assert!(!all.truncated, "恰好等于上限时不算截断");
}

#[test]
fn history_subject_with_separators_survives() {
    // 标题里出现 tab、引号、竖线这些字符不能把解析打乱
    let dir = init();
    let d = dir.path();
    fs::write(d.join("f.txt"), "x\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "feat: \"a\"\t| b 中文"]);
    let h = file_history::file_history(&p(&dir), "f.txt", &head()).unwrap();
    assert_eq!(h.entries[0].subject, "feat: \"a\"\t| b 中文");
}

/// 文件名带 `[` 时 git 默认把它当通配符：`a[1].txt` 会匹配到 `a1.txt`（独立 review 第 3 条）。
#[test]
fn history_matches_the_path_literally() {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("a[1].txt"), "x\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "bracket"]);
    fs::write(d.join("a1.txt"), "y\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "plain only"]);
    let h = file_history::file_history(&p(&dir), "a[1].txt", &head()).unwrap();
    let subjects: Vec<&str> = h.entries.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, ["bracket"]);
    assert_eq!(h.entries[0].path, "a[1].txt");
    // `:` 开头的文件名（pathspec magic）不测：Windows 上 `:` 不能出现在文件名里，写出来是 NTFS 的备用数据流
}

/// 两边都改了 f.txt、合并时手工解冲突：真实 git 的 `log --follow` 输出什么，就固化什么
/// （独立 review 第 2 条：原先的 lib 单测用的是手写输入，没有合并的真实形态）。
#[test]
fn history_and_blame_around_a_conflict_resolving_merge() {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("f.txt"), "base\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "base"]);
    git(d, &["checkout", "-q", "-b", "side"]);
    fs::write(d.join("f.txt"), "side\n").unwrap();
    git(d, &["commit", "-qam", "side change"]);
    git(d, &["checkout", "-q", "main"]);
    fs::write(d.join("f.txt"), "main\n").unwrap();
    git(d, &["commit", "-qam", "main change"]);
    let merged = Command::new("git")
        .current_dir(d)
        .args(["merge", "-q", "side"])
        .status()
        .unwrap();
    assert!(!merged.success(), "应当冲突");
    fs::write(d.join("f.txt"), "resolved\n").unwrap();
    git(d, &["commit", "-qam", "merge side"]);
    let merge = rev(d, "HEAD");

    let h = file_history::file_history(&p(&dir), "f.txt", &head()).unwrap();
    let subjects: Vec<&str> = h.entries.iter().map(|e| e.subject.as_str()).collect();
    // 两条分支上的改动都在
    assert!(subjects.contains(&"side change"), "{subjects:?}");
    assert!(subjects.contains(&"main change"), "{subjects:?}");
    // 实测（git 2.51）：`--follow` 下没有 diff 输出的合并提交整条不显示，解冲突的那一笔不在历史里
    assert!(
        !h.entries.iter().any(|e| e.id == merge),
        "合并提交出现在了 --follow 的历史里：{subjects:?}"
    );
    // 每一条都有父提交与路径，没有解析残缺的记录
    assert!(h.entries.iter().all(|e| e.path == "f.txt"));

    // Blame 把解冲突写下的行归到合并提交：前端点它时历史里没有，退回「在提交图中定位」
    let b = file_history::blame_file(&p(&dir), "f.txt", Some("HEAD")).unwrap();
    assert_eq!(b.lines[0].commit, merge);
    assert_eq!(b.commits[&merge].summary, "merge side");
}

/// `git log --follow` 整个遍历只有一份「当前路径」：多个起点时，一条分支上跨过改名之后，
/// 别的分支上的提交也按旧名去匹配（/code-review 第 4 条）。
///
/// base 建 f.txt；side 上把它改名成 g.txt（S1）再改 g.txt（S2）；main 上在 base 与 S1 之间改了 f.txt（M1）。
/// 从 HEAD(main) + S2 打开 g.txt 的历史：M1 从来不在 g.txt 的历史里。
#[test]
fn history_from_two_starts_keeps_each_branchs_rename_to_itself() {
    let dir = init();
    let d = dir.path();
    let at = |date: &str, args: &[&str]| {
        let ok = Command::new("git")
            .current_dir(d)
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args(args)
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    };
    fs::write(d.join("f.txt"), "1\n2\n3\n").unwrap();
    git(d, &["add", "."]);
    at("2026-01-01T00:00:00+00:00", &["commit", "-qm", "base"]);
    git(d, &["checkout", "-q", "-b", "side"]);
    git(d, &["mv", "f.txt", "g.txt"]);
    at("2026-01-03T00:00:00+00:00", &["commit", "-qm", "S1 rename"]);
    fs::write(d.join("g.txt"), "1\n2\n3\n4\n").unwrap();
    at(
        "2026-01-04T00:00:00+00:00",
        &["commit", "-qam", "S2 edit g"],
    );
    let s2 = rev(d, "HEAD");
    git(d, &["checkout", "-q", "main"]);
    fs::write(d.join("f.txt"), "1\nmain\n3\n").unwrap();
    at(
        "2026-01-02T00:00:00+00:00",
        &["commit", "-qam", "M1 edit f on main"],
    );

    let h = file_history::file_history(&p(&dir), "g.txt", &["HEAD".to_string(), s2]).unwrap();
    let subjects: Vec<&str> = h.entries.iter().map(|e| e.subject.as_str()).collect();
    assert_eq!(subjects, ["S2 edit g", "S1 rename", "base"]);
}

/// `diff.renames=false` 时 libgit2 的 `find_similar(None)` 按配置不做改名检测，
/// 历史里那条 `renamed`（git log 带了 -M）的 diff 就成了整份新增（/code-review 第 2 条）。
#[test]
fn file_diff_pairs_a_rename_even_when_diff_renames_is_off() {
    let dir = renamed_repo();
    git(dir.path(), &["config", "diff.renames", "false"]);
    let h = file_history::file_history(&p(&dir), RENAMED, &head()).unwrap();
    let rename = h.entries.iter().find(|e| e.subject == "rename").unwrap();
    assert_eq!(rename.status, "renamed", "git log 加了 -M，历史里仍是改名");
    let data = file_history::commit_file_diff(
        &p(&dir),
        &rename.id,
        &rename.path,
        rename.old_path.as_deref(),
    )
    .unwrap();
    assert_eq!(data.files.len(), 1);
    assert_eq!(data.files[0].status, "renamed");
    assert_eq!(data.files[0].old_path.as_deref(), Some("a.txt"));
}

/// `git rm --cached` 之后：暂存区里是删除、工作区里是未跟踪，但 HEAD 里有它，工作区 Blame 照样能做
/// （/code-review 第 1 条的反例：前端原来把它当新文件置灰）。
#[test]
fn blame_of_the_workdir_works_after_rm_cached() {
    let dir = renamed_repo();
    git(dir.path(), &["rm", "-q", "--cached", RENAMED]);
    let b = file_history::blame_file(&p(&dir), RENAMED, None).unwrap();
    assert_eq!(b.lines.len(), 4);
    assert_eq!(b.lines[0].commit, rev(dir.path(), "HEAD~4"));
}

/// 二进制判定两侧同一个口径（前 8000 字节里有 NUL）。很多控制字符、没有 NUL 的文本，
/// libgit2 的 `is_binary` 会判成二进制，`git blame` 照常按行处理（/code-review 第 6 条）。
#[test]
fn blame_binary_check_is_the_same_for_a_commit_and_the_workdir() {
    let dir = init();
    let d = dir.path();
    let body: String = (0..200)
        .map(|i| format!("{i}\x01\x02\x03\x04\x05\x06\n"))
        .collect();
    fs::write(d.join("ctl.txt"), &body).unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "ctl"]);
    let at_head = file_history::blame_file(&p(&dir), "ctl.txt", Some("HEAD")).unwrap();
    let workdir = file_history::blame_file(&p(&dir), "ctl.txt", None).unwrap();
    assert_eq!(at_head.binary, workdir.binary);
    assert!(!at_head.binary);
    assert_eq!(at_head.lines.len(), 200);
}

/// 上限用 `-L` 交给 git，少算的那部分不用算；恰好等于上限、比上限少的文件 `-L` 不能越界报错。
#[test]
fn blame_limit_boundaries() {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("four.txt"), "1\n2\n3\n4\n").unwrap();
    fs::write(d.join("no-eol.txt"), "1\n2\n3\n4\n5").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "files"]);
    for rev in [Some("HEAD"), None] {
        let exact = file_history::blame_file_limited(&p(&dir), "four.txt", rev, 4).unwrap();
        assert_eq!(
            (exact.lines.len(), exact.truncated, exact.total_lines),
            (4, false, 4)
        );
        let more = file_history::blame_file_limited(&p(&dir), "four.txt", rev, 10).unwrap();
        assert_eq!((more.lines.len(), more.truncated), (4, false));
        // 最后一行没有换行也算一行
        let cut = file_history::blame_file_limited(&p(&dir), "no-eol.txt", rev, 3).unwrap();
        assert_eq!(
            (cut.lines.len(), cut.truncated, cut.total_lines),
            (3, true, 5)
        );
        assert_eq!(cut.lines[2].content, "3");
    }
}

#[test]
fn history_rejects_empty_file_and_option_like_revs() {
    let dir = renamed_repo();
    // 空 pathspec 在 git 里是「全部」（AGENTS §6.2）
    assert!(file_history::file_history(&p(&dir), "", &head()).is_err());
    assert!(file_history::file_history(&p(&dir), RENAMED, &[]).is_err());
    for bad in ["--all", "-p", "", "--output=x.txt"] {
        assert!(
            file_history::file_history(&p(&dir), RENAMED, &[bad.to_string()]).is_err(),
            "rev {bad:?} 应被拒绝"
        );
    }
    assert!(!dir.path().join("x.txt").exists(), "选项注入不能真的执行");
}

// ---------------------------------------------------------------------------
// blame_file
// ---------------------------------------------------------------------------

#[test]
fn blame_attributes_each_line_to_its_commit_across_renames() {
    let dir = renamed_repo();
    let d = dir.path();
    let b = file_history::blame_file(&p(&dir), RENAMED, Some("HEAD")).unwrap();
    let texts: Vec<&str> = b.lines.iter().map(|l| l.content.as_str()).collect();
    assert_eq!(texts, ["l1", "L2", "l3", "l4"]);
    let add = rev(d, "HEAD~4");
    let edit = rev(d, "HEAD~3");
    let append = rev(d, "HEAD");
    let ids: Vec<&str> = b.lines.iter().map(|l| l.commit.as_str()).collect();
    assert_eq!(ids, [&add, &edit, &add, &append]);
    // 改名之前的行：当时的路径是旧路径
    assert_eq!(b.lines[0].orig_path, "a.txt");
    assert_eq!(b.lines[3].orig_path, RENAMED);
    assert_eq!(b.lines[1].orig_line, 2);
    assert_eq!(b.lines[3].final_line, 4);

    // 同一提交的信息只在第一次出现时给，后面的行也要能查到
    let info = b.commits.get(&add).expect("add 的提交信息");
    assert_eq!(info.author, "Tester");
    assert_eq!(info.summary, "add a");
    assert!(info.time > 0);
    assert!(info.previous.is_none(), "根提交没有 previous");
    let e = b.commits.get(&edit).unwrap();
    assert_eq!(
        e.previous
            .as_ref()
            .map(|p| (p.commit.as_str(), p.path.as_str())),
        Some((add.as_str(), "a.txt"))
    );
    assert!(!b.binary);
    assert!(!b.truncated);
}

#[test]
fn blame_of_the_workdir_marks_uncommitted_lines() {
    let dir = renamed_repo();
    fs::write(dir.path().join(RENAMED), "l1\nWORK\nl3\nl4\n").unwrap();
    let b = file_history::blame_file(&p(&dir), RENAMED, None).unwrap();
    let line = &b.lines[1];
    assert_eq!(line.content, "WORK");
    assert_eq!(line.commit, "0".repeat(40));
    // 提交过的行照常归属
    assert_eq!(b.lines[0].commit, rev(dir.path(), "HEAD~4"));
    // 带 rev 时看的是提交里那一版，不受工作区影响
    let at_head = file_history::blame_file(&p(&dir), RENAMED, Some("HEAD")).unwrap();
    assert_eq!(at_head.lines[1].content, "L2");
}

#[test]
fn blame_of_an_old_revision_uses_that_revision() {
    let dir = renamed_repo();
    let b = file_history::blame_file(&p(&dir), "a.txt", Some("HEAD~2")).unwrap();
    let texts: Vec<&str> = b.lines.iter().map(|l| l.content.as_str()).collect();
    assert_eq!(texts, ["l1", "L2", "l3"]);
}

#[test]
fn blame_strips_crlf_from_line_content() {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("w.txt"), "a\r\nb\r\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "crlf"]);
    let b = file_history::blame_file(&p(&dir), "w.txt", Some("HEAD")).unwrap();
    let texts: Vec<&str> = b.lines.iter().map(|l| l.content.as_str()).collect();
    assert_eq!(texts, ["a", "b"]);
}

#[test]
fn blame_flags_binary_files_and_drops_their_lines() {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("bin.dat"), b"\x89PNG\r\n\x1a\n\x00\x00rest\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "bin"]);
    let b = file_history::blame_file(&p(&dir), "bin.dat", Some("HEAD")).unwrap();
    assert!(b.binary);
    assert!(b.lines.is_empty(), "二进制内容不该送到前端");
}

#[test]
fn blame_is_capped_and_marked_truncated() {
    let dir = init();
    let d = dir.path();
    let body: String = (0..10).map(|i| format!("{i}\n")).collect();
    fs::write(d.join("long.txt"), body).unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "long"]);
    let b = file_history::blame_file_limited(&p(&dir), "long.txt", Some("HEAD"), 4).unwrap();
    assert_eq!(b.lines.len(), 4);
    assert!(b.truncated);
    assert_eq!(b.total_lines, 10);
}

#[test]
fn blame_of_an_empty_file_has_no_lines() {
    let dir = init();
    let d = dir.path();
    fs::write(d.join("empty.txt"), "").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "empty"]);
    let b = file_history::blame_file(&p(&dir), "empty.txt", Some("HEAD")).unwrap();
    assert!(b.lines.is_empty());
    assert!(!b.binary);
}

#[test]
fn blame_errors_for_missing_path_empty_file_and_option_like_rev() {
    let dir = renamed_repo();
    assert!(file_history::blame_file(&p(&dir), "nope.txt", Some("HEAD")).is_err());
    assert!(file_history::blame_file(&p(&dir), "", Some("HEAD")).is_err());
    assert!(file_history::blame_file(&p(&dir), RENAMED, Some("")).is_err());
    assert!(file_history::blame_file(&p(&dir), RENAMED, Some("--reverse")).is_err());
}

// ---------------------------------------------------------------------------
// commit_file_diff
// ---------------------------------------------------------------------------

fn changed(d: &app_lib::engine::diff::FileDiff) -> Vec<String> {
    d.hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .filter(|l| l.origin != ' ')
        .map(|l| format!("{}{}", l.origin, l.content))
        .collect()
}

#[test]
fn file_diff_shows_only_that_file_in_that_commit() {
    let dir = renamed_repo();
    let d = dir.path();
    // 这个提交里还改了别的文件：结果只能有这一个
    fs::write(d.join(RENAMED), "l1\nL2\nl3\nl4\nl5\n").unwrap();
    fs::write(d.join("other.txt"), "changed\n").unwrap();
    git(d, &["commit", "-qam", "two files"]);
    let data = file_history::commit_file_diff(&p(&dir), "HEAD", RENAMED, None).unwrap();
    assert_eq!(data.files.len(), 1);
    assert_eq!(data.files[0].path, RENAMED);
    assert_eq!(changed(&data.files[0]), ["+l5"]);
}

#[test]
fn file_diff_of_a_rename_commit_pairs_old_and_new_paths() {
    let dir = renamed_repo();
    let data = file_history::commit_file_diff(&p(&dir), "HEAD~1", RENAMED, Some("a.txt")).unwrap();
    assert_eq!(data.files.len(), 1);
    let f = &data.files[0];
    assert_eq!(f.status, "renamed");
    assert_eq!(f.old_path.as_deref(), Some("a.txt"));
    assert!(changed(f).is_empty(), "纯改名没有行改动");
}

#[test]
fn file_diff_of_root_and_deletion_commits() {
    let dir = renamed_repo();
    let d = dir.path();
    let root = file_history::commit_file_diff(&p(&dir), "HEAD~4", "a.txt", None).unwrap();
    assert_eq!(root.files[0].status, "added");
    assert_eq!(changed(&root.files[0]), ["+l1", "+l2", "+l3"]);

    git(d, &["rm", "-q", RENAMED]);
    git(d, &["commit", "-qm", "delete"]);
    let del = file_history::commit_file_diff(&p(&dir), "HEAD", RENAMED, None).unwrap();
    assert_eq!(del.files[0].status, "deleted");
    assert_eq!(changed(&del.files[0]).len(), 4);
}

#[test]
fn file_diff_matches_paths_literally() {
    // `f[1].txt` 当模式会匹配到 `f1.txt`
    let dir = init();
    let d = dir.path();
    fs::write(d.join("f[1].txt"), "a\n").unwrap();
    fs::write(d.join("f1.txt"), "b\n").unwrap();
    git(d, &["add", "."]);
    git(d, &["commit", "-qm", "both"]);
    let data = file_history::commit_file_diff(&p(&dir), "HEAD", "f[1].txt", None).unwrap();
    let paths: Vec<&str> = data.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["f[1].txt"]);
}

#[test]
fn file_diff_rejects_empty_file() {
    let dir = renamed_repo();
    assert!(file_history::commit_file_diff(&p(&dir), "HEAD", "", None).is_err());
    // 空的旧路径当成没有旧路径，不能把 pathspec 放宽成全部
    let data = file_history::commit_file_diff(&p(&dir), "HEAD", RENAMED, Some("")).unwrap();
    assert_eq!(data.files.len(), 1);
}
