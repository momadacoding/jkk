use app_lib::engine::external_launch::{self, TempWorkspace};
use app_lib::engine::external_merge::{self, PreparedMerge};
use app_lib::engine::external_tools_config::{ExternalTool, ExternalToolsConfig};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn git_out(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_out(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn p(dir: &Path) -> &str {
    dir.to_str().unwrap()
}

/// 本机全局 / 系统配置里的 `merge.*`、过滤器会混进判定，测试只认 fixture 的本地配置。
fn isolate_git_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let empty = Path::new(env!("CARGO_TARGET_TMPDIR")).join("empty-global-gitconfig");
        fs::write(&empty, b"").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    });
}

fn repo(autocrlf: bool) -> TempDir {
    isolate_git_config();
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "t@t.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    let crlf = if autocrlf { "true" } else { "false" };
    git(dir.path(), &["config", "core.autocrlf", crlf]);
    dir
}

fn write(dir: &Path, rel: &str, content: &[u8]) {
    let abs = dir.join(rel);
    fs::create_dir_all(abs.parent().unwrap()).unwrap();
    fs::write(abs, content).unwrap();
}

fn commit_all(dir: &Path, msg: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
}

/// base → feat 改成 theirs → 当前分支改成 ours → merge feat。
fn merge_conflict(dir: &Path, file: &str, base: &[u8], ours: &[u8], theirs: &[u8]) {
    let home = git(dir, &["symbolic-ref", "--short", "HEAD"]);
    write(dir, file, base);
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "feat"]);
    write(dir, file, theirs);
    commit_all(dir, "theirs");
    git(dir, &["checkout", "-q", &home]);
    write(dir, file, ours);
    commit_all(dir, "ours");
    assert!(!git_out(dir, &["merge", "feat"]).status.success());
}

fn text_conflict(dir: &Path, file: &str) {
    merge_conflict(
        dir,
        file,
        b"line1\nline2\n",
        b"ours1\nline2\n",
        b"theirs1\nline2\n",
    );
}

fn prepare(dir: &Path, file: &str) -> (PreparedMerge, TempWorkspace) {
    let ws = TempWorkspace::create().unwrap();
    let prepared = external_merge::prepare_merge(p(dir), file, &ws).unwrap();
    (prepared, ws)
}

fn prepare_err(dir: &Path, file: &str) -> String {
    let ws = TempWorkspace::create().unwrap();
    external_merge::prepare_merge(p(dir), file, &ws)
        .unwrap_err()
        .code
}

fn canon(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap()
}

/// 三份临时文件按工作区形态导出：autocrlf 下是 CRLF，并分别等于 stage 2 / 3 / 1。
#[test]
fn merge_exports_stage_2_3_1() {
    let dir = repo(true);
    text_conflict(dir.path(), "a.txt");
    let (m, ws) = prepare(dir.path(), "a.txt");
    assert_eq!(fs::read(&m.local).unwrap(), b"ours1\r\nline2\r\n");
    assert_eq!(fs::read(&m.remote).unwrap(), b"theirs1\r\nline2\r\n");
    assert_eq!(fs::read(&m.base).unwrap(), b"line1\r\nline2\r\n");
    assert!(m.local.starts_with(external_launch::temp_root()));
    assert!(m.local.ends_with("a.LOCAL.txt"), "{:?}", m.local);
    assert!(m.remote.ends_with("a.REMOTE.txt"), "{:?}", m.remote);
    assert!(m.base.ends_with("a.BASE.txt"), "{:?}", m.base);
    assert_eq!(canon(&m.merged), canon(&dir.path().join("a.txt")));
    ws.remove().unwrap();
}

#[test]
fn merge_titles_use_branch_names() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    let (m, ws) = prepare(dir.path(), "a.txt");
    assert_eq!(m.titles.local, "main（HEAD）");
    assert_eq!(m.titles.remote, "feat");
    assert_eq!(m.titles.base, "共同祖先");
    assert_eq!(m.titles.merged, "结果（工作区）");
    ws.remove().unwrap();
}

/// 被合并的提交没有分支指着它：用短 hash。
#[test]
fn merge_title_falls_back_to_short_hash() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    git(dir.path(), &["merge", "--abort"]);
    let feat = git(dir.path(), &["rev-parse", "feat"]);
    git(dir.path(), &["branch", "-D", "feat"]);
    assert!(!git_out(dir.path(), &["merge", &feat]).status.success());
    let (m, ws) = prepare(dir.path(), "a.txt");
    assert_eq!(m.titles.remote, feat[..7]);
    ws.remove().unwrap();
}

/// feature 有两个提交：c1 改第 5 行（能重放），c2 改第 1 行（和 main 冲突）。
/// 返回 c2 的完整 hash。
fn rebase_conflict(dir: &Path) -> String {
    write(dir, "a.txt", b"line1\nline2\nline3\nline4\nline5\n");
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "feature"]);
    write(dir, "a.txt", b"line1\nline2\nline3\nline4\nline5 c1\n");
    commit_all(dir, "c1 改第五行");
    write(dir, "a.txt", b"line1 c2\nline2\nline3\nline4\nline5 c1\n");
    commit_all(dir, "c2 改第一行");
    let c2 = git(dir, &["rev-parse", "HEAD"]);
    git(dir, &["checkout", "-q", "main"]);
    write(dir, "a.txt", b"line1 main\nline2\nline3\nline4\nline5\n");
    commit_all(dir, "main 改第一行");
    git(dir, &["checkout", "-q", "feature"]);
    assert!(!git_out(dir, &["rebase", "main"]).status.success());
    c2
}

/// rebase 时 LOCAL 是「上游 + 已经重放成功的提交」，不只是上游。
#[test]
fn rebase_second_commit_local_contains_first() {
    let dir = repo(false);
    rebase_conflict(dir.path());
    let (m, ws) = prepare(dir.path(), "a.txt");
    let local = fs::read_to_string(&m.local).unwrap();
    assert!(local.contains("line5 c1"), "{local}");
    assert!(local.contains("line1 main"), "{local}");
    let remote = fs::read_to_string(&m.remote).unwrap();
    assert!(remote.contains("line1 c2"), "{remote}");
    ws.remove().unwrap();
}

#[test]
fn rebase_titles_mention_rebase_head() {
    let dir = repo(false);
    let c2 = rebase_conflict(dir.path());
    let (m, ws) = prepare(dir.path(), "a.txt");
    assert_eq!(m.titles.local, "当前重放结果（HEAD，基于 main）");
    assert_eq!(
        m.titles.remote,
        format!("REBASE_HEAD {} c2 改第一行", &c2[..7])
    );
    ws.remove().unwrap();
}

#[test]
fn cherry_pick_titles_use_cherry_pick_head() {
    let dir = repo(false);
    write(dir.path(), "a.txt", b"line1\n");
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    write(dir.path(), "a.txt", b"theirs\n");
    commit_all(dir.path(), "挑过来的提交");
    let pick = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "a.txt", b"ours\n");
    commit_all(dir.path(), "ours");
    assert!(!git_out(dir.path(), &["cherry-pick", &pick])
        .status
        .success());
    let (m, ws) = prepare(dir.path(), "a.txt");
    assert_eq!(m.titles.local, "main（HEAD）");
    assert_eq!(
        m.titles.remote,
        format!("CHERRY_PICK_HEAD {} 挑过来的提交", &pick[..7])
    );
    ws.remove().unwrap();
}

#[test]
fn add_add_base_is_empty() {
    let dir = repo(false);
    write(dir.path(), "seed.txt", b"seed\n");
    commit_all(dir.path(), "seed");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    write(dir.path(), "new.txt", b"theirs\n");
    commit_all(dir.path(), "theirs");
    git(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "new.txt", b"ours\n");
    commit_all(dir.path(), "ours");
    assert!(!git_out(dir.path(), &["merge", "feat"]).status.success());
    let (m, ws) = prepare(dir.path(), "new.txt");
    assert_eq!(fs::read(&m.base).unwrap(), b"");
    assert_eq!(fs::read(&m.local).unwrap(), b"ours\n");
    ws.remove().unwrap();
}

#[test]
fn delete_modify_is_unsupported() {
    let dir = repo(false);
    write(dir.path(), "d.txt", b"keep\n");
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    git(dir.path(), &["rm", "-q", "d.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "del"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "d.txt", b"changed\n");
    commit_all(dir.path(), "mod");
    assert!(!git_out(dir.path(), &["merge", "feat"]).status.success());
    assert_eq!(
        prepare_err(dir.path(), "d.txt"),
        "external_merge_unsupported"
    );
}

#[test]
fn binary_content_is_unsupported() {
    let dir = repo(false);
    merge_conflict(dir.path(), "a.txt", b"base\n", b"ours\0\n", b"theirs\n");
    assert_eq!(
        prepare_err(dir.path(), "a.txt"),
        "external_merge_unsupported"
    );
}

/// ④a（`merge=binary`）可以交给外部工具：拿到的是三个原始版本，不是工作区里的半成品。
#[test]
fn not_text_merged_exports_original_versions() {
    let dir = repo(false);
    write(dir.path(), ".gitattributes", b"*.txt merge=binary\n");
    text_conflict(dir.path(), "a.txt");
    let (m, ws) = prepare(dir.path(), "a.txt");
    assert_eq!(fs::read(&m.remote).unwrap(), b"theirs1\nline2\n");
    assert_eq!(fs::read(&m.base).unwrap(), b"line1\nline2\n");
    ws.remove().unwrap();
}

/// 别处已经把冲突解决掉了：不能再按旧状态打开。
#[test]
fn resolved_file_is_rejected() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    write(dir.path(), "a.txt", b"done\n");
    git(dir.path(), &["add", "a.txt"]);
    assert_eq!(prepare_err(dir.path(), "a.txt"), "conflict_state_changed");
}

#[test]
fn bad_paths_are_rejected_before_any_lookup() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    for bad in ["", "../a.txt", "C:/x/a.txt", "/a.txt"] {
        assert_eq!(
            prepare_err(dir.path(), bad),
            "external_tool_bad_path",
            "{bad}"
        );
    }
}

/// 工作区文件被用户删掉了：`$MERGED` 必须是已有的普通文件，不替用户造一个。
#[test]
fn missing_merged_file_is_refused() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    fs::remove_file(dir.path().join("a.txt")).unwrap();
    assert_eq!(
        prepare_err(dir.path(), "a.txt"),
        "external_tool_not_regular"
    );
    assert!(!dir.path().join("a.txt").exists());
}

/// `$MERGED` 所在目录被换成指向仓库外的 junction：外部工具存盘会写到仓库外，必须拒绝。
#[cfg(windows)]
#[test]
fn merged_under_linked_dir_is_refused() {
    let dir = repo(false);
    text_conflict(dir.path(), "sub/a.txt");
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("a.txt"), b"<<<<<<< HEAD\noutside\n").unwrap();
    fs::remove_dir_all(dir.path().join("sub")).unwrap();
    let ok = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(dir.path().join("sub"))
        .arg(outside.path())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("跳过：建不了 junction");
        return;
    }
    let code = prepare_err(dir.path(), "sub/a.txt");
    assert!(
        code == "external_tool_not_regular" || code == "external_merge_unsupported",
        "{code}"
    );
    assert_eq!(
        fs::read(outside.path().join("a.txt")).unwrap(),
        b"<<<<<<< HEAD\noutside\n"
    );
}

#[test]
fn chinese_space_bracket_dash_path() {
    let dir = repo(false);
    let file = "目录 a/-f[1] x.txt";
    text_conflict(dir.path(), file);
    let (m, ws) = prepare(dir.path(), file);
    assert!(m.local.ends_with("-f[1] x.LOCAL.txt"), "{:?}", m.local);
    assert_eq!(fs::read(&m.local).unwrap(), b"ours1\nline2\n");
    assert_eq!(canon(&m.merged), canon(&dir.path().join(file)));
    ws.remove().unwrap();
}

#[test]
fn linked_worktree_merged_is_in_that_worktree() {
    let dir = repo(false);
    write(dir.path(), "seed.txt", b"seed\n");
    commit_all(dir.path(), "seed");
    let parent = TempDir::new().unwrap();
    let wt = parent.path().join("wt 目录");
    git(dir.path(), &["worktree", "add", "-q", "-b", "side", p(&wt)]);
    text_conflict(&wt, "a.txt");
    let (m, ws) = prepare(&wt, "a.txt");
    assert_eq!(canon(&m.merged), canon(&wt.join("a.txt")));
    assert_eq!(m.titles.local, "side（HEAD）");
    ws.remove().unwrap();
}

fn merge_tool(program: &str, merge_args: Option<&[&str]>) -> ExternalToolsConfig {
    ExternalToolsConfig {
        version: 1,
        default_diff: None,
        default_merge: Some("t".into()),
        rules: Vec::new(),
        tools: vec![ExternalTool {
            id: "t".into(),
            label: "Test Tool".into(),
            program: program.into(),
            diff_args: None,
            merge_args: merge_args.map(|a| a.iter().map(|s| s.to_string()).collect()),
            success_exit_codes: None,
            exit_code_messages: None,
        }],
    }
}

fn never(_: external_launch::ExitReport) {
    panic!("没启动成功时不应回调 on_exit");
}

#[test]
fn tool_without_merge_args_is_unsupported() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    let err = external_merge::open_external_merge(
        p(dir.path()),
        &merge_tool("whatever", None),
        Path::new("C:/cfg/externalTools.json"),
        "t",
        "a.txt",
        never,
    )
    .unwrap_err();
    assert_eq!(err.code, "external_tool_unsupported");
}

#[test]
fn missing_program_points_to_config_and_releases_lock() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    let err = external_merge::open_external_merge(
        p(dir.path()),
        &merge_tool("jkk-no-such-merge-tool", Some(&["$MERGED"])),
        Path::new("C:/cfg/externalTools.json"),
        "t",
        "a.txt",
        never,
    )
    .unwrap_err();
    assert_eq!(err.code, "launcher");
    assert!(
        err.message.contains("externalTools.json"),
        "{}",
        err.message
    );
    drop(external_launch::try_lock(p(dir.path()), "a.txt", "x").unwrap());
}

/// 工具退出后：临时目录被删、`$MERGED` 还在且内容不变、锁已放开。
#[cfg(windows)]
#[test]
fn merged_path_survives_cleanup() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    let merged_before = fs::read(dir.path().join("a.txt")).unwrap();
    let bin = TempDir::new().unwrap();
    let script = bin.path().join("fake.cmd");
    let record = bin.path().join("args.txt");
    fs::write(&script, "@echo %~1>\"%~dp0args.txt\"\r\n@exit /b 0\r\n").unwrap();

    let (tx, rx) = mpsc::channel();
    external_merge::open_external_merge(
        p(dir.path()),
        &merge_tool(
            script.to_str().unwrap(),
            Some(&["$LOCAL", "$REMOTE", "$BASE", "$MERGED"]),
        ),
        Path::new("externalTools.json"),
        "t",
        "a.txt",
        move |r| tx.send(r).unwrap(),
    )
    .unwrap();
    let report = rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(report.code, Some(0));

    let local = PathBuf::from(fs::read_to_string(&record).unwrap().trim());
    assert!(local.starts_with(external_launch::temp_root()), "{local:?}");
    assert!(!local.parent().unwrap().exists(), "临时子目录应已删除");
    assert_eq!(fs::read(dir.path().join("a.txt")).unwrap(), merged_before);
    drop(external_launch::try_lock(p(dir.path()), "a.txt", "x").unwrap());
}

/// merge 工具在仓库根目录下启动（与 git mergetool 一致），不继承应用进程的工作目录。
#[cfg(windows)]
#[test]
fn merge_tool_runs_in_repo_root() {
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    let bin = TempDir::new().unwrap();
    let script = bin.path().join("fake.cmd");
    fs::write(&script, "@cd>\"%~dp0cwd.txt\"\r\n@exit /b 0\r\n").unwrap();

    let (tx, rx) = mpsc::channel();
    external_merge::open_external_merge(
        p(dir.path()),
        &merge_tool(
            script.to_str().unwrap(),
            Some(&["$LOCAL", "$REMOTE", "$BASE", "$MERGED"]),
        ),
        Path::new("externalTools.json"),
        "t",
        "a.txt",
        move |r| tx.send(r).unwrap(),
    )
    .unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(60)).unwrap().code,
        Some(0)
    );

    let cwd = PathBuf::from(
        fs::read_to_string(bin.path().join("cwd.txt"))
            .unwrap()
            .trim(),
    );
    assert_eq!(
        fs::canonicalize(cwd).unwrap(),
        fs::canonicalize(dir.path()).unwrap()
    );
}

/// 同一个文件已经在外部工具里打开时，第二次打开被锁拦住。
#[test]
fn second_open_of_same_file_is_busy() {
    let dir = repo(false);
    text_conflict(dir.path(), "a.txt");
    let _held = external_launch::try_lock(p(dir.path()), "a.txt", "BC").unwrap();
    let err = external_merge::open_external_merge(
        p(dir.path()),
        &merge_tool("whatever", Some(&["$MERGED"])),
        Path::new("externalTools.json"),
        "t",
        "a.txt",
        never,
    )
    .unwrap_err();
    assert_eq!(err.code, "external_tool_busy");
}
