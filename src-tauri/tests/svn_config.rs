//! 配置校验的测试。
//!
//! 这个文件**用户手写**，所以它和输入框里敲的字符串是同一个可信级别，
//! 每个字段都会被送进 git 或 svn 的命令行（spec §6.2.1）。

use app_lib::engine::svn_config::{normalize_whitelist_entry, read_config, validate, SvnConfig};
use std::fs;
use tempfile::TempDir;

fn ok_config() -> SvnConfig {
    SvnConfig {
        version: 1,
        mode: "svn-git".to_string(),
        svn_url: "https://svn.example.com/game/trunk".to_string(),
        whitelist: vec!["Assets/Scripts/".to_string()],
        extensions: vec![".cs".to_string()],
        baseline_branch: "svn/base".to_string(),
    }
}

#[test]
fn a_well_formed_config_passes() {
    validate(&ok_config()).unwrap();
}

#[test]
fn baseline_branch_must_be_exactly_svn_base() {
    // 全文所有流程都硬编码了这个名字，留一个可配字段只会让两者不一致（spec §6.2.1）。
    let mut cfg = ok_config();
    cfg.baseline_branch = "svn/mybase".to_string();
    let err = validate(&cfg).expect_err("非 svn/base 必须被拒");
    assert_eq!(err.code, "svn_config");
    assert!(err.message.contains("baselineBranch"));
}

#[test]
fn both_lists_empty_is_rejected() {
    // 空白名单会让空参数保护在每一步触发，等于功能不可用（spec §6.2.1）。
    let mut cfg = ok_config();
    cfg.whitelist.clear();
    assert!(validate(&cfg).is_err());

    let mut cfg = ok_config();
    cfg.extensions.clear();
    assert!(validate(&cfg).is_err());
}

#[test]
fn dangerous_whitelist_entries_are_rejected() {
    // 白名单条目同时被当 git pathspec 和 SVN target 用，两套语义各有魔法字符。
    // 本项目已因空 pathspec 误伤过整个仓库（AGENTS.md §6.2）。
    for bad in [
        "",                 // 空串
        "   ",              // 只有空白
        "/",                // 根
        ".",                // 当前目录
        "..",               // 上跳
        "Assets/../../etc", // 中间段上跳
        "C:/Windows",       // 盘符
        "//server/share",   // UNC
        ":(exclude)Assets", // git pathspec magic
        ".git/hooks",       // 以 .git 开头
        ".svn/wc.db",       // 以 .svn 开头
        "./",               // 精确比较挡不住它，而 ./ 两边都等于「全部」
        ".//",              // 同上
        "./Assets",         // 首段是 .
        "Assets/./Scripts", // 中间段是 .
        ".SVN/wc.db",       // NTFS 大小写不敏感，会命中真实的 .svn
        ".Git/hooks",       // 同上
        "-rHEAD",           // 会被 svn 当成选项解析
        "--version",        // 同上
    ] {
        assert!(
            normalize_whitelist_entry(bad).is_err(),
            "{bad:?} 应当被拒绝"
        );
    }
}

#[test]
fn windows_backslashes_are_normalized_to_forward_slashes() {
    // 跨平台路径是本项目的高发缺陷区（AGENTS.md §8）。
    assert_eq!(
        normalize_whitelist_entry(r"Assets\Scripts\").unwrap(),
        "Assets/Scripts/"
    );
    assert_eq!(
        normalize_whitelist_entry(r"Assets\Scripts\Player.cs").unwrap(),
        "Assets/Scripts/Player.cs"
    );
}

#[test]
fn a_backslash_drive_letter_is_rejected_too() {
    // `C:\Windows` 规范化之后才看得出是盘符——校验必须在规范化**之后**做，
    // 否则反斜杠写法会绕过盘符检查。
    assert!(normalize_whitelist_entry(r"C:\Windows").is_err());
}

#[test]
fn whitelist_entries_inside_externals_are_allowed() {
    // §6.5 P4 改写之后，白名单**可以**落在 external 内——那正是目标场景
    // （真实 Unity 工程的代码就在 external 里，spec §1.1.1）。
    // 跨仓库的拦截在 svn_wc 的 P4，不在这里按路径文本猜。
    assert_eq!(normalize_whitelist_entry("Code/").unwrap(), "Code/");
}

#[test]
fn a_trailing_slash_is_preserved() {
    // 尾斜杠是「目录（递归）」与「单个文件」的唯一区分（spec §6.3），
    // 规范化时吃掉它会让 exclude 生成走错分支。
    assert!(normalize_whitelist_entry("Code/").unwrap().ends_with('/'));
    assert!(!normalize_whitelist_entry("Code/A.cs")
        .unwrap()
        .ends_with('/'));
}

#[test]
fn extensions_must_start_with_a_dot_and_carry_no_separators() {
    let mut cfg = ok_config();
    cfg.extensions = vec!["cs".to_string()];
    assert!(validate(&cfg).is_err(), "缺前导点必须被拒");

    let mut cfg = ok_config();
    cfg.extensions = vec![".a/b".to_string()];
    assert!(validate(&cfg).is_err(), "含路径分隔符必须被拒");

    // "*" 是 spec §6.3 明文给出的全纳入写法，必须放行
    let mut cfg = ok_config();
    cfg.extensions = vec!["*".to_string()];
    validate(&cfg).unwrap();
}

#[test]
fn an_unknown_version_is_rejected() {
    // 与 baselineBranch 被钉死同一条理由：留一个不校验的字段，只会让「文件里
    // 写的」与「代码假设的」不一致。这里一个版本语义差异会直接改变白名单的
    // 含义，进而改变哪些文件受保护——比 custom_config 那个 version 严重得多。
    let mut cfg = ok_config();
    cfg.version = 2;
    let err = validate(&cfg).expect_err("未知 version 必须被拒");
    assert_eq!(err.code, "svn_config");
    assert!(err.message.contains("version"));
}

#[test]
fn a_wrong_mode_is_rejected() {
    // §6.1 的仓库识别据 mode 判定这是不是混合仓库。
    for bad in ["", "git", "svn", "svn_git"] {
        let mut cfg = ok_config();
        cfg.mode = bad.to_string();
        assert!(validate(&cfg).is_err(), "mode={bad:?} 应当被拒绝");
    }
}

#[test]
fn svn_url_must_not_be_empty() {
    let mut cfg = ok_config();
    cfg.svn_url.clear();
    assert!(validate(&cfg).is_err());
}

#[test]
fn a_missing_config_file_reports_not_configured() {
    // 这个错误码是 §6.1 分流的输入之一：「有 .svn 无配置」与「配置坏了」
    // 要给出完全不同的引导，不能塌缩成同一个错误。
    let dir = TempDir::new().unwrap();
    let err = read_config(dir.path().to_str().unwrap()).expect_err("没有配置文件应当报错");
    assert_eq!(err.code, "not_configured");
}

#[test]
fn a_malformed_config_file_reports_svn_config_not_not_configured() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/mygitclient.json"), "{ not json").unwrap();
    let err = read_config(dir.path().to_str().unwrap()).expect_err("坏 JSON 应当报错");
    assert_eq!(err.code, "svn_config", "不能把「坏了」报成「没配」");
}

#[test]
fn read_config_normalizes_before_validating() {
    // 读盘路径必须把反斜杠写法也挡住——校验若在规范化之前做，
    // `C:\Windows` 这种写法会从盘符检查底下溜过去。
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(
        dir.path().join(".git/mygitclient.json"),
        r#"{"version":1,"mode":"svn-git","svnUrl":"https://x/y",
            "whitelist":["C:\\Windows"],"extensions":[".cs"],
            "baselineBranch":"svn/base"}"#,
    )
    .unwrap();
    assert!(read_config(dir.path().to_str().unwrap()).is_err());
}

#[test]
fn read_config_accepts_a_good_file_and_normalizes_separators() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(
        dir.path().join(".git/mygitclient.json"),
        r#"{"version":1,"mode":"svn-git","svnUrl":"https://svn.example.com/game/trunk",
            "whitelist":["Assets\\Scripts\\"],"extensions":[".cs",".meta"],
            "baselineBranch":"svn/base"}"#,
    )
    .unwrap();
    let cfg = read_config(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.whitelist, vec!["Assets/Scripts/".to_string()]);
    assert_eq!(cfg.extensions, vec![".cs".to_string(), ".meta".to_string()]);
}
