//! 换索引 / 建库之后，索引条目要带着 stat 信息。
//!
//! 2026-10-28 现场：建库与每次同步都用 `read-tree` 在**空**索引里生成新索引，条目的
//! ctime / mtime / size 全是 0。git 与 libgit2 都只能逐个读文件哈希来判断「没改」，
//! 一万五千个文件的仓库 `git status` 冷缓存 53 秒；而 libgit2 只读不回写，这个慢
//! 永远不会自愈，客户端的 Local Changes 长时间刷不出来。
//!
//! 断言对象是索引里的 stat 字段本身，不是耗时——耗时在 fixture 的几个文件上测不出来。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::{run_git, run_git_code};
use app_lib::engine::svn_baseline::swap_index_to_tree;
use app_lib::engine::svn_init::{init_hybrid, InitRequest};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn fixture_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 索引里这个条目的 (mtime 秒, mtime 纳秒, size)。mtime 为 0 = 没有 stat 信息。
///
/// size 也要比：git 对「racily clean」的条目会把 size 抹成 0（smudge），mtime 照留。
/// 只看 mtime 的话，一个 stat 实际已经失效、status 仍要重新哈希的条目也能过。
fn index_stat(root: &Path, path: &str) -> (i32, u32, u32) {
    let repo = git2::Repository::open(root).unwrap();
    let idx = repo.index().unwrap();
    let e = idx
        .get_path(Path::new(path), 0)
        .unwrap_or_else(|| panic!("索引里没有 {path}"));
    (e.mtime.seconds(), e.mtime.nanoseconds(), e.file_size)
}

/// 把文件 mtime 拨回一小时前。fixture 里写文件、`git add`、写索引都在同一秒内，
/// 条目个个都是 racily clean，git 写索引时会抹掉 size——测不出 stat 有没有被沿用。
fn age(p: &Path) {
    let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    fs::OpenOptions::new()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(past)
        .unwrap();
}

/// 两棵树：`old` 有 a / b / c，`new` 只改了 b。真实索引 = `old`，且 stat 齐全
/// （`git add` 写入的索引本来就带 stat）。
struct TwoTrees {
    dir: TempDir,
    old: String,
    new: String,
}

fn two_trees() -> TwoTrees {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    fixture_git(p, &["init", "-b", "svn/base"]);
    fixture_git(p, &["config", "user.email", "t@t"]);
    fixture_git(p, &["config", "user.name", "T"]);
    fixture_git(p, &["config", "core.autocrlf", "false"]);
    for (f, c) in [("a.txt", "a\n"), ("b.txt", "b\n"), ("c.txt", "c\n")] {
        fs::write(p.join(f), c).unwrap();
        age(&p.join(f));
    }
    fixture_git(p, &["add", "-A"]);
    fixture_git(p, &["commit", "-qm", "one"]);
    let old = fixture_git(p, &["rev-parse", "HEAD^{tree}"]);
    // 新树只在一个临时索引里拼，不碰真实索引
    let tmp = p.join(".git").join("probe-index");
    let env_git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(p)
            .env("GIT_INDEX_FILE", &tmp)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    env_git(&["read-tree", &old]);
    // 新内容放在工作区之外写对象，免得改动磁盘上的 b.txt
    let src = p.join(".git").join("probe-b");
    fs::write(&src, "B2\n").unwrap();
    let blob = fixture_git(p, &["hash-object", "-w", src.to_str().unwrap()]);
    fs::remove_file(&src).unwrap();
    env_git(&[
        "update-index",
        "--cacheinfo",
        &format!("100644,{blob},b.txt"),
    ]);
    let new = env_git(&["write-tree"]);
    fs::remove_file(&tmp).unwrap();
    // 前提：真实索引的条目确实带完整 stat（size 没被 smudge），否则下面「保留」的
    // 断言什么也证明不了
    for f in ["a.txt", "b.txt", "c.txt"] {
        let (s, _, size) = index_stat(p, f);
        assert!(
            s != 0 && size != 0,
            "前提没成立：{f} 在原索引里 stat 不完整"
        );
    }
    TwoTrees { dir, old, new }
}

#[test]
fn swapping_the_index_keeps_stat_for_entries_the_new_tree_did_not_change() {
    let t = two_trees();
    let p = t.dir.path();
    let (a_before, c_before) = (index_stat(p, "a.txt"), index_stat(p, "c.txt"));
    let b_old_stat = index_stat(p, "b.txt");
    // 同步之后磁盘上 b.txt 就是新内容
    fs::write(p.join("b.txt"), "B2\n").unwrap();
    age(&p.join("b.txt"));

    swap_index_to_tree(p.to_str().unwrap(), &t.old, &t.new).unwrap();

    assert_eq!(
        run_git(p.to_str().unwrap(), &["write-tree"])
            .unwrap()
            .trim(),
        t.new,
        "换上去的索引内容必须恰好是新树"
    );
    assert_eq!(
        index_stat(p, "a.txt"),
        a_before,
        "a.txt 没变，stat 必须原样保留"
    );
    assert_eq!(
        index_stat(p, "c.txt"),
        c_before,
        "c.txt 没变，stat 必须原样保留"
    );
    // b.txt 换了内容：旧 stat 对新 blob 无效不能沿用，而且要按磁盘重新填上——
    // 留成 0 的话，行尾迁移这种改写全部 blob 的操作之后，慢又全部回来了（独立 review）
    let b_now = index_stat(p, "b.txt");
    assert_ne!(b_now, b_old_stat, "b.txt 变了，旧 stat 不能沿用");
    assert!(
        b_now.0 != 0 && b_now.2 == 3,
        "b.txt 与磁盘一致，stat 应当按磁盘重新填上，实际 {b_now:?}"
    );
    let (code, _) = run_git_code(p.to_str().unwrap(), &["diff", "--quiet"]).unwrap();
    assert_eq!(code, 0, "磁盘就是新树的内容，工作区相对索引应当干净");
}

#[test]
fn swapping_succeeds_when_a_changed_entry_has_unstaged_edits_on_disk() {
    // 2026-10-30 实测：`git read-tree -m <tree>` 在「这个条目要换、工作区里它又有未暂存
    // 改动」时直接拒绝（`Entry 'b.txt' not uptodate. Cannot merge.`，退出 128）。
    // SVN 同步把服务器改动合进一个本地也改过的文件之后，正是这个形态——不能让同步
    // 因此失败，也不能动磁盘上的本地改动。
    let t = two_trees();
    let p = t.dir.path();
    fs::write(p.join("b.txt"), "B2\nlocal edit\n").unwrap();

    swap_index_to_tree(p.to_str().unwrap(), &t.old, &t.new).unwrap();

    assert_eq!(
        run_git(p.to_str().unwrap(), &["write-tree"])
            .unwrap()
            .trim(),
        t.new
    );
    assert_eq!(
        fs::read_to_string(p.join("b.txt")).unwrap(),
        "B2\nlocal edit\n",
        "磁盘上的本地改动一个字节都不能动"
    );
    // 只看「索引 vs 工作区」：fixture 里 HEAD 没跟着移到新树，status 的暂存列没有意义
    let unstaged = run_git(p.to_str().unwrap(), &["diff", "--name-only"]).unwrap();
    assert_eq!(unstaged.trim(), "b.txt", "本地改动应当显示为未暂存的修改");
}

#[test]
fn swapping_still_refuses_when_something_is_staged() {
    // 换索引会冲掉暂存内容，所以「索引此刻仍等于 expect」的守卫不能因为换了
    // 生成新索引的写法而松掉
    let t = two_trees();
    let p = t.dir.path();
    fs::write(p.join("a.txt"), "staged\n").unwrap();
    fixture_git(p, &["add", "a.txt"]);
    let before = run_git(p.to_str().unwrap(), &["write-tree"]).unwrap();

    let err = swap_index_to_tree(p.to_str().unwrap(), &t.old, &t.new).expect_err("必须拒绝");
    assert_eq!(err.code, "svn_index_moved");
    assert_eq!(
        run_git(p.to_str().unwrap(), &["write-tree"]).unwrap(),
        before,
        "拒绝时索引必须原样不动"
    );
    assert!(
        !p.join(".git/index.lock").exists(),
        "拒绝之后不能留下索引锁"
    );
}

#[test]
fn init_leaves_clean_files_with_stat_in_the_index() {
    let fx = svn_fixtures::svn_repo();
    let wc = fx.wc();
    // 建库前就有的本地 SVN 修改：建库后要照样显示成已修改
    fs::write(wc.join("Code/B.cs"), "local\n").unwrap();
    // 拨回 mtime 只为让 A.cs 不处于 racily clean，size 断言才有意义；内容不变，svn 仍判它干净
    age(&wc.join("Code/A.cs"));

    init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".to_string()],
        extensions: vec![".cs".to_string()],
    })
    .unwrap();

    let (secs, _, size) = index_stat(&wc, "Code/A.cs");
    let on_disk = fs::metadata(wc.join("Code/A.cs")).unwrap().len() as u32;
    assert!(
        secs != 0 && size == on_disk,
        "干净的文件建库后必须带完整 stat，否则每次 status 都要全量重新哈希，实际 ({secs}, {size})"
    );
    let status = run_git(&fx.wc_str(), &["status", "--porcelain"]).unwrap();
    assert!(
        status.lines().any(|l| l == " M Code/B.cs"),
        "本地修改必须照样显示，实际：{status:?}"
    );
    assert!(
        !status.lines().any(|l| l.ends_with("Code/A.cs")),
        "A.cs 没改过，不该出现在 status 里，实际：{status:?}"
    );
}
