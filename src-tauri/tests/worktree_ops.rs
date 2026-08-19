use app_lib::commands::worktree as cmd;
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
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// 盘上真实拼写。TempDir 给的是调用方拼写：CI runner 的 %TEMP% 带 8.3 短名
///（`C:\Users\RUNNER~1\...`），而 git 写登记/gitdir 一律归一成盘上真实拼写
///（实测：从别名路径进仓库，gitdir 里落的也是长名）。登记反查是精确匹配，
/// 两种拼写对不上会被当成「找不到登记」拒删。生产里入参来自 git 自己的输出，
/// 天然同族；测试要复刻这种形态，fixture 根目录必须先归一。
///
/// canonicalize 在 Windows 上返回 `\\?\` verbatim 前缀，git CLI 不认，剥掉；
/// UNC（`\\?\UNC\...`）不能剥，保持原样。
fn on_disk_spelling(path: &std::path::Path) -> std::path::PathBuf {
    let canon = path.canonicalize().unwrap();
    let s = canon.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => std::path::PathBuf::from(rest),
        _ => canon,
    }
}

/// 主仓库 + 一个位于 `<主仓库>/.worktrees/<name>` 的 worktree，
/// 复刻用户实际的目录布局。
fn repo_with_worktree(name: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let main = on_disk_spelling(dir.path()).join("main");
    fs::create_dir_all(&main).unwrap();
    run_git(&main, &["init"]);
    run_git(&main, &["config", "user.email", "test@test.com"]);
    run_git(&main, &["config", "user.name", "Test"]);
    fs::write(main.join("a.txt"), "1").unwrap();
    run_git(&main, &["add", "."]);
    run_git(&main, &["commit", "-m", "c1"]);

    let wt = main.join(".worktrees").join(name);
    run_git(
        &main,
        &["worktree", "add", wt.to_str().unwrap(), "-b", name],
    );
    (dir, wt)
}

fn main_of(wt: &std::path::Path) -> std::path::PathBuf {
    wt.parent().unwrap().parent().unwrap().to_path_buf()
}

fn delete(main: &std::path::Path, wt: &std::path::Path, force: bool) -> cmd::DeletedWorktree {
    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
        force,
    )
    .expect("删除应当成功")
}

fn try_delete(
    main: &std::path::Path,
    wt: &std::path::Path,
    force: bool,
) -> app_lib::error::GitError {
    cmd::delete_worktree(
        main.to_str().unwrap().to_string(),
        wt.to_str().unwrap().to_string(),
        force,
    )
    .expect_err("这一步应当被拒绝")
}

#[test]
fn delete_moves_a_clean_worktree_out_of_the_way() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    let deleted = delete(&main, &wt, false);

    assert!(!wt.exists(), "原位置该空出来");
    assert!(
        !git_out(&main, &["worktree", "list"]).contains("feature"),
        "登记该没了"
    );
    // 东西进了回收站，不是被抹掉
    let slot = std::path::Path::new(&deleted.trash_path);
    assert!(
        slot.join("meta.json").exists(),
        "返回给前端的必须是已落名的正式槽位（正式标记就是 meta.json）"
    );
    assert!(
        slot.join("worktree").join("a.txt").exists(),
        "文件该在回收站里"
    );
    assert!(slot.join("admin").exists(), "管理目录也该一起进回收站");
}

/// 删掉之后同名分支要能立刻重新建 worktree。
///
/// 这一条排除的是「只搬工作树、把登记留成 prunable」那种做法：那样 git 会说
/// `'feature' is already used by worktree at ...`，用户删了马上重建就撞墙。
#[test]
fn the_branch_can_be_checked_out_again_right_after_deletion() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    delete(&main, &wt, false);

    let again = main.join(".worktrees").join("again");
    run_git(
        &main,
        &["worktree", "add", again.to_str().unwrap(), "feature"],
    );
    assert!(again.join("a.txt").exists());
}

/// 脏的 worktree 默认拦下来，而且**什么都不动**。
#[test]
fn a_dirty_worktree_is_refused_and_left_untouched() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("scratch.txt"), "untracked").unwrap();

    let err = try_delete(&main, &wt, false);

    assert_eq!(err.code, "worktree_dirty");
    // 干净的拒绝：目录、内容、登记全都原样
    assert!(wt.join("a.txt").exists());
    assert_eq!(
        fs::read_to_string(wt.join("scratch.txt")).unwrap(),
        "untracked"
    );
    assert!(
        git_out(&main, &["worktree", "list"]).contains("feature"),
        "登记不该被解除"
    );
    // 也不该留下空的回收站槽位
    let trash = main.join(".worktrees").join(".trash-v2");
    assert!(
        !trash.exists() || fs::read_dir(&trash).unwrap().count() == 0,
        "被拒绝的删除不该在回收站里留垃圾"
    );
}

/// 用户确认之后（force）脏的也能删，而且未提交的东西完整躺在回收站里。
#[test]
fn force_deletes_a_dirty_worktree_but_keeps_the_work_in_the_trash() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("scratch.txt"), "别弄丢我").unwrap();

    let deleted = delete(&main, &wt, true);

    assert!(!wt.exists());
    let saved = std::path::Path::new(&deleted.trash_path)
        .join("worktree")
        .join("scratch.txt");
    assert_eq!(
        fs::read_to_string(&saved).unwrap(),
        "别弄丢我",
        "未提交的东西必须原样留在回收站里"
    );
}

/// 被 `git worktree lock` 锁住的一律拒绝，force 也不放行。
///
/// 锁的判据在登记里而不在目录里。要是不预检就先搬目录，git 照样会因为锁拒绝，
/// 那时候目录已经进回收站了——这个测试钉住的就是「预检必须排在搬移前面」。
#[test]
fn a_locked_worktree_is_refused_even_with_force() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    run_git(
        &main,
        &[
            "worktree",
            "lock",
            wt.to_str().unwrap(),
            "--reason",
            "别删我",
        ],
    );

    for force in [false, true] {
        let err = try_delete(&main, &wt, force);
        assert_eq!(err.code, "worktree_locked", "force={force}");
        assert!(
            err.message.contains("别删我"),
            "该带上锁的理由：{}",
            err.message
        );
        assert!(wt.exists(), "force={force} 时目录被动了");
    }
}

/// 同一个目录换个写法，锁照样要拦住。
///
/// 锁检查比的是两个**来源不同**的字符串：`git worktree list` 报的路径，和调用方
/// 传进来的路径。只要它们指向同一个目录但写法不同，纯字符串比较就判成两个不同的
/// worktree，锁被漏过去，删除照常执行——**保护是往不安全的方向失效的**。
///
/// 这不是假想：GitHub Windows runner 的 TEMP 是 8.3 短名（`C:\Users\RUNNER~1\...`），
/// git 报的是展开后的长名，两边对不上，`a_locked_worktree_is_refused_even_with_force`
/// 在 CI 上就是这么红的。真实机器上 junction、subst 盘符、8.3 短名都能触发。
///
/// 这里用 junction 造别名：它和短名走的是同一条失效路径（两个字符串、一个目录），
/// 而且不需要提权、在本机就能复现。
#[cfg(windows)]
#[test]
fn the_lock_holds_even_when_the_path_is_spelled_differently() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    run_git(
        &main,
        &[
            "worktree",
            "lock",
            wt.to_str().unwrap(),
            "--reason",
            "别删我",
        ],
    );

    // main/alias -> main/.worktrees，于是 main/alias/feature 和 wt 是同一个目录
    let link = main.join("alias");
    junction(&link, &main.join(".worktrees"));
    let aliased = link.join("feature");
    assert!(aliased.join("a.txt").exists(), "别名路径该能读到同一份文件");

    for force in [false, true] {
        let err = cmd::delete_worktree(
            main.to_str().unwrap().to_string(),
            aliased.to_str().unwrap().to_string(),
            force,
        )
        .expect_err("锁住的 worktree 换个写法也必须被拒绝");
        assert_eq!(err.code, "worktree_locked", "force={force}");
        assert!(wt.exists(), "force={force} 时目录被动了");
    }
}

/// 取路径的 8.3 短名（`C:\Users\RUNNER~1\...`）。
///
/// 卷上可以关掉 8.3 名字生成，那种机器上 `%~sI` 原样吐回长名。返回 `None` 让用例
/// 显式跳过，而不是拿「短名==长名」凑一次没测到东西的假绿灯。
#[cfg(windows)]
fn short_path(p: &std::path::Path) -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = Command::new("cmd")
        .arg("/c")
        .raw_arg(format!("for %I in (\"{}\") do @echo %~sI", p.display()))
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() || s.eq_ignore_ascii_case(&p.display().to_string()) {
        return None;
    }
    Some(s)
}

/// CI 上那次红灯的原样复现：调用方拿到的是 8.3 短名，git 报的是展开后的长名。
///
/// `the_lock_holds_even_when_the_path_is_spelled_differently` 用 junction 覆盖的是
/// 同一条失效路径，但短名才是 GitHub Windows runner 上的实际形态——两个都留着，
/// junction 那个保证任何机器都跑得到，这个保证真实事故形态被钉住。
#[cfg(windows)]
#[test]
fn the_lock_holds_when_the_caller_has_the_short_path() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    run_git(
        &main,
        &[
            "worktree",
            "lock",
            wt.to_str().unwrap(),
            "--reason",
            "别删我",
        ],
    );

    let Some(short) = short_path(&wt) else {
        eprintln!("这个卷关了 8.3 名字生成，跳过");
        return;
    };
    assert!(
        !short.eq_ignore_ascii_case(wt.to_str().unwrap()),
        "短名该和长名不同，否则这个用例什么都没测：{short}"
    );

    let err = cmd::delete_worktree(main.to_str().unwrap().to_string(), short.clone(), true)
        .expect_err("拿短名来删，锁一样要拦住");
    assert_eq!(err.code, "worktree_locked");
    assert!(wt.exists(), "目录被动了");
}

/// 主工作区一票否决——这一条破了就是把整个仓库的 `.git` 搬进回收站。
///
/// 原先这道保护是 git 给的（`fatal: '.' is a main working tree`）。改成由我们搬移
/// 之后 git 不再参与，保护必须自己补上。界面上禁着按钮不算数：那拦不住直接调命令。
#[test]
fn the_main_worktree_can_never_be_deleted() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    for force in [false, true] {
        let err = try_delete(&main, &main, force);
        assert_eq!(err.code, "worktree_is_main", "force={force}");
    }

    // 仓库还是完好的
    assert!(main.join(".git").exists(), "仓库的 .git 被动了");
    assert!(main.join("a.txt").exists());
    assert!(
        git_out(&main, &["worktree", "list"]).contains("feature"),
        "别的 worktree 也不该受影响"
    );
    let trash = main.join(".worktrees").join(".trash-v2");
    assert!(
        !trash.exists() || fs::read_dir(&trash).unwrap().count() == 0,
        "不该在回收站里留下任何东西"
    );
}

/// 工作树和管理目录必须**成对**搬进回收站。
///
/// index 存在管理目录 `.git/worktrees/<name>/index` 里，不在工作树目录内。只搬
/// 工作树的话暂存状态会丢，实测过。撤销功能删掉之后这条不变量反而更要紧：
/// 回收站里那份副本必须是完整的，用户手工捞回去才捞得全。
#[test]
fn the_worktree_and_its_admin_dir_move_together() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);
    fs::write(wt.join("s.txt"), "staged").unwrap();
    run_git(&main, &["-C", wt.to_str().unwrap(), "add", "s.txt"]);
    let before = git_out(&wt, &["status", "--porcelain"]);
    assert!(before.contains("A  s.txt"), "前提没造对：{before}");

    let deleted = delete(&main, &wt, true);

    let slot = std::path::Path::new(&deleted.trash_path);
    assert!(
        slot.join("worktree").join("s.txt").exists(),
        "工作树该整个进回收站"
    );
    assert!(
        slot.join("admin").join("index").exists(),
        "暂存区存在管理目录的 index 里，必须跟着一起进回收站"
    );
}

/// 往 worktree 里塞 n 个小文件，模拟装了依赖的 node_modules。
fn stuff(dir: &std::path::Path, n: usize) {
    for i in 0..n {
        let d = dir.join(format!("pkg{}", i / 100));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join(format!("f{i}.js")), "1").unwrap();
    }
}

/// 回收站条目数收敛到上限之内（清理在后台跑，给足等待时间）。
fn wait_trash_converged(trash: &std::path::Path) -> usize {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let kept = fs::read_dir(trash).unwrap().count();
        if kept <= 5 || std::time::Instant::now() >= deadline {
            return kept;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// 回收站只留固定个数，旧的自动清掉，否则装了 node_modules 的工作树会把盘吃光。
#[test]
fn the_trash_keeps_only_the_most_recent_entries() {
    let (_dir, wt) = repo_with_worktree("w0");
    let main = main_of(&wt);
    delete(&main, &wt, false);
    for i in 1..8 {
        let name = format!("w{i}");
        let path = main.join(".worktrees").join(&name);
        run_git(
            &main,
            &["worktree", "add", path.to_str().unwrap(), "-b", &name],
        );
        delete(&main, &path, false);
    }

    let trash = main.join(".worktrees").join(".trash-v2");
    let kept = wait_trash_converged(&trash);
    assert!(kept <= 5, "回收站该只留最近 5 个，实际留了 {kept} 个");
    assert!(kept > 0, "不该清空");
}

/// 删除本身必须毫秒级返回，哪怕这次删除触发了对旧槽位的清理。
///
/// 回归背景：清理曾在删除命令里同步执行，而清掉一个装了依赖的工作树是
/// 分钟级操作（实测 10 万文件要两分多钟），用户的「删除」点击会替之前的
/// 回收站买单。改成后台清理后，删除的耗时与回收站里躺了什么无关。
///
/// 时限取 2 秒：同步清理 2 万文件实测 6 秒以上，毫秒级的搬移离时限有一个
/// 数量级的余量，正常负载下不会抖红。
#[test]
fn deleting_returns_fast_even_when_the_trash_needs_pruning() {
    let (_dir, wt) = repo_with_worktree("w0");
    let main = main_of(&wt);
    // 第一个槽位塞成大树，它就是第 6 次删除时该被清掉的 victim
    stuff(&wt.join("node_modules"), 20_000);
    delete(&main, &wt, true);
    for i in 1..5 {
        let name = format!("w{i}");
        let path = main.join(".worktrees").join(&name);
        run_git(
            &main,
            &["worktree", "add", path.to_str().unwrap(), "-b", &name],
        );
        delete(&main, &path, false);
    }

    let name = "w5".to_string();
    let path = main.join(".worktrees").join(&name);
    run_git(
        &main,
        &["worktree", "add", path.to_str().unwrap(), "-b", &name],
    );
    let started = std::time::Instant::now();
    delete(&main, &path, false);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "删除被回收站清理拖慢了：{:?}",
        started.elapsed()
    );

    // 后台清理最终要把 victim 清掉，磁盘不会漏
    let trash = main.join(".worktrees").join(".trash-v2");
    let kept = wait_trash_converged(&trash);
    assert!(kept <= 5, "后台清理该把回收站收敛到 5 个，实际 {kept} 个");
}

/// 删掉 worktree 不会动里面做的提交——提交对象在共享的对象库里，
/// 分支引用也在共享的 refs 里，worktree 目录只是一份检出。
#[test]
fn commits_made_in_the_worktree_survive_its_deletion() {
    let (_dir, wt) = repo_with_worktree("hasWork");
    let main = main_of(&wt);
    fs::write(wt.join("work.txt"), "在 worktree 里做的活").unwrap();
    run_git(&main, &["-C", wt.to_str().unwrap(), "add", "work.txt"]);
    run_git(
        &main,
        &["-C", wt.to_str().unwrap(), "commit", "-m", "wt commit"],
    );

    delete(&main, &wt, false);

    let out = git_out(&main, &["show", "hasWork:work.txt"]);
    assert_eq!(out.trim(), "在 worktree 里做的活", "分支上的提交该还在");
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

/// **删除绝不能跟进 junction 去动链接指向的地方。**
///
/// 这是整个改造最重要的安全收益。原先走 `git worktree remove`，而 git **会**跟进
/// junction 把目标里的东西删掉（改造前单独验证过）；worktree 里只要有一个指向仓库外
/// 的链接，删 worktree 就会连累外面的数据。
///
/// 改成搬移之后 git 不再碰任何文件，rename 只动目录项本身，这条通道就断了。
#[cfg(windows)]
#[test]
fn deleting_never_touches_what_a_junction_points_at() {
    let (dir, wt) = repo_with_worktree("linky");
    let main = main_of(&wt);

    let outside = dir.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("important.txt"),
        "仓库外的数据，跟进删就是事故",
    )
    .unwrap();

    let nm = wt.join("nm");
    fs::create_dir_all(&nm).unwrap();
    junction(&nm.join("external"), &outside);
    fs::write(wt.join(".gitignore"), "nm/\n").unwrap();
    run_git(&main, &["-C", wt.to_str().unwrap(), "add", ".gitignore"]);
    run_git(
        &main,
        &["-C", wt.to_str().unwrap(), "commit", "-m", "ignore nm"],
    );

    delete(&main, &wt, false);

    assert!(!wt.exists(), "worktree 该被搬走");
    assert!(
        outside.join("important.txt").exists(),
        "链接指向的外部数据被删掉了——这正是改造要消灭的事故"
    );
}

/// 目录已经丢了（prunable）的 worktree 也必须删得掉，删完分支不再被登记挡着。
///
/// 真实形态：用户或别的工具手动删过目录，只剩登记。这种状态下 git 照样拒绝
/// `branch -D`（实测），界面删分支勾上「连 worktree 一起删」时就得能收拾它。
/// 此时盘上没有要保护的用户数据，但登记本身仍进回收站，不直接抹掉。
#[test]
fn a_worktree_whose_directory_is_gone_can_still_be_deleted() {
    let (_dir, wt) = repo_with_worktree("gone");
    let main = main_of(&wt);
    fs::remove_dir_all(&wt).unwrap();

    let deleted = delete(&main, &wt, false);

    assert!(
        !git_out(&main, &["worktree", "list"]).contains("gone"),
        "登记该清掉，branch -D 不再被挡"
    );
    run_git(&main, &["branch", "-D", "gone"]);
    // 槽位里只有管理目录，没有工作树部分
    let slot = std::path::Path::new(&deleted.trash_path);
    assert!(slot.join("admin").exists(), "管理目录该进回收站");
    assert!(!slot.join("worktree").exists(), "丢了的工作树变不出来");
}

/// 同一目录可以有两种拼写：8.3 短名（`RUNNER~1`）和长名。CI runner 的 %TEMP%
/// 就带短名，上面那个「目录已丢」的用例曾在那里红过——登记反查是精确匹配，
/// 别名拼写必须**拒绝**（over-refusal 是安全方向，绝不猜着去搬别人的登记）；
/// 归一回盘上真实拼写（= git 登记的拼写）后，同一个 worktree 必须删得掉。
#[cfg(windows)]
#[test]
fn a_gone_worktree_refuses_an_aliased_spelling_of_its_path() {
    let (_dir, wt) = repo_with_worktree("gone");
    let main = main_of(&wt);
    // 别名必须在目录还在盘上时取；删掉之后 8.3 解析无从谈起
    let aliased = short_spelling(&wt);
    if aliased == wt {
        eprintln!("跳过：本机临时卷未启用 8.3 短名");
        return;
    }
    fs::remove_dir_all(&wt).unwrap();

    let err = try_delete(&main, &aliased, false);
    assert_eq!(
        err.code, "worktree_not_registered",
        "别名拼写必须按「找不到登记」拒绝，不能折叠匹配：{} / {}",
        err.code, err.message
    );

    // 归一成盘上真实拼写后（fixture 已经归一），同一个 worktree 删得掉
    delete(&main, &wt, false);
    assert!(
        !git_out(&main, &["worktree", "list"]).contains("gone"),
        "登记该清掉"
    );
}

/// 盘上路径的 8.3 短名拼写。卷上没启用 8.3 时返回原样（调用方据此跳过）。
#[cfg(windows)]
fn short_spelling(path: &std::path::Path) -> std::path::PathBuf {
    use std::os::windows::process::CommandExt;
    // 必须 raw_arg：Rust 会把内嵌引号转义成 \"，cmd 不认反斜杠转义，
    // 多出来的反斜杠会让路径变成非法名（os error 123）
    let out = Command::new("cmd")
        .arg("/c")
        .raw_arg(format!("for %I in (\"{}\") do @echo %~sI", path.display()))
        .output()
        .expect("cmd 起不来");
    assert!(out.status.success());
    std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
}

/// pnpm 那种「目标带点、排序在链接前面」的布局，原先会让 git 递归删除栽在死链接上、
/// 删一半放弃。改成搬移之后根本不存在「删一半」这个状态。
#[cfg(windows)]
#[test]
fn a_pnpm_style_worktree_moves_in_one_piece() {
    let (_dir, wt) = repo_with_worktree("pnpmish");
    let main = main_of(&wt);
    let nm = wt.join("nm");
    let store = nm.join(".store").join("pkg");
    fs::create_dir_all(&store).unwrap();
    fs::write(store.join("index.js"), "1").unwrap();
    junction(&nm.join("zlib"), &store);
    fs::write(wt.join(".gitignore"), "nm/\n").unwrap();
    run_git(&main, &["-C", wt.to_str().unwrap(), "add", ".gitignore"]);
    run_git(
        &main,
        &["-C", wt.to_str().unwrap(), "commit", "-m", "ignore nm"],
    );

    let deleted = delete(&main, &wt, false);

    assert!(!wt.exists(), "原位置该空出来");
    let moved = std::path::Path::new(&deleted.trash_path).join("worktree");
    assert!(
        moved
            .join("nm")
            .join(".store")
            .join("pkg")
            .join("index.js")
            .exists(),
        "整棵树该原样搬过去"
    );
}

/// 回收站清理依赖的那条不变量：**解链接，不跟进目标**。
///
/// 槽位里存的是原样搬进来的工作树，node_modules 里几千个 junction，跟进去删等于
/// 把链接指向的东西一并抹掉——这一条破了就是丢数据，不是体验问题。
///
/// 直接测 `remove_dir_all` 本身：既然把安全性完全押在这个标准库函数上，就直接把它钉住。
#[cfg(windows)]
#[test]
fn trash_pruning_unlinks_junctions_instead_of_following_them() {
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
fn trash_pruning_handles_dead_junctions() {
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

    cmd::create_worktree(
        main.to_str().unwrap().to_string(),
        "second".to_string(),
        None,
    )
    .unwrap();

    let created = main.join(".worktrees").join("second");
    assert!(
        created.exists(),
        "应该建在 .worktrees 下面，实际没有：{created:?}"
    );
    assert!(created.join("a.txt").exists(), "worktree 里应该有仓库内容");
}

#[test]
fn create_surfaces_the_real_git_error() {
    let (_dir, wt) = repo_with_worktree("feature");
    let main = main_of(&wt);

    // 同名再建一次，git 会说分支已被占用
    let err = cmd::create_worktree(
        main.to_str().unwrap().to_string(),
        "feature".to_string(),
        None,
    )
    .expect_err("重名的 worktree 应该失败");
    assert_eq!(err.code, "git_cli");
    assert!(!err.message.is_empty());
    assert!(
        !err.message.contains("worktree add failed"),
        "这是我们自造的占位文案，不是 git 的原文：{}",
        err.message,
    );
}
