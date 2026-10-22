use app_lib::engine::conflicts::{
    self, merge_config, path_attrs, scan_markers, ConflictCase, ConflictInfo, NotTextReason,
};
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

/// 用户本机的全局 / 系统 git 配置里可能有 `merge.*`（例如 git-lfs 装的 `merge.lfs-text`），
/// 会混进 `merge_config` 的结果。测试进程只认 fixture 自己的本地配置。
fn isolate_git_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let empty = Path::new(env!("CARGO_TARGET_TMPDIR")).join("empty-global-gitconfig");
        fs::write(&empty, b"").unwrap();
        std::env::set_var("GIT_CONFIG_GLOBAL", &empty);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
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

/// 造一次 merge 冲突：base 提交 → feat 改成 theirs → 当前分支改成 ours → merge feat。
/// `attrs` 在 base 里就提交，两边都看得到；`config` 写进本地配置。返回 merge 的退出码。
fn merge_conflict(
    dir: &Path,
    attrs: Option<&str>,
    config: &[(&str, &str)],
    file: &str,
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
) -> i32 {
    for (k, v) in config {
        git(dir, &["config", k, v]);
    }
    if let Some(a) = attrs {
        write(dir, ".gitattributes", a.as_bytes());
    }
    // 当前分支不一定是 main：linked worktree 里是另一个分支
    let home = git(dir, &["symbolic-ref", "--short", "HEAD"]);
    write(dir, file, base);
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "feat"]);
    write(dir, file, theirs);
    commit_all(dir, "theirs");
    git(dir, &["checkout", "-q", &home]);
    write(dir, file, ours);
    commit_all(dir, "ours");
    git_out(dir, &["merge", "feat"]).status.code().unwrap()
}

/// 最常见的形状：三份都是文本，两边都改了第 1 行。
fn text_conflict(dir: &Path, attrs: Option<&str>, config: &[(&str, &str)]) -> i32 {
    merge_conflict(
        dir,
        attrs,
        config,
        "a.txt",
        b"line1\nline2\n",
        b"ours1\nline2\n",
        b"theirs1\nline2\n",
    )
}

fn only(dir: &Path) -> ConflictInfo {
    let list = conflicts::read_conflicts(p(dir)).unwrap();
    assert_eq!(list.len(), 1, "{list:?}");
    list.into_iter().next().unwrap()
}

fn by_path(list: &[ConflictInfo], path: &str) -> ConflictInfo {
    list.iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("没有 {path}：{list:?}"))
        .clone()
}

// ---------- scan_markers ----------

#[test]
fn scan_markers_merge_style() {
    let text = b"a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> feat\nb\n";
    assert!(scan_markers(text, 7));
}

#[test]
fn scan_markers_diff3_style() {
    let text = b"<<<<<<< HEAD\nours\n||||||| base\nbase\n=======\ntheirs\n>>>>>>> feat\n";
    assert!(scan_markers(text, 7));
}

#[test]
fn scan_markers_half_removed() {
    assert!(scan_markers(b"x\n<<<<<<< HEAD\nours\n", 7));
    assert!(scan_markers(b"ours\n>>>>>>> x\n", 7));
    // 标记行后面直接是行尾也算
    assert!(scan_markers(b"ours\n>>>>>>>\n", 7));
    assert!(scan_markers(b"ours\n<<<<<<<", 7));
}

#[test]
fn scan_markers_ignores_lone_separators() {
    // Markdown / RST 的标题下划线
    assert!(!scan_markers(b"Title\n=======\nbody\n", 7));
    assert!(!scan_markers(b"x\n|||||||\ny\n", 7));
    assert!(!scan_markers(b"x\n||||||| base\ny\n", 7));
}

#[test]
fn scan_markers_respects_marker_size() {
    let long = format!("{} HEAD\nours\n", "<".repeat(32));
    assert!(scan_markers(long.as_bytes(), 32));
    assert!(!scan_markers(b"<<<<<<< HEAD\nours\n", 32));
    // 「至少」L 个：git 遇到改名冲突会把标记加长
    assert!(scan_markers(b"<<<<<<<<< HEAD\nours\n", 7));
}

#[test]
fn scan_markers_needs_space_or_eol_after_run() {
    assert!(!scan_markers(b"<<<<<<<x\n", 7));
    assert!(!scan_markers(b"  <<<<<<< HEAD\n", 7), "必须在行首");
    assert!(!scan_markers(b"<<<<<< HEAD\n", 7), "6 个不够");
}

#[test]
fn scan_markers_crlf() {
    assert!(scan_markers(b"a\r\n<<<<<<< HEAD\r\nours\r\n", 7));
    assert!(scan_markers(b"a\r\n>>>>>>>\r\n", 7));
}

// ---------- 属性与配置 ----------

#[test]
fn path_attrs_reads_merge_filter_and_marker_size() {
    let dir = repo();
    write(
        dir.path(),
        ".gitattributes",
        b"*.unity merge=unityyamlmerge conflict-marker-size=32\n*.bin filter=lfs -merge\n",
    );
    let attrs = path_attrs(p(dir.path()), &["a/Main.unity", "x.bin", "-dash [1].txt"]).unwrap();
    assert_eq!(attrs["a/Main.unity"].merge, "unityyamlmerge");
    assert_eq!(attrs["a/Main.unity"].marker_size, 32);
    assert_eq!(attrs["x.bin"].filter, "lfs");
    assert_eq!(attrs["x.bin"].merge, "unset");
    assert_eq!(attrs["-dash [1].txt"].merge, "unspecified");
    assert_eq!(attrs["-dash [1].txt"].marker_size, 7);
}

#[test]
fn path_attrs_empty_input_is_empty() {
    let dir = repo();
    assert!(path_attrs(p(dir.path()), &[]).unwrap().is_empty());
}

#[test]
fn merge_config_zero_matches_is_empty() {
    let dir = repo();
    let cfg = merge_config(p(dir.path())).unwrap();
    assert!(cfg.default.is_none());
    assert!(cfg.drivers.is_empty());
}

#[test]
fn merge_config_groups_drivers_and_takes_last_value() {
    let dir = repo();
    git(dir.path(), &["config", "merge.default", "binary"]);
    git(dir.path(), &["config", "merge.uym.name", "Unity"]);
    git(dir.path(), &["config", "merge.uym.driver", "old %A"]);
    git(
        dir.path(),
        &["config", "--add", "merge.uym.driver", "new %A"],
    );
    git(dir.path(), &["config", "merge.half.name", "Half"]);
    git(dir.path(), &["config", "merge.a.b.driver", "dotted %A"]);
    let cfg = merge_config(p(dir.path())).unwrap();
    assert_eq!(cfg.default.as_deref(), Some("binary"));
    assert_eq!(cfg.drivers["uym"].driver.as_deref(), Some("new %A"));
    assert!(cfg.drivers["uym"].has_other_keys);
    assert_eq!(cfg.drivers["half"].driver, None);
    assert!(cfg.drivers["half"].has_other_keys);
    assert_eq!(cfg.drivers["a.b"].driver.as_deref(), Some("dotted %A"));
    assert!(!cfg.drivers["a.b"].has_other_keys);
}

// ---------- spec §4.1 表 1 ----------

#[test]
fn no_attr_conflict_is_markers() {
    let dir = repo();
    assert_eq!(text_conflict(dir.path(), None, &[]), 1);
    let c = only(dir.path());
    assert_eq!(c.path, "a.txt");
    assert_eq!(c.code, "UU");
    assert_eq!(c.case, ConflictCase::Markers);
    assert_eq!(c.reason, None);
    assert!(c.base.is_some() && c.ours.is_some() && c.theirs.is_some());
    assert_eq!(c.ours.as_ref().unwrap().mode, 0o100644);
    assert!(!c.same_as_head);
}

#[test]
fn binary_attr_is_not_text_merged() {
    let dir = repo();
    assert_eq!(
        text_conflict(dir.path(), Some("*.txt merge=binary\n"), &[]),
        1
    );
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::BinaryAttr));
    assert_eq!(c.driver, None);
}

#[test]
fn unset_merge_attr_is_not_text_merged() {
    let dir = repo();
    assert_eq!(text_conflict(dir.path(), Some("*.txt -merge\n"), &[]), 1);
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::BinaryAttr));
}

/// 本机完全没配这个名字：git 退回逐行文本合并，写冲突标记。
#[test]
fn undefined_custom_driver_falls_back_to_markers() {
    let dir = repo();
    assert_eq!(
        text_conflict(dir.path(), Some("*.txt merge=unityyamlmerge\n"), &[]),
        1
    );
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::Markers);
    assert_eq!(c.reason, None);
}

#[test]
fn failing_custom_driver_is_not_text_merged() {
    let dir = repo();
    let code = text_conflict(
        dir.path(),
        Some("*.txt merge=faildriver\n"),
        &[(
            "merge.faildriver.driver",
            "echo driver-left-this > %A; exit 1",
        )],
    );
    assert_eq!(code, 1);
    let work = fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert!(work.contains("driver-left-this"), "{work}");
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::CustomDriver));
    assert_eq!(c.driver.as_deref(), Some("faildriver"));
}

#[test]
fn missing_driver_program_is_not_text_merged() {
    let dir = repo();
    let code = text_conflict(
        dir.path(),
        Some("*.txt merge=missingexe\n"),
        &[("merge.missingexe.driver", "C:/nope/NotThere.exe %O %A %B")],
    );
    assert_eq!(code, 1);
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::CustomDriver));
    assert_eq!(c.driver.as_deref(), Some("missingexe"));
    // driver 失败时工作区保持 ours 原样
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "ours1\nline2\n"
    );
}

#[test]
fn merge_default_custom_is_not_text_merged() {
    let dir = repo();
    let code = text_conflict(
        dir.path(),
        None,
        &[
            ("merge.default", "faildriver"),
            ("merge.faildriver.driver", "echo x > %A; exit 1"),
        ],
    );
    assert_eq!(code, 1);
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::MergeDefault));
    assert_eq!(c.driver.as_deref(), Some("faildriver"));
}

#[test]
fn merge_default_binary_is_not_text_merged() {
    let dir = repo();
    assert_eq!(
        text_conflict(dir.path(), None, &[("merge.default", "binary")]),
        1
    );
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::MergeDefault));
    assert_eq!(c.driver, None);
}

/// 显式写了 `merge=text` 时，`merge.default` 不生效。
#[test]
fn explicit_text_attr_overrides_merge_default() {
    let dir = repo();
    assert_eq!(
        text_conflict(
            dir.path(),
            Some("*.txt merge\n"),
            &[("merge.default", "binary")]
        ),
        1
    );
    assert_eq!(only(dir.path()).case, ConflictCase::Markers);
}

#[test]
fn binary_base_only_is_not_text_merged() {
    let dir = repo();
    let code = merge_conflict(
        dir.path(),
        None,
        &[],
        "a.txt",
        b"base\0bytes\n",
        b"ours\n",
        b"theirs\n",
    );
    assert_eq!(code, 1);
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::BinaryContent));
}

/// 二进制判定优先于属性：④c 不给外部 merge 入口，不能被 ④a 盖掉。
#[test]
fn binary_content_wins_over_binary_attr() {
    let dir = repo();
    let code = merge_conflict(
        dir.path(),
        Some("*.txt merge=binary\n"),
        &[],
        "a.txt",
        b"base\n",
        b"ours\0\n",
        b"theirs\n",
    );
    assert_eq!(code, 1);
    assert_eq!(only(dir.path()).reason, Some(NotTextReason::BinaryContent));
}

#[test]
fn lfs_filter_is_not_text_merged() {
    let dir = repo();
    // 只写属性、不装 lfs 过滤器：git 把它当普通文本合并，内容仍然有冲突标记。
    // 判定只看属性，按最保守的 ④ 处理
    assert_eq!(
        text_conflict(dir.path(), Some("*.txt filter=lfs\n"), &[]),
        1
    );
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::Lfs));
}

#[test]
fn cleared_markers_is_no_markers() {
    let dir = repo();
    assert_eq!(text_conflict(dir.path(), None, &[]), 1);
    write(dir.path(), "a.txt", b"hand merged\nline2\n");
    let c = only(dir.path());
    assert_eq!(c.case, ConflictCase::NoMarkers);
    assert_eq!(c.code, "UU", "不自动暂存，index 仍是冲突");
}

#[test]
fn marker_size_attr_is_respected() {
    let dir = repo();
    assert_eq!(
        text_conflict(dir.path(), Some("*.txt conflict-marker-size=32\n"), &[]),
        1
    );
    let work = fs::read_to_string(dir.path().join("a.txt")).unwrap();
    assert!(work.contains(&"<".repeat(32)), "{work}");
    assert_eq!(only(dir.path()).case, ConflictCase::Markers);
    // 用户把标记缩短成 7 个，仍不足 32，不算标记
    write(dir.path(), "a.txt", b"<<<<<<< HEAD\nx\n");
    assert_eq!(only(dir.path()).case, ConflictCase::NoMarkers);
}

#[test]
fn delete_modify_codes() {
    // feat 删、main 改 → stage {1,2} → UD（deleted by them）
    let dir = repo();
    write(dir.path(), "d.txt", b"keep\n");
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    git(dir.path(), &["rm", "-q", "d.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "del"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "d.txt", b"changed\n");
    commit_all(dir.path(), "mod");
    git_out(dir.path(), &["merge", "feat"]);
    let c = only(dir.path());
    assert_eq!(c.code, "UD");
    assert_eq!(c.case, ConflictCase::DeleteModify);
    assert!(c.theirs.is_none());

    // 反过来：main 删、feat 改 → stage {1,3} → DU
    let dir = repo();
    write(dir.path(), "d.txt", b"keep\n");
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    write(dir.path(), "d.txt", b"changed\n");
    commit_all(dir.path(), "mod");
    git(dir.path(), &["checkout", "-q", "main"]);
    git(dir.path(), &["rm", "-q", "d.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "del"]);
    git_out(dir.path(), &["merge", "feat"]);
    let c = only(dir.path());
    assert_eq!(c.code, "DU");
    assert_eq!(c.case, ConflictCase::DeleteModify);
    assert!(c.ours.is_none());
}

#[test]
fn add_add_conflict_uses_aa() {
    let dir = repo();
    write(dir.path(), "seed.txt", b"seed\n");
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    write(dir.path(), "new.txt", b"theirs\n");
    commit_all(dir.path(), "theirs");
    git(dir.path(), &["checkout", "-q", "main"]);
    write(dir.path(), "new.txt", b"ours\n");
    commit_all(dir.path(), "ours");
    git_out(dir.path(), &["merge", "feat"]);
    let c = only(dir.path());
    assert_eq!(c.code, "AA");
    assert!(c.base.is_none());
    assert_eq!(c.case, ConflictCase::Markers);
}

/// rename/rename（一改二）：git 2.51 留下 a.txt 只有 stage 1（DD）、
/// b.txt 只有 stage 2（AU）、c.txt 只有 stage 3（UA）。
#[test]
fn structural_codes() {
    let dir = repo();
    write(dir.path(), "a.txt", "same line\n".repeat(20).as_bytes());
    commit_all(dir.path(), "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    git(dir.path(), &["mv", "a.txt", "c.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "to c"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    git(dir.path(), &["mv", "a.txt", "b.txt"]);
    git(dir.path(), &["commit", "-q", "-m", "to b"]);
    git_out(dir.path(), &["merge", "feat"]);
    let list = conflicts::read_conflicts(p(dir.path())).unwrap();
    assert_eq!(list.len(), 3, "{list:?}");
    for (path, code) in [("a.txt", "DD"), ("b.txt", "AU"), ("c.txt", "UA")] {
        let c = by_path(&list, path);
        assert_eq!(c.code, code, "{path}");
        assert_eq!(c.case, ConflictCase::Structural, "{path}");
    }
}

/// 符号链接冲突：直接往 index 里写 mode 120000 的条目，不需要建链接的权限。
#[test]
fn symlink_conflict_is_structural() {
    let dir = repo();
    let link_commit = |target: &str, msg: &str| {
        let out = Command::new("git")
            .current_dir(dir.path())
            .args(["hash-object", "-w", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(target.as_bytes())?;
                child.wait_with_output()
            })
            .unwrap();
        let oid = String::from_utf8(out.stdout).unwrap().trim().to_string();
        git(
            dir.path(),
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("120000,{oid},link"),
            ],
        );
        git(dir.path(), &["commit", "-q", "-m", msg]);
    };
    link_commit("../outside-base", "base");
    git(dir.path(), &["checkout", "-q", "-b", "feat"]);
    link_commit("../outside-theirs", "theirs");
    git(dir.path(), &["checkout", "-q", "main"]);
    link_commit("../outside-ours", "ours");
    git_out(dir.path(), &["merge", "feat"]);
    let c = only(dir.path());
    assert_eq!(c.code, "UU");
    assert_eq!(c.ours.as_ref().unwrap().mode, 0o120000);
    assert_eq!(c.case, ConflictCase::Structural);
}

/// 只配了 `name` 没配 `driver`：git fatal，整个 merge 中止，什么冲突都没有。
#[test]
fn name_only_driver_aborts_merge() {
    let dir = repo();
    let code = text_conflict(
        dir.path(),
        Some("*.txt merge=halfdriver\n"),
        &[("merge.halfdriver.name", "Half")],
    );
    assert_eq!(code, 128);
    assert!(
        git_out(dir.path(), &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .stdout
            .is_empty()
    );
    assert!(conflicts::read_conflicts(p(dir.path())).unwrap().is_empty());
}

#[test]
fn same_as_head_flag() {
    let dir = repo();
    assert_eq!(
        text_conflict(dir.path(), Some("*.txt merge=binary\n"), &[]),
        1
    );
    assert!(
        only(dir.path()).same_as_head,
        "merge=binary 时工作区就是 HEAD"
    );
    write(dir.path(), "a.txt", b"hand merged\n");
    assert!(!only(dir.path()).same_as_head);
}

/// autocrlf 下工作区是 CRLF、HEAD 的 blob 是 LF，仍要判成相同。
#[test]
fn same_as_head_survives_autocrlf() {
    let dir = repo();
    git(dir.path(), &["config", "core.autocrlf", "true"]);
    assert_eq!(
        text_conflict(dir.path(), Some("*.txt merge=binary\n"), &[]),
        1
    );
    // git 不会重写它没改过的工作区文件，这里手工换成 checkout 出来的 CRLF 形态
    write(dir.path(), "a.txt", b"ours1\r\nline2\r\n");
    assert!(only(dir.path()).same_as_head);
    write(dir.path(), "a.txt", b"ours1\r\nline2 x\r\n");
    assert!(!only(dir.path()).same_as_head);
}

#[test]
fn no_conflicts_is_empty() {
    let dir = repo();
    write(dir.path(), "a.txt", b"x\n");
    commit_all(dir.path(), "c1");
    write(dir.path(), "a.txt", b"y\n");
    assert!(conflicts::read_conflicts(p(dir.path())).unwrap().is_empty());
}

/// 属性要在 linked worktree 里解析：`.gitattributes` 只在 feature 那边有，
/// 主工作区没有冲突、也看不到这条属性。
#[test]
fn conflicts_in_linked_worktree() {
    let dir = repo();
    write(dir.path(), "seed.txt", b"seed\n");
    commit_all(dir.path(), "seed");
    let wt_parent = TempDir::new().unwrap();
    let wt = wt_parent.path().join("wt dir");
    git(
        dir.path(),
        &["worktree", "add", "-q", "-b", "side", wt.to_str().unwrap()],
    );
    assert_eq!(text_conflict(&wt, Some("*.txt merge=binary\n"), &[]), 1);
    let c = only(&wt);
    assert_eq!(c.case, ConflictCase::NotTextMerged);
    assert_eq!(c.reason, Some(NotTextReason::BinaryAttr));
    assert!(conflicts::read_conflicts(p(dir.path())).unwrap().is_empty());
}

#[test]
fn chinese_space_bracket_dash_path() {
    let dir = repo();
    let file = "目录 a/-f[1] x.txt";
    let code = merge_conflict(
        dir.path(),
        Some("*.txt merge=binary\n"),
        &[],
        file,
        b"base\n",
        b"ours\n",
        b"theirs\n",
    );
    assert_eq!(code, 1);
    let c = only(dir.path());
    assert_eq!(c.path, file);
    assert_eq!(c.reason, Some(NotTextReason::BinaryAttr));
    assert!(c.same_as_head);
}

// ---------- ③ 保留 / 删除 ----------

/// 一边删一边改。`we_delete` 为真时 main 删、feat 改（DU），否则 main 改、feat 删（UD）。
/// 删除一律用 literal pathspec：`git rm 'f[1].txt'` 会把 `f1.txt` 一起删掉。
fn delete_modify(dir: &Path, file: &str, we_delete: bool) {
    write(dir, file, b"keep\n");
    commit_all(dir, "base");
    git(dir, &["checkout", "-q", "-b", "feat"]);
    let delete = |dir: &Path| {
        git(dir, &["--literal-pathspecs", "rm", "-q", "--", file]);
        git(dir, &["commit", "-q", "-m", "del"]);
    };
    let modify = |dir: &Path| {
        write(dir, file, b"changed\n");
        commit_all(dir, "mod");
    };
    // feat 做的是 main 的另一半
    if we_delete {
        modify(dir)
    } else {
        delete(dir)
    }
    git(dir, &["checkout", "-q", "main"]);
    if we_delete {
        delete(dir)
    } else {
        modify(dir)
    }
    assert!(!git_out(dir, &["merge", "feat"]).status.success());
}

fn unmerged(dir: &Path) -> String {
    git(dir, &["ls-files", "-u"])
}

#[test]
fn keep_resolves_du() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", true);
    assert_eq!(only(dir.path()).code, "DU");
    conflicts::resolve_delete_modify(p(dir.path()), "d.txt", true).unwrap();
    assert_eq!(unmerged(dir.path()), "");
    assert_eq!(git(dir.path(), &["show", ":d.txt"]), "changed");
    assert_eq!(git(dir.path(), &["status", "--short"]), "A  d.txt");
}

#[test]
fn delete_resolves_ud() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", false);
    assert_eq!(only(dir.path()).code, "UD");
    conflicts::resolve_delete_modify(p(dir.path()), "d.txt", false).unwrap();
    assert_eq!(unmerged(dir.path()), "");
    assert!(!dir.path().join("d.txt").exists());
    assert_eq!(git(dir.path(), &["status", "--short"]), "D  d.txt");
}

#[test]
fn keep_resolves_ud_and_delete_resolves_du() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", false);
    conflicts::resolve_delete_modify(p(dir.path()), "d.txt", true).unwrap();
    assert_eq!(unmerged(dir.path()), "");
    assert_eq!(git(dir.path(), &["show", ":d.txt"]), "changed");

    let dir = repo();
    delete_modify(dir.path(), "d.txt", true);
    conflicts::resolve_delete_modify(p(dir.path()), "d.txt", false).unwrap();
    assert_eq!(unmerged(dir.path()), "");
    assert!(!dir.path().join("d.txt").exists());
}

/// `f[1].txt` 按 pathspec 模式会匹配到 `f1.txt`；必须按字面量处理，旁边的文件不受影响。
#[test]
fn literal_pathspec_spares_lookalike() {
    let dir = repo();
    write(dir.path(), "f1.txt", b"one\n");
    commit_all(dir.path(), "seed");
    delete_modify(dir.path(), "f[1].txt", false);
    write(dir.path(), "f1.txt", b"one modified\n");
    let before = git(dir.path(), &["status", "--short", "--", ":(literal)f1.txt"]);
    assert_eq!(before, "M f1.txt");

    conflicts::resolve_delete_modify(p(dir.path()), "f[1].txt", false).unwrap();
    assert_eq!(
        fs::read(dir.path().join("f1.txt")).unwrap(),
        b"one modified\n"
    );
    assert_eq!(
        git(dir.path(), &["status", "--short", "--", ":(literal)f1.txt"]),
        before
    );
    assert_eq!(unmerged(dir.path()), "");

    // keep 同理：不能把 f1.txt 的改动一起暂存
    let dir = repo();
    write(dir.path(), "f1.txt", b"one\n");
    commit_all(dir.path(), "seed");
    delete_modify(dir.path(), "f[1].txt", true);
    write(dir.path(), "f1.txt", b"one modified\n");
    conflicts::resolve_delete_modify(p(dir.path()), "f[1].txt", true).unwrap();
    assert_eq!(
        git(dir.path(), &["status", "--short", "--", ":(literal)f1.txt"]),
        "M f1.txt"
    );
}

#[test]
fn dash_and_chinese_path_resolves() {
    let dir = repo();
    let file = "目录 a/-x [1].txt";
    delete_modify(dir.path(), file, true);
    conflicts::resolve_delete_modify(p(dir.path()), file, true).unwrap();
    assert_eq!(unmerged(dir.path()), "");
}

#[test]
fn empty_path_rejected() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", false);
    for keep in [true, false] {
        let err = conflicts::resolve_delete_modify(p(dir.path()), "", keep).unwrap_err();
        assert_eq!(err.code, "empty_path");
    }
    assert_ne!(unmerged(dir.path()), "", "什么都不能动");
}

#[test]
fn escaping_path_rejected() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", false);
    for bad in ["../d.txt", "C:/d.txt", "/d.txt"] {
        assert!(conflicts::resolve_delete_modify(p(dir.path()), bad, false).is_err());
    }
    assert_ne!(unmerged(dir.path()), "");
}

/// 执行前状态已经变了（在外面解决掉了）：拒绝，不再执行。
#[test]
fn state_changed_rejected() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", false);
    git(dir.path(), &["add", "d.txt"]);
    let err = conflicts::resolve_delete_modify(p(dir.path()), "d.txt", false).unwrap_err();
    assert_eq!(err.code, "conflict_state_changed");
    assert!(dir.path().join("d.txt").exists(), "不能删掉已经解决的文件");
}

/// 不是一边删一边改的冲突（UU）也不能走这条路。
#[test]
fn not_delete_modify_rejected() {
    let dir = repo();
    assert_eq!(text_conflict(dir.path(), None, &[]), 1);
    let err = conflicts::resolve_delete_modify(p(dir.path()), "a.txt", true).unwrap_err();
    assert_eq!(err.code, "conflict_state_changed");
    assert_ne!(unmerged(dir.path()), "");
}

#[test]
fn keep_missing_file_rejected() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", true);
    fs::remove_file(dir.path().join("d.txt")).unwrap();
    let err = conflicts::resolve_delete_modify(p(dir.path()), "d.txt", true).unwrap_err();
    assert_eq!(err.code, "conflict_state_changed");
    assert_ne!(unmerged(dir.path()), "");
}

#[test]
fn refused_while_sync_lock_held() {
    let dir = repo();
    delete_modify(dir.path(), "d.txt", false);
    let _lock = app_lib::engine::svn_lock::acquire(p(dir.path()), "test-op").unwrap();
    let err = conflicts::resolve_delete_modify(p(dir.path()), "d.txt", false).unwrap_err();
    assert_eq!(err.code, "sync_in_progress");
    assert_ne!(unmerged(dir.path()), "");
    assert!(dir.path().join("d.txt").exists());
}
