#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
#[path = "common/long_ref.rs"]
mod long_ref;

use app_lib::engine::refs;
use git2::Repository;
use long_ref::plant_long_loose_ref;
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

/// `git branch -vv` 那一行里有没有 `[<upstream>: gone]`。拿 git 自己的判断做基准。
fn git_says_gone(dir: &std::path::Path, branch: &str) -> bool {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["branch", "-vv", "--no-color"])
        .output()
        .expect("git not found");
    assert!(out.status.success(), "git branch -vv failed");
    String::from_utf8_lossy(&out.stdout).lines().any(|line| {
        line.trim_start_matches(['*', '+', ' '])
            .split_whitespace()
            .next()
            == Some(branch)
            && line.contains(": gone]")
    })
}

#[test]
fn upstream_deleted_on_remote_and_pruned_is_reported_gone() {
    // 真实形态：分支推过并设了上游，远端（GitHub 合并 PR 后自动删分支）把它删了，
    // 本地再 `fetch --prune`。此时 `git branch -vv` 显示 `[origin/feature: gone]`，
    // 远程跟踪 ref 已经不存在，graph 上无从画远程标签——必须另有字段说清楚。
    let (local, remote) = repo_with_remote();
    run_git(local.path(), &["checkout", "-b", "feature"]);
    commit(local.path(), "f1");
    run_git(local.path(), &["push", "-u", "origin", "feature"]);
    // 直接在 bare 远程上删，模拟别处删掉分支；本地这边只做 fetch --prune
    run_git(remote.path(), &["branch", "-D", "feature"]);
    run_git(local.path(), &["fetch", "--prune", "origin"]);
    assert!(
        git_says_gone(local.path(), "feature"),
        "fixture 没造出 gone 状态"
    );

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let feature = find(&all, "refs/heads/feature");
    assert_eq!(
        feature.gone_upstream.as_deref(),
        Some("refs/remotes/origin/feature")
    );
    // 上游不在了就没有可比的对象，计数仍是 None，不能画成 0
    assert_eq!(feature.ahead, None);
    assert_eq!(feature.behind, None);
    assert!(all.iter().all(|r| r.name != "refs/remotes/origin/feature"));
}

#[test]
fn upstream_deleted_on_remote_but_not_yet_pruned_is_not_gone() {
    // 本地还没 prune 时远程跟踪 ref 仍在，git 也不认为 gone——
    // 界面照旧画 origin/feature 标签和 ↑↓，不能抢先报「已删除」
    let (local, remote) = repo_with_remote();
    run_git(local.path(), &["checkout", "-b", "feature"]);
    commit(local.path(), "f1");
    run_git(local.path(), &["push", "-u", "origin", "feature"]);
    run_git(remote.path(), &["branch", "-D", "feature"]);
    assert!(!git_says_gone(local.path(), "feature"));

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    let feature = find(&all, "refs/heads/feature");
    assert_eq!(feature.gone_upstream, None);
    assert_eq!(feature.ahead, Some(0));
}

#[test]
fn branch_without_upstream_config_is_not_gone() {
    // 从没推过（EnterWorktree 建的分支就是这样）和「推过但被删」必须区分开
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["branch", "solo"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    assert_eq!(find(&all, "refs/heads/solo").gone_upstream, None);
    assert_eq!(find(&all, "refs/heads/master").gone_upstream, None);
}

#[test]
fn upstream_pointing_at_deleted_local_branch_is_gone() {
    // 上游也可以是本地分支（branch.<name>.remote = "."）。它被删掉后
    // git 同样报 `[base: gone]`，这条路径不经过任何 remote 的 refspec 映射。
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["branch", "base"]);
    run_git(local.path(), &["branch", "--track", "child", "base"]);
    run_git(local.path(), &["branch", "-D", "base"]);
    assert!(
        git_says_gone(local.path(), "child"),
        "fixture 没造出 gone 状态"
    );

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    assert_eq!(
        find(&all, "refs/heads/child").gone_upstream.as_deref(),
        Some("refs/heads/base")
    );
}

#[test]
fn only_local_branches_carry_gone_upstream() {
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["tag", "v1"]);
    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();
    for r in all.iter().filter(|r| r.kind != "branch") {
        assert_eq!(r.gone_upstream, None, "{} 不该有 gone_upstream", r.name);
    }
}

/// 现场的形状：`packed-refs` 里是旧值，loose 里是新值，超长 ref 排在 `refs/heads` 之前。
/// 返回的仓库里 master 和 origin/master 都比 packed 里的值新一条，另有两个只有 loose 的 ref。
fn repo_with_packed_stale_refs() -> (TempDir, TempDir) {
    let (local, remote) = repo_with_remote();
    run_git(local.path(), &["update-ref", "refs/packed-ns/x", "HEAD"]);
    run_git(local.path(), &["pack-refs", "--all"]);
    commit(local.path(), "c2");
    run_git(local.path(), &["push", "origin", "HEAD"]);
    run_git(local.path(), &["branch", "loose-only"]);
    run_git(local.path(), &["tag", "loose-tag"]);
    plant_long_loose_ref(local.path(), &rev_parse(local.path(), "HEAD"));
    (local, remote)
}

#[test]
fn collect_prefers_loose_values_over_stale_packed_ones_behind_a_long_ref() {
    let (local, _remote) = repo_with_packed_stale_refs();
    let head = rev_parse(local.path(), "HEAD");

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert_eq!(find(&all, "refs/heads/master").hash, head);
    assert_eq!(find(&all, "refs/remotes/origin/master").hash, head);
}

#[test]
fn collect_lists_loose_only_refs_behind_a_long_ref() {
    let (local, _remote) = repo_with_packed_stale_refs();
    let head = rev_parse(local.path(), "HEAD");

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert_eq!(find(&all, "refs/heads/loose-only").hash, head);
    assert_eq!(find(&all, "refs/tags/loose-tag").hash, head);
}

#[test]
fn collect_reads_shared_loose_refs_from_a_linked_worktree() {
    // 用户可能直接打开 linked worktree：分支 ref 在主仓库的 commondir 下，不在它自己的 gitdir 里
    let (local, _remote) = repo_with_packed_stale_refs();
    let head = rev_parse(local.path(), "HEAD");
    let holder = TempDir::new().unwrap();
    let linked = holder.path().join("wt");
    run_git(
        local.path(),
        &[
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "wt",
        ],
    );

    let repo = Repository::open(&linked).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert_eq!(find(&all, "refs/heads/master").hash, head);
    assert_eq!(find(&all, "refs/heads/wt").hash, head);
}

fn assert_each_name_listed_once(all: &[refs::RefPointer]) {
    let mut names: Vec<&str> = all.iter().map(|r| r.name.as_str()).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "有 ref 被列了两次：{names:?}");
}

#[test]
fn collect_keeps_other_namespaces_without_listing_any_ref_twice() {
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["update-ref", "refs/custom/x", "HEAD"]);
    run_git(local.path(), &["tag", "v1"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert_eq!(find(&all, "refs/custom/x").kind, "other");
    assert_each_name_listed_once(&all);
}

#[test]
fn collect_lists_every_other_namespace_behind_a_long_ref() {
    let (local, _remote) = repo_with_packed_stale_refs();
    let head = rev_parse(local.path(), "HEAD");
    let packed = rev_parse(local.path(), "HEAD~1");
    run_git(local.path(), &["update-ref", "refs/custom/x", "HEAD"]);
    // refs/stash 是 refs/ 下的顶层文件，不是目录
    run_git(local.path(), &["update-ref", "refs/stash", "HEAD"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert_eq!(find(&all, "refs/custom/x").hash, head);
    assert_eq!(find(&all, "refs/stash").hash, head);
    assert_eq!(
        find(&all, "refs/packed-ns/x").hash,
        packed,
        "磁盘上没有目录、只在 packed-refs 里的命名空间也要列出来"
    );
    assert_each_name_listed_once(&all);
}

/// 阳性对照：证明上面这些 `..._behind_a_long_ref` 用例测的是真实缺陷，而不是摆设。
/// 这条失败说明 libgit2 的行为变了：可能是修好了，也可能是 fixture 不再能触发截断，
/// 要重新确认 `refs.rs` 里绕开全量遍历的做法还有没有必要。
#[cfg(windows)]
#[test]
fn long_ref_fixture_really_truncates_the_full_walk() {
    let (local, _remote) = repo_with_packed_stale_refs();
    let repo = Repository::open(local.path()).unwrap();

    let names: Vec<String> = repo
        .references()
        .unwrap()
        .flatten()
        .filter_map(|r| r.name().map(str::to_string))
        .collect();

    assert!(
        !names.iter().any(|n| n == "refs/heads/loose-only"),
        "全量遍历没有被截断：{names:?}"
    );
}

#[test]
fn branches_filters_by_kind_and_sees_loose_refs_behind_a_long_ref() {
    let (local, _remote) = repo_with_packed_stale_refs();
    let repo = Repository::open(local.path()).unwrap();

    let names = |kind| -> Vec<String> {
        refs::branches(&repo, kind)
            .unwrap()
            .iter()
            .map(|b| b.get().name().unwrap().to_string())
            .collect()
    };

    let local_names = names(Some(git2::BranchType::Local));
    assert!(
        local_names.contains(&"refs/heads/loose-only".to_string()),
        "{local_names:?}"
    );
    assert!(local_names.iter().all(|n| n.starts_with("refs/heads/")));

    let remote_names = names(Some(git2::BranchType::Remote));
    assert!(
        remote_names.contains(&"refs/remotes/origin/master".to_string()),
        "{remote_names:?}"
    );
    assert!(remote_names.iter().all(|n| n.starts_with("refs/remotes/")));

    assert_eq!(
        names(None).len(),
        local_names.len() + remote_names.len(),
        "None 应是两类之和"
    );
}

// ---- reflog 证据：分支上新建过哪些提交（图上判定分叉方向用） ----

fn plain_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    commit(dir.path(), "base");
    dir
}

fn made_here(all: &[refs::RefPointer], name: &str) -> Vec<String> {
    let mut list = find(all, name).made_here.clone();
    list.sort();
    list
}

#[test]
fn made_here_lists_commits_created_on_the_branch() {
    let dir = plain_repo();
    let d = dir.path();
    run_git(d, &["switch", "-q", "-c", "a"]);
    commit(d, "a1");
    run_git(d, &["switch", "-q", "-c", "b"]);
    commit(d, "b1");
    run_git(d, &["switch", "-q", "a"]);
    commit(d, "a2");
    run_git(d, &["commit", "-q", "--amend", "-m", "a2x"]);
    run_git(d, &["cherry-pick", "b"]);
    run_git(d, &["revert", "--no-edit", "HEAD"]);
    run_git(d, &["switch", "-q", "master"]);
    commit(d, "m1");
    run_git(d, &["switch", "-q", "a"]);
    run_git(
        d,
        &["merge", "-q", "--no-ff", "-m", "merge master", "master"],
    );

    let a1 = rev_parse(d, "a~4");
    let a2_amended = rev_parse(d, "a~3");
    let picked = rev_parse(d, "a~2");
    let reverted = rev_parse(d, "a~1");
    let merge = rev_parse(d, "a");
    let b1 = rev_parse(d, "b");

    let repo = Repository::open(d).unwrap();
    let all = refs::collect(&repo).unwrap();

    // amend 掉的旧 a2 也在 reflog 里：它确实是在 a 上做的，只是已经不在 a 上了。
    // 后端照实报，是否在第一父链上由前端判断。
    let mine = made_here(&all, "refs/heads/a");
    for oid in [&a1, &a2_amended, &picked, &reverted, &merge] {
        assert!(mine.contains(oid), "a 的 made_here 缺 {oid}：{mine:?}");
    }
    assert_eq!(
        mine.len(),
        6,
        "a1、a2、a2x、cherry-pick、revert、merge：{mine:?}"
    );
    // b1 是在 b 上做的；a 上那份 cherry-pick 是另一个提交
    assert!(!mine.contains(&b1), "{mine:?}");
    assert_eq!(made_here(&all, "refs/heads/b"), vec![b1]);
}

#[test]
fn made_here_counts_am_and_merges_made_by_pull() {
    // 实测消息：`am: <标题>`、`pull <参数>: Merge made by the 'ort' strategy.`
    let dir = plain_repo();
    let d = dir.path();
    let holder = TempDir::new().unwrap();
    let other = holder.path().join("other");
    run_git(
        holder.path(),
        &["clone", "-q", d.to_str().unwrap(), other.to_str().unwrap()],
    );
    run_git(&other, &["config", "user.email", "test@test.com"]);
    run_git(&other, &["config", "user.name", "Test"]);
    commit(&other, "theirs");
    commit(d, "mine");
    run_git(
        d,
        &[
            "-c",
            "pull.ff=true",
            "pull",
            "-q",
            "--no-rebase",
            "--no-edit",
            other.to_str().unwrap(),
            "master",
        ],
    );
    let merge = rev_parse(d, "master");

    let patches = holder.path().join("patches");
    run_git(
        &other,
        &["format-patch", "-1", "-q", "-o", patches.to_str().unwrap()],
    );
    run_git(d, &["switch", "-q", "-c", "applied", "master~1"]);
    let patch = fs::read_dir(&patches)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    run_git(d, &["am", "-q", patch.to_str().unwrap()]);
    let applied = rev_parse(d, "applied");

    let repo = Repository::open(d).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert!(made_here(&all, "refs/heads/master").contains(&merge));
    assert_eq!(made_here(&all, "refs/heads/applied"), vec![applied]);
}

#[test]
fn graph_load_keeps_only_made_here_commits_that_are_in_the_graph() {
    // amend 掉的旧提交还在 reflog 里，但已经不在任何分支上、不进图：不该随 payload 发给前端
    let dir = plain_repo();
    let d = dir.path();
    run_git(d, &["switch", "-q", "-c", "topic"]);
    commit(d, "t1");
    let old = rev_parse(d, "topic");
    run_git(d, &["commit", "-q", "--amend", "-m", "t1x"]);
    let new = rev_parse(d, "topic");

    let repo = Repository::open(d).unwrap();
    assert!(made_here(&refs::collect(&repo).unwrap(), "refs/heads/topic").contains(&old));

    let data = app_lib::engine::graph::load(d.to_str().unwrap()).unwrap();
    assert_eq!(made_here(&data.refs, "refs/heads/topic"), vec![new]);
}

#[test]
fn made_here_skips_branch_creation_and_fast_forward() {
    let dir = plain_repo();
    let d = dir.path();
    run_git(d, &["switch", "-q", "-c", "topic"]);
    commit(d, "t1");
    run_git(d, &["switch", "-q", "master"]);
    // 建分支（`branch: Created from`）和快进（`merge topic: Fast-forward`）都没有新建提交
    run_git(d, &["branch", "copy", "topic"]);
    run_git(d, &["merge", "-q", "--ff-only", "topic"]);

    let repo = Repository::open(d).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert_eq!(
        made_here(&all, "refs/heads/master"),
        vec![rev_parse(d, "master~1")],
        "master 只做过 base 这一个提交"
    );
    assert!(made_here(&all, "refs/heads/copy").is_empty());
    assert_eq!(
        made_here(&all, "refs/heads/topic"),
        vec![rev_parse(d, "topic")]
    );
}

#[test]
fn made_here_misses_rebased_commits() {
    // 分支 reflog 只记一条 `rebase (finish)`，重写出来的提交没有逐个记录：证据缺失，前端按行序退化
    let dir = plain_repo();
    let d = dir.path();
    run_git(d, &["switch", "-q", "-c", "topic"]);
    commit(d, "t1");
    run_git(d, &["switch", "-q", "master"]);
    commit(d, "m1");
    run_git(d, &["switch", "-q", "topic"]);
    run_git(d, &["rebase", "-q", "master"]);

    let repo = Repository::open(d).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert!(!made_here(&all, "refs/heads/topic").contains(&rev_parse(d, "topic")));
}

#[test]
fn made_here_is_empty_without_a_reflog() {
    let dir = plain_repo();
    let d = dir.path();
    run_git(d, &["switch", "-q", "-c", "topic"]);
    commit(d, "t1");
    // 只在本用例自建的临时 fixture 里清 reflog，模拟过期
    run_git(d, &["reflog", "expire", "--expire=now", "--all"]);

    let repo = Repository::open(d).unwrap();
    let all = refs::collect(&repo).unwrap();

    assert!(made_here(&all, "refs/heads/topic").is_empty());
    assert!(made_here(&all, "refs/heads/master").is_empty());
}

#[test]
fn only_local_branches_carry_made_here() {
    let (local, _remote) = repo_with_remote();
    run_git(local.path(), &["tag", "v1"]);

    let repo = Repository::open(local.path()).unwrap();
    let all = refs::collect(&repo).unwrap();

    let local_branch = all.iter().find(|r| r.kind == "branch").expect("本地分支");
    assert!(!local_branch.made_here.is_empty());
    for r in all.iter().filter(|r| r.kind != "branch") {
        assert!(r.made_here.is_empty(), "{} 不该带 made_here", r.name);
    }
}

#[test]
fn made_here_sees_commits_made_in_a_linked_worktree() {
    // 分支 reflog 放在 commondir，从主工作区和 linked worktree 打开都要读得到
    let dir = plain_repo();
    let d = dir.path();
    let holder = TempDir::new().unwrap();
    let linked = holder.path().join("wt");
    run_git(
        d,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wt",
            linked.to_str().unwrap(),
        ],
    );
    commit(&linked, "w1");
    let w1 = rev_parse(&linked, "HEAD");

    for path in [d, linked.as_path()] {
        let repo = Repository::open(path).unwrap();
        let all = refs::collect(&repo).unwrap();
        assert_eq!(
            made_here(&all, "refs/heads/wt"),
            vec![w1.clone()],
            "从 {path:?} 打开"
        );
    }
}
