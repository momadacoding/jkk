use app_lib::engine::external_tools_config::{self, ExternalToolsConfig};
use std::fs;
use tempfile::TempDir;

fn write(dir: &TempDir, text: &str) -> std::path::PathBuf {
    let path = dir.path().join("externalTools.json");
    fs::write(&path, text).unwrap();
    path
}

/// 配置文件缺失时写出 BC + VS Code 两个预设，并且落盘的内容能被自己再读回来。
#[test]
fn missing_file_writes_defaults_with_bc_and_vscode() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("externalTools.json");

    let cfg = external_tools_config::load_from(&path).unwrap();

    let ids: Vec<&str> = cfg.tools.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["bc", "vscode"]);
    assert_eq!(cfg.default_diff.as_deref(), Some("bc"));
    assert_eq!(cfg.default_merge.as_deref(), Some("bc"));
    assert!(cfg.rules.is_empty());
    #[cfg(target_os = "windows")]
    assert_eq!(
        cfg.tools[0].program,
        r"C:\Program Files\Beyond Compare 5\BComp.exe"
    );
    assert_eq!(
        cfg.tools[1].merge_args.as_deref(),
        Some(&["--wait", "--merge", "$REMOTE", "$LOCAL", "$BASE", "$MERGED"].map(String::from)[..])
    );

    assert!(path.exists(), "缺失时应当把默认预设落盘");
    let reread = external_tools_config::load_from(&path).unwrap();
    assert_eq!(cfg, reread);
}

/// 用户手写 JSON 漏个逗号，不能拿默认预设把整份配置冲掉。
#[test]
fn broken_json_errors_and_leaves_file_untouched() {
    let dir = TempDir::new().unwrap();
    let broken = "{ \"tools\": [ ";
    let path = write(&dir, broken);

    let err = external_tools_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "external_tools_config");
    assert!(
        err.message.contains("externalTools.json"),
        "报错要指向配置文件：{}",
        err.message
    );
    assert_eq!(fs::read(&path).unwrap(), broken.as_bytes());
}

/// 规则引用了不存在的工具：报配置错误，不回退到默认工具。
#[test]
fn rule_referencing_unknown_tool_is_rejected() {
    let dir = TempDir::new().unwrap();
    let path = write(
        &dir,
        r#"{"version":1,"rules":[{"patterns":["*.unity"],"merge":"nope"}],
            "tools":[{"id":"bc","label":"BC","program":"bcomp","diffArgs":["$LOCAL","$REMOTE"]}]}"#,
    );

    let err = external_tools_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "external_tools_config");
    assert!(err.message.contains("nope"), "{}", err.message);
    assert!(
        err.message.contains("externalTools.json"),
        "{}",
        err.message
    );
}

#[test]
fn default_referencing_unknown_tool_is_rejected() {
    let dir = TempDir::new().unwrap();
    let path = write(
        &dir,
        r#"{"version":1,"defaultDiff":"ghost",
            "tools":[{"id":"bc","label":"BC","program":"bcomp"}]}"#,
    );

    let err = external_tools_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "external_tools_config");
    assert!(err.message.contains("ghost"), "{}", err.message);
}

/// id 重复时规则和默认值指向哪个工具就说不清了，必须拒绝。
#[test]
fn duplicate_tool_ids_are_rejected() {
    let dir = TempDir::new().unwrap();
    let path = write(
        &dir,
        r#"{"version":1,"tools":[
            {"id":"bc","label":"A","program":"a"},
            {"id":"bc","label":"B","program":"b"}]}"#,
    );

    let err = external_tools_config::load_from(&path).unwrap_err();
    assert_eq!(err.code, "external_tools_config");
    assert!(err.message.contains("bc"), "{}", err.message);
}

/// 只写了 merge 的规则、没写 diffArgs 的工具都是合法配置。
#[test]
fn optional_fields_fall_back_to_none() {
    let dir = TempDir::new().unwrap();
    let path = write(
        &dir,
        r#"{"version":1,"defaultDiff":"bc",
            "rules":[{"patterns":["*.unity"],"merge":"uym"}],
            "tools":[
              {"id":"bc","label":"BC","program":"bcomp","diffArgs":["$LOCAL","$REMOTE"]},
              {"id":"uym","label":"UnityYAMLMerge","program":"u.exe","mergeArgs":["merge"]}]}"#,
    );

    let cfg: ExternalToolsConfig = external_tools_config::load_from(&path).unwrap();
    assert_eq!(cfg.default_merge, None);
    assert_eq!(cfg.rules[0].diff, None);
    assert_eq!(cfg.tools[1].diff_args, None);
}

/// BComp.exe 的退出码 1/2/11–14 是比较结果（相同、有差异、有冲突），不是故障（BC5 命令行文档）。
#[test]
fn bc_preset_treats_comparison_results_as_success() {
    let cfg = external_tools_config::default_config();
    let bc = external_tools_config::find_tool(&cfg, "bc").unwrap();
    for code in [0, 1, 2, 11, 12, 13, 14] {
        assert!(external_tools_config::is_success_exit(bc, code), "{code}");
    }
    for code in [3, 100, 101, 102, 103, 107] {
        assert!(!external_tools_config::is_success_exit(bc, code), "{code}");
    }
    assert!(external_tools_config::exit_code_message(bc, 103)
        .unwrap()
        .contains("找不到 BCompare.exe"));
    assert_eq!(external_tools_config::exit_code_message(bc, 13), None);
}

/// 没写 successExitCodes 的工具（VS Code、用户自己加的）只认 0。
#[test]
fn tool_without_success_codes_only_accepts_zero() {
    let cfg = external_tools_config::default_config();
    let code = external_tools_config::find_tool(&cfg, "vscode").unwrap();
    assert!(code.success_exit_codes.is_none());
    assert!(external_tools_config::is_success_exit(code, 0));
    assert!(!external_tools_config::is_success_exit(code, 1));
    assert_eq!(external_tools_config::exit_code_message(code, 1), None);
}

/// 用户在配置里写的退出码说明能读出来；JSON 的键只能是字符串。
#[test]
fn exit_codes_and_messages_load_from_json() {
    let dir = TempDir::new().unwrap();
    let path = write(
        &dir,
        r#"{"version":1,"tools":[{"id":"t","label":"T","program":"t.exe",
            "successExitCodes":[0,5],"exitCodeMessages":{"9":"磁盘满了"}}]}"#,
    );
    let cfg = external_tools_config::load_from(&path).unwrap();
    let t = &cfg.tools[0];
    assert!(external_tools_config::is_success_exit(t, 5));
    assert!(!external_tools_config::is_success_exit(t, 9));
    assert_eq!(
        external_tools_config::exit_code_message(t, 9).as_deref(),
        Some("磁盘满了")
    );
}

#[test]
fn find_tool_reports_unknown_id() {
    let cfg = external_tools_config::default_config();
    assert_eq!(
        external_tools_config::find_tool(&cfg, "vscode")
            .unwrap()
            .label,
        "VS Code"
    );
    let err = external_tools_config::find_tool(&cfg, "nope").unwrap_err();
    assert_eq!(err.code, "external_tools_config");
    assert!(err.message.contains("nope"), "{}", err.message);
}
