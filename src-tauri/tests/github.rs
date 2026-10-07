//! `gh pr list --json` 输出的解析，以及 gh 不可用时的降级行为。
//!
//! 这里**不真跑网络**：断言的是解析层和失败路径，样例 JSON 取自
//! `gh pr list --state <open|merged> --json number,headRefName,headRefOid,title,isDraft,state,url`
//! 的真实输出形状。

use app_lib::engine::github::{self, LocalRelation, PullRequest};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn init_repo(dir: &Path) {
    run_git(dir, &["init", "-b", "master"]);
    run_git(dir, &["config", "user.email", "test@test.com"]);
    run_git(dir, &["config", "user.name", "Test"]);
}

/// 提交一个新文件，返回新提交的 oid。
fn commit(dir: &Path, name: &str) -> String {
    fs::write(dir.join(format!("{name}.txt")), name).unwrap();
    run_git(dir, &["add", "."]);
    run_git(dir, &["commit", "-m", name]);
    git_out(dir, &["rev-parse", "HEAD"])
}

fn merged_pr(branch: &str, head_ref_oid: &str) -> PullRequest {
    PullRequest {
        number: 321,
        head_ref_name: branch.to_string(),
        head_ref_oid: head_ref_oid.to_string(),
        title: "t".to_string(),
        is_draft: false,
        state: "MERGED".to_string(),
        url: "u".to_string(),
        local_relation: None,
    }
}

fn path_str(dir: &TempDir) -> String {
    dir.path().to_string_lossy().to_string()
}

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
            // gh 不给这个字段，它由 list_branch_prs 在本地补算
            local_relation: None,
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

// ---- 本地分支相对 PR 合并点的位置（annotate_local_relations）----
//
// 已合并 PR 的徽章要区分「本地领先 / 落后 / 分叉」，只比 SHA 是否相等的话
// 落后会被画成领先，正好反了。以下 fixture 全是自建临时仓库，判定以 git 的祖先关系为准。

/// 在 master 上开出 `feat`，返回（仓库，`feat` 起点的 oid）。
fn repo_with_feat_branch() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    init_repo(dir.path());
    commit(dir.path(), "c1");
    run_git(dir.path(), &["checkout", "-b", "feat"]);
    let base = commit(dir.path(), "base");
    (dir, base)
}

/// 造一个以 `parent` 为父、没有任何 ref 指着的提交——即「对象在本地，但谁都够不着」。
fn unreachable_child(dir: &Path, parent: &str) -> String {
    let tree = git_out(dir, &["rev-parse", &format!("{parent}^{{tree}}")]);
    git_out(
        dir,
        &["commit-tree", &tree, "-p", parent, "-m", "pushed elsewhere"],
    )
}

#[test]
fn behind_when_merged_head_was_pushed_elsewhere_and_remote_branch_pruned() {
    // 真实案例：本地分支停在 736890d；另一个工作副本在它上面又提交了 93e3d22 并推上去，
    // PR 按 93e3d22 合并后远端分支被删，本地 fetch --prune。于是 93e3d22 的对象在本地、
    // 却没有任何 ref 指着它。旧徽章把这种情况画成 ↑（本地领先），实际是本地落后。
    let remote = TempDir::new().unwrap();
    run_git(remote.path(), &["init", "--bare", "-b", "master"]);
    let remote_url = path_str(&remote);

    let local = TempDir::new().unwrap();
    init_repo(local.path());
    commit(local.path(), "c1");
    run_git(local.path(), &["remote", "add", "origin", &remote_url]);
    run_git(local.path(), &["push", "-u", "origin", "master"]);
    run_git(local.path(), &["checkout", "-b", "feat"]);
    let base = commit(local.path(), "base");
    run_git(local.path(), &["push", "-u", "origin", "feat"]);

    let other = TempDir::new().unwrap();
    run_git(other.path(), &["clone", &remote_url, "."]);
    run_git(other.path(), &["config", "user.email", "test@test.com"]);
    run_git(other.path(), &["config", "user.name", "Test"]);
    run_git(other.path(), &["checkout", "feat"]);
    let head = commit(other.path(), "elsewhere");
    run_git(other.path(), &["push", "origin", "feat"]);

    // 先 fetch 一次把对象拉下来，再在远端删分支、本地 prune——和真实经过同序
    run_git(local.path(), &["fetch", "origin"]);
    run_git(other.path(), &["push", "origin", "--delete", "feat"]);
    run_git(local.path(), &["fetch", "--prune", "origin"]);

    // fixture 自检：形态必须和真实案例一致，否则这条测试证明不了什么
    assert_eq!(git_out(local.path(), &["rev-parse", "feat"]), base);
    run_git(local.path(), &["cat-file", "-e", &head]);
    assert_eq!(
        git_out(local.path(), &["for-each-ref", "--contains", &head]),
        "",
        "合并点不该被任何 ref 指着"
    );
    assert!(git_out(local.path(), &["branch", "-vv"]).contains(": gone]"));

    let mut prs = vec![merged_pr("feat", &head)];
    github::annotate_local_relations(&path_str(&local), &mut prs);
    assert_eq!(
        prs[0].local_relation,
        Some(LocalRelation {
            tip: base,
            ahead: 0,
            behind: 1
        })
    );
}

#[test]
fn equal_when_branch_sits_on_merged_head() {
    let (dir, base) = repo_with_feat_branch();
    let mut prs = vec![merged_pr("feat", &base)];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(
        prs[0].local_relation,
        Some(LocalRelation {
            tip: base,
            ahead: 0,
            behind: 0
        })
    );
}

#[test]
fn ahead_when_branch_grew_after_merge() {
    // 合并后本地又提交：这是会丢东西的那种，必须报 ahead
    let (dir, base) = repo_with_feat_branch();
    let tip = commit(dir.path(), "after-merge");
    let mut prs = vec![merged_pr("feat", &base)];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(
        prs[0].local_relation,
        Some(LocalRelation {
            tip,
            ahead: 1,
            behind: 0
        })
    );
}

#[test]
fn diverged_when_both_sides_have_their_own_commits() {
    let (dir, base) = repo_with_feat_branch();
    let head = unreachable_child(dir.path(), &base);
    let tip = commit(dir.path(), "local-only");
    let mut prs = vec![merged_pr("feat", &head)];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(
        prs[0].local_relation,
        Some(LocalRelation {
            tip,
            ahead: 1,
            behind: 1
        })
    );
}

#[test]
fn no_relation_when_merged_head_object_is_not_local() {
    // 从没 fetch 过合并点：祖先关系无从判断，不能瞎猜成任何一种
    let (dir, _) = repo_with_feat_branch();
    let mut prs = vec![merged_pr("feat", &"1".repeat(40))];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(prs[0].local_relation, None);
}

#[test]
fn no_relation_without_matching_local_branch() {
    let (dir, base) = repo_with_feat_branch();
    let mut prs = vec![merged_pr("no-such-branch", &base)];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(prs[0].local_relation, None);
}

#[test]
fn no_relation_for_empty_or_malformed_head_ref_oid() {
    // 老版本 gh 不给 headRefOid 时是空串；空输入不能被当成任何 oid
    let (dir, _) = repo_with_feat_branch();
    let mut prs = vec![merged_pr("feat", ""), merged_pr("feat", "not-an-oid")];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(prs[0].local_relation, None);
    assert_eq!(prs[1].local_relation, None);
}

#[test]
fn open_prs_are_not_annotated() {
    // 只有已合并的 PR 用得到这个判断；open PR 的领先落后由分支行的 ↑↓ 表达
    let (dir, base) = repo_with_feat_branch();
    let mut pr = merged_pr("feat", &base);
    pr.state = "OPEN".to_string();
    let mut prs = vec![pr];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(prs[0].local_relation, None);
}

#[test]
fn annotate_on_non_repo_path_leaves_prs_untouched() {
    // 仓库打不开不是错误：徽章退回「无法判断」，不能让整条 PR 通道失败
    let dir = TempDir::new().unwrap();
    let mut prs = vec![merged_pr("feat", &"1".repeat(40))];
    github::annotate_local_relations(&path_str(&dir), &mut prs);
    assert_eq!(prs[0].local_relation, None);
}

#[test]
fn local_relation_serializes_in_camel_case() {
    // 前端按 `localRelation.{tip,ahead,behind}` 取值，这层转换没有编译期检查
    let mut pr = merged_pr("feat", "a");
    pr.local_relation = Some(LocalRelation {
        tip: "b".to_string(),
        ahead: 2,
        behind: 3,
    });
    let v = serde_json::to_value(&pr).unwrap();
    assert_eq!(
        v["localRelation"],
        serde_json::json!({ "tip": "b", "ahead": 2, "behind": 3 })
    );
    let none = serde_json::to_value(merged_pr("feat", "a")).unwrap();
    assert!(none["localRelation"].is_null());
}
