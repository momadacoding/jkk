use app_lib::commands::ops;
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

fn try_git(dir: &std::path::Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success()
}

fn out(dir: &std::path::Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(o.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

fn init(dir: &std::path::Path) {
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
    // 关掉行尾转换，否则 patch 里的内容会变成在测 autocrlf
    run_git(dir, &["config", "core.autocrlf", "false"]);
}

/// c1: a.txt = "1"，c2: a.txt = "2"。
fn linear_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    init(dir.path());
    fs::write(dir.path().join("a.txt"), "1\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2\n").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

// ---------- create_tag ----------

#[test]
fn tags_the_given_commit_not_head() {
    let dir = linear_repo();
    let c1 = out(dir.path(), &["rev-parse", "HEAD~1"]);

    ops::create_tag(p(&dir), "v0.1".into(), c1.clone()).unwrap();

    // 打在指定的那条上，不是当前 HEAD——右键菜单点的是某一行，不是「现在在哪」
    assert_eq!(out(dir.path(), &["rev-parse", "v0.1^{commit}"]), c1);
}

#[test]
fn refuses_to_move_an_existing_tag() {
    let dir = linear_repo();
    let c1 = out(dir.path(), &["rev-parse", "HEAD~1"]);
    let head = out(dir.path(), &["rev-parse", "HEAD"]);
    ops::create_tag(p(&dir), "v0.1".into(), c1.clone()).unwrap();

    // 同名 tag 必须报错。静默改写指向会把别人已经拉走的引用悄悄挪位置
    let err = ops::create_tag(p(&dir), "v0.1".into(), head);
    assert!(err.is_err(), "同名 tag 被静默改写了");
    assert_eq!(out(dir.path(), &["rev-parse", "v0.1^{commit}"]), c1);
}

// ---------- cherry_pick ----------

#[test]
fn cherry_picks_a_commit_from_another_branch() {
    let dir = linear_repo();
    run_git(dir.path(), &["checkout", "-q", "-b", "side"]);
    fs::write(dir.path().join("b.txt"), "from side\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-qm", "side change"]);
    let side = out(dir.path(), &["rev-parse", "HEAD"]);
    run_git(dir.path(), &["checkout", "-q", "-"]);
    assert!(!dir.path().join("b.txt").exists());
    // 先让 master 走出一步，两条分支才是真的分叉。
    // 实测：分叉点相同时 cherry-pick 出来的提交 tree/parent/author/date/message
    // 全都一样，git 给出的是**同一个 SHA**——那种场景下断言「产生了新提交」不成立。
    fs::write(dir.path().join("a.txt"), "3\n").unwrap();
    run_git(dir.path(), &["commit", "-qam", "c3"]);

    ops::cherry_pick(p(&dir), side.clone()).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "from side\n"
    );
    // 落在新的父提交上，所以是另一条提交，原提交仍留在 side 上
    assert_ne!(out(dir.path(), &["rev-parse", "HEAD"]), side);
    assert_eq!(
        out(dir.path(), &["log", "-1", "--format=%s"]),
        "side change"
    );
    // master 自己那步没有被踩掉
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "3\n");
}

#[test]
fn cherry_pick_conflict_surfaces_as_an_error() {
    let dir = linear_repo();
    run_git(dir.path(), &["checkout", "-q", "-b", "side"]);
    fs::write(dir.path().join("a.txt"), "side\n").unwrap();
    run_git(dir.path(), &["commit", "-qam", "side edit"]);
    let side = out(dir.path(), &["rev-parse", "HEAD"]);
    run_git(dir.path(), &["checkout", "-q", "-"]);
    fs::write(dir.path().join("a.txt"), "ours\n").unwrap();
    run_git(dir.path(), &["commit", "-qam", "our edit"]);

    // 冲突不能被吞掉：界面要能显示出来，否则用户看不出仓库已经停在 cherry-pick 中途
    assert!(
        ops::cherry_pick(p(&dir), side).is_err(),
        "冲突的 cherry-pick 报成了成功"
    );
}

// ---------- revert ----------

#[test]
fn reverts_a_commit_without_opening_an_editor() {
    let dir = linear_repo();
    let c2 = out(dir.path(), &["rev-parse", "HEAD"]);

    // 不带 --no-edit 时 git 会拉起编辑器等一个永远不会来的输入，GUI 里就是挂死。
    // 这个用例跑得完本身就是在验证那件事。
    ops::revert(p(&dir), c2).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1\n");
    // 是新增一条提交，不是把历史改掉
    assert_eq!(out(dir.path(), &["rev-list", "--count", "HEAD"]), "3");
}

// ---------- save_patch ----------

#[test]
fn writes_a_patch_that_git_am_can_replay() {
    let dir = linear_repo();
    let c2 = out(dir.path(), &["rev-parse", "HEAD"]);
    let store = TempDir::new().unwrap();
    let patch = store.path().join("c2.patch");

    ops::save_patch(p(&dir), c2, patch.to_str().unwrap().to_string()).unwrap();

    // 落到用户选的那个文件名上，而不是 format-patch 自动编号的名字
    assert!(patch.exists(), "patch 没写到指定路径");

    // 真正的验收标准是能被 git am 回放，而不是文件里有些什么字符串
    let target = TempDir::new().unwrap();
    init(target.path());
    fs::write(target.path().join("a.txt"), "1\n").unwrap();
    run_git(target.path(), &["add", "."]);
    run_git(target.path(), &["commit", "-qm", "c1"]);

    assert!(
        try_git(target.path(), &["am", patch.to_str().unwrap()]),
        "git am 打不上这个 patch"
    );
    assert_eq!(
        fs::read_to_string(target.path().join("a.txt")).unwrap(),
        "2\n"
    );
    // 作者和提交说明都要跟过来——这正是 format-patch 相对 `git show` 的意义
    assert_eq!(out(target.path(), &["log", "-1", "--format=%s"]), "c2");
    assert_eq!(
        out(target.path(), &["log", "-1", "--format=%ae"]),
        "test@test.com"
    );
}

#[test]
fn exports_only_the_selected_commit() {
    let dir = linear_repo();
    let c2 = out(dir.path(), &["rev-parse", "HEAD"]);
    let store = TempDir::new().unwrap();
    let patch = store.path().join("only.patch");

    ops::save_patch(p(&dir), c2, patch.to_str().unwrap().to_string()).unwrap();

    let text = fs::read_to_string(&patch).unwrap();
    // 不带 -1 的话 format-patch 会把这条提交的所有祖先一起导出来
    assert!(
        text.contains("Subject: [PATCH] c2"),
        "patch 里没有 c2: {text}"
    );
    assert!(
        !text.contains("Subject: [PATCH] c1"),
        "patch 里混进了祖先提交 c1"
    );
}

#[test]
fn save_patch_reports_a_bad_rev_instead_of_writing_an_empty_file() {
    let dir = linear_repo();
    let store = TempDir::new().unwrap();
    let patch = store.path().join("nope.patch");

    let result = ops::save_patch(
        p(&dir),
        "deadbeef".into(),
        patch.to_str().unwrap().to_string(),
    );

    assert!(result.is_err(), "无效的 rev 应当报错");
    assert!(!patch.exists(), "报错了却还是留下了一个空文件");
}
