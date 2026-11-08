#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! dev 构建的独立配置目录：identifier 怎么变、首次启动从安装版复制哪些种子。
//! 目录全在 tempfile 里，不碰真实的 `%APPDATA%`。

use app_lib::dev_profile::{runtime_identifier, seed_dev_config, SEED_FILES};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn release_keeps_the_installed_identifier() {
    assert_eq!(
        runtime_identifier("com.momadacoding.jkk", false),
        "com.momadacoding.jkk"
    );
}

#[test]
fn debug_gets_a_dev_suffix() {
    assert_eq!(
        runtime_identifier("com.momadacoding.jkk", true),
        "com.momadacoding.jkk.dev"
    );
}

#[test]
fn debug_suffix_is_not_doubled() {
    // `pnpm tauri dev --config` 已经给了 .dev 结尾的 identifier 时不再叠一层
    assert_eq!(
        runtime_identifier("com.momadacoding.jkk.dev", true),
        "com.momadacoding.jkk.dev"
    );
}

#[test]
fn debug_suffix_keeps_a_custom_identifier_isolated() {
    // worktree 验证的老做法：--config 换成 .sitest 之类，加后缀后仍与安装版、普通 dev 都不同
    assert_eq!(
        runtime_identifier("com.momadacoding.jkk.sitest", true),
        "com.momadacoding.jkk.sitest.dev"
    );
}

#[test]
fn first_dev_launch_copies_the_seed_files_that_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = tmp.path().join("com.momadacoding.jkk");
    let dev = tmp.path().join("com.momadacoding.jkk.dev");
    fs::create_dir_all(&installed).unwrap();
    fs::write(
        installed.join("session.json"),
        r#"{"tabs":["D:\\work\\game"]}"#,
    )
    .unwrap();
    fs::write(installed.join("externalTools.json"), "{\"bc\":1}").unwrap();
    // 不在清单里的不复制：日志、缓存之类
    fs::write(installed.join("debug.log"), "x").unwrap();

    let copied = seed_dev_config(&installed, &dev).unwrap();

    assert_eq!(copied, vec!["session.json", "externalTools.json"]);
    assert_eq!(
        fs::read_to_string(dev.join("session.json")).unwrap(),
        r#"{"tabs":["D:\\work\\game"]}"#
    );
    assert_eq!(
        fs::read_to_string(dev.join("externalTools.json")).unwrap(),
        "{\"bc\":1}"
    );
    assert!(!dev.join("debug.log").exists());
    assert!(!dev.join("settings.json").exists());
    // 安装版的目录原样不动
    assert!(installed.join("session.json").exists());
}

#[test]
fn existing_dev_dir_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let installed = tmp.path().join("com.momadacoding.jkk");
    let dev = tmp.path().join("com.momadacoding.jkk.dev");
    fs::create_dir_all(&installed).unwrap();
    fs::create_dir_all(&dev).unwrap();
    fs::write(installed.join("session.json"), "installed").unwrap();
    fs::write(dev.join("session.json"), "dev").unwrap();

    let copied = seed_dev_config(&installed, &dev).unwrap();

    assert!(copied.is_empty());
    assert_eq!(fs::read_to_string(dev.join("session.json")).unwrap(), "dev");
}

#[test]
fn existing_but_empty_dev_dir_is_also_left_alone() {
    // 目录在就说明 dev 已经跑过（用户可能故意清空了配置），不再补种子
    let tmp = tempfile::tempdir().unwrap();
    let installed = tmp.path().join("com.momadacoding.jkk");
    let dev = tmp.path().join("com.momadacoding.jkk.dev");
    fs::create_dir_all(&installed).unwrap();
    fs::create_dir_all(&dev).unwrap();
    fs::write(installed.join("session.json"), "installed").unwrap();

    assert!(seed_dev_config(&installed, &dev).unwrap().is_empty());
    assert!(!dev.join("session.json").exists());
}

#[test]
fn no_installed_config_creates_nothing() {
    // 新机器没装过 jkk：不建空目录，交给应用自己按需创建
    let tmp = tempfile::tempdir().unwrap();
    let installed = tmp.path().join("com.momadacoding.jkk");
    let dev = tmp.path().join("com.momadacoding.jkk.dev");

    assert!(seed_dev_config(&installed, &dev).unwrap().is_empty());
    assert!(!dev.exists());
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn seed_list_covers_every_file_in_app_config_dir() {
    // 新增配置文件时要决定它该不该随种子复制；漏了的话 dev 首次启动就少一份配置。
    // 认法：用了 `app_config_dir()` 的源文件里，`.join("xxx.json")` 拼出来的文件名
    let mut files = Vec::new();
    collect_rs(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let mut found = BTreeSet::new();
    for f in &files {
        let src = fs::read_to_string(f).unwrap();
        if !src.contains("app_config_dir()") {
            continue;
        }
        for piece in src.split(".join(\"").skip(1) {
            let name = &piece[..piece.find('"').unwrap()];
            if name.ends_with(".json") {
                found.insert(name.to_string());
            }
        }
    }
    assert!(
        found.len() >= 4,
        "一个配置文件都没扫到，测试本身失效了：{found:?}"
    );
    let seeds: BTreeSet<String> = SEED_FILES.iter().map(|s| s.to_string()).collect();
    assert_eq!(found, seeds, "种子清单与 app_config_dir 下的配置文件不一致");
}
