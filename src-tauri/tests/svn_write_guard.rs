//! 守住「同步进行中，所有写操作必须拒绝」这条约束（spec §5.1）。
//!
//! 扫源码而不是靠 review 盯着——理由与 `commands_are_async.rs` 完全一样：
//! 新增一个写方法时忘记加守卫是**无声的**，编译器和 clippy 都不会报，而后果是
//! 同步跑到一半被一次 stage / checkout 从中间插进来。

use std::fs;
use std::path::Path;

/// `trait Vcs` 里属于「写」的方法。读方法（open / graph / status / diff /
/// list_stashes / capabilities）**不得**加守卫：spec §5.1 明确要求读不阻塞。
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

fn svn_backend_src() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/vcs/svn.rs");
    fs::read_to_string(&path).expect("读不到 src/vcs/svn.rs")
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
fn read_methods_are_not_guarded() {
    // 反向约束，同样重要：spec §5.1 要求「读操作不阻塞」。给读方法也加上守卫的话，
    // 同步期间界面会整个瞎掉——而那恰恰是最需要显示状态的时候。
    let src = svn_backend_src();

    for m in ["open", "graph", "status", "diff", "list_stashes"] {
        assert!(
            !method_body(&src, m).contains("assert_not_syncing"),
            "读方法 `{m}` 不该有同步锁守卫：同步期间读必须照常可用（spec §5.1）"
        );
    }
}
