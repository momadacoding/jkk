//! TortoiseSVN 钩子注册表条目的解析、注册计划与健康判定（计划 Task 1；日志 H1、H3）。
//!
//! 全是纯字符串，不读写注册表。

use app_lib::engine::tsvn_hooks::{
    cas_write, hook_command, is_ours, judge_health, parse_hooks, plan_registration,
    registered_value, serialize_hooks, try_unregistered_value, unregistered_value, without_ours,
    HookEntry, HookHealth,
};
use std::cell::RefCell;
use std::path::Path;

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

const EXE: &str = r"C:\Tools\jkk\app.exe";

// —— CAS（计划 Review Focus 第 2 条）：内存里的值代替注册表，测试不碰 HKCU ——

/// 跑一次 `cas_write`。`overwritten_by` 模拟写入之后立刻被别的程序覆盖。返回结果、写入次数和
/// 最终的值。
fn run_cas(
    stored: Option<&str>,
    expected: Option<&str>,
    new: Option<&str>,
    overwritten_by: Option<&str>,
) -> (Result<(), String>, usize, Option<String>) {
    let cell = RefCell::new(stored.map(str::to_string));
    let writes = RefCell::new(0);
    let r = cas_write(
        expected,
        new,
        || Ok(cell.borrow().clone()),
        |v| {
            *writes.borrow_mut() += 1;
            *cell.borrow_mut() = overwritten_by.or(v).map(str::to_string);
            Ok(())
        },
    );
    (
        r.map_err(|e| e.code),
        writes.into_inner(),
        cell.into_inner(),
    )
}

#[test]
fn cas_writes_when_the_value_is_unchanged() {
    let (r, writes, value) = run_cas(Some("old"), Some("old"), Some("new"), None);
    assert_eq!(r, Ok(()));
    assert_eq!(writes, 1);
    assert_eq!(value.as_deref(), Some("new"));
}

#[test]
fn cas_deletes_the_value_when_new_is_none() {
    let (r, writes, value) = run_cas(Some("old"), Some("old"), None, None);
    assert_eq!(r, Ok(()));
    assert_eq!(writes, 1);
    assert_eq!(value, None);
}

#[test]
fn cas_refuses_to_write_when_the_value_changed_since_it_was_read() {
    let (r, writes, value) = run_cas(Some("theirs"), Some("old"), Some("new"), None);
    assert_eq!(r, Err("tsvn_hooks_registry_changed".to_string()));
    assert_eq!(writes, 0, "别人的改动不能被覆盖");
    assert_eq!(value.as_deref(), Some("theirs"));
}

#[test]
fn cas_reports_a_value_overwritten_right_after_the_write() {
    let (r, _, _) = run_cas(Some("old"), Some("old"), Some("new"), Some("theirs"));
    assert_eq!(r, Err("tsvn_hooks_registry_changed".to_string()));
}

// —— 命令层的守卫（Task 6）：在读写注册表之前就返回，测试不碰注册表 ——

#[test]
fn hook_commands_refuse_a_repo_that_is_not_hybrid() {
    use app_lib::commands::svn::{tsvn_hooks_register, tsvn_hooks_status, tsvn_hooks_unregister};
    let dir = tempfile::TempDir::new().unwrap();
    let p = dir.path().display().to_string();
    assert_eq!(
        tsvn_hooks_status(p.clone()).unwrap_err().code,
        "svn_not_hybrid"
    );
    assert_eq!(
        tsvn_hooks_register(p.clone(), true).unwrap_err().code,
        "svn_not_hybrid"
    );
    assert_eq!(tsvn_hooks_unregister(p).unwrap_err().code, "svn_not_hybrid");
}

/// linked worktree 的形状（`.git` 是指向别处的文件）：钩子永远写不进事件，必须在碰注册表之前
/// 挡住。今天是 `detect_repo_kind` 先挡（报 `svn_not_hybrid`），`hooks_target` 里的
/// `tsvn_hooks_no_git_dir` 是纵深防御；两者任一挡住都算对。
#[test]
fn hook_commands_refuse_a_linked_worktree_layout() {
    use app_lib::commands::svn::{tsvn_hooks_register, tsvn_hooks_status, tsvn_hooks_unregister};
    let fx = svn_fixtures::hybrid_with_colleague();
    let git_dir = fx.wc().join(".git");
    let moved = fx.path().join("gitdir-moved");
    std::fs::rename(&git_dir, &moved).unwrap();
    std::fs::write(&git_dir, format!("gitdir: {}", moved.display())).unwrap();
    let p = fx.wc_str();
    for code in [
        tsvn_hooks_status(p.clone()).unwrap_err().code,
        tsvn_hooks_register(p.clone(), true).unwrap_err().code,
        tsvn_hooks_unregister(p).unwrap_err().code,
    ] {
        assert!(
            code == "tsvn_hooks_no_git_dir" || code == "svn_not_hybrid",
            "linked worktree 形态必须在碰注册表之前就被挡住：{code}"
        );
    }
}

#[test]
fn status_serializes_in_camel_case() {
    use app_lib::commands::svn::TsvnHooksStatus;
    let e = HookEntry {
        kind: "post_commit".into(),
        wc_path: r"D:\w".into(),
        command: "c".into(),
        wait: "true".into(),
        show: "hide".into(),
        enforce: "ask".into(),
    };
    let v = serde_json::to_value(TsvnHooksStatus {
        supported: true,
        health: HookHealth::Healthy { block_leaks: false },
        planned: vec![e.clone()],
        conflicts: vec![e],
        exe: r"C:\jkk.exe".into(),
    })
    .unwrap();
    assert_eq!(
        v,
        serde_json::json!({
            "supported": true,
            "health": { "state": "healthy", "blockLeaks": false },
            "planned": [{ "kind": "post_commit", "wcPath": "D:\\w", "command": "c",
                          "wait": "true", "show": "hide", "enforce": "ask" }],
            "conflicts": [{ "kind": "post_commit", "wcPath": "D:\\w", "command": "c",
                            "wait": "true", "show": "hide", "enforce": "ask" }],
            "exe": "C:\\jkk.exe",
        })
    );
}
const REPO: &str = r"D:\work\game";
const WC: &str = r"D:\work\game";

/// H1：用户在 TortoiseSVN 设置界面里添加的 Pre-commit，注册表里的原文。
const UI_WRITTEN: &str = "pre_commit_hook\nC:\\fx\\wc\npwsh -NoProfile -File C:\\fx\\probe.ps1 pre_commit\ntrue\nhide\nask\n";

/// 别人的两条钩子，手写的原文（不经 `serialize_hooks` 生成，否则序列化的毛病两边对称、测不出来）。
const FOREIGN_RAW: &str = "manual_precommit\nC:\\corp\nC:\\corp\\check.exe\ntrue\nshow\nask\npost_update_hook\nE:\\x y\\中文\nC:\\corp\\hook.exe up\nfalse\nhide\nenforce\n";

#[test]
fn foreign_raw_value_round_trips_byte_for_byte_including_manual_precommit() {
    assert_eq!(serialize_hooks(&parse_hooks(FOREIGN_RAW)), FOREIGN_RAW);
    let after =
        registered_value(FOREIGN_RAW, &[WC.to_string()], Path::new(EXE), REPO, true).unwrap();
    assert!(
        after.starts_with(FOREIGN_RAW),
        "别人的条目必须逐字节在前：{after:?}"
    );
    assert_eq!(
        unregistered_value(&after, REPO).as_deref(),
        Some(FOREIGN_RAW)
    );
}

#[test]
fn value_that_does_not_round_trip_is_refused_not_rewritten() {
    // CRLF 写法、凑不满 6 行的残条、旧格式只有 5 行、第 4–6 行取值不认识：都不碰
    let bad = [
        UI_WRITTEN.replace('\n', "\r\n"),
        format!("{UI_WRITTEN}post_commit_hook\nC:\\x\n"),
        "pre_commit_hook\nC:\\fx\\wc\ncmd\ntrue\nhide\n".to_string(),
        UI_WRITTEN.replace("\ntrue\n", "\nyes\n"),
    ];
    for v in &bad {
        let err = registered_value(v, &[WC.to_string()], Path::new(EXE), REPO, true).unwrap_err();
        assert_eq!(err.code, "tsvn_hooks_unrecognized", "{v:?}");
        let err = try_unregistered_value(v, REPO).unwrap_err();
        assert_eq!(err.code, "tsvn_hooks_unrecognized", "{v:?}");
    }
    // 空值等于没有钩子，照常注册
    assert!(registered_value("", &[WC.to_string()], Path::new(EXE), REPO, true).is_ok());
}

#[test]
fn hook_command_trims_trailing_separator_of_a_drive_root_repo() {
    // `--repo "D:\"` 会让 `\"` 变成字面引号，TortoiseSVN 追加的参数全部并进 repo（review 第 3 条）
    assert_eq!(
        hook_command(Path::new(r"C:\jkk\app.exe"), "post_commit", r"D:\"),
        r#""C:\jkk\app.exe" --tsvn-hook post_commit --repo "D:""#
    );
    assert_eq!(
        hook_command(Path::new(r"C:\jkk\app.exe"), "post_commit", r"D:\work\"),
        r#""C:\jkk\app.exe" --tsvn-hook post_commit --repo "D:\work""#
    );
}

fn foreign(kind: &str, path: &str) -> HookEntry {
    HookEntry {
        kind: kind.to_string(),
        wc_path: path.to_string(),
        command: format!(r"C:\corp\hook.exe {kind}"),
        wait: "true".to_string(),
        show: "hide".to_string(),
        enforce: "ask".to_string(),
    }
}

fn ours(kind: &str, path: &str, exe: &str, repo: &str) -> HookEntry {
    HookEntry {
        kind: kind.to_string(),
        wc_path: path.to_string(),
        command: hook_command(Path::new(exe), kind, repo),
        wait: "true".to_string(),
        show: "hide".to_string(),
        enforce: "ask".to_string(),
    }
}

fn kinds(entries: &[HookEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.kind.as_str()).collect()
}

#[test]
fn parses_the_value_tortoisesvn_wrote() {
    let entries = parse_hooks(UI_WRITTEN);
    assert_eq!(entries.len(), 1);
    let e = &entries[0];
    assert_eq!(e.kind, "pre_commit");
    assert_eq!(e.wc_path, r"C:\fx\wc");
    assert_eq!(
        e.command,
        r"pwsh -NoProfile -File C:\fx\probe.ps1 pre_commit"
    );
    assert_eq!((e.wait.as_str(), e.show.as_str()), ("true", "hide"));
    assert_eq!(e.enforce, "ask");
    assert!(!is_ours(e));
    assert_eq!(serialize_hooks(&entries), UI_WRITTEN);
}

#[test]
fn trailing_partial_entry_is_ignored() {
    let value = format!("{UI_WRITTEN}post_commit_hook\nC:\\x\n");
    assert_eq!(parse_hooks(&value).len(), 1);
    assert!(parse_hooks("").is_empty());
}

#[test]
fn foreign_entries_survive_a_round_trip() {
    let a = foreign("pre_commit", r"C:\other");
    let mine = ours("post_commit", WC, EXE, REPO);
    let b = foreign("post_update", r"E:\x y\中文");
    let value = serialize_hooks(&[a.clone(), mine.clone(), b.clone()]);
    let kept = without_ours(&parse_hooks(&value), REPO);
    assert_eq!(serialize_hooks(&kept), serialize_hooks(&[a, b]));
    assert!(is_ours(&mine));
}

#[test]
fn without_ours_keeps_entries_of_another_repo() {
    let other = ours("post_commit", r"E:\other", EXE, r"E:\other");
    let mine = ours("post_commit", WC, EXE, REPO);
    let kept = without_ours(&[other.clone(), mine], REPO);
    assert_eq!(kept, vec![other]);
}

#[test]
fn plan_adds_four_kinds_per_root_when_blocking_leaks() {
    let roots = vec![WC.to_string(), r"D:\work\ext".to_string()];
    let plan = plan_registration(&[], &roots, Path::new(EXE), REPO, true);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.add.len(), 8);
    for root in &roots {
        let mine: Vec<&HookEntry> = plan.add.iter().filter(|e| &e.wc_path == root).collect();
        let mut k: Vec<&str> = mine.iter().map(|e| e.kind.as_str()).collect();
        k.sort();
        assert_eq!(
            k,
            vec!["check_commit", "post_commit", "post_update", "pre_commit"]
        );
        for e in mine {
            assert_eq!(e.command, hook_command(Path::new(EXE), &e.kind, REPO));
            assert_eq!(
                (e.wait.as_str(), e.show.as_str(), e.enforce.as_str()),
                ("true", "hide", "ask")
            );
        }
    }
}

#[test]
fn plan_skips_check_commit_without_blocking() {
    let plan = plan_registration(&[], &[WC.to_string()], Path::new(EXE), REPO, false);
    let mut k = kinds(&plan.add);
    k.sort();
    assert_eq!(k, vec!["post_commit", "post_update", "pre_commit"]);
}

#[test]
fn plan_reports_foreign_hook_of_the_same_kind_on_the_same_or_ancestor_path_as_conflict() {
    let roots = vec![WC.to_string()];
    // 父目录、不同大小写与分隔符
    let ancestor = foreign("post_commit", "d:/WORK");
    let plan = plan_registration(
        std::slice::from_ref(&ancestor),
        &roots,
        Path::new(EXE),
        REPO,
        true,
    );
    assert_eq!(plan.conflicts, vec![ancestor]);

    // 子目录同样算：TortoiseSVN 选哪条没测过
    let child = foreign("pre_commit", r"D:\work\game\Code");
    let plan = plan_registration(
        std::slice::from_ref(&child),
        &roots,
        Path::new(EXE),
        REPO,
        true,
    );
    assert_eq!(plan.conflicts, vec![child]);

    // 不相干的路径、或同路径不同类型：不冲突
    let unrelated = foreign("post_commit", r"D:\work\game2");
    let other_kind = foreign("start_commit", WC);
    let plan = plan_registration(&[unrelated, other_kind], &roots, Path::new(EXE), REPO, true);
    assert!(plan.conflicts.is_empty());
    assert_eq!(plan.add.len(), 4);
}

#[test]
fn plan_is_empty_when_already_registered_for_this_exe_and_repo() {
    let existing: Vec<HookEntry> = ["check_commit", "pre_commit", "post_commit", "post_update"]
        .iter()
        .map(|k| ours(k, WC, EXE, REPO))
        .collect();
    let plan = plan_registration(&existing, &[WC.to_string()], Path::new(EXE), REPO, true);
    assert!(plan.add.is_empty());
    assert!(plan.conflicts.is_empty());
}

#[test]
fn health_is_healthy_with_the_three_required_kinds() {
    let mut existing: Vec<HookEntry> = ["pre_commit", "post_commit", "post_update"]
        .iter()
        .map(|k| ours(k, WC, EXE, REPO))
        .collect();
    let roots = vec![WC.to_string()];
    assert_eq!(
        judge_health(&existing, &roots, Path::new(EXE), REPO),
        HookHealth::Healthy { block_leaks: false }
    );
    existing.push(ours("check_commit", WC, EXE, REPO));
    assert_eq!(
        judge_health(&existing, &roots, Path::new(EXE), REPO),
        HookHealth::Healthy { block_leaks: true }
    );
}

#[test]
fn health_is_stale_when_the_exe_moved() {
    let old = r"E:\old place\app.exe";
    let existing: Vec<HookEntry> = ["pre_commit", "post_commit", "post_update"]
        .iter()
        .map(|k| ours(k, WC, old, REPO))
        .collect();
    match judge_health(&existing, &[WC.to_string()], Path::new(EXE), REPO) {
        HookHealth::Stale { reasons } => {
            assert!(reasons.iter().any(|r| r.contains(old)), "{reasons:?}")
        }
        h => panic!("应为 Stale：{h:?}"),
    }
}

#[test]
fn health_is_stale_when_a_required_kind_is_missing() {
    let existing: Vec<HookEntry> = ["pre_commit", "post_commit"]
        .iter()
        .map(|k| ours(k, WC, EXE, REPO))
        .collect();
    match judge_health(&existing, &[WC.to_string()], Path::new(EXE), REPO) {
        HookHealth::Stale { reasons } => {
            assert!(
                reasons.iter().any(|r| r.contains("post_update")),
                "{reasons:?}"
            )
        }
        h => panic!("应为 Stale：{h:?}"),
    }
}

#[test]
fn health_is_stale_when_a_root_is_not_covered() {
    let existing: Vec<HookEntry> = ["pre_commit", "post_commit", "post_update"]
        .iter()
        .map(|k| ours(k, WC, EXE, REPO))
        .collect();
    let roots = vec![WC.to_string(), r"D:\work\ext".to_string()];
    assert!(matches!(
        judge_health(&existing, &roots, Path::new(EXE), REPO),
        HookHealth::Stale { .. }
    ));
}

#[test]
fn health_is_not_registered_without_any_entry_of_this_repo() {
    assert_eq!(
        judge_health(&[], &[WC.to_string()], Path::new(EXE), REPO),
        HookHealth::NotRegistered
    );
    let other_repo: Vec<HookEntry> = ["pre_commit", "post_commit", "post_update"]
        .iter()
        .map(|k| ours(k, WC, EXE, r"E:\elsewhere"))
        .collect();
    assert_eq!(
        judge_health(&other_repo, &[WC.to_string()], Path::new(EXE), REPO),
        HookHealth::NotRegistered
    );
}

#[test]
fn hook_command_quotes_paths_with_spaces_and_cjk() {
    assert_eq!(
        hook_command(Path::new(r"C:\a b\jkk.exe"), "post_commit", r"D:\项目 x"),
        r#""C:\a b\jkk.exe" --tsvn-hook post_commit --repo "D:\项目 x""#
    );
}

#[test]
fn registered_value_replaces_stale_entries_and_keeps_foreign_ones() {
    let foreign_one = foreign("start_update", WC);
    let stale = ours("post_commit", WC, r"E:\old\app.exe", REPO);
    let before = serialize_hooks(&[foreign_one.clone(), stale]);
    let after = registered_value(&before, &[WC.to_string()], Path::new(EXE), REPO, false).unwrap();
    let parsed = parse_hooks(&after);
    assert_eq!(parsed[0], foreign_one, "别人的条目原样保留且在最前");
    let mut k = kinds(&parsed[1..]);
    k.sort();
    assert_eq!(k, vec!["post_commit", "post_update", "pre_commit"]);
    assert!(parsed[1..].iter().all(|e| e.command.contains(EXE)));
    assert_eq!(
        judge_health(&parsed, &[WC.to_string()], Path::new(EXE), REPO),
        HookHealth::Healthy { block_leaks: false }
    );
}

#[test]
fn registered_value_refuses_on_conflict() {
    let before = serialize_hooks(&[foreign("post_commit", WC)]);
    let err = registered_value(&before, &[WC.to_string()], Path::new(EXE), REPO, true).unwrap_err();
    assert_eq!(err.code, "tsvn_hooks_conflict");
    assert!(err.message.contains("post_commit"), "{}", err.message);
}

#[test]
fn unregistered_value_drops_only_this_repo_and_deletes_when_empty() {
    let mine = ours("post_commit", WC, EXE, REPO);
    assert_eq!(
        unregistered_value(&serialize_hooks(std::slice::from_ref(&mine)), REPO),
        None
    );
    let keep = foreign("pre_commit", r"C:\x");
    assert_eq!(
        unregistered_value(&serialize_hooks(&[keep.clone(), mine]), REPO),
        Some(serialize_hooks(&[keep]))
    );
}

#[test]
fn health_serializes_with_state_tag() {
    let v = serde_json::to_value(HookHealth::Healthy { block_leaks: true }).unwrap();
    assert_eq!(
        v,
        serde_json::json!({ "state": "healthy", "blockLeaks": true })
    );
    let v = serde_json::to_value(HookHealth::Stale {
        reasons: vec!["x".into()],
    })
    .unwrap();
    assert_eq!(v, serde_json::json!({ "state": "stale", "reasons": ["x"] }));
    let v = serde_json::to_value(HookHealth::NotRegistered).unwrap();
    assert_eq!(v, serde_json::json!({ "state": "notRegistered" }));
}
