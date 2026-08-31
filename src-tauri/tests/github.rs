//! `gh pr list --json` 输出的解析，以及 gh 不可用时的降级行为。
//!
//! 这里**不真跑网络**：断言的是解析层和失败路径，样例 JSON 取自
//! `gh pr list --state <open|merged> --json number,headRefName,headRefOid,title,isDraft,state,url`
//! 的真实输出形状。

use app_lib::engine::github::{self, PullRequest};
use tempfile::TempDir;

#[test]
fn parses_real_gh_output() {
    let json = r#"[
      {"headRefName":"fix/watch","headRefOid":"ac6e785eb547aadf74ff83993fe44b231aaf9b8a","isDraft":false,"number":36,"state":"OPEN","title":"linked worktree 下外部 commit 不触发刷新","url":"https://github.com/o/r/pull/36"},
      {"headRefName":"feat/tab","headRefOid":"6e6f014513cd3cc520cfa3838228517af47ad3ef","isDraft":true,"number":35,"state":"OPEN","title":"中键点击标签即关闭","url":"https://github.com/o/r/pull/35"}
    ]"#;

    let prs = github::parse_pr_list(json).expect("真实形状的输出必须能解析");
    assert_eq!(prs.len(), 2);
    assert_eq!(
        prs[0],
        PullRequest {
            number: 36,
            head_ref_name: "fix/watch".to_string(),
            head_ref_oid: "ac6e785eb547aadf74ff83993fe44b231aaf9b8a".to_string(),
            title: "linked worktree 下外部 commit 不触发刷新".to_string(),
            is_draft: false,
            state: "OPEN".to_string(),
            url: "https://github.com/o/r/pull/36".to_string(),
        }
    );
    assert!(prs[1].is_draft, "isDraft 必须映射到 is_draft");
}

#[test]
fn missing_head_ref_oid_defaults_to_empty_not_a_parse_error() {
    // 老版本 gh 可能不给这个字段。缺了就退回普通 merged 徽章，
    // 不能让整条通道因为一个可选字段而哑掉。
    let json = r#"[{"headRefName":"a","isDraft":false,"number":1,"state":"MERGED","title":"t","url":"u"}]"#;
    let prs = github::parse_pr_list(json).expect("缺 headRefOid 不该是解析错误");
    assert_eq!(prs[0].head_ref_oid, "");
}

#[test]
fn parses_merged_state() {
    // 已合并的 PR 是这个功能的第二个主场景：分支还在，内容早进主干了
    let json = r#"[{"headRefName":"feat/portable-zip-signing","headRefOid":"ac6e785eb547aadf74ff83993fe44b231aaf9b8a","isDraft":false,"number":33,"state":"MERGED","title":"绿色版 zip","url":"u"}]"#;
    let prs = github::parse_pr_list(json).unwrap();
    assert_eq!(prs[0].state, "MERGED");
    assert_eq!(
        prs[0].head_ref_oid,
        "ac6e785eb547aadf74ff83993fe44b231aaf9b8a"
    );
}

#[test]
fn parses_empty_list() {
    // 没有 open PR 时 gh 打印 `[]`，这不是错误
    assert!(github::parse_pr_list("[]").unwrap().is_empty());
}

#[test]
fn blank_stdout_is_an_empty_list_not_an_error() {
    // gh 带 --json 时总会打印 `[]`，但空 stdout 更像「没有 PR」而不是「解析崩了」，
    // 而这条信息只是锦上添花，不值得把错误横幅让给它
    assert!(github::parse_pr_list("").unwrap().is_empty());
    assert!(github::parse_pr_list("   \n").unwrap().is_empty());
}

#[test]
fn malformed_json_is_an_error() {
    // gh 成功退出却吐出解析不了的东西，说明我们对它的输出契约理解错了——
    // 这个要报出来，不能悄悄当成「没有 PR」
    let err = github::parse_pr_list("{oops").expect_err("畸形 JSON 必须报错");
    assert_eq!(err.code, "gh_json");
}

#[test]
fn unicode_branch_names_survive() {
    let json = r#"[{"headRefName":"feat/中文-分支","isDraft":false,"number":1,"state":"OPEN","title":"标题","url":"u"}]"#;
    let prs = github::parse_pr_list(json).unwrap();
    assert_eq!(prs[0].head_ref_name, "feat/中文-分支");
}

#[test]
fn unknown_fields_are_ignored() {
    // gh 升级后可能多吐字段；多一个字段不该让整条通道失效
    let json = r#"[{"headRefName":"a","isDraft":false,"number":1,"state":"OPEN","title":"t","url":"u","baseRefName":"master"}]"#;
    assert_eq!(github::parse_pr_list(json).unwrap().len(), 1);
}

#[test]
fn plain_git_repo_without_github_remote_yields_no_prs() {
    // 非 GitHub 仓库（这里干脆连远端都没有）上，gh 会非零退出。
    // 必须降级成空列表：分支上不显示徽章，但不能变成一条错误横幅，
    // 那会盖住真正的 git 错误。
    let dir = TempDir::new().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    std::process::Command::new("git")
        .current_dir(dir.path())
        .args(["init"])
        .status()
        .expect("git not found");

    let prs = github::list_branch_prs(&path).expect("gh 不可用时不该返回 Err");
    assert!(prs.is_empty());
}

#[test]
fn nonexistent_path_yields_no_prs() {
    // 仓库目录被删掉/盘符掉线时，子进程连 current_dir 都设不上，
    // 这条 io 错误同样只能降级，不能冒泡到界面
    let prs = github::list_branch_prs("Z:/definitely/not/here").expect("路径不存在时不该返回 Err");
    assert!(prs.is_empty());
}
