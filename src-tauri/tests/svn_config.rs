//! 配置校验的测试。
//!
//! 这个文件**用户手写**，所以它和输入框里敲的字符串是同一个可信级别，
//! 每个字段都会被送进 git 或 svn 的命令行（spec §6.2.1）。

use app_lib::engine::svn_config::{
    normalize_whitelist_entry, read_config, validate, validate_new_extensions,
    validate_whitelist_against_disk, write_config, SvnConfig,
};
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
        // 2026-09-13 独立 review：白名单漏挡 glob 元字符。实测 `["*"]` + `[".cs"]`
        // 生成的规则连 Art/boss.fbx 和 Code/Junk.psd 都收——**扩展名过滤被整个
        // 绕开、白名单退化成「整个仓库」**。extensions 那侧早就挡了 * 和 ?。
        "*",
        "Code/*",
        "Code/**",
        "Code/?.cs",
        "Code/[ab].cs",
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

/// 2026-10-28 现场那一类：整串列表被写成一条扩展名。以点开头、不含路径分隔符，
/// 旧校验全部放行，生成的规则匹配不到任何文件。
const LISTED_EXTENSIONS: &[&str] = &[
    ".cs、.lua",
    ".cs，.lua",
    ".cs,.lua",
    ".cs;.lua",
    ".cs；.lua",
    ".cs .lua",
    ".cs\t",
    ".cs\u{200B}",
    ".。cs",
    ".．cs",
    ".c[s]",
    ".cs]",
];

#[test]
fn an_extension_that_is_really_a_whole_list_cannot_be_written() {
    for bad in LISTED_EXTENSIONS {
        let exts = vec![".cs".to_string(), bad.to_string()];
        let err = validate_new_extensions(&exts).expect_err(&format!("{bad:?} 必须被拒"));
        assert_eq!(err.code, "svn_config", "{bad:?}");
        assert!(err.message.contains("英文逗号"), "{bad:?}: {}", err.message);

        // 写配置文件这条路同样要挡：它是唯一的落盘入口
        let dir = TempDir::new().unwrap();
        let mut cfg = ok_config();
        cfg.extensions = exts;
        write_config(dir.path().to_str().unwrap(), &cfg).expect_err(&format!("{bad:?}"));
        assert!(
            !dir.path().join(".git/mygitclient.json").exists(),
            "{bad:?}：被拒时不能留下配置文件"
        );
    }
    validate_new_extensions(&[".cs".to_string(), ".lua".to_string(), "*".to_string()]).unwrap();
}

#[test]
fn a_repo_built_by_an_older_version_with_a_listed_extension_still_opens() {
    // 旧版本写下的 `[".cs", ".lua、.java"]` 是**半能用**的：.cs 照常受管，上面可能
    // 已有提交和 stash。读配置时拒绝会把仓库整个锁死，应用里没有任何出路（独立 review）。
    // 这类扩展名不危险，只是匹配不到文件，exclude 与纳入集合核对按同一份字符串算，
    // 两边仍然一致——所以读盘照旧放行。
    for bad in LISTED_EXTENSIONS {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        let mut raw = serde_json::to_value(ok_config()).unwrap();
        raw["extensions"] = serde_json::json!([".cs", bad]);
        fs::write(
            dir.path().join(".git/mygitclient.json"),
            serde_json::to_string(&raw).unwrap(),
        )
        .unwrap();
        let cfg = read_config(dir.path().to_str().unwrap())
            .unwrap_or_else(|e| panic!("{bad:?} 不该锁死旧仓库：{}", e.message));
        assert_eq!(cfg.extensions, vec![".cs".to_string(), bad.to_string()]);
    }
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

// ---------------------------------------------------------------------------
// 白名单与磁盘对账（2026-09-13 独立 review 的 A2 / A3）
//
// 这两种配置**看着都没问题**，但行为完全不对，而且都不会报错：
// 漏尾斜杠 → 整棵子树绕过扩展名过滤进 git；大小写写错 → 纳入集合核对在一个
// 完全正常的仓库上误报阻断。
// ---------------------------------------------------------------------------

/// 造一个 `Code/` 目录 + 一个 `Code/Top.cs` 文件。
fn disk_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("Code/Sub")).unwrap();
    fs::write(dir.path().join("Code/Top.cs"), "x").unwrap();
    dir
}

#[test]
fn a_directory_entry_without_a_trailing_slash_is_rejected() {
    // 尾斜杠是「目录（递归）」与「单个文件」的唯一区分，而单文件条目**不过
    // 扩展名过滤**。实测 whitelist:["Code"] + extensions:[".cs"] 会把
    // Code/Junk.psd 也收进 git——少写一个 / 就是「整棵树全收」。
    let dir = disk_fixture();
    let root = dir.path().to_str().unwrap();
    let err = validate_whitelist_against_disk(root, &["Code".to_string()])
        .expect_err("目录条目漏尾斜杠必须被拒");
    assert_eq!(err.code, "svn_config");
    assert!(err.message.contains('/'), "错误信息要直说少了个斜杠");

    // 写对了就放行
    validate_whitelist_against_disk(root, &["Code/".to_string()]).unwrap();
    validate_whitelist_against_disk(root, &["Code/Top.cs".to_string()]).unwrap();
}

#[test]
fn a_file_entry_with_a_trailing_slash_is_rejected_too() {
    // 反过来也要挡：`Code/Top.cs/` 会走目录分支，生成一堆对不上任何东西的规则。
    let dir = disk_fixture();
    assert!(validate_whitelist_against_disk(
        dir.path().to_str().unwrap(),
        &["Code/Top.cs/".to_string()]
    )
    .is_err());
}

#[test]
fn a_case_mismatch_against_the_disk_is_rejected() {
    // Windows 上 git 的忽略规则匹配大小写**不敏感**，但 pathspec 匹配是**敏感**的。
    // 于是 "code/" 这种写法下 git 照常跟踪 Code/Top.cs，而
    // `git ls-files -- code/` 返回空——核对会报「应当被跟踪、实际没有」。
    //
    // NTFS 不区分大小写，所以不能靠 Path::exists() 判，只能列目录逐项比。
    let dir = disk_fixture();
    let root = dir.path().to_str().unwrap();
    let err = validate_whitelist_against_disk(root, &["code/".to_string()])
        .expect_err("大小写对不上必须被拒");
    assert_eq!(err.code, "svn_config");
    assert!(
        err.message.contains("Code"),
        "错误信息要给出磁盘上的真实写法，实际：{}",
        err.message
    );

    // 中间段的大小写同样要查
    assert!(validate_whitelist_against_disk(root, &["Code/top.cs".to_string()]).is_err());
}

#[test]
fn a_path_that_does_not_exist_on_disk_is_not_an_error_here() {
    // 本函数只管「存在但对不上」。条目可能是用户刚删掉的目录，在这里硬阻断
    // 会把人锁在仓库外面；「必须存在」是初始化向导（Phase 1b）自己的事。
    let dir = disk_fixture();
    let root = dir.path().to_str().unwrap();
    validate_whitelist_against_disk(root, &["Nope/".to_string()]).unwrap();
    validate_whitelist_against_disk(root, &["Code/Gone/".to_string()]).unwrap();
}

#[test]
fn an_empty_whitelist_is_a_no_op_here() {
    // G6：空列表一律 no-op。
    let dir = disk_fixture();
    validate_whitelist_against_disk(dir.path().to_str().unwrap(), &[]).unwrap();
}

#[test]
fn a_missing_entry_only_skips_itself_and_the_rest_is_still_checked() {
    // 「路径不存在时不报错」原本写成了 `return Ok(())`——那是**整函数返回**，
    // 于是第一条不存在的条目会把它后面所有条目的校验一并放行。
    // 独立 review 给的反例：`["Missing/", "Code"]` 通过，而 `Code` 单独送进来
    // 是被拒的（漏尾斜杠 → 整棵树绕开扩展名过滤）。
    let dir = disk_fixture();
    let root = dir.path().to_str().unwrap();

    // 前提断言：这条错误本身确实抓得到，否则下面测的就不是短路
    validate_whitelist_against_disk(root, &["Code".to_string()])
        .expect_err("前提不成立：Code 漏尾斜杠本就该被拒");

    let err = validate_whitelist_against_disk(root, &["Missing/".to_string(), "Code".to_string()])
        .expect_err("前面有条目不存在，不能让后面的错误蒙混过关");
    assert_eq!(err.code, "svn_config");
    assert!(err.message.contains("Code"), "实际：{}", err.message);

    // 大小写错误同样不能被前面的缺失条目掩盖
    validate_whitelist_against_disk(root, &["Missing/".to_string(), "code/".to_string()])
        .expect_err("大小写错误不能被前面的缺失条目掩盖");

    // 反向：缺失条目本身仍然不报错，跳过的只是它自己
    validate_whitelist_against_disk(root, &["Missing/".to_string(), "Code/".to_string()]).unwrap();
}
