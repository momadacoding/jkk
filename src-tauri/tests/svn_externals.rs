//! P5：白名单覆盖到的 external 有没有被 `-r` 钉死（spec §6.5.2 手段一）。
//!
//! 命中**不阻断**——它只是把流程切到 pinned 模式（§6.5.3），所以这里的断言全是
//! 「报出了哪几条」，没有一条断言 `Err`。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_externals::{detect_pinned, parse_external_line};
use svn_fixtures::{svn_repo, svn_repo_with_five_external_forms};

// ---------------------------------------------------------------------------
// 纯函数：一行定义怎么读
// ---------------------------------------------------------------------------

#[test]
fn all_five_pinning_forms_are_detected() {
    // 2026-09-14 实测：这五种写法签出后 follow 停在 HEAD(r2)，其余四种全在 r1。
    //
    // ⚠️ 新格式是 `[-r N] URL LOCAL`、旧格式是 `LOCAL [-r N] URL`，**字段顺序相反**，
    // 按列取字段的实现会漏掉最后两种。
    for (line, want) in [
        ("^/lib follow", None),
        ("-r1 ^/lib pinA", Some(1)),
        ("-r 1 ^/lib pinB", Some(1)), // -r 与数字之间有空格
        ("^/lib@1 pinC", Some(1)),    // peg 语法，**整行不含 -r**
        ("pinD -r1 ^/lib", Some(1)),  // 旧格式，本地路径在前
    ] {
        let def = parse_external_line(line).unwrap_or_else(|| panic!("{line:?} 没解析出来"));
        assert_eq!(def.pinned_rev, want, "line={line:?}");
    }
}

#[test]
fn the_local_path_is_found_regardless_of_which_side_it_is_on() {
    // 钉没钉住之外，pinned 模式还要知道**是哪个目录**——两步 update 时要显式
    // 指名它（§6.5.3 第 1 条）。取错了列就会去 update 一个 URL。
    assert_eq!(
        parse_external_line("-r1 ^/lib pinA").unwrap().local_path,
        "pinA"
    );
    assert_eq!(
        parse_external_line("pinD -r1 ^/lib").unwrap().local_path,
        "pinD"
    );
    assert_eq!(
        parse_external_line("^/lib@1 pinC").unwrap().local_path,
        "pinC"
    );
}

#[test]
fn the_peg_suffix_is_stripped_from_the_url() {
    // `@1` 属于 peg 记号，不是 URL 的一部分。留在 URL 里的话，
    // 任何拿它去拼命令的地方都会带上一个假的路径段。
    let def = parse_external_line("^/lib@1 pinC").unwrap();
    assert_eq!(def.url, "^/lib");
    // 只有「最后一个 @ 之后全是数字」才算 peg；一个含 @ 的普通路径不该被误判
    let def = parse_external_line("^/team@home/lib pinE").unwrap();
    assert_eq!(def.pinned_rev, None, "@home 不是 peg");
    assert_eq!(def.url, "^/team@home/lib");
}

#[test]
fn an_operative_revision_wins_over_the_peg_when_both_are_present() {
    // `-r` 是 operative revision，`@` 是 peg；内容来自前者。
    let def = parse_external_line("-r2 ^/lib@1 pinF").unwrap();
    assert_eq!(def.pinned_rev, Some(2));
}

#[test]
fn malformed_external_lines_do_not_panic() {
    // 实测发现：旧格式不带 -r（`lib ^/lib`）时签出直接失败——那种定义本来就
    // 没法用，但解析器不能因此 panic（spec §6.5.2）。
    for line in [
        "",
        "   ",
        "#comment",
        "lib ^/lib",
        "-r ^/lib x",
        "-rABC ^/lib x",
    ] {
        let _ = parse_external_line(line); // 不 panic 即可
    }
    // 顺带把「解析得出来但没钉住」和「根本解析不出来」分开钉一下
    assert_eq!(parse_external_line("lib ^/lib").unwrap().pinned_rev, None);
    assert!(
        parse_external_line("-r ^/lib x").is_none(),
        "-r 后面不是数字"
    );
    assert!(parse_external_line("-rABC ^/lib x").is_none());
    assert!(parse_external_line("").is_none());
}

// ---------------------------------------------------------------------------
// 端到端：真实 svn propset 出来的五行定义
// ---------------------------------------------------------------------------

#[test]
fn detect_pinned_reports_the_four_pinned_ones_and_not_the_following_one() {
    // 这条同时是 E10 那个坑的守卫：propset -F 遇到混合行尾会静默失败，
    // 属性根本没设上。那种情况下 detect_pinned 返回空列表，这里就会红。
    let fx = svn_repo_with_five_external_forms();
    let got = detect_pinned(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    let paths: Vec<&str> = got.iter().map(|p| p.local_path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["Code/pinA", "Code/pinB", "Code/pinC", "Code/pinD"],
        "实际：{got:?}"
    );
    assert!(
        !paths.contains(&"Code/follow"),
        "跟随 HEAD 的那条不该被报成钉死"
    );
    assert!(got.iter().all(|p| p.pinned_rev == 1));
    assert!(got.iter().all(|p| p.defined_in == "Code"));
}

#[test]
fn a_whitelist_inside_a_pinned_external_is_reported() {
    // 白名单直接落在被钉死的 external 里——这才是 spec §6.5 P5 的主场景
    // 「白名单**所在**的 external 被钉死」。
    let fx = svn_repo_with_five_external_forms();
    let got = detect_pinned(&fx.wc_str(), &["Code/pinA".to_string()]).unwrap();
    assert_eq!(got.len(), 1, "实际：{got:?}");
    assert_eq!(got[0].local_path, "Code/pinA");
}

#[test]
fn externals_the_whitelist_does_not_touch_are_not_reported() {
    // 仓库里别处钉死的 external 与本次建库无关，报出来只会制造噪音，
    // 还会让用户以为自己必须去改一个跟他无关的 svn:externals 定义。
    let fx = svn_repo_with_five_external_forms();
    let got = detect_pinned(&fx.wc_str(), &["lib".to_string()]).unwrap();
    assert!(got.is_empty(), "实际：{got:?}");
}

#[test]
fn a_repository_without_externals_yields_an_empty_list_not_an_error() {
    // P5 命中不阻断，没命中更不该报错。
    let fx = svn_repo();
    let got = detect_pinned(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert!(got.is_empty(), "实际：{got:?}");
}

#[test]
fn an_empty_whitelist_sends_no_svn_command_and_returns_empty() {
    // 与 enumerate_wcs 对齐（spec §12.2）。
    let fx = svn_repo_with_five_external_forms();
    let got = detect_pinned(&fx.wc_str(), &[]).unwrap();
    assert!(got.is_empty());
}
