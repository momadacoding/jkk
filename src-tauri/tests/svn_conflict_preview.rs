#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 跟进冲突框的预览（事故分析 6.5，`svn_follow::preview_of`）。纯字符串处理，不需要仓库。

use app_lib::engine::svn_follow::{preview_of, PREVIEW_LINE_CHARS, PREVIEW_SIDE_LINES};

#[test]
fn labels_are_rewritten_and_both_sides_shown() {
    let text = "a\n<<<<<<< 1f2e3d\nsvn\n=======\nmine\n>>>>>>> 9a8b7c\nz\n";
    assert_eq!(
        preview_of(text).as_deref(),
        Some("<<<<<<< 新基线（SVN）\nsvn\n=======\nmine\n>>>>>>> 你的提交")
    );
}

#[test]
fn a_long_svn_side_does_not_hide_my_side() {
    // review M1：按总行数截断时，SVN 那一侧一长，「你的提交」就整段看不到了
    let ours: String = (0..20).map(|i| format!("svn{i}\n")).collect();
    let text = format!("<<<<<<< x\n{ours}=======\nmine\n>>>>>>> y\n");
    let p = preview_of(&text).unwrap();
    let lines: Vec<&str> = p.lines().collect();
    assert_eq!(lines.len(), 1 + PREVIEW_SIDE_LINES + 1 + 1 + 1 + 1, "{p}");
    assert!(
        p.contains(&format!("…（还有 {} 行）", 20 - PREVIEW_SIDE_LINES)),
        "{p}"
    );
    assert!(p.contains("=======\nmine\n>>>>>>> 你的提交"), "{p}");
}

#[test]
fn diff3_base_section_is_labelled_and_crlf_is_stripped() {
    let text = "<<<<<<< x\r\nsvn\r\n||||||| base\r\nold\r\n=======\r\nmine\r\n>>>>>>> y\r\n";
    assert_eq!(
        preview_of(text).as_deref(),
        Some("<<<<<<< 新基线（SVN）\nsvn\n||||||| 共同的旧版本\nold\n=======\nmine\n>>>>>>> 你的提交")
    );
}

#[test]
fn very_long_lines_are_clipped() {
    let long = "x".repeat(PREVIEW_LINE_CHARS * 10);
    let text = format!("<<<<<<< a\n{long}\n=======\nb\n>>>>>>> c\n");
    let p = preview_of(&text).unwrap();
    let first = p.lines().nth(1).unwrap();
    assert_eq!(
        first.chars().count(),
        PREVIEW_LINE_CHARS + 1,
        "截断后加一个省略号"
    );
    assert!(first.ends_with('…'));
}

#[test]
fn no_markers_means_no_preview() {
    // 修改与删除撞上时冲突树里的文件没有冲突标记
    assert_eq!(preview_of("plain\ntext\n"), None);
    assert_eq!(preview_of(""), None);
}

#[test]
fn only_the_first_conflict_block_is_shown() {
    let text = "<<<<<<< a\n1\n=======\n2\n>>>>>>> b\nmid\n<<<<<<< a\n3\n=======\n4\n>>>>>>> b\n";
    let p = preview_of(text).unwrap();
    assert!(!p.contains('3') && !p.contains("mid"), "{p}");
}
