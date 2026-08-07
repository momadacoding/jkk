use app_lib::commands::worktree as cmd;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

/// 主仓库 + 一个位于 `<主仓库>/.worktrees/<name>` 的 worktree，
/// 复刻用户实际的目录布局。
fn repo_with_worktree(name: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let main = dir.path().join("main");
    fs::create_dir_all(&main).unwrap();
    run_git(&main, &["init"]);
    run_git(&main, &["config", "user.email", "test@test.com"]);
    run_git(&main, &["config", "user.name", "Test"]);
    fs::write(main.join("a.txt"), "1").unwrap();
    run_git(&main, &["add", "."]);
    run_git(&main, &["commit", "-m", "c1"]);

    let wt = main.join(".worktrees").join(name);
    run_git(&main, &["worktree", "add", wt.to_str().unwrap(), "-b", name]);
    (dir, wt)
}

fn main_of(wt: &std::path::Path) -> std::path::PathBuf {
    wt.parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
fn delete_removes_a_clean_worktree() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .unwrap();

    assert!(!wt.exists(), "worktree directory should be gone");
    let out = Command::new("git").current_dir(&main).args(["worktree", "list"]).output().unwrap();
    let listing = String::from_utf8_lossy(&out.stdout);
    assert!(!listing.contains("feature"), "still listed: {listing}");
}

#[test]
fn delete_surfaces_the_real_git_error() {
    // 界面只显示我们返回的 message。自造一句 "worktree remove failed" 等于把
    // git 唯一说清楚原因的那句话扔了——用户只能靠猜。
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("scratch.txt"), "untracked").unwrap();

    let err = cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .expect_err("git 会拒绝删除有未跟踪文件的 worktree");

    assert_eq!(err.code, "git_cli");
    assert!(
        err.message.contains("contains modified or untracked files"),
        "拿到的是 git 的原文吗？实际是：{}",
        err.message,
    );
    // 干净的拒绝：什么都不该被删
    assert!(wt.join("a.txt").exists());
    // 这是「什么都没动」的失败，别把它说成孤儿目录吓唬人
    assert_ne!(err.code, "worktree_partial");
    assert!(!err.message.contains("手工删除"), "{}", err.message);
}

/// 登记还在的失败不该被改写。
///
/// `git worktree remove` 分两步：先解除登记、再删目录。第一步就被拒（有未提交
/// 改动）和第二步栽了（目录没删干净、登记已经没了）是两种完全不同的处境，前者
/// 原地重试就行，后者留下的目录已经不受 git 管了。混为一谈的话，要么白白吓唬
/// 用户，要么让人看不出目录已经成了孤儿。
#[test]
fn a_failure_before_unregistering_keeps_the_worktree_usable() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("scratch.txt"), "untracked").unwrap();

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .expect_err("有未跟踪文件时会被拒");

    // 登记还在，worktree 还能正常用
    let out = Command::new("git")
        .current_dir(&main)
        .args(["worktree", "list"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("feature"),
        "登记不该消失"
    );
}

/// 造一个 junction（Windows 上 pnpm 就是用它连 node_modules 的）。
#[cfg(windows)]
fn junction(link: &std::path::Path, target: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    // mklink 是 cmd 的内建命令，整条命令要当成一个字符串交给 /c
    let out = Command::new("cmd")
        .arg("/c")
        .raw_arg(format!(
            "mklink /J \"{}\" \"{}\"",
            link.display(),
            target.display()
        ))
        .output()
        .expect("cmd 起不来");
    assert!(
        out.status.success(),
        "mklink 失败: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
}

/// 复刻 pnpm 的结构：目标目录带个点，排序在指向它的链接前面。
///
/// git 递归删除时会先删掉目标，等走到 junction 时目标已经不在，它处理不了这种
/// 死链接就整个放弃（`remove_dir_recurse` 遇到第一个失败就 break）。加 `--force`
/// 也一样——那只跳过「工作区是否干净」的检查，删除路径完全相同。
#[cfg(windows)]
fn worktree_with_pnpm_style_links(name: &str) -> (TempDir, std::path::PathBuf) {
    let (dir, wt) = repo_with_worktree(name);
    let nm = wt.join("nm");
    let store = nm.join(".store").join("pkg");
    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("index.js"), "1").unwrap();
    junction(&nm.join("zlib"), &store);
    // 不加 .gitignore 的话 git 会因为「有未跟踪文件」在解除登记之前就拒绝，
    // 那就走不到我们要测的那一步了
    fs::write(wt.join(".gitignore"), "nm/\n").unwrap();
    let main = main_of(&wt);
    run_git(&main, &["-C", wt.to_str().unwrap(), "add", ".gitignore"]);
    run_git(&main, &["-C", wt.to_str().unwrap(), "commit", "-m", "ignore nm"]);
    (dir, wt)
}

/// git 删到一半放弃时，我们接手把目录删完。
#[cfg(windows)]
#[test]
fn cleans_up_after_git_gives_up_on_pnpm_junctions() {
    let (_dir, wt) = worktree_with_pnpm_style_links("pnpmish");
    let main = main_of(&wt);

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .expect("git 会栽在死链接上，但我们该接手删完");

    assert!(!wt.exists(), "目录应该被清干净，实际还在：{wt:?}");
    let out = Command::new("git")
        .current_dir(&main)
        .args(["worktree", "list"])
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("pnpmish"));
}

/// 接手删除**永远碰不到未提交的东西**——这条是整个回退方案的安全前提。
///
/// 道理在于顺序：git 的「工作区是否干净」检查排在解除登记之前，而我们只在
/// 「登记已经没了」时才接手。所以工作区一脏，git 在动任何东西之前就拒绝了，
/// 回退代码根本没机会运行。
///
/// 这里把两个条件叠在一起测：既有必然让 git 删目录失败的死链接，工作区又是脏的。
/// 要是哪天检查顺序变了、或者我们放宽了接手条件，这个测试会立刻炸。
#[cfg(windows)]
#[test]
fn cleanup_never_runs_while_there_is_uncommitted_work() {
    let (_dir, wt) = worktree_with_pnpm_style_links("dirtylink");
    let main = main_of(&wt);
    fs::write(wt.join("a.txt"), "脏数据，绝不能丢").unwrap();

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .expect_err("工作区是脏的，应当被拒绝");

    assert!(wt.exists(), "worktree 不该被删");
    assert_eq!(
        fs::read_to_string(wt.join("a.txt")).unwrap(),
        "脏数据，绝不能丢",
        "未提交的改动被动过了"
    );
    let out = Command::new("git")
        .current_dir(&main)
        .args(["worktree", "list"])
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("dirtylink"),
        "登记不该被解除"
    );
}

/// 删掉 worktree 不会动里面做的提交——提交对象在共享的对象库里，
/// 分支引用也在共享的 refs 里，worktree 目录只是一份检出。
#[test]
fn commits_made_in_the_worktree_survive_its_deletion() {
    let (_dir, wt) = repo_with_worktree("hasWork");
    let main = main_of(&wt);
    fs::write(wt.join("work.txt"), "在 worktree 里做的活").unwrap();
    run_git(&main, &["-C", wt.to_str().unwrap(), "add", "work.txt"]);
    run_git(&main, &["-C", wt.to_str().unwrap(), "commit", "-m", "wt commit"]);

    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
    )
    .unwrap();

    // 分支还在，内容还找得回来
    let out = Command::new("git")
        .current_dir(&main)
        .args(["show", "hasWork:work.txt"])
        .output()
        .unwrap();
    assert!(out.status.success(), "分支上的提交该还在");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "在 worktree 里做的活"
    );
}

/// 接手删除依赖的那条不变量：**解链接，不跟进目标**。
///
/// node_modules 里几千个 junction，跟进去删等于把链接指向的东西一并抹掉——
/// 这一条破了就是丢数据，不是体验问题。
///
/// 这里直接测 `remove_dir_all` 本身，不经过 `delete_worktree`。原因是没法让 git
/// 稳定地在碰到外部链接**之前**失败：它的目录枚举顺序不受我们控制，实测它会跳过
/// 删不掉的死链接继续处理后面的条目。既然把安全性完全押在这个标准库函数上，就
/// 直接把它钉住。
///
/// 顺带记一笔：**`git worktree remove` 会跟进 junction 把目标里的东西删掉**。
/// 单独验过——一个指向外部目录的 junction，git 删完 worktree 之后外部目录里的
/// 文件就没了。pnpm 的链接都指向 node_modules 内部所以害不到人，但这正是我们
/// 接手时绝不能照抄 git 做法的理由。
#[cfg(windows)]
#[test]
fn our_cleanup_unlinks_junctions_instead_of_following_them() {
    let dir = TempDir::new().unwrap();

    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("important.txt"), "不能被删掉").unwrap();

    let victim = dir.path().join("victim");
    fs::create_dir_all(victim.join("nm")).unwrap();
    junction(&victim.join("nm").join("external"), &outside);

    fs::remove_dir_all(&victim).unwrap();

    assert!(!victim.exists());
    assert!(
        outside.join("important.txt").exists(),
        "链接指向的外部数据被删掉了"
    );
}

/// 死链接（目标已经没了的 junction）也要能删掉——git 正是栽在这上面。
#[cfg(windows)]
#[test]
fn our_cleanup_handles_dead_junctions() {
    let dir = TempDir::new().unwrap();
    let nm = dir.path().join("nm");
    let store = nm.join(".store").join("pkg");
    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("index.js"), "1").unwrap();
    junction(&nm.join("zlib"), &store);
    fs::remove_dir_all(nm.join(".store")).unwrap(); // 制造死链接

    fs::remove_dir_all(&nm).expect("含死链接的目录也该删得掉");
    assert!(!nm.exists());
}

#[test]
fn create_puts_the_worktree_under_dot_worktrees() {
    let (_dir, wt) = repo_with_worktree("first");
    let main = main_of(&wt);

    cmd::create_worktree(main.to_str().unwrap().to_string(), "second".to_string(), None).unwrap();

    let created = main.join(".worktrees").join("second");
    assert!(created.exists(), "应该建在 .worktrees 下面，实际没有：{created:?}");
    assert!(created.join("a.txt").exists(), "worktree 里应该有仓库内容");
}

#[test]
fn create_surfaces_the_real_git_error() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    // 同名再建一次，git 会说分支已被占用
    let err = cmd::create_worktree(main.to_str().unwrap().to_string(), "feature".to_string(), None)
        .expect_err("重名的 worktree 应该失败");
    assert_eq!(err.code, "git_cli");
    assert!(!err.message.is_empty());
    assert!(
        !err.message.contains("worktree add failed"),
        "这是我们自造的占位文案，不是 git 的原文：{}",
        err.message,
    );
}
