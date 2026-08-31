use app_lib::engine;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success());
}

/// 主仓库 + 一个 linked worktree，返回 (主仓库 TempDir, worktree 根)。
/// TempDir 必须在调用方作用域里持有，提前 drop 目录就没了。
fn repo_with_linked_worktree() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    std::fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "a.txt"]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    let wt = dir.path().join("linked");
    run_git(
        dir.path(),
        &["worktree", "add", wt.to_str().unwrap(), "-b", "linked"],
    );
    (dir, wt)
}

#[test]
fn watch_targets_cover_git_metadata_of_a_linked_worktree() {
    let (_dir, wt) = repo_with_linked_worktree();
    let repo = git2::Repository::open(&wt).unwrap();
    let targets = engine::watch::watch_targets(&wt, Some(&repo));

    // 工作区根目录必须监听（工作区文件变更）
    assert!(targets.contains(&wt), "缺少工作区根: {targets:?}");
    // 外部 commit 写 refs/heads/<branch>，落在主仓库的 commondir——
    // linked worktree 下它在工作区根目录之外，不显式加入就收不到 commit 事件
    let common = repo.commondir().to_path_buf();
    assert!(
        targets.iter().any(|t| common.starts_with(t)),
        "commondir {common:?} 不在监听目标 {targets:?} 里"
    );
}

#[test]
fn watch_targets_of_a_plain_repo_is_just_the_root() {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    let repo = git2::Repository::open(dir.path()).unwrap();
    // 普通仓库的 gitdir 就是 <root>/.git，递归监听根目录已覆盖，不该重复挂
    let targets = engine::watch::watch_targets(dir.path(), Some(&repo));
    assert_eq!(targets, vec![dir.path().to_path_buf()]);
}

/// 调用方给的路径写法和 libgit2 返回的不一致时，去重不能失效。
///
/// 这不是假想场景：CI 上 `TempDir` 落在 `C:\Users\RUNNER~1\...`（8.3 短名），
/// 而 `commondir()` 返回的是 `C:/Users/runneradmin/...`。两者按 `starts_with`
/// 比对必然失配，`.git/` 就被当成独立目标多挂了一次——根目录本来就是递归监听，
/// 于是每次 git 元数据变更触发两遍刷新。v0.4.0 的发版构建正是栽在这里。
///
/// 这里用 `sub/..` 制造同样的「写法不同、指向同一处」，好处是不依赖 Windows
/// 的短名机制，Linux 上同样能复现。
#[test]
fn watch_targets_dedupes_across_different_spellings_of_the_same_path() {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    fs::create_dir(dir.path().join("sub")).unwrap();

    // 指向同一个目录，但多了一段 `..`，和 libgit2 解析出来的写法对不上
    let noisy = dir.path().join("sub").join("..");
    let repo = git2::Repository::open(&noisy).unwrap();
    let targets = engine::watch::watch_targets(&noisy, Some(&repo));

    assert_eq!(
        targets.len(),
        1,
        "gitdir 落在根目录下，不该重复挂；实际拿到 {targets:?}"
    );
}

#[test]
fn watch_targets_without_repo_is_just_the_root() {
    let dir = TempDir::new().unwrap();
    // 仓库打不开时（不是仓库）退化为只监听根目录
    let targets = engine::watch::watch_targets(dir.path(), None);
    assert_eq!(targets, vec![dir.path().to_path_buf()]);
}

#[test]
fn external_commit_in_a_linked_worktree_produces_a_git_event() {
    let (_dir, wt) = repo_with_linked_worktree();
    let repo = git2::Repository::open(&wt).unwrap();
    let targets = engine::watch::watch_targets(&wt, Some(&repo));

    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        Config::default(),
    )
    .unwrap();
    for t in &targets {
        watcher.watch(t, RecursiveMode::Recursive).unwrap();
    }

    // 模拟「别的工具在这个 worktree 上 commit」：--allow-empty 不碰工作区文件，
    // git 只写 refs / objects / index——全在工作区根目录之外。
    // 只盯 worktree 根目录时这里一个 git 事件都收不到（本次事故的复现）。
    run_git(&wt, &["commit", "--allow-empty", "-m", "external"]);

    let is_git_internal = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        (s.contains("/.git/") || s.ends_with("/.git")) && !s.ends_with(".lock")
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(remaining > Duration::ZERO, "10 秒内没收到任何 git 内部事件");
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(event)) => {
                if event.paths.iter().any(|p| is_git_internal(p)) {
                    return;
                }
            }
            Ok(Err(e)) => panic!("watcher 错误: {e}"),
            Err(_) => {}
        }
    }
}
