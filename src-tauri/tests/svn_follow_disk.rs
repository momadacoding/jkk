#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进冲突时能不能直接用磁盘上的内容（`svn_follow_disk`；设计
//! `2026-11-25-follow-resolve-with-disk.md` §6、§11）。
//!
//! 判定本身只看 git 对象与磁盘，在纯 git 的临时仓库上测：base 提交是「分支的基线」，C 是分支上
//! 改过这个文件的提交（stage 3），「SVN 的新版本」直接写成一个 blob（stage 2）。端到端的形状
//! （真 SVN fixture、跟进完成）在 `tests/svn_follow_resolve_with_disk.rs`。

use app_lib::engine::svn_follow_disk::{
    check_disk, has_conflict_markers, outside_hunks_match, DiskCheck,
};
use app_lib::engine::svn_replay::ConflictStage;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const F: &str = "A.lua";

fn git_in(dir: &Path, args: &[&str], stdin: Option<&[u8]>) -> String {
    use std::io::Write;
    let mut child = Command::new("git")
        .current_dir(dir)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("git not found");
    {
        let mut si = child.stdin.take().unwrap();
        if let Some(b) = stdin {
            si.write_all(b).unwrap();
        }
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
fn git(dir: &Path, args: &[&str]) -> String {
    git_in(dir, args, None)
}
fn blob(dir: &Path, body: &str) -> String {
    git_in(
        dir,
        &["hash-object", "-w", "--stdin"],
        Some(body.as_bytes()),
    )
}

/// 10 行（第 n 行是 `n`），`repl` 替换某些行。
fn lines(repl: &[(usize, &str)]) -> String {
    (1..=10)
        .map(|n| match repl.iter().find(|(k, _)| *k == n) {
            Some((_, v)) => format!("{v}\n"),
            None => format!("{n}\n"),
        })
        .collect()
}

struct Scene {
    dir: TempDir,
    base: String,
    head: String,
    stages: Vec<ConflictStage>,
}

impl Scene {
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn repo(&self) -> String {
        self.path().to_string_lossy().to_string()
    }
    fn disk(&self, body: &str) {
        std::fs::write(self.path().join(F), body).unwrap();
    }
    fn check(&self) -> DiskCheck {
        let files = BTreeMap::from([(F.to_string(), self.stages.clone())]);
        check_disk(&self.repo(), &self.base, &self.head, &files)
            .remove(F)
            .expect("每个文件都有结论")
    }
}

fn stage(n: u8, oid: &str) -> ConflictStage {
    ConflictStage {
        stage: n,
        mode: "100644".to_string(),
        oid: oid.to_string(),
    }
}

/// 基线是 `lines(&[])`；你的提交改第 2 行；SVN 的新版本改第 2 行和第 8 行。
fn scene_with(config: &[(&str, &str)]) -> Scene {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    git(p, &["init", "-q"]);
    git(p, &["config", "user.email", "t@t"]);
    git(p, &["config", "user.name", "t"]);
    git(p, &["config", "core.autocrlf", "false"]);
    for (k, v) in config {
        git(p, &["config", k, v]);
    }
    std::fs::write(p.join(F), lines(&[])).unwrap();
    git(p, &["add", "-A"]);
    git(p, &["commit", "-qm", "base"]);
    let base = git(p, &["rev-parse", "HEAD"]);
    std::fs::write(p.join(F), lines(&[(2, "two-mine")])).unwrap();
    git(p, &["commit", "-qam", "C"]);
    let head = git(p, &["rev-parse", "HEAD"]);
    let s1 = git(p, &["rev-parse", &format!("{base}:{F}")]);
    let s2 = blob(p, &lines(&[(2, "two-svn"), (8, "eight-svn")]));
    let s3 = git(p, &["rev-parse", &format!("{head}:{F}")]);
    Scene {
        dir,
        base,
        head,
        stages: vec![stage(1, &s1), stage(2, &s2), stage(3, &s3)],
    }
}
fn scene() -> Scene {
    scene_with(&[])
}

/// SVN 里合并好的结果：第 2 行两边都要，第 8 行取 SVN 的。
fn merged() -> String {
    lines(&[(2, "two-mine-and-svn"), (8, "eight-svn")])
}

fn refused(c: DiskCheck) -> String {
    match c {
        DiskCheck::Refused(why) => why,
        DiskCheck::Usable(r) => panic!("应当不能用，实际能用：{r:?}"),
    }
}

// —— 能用 ——

#[test]
fn svn_merge_result_on_disk_is_usable() {
    let s = scene();
    s.disk(&merged());
    match s.check() {
        DiskCheck::Usable(r) => {
            assert_eq!(r.mode, "100644");
            assert_eq!(
                git(s.path(), &["cat-file", "blob", &r.blob]),
                merged().trim_end(),
                "blob 写进了对象库，内容就是磁盘上的"
            );
        }
        DiskCheck::Refused(why) => panic!("应当能用：{why}"),
    }
}

#[test]
fn still_usable_when_the_user_configured_diff3_or_zdiff3() {
    // 设计 §11 第 4 条：用户配的 diff3 / zdiff3 不能让判定变成「不能用」。断言的是结论，
    // 不是模板的形状：diff3 多出的 `|||||||` 段落在冲突块里面，本来就不参与比对
    for style in ["diff3", "zdiff3"] {
        let s = scene_with(&[("merge.conflictStyle", style)]);
        s.disk(&merged());
        assert!(
            matches!(s.check(), DiskCheck::Usable(_)),
            "merge.conflictStyle={style}"
        );
    }
}

#[test]
fn crlf_on_disk_is_compared_after_the_add_filters() {
    // Windows 上的常态：对象里是 LF，磁盘上是 CRLF
    let s = scene_with(&[("core.autocrlf", "true")]);
    s.disk(&merged().replace('\n', "\r\n"));
    match s.check() {
        DiskCheck::Usable(r) => assert_eq!(
            git(s.path(), &["cat-file", "blob", &r.blob]),
            merged().trim_end(),
            "blob 按 git add 的规则转成了 LF"
        ),
        DiskCheck::Refused(why) => panic!("应当能用：{why}"),
    }
}

#[test]
fn conflict_hunk_may_hold_anything_including_extra_lines() {
    // 冲突块里多出来的内容交给对比这一步给人看（设计 §7）
    let s = scene();
    s.disk(&lines(&[
        (2, "two-mine-and-svn\nan extra line inside the hunk"),
        (8, "eight-svn"),
    ]));
    assert!(matches!(s.check(), DiskCheck::Usable(_)));
}

// —— 不能用 ——

#[test]
fn markers_left_on_disk_are_refused() {
    let s = scene();
    s.disk(&lines(&[
        (2, "<<<<<<< .mine\ntwo-mine\n=======\ntwo-svn\n>>>>>>> .r3"),
        (8, "eight-svn"),
    ]));
    assert!(refused(s.check()).contains("冲突标记"));
}

#[test]
fn an_edit_outside_the_conflict_is_refused() {
    let s = scene();
    s.disk(&lines(&[
        (2, "two-mine-and-svn"),
        (5, "five-edited-after-merge"),
        (8, "eight-svn"),
    ]));
    assert!(refused(s.check()).contains("冲突处以外"));
}

#[test]
fn losing_svns_non_conflicting_change_is_refused() {
    // 第 8 行是 SVN 的改动、不在冲突块里：磁盘上没有它，等于把同事的改动撤掉了
    let s = scene();
    s.disk(&lines(&[(2, "two-mine-and-svn")]));
    assert!(refused(s.check()).contains("冲突处以外"));
}

#[test]
fn disk_equal_to_your_commit_is_refused() {
    let s = scene();
    s.disk(&lines(&[(2, "two-mine")]));
    assert!(refused(s.check()).contains("你提交里的版本"));
}

#[test]
fn disk_equal_to_the_svn_version_is_refused() {
    // 设计 §10 问题 1：第一版不支持
    let s = scene();
    s.disk(&lines(&[(2, "two-svn"), (8, "eight-svn")]));
    assert!(refused(s.check()).contains("SVN 的新版本"));
}

#[test]
fn two_commits_touching_the_file_are_refused() {
    let s = scene();
    std::fs::write(s.path().join(F), lines(&[(2, "two-mine"), (9, "nine")])).unwrap();
    git(s.path(), &["commit", "-qam", "C2"]);
    let head = git(s.path(), &["rev-parse", "HEAD"]);
    let s = Scene { head, ..s };
    s.disk(&merged());
    assert!(refused(s.check()).contains("2 个提交"));
}

#[test]
fn missing_file_on_disk_is_refused() {
    let s = scene();
    std::fs::remove_file(s.path().join(F)).unwrap();
    assert!(refused(s.check()).contains("磁盘上没有"));
}

#[test]
fn binary_content_is_refused() {
    let s = scene();
    s.disk("bin\0ary\n");
    assert!(refused(s.check()).contains("二进制"));
}

#[test]
fn modify_delete_is_not_a_text_conflict() {
    let s = scene();
    s.disk(&merged());
    let files = BTreeMap::from([(
        F.to_string(),
        vec![s.stages[0].clone(), s.stages[2].clone()],
    )]);
    let why = match check_disk(&s.repo(), &s.base, &s.head, &files).remove(F) {
        Some(c) => refused(c),
        None => panic!("没有结论"),
    };
    assert!(why.contains("SVN 上删除"), "{why}");
}

#[test]
fn different_file_modes_are_refused() {
    let mut s = scene();
    s.disk(&merged());
    s.stages[2].mode = "100755".to_string();
    assert!(refused(s.check()).contains("权限"));
}

// —— 纯函数 ——

const O: &[u8] = b"<<<<<<< X";
const C: &[u8] = b">>>>>>> Y";

fn tpl(parts: &[&str]) -> Vec<u8> {
    parts.concat().into_bytes()
}

#[test]
fn matcher_accepts_anything_in_place_of_each_hunk() {
    let t = tpl(&[
        "a\n",
        "<<<<<<< X\n",
        "o\n",
        "=======\n",
        "t\n",
        ">>>>>>> Y\n",
        "b\n",
    ]);
    assert_eq!(
        outside_hunks_match(&t, b"a\nb\n", O, C),
        Some(true),
        "冲突块解成空"
    );
    assert_eq!(outside_hunks_match(&t, b"a\nx\ny\nb\n", O, C), Some(true));
    assert_eq!(
        outside_hunks_match(&t, b"a\nx\n", O, C),
        Some(false),
        "结尾段丢了"
    );
    assert_eq!(
        outside_hunks_match(&t, b"z\nx\nb\n", O, C),
        Some(false),
        "开头段改了"
    );
}

#[test]
fn matcher_handles_hunks_at_the_edges_and_adjacent_ones() {
    let t = tpl(&[
        "<<<<<<< X\n",
        "o\n",
        ">>>>>>> Y\n",
        "<<<<<<< X\n",
        "p\n",
        ">>>>>>> Y\n",
        "m\n",
        "<<<<<<< X\n",
        "q\n",
        ">>>>>>> Y\n",
    ]);
    assert_eq!(outside_hunks_match(&t, b"1\n2\nm\n3\n", O, C), Some(true));
    assert_eq!(outside_hunks_match(&t, b"m\n", O, C), Some(true));
    assert_eq!(
        outside_hunks_match(&t, b"1\n2\n3\n", O, C),
        Some(false),
        "中间段 m 丢了"
    );
}

#[test]
fn matcher_finds_a_repeated_segment_further_down() {
    // 中间段 `end` 在冲突块的解法里也出现了一次：贪心最左匹配不能因此判错
    let t = tpl(&[
        "a\n",
        "<<<<<<< X\n",
        "o\n",
        ">>>>>>> Y\n",
        "end\n",
        "tail\n",
    ]);
    assert_eq!(
        outside_hunks_match(&t, b"a\nend\nmore\nend\ntail\n", O, C),
        Some(true)
    );
    assert_eq!(
        outside_hunks_match(&t, b"a\nend\nmore\nend\nTAIL\n", O, C),
        Some(false)
    );
}

#[test]
fn matcher_gives_up_on_a_template_without_hunks_or_unbalanced_markers() {
    assert_eq!(outside_hunks_match(b"a\nb\n", b"a\nb\n", O, C), None);
    assert_eq!(
        outside_hunks_match(b"a\n<<<<<<< X\no\n", b"a\no\n", O, C),
        None
    );
}

#[test]
fn marker_detection_needs_the_marker_at_line_start() {
    assert!(has_conflict_markers(b"x\n<<<<<<< .mine\ny\n"));
    assert!(has_conflict_markers(b">>>>>>> .r70760\n"));
    assert!(has_conflict_markers(b"<<<<<<<\n"));
    assert!(!has_conflict_markers(b"-- a <<<<<<< b\n"));
    assert!(
        !has_conflict_markers(b"=======\n"),
        "单独的分隔线在注释里很常见"
    );
    assert!(!has_conflict_markers(b"<<<<<<<<< nine\n"), "只认 7 个");
}
