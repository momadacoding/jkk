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

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    // 关掉行尾转换，否则 checkout 出来的是 CRLF，测试就变成在测 autocrlf 了
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);
    fs::write(dir.path().join("tracked.txt"), "original\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    dir
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

fn git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 只有 `git init` 和身份配置，没有任何提交。
fn unborn_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    run_git(dir.path(), &["config", "core.autocrlf", "false"]);
    dir
}

#[test]
fn discards_a_worktree_modification() {
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();

    ops::discard(p(&dir), vec!["tracked.txt".into()], false).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "original\n"
    );
}

#[test]
fn discarding_unstaged_keeps_what_was_already_staged() {
    // Unstaged 区的 Discard 只该回退「工作区相对暂存区」的那部分。
    // 连暂存的改动一起抹掉是另一回事，那是 Staged 区的语义。
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "staged version\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);
    fs::write(dir.path().join("tracked.txt"), "further edits\n").unwrap();

    ops::discard(p(&dir), vec!["tracked.txt".into()], false).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "staged version\n"
    );
}

#[test]
fn discarding_staged_goes_all_the_way_back_to_head() {
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "staged version\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);

    ops::discard(p(&dir), vec!["tracked.txt".into()], true).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "original\n"
    );
    let out = Command::new("git")
        .current_dir(dir.path())
        .args(["diff", "--cached", "--name-only"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "暂存区应该也干净了"
    );
}

#[test]
fn deletes_an_untracked_file() {
    // git 里没有它的任何副本，除了删掉别无选择
    let dir = repo();
    fs::write(dir.path().join("scratch.png"), "junk").unwrap();

    ops::discard(p(&dir), vec!["scratch.png".into()], false).unwrap();

    assert!(!dir.path().join("scratch.png").exists());
}

#[test]
fn discarding_a_staged_new_file_removes_it_from_index_and_disk() {
    let dir = repo();
    fs::write(dir.path().join("fresh.txt"), "new\n").unwrap();
    run_git(dir.path(), &["add", "fresh.txt"]);

    ops::discard(p(&dir), vec!["fresh.txt".into()], true).unwrap();

    assert!(!dir.path().join("fresh.txt").exists());
    let out = Command::new("git")
        .current_dir(dir.path())
        .args(["diff", "--cached", "--name-only"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "索引里不该还留着它"
    );
}

#[test]
fn handles_a_mixed_selection_in_one_call() {
    // 多选之后一次 discard，三类文件混在一起是常态
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();
    fs::write(dir.path().join("scratch.png"), "junk").unwrap();
    fs::write(dir.path().join("second.txt"), "also new").unwrap();

    ops::discard(
        p(&dir),
        vec![
            "tracked.txt".into(),
            "scratch.png".into(),
            "second.txt".into(),
        ],
        false,
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "original\n"
    );
    assert!(!dir.path().join("scratch.png").exists());
    assert!(!dir.path().join("second.txt").exists());
}

#[test]
fn leaves_files_that_were_not_selected_alone() {
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();
    fs::write(dir.path().join("keep-me.txt"), "precious").unwrap();

    ops::discard(p(&dir), vec!["tracked.txt".into()], false).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("keep-me.txt")).unwrap(),
        "precious"
    );
}

#[test]
fn an_empty_selection_is_a_no_op() {
    // 空 pathspec 会被 git 当成「全部」，那就成了一次误伤全仓库的 discard
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "messed up\n").unwrap();

    ops::discard(p(&dir), vec![], false).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "messed up\n"
    );
}

#[test]
fn discards_a_file_inside_a_subdirectory() {
    let dir = repo();
    fs::create_dir_all(dir.path().join("src/deep")).unwrap();
    fs::write(dir.path().join("src/deep/a.txt"), "v1\n").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c2"]);
    fs::write(dir.path().join("src/deep/a.txt"), "broken\n").unwrap();

    ops::discard(p(&dir), vec!["src/deep/a.txt".into()], false).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("src/deep/a.txt")).unwrap(),
        "v1\n"
    );
}

#[test]
fn staged_discard_stops_instead_of_deleting_when_head_cannot_be_read() {
    // finding #1 的回归测试。
    //
    // 实测（2026-09-13）：删掉 HEAD 的 tree 对象之后，`git ls-tree -r HEAD` 报
    // `fatal: not a tree object` 退出 128，而 `git rm -f` 照样退出 0 并把文件删掉。
    // 旧实现把 ls-tree 的**任何**失败都当成「HEAD 里什么都没有」，于是一个明明在
    // HEAD 里、本该被恢复成 original 的文件，会被分进 rm 桶直接删除——用户点的是
    // 「丢弃改动」，拿到的是「文件没了」。
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "staged version\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);

    let tree = git_out(dir.path(), &["rev-parse", "HEAD^{tree}"]);
    let obj = dir
        .path()
        .join(".git/objects")
        .join(&tree[..2])
        .join(&tree[2..]);
    assert!(obj.exists(), "fixture 期望松散对象：{}", obj.display());
    fs::remove_file(&obj).unwrap();

    let err = ops::discard(p(&dir), vec!["tracked.txt".into()], true)
        .expect_err("HEAD 读不出来时必须报错，不能当成空 HEAD");

    // 关键断言：文件还在，而且一个字节都没动过
    assert!(
        dir.path().join("tracked.txt").exists(),
        "文件被删了——这正是 finding #1 的后果：{}",
        err.message
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "staged version\n",
        "分类都没做成就不该动任何文件"
    );
}

#[test]
fn staged_discard_stops_instead_of_deleting_when_the_branch_ref_is_corrupt() {
    // 同一个缺陷的第二种形态，由对抗性 review 指出（2026-09-13）。
    //
    // 上一条测试删的是 tree 对象，此时 `show-ref --verify` 仍然成功；这里损坏的是
    // **分支 ref 本身**（指向一个不存在的 OID），`show-ref --verify --quiet` 退出
    // 128 `bad ref`。只判「show-ref 有没有失败」的写法会把它当成「ref 不存在」，
    // 也就是空仓库，于是文件又落进 rm 桶——而 `git rm -f` 照样退出 0 并删掉它。
    //
    // 实测的退出码区分（这是判定必须看退出码而非成败的理由）：
    //   ref 文件不存在（真 unborn）→ 1
    //   ref 指向不存在的 OID       → 128
    let dir = repo();
    fs::write(dir.path().join("tracked.txt"), "staged version\n").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);

    let head_ref = git_out(dir.path(), &["symbolic-ref", "--quiet", "HEAD"]);
    fs::write(
        dir.path().join(".git").join(&head_ref),
        "0123456789012345678901234567890123456789",
    )
    .unwrap();

    let err = ops::discard(p(&dir), vec!["tracked.txt".into()], true)
        .expect_err("分支 ref 损坏时必须报错，不能当成空仓库");

    assert!(
        dir.path().join("tracked.txt").exists(),
        "文件被删了：{}",
        err.message
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("tracked.txt")).unwrap(),
        "staged version\n"
    );
    // 病根要说出来：`ls-tree` 只会报 `not a tree object`，对用户没有信息量
    assert!(
        err.message.contains(&head_ref),
        "错误里应指出是哪个 ref 坏了：{}",
        err.message
    );
}

#[test]
fn an_unborn_head_pointing_at_a_slash_prefixed_name_is_still_unborn() {
    // `for-each-ref refs/heads/svn` 会按路径前缀匹配到 `refs/heads/svn/base`，
    // 把「HEAD 指向的 ref 不存在」误判成「存在」。spec D10 的基线分支就叫
    // `svn/base`，所以这个形态在本项目里是会真实出现的，不是造出来的边界。
    // `show-ref --verify` 是精确匹配，实测退出 1，判定正确。
    let dir = repo();
    run_git(dir.path(), &["branch", "svn/base"]);
    run_git(dir.path(), &["symbolic-ref", "HEAD", "refs/heads/svn"]);
    fs::write(dir.path().join("fresh.txt"), "new\n").unwrap();
    run_git(dir.path(), &["add", "fresh.txt"]);

    // HEAD 确实没有提交，fresh.txt 也确实不在任何 HEAD 树里 → 应当照常删掉
    ops::discard(p(&dir), vec!["fresh.txt".into()], true).unwrap();

    assert!(!dir.path().join("fresh.txt").exists());
}

#[test]
fn staged_discard_of_a_new_file_still_works_in_a_repo_with_no_commits() {
    // 守住 finding #1 修复时最容易过度纠正的那条合法路径：
    // `git ls-tree HEAD` 在还没有任何提交的仓库里必然失败（实测退出 128
    // `Not a valid object name HEAD`），而那正是「HEAD 里什么都没有」的正常表达，
    // 不是错误。此时丢弃一个已暂存的新文件，应当把它从索引和磁盘一起去掉。
    let dir = unborn_repo();
    fs::write(dir.path().join("fresh.txt"), "new\n").unwrap();
    run_git(dir.path(), &["add", "fresh.txt"]);

    ops::discard(p(&dir), vec!["fresh.txt".into()], true).unwrap();

    assert!(!dir.path().join("fresh.txt").exists());
}

#[test]
fn a_failed_stage_says_what_was_already_done_and_what_was_not() {
    // finding #4 的回归测试。
    //
    // 三个阶段都不可回滚——`git checkout` 抹掉的工作区内容没有副本可取回——所以
    // 失败时唯一能补救的是让用户知道仓库停在哪。旧实现只把底层那条 git stderr
    // 抛上去，界面上看不出「前面的恢复是不是已经生效」「剩下的文件还在不在」。
    //
    // 用 `git checkout -- nonexistent.txt` 触发第一阶段失败（实测退出 1）。
    let dir = repo();
    fs::write(dir.path().join("scratch.png"), "junk").unwrap();

    let err = ops::discard(
        p(&dir),
        vec!["nonexistent.txt".into(), "scratch.png".into()],
        false,
    )
    .expect_err("第一阶段失败必须报错");

    assert_eq!(err.code, "discard_partial");
    assert!(
        err.message.contains("scratch.png"),
        "没被处理的文件必须说出来：{}",
        err.message
    );
    // 出错之后不再执行更不可逆的步骤
    assert!(
        dir.path().join("scratch.png").exists(),
        "第一阶段失败后不该继续删未跟踪文件"
    );
}

#[cfg(windows)]
#[test]
fn a_locked_untracked_file_reports_which_ones_were_already_deleted() {
    // 删未跟踪文件是逐个做的，所以这一阶段能精确说出卡在哪个文件上、
    // 前面哪些已经删掉了。模拟 Unity 抓着资源文件不放的情形。
    use std::os::windows::fs::OpenOptionsExt;

    let dir = repo();
    fs::write(dir.path().join("first.png"), "junk").unwrap();
    fs::write(dir.path().join("locked.png"), "junk").unwrap();
    // share_mode(0) 是完全独占，连删除都不允许；Rust 默认的打开方式带
    // FILE_SHARE_DELETE，那样文件照样能删掉，测不出东西来
    let _held = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(dir.path().join("locked.png"))
        .unwrap();

    let err = ops::discard(
        p(&dir),
        vec!["first.png".into(), "locked.png".into()],
        false,
    )
    .expect_err("锁住的文件删不掉，必须报错");

    assert_eq!(err.code, "discard_partial");
    assert!(
        err.message.contains("first.png"),
        "已经删掉的要说出来：{}",
        err.message
    );
    assert!(
        err.message.contains("locked.png"),
        "卡在哪个文件要说出来：{}",
        err.message
    );
    assert!(!dir.path().join("first.png").exists());
    assert!(dir.path().join("locked.png").exists());
}
