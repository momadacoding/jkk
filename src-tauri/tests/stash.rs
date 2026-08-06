use app_lib::commands::stash as cmd;
use app_lib::engine::stash;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git").current_dir(dir).args(args).status().expect("git not found");
    assert!(status.success(), "git {args:?} failed");
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init"]);
    run_git(dir.path(), &["config", "user.email", "test@test.com"]);
    run_git(dir.path(), &["config", "user.name", "Test"]);
    fs::write(dir.path().join("a.txt"), "1").unwrap();
    run_git(dir.path(), &["add", "."]);
    run_git(dir.path(), &["commit", "-m", "c1"]);
    dir
}

#[test]
fn list_is_empty_on_a_clean_repo() {
    let dir = repo();
    assert!(stash::list(dir.path().to_str().unwrap()).unwrap().is_empty());
}

#[test]
fn push_then_list_reports_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();

    cmd::stash_push(p.clone(), Some("my work".into()), false).unwrap();
    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].index, 0);
    assert!(entries[0].message.contains("my work"), "got {}", entries[0].message);
    // hash 要能直接喂给现成的 commit:<hash> diff scope，所以必须是完整 oid
    assert_eq!(entries[0].hash.len(), 40);
    // 工作区已经被还原
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
}

#[test]
fn push_without_message_still_produces_an_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();

    cmd::stash_push(p.clone(), None, false).unwrap();
    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].message.is_empty());
}

#[test]
fn include_untracked_captures_new_files() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("new.txt"), "fresh").unwrap();

    cmd::stash_push(p.clone(), None, true).unwrap();
    assert!(!dir.path().join("new.txt").exists(), "untracked file was not stashed");
    assert_eq!(stash::list(&p).unwrap().len(), 1);
}

#[test]
fn newest_stash_is_index_zero() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "first").unwrap();
    cmd::stash_push(p.clone(), Some("first".into()), false).unwrap();
    fs::write(dir.path().join("a.txt"), "second").unwrap();
    cmd::stash_push(p.clone(), Some("second".into()), false).unwrap();

    let entries = stash::list(&p).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].message.contains("second"), "got {}", entries[0].message);
    assert_eq!(entries[0].index, 0);
    assert_eq!(entries[1].index, 1);
}

#[test]
fn pop_restores_changes_and_removes_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_pop(p.clone(), 0).unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "changed");
    assert!(stash::list(&p).unwrap().is_empty());
}

#[test]
fn apply_restores_changes_but_keeps_the_entry() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_apply(p.clone(), 0).unwrap();
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "changed");
    assert_eq!(stash::list(&p).unwrap().len(), 1);
}

#[test]
fn drop_removes_the_entry_without_touching_the_working_tree() {
    let dir = repo();
    let p = dir.path().to_str().unwrap().to_string();
    fs::write(dir.path().join("a.txt"), "changed").unwrap();
    cmd::stash_push(p.clone(), None, false).unwrap();

    cmd::stash_drop(p.clone(), 0).unwrap();
    assert!(stash::list(&p).unwrap().is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "1");
}
