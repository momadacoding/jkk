#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
use app_lib::commands::stash as cmd;
use app_lib::engine::stash;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    dir
}

#[test]
fn list_is_empty_on_a_clean_repo() {
    let dir = repo();
    assert!(stash::list(dir.path().to_str().unwrap())
        .unwrap()
        .is_empty());
}

#[test]
fn push_then_list_reports_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();

    cmd::stash_push(p.clone(), Some("my work".into()), false).unwrap();
    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 0);
    assert!(
        entries[0].message.contains("my work"),
        "got {}",
        entries[0].message
    );
    // hash 要能直接喂给现成的 commit:<hash> diff scope，所以必须是完整 oid
    assert_eq!(entries[0].hash.len(), 40);
    // 工作区已经被还原
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
}

#[test]
fn entry_carries_enough_metadata_for_the_detail_panel() {
    // stash 提交不被任何分支引用，所以不在 load_graph 的输出里。详情面板靠
    // author/time/parents 渲染，这些字段拿不到就只能显示一个空面板——
    // 点了 stash 却什么都不出来。
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), Some("meta".into()), false).unwrap();

    let entries = stash::list(&p).unwrap();
    let e = &entries[0];
    assert_eq!(e.author, "Test");
    assert!(e.time > 0, "time was {}", e.time);
    // stash 是个 merge 提交，第一父就是 stash 时所在的那个提交
    assert!(!e.parents.is_empty());
    assert_eq!(e.parents[0], git_out(dir.path(), &["rev-parse", "HEAD"]));
}

#[test]
fn push_without_message_still_produces_an_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();

    cmd::stash_push(p.clone(), None, false).unwrap();
    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].message.is_empty());
}

#[test]
fn include_untracked_captures_new_files() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("new.txt"), "fresh").unwrap();

    cmd::stash_push(p.clone(), None, true).unwrap();
    assert!(
        !dir.path().join("new.txt").exists(),
        "untracked file was not stashed"
    );
    assert_eq!(stash::list(&p).unwrap().len(), 1);
}

#[test]
fn newest_stash_is_index_zero() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "first").unwrap();
    cmd::stash_push(p.clone(), Some("first".into()), false).unwrap();
    fs::write(dir.path().join("a.txt"), "second").unwrap();
    cmd::stash_push(p.clone(), Some("second".into()), false).unwrap();

    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(
        entries[0].message.contains("second"),
        "got {}",
        entries[0].message
    );
    assert_eq!(entries[0].index, 0);
    assert_eq!(entries[1].index, 1);
}

#[test]
fn pop_restores_changes_and_removes_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_pop(p.clone(), 0).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed"
    );
    assert!(stash::list(&p).unwrap().is_empty());
}

#[test]
fn apply_restores_changes_but_keeps_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_apply(p.clone(), 0).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed"
    );
    assert_eq!(stash::list(&p).unwrap().len(), 1);
}

#[test]
fn drop_removes_the_entry_without_touching_the_working_tree() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_drop(p.clone(), 0).unwrap();
    assert!(stash::list(&p).unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
}

/// 已跟踪改动 + 一个新增文件，用 `-u` 压进 stash，工作区回到干净。
fn stash_with_untracked(dir: &std::path::Path, rel: &str) -> String {
    let p = dir.to_str().unwrap().to_string();
    fs::write(dir.join("a.txt"), "changed").unwrap();
    let full = dir.join(rel);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(&full, "from stash").unwrap();
    cmd::stash_push(p.clone(), None, true).unwrap();
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "1");
    assert!(!full.exists());
    p
}

/// 撞名被拦下之后：stash 还在，已跟踪的改动**没有**写上去，挡路的文件原样。
/// git 自己撞名时会先写已跟踪部分、再在未跟踪上失败（2026-11-16 实测 git 2.51，半套 apply）。
fn assert_untouched(dir: &std::path::Path, p: &str, blocker: &str, blocker_content: &str) {
    assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "1");
    assert_eq!(
        fs::read_to_string(dir.join(blocker)).unwrap(),
        blocker_content
    );
    assert_eq!(stash::list(p).unwrap().len(), 1);
}

#[test]
fn pop_with_untracked_restores_new_files_too() {
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "new.txt");

    cmd::stash_pop(p.clone(), 0).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("new.txt")).unwrap(),
        "from stash"
    );
    assert!(stash::list(&p).unwrap().is_empty());
}

#[test]
fn pop_refuses_without_touching_anything_when_a_new_file_is_already_on_disk() {
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "new.txt");
    fs::write(dir.path().join("new.txt"), "mine").unwrap();

    let err = cmd::stash_pop(p.clone(), 0).unwrap_err();
    assert_eq!(err.code, "stash_untracked_exists");
    assert!(err.message.contains("new.txt"), "got {}", err.message);
    assert_untouched(dir.path(), &p, "new.txt", "mine");
}

#[test]
fn apply_refuses_without_touching_anything_when_a_new_file_is_already_on_disk() {
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "sub/new.txt");
    // `-u` 收走文件时连空下来的 `sub/` 一起删了
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/new.txt"), "mine").unwrap();

    let err = cmd::stash_apply(p.clone(), 0).unwrap_err();
    assert_eq!(err.code, "stash_untracked_exists");
    assert!(err.message.contains("sub/new.txt"), "got {}", err.message);
    assert_untouched(dir.path(), &p, "sub/new.txt", "mine");
}

#[test]
fn pop_refuses_when_a_file_sits_where_a_new_files_directory_should_be() {
    // `sub` 被一个普通文件占着：对 `sub/new.txt` 取元数据会报错、看着像「不存在」，
    // 但 git 建不出这一级目录，照样半套 apply
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "sub/new.txt");
    fs::remove_dir_all(dir.path().join("sub")).ok();
    fs::write(dir.path().join("sub"), "a file").unwrap();

    let err = cmd::stash_pop(p.clone(), 0).unwrap_err();
    assert_eq!(err.code, "stash_untracked_exists");
    assert!(err.message.contains("sub"), "got {}", err.message);
    assert_untouched(dir.path(), &p, "sub", "a file");
}

#[test]
fn the_check_looks_at_the_stash_being_applied_not_the_newest_one() {
    // stash@{1} 带新增文件且撞名，stash@{0} 只有已跟踪改动。查错条目（总看 {0}）的话
    // apply(1) 会放行、git 半套 apply
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "new.txt");
    fs::write(dir.path().join("b.txt"), "tracked later").unwrap();
    run_git(dir.path(), &["add", "b.txt"]);
    run_git(dir.path(), &["commit", "-qm", "c2"]);
    fs::write(dir.path().join("b.txt"), "changed b").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();
    fs::write(dir.path().join("new.txt"), "mine").unwrap();

    let err = cmd::stash_apply(p.clone(), 1).unwrap_err();
    assert_eq!(err.code, "stash_untracked_exists");
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "tracked later"
    );

    // {0} 没有新增文件：磁盘上的 new.txt 不挡它
    cmd::stash_apply(p.clone(), 0).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "changed b"
    );
    assert_eq!(stash::list(&p).unwrap().len(), 2);
}

#[test]
fn pop_refuses_when_a_directory_sits_at_a_new_files_path() {
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "new.txt");
    fs::create_dir_all(dir.path().join("new.txt")).unwrap();
    fs::write(dir.path().join("new.txt/inner.txt"), "mine").unwrap();

    let err = cmd::stash_pop(p.clone(), 0).unwrap_err();
    assert_eq!(err.code, "stash_untracked_exists");
    assert!(err.message.contains("new.txt"), "got {}", err.message);
    assert_untouched(dir.path(), &p, "new.txt/inner.txt", "mine");
}

#[test]
fn pop_goes_through_when_the_new_file_came_back_identical_and_tracked() {
    // 计划 A17 的形状：混合仓库里同事新增的文件被 `-u` 收进 stash，跟进之后它作为已跟踪文件、
    // 同样的字节回到磁盘。stash、HEAD、磁盘三处同一个 blob，不算撞名
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "new.txt");
    fs::write(dir.path().join("new.txt"), "from stash").unwrap();
    run_git(dir.path(), &["add", "new.txt"]);
    run_git(dir.path(), &["commit", "-qm", "colleague"]);

    cmd::stash_pop(p.clone(), 0).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("new.txt")).unwrap(),
        "from stash"
    );
    assert!(stash::list(&p).unwrap().is_empty());
    // new.txt 是已跟踪的、和 HEAD 一样：不该留下 ` D` 或 `??`
    let status = git_out(dir.path(), &["status", "--porcelain"]);
    assert_eq!(status, "M a.txt", "status: {status}");
}

#[test]
fn pop_still_refuses_when_the_tracked_copy_differs() {
    // 同名文件已被提交，但字节和 stash 里的不同：真撞名，照样拦
    let dir = repo();
    let p = stash_with_untracked(dir.path(), "new.txt");
    fs::write(dir.path().join("new.txt"), "colleague's version").unwrap();
    run_git(dir.path(), &["add", "new.txt"]);
    run_git(dir.path(), &["commit", "-qm", "colleague"]);

    let err = cmd::stash_pop(p.clone(), 0).unwrap_err();
    assert_eq!(err.code, "stash_untracked_exists");
    assert_untouched(dir.path(), &p, "new.txt", "colleague's version");
}

#[test]
fn pop_of_a_stash_without_untracked_ignores_unrelated_new_files() {
    // 没带 `-u` 的 stash 没有第三个父提交：磁盘上随便有什么新增文件都不挡
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();
    fs::write(dir.path().join("new.txt"), "mine").unwrap();

    cmd::stash_pop(p.clone(), 0).unwrap();
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "changed"
    );
}
