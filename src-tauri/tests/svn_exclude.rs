//! exclude 规则生成的测试。
//!
//! **断言对象是 git 的实际行为（`git ls-files`），不是生成出来的规则文本**
//! ——测规则文本只能证明「生成了我以为的字符串」，证明不了 git 会怎么裁决，
//! 而这个模块存在的全部理由就是后者（spec §6.3）。

use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
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
