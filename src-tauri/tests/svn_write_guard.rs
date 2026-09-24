//! 守住「同步进行中，所有写操作必须拒绝」这条约束（spec §5.1）。
//!
//! 扫源码而不是靠 review 盯着——理由与 `commands_are_async.rs` 完全一样：
//! 新增一个写方法时忘记加守卫是**无声的**，编译器和 clippy 都不会报，而后果是
//! 同步跑到一半被一次 stage / checkout 从中间插进来。
//!
//! # 分类表必须**穷尽**，这是第一版的教训
//!
//! 第一版把写方法写成一份硬编码清单，然后只断言「清单里的都有守卫」。
//! 2026-09-25 独立 review 指出它抓不到它自己声称要抓的那件事：**新增**一个
//! 写方法而忘了加守卫时，那个方法根本不在清单里，测试照样绿。
//!
//! 现在改成：先把 `impl Vcs for SvnGitBackend` 里**实际存在**的方法全部枚举
//! 出来，任何一个没被归类到「写」或「读」的，测试直接失败并要求作者归类。
//! 于是「新增方法」这条路上必然会撞到这道门。

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// 写方法：必须有 `assert_not_syncing`。
const WRITE_METHODS: &[&str] = &[
    "stage",
    "unstage",
    "discard",
    "commit",
    "create_branch",
    "delete_branch",
    "rename_branch",
    "checkout",
    "stash_push",
    "stash_apply",
    "stash_pop",
    "stash_drop",
];

/// 读方法：**不得**有守卫。spec §5.1 明确要求读操作不阻塞——同步期间界面还要
/// 显示状态，把读也挡掉等于在最需要看清状况的时候把灯关了。
const READ_METHODS: &[&str] = &[
    "capabilities",
    "open",
    "graph",
    "status",
    "diff",
    "list_stashes",
];

fn svn_backend_src() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vcs/svn.rs");
    fs::read_to_string(&path).expect("读不到 src/vcs/svn.rs")
}

/// `impl Vcs for SvnGitBackend` 块里实际写着的方法名。
///
/// 只取那个 impl 块，不取 `impl SvnGitBackend`（`open()` 构造函数在那边，
/// 与 trait 的 `open()` 重名但不是一回事）。
fn trait_methods(src: &str) -> BTreeSet<String> {
    let start = src
        .find("impl Vcs for SvnGitBackend {")
        .expect("找不到 `impl Vcs for SvnGitBackend` 块——重命名了？");
    let body = &src[start..];
    let mut out = BTreeSet::new();
    for line in body.lines() {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("fn ") {
            if let Some(name) = rest.split('(').next() {
                out.insert(name.to_string());
            }
        }
        // impl 块结束
        if line == "}" {
            break;
        }
    }
    out
}

/// 取某个方法从签名到下一个 `    fn ` 之间的方法体。
fn method_body<'a>(src: &'a str, name: &str) -> &'a str {
    let sig = format!("    fn {name}(");
    let start = src
        .find(&sig)
        .unwrap_or_else(|| panic!("`{name}` 在 src/vcs/svn.rs 里找不到——方法改名了？"));
    let rest = &src[start + sig.len()..];
    let end = rest.find("\n    fn ").unwrap_or(rest.len());
    &rest[..end]
}

#[test]
fn every_trait_method_is_classified_as_read_or_write() {
    // **这条是那道门。** 新增一个方法而不归类，这里就红——作者被迫回答
    // 「它是读还是写」，而那正是会被忘掉的那个判断。
    let src = svn_backend_src();
    let actual = trait_methods(&src);

    let classified: BTreeSet<String> = WRITE_METHODS
        .iter()
        .chain(READ_METHODS.iter())
        .map(|s| s.to_string())
        .collect();

    let unclassified: Vec<&String> = actual.difference(&classified).collect();
    assert!(
        unclassified.is_empty(),
        "`impl Vcs for SvnGitBackend` 里出现了没有归类的方法：{unclassified:?}\n\
         请把它加进 tests/svn_write_guard.rs 的 WRITE_METHODS 或 READ_METHODS。\n\
         写方法还要在方法体第一行加 `assert_not_syncing(&self.root)?;`（spec §5.1）。"
    );

    let vanished: Vec<&String> = classified.difference(&actual).collect();
    assert!(
        vanished.is_empty(),
        "分类表里这些方法在 src/vcs/svn.rs 里已经不存在了：{vanished:?}\n\
         表过期了会让下面两条断言悄悄失去覆盖，请同步删掉。"
    );
}

#[test]
fn every_write_method_checks_the_sync_lock() {
    let src = svn_backend_src();

    let missing: Vec<&str> = WRITE_METHODS
        .iter()
        .copied()
        .filter(|m| !method_body(&src, m).contains("assert_not_syncing"))
        .collect();

    assert!(
        missing.is_empty(),
        "以下写方法没有检查同步锁，同步跑到一半会被它们从中间插进来：\n  {}\n\
         在方法体第一行加 `assert_not_syncing(&self.root)?;`。",
        missing.join("\n  ")
    );
}

#[test]
fn the_guard_runs_before_the_delegation_not_after() {
    // 只断言「方法体里出现了这个字符串」的话，守卫写在委托**之后**也能通过——
    // 那时活儿已经干完了，守卫等于没有。只有 `stage` 有行为测试兜底，
    // 其余 11 个靠这一条。
    let src = svn_backend_src();
    for m in WRITE_METHODS {
        let body = method_body(&src, m);
        let guard = body
            .find("assert_not_syncing")
            .unwrap_or_else(|| panic!("`{m}` 没有守卫"));
        let delegate = body
            .find("self.git.")
            .unwrap_or_else(|| panic!("`{m}` 找不到对 GitBackend 的委托"));
        assert!(
            guard < delegate,
            "`{m}` 的守卫写在委托之后了——那时活儿已经干完，守卫等于没有"
        );
    }
}

#[test]
fn read_methods_are_not_guarded() {
    // 反向约束，同样重要：spec §5.1 要求「读操作不阻塞」。给读方法也加上守卫的话，
    // 同步期间界面会整个瞎掉——而那恰恰是最需要显示状态的时候。
    let src = svn_backend_src();

    for m in READ_METHODS {
        assert!(
            !method_body(&src, m).contains("assert_not_syncing"),
            "读方法 `{m}` 不该有同步锁守卫：同步期间读必须照常可用（spec §5.1）"
        );
    }
}

/// 绕开 `trait Vcs`、但在 SVN 混合仓库上**可达且会写**的 Tauri 命令。
///
/// 2026-09-25 独立 review 抓到的缺口。`vcs/mod.rs` 的注释说有一批命令「仍然直连
/// `run_git`」，靠「界面不渲染入口」挡着——但这三条不在那个范围里：
///
/// | 命令 | 它干什么 | 为什么可达 |
/// |---|---|---|
/// | `ops::revert` | `git revert --no-edit`，移动 HEAD + 重写工作目录 | `commitActions.ts` 的能力过滤只关掉 `new-tag`/`cherry-pick`/`save-patch`，revert 走 `?? true` |
/// | `svn::svn_rebuild_baseline` | 移动 `svn/base` 指针 | 失同步横幅上的按钮 |
/// | `svn::svn_migrate_eol` | 同上 | 行尾迁移横幅上的按钮 |
///
/// `svn_init_hybrid` **不在**此列：它只在 `SvnUninitialized` 上跑，那种目录里
/// 还没有 `.git`，也就不可能有锁。
const GUARDED_COMMANDS: &[(&str, &str)] = &[
    ("commands/ops.rs", "revert"),
    ("commands/svn.rs", "svn_rebuild_baseline"),
    ("commands/svn.rs", "svn_migrate_eol"),
];

#[test]
fn write_commands_outside_the_vcs_trait_are_guarded_too() {
    for (file, func) in GUARDED_COMMANDS {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(file);
        let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("读不到 {file}: {e}"));
        let sig = format!("pub fn {func}(");
        let start = src
            .find(&sig)
            .unwrap_or_else(|| panic!("{file} 里找不到 `{func}`——改名了？"));
        let rest = &src[start + sig.len()..];
        let end = rest.find("\npub fn ").unwrap_or(rest.len());
        assert!(
            rest[..end].contains("assert_not_syncing"),
            "`{file}::{func}` 会写仓库但没有检查同步锁。\
             它不走 `trait Vcs`，所以 vcs/svn.rs 上的守卫拦不到它（spec §5.1）。"
        );
    }
}
