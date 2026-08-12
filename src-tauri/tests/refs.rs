use app_lib::engine::refs;
use git2::Repository;
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

/// 本地仓库 + bare 远程，master 已设上游，全程不碰网络。
fn repo_with_remote() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let local = TempDir::new().unwrap();
    run_git(local.path(), &["init"]);
    run_git(local.path(), &["config", "user.email", "test@test.com"]);
    run_git(local.path(), &["config", "user.name", "Test"]);
    commit(local.path(), "c1");
    run_git(
        local.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    run_git(local.path(), &["push", "-u", "origin", "HEAD"]);
    (local, remote)
}

fn find<'a>(refs: &'a [refs::RefPointer], name: &str) -> &'a refs::RefPointer {
    refs.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no ref {name}"))
}

/// `git rev-parse <rev>` 的完整 oid。用来在断言里拿 git 自己的答案做基准。
fn rev_parse(dir: &std::path::Path, rev: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", rev])
        .output()
        .expect("git not found");
    assert!(out.status.success(), "git rev-parse {rev} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn local_branch_in_sync_reports_zero_ahead_behind() {
    let (local, _remote) = repo_with_remote();
    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    let master = find(&all, "refs/heads/master");
    assert_eq!(master.ahead, Some(0));
    assert_eq!(master.behind, Some(0));
}

#[test]
fn local_branch_counts_its_own_ahead_and_behind() {
    // 侧边栏每个分支右边都要显示自己的 ↑↓，不能只有当前分支有
    let (local, _remote) = repo_with_remote();
    commit(local.path(), "c2");
    commit(local.path(), "c3");

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let master = find(&all, "refs/heads/master");
    assert_eq!(master.ahead, Some(2));
    assert_eq!(master.behind, Some(0));
}

#[test]
fn a_branch_without_upstream_reports_none() {
    // None 和 Some(0) 含义不同：没有上游时界面根本不该画徽章
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["branch", "solo"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let solo = find(&all, "refs/heads/solo");
    assert_eq!(solo.ahead, None);
    assert_eq!(solo.behind, None);
}

#[test]
fn remote_branches_carry_no_counts() {
    let (local, _remote) = repo_with_remote();
    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    let origin = find(&all, "refs/remotes/origin/master");
    assert_eq!(origin.kind, "remote");
    assert_eq!(origin.ahead, None);
    assert_eq!(origin.behind, None);
}

#[test]
fn annotated_tag_points_at_the_commit_not_the_tag_object() {
    // 附注 tag 的 refs/tags/x 指向的是一个 **tag object**，不是提交。
    // 直接用 reference.target() 拿到的 oid 在提交图里根本不存在，前端按 hash
    // 精确匹配提交行，于是这条 tag 的 pill 永远画不出来（实测 v0.1.0 就是这样）。
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["tag", "-a", "v1", "-m", "release one"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let tag = find(&all, "refs/tags/v1");

    assert_eq!(tag.kind, "tag");
    // 必须是 tag 指向的提交
    assert_eq!(tag.hash, rev_parse(local.path(), "v1^{commit}"));
    // 且必须**不是** tag object 自己 —— 少了这条断言，轻量 tag 也能让上一条通过，
    // 这个用例就测不出 peel 有没有真的做
    assert_ne!(tag.hash, rev_parse(local.path(), "v1"));
}

#[test]
fn lightweight_tag_still_points_at_the_commit() {
    // 轻量 tag 本来就直接指向提交，peel 之后不能把它弄坏
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["tag", "v2"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let tag = find(&all, "refs/tags/v2");

    assert_eq!(tag.kind, "tag");
    assert_eq!(tag.hash, rev_parse(local.path(), "v2^{commit}"));
    // 轻量 tag 的 ref 目标就是提交本身，两种写法应当同值
    assert_eq!(tag.hash, rev_parse(local.path(), "v2"));
}

#[test]
fn a_tag_on_a_blob_is_still_listed() {
    // tag 可以合法地指向 blob（git.git 的 junio-gpg-pub 就是），peel 不到提交。
    // 这种 tag 必须仍然出现在列表里——丢掉它会让侧边栏的数量和 `git tag` 对不上，
    // 而且没有任何提示说明少了什么。
    let (local, _remote) = repo_with_remote();
    let blob = {
        let out = Command::new("git")
            .current_dir(local.path())
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child
                    .stdin
                    .as_mut()
                    .unwrap()
                    .write_all(b"just a blob\n")
                    .unwrap();
                child.wait_with_output()
            })
            .expect("git hash-object failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    run_git(local.path(), &["tag", "blobtag", &blob]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let tag = find(&all, "refs/tags/blobtag");
    assert_eq!(tag.kind, "tag");
    // peel 不到提交时退回 ref 自己的目标，而不是把整条 ref 丢掉
    assert_eq!(tag.hash, blob);
}

#[test]
fn counts_are_per_branch_not_per_head() {
    // 曾经只有当前分支有计数。切到别的分支后，原分支的计数不能跟着 HEAD 跑。
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["checkout", "-b", "feature"]);
    commit(local.path(), "f1");
    run_git(local.path(), &["push", "-u", "origin", "feature"]);
    commit(local.path(), "f2");
    run_git(local.path(), &["checkout", "master"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    // HEAD 在 master 上，但 feature 自己领先 1
    assert_eq!(find(&all, "refs/heads/master").ahead, Some(0));
    assert_eq!(find(&all, "refs/heads/feature").ahead, Some(1));
}
