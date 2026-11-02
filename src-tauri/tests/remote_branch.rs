use app_lib::commands::ops;
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

/// bare 远程 + 一个克隆，远程上带一条 feat/x 分支。全程走本地路径，不碰网络。
fn cloned_repo() -> (TempDir, TempDir) {
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare"]);
    let url = remote.path().to_str().unwrap().to_string();

    let seed = TempDir::new().unwrap();
    run_git(seed.path(), &["init"]);
    run_git(seed.path(), &["config", "user.email", "test@test.com"]);
    run_git(seed.path(), &["config", "user.name", "Test"]);
    commit(seed.path(), "c1");
    run_git(seed.path(), &["remote", "add", "origin", &url]);
    run_git(seed.path(), &["push", "-u", "origin", "HEAD"]);
    run_git(seed.path(), &["checkout", "-q", "-b", "feat/x"]);
    commit(seed.path(), "c2");
    run_git(seed.path(), &["push", "-u", "origin", "feat/x"]);

    let work = TempDir::new().unwrap();
    run_git(work.path(), &["clone", &url, work.path().to_str().unwrap()]);
    run_git(work.path(), &["config", "user.email", "test@test.com"]);
    run_git(work.path(), &["config", "user.name", "Test"]);

    (remote, work)
}

/// bare 远程 + 用 `git remote add` 接上去的工作仓库——**没有**
/// `refs/remotes/origin/HEAD`，那条 ref 只有 clone 才会写下。
///
/// fork 工作流里的 `upstream` 远程正是这么加的，而它恰恰是删错分支代价最大的
/// 那个远程（那是别人的主仓库）。
fn repo_with_added_remote() -> (TempDir, TempDir) {
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

/// 拿 bare 远程自己的 HEAD 当事实来源，不去猜 init.defaultBranch 是什么。
fn default_branch_of(remote: &TempDir) -> String {
    let head = git_out(remote.path(), &["symbolic-ref", "HEAD"]);
    head.strip_prefix("refs/heads/").expect(&head).to_string()
}

/// 离开作用域时把目录挪回原处。放在 Drop 里：`f` panic 时也要挪回去，
/// TempDir 只认原路径，否则目录会留在磁盘上。
struct MoveBack<'a> {
    moved: std::path::PathBuf,
    original: &'a std::path::Path,
}

impl Drop for MoveBack<'_> {
    fn drop(&mut self) {
        let _ = fs::rename(&self.moved, self.original);
    }
}

/// 在远程目录被挪走（模拟连不上）期间跑 `f`，跑完挪回去。
fn with_remote_moved_away<T>(remote: &TempDir, f: impl FnOnce() -> T) -> T {
    let moved = remote.path().with_extension("moved");
    fs::rename(remote.path(), &moved).unwrap();
    let _back = MoveBack {
        moved,
        original: remote.path(),
    };
    f()
}

#[test]
fn splits_the_remote_name_off_the_ref() {
    let (_remote, work) = cloned_repo();
    let (remote, branch_name) = branch::split_remote_ref(&p(&work), "origin/feat/x").unwrap();
    // 分支名里的斜杠必须留着，只能切掉远程名那一段
    assert_eq!(remote, "origin");
    assert_eq!(branch_name, "feat/x");
}

#[test]
fn a_ref_from_an_unknown_remote_is_an_error() {
    let (_remote, work) = cloned_repo();
    // 不能凭第一个斜杠瞎猜：upstream 这个远程根本不存在
    let result = branch::split_remote_ref(&p(&work), "upstream/feat/x");
    assert!(result.is_err());
}

#[test]
fn checkout_creates_a_local_branch_that_tracks_the_remote() {
    let (_remote, work) = cloned_repo();

    let local = ops::checkout_remote_branch(p(&work), "origin/feat/x".into()).unwrap();
    assert_eq!(local, "feat/x");

    // 必须是真的在分支上，不能是游离 HEAD——那样之后的提交不属于任何分支
    assert_eq!(
        git_out(work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat/x"
    );
    // 而且上游要设好，否则接下来 pull/push 都得手动指定
    assert_eq!(
        git_out(work.path(), &["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/feat/x"
    );
}

#[test]
fn checkout_switches_to_an_existing_local_branch_without_resetting_it() {
    let (_remote, work) = cloned_repo();
    // 先检出一次，再在本地多提交一条，然后回到 master
    ops::checkout_remote_branch(p(&work), "origin/feat/x".into()).unwrap();
    commit(work.path(), "local-only");
    let tip = git_out(work.path(), &["rev-parse", "HEAD"]);
    run_git(work.path(), &["checkout", "-q", "-"]);

    ops::checkout_remote_branch(p(&work), "origin/feat/x".into()).unwrap();

    assert_eq!(
        git_out(work.path(), &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat/x"
    );
    // 分支已经存在时只切过去，不去动它指向哪：静默重置到远程会丢掉这条本地提交
    assert_eq!(git_out(work.path(), &["rev-parse", "HEAD"]), tip);
}

#[test]
fn delete_remote_ref_removes_the_branch_from_the_remote() {
    let (_remote, work) = cloned_repo();
    assert!(git_out(work.path(), &["ls-remote", "--heads", "origin", "feat/x"]).contains("feat/x"));

    ops::delete_remote_ref(p(&work), "origin/feat/x".into()).unwrap();

    assert!(git_out(work.path(), &["ls-remote", "--heads", "origin", "feat/x"]).is_empty());
}

/// 远程的默认分支一律不给删，push 都不发出去。
///
/// 这是算错要删哪条分支时的最后一道护栏，不能把安全性外包给服务端：GitHub 会
/// 用 `refusing to delete the current branch` 挡住默认分支，但自建 Gitea/裸仓库
/// 未必挡，而且**只有默认分支**受这个待遇——别的分支删了就是删了。
#[test]
fn refuses_to_delete_the_remotes_default_branch() {
    let (_remote, work) = cloned_repo();
    // 克隆下来的 refs/remotes/origin/HEAD 指着远程的默认分支
    let default_ref = git_out(work.path(), &["rev-parse", "--abbrev-ref", "origin/HEAD"]);
    let default_branch = default_ref
        .strip_prefix("origin/")
        .expect(&default_ref)
        .to_string();

    let err = ops::delete_remote_ref(p(&work), default_ref.clone()).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);

    // 而且远程上它必须原封不动地还在
    assert!(
        !git_out(
            work.path(),
            &["ls-remote", "--heads", "origin", &default_branch]
        )
        .is_empty(),
        "默认分支被删掉了"
    );
}

/// 远程是 `git remote add` 接上去的（没有本地 HEAD 记录），护栏照样要挡住。
///
/// 只读 `refs/remotes/<remote>/HEAD` 的话这里会直接放行——而这正是最该拦住的
/// 场景：fork 工作流里 `upstream` 指的是别人的主仓库，本地 `main` 跟踪
/// `upstream/main` 时**名字是对得上的**，[`find_remote_branch`] 那层修复
/// 拦不住它，只剩这道护栏。
#[test]
fn refuses_to_delete_the_default_branch_of_a_remote_added_by_hand() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    // 前提确认：本地确实没有这条 ref，纯靠它判断必然放行
    assert!(
        git_out(
            work.path(),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                "refs/remotes/origin/HEAD"
            ]
        )
        .is_empty(),
        "这个 fixture 的前提是本地没有 origin/HEAD 记录"
    );

    let err = ops::delete_remote_ref(p(&work), format!("origin/{default_branch}")).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(
        !git_out(
            work.path(),
            &["ls-remote", "--heads", "origin", &default_branch]
        )
        .is_empty(),
        "默认分支被删掉了"
    );
}

/// 护栏只挡默认分支，别的分支该删还是要删得掉。
///
/// 加护栏最容易犯的错是拦过头——真拦过头的话用户所有的删远程分支操作都废了，
/// 而这条路径上没有 CI 会告诉我们。
#[test]
fn an_ordinary_branch_is_still_deletable_on_a_remote_without_a_head_ref() {
    let (_remote, work) = repo_with_added_remote();
    run_git(work.path(), &["checkout", "-q", "-b", "feat/ordinary"]);
    commit(work.path(), "c2");
    run_git(
        work.path(),
        &["push", "-q", "-u", "origin", "feat/ordinary"],
    );
    run_git(work.path(), &["checkout", "-q", "-"]);

    ops::delete_remote_ref(p(&work), "origin/feat/ordinary".into()).unwrap();

    assert!(git_out(
        work.path(),
        &["ls-remote", "--heads", "origin", "feat/ordinary"]
    )
    .is_empty());
}

/// 预取过之后护栏照样挡住默认分支（预取不能削弱护栏）。
///
/// 用 `git remote add` 接上的远程：本地没有 `origin/HEAD`，挡不挡只取决于预取或
/// 当场查询的结果。「护栏确实用了预取」由下一条测试证明。
#[test]
fn a_prefetched_default_branch_is_still_refused() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    branch::prefetch_remote_default(&p(&work), "origin");

    let err = ops::delete_remote_ref(p(&work), format!("origin/{default_branch}")).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(
        !git_out(
            work.path(),
            &["ls-remote", "--heads", "origin", &default_branch]
        )
        .is_empty(),
        "默认分支被删掉了"
    );
}

/// 护栏**真的用了**预取结果，而不是又问了一次远程。
///
/// 预取之后把远程目录挪走：护栏再去 ls-remote 必然失败，本地又没有 `origin/HEAD`
/// 可兜底——没用上预取的话护栏无依据放行，错误码会是 push 失败的 `git_cli`。
/// 只有复用了预取结果才会报 `protected_branch`。
#[test]
fn the_guard_uses_the_prefetch_instead_of_asking_the_remote_again() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    branch::prefetch_remote_default(&p(&work), "origin");

    let result = with_remote_moved_away(&remote, || {
        ops::delete_remote_ref(p(&work), format!("origin/{default_branch}"))
    });
    let err = result.unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 侧边栏入口按 `origin/x` 这种形式预取，护栏同样要用上这个结果。
///
/// 手法同上一条：预取后挪走远程、本地没有 `origin/HEAD`，只有预取真的按拆出来的
/// 远程名存进了缓存，才可能报 `protected_branch`。
#[test]
fn prefetching_by_remote_ref_fills_the_cache_the_guard_reads() {
    let (remote, work) = repo_with_added_remote();
    let remote_ref = format!("origin/{}", default_branch_of(&remote));

    branch::prefetch_remote_default_for_ref(&p(&work), &remote_ref);

    let result = with_remote_moved_away(&remote, || ops::delete_remote_ref(p(&work), remote_ref));
    let err = result.unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 预取失败（当时断网）之后网络恢复：护栏不能沿用那次失败，要自己再问一次。
///
/// 本地没有 `origin/HEAD` 可兜底，沿用失败就会放行，而此刻 push 已经能到达远程。
#[test]
fn a_failed_prefetch_does_not_disarm_the_guard() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    with_remote_moved_away(&remote, || {
        branch::prefetch_remote_default(&p(&work), "origin");
    });

    let err = ops::delete_remote_ref(p(&work), format!("origin/{default_branch}")).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 预取之后改了远程 URL，旧结果不能拿来给新远程把关。
///
/// 两个远程的默认分支不同：A 是建仓时的默认分支，B 是 `trunk`。预取的是 A，
/// 随后 origin 改指 B。缓存要是只按远程**名字**记，护栏会拿 A 的默认分支去比，
/// 于是放行删除 B 的默认分支 `trunk`。
#[test]
fn changing_the_remote_url_does_not_reuse_a_stale_prefetch() {
    let (_remote_a, work) = repo_with_added_remote();
    let url_b_dir = TempDir::new().unwrap();
    let url_b = url_b_dir.path().to_str().unwrap().to_string();
    run_git(url_b_dir.path(), &["init", "--bare"]);
    run_git(
        work.path(),
        &["push", "-q", &url_b, "HEAD:refs/heads/trunk"],
    );
    run_git(
        url_b_dir.path(),
        &["symbolic-ref", "HEAD", "refs/heads/trunk"],
    );

    branch::prefetch_remote_default(&p(&work), "origin");
    run_git(work.path(), &["remote", "set-url", "origin", &url_b]);
    run_git(work.path(), &["fetch", "-q", "origin"]);

    let err = ops::delete_remote_ref(p(&work), "origin/trunk".into()).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(
        !git_out(url_b_dir.path(), &["branch", "--list", "trunk"]).is_empty(),
        "B 的默认分支被删掉了"
    );
}

/// 预取之后删普通分支照常能删——缓存不能把护栏变成「全拦」。
#[test]
fn an_ordinary_branch_is_deletable_after_a_prefetch() {
    let (_remote, work) = cloned_repo();
    branch::prefetch_remote_default_for_ref(&p(&work), "origin/feat/x");

    ops::delete_remote_ref(p(&work), "origin/feat/x".into()).unwrap();

    assert!(git_out(work.path(), &["ls-remote", "--heads", "origin", "feat/x"]).is_empty());
}

/// 再建一个 bare 仓库，默认分支是 `default`，分支 `default` 和 `feat/x` 都指向 work 的 HEAD。
fn bare_repo_with_default(work: &TempDir, default: &str) -> TempDir {
    let bare = TempDir::new().unwrap();
    run_git(bare.path(), &["init", "--bare"]);
    run_git(
        work.path(),
        &[
            "push",
            "-q",
            bare.path().to_str().unwrap(),
            &format!("HEAD:refs/heads/{default}"),
            "HEAD:refs/heads/feat/x",
        ],
    );
    run_git(
        bare.path(),
        &["symbolic-ref", "HEAD", &format!("refs/heads/{default}")],
    );
    bare
}

fn add_push_url(work: &TempDir, url: &str) {
    run_git(
        work.path(),
        &["remote", "set-url", "--add", "--push", "origin", url],
    );
}

fn has_branch(bare: &TempDir, name: &str) -> bool {
    !git_out(bare.path(), &["branch", "--list", name]).is_empty()
}

fn delete_on_origin(work: &TempDir, branch: &str) -> app_lib::error::Result<()> {
    ops::delete_remote_branch(p(work), "origin".into(), branch.into())
}

/// push 走 pushurl 时，护栏要看 **push 到达的那个仓库**。
///
/// fetch 侧的默认分支不叫 trunk，只问 fetch URL 的话护栏放行，push 目标的默认分支就没了。
#[test]
fn refuses_to_delete_the_default_branch_of_the_push_target() {
    let (_fetch_side, work) = repo_with_added_remote();
    let push_side = bare_repo_with_default(&work, "trunk");
    add_push_url(&work, &p(&push_side));

    let err = delete_on_origin(&work, "trunk").unwrap_err();

    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(has_branch(&push_side, "trunk"), "push 目标的默认分支被删了");
}

/// 反过来：fetch 侧的默认分支在 push 目标上只是普通分支，删的又是 push 目标，不该拦。
#[test]
fn the_fetch_sides_default_is_deletable_on_a_push_target_that_does_not_use_it() {
    let (fetch_side, work) = repo_with_added_remote();
    let fetch_default = default_branch_of(&fetch_side);
    let push_side = bare_repo_with_default(&work, "trunk");
    run_git(
        work.path(),
        &[
            "push",
            "-q",
            &p(&push_side),
            &format!("HEAD:refs/heads/{fetch_default}"),
        ],
    );
    add_push_url(&work, &p(&push_side));

    delete_on_origin(&work, &fetch_default).unwrap();

    assert!(!has_branch(&push_side, &fetch_default));
    assert!(has_branch(&fetch_side, &fetch_default), "删到 fetch 侧去了");
}

/// 配了多个 pushurl 时 `git push` 会推到**每一个**，所以任何一个的默认分支都不能删。
#[test]
fn refuses_when_any_push_url_has_the_branch_as_default() {
    let (_fetch_side, work) = repo_with_added_remote();
    let first = bare_repo_with_default(&work, "develop");
    let second = bare_repo_with_default(&work, "trunk");
    run_git(
        work.path(),
        &["push", "-q", &p(&first), "HEAD:refs/heads/trunk"],
    );
    add_push_url(&work, &p(&first));
    add_push_url(&work, &p(&second));

    let err = delete_on_origin(&work, "trunk").unwrap_err();

    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(has_branch(&first, "trunk"), "第一个 pushurl 上的分支被删了");
    assert!(
        has_branch(&second, "trunk"),
        "第二个 pushurl 的默认分支被删了"
    );
}

/// `url.<base>.pushInsteadOf` 只改写 push 地址，配置里看不到 pushurl，护栏同样要跟过去。
#[test]
fn push_instead_of_is_followed_to_the_real_push_target() {
    let (fetch_side, work) = repo_with_added_remote();
    let push_side = bare_repo_with_default(&work, "trunk");
    run_git(
        work.path(),
        &[
            "config",
            &format!("url.{}.pushInsteadOf", p(&push_side)),
            &p(&fetch_side),
        ],
    );

    let err = delete_on_origin(&work, "trunk").unwrap_err();

    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(has_branch(&push_side, "trunk"));
}

/// 没配 pushurl 的远程要**按名字**问：`remote.<name>.proxy` 这类配置只认名字，push 按
/// 名字走得通，问 URL 却可能失败，护栏就无依据放行了。
///
/// 用 `uploadpack` 配一个不存在的命令来区分两种问法：它只影响按名字的查询。按 URL 问会
/// 成功并报 `protected_branch`；按名字问才会失败，放行后由 bare 仓库拒绝，报 `git_cli`。
#[test]
fn a_remote_without_a_pushurl_is_still_asked_by_name() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);
    run_git(
        work.path(),
        &["config", "remote.origin.uploadpack", "no-such-upload-pack"],
    );

    let err = delete_on_origin(&work, &default_branch).unwrap_err();

    assert_eq!(err.code, "git_cli", "{}", err.message);
}

/// 同一个 URL 配成 pushurl，和没配 pushurl 时的 push 目标相同，同样按名字问。
#[test]
fn a_push_url_equal_to_the_fetch_url_is_still_asked_by_name() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);
    add_push_url(&work, &p(&remote));
    run_git(
        work.path(),
        &["config", "remote.origin.uploadpack", "no-such-upload-pack"],
    );

    let err = delete_on_origin(&work, &default_branch).unwrap_err();

    assert_eq!(err.code, "git_cli", "{}", err.message);
}

/// 配了多个 fetch URL、没配 pushurl 时 push 同样到达每一个，而按名字只问得到第一个。
#[test]
fn every_url_is_asked_when_a_remote_has_several() {
    let (_first, work) = repo_with_added_remote();
    let second = bare_repo_with_default(&work, "trunk");
    run_git(
        work.path(),
        &["remote", "set-url", "--add", "origin", &p(&second)],
    );

    let err = delete_on_origin(&work, "trunk").unwrap_err();

    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(has_branch(&second, "trunk"));
}

/// 预取问的也必须是 push 目标：问成 fetch 侧的话，这次握手白做，护栏还要再连一次。
///
/// 手法同 `the_guard_uses_the_prefetch_instead_of_asking_the_remote_again`。
#[test]
fn the_prefetch_covers_the_push_target() {
    let (_fetch_side, work) = repo_with_added_remote();
    let push_side = bare_repo_with_default(&work, "trunk");
    add_push_url(&work, &p(&push_side));

    branch::prefetch_remote_default(&p(&work), "origin");

    let result = with_remote_moved_away(&push_side, || delete_on_origin(&work, "trunk"));
    let err = result.unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 多个 push 目标都要预取到：只预取第一个的话，后面的还得当场握手。
#[test]
fn the_prefetch_covers_every_push_target() {
    let (_fetch_side, work) = repo_with_added_remote();
    let first = bare_repo_with_default(&work, "develop");
    let second = bare_repo_with_default(&work, "trunk");
    run_git(
        work.path(),
        &["push", "-q", &p(&first), "HEAD:refs/heads/trunk"],
    );
    add_push_url(&work, &p(&first));
    add_push_url(&work, &p(&second));

    branch::prefetch_remote_default(&p(&work), "origin");

    let result = with_remote_moved_away(&second, || delete_on_origin(&work, "trunk"));
    let err = result.unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 空的、不存在的、长得像选项的远程名：预检问不到就放行，不崩，也不删东西。
#[test]
fn the_preflight_survives_empty_unknown_and_option_like_remote_names() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    for name in ["", "no-such-remote", "--upload-pack=no-such-upload-pack"] {
        ops::check_remote_deletable(p(&work), name.into(), default_branch.clone()).unwrap();
    }

    assert!(has_branch(&remote, &default_branch));
}

/// 预取之后改了 pushurl，旧结果不能拿来给新的 push 目标把关。
///
/// 旧目标的默认分支是 develop，trunk 在它上面只是普通分支；新目标的默认分支才是 trunk。
#[test]
fn changing_the_push_url_does_not_reuse_a_stale_prefetch() {
    let (_fetch_side, work) = repo_with_added_remote();
    let old_target = bare_repo_with_default(&work, "develop");
    let new_target = bare_repo_with_default(&work, "trunk");
    add_push_url(&work, &p(&old_target));

    branch::prefetch_remote_default(&p(&work), "origin");
    run_git(
        work.path(),
        &["remote", "set-url", "--push", "origin", &p(&new_target)],
    );

    let err = delete_on_origin(&work, "trunk").unwrap_err();

    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(has_branch(&new_target, "trunk"));
}

/// 删 worktree 之前的预检：默认分支要在这里就被拒，而且什么都没动。
///
/// 用 `git remote add` 接上的远程：弹框的 `remoteProtected` 只看本地 ref，这种远程上恒为
/// false，勾选框不会灰掉，所以这道预检是防止「worktree 删了、远程却被护栏拒绝」的唯一一关。
#[test]
fn the_preflight_refuses_the_default_branch_of_a_remote_added_by_hand() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    let err =
        ops::check_remote_deletable(p(&work), "origin".into(), default_branch.clone()).unwrap_err();

    assert_eq!(err.code, "protected_branch", "{}", err.message);
    assert!(has_branch(&remote, &default_branch));
}

/// 预检只读：普通分支放行，且不能真的删掉它。
#[test]
fn the_preflight_passes_an_ordinary_branch_without_deleting_it() {
    let (_remote, work) = cloned_repo();

    ops::check_remote_deletable(p(&work), "origin".into(), "feat/x".into()).unwrap();

    assert!(!git_out(work.path(), &["ls-remote", "--heads", "origin", "feat/x"]).is_empty());
}

/// 预检不能吃掉预取：吃掉的话紧接着的删除要再握一次手，提速就白做了。
#[test]
fn the_preflight_leaves_the_prefetch_for_the_delete() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    branch::prefetch_remote_default(&p(&work), "origin");
    let first = ops::check_remote_deletable(p(&work), "origin".into(), default_branch.clone());
    assert_eq!(first.unwrap_err().code, "protected_branch");

    let result = with_remote_moved_away(&remote, || delete_on_origin(&work, &default_branch));
    let err = result.unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 预检自己查到的答案也要留给删除：没赶上预取时不该为此多连一次网。
#[test]
fn the_delete_reuses_the_answer_the_preflight_queried_itself() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    let first = ops::check_remote_deletable(p(&work), "origin".into(), default_branch.clone());
    assert_eq!(first.unwrap_err().code, "protected_branch");

    let result = with_remote_moved_away(&remote, || delete_on_origin(&work, &default_branch));
    let err = result.unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}

/// 预检留下的答案仍然只服务一次删除：真正的删除取走之后，下一次必须重新问。
///
/// 第二次把远程挪走，本地又没有 `origin/HEAD`：还留着答案的话会报 `protected_branch`，
/// 答案确实没了才会走到 push，由 push 报连不上。
#[test]
fn the_preflight_does_not_turn_the_answer_into_a_lasting_cache() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    branch::prefetch_remote_default(&p(&work), "origin");
    let _ = ops::check_remote_deletable(p(&work), "origin".into(), default_branch.clone());
    let consumed = delete_on_origin(&work, &default_branch);
    assert_eq!(consumed.unwrap_err().code, "protected_branch");

    let result = with_remote_moved_away(&remote, || delete_on_origin(&work, &default_branch));
    let err = result.unwrap_err();
    assert_eq!(err.code, "git_cli", "{}", err.message);
}

/// 预检问不到远程时放行（与护栏的口径一致），失败也不能留下来。
#[test]
fn an_unreachable_preflight_does_not_block_or_disarm_the_guard() {
    let (remote, work) = repo_with_added_remote();
    let default_branch = default_branch_of(&remote);

    let offline = with_remote_moved_away(&remote, || {
        ops::check_remote_deletable(p(&work), "origin".into(), default_branch.clone())
    });
    offline.unwrap();

    let err = delete_on_origin(&work, &default_branch).unwrap_err();
    assert_eq!(err.code, "protected_branch", "{}", err.message);
}
