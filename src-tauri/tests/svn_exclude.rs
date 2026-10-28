//! exclude 规则生成的测试。
//!
//! **断言对象是 git 的实际行为（`git ls-files`），不是生成出来的规则文本**
//! ——测规则文本只能证明「生成了我以为的字符串」，证明不了 git 会怎么裁决，
//! 而这个模块存在的全部理由就是后者（spec §6.3）。

use app_lib::engine::svn_baseline::TempIndex;
use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
use app_lib::engine::svn_include_check::{dry_run_include_set, expected_set, verify_include_set};
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

/// 建候选索引 → `add -A -- <W-path>` → 核对。
///
/// spec §6.3.1 要求核对与录制共用**同一份**候选索引，所以 `verify_include_set`
/// 收的是一个已经 `add_all` 过的 `TempIndex`，而不是自己现算实际集合。
/// 这里从空索引起步，与初始化向导跑到这一步时的状态一致。
fn check(dir: &Path, w: &[String], e: &[String]) -> app_lib::error::Result<()> {
    let root = dir.to_str().unwrap();
    let idx = TempIndex::empty(root)?;
    idx.add_all(root, w)?;
    // 这些 fixture 是纯 git 目录、没有 SVN 工作副本，脏清单按定义为空。
    // 空 `fixes` 时期望集合退化成「磁盘上的 W-file」，也就是 §16.1 之前的口径——
    // 这批测试覆盖的正是那条不变的行为（spec §6.3.1）。
    verify_include_set(root, &idx, w, e, &std::collections::BTreeMap::new())
}

#[test]
fn include_check_passes_before_anything_is_staged() {
    // 核对发生在「生成 exclude 之后、录第一个基线之前」（spec §6.3.1），
    // 那个时刻索引还是空的。所以实际集合不能只读索引。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    check(dir.path(), &w, &e).unwrap();
}

#[test]
fn include_check_passes_after_the_files_are_staged() {
    // 另一半：文件已经进索引之后再核对，结论必须不变。只测上一条的话，
    // 实际集合里读索引的那一路完全没被覆盖。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    run_git(dir.path(), &["add", "-A"]);
    check(dir.path(), &w, &e).unwrap();
}

#[test]
fn the_candidate_index_equals_what_git_add_would_have_produced() {
    // 2026-09-14 改写。原来这条叫 `the_actual_set_equals_what_git_add_would_have_
    // produced`，比的是 1a 那个**纯读模拟**与真实 `git add -A` 是否等价。模拟版
    // 已随 spec §6.3.1 的新写法删掉（核对与录制必须共用同一份候选索引）。
    //
    // 断言目标仍然成立、也仍然有价值，只是被测对象换了：候选索引跑在**临时索引**
    // 上，用户的真实索引跑 `git add -A`，两者收到的文件集合必须一致。
    // 这不是恒等式——一个从空索引 + `-- Code` 起步，一个在用户索引上收整个仓库。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);

    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &w).unwrap();
    let candidate = idx.ls_files(root).unwrap();

    run_git(dir.path(), &["add", "-A"]);
    let after_add: BTreeSet<String> = tracked(dir.path()).into_iter().collect();

    assert_eq!(candidate, after_add, "候选索引与 git add -A 的结果必须一致");
    assert!(!candidate.is_empty(), "两边都空的话这条断言什么也没证明");
}

#[test]
fn a_deleted_but_unstaged_file_does_not_look_like_an_unexpected_file() {
    // 2026-09-13 独立 review 的阻断项 B1。
    //
    // 上一条测试声称「纯读取数与 git add -A 等价」，但它在**空索引**上跑
    // ——`--cached` 那一路恒为空，真正会分叉的那一半根本没执行到。
    //
    // 真正的分叉在这里：一个已跟踪文件被从工作树删掉、删除还没 stage 时
    //   `--cached`     仍然列它（它还在索引里）
    //   `git add -A`   会把删除入索引，之后 ls-files 不列它
    //   `expected_set` 走磁盘，也不含它
    // 于是核对会报「不该被跟踪、实际跟踪了 Code/Top.cs」并硬阻断——而用户
    // 只是删了个文件还没提交，仓库状态完全正常。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    run_git(dir.path(), &["add", "-A"]);
    fs::remove_file(dir.path().join("Code/Top.cs")).unwrap();

    check(dir.path(), &w, &e).expect("删了文件还没 stage 不该被判成「不该被跟踪」");
}

#[test]
fn the_candidate_index_equals_git_add_even_when_the_users_index_is_already_populated() {
    // B1 的另一半，同样改写（见上一条）。把等价性断言放到**用户索引非空**的时刻
    // 再跑一遍，并制造一个删除未 stage 的差异——1a 那个 bug 就死在这个形态上：
    // 模拟版的 `--cached` 仍列已删文件，`git add -A` 之后不列，于是核对报
    // 「不该被跟踪、实际跟踪了 Code/Sub/Mid.cs」并硬阻断，而用户只是删了个文件。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    run_git(dir.path(), &["add", "-A"]);
    fs::remove_file(dir.path().join("Code/Sub/Mid.cs")).unwrap();
    fs::write(dir.path().join("Code/Fresh.cs"), "x").unwrap();

    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &w).unwrap();
    let candidate = idx.ls_files(root).unwrap();

    run_git(dir.path(), &["add", "-A"]);
    let after_add: BTreeSet<String> = tracked(dir.path()).into_iter().collect();

    assert_eq!(candidate, after_add, "用户索引非空时两条路径仍必须一致");
    assert!(
        !candidate.contains("Code/Sub/Mid.cs"),
        "已删除的文件不该留在集合里：{candidate:?}"
    );
    assert!(
        candidate.contains("Code/Fresh.cs"),
        "新增未入索引的文件必须在集合里：{candidate:?}"
    );
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

    let err = check(dir.path(), &w, &e).expect_err("被 .gitignore 盖掉必须阻断");
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

    let err = check(dir.path(), &w, &e).expect_err("多出 .svn 下的文件必须被报出来");
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

    // 空白名单 → `add_all` 是 no-op → 候选索引为空。
    // 退化成「全部」的那条路会在这里把整个仓库收进来。
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &[]).unwrap();
    assert!(
        idx.ls_files(root).unwrap().is_empty(),
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
    check(dir.path(), &w, &e).unwrap();
}

/// 建一个 Windows 目录 junction。成功返回 true。
///
/// 用 junction 而不是 `std::os::windows::fs::symlink_dir`：后者要管理员权限或
/// 开发者模式，前者不需要。
///
/// ⚠️ 路径里的正斜杠必须先换成反斜杠。`mklink` 是 cmd 的内建命令，cmd 按
/// `/X` 切选项——`C:\…\Code/Mirror` 里的 `/Mirror` 会被当成一个未知选项，
/// 命令失败且错误信息完全指不到这上面（实测踩过一次）。
#[cfg(windows)]
fn make_junction(link: &Path, target: &Path) -> bool {
    let win = |p: &Path| p.to_string_lossy().replace('/', "\\");
    Command::new("cmd")
        .args(["/c", "mklink", "/J", &win(link), &win(target)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(windows)]
#[test]
fn a_directory_junction_inside_the_whitelist_does_not_cause_a_phantom_mismatch() {
    // 2026-09-13 独立 review 的 A4。
    //
    // 2026-09-13 实测：`git ls-files --others --exclude-standard -- Code/` 会列出
    // `Code/Mirror/Linked.cs` ——**git 把 junction 当普通目录走进去了**。
    // 所以 `expected_set` 也必须走进去；把它当成「一个文件」会让期望集合少一批
    // 文件，在一个完全正常的仓库上误报阻断。
    //
    // （这与「git 把符号链接记成 blob、不递归」那条常识不矛盾：那条对
    // `core.symlinks=true` 下的真符号链接成立，而 Windows 上该项默认 false。）
    let dir = workdir();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("Linked.cs"), "x").unwrap();
    let link = dir.path().join("Code").join("Mirror");
    if !make_junction(&link, outside.path()) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }

    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    check(dir.path(), &w, &e).unwrap();

    // 前提校验：junction 内部的文件确实两边都收到了，否则上面那条 unwrap
    // 可能只是因为「两边都没看见它」而通过——那什么也没证明
    let set = expected_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
    assert!(
        set.contains("Code/Mirror/Linked.cs"),
        "期望集合必须走进 junction：{set:?}"
    );

    // 拆掉 junction 再让 TempDir 清理，否则 remove_dir_all 会顺着它删掉
    // outside 里的内容
    fs::remove_dir(&link).unwrap();
}

#[cfg(windows)]
#[test]
fn a_junction_loop_terminates_instead_of_blowing_the_stack() {
    // 同一条的另一半，后果更重：junction 指回自己的祖先时递归没有出口，
    // 栈溢出会让 Rust 直接 abort——在 Tauri 里就是整个应用凭空消失，
    // 连错误横幅都来不及显示。
    //
    // ⚠️ 这条要断言的是「**终止**」，不是「不进去」。上一条已经证明 junction
    // 必须跟随（git 就是这么干的），所以出口只能靠环检测：`canonicalize` 在
    // Windows 上会解开 reparse point，`Code/Loop` 与 `Code` 得到同一个规范路径。
    //
    // 集合与 git 对不上是允许的——实测 git 自己在这种工作树上也只是一路递归到
    // 64 层才放弃、吐出一堆 `Code/Loop/Loop/…/Top.cs` 并打印
    // `could not open directory` 警告。那种工作树本来就是坏的，报一次集合不一致
    // 是正确行为；这里要保证的只是不崩。
    let dir = workdir();
    let code = dir.path().join("Code");
    if !make_junction(&code.join("Loop"), &code) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }

    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    let set = expected_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
    assert!(set.contains("Code/Top.cs"), "正常文件仍要收进来：{set:?}");
    // 环检测在第二次遇到同一个规范路径时停手，所以最多下探一层
    assert!(
        !set.iter().any(|f| f.contains("Loop/Loop/")),
        "环必须被截断，不能一层层递归下去：{set:?}"
    );

    // junction 必须手工拆掉再让 TempDir 清理，否则 remove_dir_all 会顺着它
    // 把目标目录的内容一起删了——这里目标就是 Code 自己，后果是测试互相打架
    fs::remove_dir(code.join("Loop")).unwrap();
}

#[test]
fn a_users_own_exclude_is_backed_up_before_we_take_over() {
    // 2026-09-13 独立 review 的 A6。`.git/info/exclude` 是 git 留给用户自己用的
    // 地方，无条件整文件覆盖等于删掉他手写的规则、且不留任何痕迹。
    let dir = workdir();
    let info = dir.path().join(".git").join("info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("exclude"), "# 我自己写的\n*.log\n").unwrap();

    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);

    let backup = fs::read_to_string(info.join("exclude.bak")).expect("原件必须被备份");
    assert!(backup.contains("*.log"), "备份内容不对：{backup}");

    // 再写一次：备份**不能**被我们自己生成的内容盖掉，否则用户的原件就没了
    write_rules(dir.path(), &w, &e);
    let backup2 = fs::read_to_string(info.join("exclude.bak")).unwrap();
    assert_eq!(backup2, backup, "第二次写入不该覆盖备份");

    // 一个刚 git init 的仓库不该产生备份。`git init` 会在那里写一份**纯注释**的
    // 模板，按「非空」判定的话每个新仓库都会多出一个什么也没保存的 exclude.bak。
    let dir2 = workdir();
    let stock = fs::read_to_string(dir2.path().join(".git/info/exclude")).unwrap();
    assert!(
        !stock.trim().is_empty(),
        "前提没成立：git init 本该写一份模板，这条测不到「纯注释」那一路"
    );
    write_rules(dir2.path(), &w, &e);
    write_rules(dir2.path(), &w, &e);
    assert!(
        !dir2.path().join(".git/info/exclude.bak").exists(),
        "纯注释的 git 模板和我们自己生成的内容都不该产生备份"
    );
}

#[test]
fn write_exclude_leaves_no_temp_file_behind() {
    // 原子替换用的临时文件必须收干净——`.git/info/` 里留一个
    // `exclude.mygitclient.tmp` 不会报错，但下次有人来看这个目录会一头雾水。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);

    let leftovers: Vec<String> = fs::read_dir(dir.path().join(".git").join("info"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains("tmp"))
        .collect();
    assert!(leftovers.is_empty(), "残留了临时文件：{leftovers:?}");
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
    check(dir.path(), &w, &e).unwrap();
}

#[test]
fn core_ignorecase_is_read_with_gits_own_boolean_parsing() {
    // 上一条只覆盖了 `git init` 自己写的字面量 `true`。git 的布尔解析还接受
    // `1` / `yes` / `on`（2026-09-14 实测 `git config --bool` 把这三个都归一成
    // `true`），而原来的实现只认字符串 "true"。
    //
    // 于是 `core.ignoreCase=1` 时：git 照常把 `Code/Loud.CS` 纳入，期望集合却
    // 按大小写敏感算、不含它 → 核对报「不该被跟踪、实际跟踪了」并硬阻断，
    // 而仓库完全正常。这是**误报阻断**，不是发现问题。
    for truthy in ["1", "yes", "on", "TRUE"] {
        let dir = workdir();
        run_git(dir.path(), &["config", "core.ignoreCase", truthy]);
        fs::write(dir.path().join("Code/Loud.CS"), "x").unwrap();
        let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
        write_rules(dir.path(), &w, &e);
        check(dir.path(), &w, &e)
            .unwrap_or_else(|err| panic!("core.ignoreCase={truthy} 时误报：{}", err.message));
    }

    // 反向：git 认的假值也要认，否则大小写敏感的卷上会反过来漏报
    for falsy in ["0", "no", "off"] {
        let dir = workdir();
        run_git(dir.path(), &["config", "core.ignoreCase", falsy]);
        fs::write(dir.path().join("Code/Loud.CS"), "x").unwrap();
        let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
        write_rules(dir.path(), &w, &e);
        let set = expected_set(dir.path().to_str().unwrap(), &w, &e).unwrap();
        assert!(
            !set.contains("Code/Loud.CS"),
            "core.ignoreCase={falsy} 时期望集合不该含 Loud.CS，实际 {set:?}"
        );
    }
}

#[test]
fn the_mismatch_report_names_the_rule_for_files_that_are_tracked_but_should_not_be() {
    // 「多收」方向原来只列文件名、不指认规则，而这正是最需要指认的一侧：
    // 用户想知道的是「凭什么它进来了」。
    //
    // 而且这一侧的文件**必然已在索引里**，`git check-ignore` 不带 `--no-index`
    // 时对已跟踪文件只吐一行空裁决 `::<TAB>路径`（2026-09-14 实测，exit 1）,
    // 带上才会给出 `.git/info/exclude:3:/Code/**`。spec §6.3.1 新版已写明要带。
    //
    // ⚠️ 2026-09-14 换了造「多收」的方式。原来是往**用户的真实索引**里
    // `git add -f`，那在 1a 的纯读模拟下会被 `--cached` 读到。现在核对读的是
    // 候选索引（从空或 HEAD 树起步），用户暂存区里那一笔根本进不来——也确实
    // **不该**进来，它永远到不了基线。新模型下多收的真实路径是
    // 「**HEAD 树里已经躺着**一个不该被跟踪的文件」：`from_head` 会把它带进候选
    // 索引，`add -A` 不会删掉它（磁盘上还在），于是它一路进到基线里。
    let dir = workdir();
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(dir.path(), &w, &e);
    run_git(dir.path(), &["add", "-f", "--", "Code/Junk.psd"]);
    run_git(dir.path(), &["commit", "-m", "误把 psd 提交了"]);

    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::from_head(root).unwrap();
    idx.add_all(root, &w).unwrap();
    let err = verify_include_set(root, &idx, &w, &e, &std::collections::BTreeMap::new())
        .expect_err("多收必须被抓到");
    assert_eq!(err.code, "include_set_mismatch");
    assert!(
        err.message.contains("Code/Junk.psd"),
        "实际：{}",
        err.message
    );
    assert!(
        err.message.contains("exclude:"),
        "多收方向必须指认是哪条规则本来该挡住它，实际：{}",
        err.message
    );
    assert!(
        !err.message.contains("::\t"),
        "拿到的是不带 --no-index 的空裁决，实际：{}",
        err.message
    );
}

// ---------------------------------------------------------------------------
// 不一致报告的可读性（2026-10-27，用户在真实 Unity 工程上的报告）
//
// 原来的报告只列按字母序的前 20 个文件、每个文件后面跟一条裁决规则。几百个文件
// 被同一条规则挡掉时，用户真正要的「是哪条规则、挡了多少个」反而看不出来。
// ---------------------------------------------------------------------------

/// 一次 `TempIndex` 录制 + 核对，扩展名用 `*`（用户的真实配置）。
fn check_wildcard(dir: &Path) -> app_lib::error::GitError {
    let (w, e) = (strs(&["Code/"]), strs(&["*"]));
    write_rules(dir, &w, &e);
    check(dir, &w, &e).expect_err("必须报不一致")
}

#[test]
fn the_mismatch_report_groups_dropped_files_by_the_rule_that_dropped_them() {
    let dir = workdir();
    let p = dir.path();
    fs::create_dir_all(p.join("Code/Tools")).unwrap();
    fs::create_dir_all(p.join("Code/Py")).unwrap();
    for i in 0..30 {
        fs::write(p.join(format!("Code/Tools/t{i:02}.exe")), "x").unwrap();
    }
    for i in 0..3 {
        fs::write(p.join(format!("Code/Py/m{i}.pyc")), "x").unwrap();
    }
    // 一份在白名单内、一份在仓库根——两处都是用户以前本地 git 可能留下的位置
    fs::write(p.join("Code/.gitignore"), "*.exe\n").unwrap();
    fs::write(p.join(".gitignore"), "*.pyc\n").unwrap();

    let err = check_wildcard(p);
    assert_eq!(err.code, "include_set_mismatch");
    let m = &err.message;
    assert!(m.contains("共 33 个"), "应当给出漏收总数，实际：{m}");
    assert!(
        m.contains("30 个被 Code/.gitignore:1:*.exe 挡掉"),
        "应当按规则汇总数量，实际：{m}"
    );
    assert!(
        m.contains("3 个被 .gitignore:1:*.pyc 挡掉"),
        "第二条规则也要单独成组，实际：{m}"
    );
    assert!(
        m.find("*.exe").unwrap() < m.find("*.pyc").unwrap(),
        "挡得多的规则排在前面，实际：{m}"
    );
    let listed = m.lines().filter(|l| l.trim_end().ends_with(".exe")).count();
    assert!(
        (1..=5).contains(&listed),
        "同一组只列几个样例，不能把 30 个全倒出来（实际列了 {listed} 个）：{m}"
    );
    assert!(
        m.contains("…还有 27 个"),
        "样例之外要说清还有多少，实际：{m}"
    );
}

#[test]
fn the_mismatch_report_points_at_a_nested_git_repository() {
    // 嵌套仓库里的文件不是被忽略规则挡掉的（`check-ignore` 说它们没被忽略），
    // `git add -A` 只给它记一个子模块指针。不单独说明的话，用户看到的是
    // 「没有匹配规则却没被收」，无从下手。
    let dir = workdir();
    let p = dir.path();
    let nested = p.join("Code/Plugin");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("P.cs"), "x").unwrap();
    run_git(&nested, &["init", "-q"]);
    run_git(&nested, &["config", "user.email", "t@t"]);
    run_git(&nested, &["config", "user.name", "T"]);
    run_git(&nested, &["add", "-A"]);
    run_git(&nested, &["commit", "-qm", "plugin"]);

    let err = check_wildcard(p);
    let m = &err.message;
    assert!(
        m.contains("嵌套的 git 仓库 Code/Plugin"),
        "应当点名嵌套仓库，实际：{m}"
    );
    assert!(m.contains("Code/Plugin/P.cs"), "实际：{m}");
}

#[cfg(windows)]
#[test]
fn a_junction_inside_the_whitelist_does_not_break_the_batch_rule_lookup() {
    // review 担心（2026-10-28）：裁决规则一次批量查，若 junction 底下的路径让
    // `check-ignore` fatal（「越过符号链接」），那些路径会拿不到裁决。
    // 实测（git 2.51.2.windows.1）：不会 fatal，junction 底下与普通目录同样给出
    // 裁决。这条把这个实测钉住；真符号链接会 fatal，见下一条。
    let dir = workdir();
    let p = dir.path();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("m.exe"), "x").unwrap();
    if !make_junction(&p.join("Code/Mirror"), outside.path()) {
        panic!("建 junction 失败——本条测试的前提没成立，不能当成通过");
    }
    fs::create_dir_all(p.join("Code/Tools")).unwrap();
    fs::write(p.join("Code/Tools/a.exe"), "x").unwrap();
    fs::write(p.join("Code/.gitignore"), "*.exe\n").unwrap();

    let m = check_wildcard(p).message;
    // 2 个 = 普通目录里的 a.exe + junction 底下的 m.exe，两个都拿到了裁决
    assert!(
        m.contains("2 个被 Code/.gitignore:1:*.exe 挡掉"),
        "junction 底下与普通目录的文件都要拿到裁决规则，实际：{m}"
    );
    fs::remove_dir(p.join("Code/Mirror")).unwrap();
}

/// 第二轮 review（2026-10-28）：真的**目录符号链接**会让 `check-ignore` 对它底下的
/// 路径报 `beyond a symbolic link` 并退出。原实现整批作废，同一份报告里被
/// `.gitignore` 挡掉的文件也跟着变成「查不到裁决规则」。
///
/// 只在 unix 上跑：Windows 建符号链接要管理员权限或开发者模式，本机两者都没有
/// （2026-10-28 实测 `New-Item -ItemType SymbolicLink` 被拒）。Windows 上「一个
/// fatal 不连累别的路径」由 `svn_include_check` 的内联单测用另一种 fatal 钉住。
#[cfg(unix)]
#[test]
fn a_directory_symlink_inside_the_whitelist_does_not_cost_other_files_their_rule() {
    let dir = workdir();
    let p = dir.path();
    let outside = TempDir::new().unwrap();
    fs::write(outside.path().join("m.cs"), "x").unwrap();
    std::os::unix::fs::symlink(outside.path(), p.join("Code/Link")).unwrap();
    fs::create_dir_all(p.join("Code/Tools")).unwrap();
    // 字母序排在 `Code/Link/m.cs` 之后——原实现里正是它被连累
    fs::write(p.join("Code/Tools/a.exe"), "x").unwrap();
    fs::write(p.join("Code/.gitignore"), "*.exe\n").unwrap();

    let m = check_wildcard(p).message;
    assert!(
        m.contains("1 个被 Code/.gitignore:1:*.exe 挡掉"),
        "符号链接之后的文件也要拿到裁决规则，实际：{m}"
    );
}

#[test]
fn the_mismatch_report_keeps_non_ascii_paths_readable() {
    // 批量查裁决规则走的是 `-z`：不带它的话 git 会把中文路径转义成八进制，
    // 回填时对不上号，整组落进「查不到规则」。
    let dir = workdir();
    let p = dir.path();
    fs::create_dir_all(p.join("Code/工具")).unwrap();
    fs::write(p.join("Code/工具/甲.exe"), "x").unwrap();
    fs::write(p.join("Code/.gitignore"), "*.exe\n").unwrap();

    let m = check_wildcard(p).message;
    assert!(
        m.contains("1 个被 Code/.gitignore:1:*.exe 挡掉"),
        "实际：{m}"
    );
    assert!(m.contains("Code/工具/甲.exe"), "实际：{m}");
}

// ---------------------------------------------------------------------------
// 预检：`git add` 之前的只读核对（2026-10-27）
//
// 最终核对排在 `git add -A` 之后。真实 Unity 工程上 `git add` 要把白名单里每个
// 文件哈希压缩一遍，用户等了约半小时才看到 `include_set_mismatch`，然后回退
// 把 `.git` 整个删掉——下一次重试又是半小时。预检只遍历目录、不写任何对象。
// ---------------------------------------------------------------------------

/// 跑预检。这些 fixture 是纯 git 目录，svn 脏清单按定义为空——与 `check` 同口径。
/// 脏清单修正那一侧由 `svn_init` 里带真实 svn 工作副本的用例覆盖。
fn dry_run(root: &str, w: &[String], e: &[String]) -> app_lib::error::Result<()> {
    dry_run_include_set(
        root,
        w,
        e,
        &std::collections::BTreeMap::new(),
        &BTreeSet::new(),
    )
}

/// `.git/objects` 下的文件数。预检必须一个对象都不写。
fn object_files(dir: &Path) -> usize {
    fn walk(p: &Path) -> usize {
        fs::read_dir(p)
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                if e.path().is_dir() {
                    walk(&e.path())
                } else {
                    1
                }
            })
            .sum()
    }
    walk(&dir.join(".git/objects"))
}

#[test]
fn the_precheck_catches_a_gitignore_without_writing_any_object() {
    let dir = workdir();
    let p = dir.path();
    fs::create_dir_all(p.join("Code/Tools")).unwrap();
    fs::write(p.join("Code/Tools/t.exe"), "x").unwrap();
    fs::write(p.join("Code/.gitignore"), "*.exe\n").unwrap();
    let (w, e) = (strs(&["Code/"]), strs(&["*"]));
    write_rules(p, &w, &e);
    assert_eq!(object_files(p), 0, "前置条件：fixture 里还没有任何对象");

    let err = dry_run(p.to_str().unwrap(), &w, &e).expect_err("必须在预检拦住");
    assert_eq!(err.code, "include_set_mismatch");
    assert!(
        err.message.contains("预检"),
        "要能看出是哪一道拦的：{}",
        err.message
    );
    assert!(
        err.message.contains("1 个被 Code/.gitignore:1:*.exe 挡掉"),
        "实际：{}",
        err.message
    );
    assert_eq!(object_files(p), 0, "预检写了对象——它就不再是便宜的那一道了");
}

#[test]
fn the_precheck_passes_the_clean_configurations_the_final_check_passes() {
    // 预检挡住一个最终核对会放行的仓库，就是误报阻断。用同一份 fixture 两边都跑。
    //
    // 只覆盖**干净的纯 git 目录**下的三种白名单 / 扩展名组合（含中文文件名、
    // 单文件条目、`*`），不是「两道核对处处等价」的证明。其余形态在别处：
    // 脏工作副本 → `svn_init` 的 `the_precheck_does_not_block_what_the_final_check_
    // accepts_on_a_dirty_working_copy`；junction / external / GBK 等 → `svn_init`
    // 的成功路径用例（它们都要先过预检）；嵌套仓库 → 两道都拦，见下方用例。
    let dir = workdir();
    fs::write(dir.path().join("Code/角色.cs"), "x").unwrap();
    for (w, e) in [
        (strs(&["Code/"]), strs(&[".cs"])),
        (strs(&["Code/"]), strs(&["*"])),
        (strs(&["Code/", "Code/Junk.psd"]), strs(&[".cs"])),
    ] {
        write_rules(dir.path(), &w, &e);
        dry_run(dir.path().to_str().unwrap(), &w, &e)
            .unwrap_or_else(|err| panic!("{w:?} {e:?} 预检误报：{}", err.message));
        check(dir.path(), &w, &e).unwrap();
    }
}

#[test]
fn the_precheck_catches_a_nested_repository_that_git_add_would_choke_on() {
    // 没有提交的嵌套仓库会让 `git add -A` 直接失败（2026-10-27 实测：
    // `'Code/Plugin/' does not have a commit checked out`），用户只看到一句
    // git 的英文报错。预检要在那之前把它点名。
    let dir = workdir();
    let p = dir.path();
    let nested = p.join("Code/Plugin");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("P.cs"), "x").unwrap();
    run_git(&nested, &["init", "-q"]);
    let (w, e) = (strs(&["Code/"]), strs(&[".cs"]));
    write_rules(p, &w, &e);

    let err = dry_run(p.to_str().unwrap(), &w, &e).expect_err("必须拦住");
    assert!(
        err.message.contains("嵌套的 git 仓库 Code/Plugin"),
        "实际：{}",
        err.message
    );
}

#[test]
fn the_precheck_with_an_empty_whitelist_checks_nothing_rather_than_everything() {
    // AGENTS.md §6.2：`ls-files -o` 不带 pathspec 会列出整个仓库。
    let dir = workdir();
    write_rules(dir.path(), &[], &strs(&[".cs"]));
    dry_run(dir.path().to_str().unwrap(), &[], &strs(&[".cs"])).unwrap();
}
