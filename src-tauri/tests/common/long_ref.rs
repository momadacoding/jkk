//! 复现「libgit2 遍历 refs 时被超长路径静默截断」的构造。
//!
//! 现场是 Codex 写的 `refs/codex/turn-diffs/checkpoints/<hash>/<hash>/...`：绝对路径 274 字符，
//! 按字母序排在 `refs/heads` 之前。libgit2 在 Windows 上读到它时遍历提前结束且不报错，
//! 后面的 loose ref 全部漏掉，只剩 `packed-refs` 里的旧值。
//!
//! 只有 Windows 上才会真的截断，其他平台这些用例恒绿。

use std::fs;
use std::path::Path;

/// 在 `repo` 里种一个排在 `refs/heads` 之前的超长 loose ref，指向 `oid`。
///
/// 直接写文件而不是 `git update-ref`：git 要 `core.longpaths` 才写得进去，而这个配置
/// 取决于跑测试的机器；Rust 标准库在 Windows 上会自己转成 `\\?\` 路径。
pub fn plant_long_loose_ref(repo: &Path, oid: &str) {
    let mut dir = repo.join(".git").join("refs").join("codex");
    for _ in 0..4 {
        dir.push("a".repeat(100));
    }
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("r");
    fs::write(&file, format!("{oid}\n")).unwrap();
    assert!(file.is_file(), "fixture 没写出超长 ref");
}
