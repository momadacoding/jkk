//! 窗口标题里的版本信息：编译期 `git describe` 的真实形状 + 标题拼装规则。
//!
//! `build_support/git_describe.rs` 只给 build.rs 用，不进 lib，这里按路径直接引入。

#[path = "../build_support/git_describe.rs"]
mod git_describe;

use app_lib::build_info::{checkout_name, format_title};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} 失败");
}

fn commit(dir: &Path, content: &str) {
    fs::write(dir.join("a.txt"), content).unwrap();
    run_git(dir, &["add", "a.txt"]);
    run_git(dir, &["commit", "-m", content]);
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    commit(dir.path(), "c1");
    dir
}

fn describe(dir: &Path) -> String {
    git_describe::git_describe(dir).expect("应能取到 describe")
}

// ---- format_title ----

#[test]
fn release_build_exactly_on_its_tag_shows_plain_version() {
    assert_eq!(
        format_title("0.5.0", Some("v0.5.0"), false, None),
        "jkk 0.5.0"
    );
}

#[test]
fn release_build_after_tag_shows_describe() {
    assert_eq!(
        format_title("0.5.0", Some("v0.5.0-3-g610fdb0"), false, None),
        "jkk 0.5.0 (v0.5.0-3-g610fdb0)"
    );
}

#[test]
fn bumped_version_not_yet_tagged_is_not_mistaken_for_release() {
    // 版本号已升到 0.6.0、tag 还没打：最近的 tag 恰好是 v0.5.0 本身也不能算「正式版」
    assert_eq!(
        format_title("0.6.0", Some("v0.5.0"), false, None),
        "jkk 0.6.0 (v0.5.0)"
    );
}

#[test]
fn dev_build_is_marked_even_on_exact_tag() {
    assert_eq!(
        format_title("0.5.0", Some("v0.5.0"), true, None),
        "jkk 0.5.0 [dev]"
    );
    assert_eq!(
        format_title("0.5.0", Some("v0.5.0-3-g610fdb0"), true, None),
        "jkk 0.5.0 (v0.5.0-3-g610fdb0) [dev]"
    );
}

#[test]
fn missing_or_empty_describe_is_flagged_not_passed_off_as_release() {
    assert_eq!(
        format_title("0.5.0", None, false, None),
        "jkk 0.5.0 (未知提交)"
    );
    assert_eq!(
        format_title("0.5.0", Some(""), false, None),
        "jkk 0.5.0 (未知提交)"
    );
    assert_eq!(
        format_title("0.5.0", None, true, None),
        "jkk 0.5.0 (未知提交) [dev]"
    );
}

#[test]
fn dev_build_names_its_checkout() {
    // 不同 worktree 基于同一个提交起的 dev 实例，describe 完全相同，只能靠目录名分
    assert_eq!(
        format_title(
            "0.5.0",
            Some("v0.5.0-1-g610fdb0"),
            true,
            Some("window-title-version")
        ),
        "jkk 0.5.0 (v0.5.0-1-g610fdb0) [dev · window-title-version]"
    );
}

#[test]
fn release_build_never_shows_checkout() {
    // 构建机上的目录名对用户没有意义
    assert_eq!(
        format_title("0.5.0", Some("v0.5.0"), false, Some("a")),
        "jkk 0.5.0"
    );
}

#[test]
fn untagged_repo_shows_bare_hash() {
    assert_eq!(
        format_title("0.5.0", Some("610fdb0"), false, None),
        "jkk 0.5.0 (610fdb0)"
    );
}

// ---- git describe 在 fixture 仓库里的真实输出 ----

#[test]
fn describe_without_any_tag_falls_back_to_hash() {
    let dir = repo();
    let d = describe(dir.path());
    assert!(
        d.len() >= 7 && d.chars().all(|c| c.is_ascii_hexdigit()),
        "无 tag 时应退化成短 hash，实际：{d}"
    );
}

#[test]
fn describe_exact_lightweight_tag() {
    // 本仓库的 v* tag 都是 lightweight，--tags 不能省
    let dir = repo();
    run_git(dir.path(), &["tag", "v0.5.0"]);
    assert_eq!(describe(dir.path()), "v0.5.0");
}

#[test]
fn describe_exact_annotated_tag() {
    let dir = repo();
    run_git(dir.path(), &["tag", "-a", "v0.5.0", "-m", "release"]);
    assert_eq!(describe(dir.path()), "v0.5.0");
}

#[test]
fn describe_after_tag_counts_commits() {
    let dir = repo();
    run_git(dir.path(), &["tag", "v0.5.0"]);
    commit(dir.path(), "c2");
    commit(dir.path(), "c3");
    let d = describe(dir.path());
    assert!(d.starts_with("v0.5.0-2-g"), "实际：{d}");
}

#[test]
fn describe_ignores_non_version_tags() {
    let dir = repo();
    run_git(dir.path(), &["tag", "v0.5.0"]);
    commit(dir.path(), "c2");
    // 更近、但不是版本号的 tag 不能顶替 v0.5.0
    run_git(dir.path(), &["tag", "e2e-fixture"]);
    run_git(dir.path(), &["tag", "very-old"]);
    let d = describe(dir.path());
    assert!(d.starts_with("v0.5.0-1-g"), "实际：{d}");
}

#[test]
fn describe_outside_repo_is_none() {
    let dir = TempDir::new().unwrap();
    // 防止 git 往上找到某个外层仓库
    let out = Command::new("git")
        .current_dir(dir.path())
        .args(["rev-parse", "--git-dir"])
        .output()
        .unwrap();
    if out.status.success() {
        eprintln!("临时目录位于某个 git 仓库内，跳过");
        return;
    }
    assert_eq!(git_describe::git_describe(dir.path()), None);
}

// ---- rerun-if-changed 要盯的文件 ----

#[test]
fn watch_paths_all_exist_and_cover_head_and_refs() {
    // cargo 对不存在的 rerun-if-changed 路径会每次都重跑 build.rs，所以只能报存在的
    let dir = repo();
    run_git(dir.path(), &["tag", "v0.5.0"]);
    let paths = git_describe::watch_paths(dir.path());
    assert!(!paths.is_empty());
    for p in &paths {
        assert!(p.exists(), "报了不存在的路径：{}", p.display());
    }
    let git_dir = dir.path().join(".git");
    let same = |a: &Path, b: &Path| fs::canonicalize(a).unwrap() == fs::canonicalize(b).unwrap();
    let has = |want: &Path| paths.iter().any(|p| same(p, want));
    assert!(has(&git_dir.join("HEAD")), "{paths:?}");
    assert!(has(&git_dir.join("refs").join("tags")), "{paths:?}");
    // 盯的是当前分支自己的松散 ref，而不是整个 refs/heads
    let branch = current_branch_ref(dir.path());
    assert!(has(&git_dir.join(&branch)), "没盯 {branch}：{paths:?}");
    assert!(!has(&git_dir.join("refs").join("heads")), "{paths:?}");
}

fn current_branch_ref(dir: &Path) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["symbolic-ref", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn watch_paths_fall_back_to_heads_dir_when_branch_ref_is_packed() {
    // gc 之后分支 ref 只在 packed-refs 里；下一次提交会新建松散文件，
    // 只有盯目录才能发现
    let dir = repo();
    run_git(dir.path(), &["pack-refs", "--all"]);
    let git_dir = dir.path().join(".git");
    let branch = current_branch_ref(dir.path());
    assert!(!git_dir.join(&branch).exists(), "前置：ref 应已被 pack");

    let paths = git_describe::watch_paths(dir.path());
    for p in &paths {
        assert!(p.exists(), "报了不存在的路径：{}", p.display());
    }
    assert!(
        paths
            .iter()
            .any(|p| p == &git_dir.join("refs").join("heads")),
        "{paths:?}"
    );
    assert!(
        paths.iter().any(|p| p == &git_dir.join("packed-refs")),
        "{paths:?}"
    );
}

#[test]
fn watch_paths_on_detached_head_only_need_head() {
    let dir = repo();
    commit(dir.path(), "c2");
    run_git(dir.path(), &["checkout", "--detach", "HEAD~1"]);
    let paths = git_describe::watch_paths(dir.path());
    let git_dir = dir.path().join(".git");
    assert!(
        paths.iter().any(|p| p == &git_dir.join("HEAD")),
        "{paths:?}"
    );
    assert!(
        !paths
            .iter()
            .any(|p| p.starts_with(git_dir.join("refs").join("heads"))),
        "detached 时不该盯分支：{paths:?}"
    );
}

#[test]
fn watch_paths_in_linked_worktree_use_its_own_head_and_shared_refs() {
    // 本项目日常就在 worktree 里开发：.git 是文件，HEAD 在 .git/worktrees/<name>/ 下，
    // refs 在主仓库的 .git 里。盯错了 HEAD，切分支后标题不会更新。
    let main = repo();
    let wt_parent = TempDir::new().unwrap();
    let wt = wt_parent.path().join("wt");
    run_git(
        main.path(),
        &["worktree", "add", "-b", "feature", wt.to_str().unwrap()],
    );
    // build.rs 实际是从 src-tauri/ 这一层子目录调用的
    let paths = git_describe::watch_paths(&subdir(&wt));
    for p in &paths {
        assert!(p.exists(), "报了不存在的路径：{}", p.display());
    }
    let common = main.path().join(".git");
    let own_head = common.join("worktrees").join("wt").join("HEAD");
    let same = |a: &Path, b: &Path| fs::canonicalize(a).unwrap() == fs::canonicalize(b).unwrap();
    assert!(
        paths.iter().any(|p| same(p, &own_head)),
        "没盯 worktree 自己的 HEAD：{paths:?}"
    );
    assert!(
        paths
            .iter()
            .any(|p| same(p, &common.join("refs").join("heads").join("feature"))),
        "没盯共享 refs 里本分支的 ref：{paths:?}"
    );
}

/// build.rs 的 cwd 是 `src-tauri/`，不是仓库根：git 在子目录里给出的路径形状不同
/// （主 checkout 的 `--git-common-dir` 是 `../.git` 这种相对路径）。
fn subdir(root: &Path) -> std::path::PathBuf {
    let sub = root.join("src-tauri");
    fs::create_dir_all(&sub).unwrap();
    sub
}

/// cargo 对 rerun-if-changed 的判定：文件看自身 mtime，目录递归看最新的 mtime。
/// 这里把目录自身的 mtime 也算进去，只对「新建/改写文件」的操作是精确的；
/// 纯删除（如 `git tag -d` 最近的松散 tag）cargo 可能察觉不到，没有覆盖。
fn newest_mtime(paths: &[std::path::PathBuf]) -> std::time::SystemTime {
    fn walk(p: &Path, best: &mut std::time::SystemTime) {
        let meta = fs::metadata(p).unwrap();
        *best = (*best).max(meta.modified().unwrap());
        if meta.is_dir() {
            for e in fs::read_dir(p).unwrap() {
                walk(&e.unwrap().path(), best);
            }
        }
    }
    let mut best = std::time::SystemTime::UNIX_EPOCH;
    for p in paths {
        walk(p, &mut best);
    }
    best
}

/// 每一种会改变 describe 结果的操作，都必须让某个被盯的路径 mtime 前进，
/// 否则 build.rs 不会重跑，标题停在旧提交上。从子目录调用，和 build.rs 一致。
/// 每步之前重新取一次路径，模拟 build.rs 上一次运行时报给 cargo 的那一份。
fn assert_watch_paths_track_describe_changes(root: &Path) {
    let sub = subdir(root);
    let step = |what: &str, op: &dyn Fn()| {
        let paths = git_describe::watch_paths(&sub);
        assert!(!paths.is_empty(), "{what} 之前一个路径都没盯");
        for p in &paths {
            assert!(p.exists(), "报了不存在的路径：{}", p.display());
        }
        let before = newest_mtime(&paths);
        // mtime 精度足够，留点余量避免落在同一时间片
        std::thread::sleep(std::time::Duration::from_millis(50));
        op();
        assert!(
            newest_mtime(&paths) > before,
            "{what} 之后被盯的路径都没变：{paths:?}"
        );
    };
    step("提交", &|| commit(root, "c-next"));
    step("打 tag", &|| run_git(root, &["tag", "v9.9.9"]));
    step("切分支", &|| run_git(root, &["switch", "-c", "other"]));
    step("amend", &|| {
        run_git(root, &["commit", "--amend", "-m", "amended"])
    });
    step("detach", &|| {
        run_git(root, &["checkout", "--detach", "HEAD~1"])
    });
}

#[test]
fn watch_paths_track_changes_in_main_checkout_from_subdir() {
    let dir = repo();
    assert_watch_paths_track_describe_changes(dir.path());
}

#[test]
fn watch_paths_track_changes_in_linked_worktree_from_subdir() {
    let main = repo();
    commit(main.path(), "c2");
    let wt_parent = TempDir::new().unwrap();
    let wt = wt_parent.path().join("wt");
    run_git(
        main.path(),
        &["worktree", "add", "-b", "feature", wt.to_str().unwrap()],
    );
    assert_watch_paths_track_describe_changes(&wt);
}

#[test]
fn watch_paths_track_changes_in_packed_refs_repo() {
    let dir = repo();
    run_git(dir.path(), &["pack-refs", "--all"]);
    assert_watch_paths_track_describe_changes(dir.path());
}

#[test]
fn watch_paths_track_changes_in_reftable_repo() {
    // git 2.45+ 的 reftable 后端：refs/ 和 HEAD 只是兼容占位，永远不变，
    // 真正的引用在 reftable/ 里
    let dir = TempDir::new().unwrap();
    let ok = Command::new("git")
        .current_dir(dir.path())
        .args(["init", "--ref-format=reftable"])
        .status()
        .unwrap()
        .success();
    if !ok {
        eprintln!("本机 git 不支持 reftable，跳过");
        return;
    }
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    commit(dir.path(), "c1");
    assert_watch_paths_track_describe_changes(dir.path());
}

// ---- dev 标题里的 checkout 名 ----

#[test]
fn checkout_name_is_the_folder_holding_src_tauri() {
    assert_eq!(
        checkout_name(r"C:\dev\mygitclient\.claude\worktrees\window-title\src-tauri"),
        Some("window-title")
    );
    assert_eq!(
        checkout_name("/home/u/mygitclient/src-tauri/"),
        Some("mygitclient")
    );
    assert_eq!(checkout_name("src-tauri"), None);
    assert_eq!(checkout_name(""), None);
}
