#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
use app_lib::commands::remote;
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
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &std::path::Path, name: &str) {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
}

fn init(dir: &std::path::Path) {
    run_git(dir, &["init"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
}

/// bare 远程 + 两个克隆，全程走本地路径，不碰网络。
fn trio() -> (TempDir, TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let a = TempDir::new().unwrap();
    init(a.path());
    commit(a.path(), "c1");
    run_git(a.path(), &["remote", "add", "origin", &url]);
    run_git(a.path(), &["push", "-u", "origin", "HEAD"]);

    let b = TempDir::new().unwrap();
    run_git(b.path(), &["clone", &url, b.path().to_str().unwrap()]);
    run_git(b.path(), &["config", "user.email", "test@test.com"]);
    run_git(b.path(), &["config", "user.name", "Test"]);

    (remote, a, b)
}

#[test]
fn fetch_updates_remote_tracking_ref() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);

    let before = git_out(b.path(), &["rev-parse", "origin/HEAD"]);
    remote::fetch(b.path().to_str().unwrap().to_string(), false).unwrap();
    let after = git_out(b.path(), &["rev-parse", "FETCH_HEAD"]);
    assert_ne!(before, after);
}

#[test]
fn pull_brings_commits_into_working_branch() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);

    remote::pull(b.path().to_str().unwrap().to_string(), false).unwrap();
    assert!(b.path().join("c2.txt").exists());
}

/// 让 `dir` 里的下一次 fetch 撞上「另一个进程同时把当前分支快进了」。
///
/// 真实现场（2026-11-25）：app 点拉取的同时，另一个会话在同一个工作区跑了
/// `git fetch` + `git merge origin/master`。`git pull` 开头记下的 HEAD 是旧的，
/// fetch 完发现 HEAD 变了，就以为是自己改的，去把工作区从旧提交快进到新提交——
/// 而工作区早被另一个进程更新好了，于是把新提交的内容全当成「本地改动」拒绝。
///
fn race_next_fetch_with_a_fast_forward(dir: &std::path::Path) {
    race_next_fetch_with(dir, "git merge -q --ff-only '@{u}' >&2");
}

/// 在下一次 fetch 写入 `refs/remotes/origin/*` 的那一刻，在 `dir` 里跑一条 shell 命令。
///
/// 用 `reference-transaction` 钩子注入：这一刻正好落在 pull 的「记下 HEAD」与
/// 「比较 HEAD」之间，时序与真实现场一致（pull 会等 fetch 子进程退出，不靠运气）。
/// 只触发一次（`.git/race-started`），否则重试那次 pull 也会被它干扰。命令**成功后**
/// 才写 `.git/race-done`：测试据此确认并发现场真的造出来了，而不是命令失败后
/// 退化成一次普通 pull 照样通过。
///
/// 钩子是 pull 的子孙进程，会继承 pull 设的 `GIT_REFLOG_ACTION=pull`，不清掉的话
/// 它做的 merge 在 reflog 里会记成 `pull: Fast-forward`，看起来像 pull 自己挪的。
/// 真实现场里另一个进程有自己的环境，记的是 `merge …: Fast-forward`。
fn race_next_fetch_with(dir: &std::path::Path, command: &str) {
    let hook = dir.join(".git/hooks/reference-transaction");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\n\
             [ \"$1\" = committed ] || exit 0\n\
             grep -q ' refs/remotes/origin/' || exit 0\n\
             [ -e .git/race-started ] && exit 0\n\
             : > .git/race-started\n\
             unset GIT_REFLOG_ACTION\n\
             {command} && : > .git/race-done\n\
             exit 0\n"
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn assert_raced(dir: &std::path::Path) {
    assert!(
        dir.join(".git/race-done").exists(),
        "钩子里的命令没有成功，测试没有造出并发现场"
    );
}

#[test]
fn pull_race_is_recovered_even_with_unrelated_local_edits() {
    // 带着手里的改动点拉取是常态。和这次更新无关的改动不该让修复失效（独立 review 中危第 2 条）
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);
    let target = git_out(a.path(), &["rev-parse", "HEAD"]);
    fs::write(b.path().join("c1.txt"), "my local edit").unwrap();
    race_next_fetch_with_a_fast_forward(b.path());

    let result = remote::pull(b.path().to_str().unwrap().to_string(), false);

    assert_raced(b.path());
    result.expect("无关的本地改动不该挡住恢复");
    assert_eq!(git_out(b.path(), &["rev-parse", "HEAD"]), target);
    assert_eq!(
        fs::read_to_string(b.path().join("c1.txt")).unwrap(),
        "my local edit"
    );
}

#[test]
fn pull_that_moved_the_branch_itself_and_then_failed_is_reported_as_is() {
    // `submodule.recurse` 下，pull 先把主仓库快进了，再去检出子模块时被子模块里的
    // 本地改动挡住。分支是 pull 自己挪的，没有别的进程——不能报成「被另一个程序
    // 移动了」，更不能重拉吞掉子模块的失败（独立 review 中危第 1 条）
    let (_remote, a, b) = trio();
    let sub = TempDir::new().unwrap();
    init(sub.path());
    fs::write(sub.path().join("s.txt"), "v1").unwrap();
    run_git(sub.path(), &["add", "."]);
    run_git(sub.path(), &["commit", "-m", "s1"]);
    let sub_url = sub.path().to_str().unwrap();
    let file_ok = ["-c", "protocol.file.allow=always"];

    run_git(
        a.path(),
        &[&file_ok[..], &["submodule", "add", sub_url, "sub"]].concat(),
    );
    run_git(a.path(), &["commit", "-m", "add sub"]);
    run_git(a.path(), &["push"]);
    run_git(b.path(), &["pull"]);
    run_git(
        b.path(),
        &[&file_ok[..], &["submodule", "update", "--init"]].concat(),
    );

    fs::write(sub.path().join("s.txt"), "v2").unwrap();
    run_git(sub.path(), &["commit", "-am", "s2"]);
    let a_sub = a.path().join("sub");
    run_git(&a_sub, &["pull"]);
    run_git(a.path(), &["commit", "-am", "bump sub"]);
    run_git(a.path(), &["push"]);

    let b_sub = b.path().join("sub");
    // 先把子模块的新提交取到本地，pull 里不用再走 file 协议
    run_git(&b_sub, &["fetch"]);
    run_git(b.path(), &["config", "submodule.recurse", "true"]);
    run_git(b.path(), &["config", "protocol.file.allow", "always"]);
    run_git(&b_sub, &["config", "protocol.file.allow", "always"]);
    fs::write(b_sub.join("s.txt"), "my edit inside the submodule").unwrap();
    let before = git_out(b.path(), &["rev-parse", "HEAD"]);

    let err = remote::pull(b.path().to_str().unwrap().to_string(), false)
        .expect_err("子模块检出被挡，pull 必须报错");

    assert_ne!(
        git_out(b.path(), &["rev-parse", "HEAD"]),
        before,
        "前提不成立：pull 没有先快进主仓库，测试没有造出「自己挪了又失败」的现场"
    );
    assert_eq!(err.code, "git_cli", "实际：{}", err.message);
    assert_eq!(
        fs::read_to_string(b_sub.join("s.txt")).unwrap(),
        "my edit inside the submodule"
    );
}

#[test]
fn pull_does_not_retry_when_the_branch_moved_but_the_working_tree_did_not() {
    // 另一种「fetch 期间 HEAD 变了」：分支被挪走，工作区却没跟着动（别的工具
    // `update-ref` 了当前分支）。光是这样 git 自己能把工作区快进过去；但用户
    // 正好改着上游也改了的文件时，快进被拒，工作区相对新 HEAD 是脏的。这时自动
    // 重拉会因为「已经是最新」报成功，把反向改动留在工作区里——必须拒绝并说清楚。
    let (_remote, a, b) = trio();
    fs::write(a.path().join("c1.txt"), "upstream edit").unwrap();
    run_git(a.path(), &["commit", "-am", "c2"]);
    run_git(a.path(), &["push"]);
    let target = git_out(a.path(), &["rev-parse", "HEAD"]);
    fs::write(b.path().join("c1.txt"), "my local edit").unwrap();
    race_next_fetch_with(b.path(), "git update-ref HEAD '@{u}'");

    let err = remote::pull(b.path().to_str().unwrap().to_string(), false)
        .expect_err("工作区没跟上新 HEAD 时不能报成功");

    assert_raced(b.path());
    assert_eq!(err.code, "pull_head_moved", "实际：{}", err.message);
    // 不许替用户做任何恢复动作：HEAD 停在别人挪到的位置，用户的改动原样
    assert_eq!(git_out(b.path(), &["rev-parse", "HEAD"]), target);
    assert_eq!(
        fs::read_to_string(b.path().join("c1.txt")).unwrap(),
        "my local edit"
    );
}

#[test]
fn pull_succeeds_when_another_process_fast_forwards_the_branch_mid_fetch() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);
    let target = git_out(a.path(), &["rev-parse", "HEAD"]);
    race_next_fetch_with_a_fast_forward(b.path());

    let result = remote::pull(b.path().to_str().unwrap().to_string(), false);

    assert_raced(b.path());
    result.expect("分支已被别的进程快进到最新，拉取应当视为成功");
    assert_eq!(git_out(b.path(), &["rev-parse", "HEAD"]), target);
    assert_eq!(git_out(b.path(), &["status", "--porcelain"]), "");
}

#[test]
fn pull_with_rebase_succeeds_when_another_process_fast_forwards_the_branch_mid_fetch() {
    let (_remote, a, b) = trio();
    commit(a.path(), "c2");
    run_git(a.path(), &["push"]);
    let target = git_out(a.path(), &["rev-parse", "HEAD"]);
    race_next_fetch_with_a_fast_forward(b.path());

    let result = remote::pull(b.path().to_str().unwrap().to_string(), true);

    assert_raced(b.path());
    result.expect("pull --rebase 撞上同样的并发也应当成功");
    assert_eq!(git_out(b.path(), &["rev-parse", "HEAD"]), target);
    assert_eq!(git_out(b.path(), &["status", "--porcelain"]), "");
}

#[test]
fn pull_with_rebase_conflict_is_reported_as_is() {
    // rebase 撞冲突时 HEAD 也挪了（停在中途、detached），这是 pull 自己的失败，
    // 不能被当成「别的进程挪了分支」去重拉或换文案
    let (_remote, a, b) = trio();
    fs::write(a.path().join("c1.txt"), "upstream").unwrap();
    run_git(a.path(), &["commit", "-am", "remote-side"]);
    run_git(a.path(), &["push"]);
    fs::write(b.path().join("c1.txt"), "local").unwrap();
    run_git(b.path(), &["commit", "-am", "local-side"]);

    let err = remote::pull(b.path().to_str().unwrap().to_string(), true).expect_err("冲突必须报错");

    assert_eq!(err.code, "git_cli", "实际：{}", err.message);
    assert_eq!(
        app_lib::engine::repo::state(b.path().to_str().unwrap()).unwrap(),
        "rebase",
        "应当停在 rebase 中途等用户解决冲突"
    );
}

#[test]
fn pull_with_rebase_keeps_history_linear() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    commit(b.path(), "local-side");

    remote::pull(b.path().to_str().unwrap().to_string(), true).unwrap();
    // rebase 不产生 merge 提交：所有提交都只有一个父
    let parents = git_out(b.path(), &["log", "--pretty=%p"]);
    for line in parents.lines() {
        assert!(
            line.split_whitespace().count() <= 1,
            "found a merge commit: {line}"
        );
    }
}

#[test]
fn push_sets_upstream_for_a_fresh_branch() {
    // 新分支没有上游，裸 `git push` 会失败；实现必须自动补 -u origin <branch>
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["checkout", "-b", "feature"]);
    commit(b.path(), "f1");

    remote::push(b.path().to_str().unwrap().to_string(), false).unwrap();
    let upstream = git_out(
        b.path(),
        &["rev-parse", "--abbrev-ref", "feature@{upstream}"],
    );
    assert_eq!(upstream, "origin/feature");
}

#[test]
fn push_propagates_rejection_as_error() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    commit(b.path(), "local-side");

    // b 落后于远程，非快进推送必须被拒绝并转成 GitError 而不是静默成功
    let err = remote::push(b.path().to_str().unwrap().to_string(), false)
        .expect_err("non-fast-forward push should fail");
    assert_eq!(err.code, "git_cli");
}

#[test]
fn push_tag_puts_the_tag_on_the_remote() {
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["tag", "-a", "v1", "-m", "release one"]);

    remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into()).unwrap();

    let listed = git_out(b.path(), &["ls-remote", "origin", "refs/tags/v1"]);
    assert!(
        listed.contains("refs/tags/v1"),
        "远端没有这条 tag：{listed:?}"
    );
}

#[test]
fn push_tag_rejects_an_empty_name() {
    // 空名不能交给 git：`git push origin refs/tags/:refs/tags/` 是删除远端引用的
    // 形态，绝不能因为界面传了个空串就走到那里去
    let (_remote, _a, b) = trio();
    let path = b.path().to_str().unwrap().to_string();

    for empty in ["", "   "] {
        let err = remote::push_tag(path.clone(), empty.into())
            .expect_err("empty tag name should be rejected");
        assert_eq!(err.code, "bad_arg", "empty name {empty:?} 没被挡住");
    }
}

#[test]
fn push_tag_works_when_the_remote_is_not_named_origin() {
    // 远端叫 upstream 的仓库不少（fork 工作流）。硬写 origin 会让这类仓库
    // 永远推不出去，而界面上没有地方能改远端名。
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["remote", "rename", "origin", "upstream"]);
    run_git(b.path(), &["tag", "v1"]);

    remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into()).unwrap();

    let listed = git_out(b.path(), &["ls-remote", "upstream", "refs/tags/v1"]);
    assert!(listed.contains("refs/tags/v1"), "tag 没推上去：{listed:?}");
}

#[test]
fn push_tag_without_any_remote_reports_a_clear_error() {
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["remote", "remove", "origin"]);
    run_git(b.path(), &["tag", "v1"]);

    let err = remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into())
        .expect_err("没有远端时应当报错");
    assert_eq!(err.code, "no_remote");
}

#[test]
fn push_tag_refuses_to_guess_between_several_remotes() {
    // 多个远端又没有 origin：猜哪个都可能把 tag 推到错的地方
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["remote", "rename", "origin", "upstream"]);
    run_git(
        b.path(),
        &["remote", "add", "fork", "https://example.invalid/x"],
    );
    run_git(b.path(), &["tag", "v1"]);

    let err = remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into())
        .expect_err("多个远端且没有 origin 时应当报错");
    assert_eq!(err.code, "ambiguous_remote");
}

#[test]
fn push_tag_does_not_push_a_same_named_branch() {
    // 短名 `git push origin v1` 在同时存在分支 v1 和 tag v1 时是有歧义的。
    // 必须用全限定 refspec，否则可能推错东西。
    let (_remote, _a, b) = trio();
    run_git(b.path(), &["branch", "v1"]);
    run_git(b.path(), &["tag", "v1"]);

    remote::push_tag(b.path().to_str().unwrap().to_string(), "v1".into()).unwrap();

    let tag = git_out(b.path(), &["ls-remote", "origin", "refs/tags/v1"]);
    assert!(tag.contains("refs/tags/v1"), "tag 没推上去：{tag:?}");
    let branch = git_out(b.path(), &["ls-remote", "origin", "refs/heads/v1"]);
    assert!(branch.is_empty(), "同名分支被一起推上去了：{branch:?}");
}

#[test]
fn force_push_overwrites_remote() {
    let (_remote, a, b) = trio();
    commit(a.path(), "remote-side");
    run_git(a.path(), &["push"]);
    run_git(b.path(), &["fetch"]);
    commit(b.path(), "local-side");
    run_git(b.path(), &["reset", "--hard", "HEAD"]);

    remote::push(b.path().to_str().unwrap().to_string(), true).unwrap();
    let local = git_out(b.path(), &["rev-parse", "HEAD"]);
    let pushed = git_out(a.path(), &["ls-remote", "origin", "HEAD"]);
    assert!(
        pushed.starts_with(&local[..8]),
        "remote HEAD {pushed} != local {local}"
    );
}
