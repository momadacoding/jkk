#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! dev 的独立配置目录：identifier 怎么变、首次启动从安装版复制哪些种子。
//! 目录全在 tempfile 里，不碰真实的 `%APPDATA%`。

use app_lib::dev_profile::{
    runtime_identifier, seed_dev_config, INSTALLED_IDENTIFIER, NOT_SEEDED, SEED_FILES,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const MANIFEST: &str = env!("CARGO_MANIFEST_DIR");

#[test]
fn installed_identifier_matches_tauri_conf() {
    let conf = fs::read_to_string(Path::new(MANIFEST).join("tauri.conf.json")).unwrap();
    let conf: serde_json::Value = serde_json::from_str(&conf).unwrap();
    assert_eq!(conf["identifier"].as_str(), Some(INSTALLED_IDENTIFIER));
}

#[test]
fn tauri_build_keeps_the_installed_identifier() {
    assert_eq!(
        runtime_identifier(INSTALLED_IDENTIFIER, false),
        INSTALLED_IDENTIFIER
    );
}

#[test]
fn tauri_dev_gets_a_dev_suffix() {
    assert_eq!(
        runtime_identifier(INSTALLED_IDENTIFIER, true),
        "com.momadacoding.jkk.dev"
    );
}

#[test]
fn identifier_given_with_config_is_used_verbatim_in_dev() {
    // worktree 验证：--config 换成 .sitest，事先往 %APPDATA%\…sitest 写好 session.json，
    // 启动后必须读的就是这个目录，不能再被挪到 .sitest.dev
    assert_eq!(
        runtime_identifier("com.momadacoding.jkk.sitest", true),
        "com.momadacoding.jkk.sitest"
    );
    assert_eq!(
        runtime_identifier("com.momadacoding.jkk.dev", true),
        "com.momadacoding.jkk.dev"
    );
}

fn installed_with(files: &[(&str, &str)]) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let installed = tmp.path().join("com.momadacoding.jkk");
    let dev = tmp.path().join("com.momadacoding.jkk.dev");
    fs::create_dir_all(&installed).unwrap();
    for (name, body) in files {
        fs::write(installed.join(name), body).unwrap();
    }
    (tmp, installed, dev)
}

#[test]
fn first_dev_launch_copies_only_tools_and_commands() {
    let (_tmp, installed, dev) = installed_with(&[
        ("externalTools.json", "{\"bc\":1}"),
        ("customCommands.json", "{\"cmd\":1}"),
        // 这两份同在配置目录，但故意不复制：会话会打开真实仓库，设置里有全局快捷键
        ("session.json", r#"{"tabs":["D:\\work\\game"]}"#),
        ("settings.json", r#"{"showShortcut":"Ctrl+Alt+J"}"#),
        ("debug.log", "x"),
    ]);

    let copied = seed_dev_config(&installed, &dev).unwrap();

    assert_eq!(copied, vec!["externalTools.json", "customCommands.json"]);
    assert_eq!(
        fs::read_to_string(dev.join("externalTools.json")).unwrap(),
        "{\"bc\":1}"
    );
    assert_eq!(
        fs::read_to_string(dev.join("customCommands.json")).unwrap(),
        "{\"cmd\":1}"
    );
    for name in ["session.json", "settings.json", "debug.log"] {
        assert!(!dev.join(name).exists(), "{name} 不该复制");
    }
    // 安装版的目录原样不动
    assert!(installed.join("session.json").exists());
    // 中转目录不留痕
    assert!(!dev
        .with_file_name("com.momadacoding.jkk.dev.seeding")
        .exists());
}

#[test]
fn existing_dev_dir_is_never_overwritten() {
    let (_tmp, installed, dev) = installed_with(&[("externalTools.json", "installed")]);
    fs::create_dir_all(&dev).unwrap();
    fs::write(dev.join("externalTools.json"), "dev").unwrap();

    assert!(seed_dev_config(&installed, &dev).unwrap().is_empty());
    assert_eq!(
        fs::read_to_string(dev.join("externalTools.json")).unwrap(),
        "dev"
    );
}

#[test]
fn existing_but_empty_dev_dir_is_also_left_alone() {
    // 目录在就说明 dev 已经跑过（用户可能故意清空了配置），不再补种子
    let (_tmp, installed, dev) = installed_with(&[("externalTools.json", "installed")]);
    fs::create_dir_all(&dev).unwrap();

    assert!(seed_dev_config(&installed, &dev).unwrap().is_empty());
    assert!(!dev.join("externalTools.json").exists());
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

#[test]
fn installed_without_seed_files_creates_nothing() {
    // 安装版只跑过、没配过外部工具和自定义命令：也不建空的 dev 目录
    let (_tmp, installed, dev) = installed_with(&[("session.json", "{}")]);

    assert!(seed_dev_config(&installed, &dev).unwrap().is_empty());
    assert!(!dev.exists());
}

#[test]
fn leftover_staging_from_a_crashed_seed_is_replaced() {
    // 上次复制到一半进程没了：.seeding 里是半成品，dev 目录还不存在
    let (_tmp, installed, dev) = installed_with(&[
        ("externalTools.json", "new"),
        ("customCommands.json", "new"),
    ]);
    let staging = dev.with_file_name("com.momadacoding.jkk.dev.seeding");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("externalTools.json"), "half").unwrap();
    fs::write(staging.join("stray.txt"), "x").unwrap();

    let copied = seed_dev_config(&installed, &dev).unwrap();

    assert_eq!(copied.len(), 2);
    assert_eq!(
        fs::read_to_string(dev.join("externalTools.json")).unwrap(),
        "new"
    );
    assert!(!dev.join("stray.txt").exists());
    assert!(!staging.exists());
}

#[test]
fn failed_seed_leaves_no_dev_dir_behind() {
    // 要么全到、要么没有：失败后 dev 目录不存在，下次启动还会再试
    let (tmp, installed, _) = installed_with(&[("externalTools.json", "x")]);
    // dev 目录的上级是个文件：中转目录建不出来
    let blocker = tmp.path().join("not-a-dir");
    fs::write(&blocker, "x").unwrap();
    let dev = blocker.join("com.momadacoding.jkk.dev");

    assert!(seed_dev_config(&installed, &dev).is_err());
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
fn literal_json_joins_next_to_app_config_dir_are_all_classified() {
    // 新增配置文件时要决定它复制还是不复制，两张清单都没写的话这里提醒。
    //
    // 认法有局限，名字照实说：只扫「用了 `app_config_dir()` 的源文件」里 `.join("xxx.json")`
    // 这种字面量写法。文件名放在常量里、或者在别的模块拼路径的，这里看不见。
    let mut files = Vec::new();
    collect_rs(&Path::new(MANIFEST).join("src"), &mut files);
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
    let classified: BTreeSet<String> = SEED_FILES
        .iter()
        .chain(NOT_SEEDED.iter())
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        found, classified,
        "app_config_dir 下的配置文件要么在 SEED_FILES、要么在 NOT_SEEDED"
    );
}
