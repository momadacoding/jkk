use app_lib::engine::custom_config::{self, CustomCommand};
use std::fs;
use tempfile::TempDir;

/// 配置文件缺失时要写出当前平台的默认预设，而不是返回空列表。
#[test]
fn missing_file_writes_platform_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("customCommands.json");

    let commands = custom_config::load_from(&path).unwrap();

    assert!(!commands.is_empty(), "默认预设不该是空的");
    assert!(path.exists(), "缺失时应当把默认预设落盘");
    // 落盘的内容必须能被自己再读回来，否则用户第一次改配置就会踩到格式问题
    let reread = custom_config::load_from(&path).unwrap();
    assert_eq!(commands, reread);
}

/// 用户手写配置写错一个逗号，不能把整份配置冲掉。
#[test]
fn broken_json_errors_and_leaves_file_untouched() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    let broken = b"{ \"commands\": [ ";
    fs::write(&path, broken).unwrap();

    let err = custom_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "custom_config");

    let after = fs::read(&path).unwrap();
    assert_eq!(after, broken, "解析失败时绝不能改写用户的文件");
}

#[test]
fn unknown_fields_are_ignored() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"future":"whatever","commands":[
            {"id":"a","label":"A","program":"echo","args":["x"],"extra":true}
        ]}"#,
    )
    .unwrap();

    let commands = custom_config::load_from(&path).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].program, "echo");
}

#[test]
fn args_and_cwd_fall_back_to_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"commands":[{"id":"a","label":"A","program":"code"}]}"#,
    )
    .unwrap();

    let commands = custom_config::load_from(&path).unwrap();
    assert_eq!(commands[0].args, Vec::<String>::new());
    assert_eq!(commands[0].cwd, "$WORKTREE");
}

#[test]
fn empty_command_list_is_valid() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(&path, r#"{"version":1,"commands":[]}"#).unwrap();

    assert_eq!(custom_config::load_from(&path).unwrap(), Vec::new());
}

/// id 重复只是小瑕疵，不值得让用户丢掉整份配置。
#[test]
fn duplicate_ids_are_kept() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("customCommands.json");
    fs::write(
        &path,
        r#"{"version":1,"commands":[
            {"id":"a","label":"One","program":"echo"},
            {"id":"a","label":"Two","program":"echo"}
        ]}"#,
    )
    .unwrap();

    assert_eq!(custom_config::load_from(&path).unwrap().len(), 2);
}

/// 默认预设里每一条都必须字段完整，否则第一次启动就是坏的。
#[test]
fn platform_defaults_are_well_formed() {
    for cmd in custom_config::default_commands() {
        let CustomCommand {
            id, label, program, ..
        } = &cmd;
        assert!(!id.is_empty(), "{cmd:?} 缺 id");
        assert!(!label.is_empty(), "{cmd:?} 缺 label");
        assert!(!program.is_empty(), "{cmd:?} 缺 program");
    }
}
