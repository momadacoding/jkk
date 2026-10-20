use app_lib::engine::merge_drivers::{self, DriverFinding, DriverReport, DriverState};
use std::fs;
use std::path::Path;
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

/// 本机全局 / 系统配置里的 `merge.*`（git-lfs 装的 `merge.lfs-text`）和全局属性文件
/// 会混进检查结果，测试只认 fixture 自己的配置。
fn isolate_git_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
        let empty = tmp.join("empty-global-gitconfig");
        fs::write(&empty, b"").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        // 没设 core.attributesFile 时 git 会读 $XDG_CONFIG_HOME/git/attributes
        let xdg = tmp.join("empty-xdg");
        fs::create_dir_all(&xdg).unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &xdg);
    });
}

fn repo() -> TempDir {
    isolate_git_config();
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "t@t.com"]);
    git(dir.path(), &["config", "user.name", "T"]);
    git(dir.path(), &["config", "core.autocrlf", "false"]);
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

/// 一个用 `merge=unityyamlmerge` 的 Unity 工程：场景放在 Assets/Scenes 下。
fn unity_repo(attrs: &str) -> TempDir {
    let dir = repo();
    write(dir.path(), ".gitattributes", attrs.as_bytes());
    write(dir.path(), "Assets/Scenes/Main.unity", b"scene\n");
    write(dir.path(), "Assets/Scenes/Other.unity", b"scene\n");
    write(dir.path(), "Assets/P.prefab", b"prefab\n");
    write(dir.path(), "Code/A.cs", b"class A {}\n");
    commit_all(dir.path(), "c1");
    dir
}

fn check(dir: &Path) -> DriverReport {
    let report = merge_drivers::check_merge_drivers(p(dir));
    assert_eq!(report.incomplete, None, "{report:?}");
    report
}

fn finding<'a>(report: &'a DriverReport, name: &str) -> &'a DriverFinding {
    report
        .findings
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("没有 {name}：{report:?}"))
}

#[test]
fn missing_driver_found_with_samples() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n*.prefab merge=unityyamlmerge\n");
    let r = check(dir.path());
    let f = finding(&r, "unityyamlmerge");
    assert_eq!(f.state, DriverState::Missing);
    assert!(
        f.sample_paths
            .contains(&"Assets/Scenes/Main.unity".to_string()),
        "{f:?}"
    );
    assert!(f.sample_paths.len() <= 3);
    assert_eq!(r.findings.len(), 1);
}

#[test]
fn sample_paths_are_capped_at_three() {
    let dir = repo();
    write(dir.path(), ".gitattributes", b"*.unity merge=uym\n");
    for i in 0..5 {
        write(dir.path(), &format!("S{i}.unity"), b"x\n");
    }
    commit_all(dir.path(), "c1");
    assert_eq!(finding(&check(dir.path()), "uym").sample_paths.len(), 3);
}

#[test]
fn incomplete_driver() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    git(
        dir.path(),
        &["config", "merge.unityyamlmerge.name", "Unity"],
    );
    let r = check(dir.path());
    assert_eq!(finding(&r, "unityyamlmerge").state, DriverState::Incomplete);
}

#[test]
fn program_missing() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    git(
        dir.path(),
        &[
            "config",
            "merge.unityyamlmerge.driver",
            "C:/nope/NotThere.exe merge -p %O %B %A %A",
        ],
    );
    let r = check(dir.path());
    assert_eq!(
        finding(&r, "unityyamlmerge").state,
        DriverState::ProgramMissing {
            program: "C:/nope/NotThere.exe".into()
        }
    );
}

/// 带引号的路径（Unity 装在 Program Files 下，路径有空格）取引号里的整段。
#[test]
fn quoted_program_path_with_spaces() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    git(
        dir.path(),
        &[
            "config",
            "merge.unityyamlmerge.driver",
            "'C:/Program Files/nope dir/UnityYAMLMerge.exe' merge -h -p %O %B %A %A",
        ],
    );
    assert_eq!(
        finding(&check(dir.path()), "unityyamlmerge").state,
        DriverState::ProgramMissing {
            program: "C:/Program Files/nope dir/UnityYAMLMerge.exe".into()
        }
    );
}

#[test]
fn ok_driver() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    let exe = std::env::current_exe().unwrap();
    let exe = exe.to_str().unwrap().replace('\\', "/");
    git(
        dir.path(),
        &[
            "config",
            "merge.unityyamlmerge.driver",
            &format!("\"{exe}\" %O %A %B"),
        ],
    );
    assert_eq!(
        finding(&check(dir.path()), "unityyamlmerge").state,
        DriverState::Ok
    );
}

/// 不是绝对路径（靠 PATH 找）的程序不检查，算 Ok。
#[test]
fn relative_program_is_not_checked() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    git(
        dir.path(),
        &[
            "config",
            "merge.unityyamlmerge.driver",
            "no-such-tool %O %A %B",
        ],
    );
    assert_eq!(
        finding(&check(dir.path()), "unityyamlmerge").state,
        DriverState::Ok
    );
}

/// 子目录覆盖了根目录的规则：被覆盖掉的旧名字不能报出来。
#[test]
fn overridden_rule_not_reported() {
    let dir = repo();
    write(dir.path(), ".gitattributes", b"*.unity merge=old\n");
    write(
        dir.path(),
        "Assets/.gitattributes",
        b"*.unity merge=binary\n",
    );
    write(dir.path(), "Assets/Main.unity", b"x\n");
    commit_all(dir.path(), "c1");
    let r = check(dir.path());
    assert!(r.findings.iter().all(|f| f.name != "old"), "{r:?}");
}

#[test]
fn macro_attr_counts() {
    let dir = unity_repo("[attr]unityscene merge=unityyamlmerge\n*.unity unityscene\n");
    assert_eq!(
        finding(&check(dir.path()), "unityyamlmerge").state,
        DriverState::Missing
    );
}

#[test]
fn info_attributes_counts() {
    let dir = unity_repo("");
    write(
        dir.path(),
        ".git/info/attributes",
        b"*.unity merge=fromInfo\n",
    );
    assert_eq!(
        finding(&check(dir.path()), "fromInfo").state,
        DriverState::Missing
    );
}

#[test]
fn core_attributes_file_counts() {
    let dir = unity_repo("");
    let attrs = TempDir::new().unwrap();
    let file = attrs.path().join("global attributes");
    fs::write(&file, b"*.unity merge=fromGlobal\n").unwrap();
    git(
        dir.path(),
        &["config", "core.attributesFile", file.to_str().unwrap()],
    );
    assert_eq!(
        finding(&check(dir.path()), "fromGlobal").state,
        DriverState::Missing
    );
}

#[test]
fn builtin_names_excluded() {
    let dir = repo();
    write(
        dir.path(),
        ".gitattributes",
        b"*.a merge=binary\n*.b -merge\n*.c merge\n*.d merge=text\n*.e merge=union\n*.f !merge\n",
    );
    for ext in ["a", "b", "c", "d", "e", "f", "g"] {
        write(dir.path(), &format!("x.{ext}"), b"x\n");
    }
    commit_all(dir.path(), "c1");
    assert!(check(dir.path()).findings.is_empty());
}

/// 完全没有 `merge.*` 配置（`git config` 退出码 1）是正常结果，不是检查失败。
#[test]
fn zero_config_matches_is_not_error() {
    let dir = unity_repo("");
    let r = merge_drivers::check_merge_drivers(p(dir.path()));
    assert_eq!(r.incomplete, None);
    assert!(r.findings.is_empty());
}

#[test]
fn merge_default_custom_is_checked() {
    let dir = unity_repo("");
    git(dir.path(), &["config", "merge.default", "mydefault"]);
    let f = check(dir.path());
    assert_eq!(finding(&f, "mydefault").state, DriverState::Missing);
}

#[test]
fn merge_default_builtin_is_not_reported() {
    let dir = unity_repo("");
    git(dir.path(), &["config", "merge.default", "binary"]);
    assert!(check(dir.path()).findings.is_empty());
}

#[test]
fn check_failure_is_reported_as_incomplete() {
    isolate_git_config();
    let not_repo = TempDir::new().unwrap();
    let r = merge_drivers::check_merge_drivers(p(not_repo.path()));
    assert!(r.incomplete.is_some(), "{r:?}");
}

/// `.gitattributes` 只在 linked worktree 检出的分支上：只有在那个 worktree 里查才看得到。
#[test]
fn linked_worktree_check() {
    let dir = repo();
    write(dir.path(), "seed.txt", b"seed\n");
    commit_all(dir.path(), "seed");
    let parent = TempDir::new().unwrap();
    let wt = parent.path().join("wt 目录");
    git(dir.path(), &["worktree", "add", "-q", "-b", "side", p(&wt)]);
    write(&wt, ".gitattributes", b"*.unity merge=sideonly\n");
    write(&wt, "Main.unity", b"x\n");
    commit_all(&wt, "side");
    assert_eq!(finding(&check(&wt), "sideonly").state, DriverState::Missing);
    assert!(check(dir.path()).findings.is_empty());
}

#[test]
fn cache_hits_until_gitattributes_changes() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    let first = check(dir.path());
    let n = merge_drivers::check_count(p(dir.path()));
    assert_eq!(n, 1);

    // 存一次普通文件不该触发重新扫描
    write(dir.path(), "Code/A.cs", b"class A { int x; }\n");
    let second = check(dir.path());
    assert_eq!(
        merge_drivers::check_count(p(dir.path())),
        1,
        "第二次应命中缓存"
    );
    assert_eq!(first, second);

    // 改了 .gitattributes（还没提交）：工作区的 mtime 变了，重新算
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(
        dir.path(),
        ".gitattributes",
        b"*.unity merge=unityyamlmerge\n*.prefab merge=prefabmerge\n",
    );
    let third = check(dir.path());
    assert_eq!(merge_drivers::check_count(p(dir.path())), 2);
    assert_eq!(finding(&third, "prefabmerge").state, DriverState::Missing);
}

#[test]
fn cache_invalidated_by_config_change() {
    let dir = unity_repo("*.unity merge=unityyamlmerge\n");
    assert_eq!(
        finding(&check(dir.path()), "unityyamlmerge").state,
        DriverState::Missing
    );
    git(
        dir.path(),
        &["config", "merge.unityyamlmerge.name", "Unity"],
    );
    assert_eq!(
        finding(&check(dir.path()), "unityyamlmerge").state,
        DriverState::Incomplete
    );
    assert_eq!(merge_drivers::check_count(p(dir.path())), 2);
}

/// 第一次没有任何场景文件，后来加了一个：规则没变也要重新算，否则缺 driver 永远报不出来。
#[test]
fn cache_invalidated_by_new_tracked_file() {
    let dir = repo();
    write(dir.path(), ".gitattributes", b"*.unity merge=uym\n");
    write(dir.path(), "a.txt", b"x\n");
    commit_all(dir.path(), "c1");
    assert!(check(dir.path()).findings.is_empty());
    write(dir.path(), "Main.unity", b"x\n");
    git(dir.path(), &["add", "Main.unity"]);
    assert_eq!(
        finding(&check(dir.path()), "uym").state,
        DriverState::Missing
    );
}

/// 实测：`git merge-tree --write-tree`（参数同 `svn_deliver.rs` 的 replay）和 `git stash apply`
/// 都会调用自定义 merge driver。所以交付慢路径和 stash 也要算进「会触发合并的入口」。
#[test]
fn merge_tree_and_stash_apply_invoke_driver() {
    let dir = repo();
    let trace = dir.path().join("..").join(format!(
        "driver-trace-{}.txt",
        dir.path().file_name().unwrap().to_string_lossy()
    ));
    let trace_sh = trace.to_str().unwrap().replace('\\', "/");
    // 假 driver：留下痕迹，把 ours 原样当结果，以 0 退出
    git(
        dir.path(),
        &[
            "config",
            "merge.tracer.driver",
            &format!("echo called >> '{trace_sh}'; exit 0"),
        ],
    );
    write(dir.path(), ".gitattributes", b"*.txt merge=tracer\n");
    write(dir.path(), "a.txt", b"line1\nline2\n");
    commit_all(dir.path(), "base");
    let base = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    write(dir.path(), "a.txt", b"theirs\nline2\n");
    commit_all(dir.path(), "theirs");
    let theirs = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "a.txt", b"ours\nline2\n");
    commit_all(dir.path(), "ours");
    let ours = git(dir.path(), &["rev-parse", "HEAD"]);

    let _ = fs::remove_file(&trace);
    let mb = format!("--merge-base={base}");
    git_out(
        dir.path(),
        &["merge-tree", "--write-tree", "-z", &mb, &ours, &theirs],
    );
    let after_merge_tree = fs::read_to_string(&trace).unwrap_or_default();
    assert!(
        after_merge_tree.contains("called"),
        "merge-tree 没有调用 driver"
    );

    let _ = fs::remove_file(&trace);
    write(dir.path(), "a.txt", b"stashed\nline2\n");
    git(dir.path(), &["stash", "push", "-q"]);
    write(dir.path(), "a.txt", b"other\nline2\n");
    commit_all(dir.path(), "move on");
    git_out(dir.path(), &["stash", "apply"]);
    let after_stash = fs::read_to_string(&trace).unwrap_or_default();
    let _ = fs::remove_file(&trace);
    assert!(
        after_stash.contains("called"),
        "stash apply 没有调用 driver"
    );
}

/// `git lfs track` 写的标准属性 `merge=lfs`：git-lfs 不定义这个 driver，不报。
#[test]
fn lfs_standard_attr_not_reported() {
    let dir =
        unity_repo("*.unity merge=unityyamlmerge\n*.png filter=lfs diff=lfs merge=lfs -text\n");
    write(dir.path(), "a.png", b"x\n");
    git(dir.path(), &["add", "a.png"]);
    let r = check(dir.path());
    assert!(r.findings.iter().all(|f| f.name != "lfs"), "{r:?}");
    assert_eq!(finding(&r, "unityyamlmerge").state, DriverState::Missing);
}
