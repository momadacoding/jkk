//! D4 登记与 §8.5 精确撤销。在 wc1 上直接摆出「D3.8 之后」的磁盘：
//! A.cs 改了、B.cs 被 git 删掉、新文件落在两层新目录里。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_journal::SvnNode;
use app_lib::engine::svn_stage::{snapshot, stage, svn_revert, unstage};
use app_lib::engine::svn_sync_out_check::DeliverChange;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn svn(fx: &SvnFixture, args: &[&str]) -> String {
    let out = Command::new("svn")
        .current_dir(fx.wc())
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "svn {args:?}：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn targets() -> Vec<String> {
    vec!["Code".to_string()]
}

fn change(status: &str, path: &str) -> DeliverChange {
    DeliverChange {
        status: status.into(),
        path: path.into(),
    }
}

/// 摆出 D3.8 之后的磁盘，返回对应的 D1 清单。
fn after_checkout(fx: &SvnFixture) -> Vec<DeliverChange> {
    std::fs::write(fx.wc().join("Code/A.cs"), "a2\n").unwrap();
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    std::fs::create_dir_all(fx.wc().join("Code/Deep/Nested")).unwrap();
    std::fs::write(fx.wc().join("Code/Deep/Nested/New.cs"), "new\n").unwrap();
    vec![
        change("M", "Code/A.cs"),
        change("D", "Code/B.cs"),
        change("A", "Code/Deep/Nested/New.cs"),
    ]
}

fn status_of(nodes: &[SvnNode], path: &str) -> Option<String> {
    nodes
        .iter()
        .find(|n| n.path == path)
        .map(|n| n.status.clone())
}

#[test]
fn staging_registers_adds_and_deletes_and_names_the_new_parent_dirs() {
    let fx = hybrid_with_colleague();
    let changes = after_checkout(&fx);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    assert_eq!(status_of(&before, "Code/B.cs").as_deref(), Some("missing"));
    assert_eq!(
        status_of(&before, "Code/Deep").as_deref(),
        Some("unversioned")
    );

    let rec = stage(&fx.wc_str(), &targets(), &changes, &before).unwrap();
    let after = snapshot(&fx.wc_str(), &targets()).unwrap();
    assert_eq!(status_of(&after, "Code/B.cs").as_deref(), Some("deleted"));
    assert_eq!(status_of(&after, "Code/A.cs").as_deref(), Some("modified"));
    assert_eq!(
        status_of(&after, "Code/Deep/Nested/New.cs").as_deref(),
        Some("added")
    );
    assert_eq!(
        rec.new_parent_dirs,
        vec!["Code/Deep".to_string(), "Code/Deep/Nested".to_string()],
        "父目录必须进 D5 的 target，否则 E200009（spec §8.2.6）"
    );
    assert!(rec
        .delta
        .iter()
        .any(|n| n.path == "Code/Deep" && n.status == "added" && n.dir));
    assert!(
        !rec.delta.iter().any(|n| n.path == "Code/A.cs"),
        "M 不是本流程新增的登记"
    );
}

#[test]
fn unstaging_returns_svn_to_exactly_the_snapshot() {
    // 必测 #29：父目录登记也要撤干净，比 svn status 而不是 git status（§8.5.1）
    let fx = hybrid_with_colleague();
    let changes = after_checkout(&fx);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    stage(&fx.wc_str(), &targets(), &changes, &before).unwrap();

    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(snapshot(&fx.wc_str(), &targets()).unwrap(), before);
    assert!(
        !fx.wc().join("Code/B.cs").exists(),
        "revert 恢复出来的 B.cs 必须删掉（§8.5 表第二行）"
    );
    assert!(
        fx.wc().join("Code/Deep/Nested/New.cs").exists(),
        "A 撤销后文件留在磁盘上"
    );
}

#[test]
fn unstaging_works_when_the_crash_came_before_the_delta_was_written() {
    // 计划 B6：只做了 svn add 就崩了，journal 里只有 svnStatusBefore
    let fx = hybrid_with_colleague();
    after_checkout(&fx);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    svn(&fx, &["add", "--parents", "Code/Deep/Nested/New.cs"]);
    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(snapshot(&fx.wc_str(), &targets()).unwrap(), before);
}

#[test]
fn a_parent_dir_the_user_had_already_added_still_goes_into_the_commit_but_is_not_undone() {
    // 本 Task 的补足：Code/Deep2 被用户自己 svn add 过（连带一个不交付的 Stray.txt）
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join("Code/Deep2/Nested")).unwrap();
    std::fs::write(fx.wc().join("Code/Deep2/Stray.txt"), "stray\n").unwrap();
    svn(&fx, &["add", "--parents", "Code/Deep2/Stray.txt"]);
    std::fs::write(fx.wc().join("Code/Deep2/Nested/New.cs"), "new\n").unwrap();
    let changes = vec![change("A", "Code/Deep2/Nested/New.cs")];
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();

    let rec = stage(&fx.wc_str(), &targets(), &changes, &before).unwrap();
    assert_eq!(
        rec.new_parent_dirs,
        vec!["Code/Deep2".to_string(), "Code/Deep2/Nested".to_string()]
    );

    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    let now = snapshot(&fx.wc_str(), &targets()).unwrap();
    assert_eq!(
        status_of(&now, "Code/Deep2").as_deref(),
        Some("added"),
        "用户自己的登记不碰"
    );
    assert_eq!(
        status_of(&now, "Code/Deep2/Stray.txt").as_deref(),
        Some("added")
    );
    assert_eq!(now, before);
}

#[test]
fn an_empty_change_list_is_refused_before_any_svn_command() {
    // 必测 #1 的 D4 一层（§8.3）
    let fx = hybrid_with_colleague();
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    let err = stage(&fx.wc_str(), &targets(), &[], &before).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets", "{}", err.message);
    assert_eq!(svn(&fx, &["status", "Code"]), "");
}

#[test]
fn an_empty_revert_is_refused() {
    // 必测 #2：空 target 的 revert 会退化成对当前目录操作
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "local\n").unwrap();
    let err = svn_revert(&fx.wc_str(), &[]).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets", "{}", err.message);
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "local\n"
    );
}

#[test]
fn a_path_with_an_at_sign_is_staged_and_unstaged() {
    // svn 把 `@` 后面当 peg revision；不加尾 `@` 时 svn add 会报 E125001
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/a@b.cs"), "x\n").unwrap();
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    stage(
        &fx.wc_str(),
        &targets(),
        &[change("A", "Code/a@b.cs")],
        &before,
    )
    .unwrap();
    assert!(svn(&fx, &["status", "Code"]).contains("a@b.cs"));
    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(snapshot(&fx.wc_str(), &targets()).unwrap(), before);
}

#[test]
fn a_file_the_user_already_added_in_tortoise_is_not_added_twice() {
    // 里程碑 review：用户先在 TortoiseSVN 里 add 了 New.cs，再在 git 里提交它。
    // 再 `svn add` 一次 svn 退出 1（W150002 + E200009），每次交付都会被挡
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join("Code/Fresh")).unwrap();
    std::fs::write(fx.wc().join("Code/Fresh/New.cs"), "new\n").unwrap();
    svn(&fx, &["add", "--parents", "Code/Fresh/New.cs"]);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();

    let rec = stage(
        &fx.wc_str(),
        &targets(),
        &[change("A", "Code/Fresh/New.cs")],
        &before,
    )
    .unwrap();
    assert_eq!(
        rec.new_parent_dirs,
        vec!["Code/Fresh".to_string()],
        "父目录照样进 D5"
    );
    assert!(rec.delta.is_empty(), "没有新增任何登记：{:?}", rec.delta);

    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(
        snapshot(&fx.wc_str(), &targets()).unwrap(),
        before,
        "用户自己的登记不碰"
    );
}
