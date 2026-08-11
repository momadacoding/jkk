use app_lib::engine::branch;
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

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

/// bare 远程 + 一个工作仓库，全程走本地路径，不碰网络。
fn repo_with_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let work = TempDir::new().unwrap();
    run_git(work.path(), &["init"]);
    run_git(work.path(), &["config", "user.email", "test@test.com"]);
    run_git(work.path(), &["config", "user.name", "Test"]);
    commit(work.path(), "c1");
    run_git(work.path(), &["remote", "add", "origin", &url]);
    run_git(work.path(), &["push", "-u", "origin", "HEAD"]);

    (remote, work)
}

fn p(dir: &TempDir) -> String {
    dir.path().to_str().unwrap().to_string()
}

#[test]
fn reports_the_upstream_and_splits_off_the_remote_name() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/x"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["push", "-u", "origin", "feat/x"]);
    // 回到别的分支，否则删的是当前分支
    run_git(work.path(), &["checkout", "-q", "-"]);

    let info = branch::delete_info(&p(&work), "feat/x").unwrap();
    assert_eq!(info.upstream.as_deref(), Some("origin/feat/x"));
    // 远程名和分支名必须分开给：前端按第一个 / 去切会把带斜杠的分支名切错
    assert_eq!(info.remote.as_deref(), Some("origin"));
    assert_eq!(info.remote_branch.as_deref(), Some("feat/x"));
}

#[test]
fn finds_the_remote_branch_even_without_a_configured_upstream() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/y"]);
    commit(work.path(), "c2");
    // 不带 -u：这样推上去的分支没有配置上游，只看 upstream 会以为它没有远程分支，
    // 勾选框就不会出现
    run_git(work.path(), &["push", "origin", "feat/y"]);
    run_git(work.path(), &["checkout", "-q", "-"]);

    let info = branch::delete_info(&p(&work), "feat/y").unwrap();
    assert_eq!(info.upstream.as_deref(), Some("origin/feat/y"));
    assert_eq!(info.remote.as_deref(), Some("origin"));
    assert_eq!(info.remote_branch.as_deref(), Some("feat/y"));
}

#[test]
fn a_purely_local_branch_has_no_remote() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "local-only"]);

    let info = branch::delete_info(&p(&work), "local-only").unwrap();
    assert_eq!(info.upstream, None);
    assert_eq!(info.remote, None);
    assert_eq!(info.remote_branch, None);
    // 刚从 HEAD 开出来，没有任何独有提交
    assert_eq!(info.unmerged, 0);
}

#[test]
fn counts_commits_that_are_not_on_the_current_branch() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/z"]);
    commit(work.path(), "c2");
    commit(work.path(), "c3");
    run_git(work.path(), &["checkout", "-q", "-"]);

    // 这两个提交只在 feat/z 上，删掉分支就只能靠 reflog 找回
    let info = branch::delete_info(&p(&work), "feat/z").unwrap();
    assert_eq!(info.unmerged, 2);
}

/// squash merge 之后 `unmerged` 必然大于 0（提交对象换了新 SHA），但内容早就
/// 进主干了。界面靠 `content_merged` 把那句「只能靠 reflog 找回」降级成中性提示，
/// 否则 GitHub 每 squash 合并一个分支就要误报一次。
#[test]
fn a_squash_merged_branch_still_counts_commits_but_matches_in_content() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/squashed"]);
    commit(work.path(), "c2");
    commit(work.path(), "c3");
    run_git(work.path(), &["checkout", "-q", "-"]);
    run_git(work.path(), &["merge", "--squash", "feat/squashed"]);
    run_git(work.path(), &["commit", "-m", "squashed (#1)"]);

    let info = branch::delete_info(&p(&work), "feat/squashed").unwrap();
    // 数字本身是对的——那两个提交对象确实不是 HEAD 的祖先
    assert_eq!(info.unmerged, 2);
    // ……但一点东西都丢不了
    assert!(info.content_merged);
}

/// squash 合并**之后主干又往前走了**——最常见的情形，也是全树对比栽跟头的地方。
///
/// 拿两边整棵 tree 比的话，主干上那些无关的新提交会让 diff 非空，于是判定
/// 「没合并」，红色警告照样弹出来。真实仓库上复现过：一个已 squash 合并的分支
/// 全树 diff 报 14 个文件有差异，全是合并之后主干上的新工作。
#[test]
fn a_squash_merged_branch_is_still_recognised_after_the_trunk_moves_on() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/old"]);
    commit(work.path(), "c2");
    commit(work.path(), "c3");
    run_git(work.path(), &["checkout", "-q", "-"]);
    run_git(work.path(), &["merge", "--squash", "feat/old"]);
    run_git(work.path(), &["commit", "-m", "squashed feat/old (#1)"]);
    // 合并之后主干继续开发，还顺手删掉一个和这个分支无关的文件
    commit(work.path(), "unrelated");
    run_git(work.path(), &["rm", "-q", "c1.txt"]);
    run_git(work.path(), &["commit", "-m", "drop c1"]);

    let info = branch::delete_info(&p(&work), "feat/old").unwrap();
    assert_eq!(info.unmerged, 2);
    assert!(
        info.content_merged,
        "主干往前走了几步，不该把已合并的分支重新判成未合并"
    );
}

/// 主干后来又改了这个分支碰过的文件——快路径（只比分支碰过的路径）会失效，
/// 只有 patch-id 兜底认得出来。
#[test]
fn the_patchid_fallback_catches_it_when_the_trunk_edits_the_same_files() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/same"]);
    fs::write(work.path().join("shared.txt"), "from branch").unwrap();
    run_git(work.path(), &["add", "."]);
    run_git(work.path(), &["commit", "-m", "branch touches shared"]);
    run_git(work.path(), &["checkout", "-q", "-"]);
    run_git(work.path(), &["merge", "--squash", "feat/same"]);
    run_git(work.path(), &["commit", "-m", "squashed (#2)"]);
    // 主干把同一个文件又改了一遍，快路径这下比不出「一字不差」
    fs::write(work.path().join("shared.txt"), "trunk changed it later").unwrap();
    run_git(work.path(), &["commit", "-am", "trunk edits shared"]);

    let info = branch::delete_info(&p(&work), "feat/same").unwrap();
    assert_eq!(info.unmerged, 1);
    assert!(
        info.content_merged,
        "patch-id 应当在主干上找到那个 squash 提交"
    );
}

/// GitHub 上的真实形态：分支开出去之后、squash 合并**之前**，主干先合了另一个
/// PR，而那个 PR 动过同一个文件。
///
/// 上面那几个 squash 用例都是分叉后主干原地不动、立刻 `merge --squash`，于是
/// squash 提交的补丁和分支净补丁天然一模一样，patch-id 怎么算都对得上——测的
/// 不是 GitHub 的真实形态。真实情况下 squash 提交是相对**合并时的主干**算的，
/// 分支净补丁是相对**分叉点**算的，中间主干动过的那些行会变成两边不同的
/// **上下文行**，即使增删的行一字不差，整体 patch-id 也对不上。
///
/// 真实仓库上复现过：一个 33 文件的分支，32 个文件的补丁都能对上，只有 README
/// 因为上一个 PR 删掉了它的一行上下文而对不上，整体指纹就此作废，红色的
/// 「删除后只能靠 reflog 找回」照弹。
#[test]
fn a_squash_merge_is_recognised_when_the_trunk_moved_before_the_merge() {
    let (_remote, work) = repo_with_remote();
    let shared = work.path().join("shared.txt");
    fs::write(&shared, "l1\nl2\nl3\nl4\nl5\n").unwrap();
    run_git(work.path(), &["add", "."]);
    run_git(work.path(), &["commit", "-m", "add shared"]);

    // 分支从这里开出去，在文件末尾追加一行
    run_git(work.path(), &["checkout", "-q", "-b", "feat/gh"]);
    fs::write(&shared, "l1\nl2\nl3\nl4\nl5\nfrom branch\n").unwrap();
    run_git(work.path(), &["commit", "-am", "branch appends"]);

    // 主干先合了别的 PR，它改掉了 l3——那正是分支那个 hunk 的三行上下文之一
    run_git(work.path(), &["checkout", "-q", "-"]);
    fs::write(&shared, "l1\nl2\nl3-changed-by-another-pr\nl4\nl5\n").unwrap();
    run_git(work.path(), &["commit", "-am", "another PR (#1)"]);

    // 然后才 squash 合并本分支
    run_git(work.path(), &["merge", "--squash", "feat/gh"]);
    run_git(work.path(), &["commit", "-m", "squashed feat/gh (#2)"]);

    let info = branch::delete_info(&p(&work), "feat/gh").unwrap();
    assert_eq!(info.unmerged, 1);
    assert!(
        info.content_merged,
        "分叉之后主干动过同一个文件，变的只是上下文行，不该判成未合并"
    );
}

/// HEAD 停在别处时，仍然要拿主干来判。
///
/// 真实踩过：主仓库处于游离 HEAD，停在某个功能分支的 tip 上，那个 tip 落后
/// main 两个提交、正好缺了目标分支的 squash 合并提交，于是一个早就合并进 main
/// 的分支被判成「有 2 个提交没合并」。用 worktree 的人常年如此——主仓库的 HEAD
/// 停在哪儿基本是随机的。
#[test]
fn judges_against_the_trunk_even_when_head_is_detached_somewhere_else() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "-m", "main"]);
    run_git(work.path(), &["checkout", "-q", "-b", "feat/done"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["checkout", "-q", "main"]);
    run_git(work.path(), &["merge", "--squash", "feat/done"]);
    run_git(work.path(), &["commit", "-m", "squashed feat/done (#1)"]);
    // main 又往前走一步
    commit(work.path(), "later");

    // 把 HEAD 游离到合并之前的位置——那里当然看不到 feat/done 的活
    let before = std::process::Command::new("git")
        .current_dir(work.path())
        .args(["rev-parse", "main~2"])
        .output()
        .unwrap();
    let before = String::from_utf8(before.stdout).unwrap();
    run_git(work.path(), &["checkout", "-q", "--detach", before.trim()]);

    let info = branch::delete_info(&p(&work), "feat/done").unwrap();
    assert_eq!(info.compared_to, "main", "该拿主干比，不是拿游离的 HEAD 比");
    assert_eq!(info.unmerged, 1);
    assert!(
        info.content_merged,
        "HEAD 停在哪儿不该影响「这个分支合并了没有」"
    );
}

/// 报了几个未合并，就得说清楚是相对谁数的。
#[test]
fn says_which_ref_the_count_is_against() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "-m", "main"]);
    run_git(work.path(), &["checkout", "-q", "-b", "feat/wip"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["checkout", "-q", "main"]);

    let info = branch::delete_info(&p(&work), "feat/wip").unwrap();
    assert_eq!(info.unmerged, 1);
    assert_eq!(info.compared_to, "main");
}

/// 真的有未合并提交时不能降级，红色警告必须留着。
#[test]
fn a_branch_with_real_unmerged_work_is_not_excused() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/real"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["checkout", "-q", "-"]);

    let info = branch::delete_info(&p(&work), "feat/real").unwrap();
    assert_eq!(info.unmerged, 1);
    assert!(!info.content_merged);
}

/// squash 合并后又提交了新东西：旧的那些内容在主干上，新的不在，
/// 两者混在同一个 unmerged 数里，只有内容对比能把它们分开。
#[test]
fn commits_made_after_the_squash_merge_break_the_content_match() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/more"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["checkout", "-q", "-"]);
    run_git(work.path(), &["merge", "--squash", "feat/more"]);
    run_git(work.path(), &["commit", "-m", "squashed"]);
    run_git(work.path(), &["checkout", "-q", "feat/more"]);
    commit(work.path(), "c3");
    run_git(work.path(), &["checkout", "-q", "master"]);

    let info = branch::delete_info(&p(&work), "feat/more").unwrap();
    assert!(!info.content_merged, "c3 还没进主干，不能说内容一致");
}

#[test]
fn reports_the_worktree_that_has_the_branch_checked_out() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "feat/wt"]);
    let wt = work.path().join("wt-dir");
    run_git(
        work.path(),
        &["worktree", "add", wt.to_str().unwrap(), "feat/wt"],
    );

    let info = branch::delete_info(&p(&work), "feat/wt").unwrap();
    let target = info.worktree.expect("被 worktree 检出的分支要报出来");
    assert_eq!(target.name, "wt-dir");
    // 路径要给全的：`git worktree remove` 收路径，靠名字拼是拼不出来的
    assert!(target.path.ends_with("wt-dir"), "{}", target.path);
}

/// 这条断言的是这个功能存在的理由，不是锦上添花：分支被 worktree 占着时
/// `git branch -D` 直接拒绝，没有那个勾选框这类分支在界面上根本删不掉。
#[test]
fn git_refuses_to_delete_a_branch_a_worktree_has_checked_out() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "feat/busy"]);
    let wt = work.path().join("busy-dir");
    let wt_path = wt.to_str().unwrap();
    run_git(work.path(), &["worktree", "add", wt_path, "feat/busy"]);

    let refused = Command::new("git")
        .current_dir(work.path())
        .args(["branch", "-D", "feat/busy"])
        .status()
        .unwrap();
    assert!(!refused.success(), "git 应当拒绝删除被检出的分支");

    // 先把 worktree 摘掉，同一条命令就通了——界面走的正是这个顺序
    run_git(work.path(), &["worktree", "remove", wt_path]);
    run_git(work.path(), &["branch", "-D", "feat/busy"]);
}

/// 主工作区不给勾选框：它删不掉，给了也是死路。
#[test]
fn the_main_worktree_is_not_offered() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/here"]);

    let info = branch::delete_info(&p(&work), "feat/here").unwrap();
    assert_eq!(info.worktree, None);
}

#[test]
fn a_branch_without_a_worktree_reports_none() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["branch", "plain"]);

    let info = branch::delete_info(&p(&work), "plain").unwrap();
    assert_eq!(info.worktree, None);
}

#[test]
fn a_merged_branch_reports_nothing_unmerged() {
    let (_remote, work) = repo_with_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/merged"]);
    commit(work.path(), "c2");
    run_git(work.path(), &["checkout", "-q", "-"]);
    run_git(
        work.path(),
        &["merge", "--no-ff", "-m", "merge", "feat/merged"],
    );

    let info = branch::delete_info(&p(&work), "feat/merged").unwrap();
    assert_eq!(info.unmerged, 0);
}
