use app_lib::engine;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success());
}

/// 允许失败的 git 调用（merge 冲突这类预期非零退出）。
fn try_git(dir: &std::path::Path, args: &[&str]) -> bool {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found")
        .success()
}

fn linear_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "a.txt"]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    fs::write(dir.path().join("a.txt"), "2").unwrap();
    run_git(dir.path(), &["commit", "-am", "c2"]);
    dir
}

#[test]
fn load_linear_graph() {
    let dir = linear_repo();
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(data.commits.len(), 2);
    assert_eq!(data.commits[0].parents.len(), 1); // c2 -> c1
    assert_eq!(data.commits[1].parents.len(), 0); // c1 root
}

#[test]
fn graph_includes_a_commit_reachable_only_by_tag() {
    // revwalk 只 push HEAD 和分支，所以一条「只被 tag 指向」的提交不会进图。
    // 侧边栏点这种 tag 会定位失败，图上也看不到它。
    let dir = linear_repo();
    fs::write(dir.path().join("tagged.txt"), "x").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "only-tagged"]);
    run_git(dir.path(), &["tag", "-a", "v9", "-m", "orphan"]);
    // 把分支退回去，让这条提交只剩 tag 指着它。
    // 只在本用例自建的临时 fixture 仓库里做，不碰任何真实仓库。
    run_git(dir.path(), &["reset", "--hard", "HEAD~1"]);

    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert!(
        data.commits.iter().any(|c| c.message == "only-tagged"),
        "只被 tag 指向的提交不在图里：{:?}",
        data.commits.iter().map(|c| &c.message).collect::<Vec<_>>()
    );
    // 而且这条 tag 的 hash 必须能在图里找到对应的行，否则前端仍然画不出 pill
    let tag = data
        .refs
        .iter()
        .find(|r| r.name == "refs/tags/v9")
        .expect("refs/tags/v9");
    assert!(data.commits.iter().any(|c| c.id == tag.hash));
}

#[test]
fn read_status_detects_unstaged() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(status.unstaged.len(), 1);
    assert_eq!(status.unstaged[0].status, "modified");
}

#[test]
fn read_diff_for_workdir() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    let diff =
        engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    assert_eq!(diff.files.len(), 1);
}

#[test]
fn read_status_counts_changed_lines() {
    let dir = linear_repo();
    // c2 里 a.txt 是单行 "2"，替换成三行 → 删 1 行、加 3 行
    fs::write(dir.path().join("a.txt"), "one\ntwo\nthree\n").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    let a = status
        .unstaged
        .iter()
        .find(|f| f.path == "a.txt")
        .expect("a.txt in unstaged");
    assert_eq!((a.additions, a.deletions), (3, 1));
}

#[test]
fn read_status_reports_untracked_as_added() {
    let dir = linear_repo();
    fs::write(dir.path().join("new.txt"), "x\ny\n").unwrap();
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    let new = status
        .unstaged
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("new.txt in unstaged");
    // 未跟踪文件曾经被 INDEX_* 判断误标成 modified、行数恒为 0
    assert_eq!(new.status, "added");
    assert_eq!((new.additions, new.deletions), (2, 0));
}

#[test]
fn read_status_counts_staged_lines_separately() {
    let dir = linear_repo();
    fs::write(dir.path().join("new.txt"), "x\ny\n").unwrap();
    run_git(dir.path(), &["add", "new.txt"]);
    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    let staged = status
        .staged
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("new.txt in staged");
    assert_eq!(staged.status, "added");
    assert_eq!((staged.additions, staged.deletions), (2, 0));
}

#[test]
fn read_status_lists_conflicted_files() {
    let dir = linear_repo();
    run_git(dir.path(), &["checkout", "-q", "-b", "other"]);
    fs::write(dir.path().join("conflict.txt"), "other side\n").unwrap();
    run_git(dir.path(), &["add", "conflict.txt"]);
    run_git(dir.path(), &["commit", "-qm", "other"]);
    run_git(dir.path(), &["checkout", "-q", "-"]);
    fs::write(dir.path().join("conflict.txt"), "this side\n").unwrap();
    run_git(dir.path(), &["add", "conflict.txt"]);
    run_git(dir.path(), &["commit", "-qm", "this"]);
    assert!(!try_git(dir.path(), &["merge", "other"]), "merge 应当冲突");

    let status = engine::diff::read_status(dir.path().to_str().unwrap()).unwrap();
    // 冲突文件既不在 INDEX_* 也不在 WT_* 里，曾经完全不出现在列表中
    let conflicted = status
        .unstaged
        .iter()
        .find(|f| f.path == "conflict.txt")
        .expect("conflict.txt in unstaged");
    assert_eq!(conflicted.status, "conflicted");
}

fn rev(dir: &std::path::Path, spec: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", spec])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `range:<a>..<b>`：两个任意 commit 之间的差异。SVN 节点用它比前后两版本地基线——
/// 交付录的基线以交付树为父（spec §8.2.2），`commit:<基线>` 比的是它和交付树，
/// 恰好把「这一版交付了什么」漏掉（2026-10-11 真实窗口：r2 / r3 看不到任何文件变化）。
#[test]
fn read_diff_for_a_range_spans_every_commit_in_between() {
    let dir = linear_repo();
    let c1 = rev(dir.path(), "HEAD~1");
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();
    run_git(dir.path(), &["add", "b.txt"]);
    run_git(dir.path(), &["commit", "-m", "c3"]);
    let c3 = rev(dir.path(), "HEAD");
    let path = dir.path().to_str().unwrap();

    let diff = engine::diff::read_diff(path, format!("range:{c1}..{c3}")).unwrap();
    let mut files: Vec<(String, String)> = diff
        .files
        .iter()
        .map(|f| (f.path.clone(), f.status.clone()))
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            ("a.txt".to_string(), "modified".to_string()),
            ("b.txt".to_string(), "added".to_string())
        ]
    );
    // 对照：commit:c3 只有它自己那一步
    let own = engine::diff::read_diff(path, format!("commit:{c3}")).unwrap();
    assert_eq!(own.files.len(), 1);
}

#[test]
fn a_malformed_range_is_an_error_not_a_workdir_diff() {
    // 解析不了时不能落到下面的「workdir」默认分支——那会把工作区改动当成这个节点的变化
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "dirty").unwrap();
    let path = dir.path().to_str().unwrap();
    let head = rev(dir.path(), "HEAD");
    for scope in [
        "range:".to_string(),
        format!("range:{head}"),
        format!("range:..{head}"),
        format!("range:{head}.."),
        "range:nothex..alsonot".to_string(),
    ] {
        assert!(
            engine::diff::read_diff(path, scope.clone()).is_err(),
            "{scope} 应报错"
        );
    }
}

#[test]
fn read_diff_includes_untracked_content() {
    let dir = linear_repo();
    fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
    let diff =
        engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    let file = diff
        .files
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("未跟踪文件也要有 diff");
    assert!(file.hunks.iter().any(|h| h
        .lines
        .iter()
        .any(|l| l.origin == '+' && l.content == "hello")));
}

#[test]
fn read_diff_excludes_header_lines() {
    let dir = linear_repo();
    fs::write(dir.path().join("a.txt"), "changed\n").unwrap();
    fs::write(dir.path().join("new.txt"), "added\n").unwrap();
    let diff =
        engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string()).unwrap();
    // libgit2 的文件头 'F' / hunk 头 'H' 曾经被当成内容行渲染出来
    for file in &diff.files {
        for hunk in &file.hunks {
            for line in &hunk.lines {
                assert!(
                    matches!(line.origin, '+' | '-' | ' '),
                    "{} 里出现了非内容行: {:?} {:?}",
                    file.path,
                    line.origin,
                    line.content
                );
            }
        }
    }
}

#[test]
fn create_and_read_worktree() {
    let dir = linear_repo();
    let wt_dir = dir.path().join("wt");
    let status = std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["worktree", "add", wt_dir.to_str().unwrap()])
        .status()
        .unwrap();
    assert!(status.success());
    let wts = engine::worktree::read_worktrees(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(wts.len(), 2);
    // 主工作区排第一且不可删；每项的分支名都要解析出来（曾经恒为空串）
    assert!(wts[0].is_main);
    assert!(!wts[1].is_main);
    assert!(!wts[0].branch.is_empty(), "主工作区分支名为空");
    assert_eq!(wts[1].branch, "wt");
    assert_eq!(wts[1].name, "wt");
}

#[test]
fn open_invalid_repo_fails() {
    let dir = TempDir::new().unwrap();
    let result = engine::repo::open(dir.path().to_str().unwrap());
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().code, "not_repo");
}

#[test]
fn open_repo_reports_both_head_name_and_hash() {
    let dir = linear_repo();
    let info = engine::repo::open(dir.path().to_str().unwrap()).unwrap();
    // head 仍是给人看的分支名
    assert!(
        info.head == "master" || info.head == "main",
        "head was {}",
        info.head
    );
    // head_hash 是完整 oid，且必须等于图里最新的那条提交——
    // 工作区伪行就是靠这个 hash 挂到 HEAD 上的
    assert_eq!(info.head_hash.len(), 40);
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(info.head_hash, data.commits[0].id);
}

/// 用固定时间戳提交，避免同一秒内多次提交导致排序断言不稳定。
fn commit_at(dir: &std::path::Path, name: &str, date: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    let status = Command::new("git")
        .current_dir(dir)
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .args(["commit", "-m", name])
        .status()
        .expect("git not found");
    assert!(status.success());
}

#[test]
fn graph_puts_newer_unmerged_tip_above_older_merge() {
    // 未合并的分支 tip 如果比主干上的合并提交更新，它必须排在合并提交前面。
    // 只用 Sort::TOPOLOGICAL 时，互不相关的提交之间顺序取决于 revwalk 的 push 顺序，
    // 这个 tip 会掉到合并提交下面——而合并提交连向第二父的线正好经过它所在的车道，
    // 于是那条线从它的圆点上穿过去，看上去它不是分支 tip 而是合并线的中转点。
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);

    commit_at(dir.path(), "base", "2026-01-01T00:00:00+00:00");
    run_git(dir.path(), &["checkout", "-b", "feature"]);
    commit_at(dir.path(), "f1", "2026-01-02T00:00:00+00:00");

    let main = String::from_utf8_lossy(
        &Command::new("git")
            .current_dir(dir.path())
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    assert_eq!(main, "feature");

    run_git(dir.path(), &["checkout", "-"]);
    // 合并提交：2026-01-03
    let status = Command::new("git")
        .current_dir(dir.path())
        .env("GIT_AUTHOR_DATE", "2026-01-03T00:00:00+00:00")
        .env("GIT_COMMITTER_DATE", "2026-01-03T00:00:00+00:00")
        .args(["merge", "--no-ff", "-m", "merge feature", "feature"])
        .status()
        .expect("git not found");
    assert!(status.success());

    // 分支上再提交一次，比合并提交更新：2026-01-04
    run_git(dir.path(), &["checkout", "feature"]);
    commit_at(dir.path(), "f2", "2026-01-04T00:00:00+00:00");

    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    let row = |msg: &str| {
        data.commits
            .iter()
            .position(|c| c.message == msg)
            .unwrap_or_else(|| {
                panic!(
                    "{msg} not found in {:?}",
                    data.commits.iter().map(|c| &c.message).collect::<Vec<_>>()
                )
            })
    };

    assert!(
        row("f2") < row("merge feature"),
        "更新的未合并 tip f2 排在了合并提交之后：{:?}",
        data.commits
            .iter()
            .map(|c| (&c.message, c.time))
            .collect::<Vec<_>>()
    );
    // 拓扑约束不能被破坏：父必须排在子之后
    assert!(row("merge feature") < row("f1"));
    assert!(row("f1") < row("base"));
}

/// 造一个目标不存在的链接。
///
/// Windows 上用**目录联接**（junction）而不是符号链接：后者需要
/// SeCreateSymbolicLinkPrivilege（开发者模式），普通账户下拿不到，
/// 用例会静默跳过、变成什么都不测的假绿灯。junction 不需要提权。
/// 先指向真实目录再把目标删掉，正好复现 pnpm store 里那种
/// 「链接还在、目标已被删」的悬空状态。
#[cfg(windows)]
fn make_dangling_link(link: &std::path::Path) -> bool {
    let target = link.with_file_name("junction-target");
    if fs::create_dir_all(&target).is_err() {
        return false;
    }
    let ok = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(&target)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok && fs::remove_dir_all(&target).is_ok()
}

#[cfg(not(windows))]
fn make_dangling_link(link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(link.with_file_name("nowhere-at-all"), link).is_ok()
}

#[test]
fn status_survives_a_dangling_link_in_the_workdir() {
    // 工作区里一个悬空符号链接会让 libgit2 在**加载内容**时报
    // "could not stat"，进而让整个 read_status 失败——一个坏路径就把整个
    // 变更列表弄瞎了。原生 git 遇到同样的情况只打一条 warning 然后继续
    // （实测：`warning: could not open directory ...: No such file or directory`）。
    // 我们要对齐 git 的行为：坏路径不该拖垮其余文件。
    let dir = linear_repo();
    fs::write(dir.path().join("real.txt"), "changed\n").unwrap();
    if !make_dangling_link(&dir.path().join("broken-link")) {
        eprintln!("跳过：本机无法创建目录联接");
        return;
    }

    let status = engine::diff::read_status(dir.path().to_str().unwrap())
        .expect("坏的链接不应该让整个 read_status 失败");
    // 正常文件仍然要出现在列表里
    assert!(
        status.unstaged.iter().any(|f| f.path == "real.txt"),
        "real.txt 不见了：{:?}",
        status.unstaged
    );
    // 读不了的未跟踪项不该出现——原生 git 实测只打一条
    // `warning: could not open directory 'broken-link/'` 而不列出它，
    // 但 libgit2 会把它报成 WT_NEW，界面上就多出一条 git 不认的变更。
    assert!(
        !status
            .unstaged
            .iter()
            .any(|f| f.path.contains("broken-link")),
        "悬空链接被当成未跟踪变更列出来了：{:?}",
        status.unstaged
    );
}

#[test]
fn read_diff_survives_a_dangling_link_in_the_workdir() {
    let dir = linear_repo();
    fs::write(dir.path().join("real.txt"), "changed\n").unwrap();
    if !make_dangling_link(&dir.path().join("broken-link")) {
        eprintln!("跳过：本机无法创建目录联接");
        return;
    }

    let diff = engine::diff::read_diff(dir.path().to_str().unwrap(), "workdir".to_string())
        .expect("坏的链接不应该让整个 read_diff 失败");
    assert!(
        diff.files.iter().any(|f| f.path == "real.txt"),
        "real.txt 的 diff 不见了：{:?}",
        diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}
