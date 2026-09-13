//! exclude 规则生成的测试。
//!
//! **断言对象是 git 的实际行为（`git ls-files`），不是生成出来的规则文本**
//! ——测规则文本只能证明「生成了我以为的字符串」，证明不了 git 会怎么裁决，
//! 而这个模块存在的全部理由就是后者（spec §6.3）。

use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
use app_lib::engine::svn_include_check::{actual_set, verify_include_set};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
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
}

fn tracked(dir: &Path) -> Vec<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["ls-files"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// 一个模拟 Unity 布局的目录：白名单目录里有多层嵌套、干扰文件，
/// 还有一个**真实存在的 `.svn` 目录**（白名单落在 external 根时就是这样）。
fn workdir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    run_git(p, &["init"]);
    run_git(p, &["config", "user.email", "t@t"]);
    run_git(p, &["config", "user.name", "T"]);
    for sub in ["Code/Sub/Deep", "Code/.svn/pristine/aa", "Art"] {
        fs::create_dir_all(p.join(sub)).unwrap();
    }
    fs::write(p.join("Code/Top.cs"), "x").unwrap();
    fs::write(p.join("Code/Sub/Mid.cs"), "x").unwrap();
    fs::write(p.join("Code/Sub/Deep/Low.cs"), "x").unwrap();
    fs::write(p.join("Code/Junk.psd"), "x").unwrap();
    fs::write(p.join("Code/.svn/wc.db"), "x").unwrap();
    fs::write(p.join("Code/.svn/pristine/aa/d.svn-base"), "x").unwrap();
    fs::write(p.join("Art/boss.fbx"), "x").unwrap();
    dir
}

fn apply(dir: &Path, whitelist: &[&str], extensions: &[&str]) -> Vec<String> {
    let w: Vec<String> = whitelist.iter().map(|s| s.to_string()).collect();
    let e: Vec<String> = extensions.iter().map(|s| s.to_string()).collect();
    write_exclude(dir.to_str().unwrap(), &generate_exclude(&w, &e)).unwrap();
    run_git(dir, &["add", "-A"]);
    tracked(dir)
}

#[test]
fn nested_source_files_at_every_depth_are_tracked() {
    // spec §6.3：缺 `!/Code/**/` 那一行时，深层文件**静默丢失**——用户以为
    // 文件受 git 保护，实际没有。Unity 的 Scripts 底下必然有多层子目录。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs"]);
    assert!(files.contains(&"Code/Top.cs".to_string()), "{files:?}");
    assert!(files.contains(&"Code/Sub/Mid.cs".to_string()), "{files:?}");
    assert!(
        files.contains(&"Code/Sub/Deep/Low.cs".to_string()),
        "{files:?}"
    );
}

#[test]
fn files_outside_the_whitelist_are_pruned() {
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs"]);
    assert!(!files.iter().any(|f| f.starts_with("Art/")), "{files:?}");
}

#[test]
fn extension_filter_excludes_other_files_inside_the_whitelist() {
    // W-file ⊊ W-path（spec §6.3.2）：.psd 在白名单目录里，但不归 git 管。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs"]);
    assert!(!files.contains(&"Code/Junk.psd".to_string()), "{files:?}");
}

#[test]
fn svn_metadata_never_enters_git_even_with_a_matching_extension() {
    // spec §6.3：白名单落在 external 根时里面有真实的 .svn。一旦 wc.db 被 git
    // 跟踪，任何一次 git checkout 都会在一个**正在使用的**工作副本底下换掉它
    // ——那不是「恢复旧数据」，是直接损坏工作副本。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &[".cs", ".db"]);
    assert!(
        !files.iter().any(|f| f.contains(".svn/")),
        "实际跟踪了：{files:?}"
    );
}

#[test]
fn svn_metadata_never_enters_git_with_the_wildcard_extension() {
    // "*" 是 spec §6.3 明文推荐的全纳入写法，这条路径下 .svn 更容易漏进来。
    let dir = workdir();
    let files = apply(dir.path(), &["Code/"], &["*"]);
    assert!(
        !files.iter().any(|f| f.contains(".svn/")),
        "实际跟踪了：{files:?}"
    );
    // 但普通代码文件仍要在——只证明「没收进 .svn」的话，一条把所有东西
    // 都排除掉的规则也能过。
    assert!(
        files.contains(&"Code/Sub/Deep/Low.cs".to_string()),
        "{files:?}"
    );
    assert!(files.contains(&"Code/Junk.psd".to_string()), "{files:?}");
}

#[test]
fn a_single_file_whitelist_entry_tracks_only_that_file() {
    let dir = workdir();
    let files = apply(dir.path(), &["Code/Top.cs"], &[".cs"]);
    assert_eq!(files, vec!["Code/Top.cs".to_string()]);
}

#[test]
fn a_narrower_entry_does_not_silently_drop_files_of_the_broader_one() {
    // 计划没覆盖的组合：白名单同时含「广的 `Code/`」和「窄的 `Code/Sub/`」。
    //
    // 窄条目为了打开 `Code/Sub` 必须先发一条 `/Code/*`（排除 Code 的直接子项），
    // 而它排在广条目的 `!/Code/**/*.cs` **之后**——git 的裁决是「最后一条匹配的
    // 模式说了算」，于是 `Code/Top.cs` 被静默剔除。
    //
    // 用户看到的是「我明明把整个 Code/ 加进了白名单，顶层的 .cs 却没受保护」，
    // 而且**没有任何报错**。这正是本模块存在的理由所指的失败形态（spec §6.3）。
    // 两种书写顺序都要测：顺序不该改变集合。
    for order in [["Code/", "Code/Sub/"], ["Code/Sub/", "Code/"]] {
        let dir = workdir();
        let files = apply(dir.path(), &order, &[".cs"]);
        assert!(
            files.contains(&"Code/Top.cs".to_string()),
            "order={order:?} 实际：{files:?}"
        );
        assert!(
            files.contains(&"Code/Sub/Mid.cs".to_string()),
            "order={order:?} 实际：{files:?}"
        );
        assert!(
            files.contains(&"Code/Sub/Deep/Low.cs".to_string()),
            "order={order:?} 实际：{files:?}"
        );
        assert!(
            !files.contains(&"Code/Junk.psd".to_string()),
            "order={order:?} 扩展名过滤不该被顺带放开：{files:?}"
        );
    }
}

#[test]
fn a_file_entry_coexists_with_a_directory_entry_that_contains_it() {
    // 「整个 Code/ 收 .cs，另外单独收一个 Code/Junk.psd」是合法配置——单文件
    // 条目本来就不过扩展名过滤（spec §6.3）。它不能被目录条目的 `/Code/**`
    // 重新盖掉，目录条目也不能被它那条 `/Code/*` 打断。同样两种顺序都测。
    for order in [["Code/", "Code/Junk.psd"], ["Code/Junk.psd", "Code/"]] {
        let dir = workdir();
        let files = apply(dir.path(), &order, &[".cs"]);
        assert!(
            files.contains(&"Code/Junk.psd".to_string()),
            "order={order:?} 单文件条目丢了：{files:?}"
        );
        assert!(
            files.contains(&"Code/Top.cs".to_string()),
            "order={order:?} 目录条目被打断了：{files:?}"
        );
        assert!(
            files.contains(&"Code/Sub/Deep/Low.cs".to_string()),
            "order={order:?} 深层文件丢了：{files:?}"
        );
        assert!(
            !files.iter().any(|f| f.contains(".svn/")),
            "order={order:?} 实际跟踪了：{files:?}"
        );
    }
}

#[test]
fn windows_backslash_input_produces_the_same_result() {
    // 跨平台路径是本项目高发缺陷区（AGENTS.md §8）。
    let dir = workdir();
    let files = apply(dir.path(), &[r"Code\Sub\"], &[".cs"]);
    assert!(files.contains(&"Code/Sub/Mid.cs".to_string()), "{files:?}");
    assert!(
        files.contains(&"Code/Sub/Deep/Low.cs".to_string()),
        "{files:?}"
    );
    assert!(!files.contains(&"Code/Top.cs".to_string()), "{files:?}");
}

// ---------------------------------------------------------------------------
// Task 6：纳入集合核对
//
// 这一组证明的是「git 实际跟踪的集合 == 配置声明的集合」，与 exclude 生成本身
// 是两回事：规则文本对了，工作树里一个 .gitignore 照样能把它整个盖掉（spec §6.3.1）。
// ---------------------------------------------------------------------------

fn strs(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn write_rules(dir: &Path, w: &[String], e: &[String]) {
    write_exclude(dir.to_str().unwrap(), &generate_exclude(w, e)).unwrap();
}

#[test]
fn include_check_passes_before_anything_is_staged() {
    // 核对发生在「生成 exclude 之后、录第一个基线之前」（spec §6.3.1），
    // 那个时刻索引还是空的。所以实际集合不能只读索引。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    verify_include_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
}

#[test]
fn include_check_passes_after_the_files_are_staged() {
    // 另一半：文件已经进索引之后再核对，结论必须不变。只测上一条的话，
    // 实际集合里读索引的那一路完全没被覆盖。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    run_git(dir.path(), &["add", "-A"]);
    verify_include_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
}

#[test]
fn the_actual_set_equals_what_git_add_would_have_produced() {
    // spec §6.3.1 的原文是「在**临时索引**上 git add -A 之后读 ls-files」。
    // 实现改成了纯读（--cached ∪ --others --exclude-standard），理由是它连
    // 临时索引和松散对象都不写——但那只有在两者**真的等价**时才成立。
    // 这条把等价性变成一条有断言兜着的事实，而不是我的推断。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);

    let read_only = actual_set(dir.path().to_str().unwrap(), &w).unwrap();
    run_git(dir.path(), &["add", "-A"]);
    let after_add: BTreeSet<String> = tracked(dir.path()).into_iter().collect();

    assert_eq!(read_only, after_add, "纯读取数与 git add -A 的结果必须一致");
    assert!(!read_only.is_empty(), "两边都空的话这条断言什么也没证明");
}

#[test]
fn a_gitignore_in_the_working_tree_is_caught() {
    // spec §6.3.1：工作树里的 .gitignore 优先级**高于** .git/info/exclude。
    // 实测：Code/.gitignore 里一行 *.cs 会让 git ls-files 变成空。
    // 它不会让初始化失败，§6.8 的基线体检也查不出来——只有这道核对能抓到。
    let dir = workdir();
    fs::write(dir.path().join("Code/.gitignore"), "*.cs\n").unwrap();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);

    let err = verify_include_set(dir.path().to_str().unwrap(), &w, &e)
        .expect_err("被 .gitignore 盖掉必须阻断");
    assert_eq!(err.code, "include_set_mismatch");
    // 错误信息必须指认裁决者，否则用户只知道「不一致」，不知道去改哪个文件
    assert!(
        err.message.contains(".gitignore"),
        "应当指出是 .gitignore 干的，实际：{}",
        err.message
    );
    assert!(
        err.message.contains("Code/Top.cs"),
        "应当列出差异文件，实际：{}",
        err.message
    );
}

#[test]
fn include_check_reports_unexpected_files_too() {
    // 双向比对：不只查「该在的没在」，也查「不该在的在了」。
    // 后者正是 .svn 兜底行失效那一类缺陷的表现，而它的后果比前者更重
    // ——一次 git checkout 就能损坏一个正在使用的 SVN 工作副本。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    // 故意写一份**缺少 .svn/ 兜底行**、且把 .db 也纳入的规则，再真的 add 进索引
    let broken = generate_exclude(&w, &strs(&[".cs", ".db"])).replace(".svn/\n", "");
    write_exclude(dir.path().to_str().unwrap(), &broken).unwrap();
    run_git(dir.path(), &["add", "-A"]);
    assert!(
        tracked(dir.path()).iter().any(|f| f.contains(".svn/")),
        "前置条件没成立：.svn 下的文件根本没进索引，这条测不到「多出来」那一侧"
    );

    let err = verify_include_set(dir.path().to_str().unwrap(), &w, &e)
        .expect_err("多出 .svn 下的文件必须被报出来");
    assert_eq!(err.code, "include_set_mismatch");
    assert!(
        err.message.contains("不该被跟踪"),
        "必须走「多出来」那一侧，实际：{}",
        err.message
    );
    assert!(
        err.message.contains(".svn/wc.db"),
        "应当点名是哪个文件，实际：{}",
        err.message
    );
}

#[test]
fn an_empty_whitelist_never_degenerates_into_everything() {
    // G6 / AGENTS.md §6.2：本项目已因空 pathspec 误伤过整个仓库。
    // `git ls-files` 不带 pathspec 会列出整仓库，核对就变成「整仓库 vs 空集」。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    run_git(dir.path(), &["add", "-A"]);
    assert!(
        !tracked(dir.path()).is_empty(),
        "仓库里得真有被跟踪的文件，否则这条恒真"
    );

    assert!(
        actual_set(dir.path().to_str().unwrap(), &[])
            .unwrap()
            .is_empty(),
        "空白名单必须得到空集"
    );
}

#[test]
fn non_ascii_filenames_do_not_produce_a_phantom_mismatch() {
    // `git ls-files` 默认按 core.quotePath 把非 ASCII 路径转义成
    // `"Code/\350\247\222\350\211\262.cs"` 这种带引号的八进制形式，而期望集合
    // 是遍历磁盘得到的原始字符串——两者**永远对不上**，于是每个装了中文文件名的
    // 工程都会在初始化时被误报阻断。用 -z 读可以绕开引号转义。
    let dir = workdir();
    fs::write(dir.path().join("Code/角色.cs"), "x").unwrap();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    verify_include_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
}

#[test]
fn extension_matching_follows_gits_own_case_sensitivity() {
    // Windows 上 `git init` 会把 core.ignoreCase 设成 true，忽略规则的匹配随之
    // 变成大小写不敏感——`!/Code/**/*.cs` 会把 `Loud.CS` 也纳入。期望集合若死板地
    // 按大小写敏感算，就会把它报成「不该被跟踪却跟踪了」，在一个完全正常的仓库上
    // **误报阻断**。所以期望集合读 core.ignoreCase，跟 git 保持一致，而不是按平台猜。
    //
    // 这条在两种平台上都成立：大小写敏感的系统上 git 不收 Loud.CS，期望集合也不收。
    let dir = workdir();
    fs::write(dir.path().join("Code/Loud.CS"), "x").unwrap();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    verify_include_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
}
