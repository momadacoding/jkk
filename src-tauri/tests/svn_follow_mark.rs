//! 失焦快照、「我是谁」与 wc.db 戳（计划 Task 9；分析 §6.6、§6.10；日志 E19、W26）。
//!
//! git 写操作全部在 fixture 里做。`svn auth` 只碰本文件在临时目录里造的假缓存
//! （`--config-dir`，虚构的用户名与口令），绝不读开发机真实的凭据缓存（AGENTS.md §11）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::commands::svn::{svn_follow_mark, svn_get_user, svn_set_user, svn_wc_db_stamps};
use app_lib::engine::git_cli::{run_git, run_git_stdin};
use app_lib::engine::svn_config::{read_config, write_config, SvnConfig};
use app_lib::engine::svn_follow_mark::{
    clear_mark, mark_path, my_svn_user, parse_svn_auth, read_mark, resolve_svn_user,
    svn_auth_listing, wc_db_stamps, wc_db_stamps_with, write_mark, SvnUserInfo, WcDbStamp,
};
use app_lib::engine::svn_wc::{enumerate_wcs, WcIdentity};
use app_lib::error::GitError;
use std::cell::Cell;
use std::fs;
use std::path::Path;
use svn_fixtures::{
    hybrid_whitelist_in_external, hybrid_with_colleague, svn_cmd, svn_repo,
    svn_repo_with_same_repo_nested_external, NestedExternal, SvnFixture,
};
use tempfile::TempDir;

const ZERO: &str = "0000000000000000000000000000000000000000";

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn write(fx: &SvnFixture, rel: &str, content: &str) {
    let p = fx.wc().join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn plain_git_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path().to_str().unwrap(), &["init", "-q"]).unwrap();
    dir
}

// ---------------------------------------------------------------------------
// 失焦快照
// ---------------------------------------------------------------------------

#[test]
fn mark_records_staged_blobs_and_head() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    write(&fx, "Code/带 空格 f.cs", "new\n");
    git(&fx, &["add", "--", "Code/A.cs", "Code/带 空格 f.cs"]);
    git(&fx, &["rm", "-q", "--cached", "--", "Code/B.cs"]);

    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str()).unwrap().expect("应当写了快照");
    assert_eq!(mark.head_oid, git(&fx, &["rev-parse", "HEAD"]));
    assert_eq!(mark.branch.as_deref(), Some("work"));
    assert!(mark.at > 1_600_000_000, "at 是 unix 秒，实际 {}", mark.at);
    assert_eq!(mark.staged.len(), 3, "实际 {:?}", mark.staged);
    assert_eq!(
        mark.staged["Code/A.cs"],
        git(&fx, &["rev-parse", ":Code/A.cs"])
    );
    // 路径带空格与中文：`-z` 原样，不被 core.quotepath 转义
    assert_eq!(
        mark.staged["Code/带 空格 f.cs"],
        git(&fx, &["rev-parse", ":Code/带 空格 f.cs"])
    );
    assert_eq!(mark.staged["Code/B.cs"], ZERO, "删除记全 0 oid");
    assert_eq!(
        mark_path(&fx.wc_str()),
        fx.wc().join(".git/mygitclient/follow-mark.json")
    );
    // rename 之后不留临时文件
    let names: Vec<String> = fs::read_dir(fx.wc().join(".git/mygitclient"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(names, ["follow-mark.json"]);
}

#[test]
fn mark_ignores_unstaged_changes() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "only on disk\n");
    write(&fx, "Code/New.cs", "untracked\n");

    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str())
        .unwrap()
        .expect("没有暂存也要写快照");
    assert!(mark.staged.is_empty(), "实际 {:?}", mark.staged);
}

#[test]
fn mark_keeps_a_file_staged_then_changed_on_disk_at_its_staged_blob() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "staged\n");
    git(&fx, &["add", "--", "Code/A.cs"]);
    let staged = git(&fx, &["rev-parse", ":Code/A.cs"]);
    write(&fx, "Code/A.cs", "changed after staging\n");

    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(mark.staged["Code/A.cs"], staged);
}

#[test]
fn mark_leaves_out_intent_to_add_entries() {
    // `diff-index --cached` 默认把 i-t-a 报成 A + 空 blob（实测）。那不是暂存内容：记进去，
    // 跟进时「当前暂存 == 快照」成立，这个条目就可能被当成「自己提交过」让位丢掉。
    let fx = hybrid_with_colleague();
    write(&fx, "Code/Ita.cs", "later\n");
    git(&fx, &["add", "-N", "--", "Code/Ita.cs"]);
    // 真的暂存一个空文件：blob 同样是 e69de29…，不能靠 oid 排除 i-t-a
    write(&fx, "Code/Empty.cs", "");
    git(&fx, &["add", "--", "Code/Empty.cs"]);

    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str()).unwrap().unwrap();
    assert!(
        !mark.staged.contains_key("Code/Ita.cs"),
        "{:?}",
        mark.staged
    );
    assert!(
        mark.staged.contains_key("Code/Empty.cs"),
        "{:?}",
        mark.staged
    );
}

#[test]
fn mark_leaves_out_conflicted_entries() {
    // 冲突条目在 diff-index 里是 U + 全 0，与「删除」同形。记进去就成了一条假的删除暂存
    let fx = hybrid_with_colleague();
    let blob = git(&fx, &["rev-parse", "HEAD:Code/A.cs"]);
    let info = format!(
        "0 {ZERO} 0\tCode/A.cs\n100644 {blob} 1\tCode/A.cs\n\
         100644 {blob} 2\tCode/A.cs\n100644 {blob} 3\tCode/A.cs\n"
    );
    run_git_stdin(
        &fx.wc_str(),
        &["update-index", "--index-info"],
        info.as_bytes(),
    )
    .unwrap();
    assert!(
        !git(&fx, &["ls-files", "-u"]).is_empty(),
        "fixture 没造出冲突"
    );

    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str()).unwrap().unwrap();
    assert!(mark.staged.is_empty(), "实际 {:?}", mark.staged);
}

#[test]
fn mark_records_no_branch_when_detached() {
    let fx = hybrid_with_colleague();
    git(&fx, &["checkout", "-q", "--detach"]);

    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(mark.branch, None);
    assert_eq!(mark.head_oid, git(&fx, &["rev-parse", "HEAD"]));
}

#[test]
fn mark_is_not_written_while_head_is_unborn() {
    let dir = plain_git_repo();
    let repo = dir.path().to_str().unwrap();

    write_mark(repo).unwrap();

    assert_eq!(read_mark(repo).unwrap(), None);
    assert!(!dir.path().join(".git/mygitclient").exists());
}

#[test]
fn mark_never_creates_a_dot_git_in_a_directory_without_one() {
    // 目录本身没有 `.git`，但 git 会往上找到外层仓库、HEAD 解析得出来。建目录时要是
    // 顺手造出一个 `.git`，§6.1 随后会把这个目录认成 SvnSuspicious
    let dir = plain_git_repo();
    let repo = dir.path().to_str().unwrap();
    fs::write(dir.path().join("f.txt"), "x").unwrap();
    run_git(repo, &["add", "f.txt"]).unwrap();
    run_git(
        repo,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "-m",
            "c",
        ],
    )
    .unwrap();
    let sub = dir.path().join("sub");
    fs::create_dir(&sub).unwrap();

    write_mark(sub.to_str().unwrap()).unwrap();

    assert!(!sub.join(".git").exists());
}

#[test]
fn a_later_mark_replaces_the_earlier_one_and_clear_removes_it() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "one\n");
    git(&fx, &["add", "--", "Code/A.cs"]);
    write_mark(&fx.wc_str()).unwrap();
    write(&fx, "Code/A.cs", "two\n");
    git(&fx, &["add", "--", "Code/A.cs"]);
    write_mark(&fx.wc_str()).unwrap();

    let mark = read_mark(&fx.wc_str()).unwrap().unwrap();
    assert_eq!(
        mark.staged["Code/A.cs"],
        git(&fx, &["rev-parse", ":Code/A.cs"])
    );

    clear_mark(&fx.wc_str());
    assert_eq!(read_mark(&fx.wc_str()).unwrap(), None);
    clear_mark(&fx.wc_str()); // 没有快照时也不出事
}

#[test]
fn a_missing_mark_reads_as_none() {
    let dir = plain_git_repo();
    assert_eq!(read_mark(dir.path().to_str().unwrap()).unwrap(), None);
}

#[test]
fn a_damaged_mark_reads_as_none_instead_of_blocking_the_follow() {
    let dir = plain_git_repo();
    let repo = dir.path().to_str().unwrap();
    let path = mark_path(repo);
    fs::create_dir_all(path.parent().unwrap()).unwrap();

    let oid = "a".repeat(40);
    let cases: Vec<Vec<u8>> = vec![
        b"{ half a json".to_vec(),
        b"".to_vec(),
        vec![0xff, 0xfe, 0x00],
        // 字段缺失
        br#"{"at":1}"#.to_vec(),
        // 形状对、oid 不对：这个值以后要进 git 命令行
        br#"{"at":1,"headOid":"--evil","branch":null,"staged":{}}"#.to_vec(),
        format!(r#"{{"at":1,"headOid":"{oid}","branch":null,"staged":{{"a":"zz"}}}}"#).into_bytes(),
        format!(r#"{{"at":1,"headOid":"{oid}","baseOid":"--evil","branch":null,"staged":{{}}}}"#)
            .into_bytes(),
    ];
    for bad in cases {
        fs::write(&path, &bad).unwrap();
        assert_eq!(
            read_mark(repo).unwrap(),
            None,
            "应当按没有快照处理：{}",
            String::from_utf8_lossy(&bad)
        );
    }
    // 对照：同样的形状、oid 合法时读得出来，上面不是因为别的原因才读成 None
    fs::write(
        &path,
        format!(r#"{{"at":1,"headOid":"{oid}","branch":null,"staged":{{"a":"{oid}"}}}}"#),
    )
    .unwrap();
    let old_format = read_mark(repo)
        .unwrap()
        .expect("旧格式没有 baseOid，照样读得出来");
    assert_eq!(old_format.base_oid, None);
}

#[test]
fn mark_in_a_repo_without_a_baseline_has_no_base_oid() {
    let dir = plain_git_repo();
    let repo = dir.path().to_str().unwrap();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    run_git(repo, &["add", "a.txt"]).unwrap();
    run_git(repo, &["commit", "-qm", "c"]).unwrap();

    write_mark(repo).unwrap();

    assert_eq!(read_mark(repo).unwrap().unwrap().base_oid, None);
}

// ---------------------------------------------------------------------------
// 「我是谁」
// ---------------------------------------------------------------------------

const SAMPLE_AUTH: &str = "\
------------------------------------------------------------------------
Credential kind: svn.simple
Authentication realm: <https://svn.corp.example:443> Corp Realm
Password cache: wincrypt
Password: [not shown]
Username: alice

------------------------------------------------------------------------
Credential kind: svn.simple
Authentication realm: <svn://svn.other.test:3690> 11111111-2222-3333-4444-555555555555
Password cache: wincrypt
Password: [not shown]
Username: bob

------------------------------------------------------------------------
Credential kind: svn.ssl.server
Authentication realm: https://svn.corp.example:443
Hostname: svn.corp.example
Valid from: Jan  1 00:00:00 2026 GMT

Credentials cache in 'C:\\Users\\x\\AppData\\Roaming\\Subversion' contains 3 credentials
";

#[test]
fn svn_auth_output_parsing_picks_matching_realm() {
    assert_eq!(
        parse_svn_auth(SAMPLE_AUTH, "https://svn.corp.example/repos/game"),
        Some("alice".to_string())
    );
    assert_eq!(
        parse_svn_auth(SAMPLE_AUTH, "svn://svn.other.test/game"),
        Some("bob".to_string())
    );
}

#[test]
fn realm_match_ignores_case_and_default_ports_but_not_scheme_host_or_other_ports() {
    let cases: &[(&str, Option<&str>)] = &[
        ("HTTPS://SVN.Corp.Example/repos", Some("alice")),
        ("https://svn.corp.example:443/repos", Some("alice")),
        ("svn://SVN.OTHER.test:3690/game", Some("bob")),
        // URL 里带用户名：realm 里没有这段
        ("https://zhang@svn.corp.example/repos", Some("alice")),
        // scheme 不同是另一个 realm
        ("http://svn.corp.example/repos", None),
        ("svn://svn.corp.example/repos", None),
        // host 不同，包括只是前缀相同
        ("https://svn.corp.example.evil.test/repos", None),
        ("https://corp.example/repos", None),
        // 非默认端口
        ("https://svn.corp.example:8443/repos", None),
        ("svn://svn.other.test:3691/game", None),
    ];
    for (root, want) in cases {
        assert_eq!(
            parse_svn_auth(SAMPLE_AUTH, root).as_deref(),
            *want,
            "repos_root = {root}"
        );
    }
}

#[test]
fn realm_without_explicit_port_or_brackets_still_matches() {
    let text = "\
Credential kind: svn.username
Authentication realm: svn+ssh://Host.Example
Username: carol
";
    assert_eq!(
        parse_svn_auth(text, "svn+ssh://zhang@host.example/repos"),
        Some("carol".to_string())
    );
    let v6 = "\
Authentication realm: <https://[::1]:8443> Lab
Username: dave
";
    assert_eq!(
        parse_svn_auth(v6, "https://[::1]:8443/r"),
        Some("dave".to_string())
    );
    assert_eq!(parse_svn_auth(v6, "https://[::1]/r"), None);
}

#[test]
fn several_usernames_for_one_realm_is_not_a_guess() {
    let text = "\
Authentication realm: <https://h:443> Realm A
Username: alice

Authentication realm: <https://h:443> Realm B
Username: bob
";
    assert_eq!(parse_svn_auth(text, "https://h/r"), None);

    let same = "\
Authentication realm: <https://h:443> Realm A
Username: alice

Authentication realm: <https://h:443> Realm B
Username: alice
";
    assert_eq!(
        parse_svn_auth(same, "https://h/r"),
        Some("alice".to_string())
    );
}

#[test]
fn a_username_that_did_not_survive_decoding_is_not_an_identity() {
    // `svn auth` 经管道输出系统代码页（实测 GBK 的 d5 c5 c8 fd），到解析时已经 lossy 过
    let raw = b"Authentication realm: <https://h:443> R\nUsername: \xd5\xc5\xc8\xfd\n";
    let text = String::from_utf8_lossy(raw);
    assert_eq!(parse_svn_auth(&text, "https://h/r"), None);

    // 与 svn 写出来的 UTF-8 用户名不同：这条走得通
    let utf8 = "Authentication realm: <https://h:443> R\nUsername: 张三\n";
    assert_eq!(
        parse_svn_auth(utf8, "https://h/r"),
        Some("张三".to_string())
    );
}

#[test]
fn a_username_only_counts_under_its_own_realm() {
    // 前一块的 realm 匹配、但没有 Username（svn.ssl.server）：不能借用下一块的
    let text = "\
------------------------------------------------------------------------
Authentication realm: https://h:443
Hostname: h

------------------------------------------------------------------------
Authentication realm: <https://other:443> Realm
Username: bob
";
    assert_eq!(parse_svn_auth(text, "https://h/r"), None);

    let no_realm = "Username: bob\n";
    assert_eq!(parse_svn_auth(no_realm, "https://h/r"), None);
}

#[test]
fn file_repositories_have_no_realm() {
    let text = "\
Authentication realm: <https://h:443> Realm
Username: alice
";
    assert_eq!(parse_svn_auth(text, "file:///C:/repo"), None);
    assert_eq!(parse_svn_auth(text, "file://server/share/repo"), None);
    assert_eq!(parse_svn_auth(text, ""), None);
    assert_eq!(parse_svn_auth(text, "not a url"), None);
}

#[test]
fn svn_user_prefers_config() {
    let fx = hybrid_with_colleague();
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some("  zhangsan ".to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();

    // 配置里有就不跑 `svn auth`：这里换成 https 的根，证明不是靠 file:// 才没跑
    assert_eq!(
        my_svn_user(&fx.wc_str(), "https://svn.corp.example/repos"),
        Some("zhangsan".to_string())
    );
}

#[test]
fn no_config_and_no_credentials_means_no_identity() {
    // file:// 的 fixture：没有 realm，不发 `svn auth`，也就不会读到开发机的凭据缓存
    let fx = hybrid_with_colleague();
    assert_eq!(my_svn_user(&fx.wc_str(), "file:///C:/repo"), None);
}

#[test]
fn identity_falls_back_to_svn_auth_only_when_config_is_blank() {
    let ran = Cell::new(0);
    let auth = || {
        ran.set(ran.get() + 1);
        Some(SAMPLE_AUTH.to_string())
    };
    let root = "https://svn.corp.example/repos";

    assert_eq!(
        resolve_svn_user(Some("me"), root, auth).as_deref(),
        Some("me")
    );
    assert_eq!(ran.get(), 0, "配置有值时不该跑 svn auth");

    for blank in [None, Some(""), Some("   ")] {
        assert_eq!(
            resolve_svn_user(blank, root, auth).as_deref(),
            Some("alice")
        );
    }
    assert_eq!(ran.get(), 3);

    assert_eq!(resolve_svn_user(None, "file:///C:/repo", auth), None);
    assert_eq!(
        resolve_svn_user(None, "file://server/share/repo", auth),
        None
    );
    assert_eq!(ran.get(), 3, "file:// 没有 realm，不该跑 svn auth");

    assert_eq!(
        resolve_svn_user(None, root, || None),
        None,
        "svn auth 跑不成"
    );
}

/// 造一条假的 svn.simple 缓存。用户名、口令都是虚构的。
fn fake_credential(config_dir: &Path, realm: &str, user: &str) {
    let dir = config_dir.join("auth").join("svn.simple");
    fs::create_dir_all(&dir).unwrap();
    let body = format!(
        "K 8\npasstype\nV 6\nsimple\nK 8\npassword\nV 11\nfakesecret1\n\
         K 15\nsvn:realmstring\nV {}\n{realm}\nK 8\nusername\nV {}\n{user}\nEND\n",
        realm.len(),
        user.len(),
    );
    fs::write(dir.join("0123456789abcdef0123456789abcdef"), body).unwrap();
}

#[test]
fn real_svn_auth_output_parses_under_a_foreign_locale_and_never_shows_passwords() {
    // `svn auth` 的标签走 gettext：LC_ALL=de 时是 `Anmeldebereich:`，中文系统同理（实测）。
    // 不强制 C locale，解析在非英文系统上永远取不到用户名，且没有任何报错。
    let cfg = TempDir::new().unwrap();
    fake_credential(
        cfg.path(),
        "<https://svn.corp.example:443> Corp Realm",
        "alice",
    );
    let cwd = TempDir::new().unwrap();

    std::env::set_var("LC_ALL", "de");
    std::env::set_var("LANGUAGE", "de");
    let text = svn_auth_listing(cwd.path().to_str().unwrap(), Some(cfg.path()));
    std::env::remove_var("LC_ALL");
    std::env::remove_var("LANGUAGE");

    let text = text.expect("svn auth 应当跑得成");
    assert!(!text.contains("fakesecret1"), "绝不能带出口令");
    assert_eq!(
        parse_svn_auth(&text, "https://svn.corp.example/repos"),
        Some("alice".to_string())
    );
}

// ---------------------------------------------------------------------------
// wc.db 戳
// ---------------------------------------------------------------------------

/// 只写配置的「混合仓库」：`wc_db_stamps` 只读配置、枚举 wc，不看基线。
fn configure(fx: &SvnFixture, whitelist: &[&str]) {
    write_config(
        &fx.wc_str(),
        &SvnConfig {
            version: 1,
            mode: "svn-git".to_string(),
            svn_url: "https://svn.corp.example/repos".to_string(),
            whitelist: whitelist.iter().map(|w| w.to_string()).collect(),
            extensions: vec![".cs".to_string()],
            baseline_branch: "svn/base".to_string(),
            respect_gitignore: false,
            disabled_gitignores: Vec::new(),
            svn_user: None,
        },
    )
    .unwrap();
}

fn stamp_ending_with<'a>(stamps: &'a [WcDbStamp], suffix: &str) -> &'a WcDbStamp {
    stamps
        .iter()
        .find(|s| s.wc_root.ends_with(suffix))
        .unwrap_or_else(|| panic!("没有以 {suffix} 结尾的 wc：{stamps:?}"))
}

/// jkk 日常发的只读命令（E19）。
fn read_only_svn(wc: &Path) {
    svn_cmd(wc, &["status", "-v", "--xml", "--non-interactive"]);
    svn_cmd(wc, &["info", "-R", "--xml", "--non-interactive"]);
    svn_cmd(wc, &["propget", "-R", "svn:externals", "--non-interactive"]);
}

/// 外部工具在 `dir` 里提交一个文件。
fn tortoise_commits(dir: &Path, rel: &str, content: &str) {
    fs::write(dir.join(rel), content).unwrap();
    svn_cmd(dir, &["commit", "-m", "external", rel, "--non-interactive"]);
}

#[test]
fn wc_db_stamp_changes_after_external_commit_not_after_status() {
    let fx = hybrid_with_colleague();
    let repo = fx.wc_str();
    let before = wc_db_stamps(&repo).unwrap();
    assert_eq!(before.len(), 1, "实际 {before:?}");
    assert!(before[0].mtime_ns > 0 && before[0].size > 0, "{before:?}");

    read_only_svn(&fx.wc());
    assert_eq!(
        wc_db_stamps(&repo).unwrap(),
        before,
        "只读 svn 命令不写 wc.db（E19）"
    );

    tortoise_commits(&fx.wc(), "Code/A.cs", "committed by tortoise\n");
    assert_ne!(wc_db_stamps(&repo).unwrap(), before, "外部提交必须被看到");
}

#[test]
fn entries_in_one_working_copy_share_one_stamp() {
    let fx = hybrid_with_colleague();
    configure(&fx, &["Code/", "Art/"]);

    let stamps = wc_db_stamps(&fx.wc_str()).unwrap();

    assert_eq!(stamps.len(), 1, "两条白名单在同一个 wc 里，实际 {stamps:?}");
    assert!(
        stamps[0].wc_root.ends_with("/wc1") && !stamps[0].wc_root.contains('\\'),
        "wc_root 是 SVN 工作副本根、正斜杠，实际 {}",
        stamps[0].wc_root
    );
}

#[test]
fn a_nested_external_gets_its_own_stamp() {
    // 只从白名单往上找 `.svn` 的话看不到它：这里的提交只写 external 自己的 wc.db
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    configure(&fx, &["Code/"]);
    let repo = fx.wc_str();

    let before = wc_db_stamps(&repo).unwrap();
    assert_eq!(before.len(), 2, "实际 {before:?}");

    tortoise_commits(&fx.wc().join("Code/Vendor"), "V.cs", "v2\n");

    let after = wc_db_stamps(&repo).unwrap();
    assert_ne!(
        stamp_ending_with(&after, "/Code/Vendor"),
        stamp_ending_with(&before, "/Code/Vendor"),
    );
    assert_eq!(
        stamp_ending_with(&after, "/wc1"),
        stamp_ending_with(&before, "/wc1"),
    );
}

#[test]
fn a_whitelist_inside_an_external_stamps_the_external_not_the_outer_wc() {
    let fx = hybrid_whitelist_in_external();

    let stamps = wc_db_stamps(&fx.wc_str()).unwrap();

    assert_eq!(stamps.len(), 1, "实际 {stamps:?}");
    assert!(stamps[0].wc_root.ends_with("/wc1/code"), "{stamps:?}");
}

#[test]
fn the_expensive_enumeration_only_reruns_when_a_wc_db_changed() {
    let fx = hybrid_with_colleague();
    let repo = fx.wc_str();
    let calls = Cell::new(0);
    let enumerate = |r: &str, w: &[String]| -> Result<Vec<WcIdentity>, GitError> {
        calls.set(calls.get() + 1);
        enumerate_wcs(r, w)
    };

    let first = wc_db_stamps_with(&repo, enumerate).unwrap();
    assert_eq!(calls.get(), 1, "首次要枚举");

    read_only_svn(&fx.wc());
    assert_eq!(wc_db_stamps_with(&repo, enumerate).unwrap(), first);
    assert_eq!(calls.get(), 1, "wc.db 没变，焦点事件不该发 svn 命令");

    tortoise_commits(&fx.wc(), "Code/A.cs", "x\n");
    let second = wc_db_stamps_with(&repo, enumerate).unwrap();
    assert_ne!(second, first);
    assert_eq!(calls.get(), 2, "wc.db 变了要重新枚举");
    wc_db_stamps_with(&repo, enumerate).unwrap();
    assert_eq!(calls.get(), 2, "重新枚举之后又回到快路径");
}

#[test]
fn a_changed_whitelist_is_not_served_from_the_cache() {
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    configure(&fx, &["Code/"]);
    assert_eq!(wc_db_stamps(&fx.wc_str()).unwrap().len(), 2);

    configure(&fx, &["vendor/"]);

    let stamps = wc_db_stamps(&fx.wc_str()).unwrap();
    assert_eq!(stamps.len(), 1, "实际 {stamps:?}");
    assert!(stamps[0].wc_root.ends_with("/wc1"), "{stamps:?}");
}

#[test]
fn an_external_added_later_shows_up_after_the_parent_wc_db_changed() {
    let fx = hybrid_with_colleague();
    let repo = fx.wc_str();
    assert_eq!(wc_db_stamps(&repo).unwrap().len(), 1);

    // 挂 external 必然写父 wc 的 wc.db：缓存据此失效，不用单独盯 external
    let url = run_url(&fx);
    let props = fx.path().join("externals.txt");
    fs::write(&props, format!("{url}/Art Vendor\n")).unwrap();
    svn_cmd(
        &fx.wc(),
        &[
            "propset",
            "svn:externals",
            "-F",
            props.to_str().unwrap(),
            "Code",
            "--non-interactive",
        ],
    );
    svn_cmd(&fx.wc(), &["update", "--non-interactive"]);

    let stamps = wc_db_stamps(&repo).unwrap();
    assert_eq!(stamps.len(), 2, "实际 {stamps:?}");
    stamp_ending_with(&stamps, "/Code/Vendor");
}

fn run_url(fx: &SvnFixture) -> String {
    svn_fixtures::file_url(&fx.path().join("repo"))
}

#[test]
fn a_failed_enumeration_still_answers_from_what_it_knew_and_retries_next_time() {
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    configure(&fx, &["Code/"]);
    let repo = fx.wc_str();
    let before = wc_db_stamps(&repo).unwrap();
    assert_eq!(before.len(), 2);

    let fail = |_: &str, _: &[String]| -> Result<Vec<WcIdentity>, GitError> {
        Err(GitError::new("svn_cli", "svn 正忙"))
    };
    tortoise_commits(&fx.wc().join("Code/Vendor"), "V.cs", "v2\n");

    let during = wc_db_stamps_with(&repo, fail).unwrap();
    assert_eq!(
        during.len(),
        2,
        "枚举失败不能丢掉已知的 external：{during:?}"
    );
    assert_ne!(during, before, "变化照样要报给前端");

    // 失败的那次没有进缓存：下一次必须重新枚举
    let calls = Cell::new(0);
    wc_db_stamps_with(&repo, |r: &str, w: &[String]| {
        calls.set(calls.get() + 1);
        enumerate_wcs(r, w)
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
}

#[test]
fn with_nothing_cached_a_failed_enumeration_walks_up_from_the_whitelist() {
    let fx = hybrid_with_colleague();
    let fail = |_: &str, _: &[String]| -> Result<Vec<WcIdentity>, GitError> {
        Err(GitError::new("svn_cli", "svn 不在 PATH"))
    };

    let stamps = wc_db_stamps_with(&fx.wc_str(), fail).unwrap();

    assert_eq!(stamps.len(), 1, "实际 {stamps:?}");
    assert!(stamps[0].wc_root.ends_with("/wc1"), "{stamps:?}");
}

#[test]
fn an_unreadable_wc_db_is_skipped_not_fatal() {
    let fx = svn_repo_with_same_repo_nested_external(NestedExternal::Clean);
    configure(&fx, &["Code/"]);
    let repo = fx.wc_str();
    assert_eq!(wc_db_stamps(&repo).unwrap().len(), 2);

    let db = fx.wc().join("Code/Vendor/.svn/wc.db");
    fs::rename(&db, db.with_extension("away")).unwrap();
    let fail = |_: &str, _: &[String]| -> Result<Vec<WcIdentity>, GitError> {
        Err(GitError::new("svn_cli", "svn 正忙"))
    };

    let stamps = wc_db_stamps_with(&repo, fail).unwrap();

    assert_eq!(stamps.len(), 1, "实际 {stamps:?}");
    assert!(stamps[0].wc_root.ends_with("/wc1"), "{stamps:?}");
}

// ---------------------------------------------------------------------------
// 命令层
// ---------------------------------------------------------------------------

#[test]
fn the_commands_are_harmless_on_anything_but_a_hybrid_repo() {
    // 焦点事件不分仓库类型：普通 git 仓库、没建过库的 SVN 工作副本都要安静地什么也不做。
    // 后者尤其不能写：`.git` 一出现，§6.1 就把它认成 SvnSuspicious
    let plain = plain_git_repo();
    let plain_path = plain.path().to_string_lossy().to_string();
    svn_follow_mark(plain_path.clone()).unwrap();
    assert_eq!(svn_wc_db_stamps(plain_path).unwrap(), Vec::new());
    assert!(!plain.path().join(".git/mygitclient").exists());

    let svn = svn_repo();
    svn_follow_mark(svn.wc_str()).unwrap();
    assert_eq!(svn_wc_db_stamps(svn.wc_str()).unwrap(), Vec::new());
    assert!(!svn.wc().join(".git").exists());
}

#[test]
fn the_commands_work_on_a_hybrid_repo() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "staged\n");
    git(&fx, &["add", "--", "Code/A.cs"]);

    svn_follow_mark(fx.wc_str()).unwrap();

    assert!(read_mark(&fx.wc_str()).unwrap().is_some());
    assert_eq!(svn_wc_db_stamps(fx.wc_str()).unwrap().len(), 1);
}

// ---------------------------------------------------------------------------
// 「我的 SVN 用户名」的入口（计划 Task 11，日志 D4）
// ---------------------------------------------------------------------------

fn config_file(fx: &SvnFixture) -> std::path::PathBuf {
    fx.wc().join(".git/mygitclient.json")
}

fn config_json(fx: &SvnFixture) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(config_file(fx)).unwrap()).unwrap()
}

#[test]
fn svn_set_user_writes_the_config_and_identifies_me() {
    let fx = hybrid_with_colleague();

    svn_set_user(fx.wc_str(), "  zhangsan ".to_string()).unwrap();

    assert_eq!(config_json(&fx)["svnUser"], "zhangsan");
    assert_eq!(
        my_svn_user(&fx.wc_str(), "svn://corp/repo").as_deref(),
        Some("zhangsan")
    );
}

#[test]
fn svn_set_user_with_blank_clears_the_field() {
    // 清除要真的删掉键：`svnUser: ""` 留在文件里，就成了「手写过」的配置
    for blank in ["", "   "] {
        let fx = hybrid_with_colleague();
        svn_set_user(fx.wc_str(), "zhangsan".to_string()).unwrap();

        svn_set_user(fx.wc_str(), blank.to_string()).unwrap();

        assert!(config_json(&fx).get("svnUser").is_none(), "{blank:?}");
        assert_eq!(read_config(&fx.wc_str()).unwrap().svn_user, None);
    }
}

#[test]
fn svn_set_user_leaves_every_other_field_alone() {
    // 旧版本写下的扩展名过不了 `write_config` 的新扩展名校验，读配置却是放行的（这样的仓库
    // 半能用，见 `validate_new_extensions`）。这里不能让「存个用户名」把它们报成错，也不能
    // 把不认识的字段丢掉：配置是用户手写的
    let fx = hybrid_with_colleague();
    let mut doc = config_json(&fx);
    doc["extensions"] = serde_json::json!([".cs", ".lua、.java"]);
    doc["whitelist"] = serde_json::json!(["Code\\"]);
    doc["someFutureField"] = serde_json::json!({ "keep": true });
    fs::write(config_file(&fx), serde_json::to_string(&doc).unwrap()).unwrap();
    read_config(&fx.wc_str()).expect("前提：这份配置读得过");

    svn_set_user(fx.wc_str(), "zhangsan".to_string()).unwrap();

    let after = config_json(&fx);
    assert_eq!(after["svnUser"], "zhangsan");
    assert_eq!(after["extensions"], doc["extensions"]);
    assert_eq!(
        after["whitelist"], doc["whitelist"],
        "不顺手规范化用户写的内容"
    );
    assert_eq!(after["someFutureField"], doc["someFutureField"]);
}

#[test]
fn svn_set_user_leaves_no_temp_file_behind() {
    let fx = hybrid_with_colleague();
    svn_set_user(fx.wc_str(), "zhangsan".to_string()).unwrap();

    let leftovers: Vec<_> = fs::read_dir(fx.wc().join(".git"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("mygitclient.json") && n != "mygitclient.json")
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn svn_set_user_refuses_a_broken_config_without_touching_it() {
    let fx = hybrid_with_colleague();
    fs::write(config_file(&fx), "{ 不是 json").unwrap();

    let err = svn_set_user(fx.wc_str(), "zhangsan".to_string()).unwrap_err();

    assert_eq!(err.code, "svn_config");
    assert_eq!(fs::read_to_string(config_file(&fx)).unwrap(), "{ 不是 json");
}

#[test]
fn svn_set_user_outside_a_hybrid_repo_creates_nothing() {
    // `.git/mygitclient.json` 一出现，§6.1 就把目录认成混合仓库
    let svn = svn_repo();

    let err = svn_set_user(svn.wc_str(), "zhangsan".to_string()).unwrap_err();

    assert_eq!(err.code, "not_configured");
    assert!(!svn.wc().join(".git").exists());
}

#[test]
fn svn_get_user_keeps_the_configured_name_and_detection_apart() {
    // file:// 没有 realm，探测不到：界面要分得清「配了」和「探测到」，才知道要不要问用户
    let fx = hybrid_with_colleague();
    assert_eq!(
        svn_get_user(fx.wc_str()).unwrap(),
        SvnUserInfo {
            configured: None,
            detected: None
        }
    );

    svn_set_user(fx.wc_str(), "zhangsan".to_string()).unwrap();

    assert_eq!(
        svn_get_user(fx.wc_str()).unwrap(),
        SvnUserInfo {
            configured: Some("zhangsan".to_string()),
            detected: None
        }
    );
}

#[test]
fn svn_get_user_treats_a_blank_config_as_unset() {
    let fx = hybrid_with_colleague();
    let mut cfg = read_config(&fx.wc_str()).unwrap();
    cfg.svn_user = Some("   ".to_string());
    write_config(&fx.wc_str(), &cfg).unwrap();

    assert_eq!(svn_get_user(fx.wc_str()).unwrap().configured, None);
}

#[test]
fn svn_get_user_without_a_baseline_only_loses_the_detection() {
    let fx = hybrid_with_colleague();
    svn_set_user(fx.wc_str(), "zhangsan".to_string()).unwrap();
    git(&fx, &["update-ref", "-d", "refs/heads/svn/base"]);

    let info = svn_get_user(fx.wc_str()).unwrap();

    assert_eq!(info.configured.as_deref(), Some("zhangsan"));
    assert_eq!(info.detected, None);
}

#[test]
fn svn_get_user_outside_a_hybrid_repo_is_an_error() {
    let svn = svn_repo();
    assert_eq!(
        svn_get_user(svn.wc_str()).unwrap_err().code,
        "not_configured"
    );
}
