//! 建库向导第 2 步：查出「起作用的」`.gitignore`（2026-10-28 设计 §3）。
//!
//! 「起作用」= 挡住了 SVN 里有的 W-file（少收），或用 `!` 把用户没选的文件纳入（多收）。
//! 这一步还没有 `.git`，检测必须只读，不能在工作副本里留下任何东西。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_config::DisabledGitignore;
use app_lib::engine::svn_gitignore::{
    disable_gitignore, display_path, restore_gitignores, GitignoreRule, RuleEffect,
};
use app_lib::engine::svn_init::init_preflight;
use std::process::Command;
use svn_fixtures::{svn_cmd, svn_repo, SvnFixture};

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// 写文件、`svn add`、提交、更新——让它成为「SVN 里有的」文件。
fn commit(fx: &SvnFixture, files: &[(&str, &str)]) {
    let wc = fx.wc();
    for (rel, content) in files {
        let p = wc.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
    }
    // 从根上递归 add，而不是逐个文件给路径：这台机器上 svn 收到的命令行参数里，
    // 中文路径会被转成 `??`（E720123），参数只用 ASCII 才稳
    svn_cmd(&wc, &["add", "--force", "--non-interactive", "."]);
    svn_cmd(&wc, &["commit", "-m", "more", "--non-interactive"]);
    svn_cmd(&wc, &["update", "--non-interactive"]);
}

fn write(fx: &SvnFixture, rel: &str, content: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn excludes(line: u32, pattern: &str, samples: &[&str]) -> GitignoreRule {
    GitignoreRule {
        line,
        pattern: pattern.to_string(),
        effect: RuleEffect::Excludes,
        count: samples.len(),
        samples: strs(samples),
    }
}

#[test]
fn no_gitignore_means_no_conflict() {
    let fx = svn_repo();
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert!(
        p.gitignore_conflicts.is_empty(),
        "{:?}",
        p.gitignore_conflicts
    );
    assert!(p.preflight.node_rev >= 1);
}

#[test]
fn an_unversioned_gitignore_hiding_versioned_files_is_reported_with_full_path_line_and_samples() {
    let fx = svn_repo();
    commit(&fx, &[("Code/a.exe", "a"), ("Code/b.exe", "b")]);
    write(&fx, "Code/.gitignore", "# 旧的本地规则\n*.exe\n");

    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert_eq!(
        p.gitignore_conflicts.len(),
        1,
        "{:?}",
        p.gitignore_conflicts
    );
    let c = &p.gitignore_conflicts[0];
    assert_eq!(c.path, "Code/.gitignore");
    assert_eq!(c.full_path, display_path(&fx.wc_str(), "Code/.gitignore"));
    assert!(
        c.full_path.ends_with(r"\Code\.gitignore"),
        "{}",
        c.full_path
    );
    assert!(!c.versioned);
    assert_eq!(
        c.rules,
        vec![excludes(2, "*.exe", &["Code/a.exe", "Code/b.exe"])]
    );
}

#[test]
fn samples_are_capped_but_the_count_is_exact() {
    let fx = svn_repo();
    let files: Vec<(String, &str)> = (0..5).map(|i| (format!("Code/t{i}.exe"), "x")).collect();
    let refs: Vec<(&str, &str)> = files.iter().map(|(p, c)| (p.as_str(), *c)).collect();
    commit(&fx, &refs);
    write(&fx, "Code/.gitignore", "*.exe\n");

    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    let rule = &p.gitignore_conflicts[0].rules[0];
    assert_eq!(rule.count, 5);
    assert_eq!(
        rule.samples,
        strs(&["Code/t0.exe", "Code/t1.exe", "Code/t2.exe"])
    );
}

#[test]
fn a_gitignore_that_only_hides_unversioned_junk_is_not_a_conflict() {
    // 只挡本地编译产物的 .gitignore 不算：那些文件本来就不进基线；改它的名反而会让
    // 这些垃圾文件全部冒进「本地改动」列表（用户 2026-10-28 确认）
    let fx = svn_repo();
    write(&fx, "Code/x.obj", "junk");
    write(&fx, "Code/.gitignore", "*.obj\n");
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert!(
        p.gitignore_conflicts.is_empty(),
        "{:?}",
        p.gitignore_conflicts
    );
}

#[test]
fn a_versioned_gitignore_is_flagged_versioned() {
    let fx = svn_repo();
    commit(&fx, &[("Code/a.exe", "a"), ("Code/.gitignore", "*.exe\n")]);
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert_eq!(
        p.gitignore_conflicts.len(),
        1,
        "{:?}",
        p.gitignore_conflicts
    );
    assert!(p.gitignore_conflicts[0].versioned);
}

#[test]
fn a_negation_that_admits_unselected_files_is_reported_as_includes() {
    let fx = svn_repo();
    write(&fx, "Code/n.txt", "n");
    write(&fx, "Code/.gitignore", "!*.txt\n");
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&[".cs"])).unwrap();
    assert_eq!(
        p.gitignore_conflicts.len(),
        1,
        "{:?}",
        p.gitignore_conflicts
    );
    assert_eq!(
        p.gitignore_conflicts[0].rules,
        vec![GitignoreRule {
            line: 1,
            pattern: "!*.txt".to_string(),
            effect: RuleEffect::Includes,
            count: 1,
            samples: strs(&["Code/n.txt"]),
        }]
    );
}

#[test]
fn a_nested_gitignore_reports_its_own_location() {
    // git 报的是 .gitignore 的实际位置，可能在任意一层子目录，不是假设在顶层
    let fx = svn_repo();
    commit(&fx, &[("Code/Sub/a.exe", "a")]);
    write(&fx, "Code/Sub/.gitignore", "*.exe\n");
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert_eq!(
        p.gitignore_conflicts.len(),
        1,
        "{:?}",
        p.gitignore_conflicts
    );
    assert_eq!(p.gitignore_conflicts[0].path, "Code/Sub/.gitignore");
    assert!(p.gitignore_conflicts[0]
        .full_path
        .ends_with(r"\Code\Sub\.gitignore"));
}

#[test]
fn a_gitignore_hidden_behind_another_is_listed_too() {
    // git 不进入已被忽略的目录：`Code/.gitignore` 写 `Sub/` 时，`Code/Sub/.gitignore` 根本
    // 不会被读。外层改名停用之后它才开始起作用——第 2 步必须把它一起列出来，否则用户在
    // 第 3 步看不到它，建库时被送回第 3 步，再查还是看不到，死循环
    // （2026-10-29 真实窗口验证发现）。
    let fx = svn_repo();
    commit(&fx, &[("Code/Sub/X.cs", "x")]);
    write(&fx, "Code/.gitignore", "Sub/\n");
    write(&fx, "Code/Sub/.gitignore", "*.cs\n");

    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    let paths: Vec<(&str, bool)> = p
        .gitignore_conflicts
        .iter()
        .map(|c| (c.path.as_str(), c.active_after_rename))
        .collect();
    assert_eq!(
        paths,
        vec![("Code/.gitignore", false), ("Code/Sub/.gitignore", true)],
        "{:?}",
        p.gitignore_conflicts
    );
    assert_eq!(
        p.gitignore_conflicts[1].rules,
        vec![excludes(1, "*.cs", &["Code/Sub/X.cs"])]
    );
    // 模拟停用只发生在临时目录里，工作副本原样
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/.gitignore")).unwrap(),
        "Sub/\n"
    );
    assert!(!fx.wc().join(".git").exists());
}

#[test]
fn probe_and_repo_agree_on_case_insensitive_rules() {
    // Review Focus 第 2 条：第 2 步的探针与建库后的真仓库对大小写必须给同样的裁决，
    // 否则第 2 步看到的清单与建库时拦下的不一致，用户会在第 3 步来回打转。
    let fx = svn_repo();
    commit(&fx, &[("Code/a.exe", "a"), ("Code/Up.CS", "u")]);
    write(&fx, "Code/.gitignore", "*.EXE\n*.cs\n");
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&[".cs", ".exe"])).unwrap();
    let rules = &p.gitignore_conflicts[0].rules;
    let total: usize = rules.iter().map(|r| r.count).sum();
    // A.cs、B.cs、Up.CS、a.exe：大小写不同的扩展名与规则都要命中
    assert_eq!(total, 4, "{rules:?}");

    // 对照：一个真的 git 仓库在同一台机器上的裁决
    let real = tempfile::TempDir::new().unwrap();
    let ok = |args: &[&str]| {
        Command::new("git")
            .current_dir(real.path())
            .args(args)
            .status()
            .unwrap()
            .success()
    };
    assert!(ok(&["init", "-q"]));
    std::fs::write(real.path().join(".gitignore"), "*.EXE\n").unwrap();
    std::fs::write(real.path().join("a.exe"), "a").unwrap();
    assert!(ok(&["check-ignore", "-q", "a.exe"]), "真仓库也应当挡住它");
}

#[test]
fn conflicts_keep_non_ascii_paths_verbatim_and_still_tell_versioned_ones() {
    // Review Focus 第 3 条。白名单本身用 ASCII：svn 的命令行参数走系统 ANSI 代码页，
    // 代码页不含中文的机器上（实测 ACP 1252）中文白名单根连 P1–P4 都过不了——那是既有
    // 的环境限制，与本条无关。这里验的是白名单**底下**的中文路径。
    //
    // 让这份 `.gitignore` 受版本控制：若「是否受控」靠把中文路径传给 svn 来问，在这类
    // 机器上会被误判成不受控，然后被改名——一个团队提交的文件。
    let fx = svn_repo();
    commit(
        &fx,
        &[
            ("Code/资源/图.exe", "x"),
            ("Code/资源/.gitignore", "*.exe\n"),
        ],
    );
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert_eq!(
        p.gitignore_conflicts.len(),
        1,
        "{:?}",
        p.gitignore_conflicts
    );
    let c = &p.gitignore_conflicts[0];
    assert_eq!(c.path, "Code/资源/.gitignore");
    assert!(
        c.full_path.ends_with(r"\Code\资源\.gitignore"),
        "{}",
        c.full_path
    );
    assert_eq!(c.rules[0].samples, strs(&["Code/资源/图.exe"]));
    assert!(c.versioned, "中文目录里受控的 .gitignore 被判成了不受控");
}

#[test]
fn preflight_detection_leaves_no_trace_in_the_working_copy() {
    // 探针目录自己删没删干净，由 `svn_gitignore` 模块内的单元测试钉住——在这里按前缀
    // 数系统临时目录，会被并行跑的其它用例干扰。
    let fx = svn_repo();
    commit(&fx, &[("Code/a.exe", "a")]);
    write(&fx, "Code/.gitignore", "*.exe\n");
    let p = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["*"])).unwrap();
    assert_eq!(p.gitignore_conflicts.len(), 1);
    assert!(
        !fx.wc().join(".git").exists(),
        "检测不能在工作副本里建 .git"
    );
    let st = Command::new("svn")
        .current_dir(fx.wc())
        .args(["status", "--non-interactive"])
        .output()
        .unwrap();
    let st = String::from_utf8_lossy(&st.stdout).to_string();
    let extra: Vec<&str> = st.lines().filter(|l| !l.ends_with(".gitignore")).collect();
    assert!(extra.is_empty(), "检测往工作副本里写了东西：{extra:?}");
}

#[test]
fn invalid_extensions_are_rejected_at_preflight() {
    let fx = svn_repo();
    let err = init_preflight(&fx.wc_str(), &strs(&["Code/"]), &strs(&["exe"])).unwrap_err();
    assert_eq!(err.code, "svn_config");
}

// ---------------------------------------------------------------------------
// 改名停用与复原（设计 §5）
// ---------------------------------------------------------------------------

fn read(root: &std::path::Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn disabling_renames_to_the_jkk_disabled_name_and_restoring_puts_it_back() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("Code")).unwrap();
    std::fs::write(root.join("Code/.gitignore"), "*.exe\n").unwrap();
    let r = root.to_str().unwrap();

    let to = disable_gitignore(r, "Code/.gitignore").unwrap();
    assert_eq!(to, "Code/.gitignore.jkk-disabled");
    assert!(!root.join("Code/.gitignore").exists());
    assert_eq!(read(root, &to), "*.exe\n");

    let failures = restore_gitignores(
        r,
        &[DisabledGitignore {
            from: "Code/.gitignore".to_string(),
            to,
        }],
    );
    assert!(failures.is_empty(), "{failures:?}");
    assert_eq!(read(root, "Code/.gitignore"), "*.exe\n");
    assert!(!root.join("Code/.gitignore.jkk-disabled").exists());
}

#[test]
fn disabling_never_overwrites_an_existing_disabled_file() {
    // Review Focus 第 4 条
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    std::fs::write(root.join(".gitignore"), "new\n").unwrap();
    std::fs::write(root.join(".gitignore.jkk-disabled"), "old1\n").unwrap();
    std::fs::write(root.join(".gitignore.jkk-disabled-2"), "old2\n").unwrap();

    let to = disable_gitignore(root.to_str().unwrap(), ".gitignore").unwrap();
    assert_eq!(to, ".gitignore.jkk-disabled-3");
    assert_eq!(read(root, ".gitignore.jkk-disabled"), "old1\n");
    assert_eq!(read(root, ".gitignore.jkk-disabled-2"), "old2\n");
    assert_eq!(read(root, &to), "new\n");
}

#[test]
fn restore_never_overwrites_a_gitignore_that_reappeared() {
    // Review Focus 第 5 条：回退时原位置又出现了一个新的 .gitignore，不能覆盖它
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let r = root.to_str().unwrap();
    std::fs::write(root.join(".gitignore"), "mine\n").unwrap();
    let to = disable_gitignore(r, ".gitignore").unwrap();
    std::fs::write(root.join(".gitignore"), "someone else\n").unwrap();

    let failures = restore_gitignores(
        r,
        &[DisabledGitignore {
            from: ".gitignore".to_string(),
            to: to.clone(),
        }],
    );
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(
        failures[0].contains(".gitignore.jkk-disabled"),
        "{failures:?}"
    );
    assert_eq!(read(root, ".gitignore"), "someone else\n");
    assert_eq!(read(root, &to), "mine\n");
}
