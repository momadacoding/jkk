#[path = "build_support/git_describe.rs"]
mod git_describe;

use std::path::Path;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo 必然设置");
    let dir = Path::new(&manifest_dir);

    match git_describe::git_describe(dir) {
        Some(d) => println!("cargo:rustc-env=JKK_GIT_DESCRIBE={d}"),
        // 本地取不到就退化成「未知提交」；发版构建（release.yml 设了这个变量）
        // 宁可失败，也不能把标题不对的安装包发出去
        None if std::env::var_os("JKK_REQUIRE_GIT_DESCRIBE").is_some() => {
            panic!(
                "设置了 JKK_REQUIRE_GIT_DESCRIBE，但在 {} 下 git describe 失败",
                dir.display()
            )
        }
        None => {}
    }
    println!("cargo:rerun-if-env-changed=JKK_REQUIRE_GIT_DESCRIBE");
    // tauri_build 已经输出了 rerun-if-changed，cargo 不会再「任何文件变了就重跑」，
    // 所以切分支、提交、打 tag 必须显式盯住，否则标题会停在旧提交上
    for p in git_describe::watch_paths(dir) {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed=build_support/git_describe.rs");

    tauri_build::build()
}
