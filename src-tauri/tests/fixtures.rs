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

/// 造一个 UU 冲突：HEAD 第 1 行是 `colleague1`，对方是 `patch-local`，共 6 行。
fn conflicted_repo(autocrlf: bool) -> TempDir {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    run_git(d, &["init", "-q", "-b", "main"]);
    run_git(d, &["config", "user.email", "t@t.com"]);
    run_git(d, &["config", "user.name", "T"]);
    run_git(
        d,
        &[
            "config",
            "core.autocrlf",
            if autocrlf { "true" } else { "false" },
        ],
    );
    let body = |first: &str| format!("{first}\nline2\nline3\nline4\nline5\nline6\n");
    fs::write(d.join("c.txt"), body("base1")).unwrap();
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    run_git(d, &["checkout", "-q", "-b", "feat"]);
    fs::write(d.join("c.txt"), body("patch-local")).unwrap();
    run_git(d, &["commit", "-qam", "theirs"]);
    run_git(d, &["checkout", "-q", "main"]);
    fs::write(d.join("c.txt"), body("colleague1")).unwrap();
    run_git(d, &["commit", "-qam", "ours"]);
    assert!(!try_git(d, &["merge", "feat"]), "merge 应当冲突");
    dir
}

/// 冲突文件在「本地改动」里出现的两个 scope：旧的 `workdir`，以及按分区比较之后
/// Unstaged 用的 `unstaged`（第 65 条）。两者对冲突文件都必须是 HEAD 对比工作区。
const CONFLICT_SCOPES: [&str; 2] = ["workdir", "unstaged"];

fn conflicted_diff_lines(dir: &std::path::Path, scope: &str) -> Vec<String> {
    let data = engine::diff::read_diff(dir.to_str().unwrap(), scope.into()).unwrap();
    let file = data
        .files
        .iter()
        .find(|f| f.path == "c.txt")
        .unwrap_or_else(|| panic!("scope {scope}：c.txt 不在 diff 里"));
    assert_eq!(file.status, "conflicted", "scope {scope}");
    file.hunks
        .iter()
        .flat_map(|h| h.lines.iter())
        .map(|l| format!("{}{}", l.origin, l.content))
        .collect()
}

/// spec §3 的实测复刻：解完冲突后显示的是 HEAD 对比工作区，不是 base，也不是 ours 对比 theirs。
#[test]
fn conflicted_file_diff_is_head_vs_workdir() {
    let dir = conflicted_repo(false);
    fs::write(
        dir.path().join("c.txt"),
        "colleague1 + patch-local（手工合并）\nline2\nline3\nline4\nline5 手改\nline6\n",
    )
    .unwrap();
    for scope in CONFLICT_SCOPES {
        let lines = conflicted_diff_lines(dir.path(), scope);
        for want in [
            "-colleague1",
            "+colleague1 + patch-local（手工合并）",
            "-line5",
            "+line5 手改",
        ] {
            assert!(
                lines.iter().any(|l| l == want),
                "scope {scope} 缺 {want}：{lines:?}"
            );
        }
    }
}

/// autocrlf 下工作区是 CRLF：只有真正改过的行出现在 diff 里，不能整份文件都算改动。
#[test]
fn conflicted_file_diff_applies_autocrlf() {
    let dir = conflicted_repo(true);
    fs::write(
        dir.path().join("c.txt"),
        "colleague1\r\nline2\r\nline3\r\nline4\r\nline5 手改\r\nline6\r\n",
    )
    .unwrap();
    for scope in CONFLICT_SCOPES {
        let lines = conflicted_diff_lines(dir.path(), scope);
        let changed: Vec<&String> = lines.iter().filter(|l| !l.starts_with(' ')).collect();
        assert_eq!(
            changed,
            ["-line5", "+line5 手改"],
            "scope {scope}：{lines:?}"
        );
    }
}

/// 用户在工作区里删掉了冲突文件：如实显示成整份删除，暂存前能看出来要暂存的是一次删除。
#[test]
fn conflicted_file_missing_in_workdir_shows_deletion() {
    let dir = conflicted_repo(false);
    fs::remove_file(dir.path().join("c.txt")).unwrap();
    for scope in CONFLICT_SCOPES {
        let lines = conflicted_diff_lines(dir.path(), scope);
        assert!(!lines.is_empty(), "scope {scope}");
        assert!(
            lines.iter().all(|l| l.starts_with('-')),
            "scope {scope}：{lines:?}"
        );
        assert!(
            lines.iter().any(|l| l == "-colleague1"),
            "scope {scope}：{lines:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 第 65 条：「本地改动」的 diff 按分区比较。
// Unstaged = 暂存区 对比 工作区（`unstaged`），Staged = HEAD 对比 暂存区（`staged`）。
// ---------------------------------------------------------------------------

/// 三行文件 `a.txt` 已提交的仓库。
fn three_line_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    run_git(d, &["init", "-q"]);
    run_git(d, &["config", "user.email", "t@t.com"]);
    run_git(d, &["config", "user.name", "T"]);
    run_git(d, &["config", "core.autocrlf", "false"]);
    fs::write(d.join("a.txt"), "line 1\nline 2\nline 3\n").unwrap();
    run_git(d, &["add", "."]);
    run_git(d, &["commit", "-qm", "base"]);
    dir
}

/// issue 第 65 条的复现：第 2 行改了并暂存，第 3 行又改了没暂存。
fn staged_and_unstaged_repo() -> TempDir {
    let dir = three_line_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "line 1\nline 2 staged\nline 3\n").unwrap();
    run_git(d, &["add", "a.txt"]);
    fs::write(d.join("a.txt"), "line 1\nline 2 staged\nline 3 workdir\n").unwrap();
    dir
}

/// 某个 scope 下某个文件的改动行（不含上下文行），找不到文件时返回 None。
fn changed_lines(dir: &std::path::Path, scope: &str, path: &str) -> Option<Vec<String>> {
    let data = engine::diff::read_diff(dir.to_str().unwrap(), scope.into()).unwrap();
    data.files.iter().find(|f| f.path == path).map(|f| {
        f.hunks
            .iter()
            .flat_map(|h| h.lines.iter())
            .filter(|l| l.origin != ' ')
            .map(|l| format!("{}{}", l.origin, l.content))
            .collect()
    })
}

fn diff_paths(dir: &std::path::Path, scope: &str) -> Vec<String> {
    let data = engine::diff::read_diff(dir.to_str().unwrap(), scope.into()).unwrap();
    let mut paths: Vec<String> = data.files.iter().map(|f| f.path.clone()).collect();
    paths.sort();
    paths
}

#[test]
fn unstaged_scope_compares_the_index_with_the_workdir() {
    let dir = staged_and_unstaged_repo();
    let lines = changed_lines(dir.path(), "unstaged", "a.txt").expect("a.txt");
    assert_eq!(lines, ["-line 3", "+line 3 workdir"]);
}

#[test]
fn staged_scope_compares_head_with_the_index() {
    let dir = staged_and_unstaged_repo();
    let lines = changed_lines(dir.path(), "staged", "a.txt").expect("a.txt");
    assert_eq!(lines, ["-line 2", "+line 2 staged"]);
}

/// 只暂存、工作区没再改：Staged 里有它，Unstaged 的 diff 里不该有。反过来同理。
#[test]
fn a_file_changed_on_one_side_only_appears_in_that_scope_only() {
    let dir = three_line_repo();
    let d = dir.path();
    fs::write(d.join("staged-only.txt"), "s\n").unwrap();
    run_git(d, &["add", "staged-only.txt"]);
    fs::write(d.join("a.txt"), "line 1\nline 2\nline 3 workdir\n").unwrap();

    assert_eq!(diff_paths(d, "staged"), ["staged-only.txt"]);
    assert_eq!(diff_paths(d, "unstaged"), ["a.txt"]);
}

/// 未跟踪文件只在 Unstaged，整份算新增。
#[test]
fn unstaged_scope_shows_untracked_files_as_whole_additions() {
    let dir = three_line_repo();
    fs::write(dir.path().join("new.txt"), "n1\nn2\n").unwrap();
    let data = engine::diff::read_diff(dir.path().to_str().unwrap(), "unstaged".into()).unwrap();
    let file = data
        .files
        .iter()
        .find(|f| f.path == "new.txt")
        .expect("new.txt");
    assert_eq!(file.status, "added");
    assert_eq!(
        changed_lines(dir.path(), "unstaged", "new.txt").unwrap(),
        ["+n1", "+n2"]
    );
    assert!(changed_lines(dir.path(), "staged", "new.txt").is_none());
}

/// 暂存了修改之后又在工作区删掉：Staged 是那次修改，Unstaged 是一次整份删除。
#[test]
fn a_staged_change_deleted_in_the_workdir_shows_on_both_sides() {
    let dir = three_line_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "line 1\nline 2 staged\nline 3\n").unwrap();
    run_git(d, &["add", "a.txt"]);
    fs::remove_file(d.join("a.txt")).unwrap();

    assert_eq!(
        changed_lines(d, "staged", "a.txt").unwrap(),
        ["-line 2", "+line 2 staged"]
    );
    let data = engine::diff::read_diff(d.to_str().unwrap(), "unstaged".into()).unwrap();
    let file = data
        .files
        .iter()
        .find(|f| f.path == "a.txt")
        .expect("a.txt");
    assert_eq!(file.status, "deleted");
    assert_eq!(
        changed_lines(d, "unstaged", "a.txt").unwrap(),
        ["-line 1", "-line 2 staged", "-line 3"],
        "删除要对着暂存区那一版，不是 HEAD 那一版"
    );
}

/// 还没有任何提交的仓库：HEAD 不存在，Staged 对着空树比。
#[test]
fn staged_scope_works_before_the_first_commit() {
    let dir = TempDir::new().unwrap();
    let d = dir.path();
    run_git(d, &["init", "-q"]);
    fs::write(d.join("first.txt"), "hello\n").unwrap();
    run_git(d, &["add", "first.txt"]);

    let data = engine::diff::read_diff(d.to_str().unwrap(), "staged".into()).unwrap();
    let file = data
        .files
        .iter()
        .find(|f| f.path == "first.txt")
        .expect("first.txt");
    assert_eq!(file.status, "added");
    assert_eq!(changed_lines(d, "staged", "first.txt").unwrap(), ["+hello"]);
    assert!(diff_paths(d, "unstaged").is_empty());
}

/// Staged 列表里的每一项，在 staged diff 里都必须找得到同名的条目——
/// 否则点到它时右侧只会显示「No changes」。改名是最容易对不上的形态：
/// 列表按 `read_status` 出，diff 若单独做改名检测，两边的路径就不一样了。
#[test]
fn every_path_in_the_staged_list_has_a_staged_diff_entry() {
    let dir = three_line_repo();
    let d = dir.path();
    run_git(d, &["mv", "a.txt", "b.txt"]);
    fs::write(d.join("added.txt"), "x\n").unwrap();
    run_git(d, &["add", "added.txt"]);

    let status = engine::diff::read_status(d.to_str().unwrap()).unwrap();
    let mut listed: Vec<String> = status.staged.iter().map(|f| f.path.clone()).collect();
    listed.sort();
    assert!(!listed.is_empty());
    assert_eq!(diff_paths(d, "staged"), listed);
}

/// 同上，Unstaged 那一侧（不含冲突文件，冲突由上面那组用例管）。
#[test]
fn every_path_in_the_unstaged_list_has_an_unstaged_diff_entry() {
    let dir = three_line_repo();
    let d = dir.path();
    fs::write(d.join("a.txt"), "changed\n").unwrap();
    fs::write(d.join("untracked.txt"), "u\n").unwrap();
    fs::create_dir_all(d.join("dir")).unwrap();
    fs::write(d.join("dir").join("nested.txt"), "n\n").unwrap();

    let status = engine::diff::read_status(d.to_str().unwrap()).unwrap();
    let mut listed: Vec<String> = status.unstaged.iter().map(|f| f.path.clone()).collect();
    listed.sort();
    assert_eq!(listed.len(), 3, "{listed:?}");
    assert_eq!(diff_paths(d, "unstaged"), listed);
}

/// 合并冲突中（AGENTS.md §8）：`read_status` 只把冲突文件放进 Unstaged，
/// staged diff 不能因为 index 里的冲突条目而失败，也不能多出 Staged 列表里没有的项。
#[test]
fn staged_scope_during_a_merge_conflict_matches_the_staged_list() {
    let dir = conflicted_repo(false);
    let d = dir.path();
    // 冲突之外再暂存一个普通改动，确认 Staged 那一侧照常有内容
    fs::write(d.join("other.txt"), "o\n").unwrap();
    run_git(d, &["add", "other.txt"]);

    let status = engine::diff::read_status(d.to_str().unwrap()).unwrap();
    let mut listed: Vec<String> = status.staged.iter().map(|f| f.path.clone()).collect();
    listed.sort();
    assert_eq!(listed, ["other.txt"]);
    assert_eq!(diff_paths(d, "staged"), listed);
}

/// 有坏链接时要用 pathspec 把能读的路径逐个列出来重建 diff。路径里的 `[` `*` 不能被当成
/// 通配符：`f[1].txt` 当模式会同时匹配到 `f1.txt`——而 `f1.txt` 恰好就是那个要绕开的坏链接，
/// 被重新拉回来之后整个 diff 又会中止。
#[test]
fn rebuilding_around_a_dangling_link_matches_paths_literally() {
    let dir = linear_repo();
    let d = dir.path();
    fs::write(d.join("f[1].txt"), "new\n").unwrap();
    if !make_dangling_link(&d.join("f1.txt")) {
        eprintln!("跳过：本机无法创建目录联接");
        return;
    }
    for scope in ["workdir", "unstaged"] {
        let data = engine::diff::read_diff(d.to_str().unwrap(), scope.into())
            .unwrap_or_else(|e| panic!("scope {scope}：{}", e.message));
        let paths: Vec<&str> = data.files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"f[1].txt"), "scope {scope}：{paths:?}");
        assert_eq!(
            changed_lines(d, scope, "f[1].txt"),
            Some(vec!["+new".to_string()]),
            "scope {scope}：f[1].txt 的内容要读得出来"
        );
    }
}

#[test]
fn unstaged_scope_survives_a_dangling_link_in_the_workdir() {
    let dir = linear_repo();
    fs::write(dir.path().join("real.txt"), "changed\n").unwrap();
    if !make_dangling_link(&dir.path().join("broken-link")) {
        eprintln!("跳过：本机无法创建目录联接");
        return;
    }
    let data = engine::diff::read_diff(dir.path().to_str().unwrap(), "unstaged".into())
        .expect("坏的链接不应该让整个 read_diff 失败");
    assert!(
        data.files.iter().any(|f| f.path == "real.txt"),
        "real.txt 的 diff 不见了：{:?}",
        data.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    // staged 不读工作区，坏链接与它无关，但也不能因此失败
    engine::diff::read_diff(dir.path().to_str().unwrap(), "staged".into())
        .expect("staged 不该被工作区里的坏链接影响");
}
