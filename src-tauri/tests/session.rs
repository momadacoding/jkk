use app_lib::engine::session::{self, Session};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .expect("git not found");
    assert!(status.success());
}

/// 一个有提交的临时仓库。
fn repo_with_commit(parent: &std::path::Path, name: &str) -> String {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    run_git(&dir, &["init", "--quiet"]);
    run_git(&dir, &["config", "user.email", "test@test.com"]);
    run_git(&dir, &["config", "user.name", "Test"]);
    fs::write(dir.join("a.txt"), "1").unwrap();
    run_git(&dir, &["add", "a.txt"]);
    run_git(&dir, &["commit", "-qm", "c1"]);
    dir.to_str().unwrap().to_string()
}

/// 只 `git init`、一条提交都没有的仓库。
fn empty_repo(parent: &std::path::Path, name: &str) -> String {
    let dir = parent.join(name);
    fs::create_dir_all(&dir).unwrap();
    run_git(&dir, &["init", "--quiet"]);
    dir.to_str().unwrap().to_string()
}

#[test]
fn missing_file_yields_an_empty_session() {
    let dir = TempDir::new().unwrap();
    let session = session::load_from(&dir.path().join("nope").join("session.json"));
    assert_eq!(session, Session::default());
}

/// 会话文件是程序自己写的，写到一半被杀就是半截 JSON。
/// 这种情况必须退化成空会话，而不是让启动路径上冒出一个解析错误。
#[test]
fn broken_json_yields_an_empty_session_instead_of_an_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    fs::write(&path, b"{\"tabs\": [\"C:/a\"").unwrap();

    assert_eq!(session::load_from(&path), Session::default());
}

#[test]
fn save_then_load_round_trips() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested").join("session.json");
    let saved = Session::new(
        vec!["C:/a".to_string(), "C:/b".to_string()],
        Some("C:/b".to_string()),
        vec!["C:/b".to_string(), "C:/a".to_string()],
    );

    session::save_to(&path, &saved).unwrap();

    assert_eq!(session::load_from(&path), saved);
}

/// 目录里除了目标文件不该剩下别的（临时文件必须被 rename 掉或清理掉）。
fn leftovers(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n != "session.json")
        .collect()
}

/// 原子写要能覆盖已存在的文件，且不留下临时文件。
/// Windows 的 rename 语义和 POSIX 不同（MoveFileEx 需要 REPLACE_EXISTING），
/// 这条就是守住「std::fs::rename 能覆盖」这个假设。
#[test]
fn save_overwrites_an_existing_file_and_leaves_no_temp_behind() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    session::save_to(&path, &Session::new(vec!["C:/a".to_string()], None, vec![])).unwrap();
    session::save_to(&path, &Session::new(vec!["C:/b".to_string()], None, vec![])).unwrap();

    assert_eq!(session::load_from(&path).tabs, vec!["C:/b".to_string()]);
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
}

/// 并发保存不能互相踩烂。
///
/// 共用一个固定的 `.tmp` 名字时，T1 正在往 tmp 里写，T2 把**同一个** tmp
/// rename 成正式文件——落地的就是 T1 写到一半的内容，正是原子写要防的那种半截
/// JSON；另一种交错则让后到的 rename 直接 `NotFound` 失败。
///
/// 这条必须真的撞得上那个窗口才有意义，所以：多线程 + `Barrier` 齐发 +
/// 每次写几百 KB 把写入窗口拉开。已实测：把 `tmp_path` 换回固定名字时本用例失败
/// （rename 报 `NotFound` / 落地内容被截断），换回唯一名字后通过。
#[test]
fn concurrent_saves_never_produce_a_half_written_file() {
    use std::sync::{Arc, Barrier};

    const THREADS: usize = 8;
    const ROUNDS: usize = 12;
    // 每条路径都很长，8 个线程各写几百 KB，写入不再是一瞬间的事
    let bulky = |tag: &str| {
        let tabs: Vec<String> = (0..2000)
            .map(|i| format!("C:/{tag}/{}/repo{i}", "x".repeat(200)))
            .collect();
        Session::new(tabs, None, vec![])
    };

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.json");

    for _ in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(THREADS));
        let handles: Vec<_> = (0..THREADS)
            .map(|i| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                let session = bulky(&format!("t{i}"));
                std::thread::spawn(move || {
                    barrier.wait();
                    session::save_to(&path, &session)
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().expect("并发保存不该失败");
        }

        // 半截 JSON 会被 load_from 吞成空会话，所以「空」就是失败信号
        let got = session::load_from(&path);
        assert_eq!(got.tabs.len(), 2000, "落地的不是一次完整写入");
        // 落地的必须整份来自同一个线程，不能是几次写入拼起来的
        let tag = got.tabs[0].split('/').nth(1).unwrap().to_string();
        assert!(
            got.tabs
                .iter()
                .all(|t| t.starts_with(&format!("C:/{tag}/"))),
            "几次写入的内容混在了一起，落地的第一条是 {tag}"
        );
    }
    assert_eq!(leftovers(dir.path()), Vec::<String>::new());
}

/// 缺字段的旧文件不能让整份会话作废——将来加字段时同一条逻辑还要生效。
#[test]
fn missing_fields_fall_back_to_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    fs::write(&path, r#"{"version":1,"tabs":["C:/a"]}"#).unwrap();

    let session = session::load_from(&path);
    assert_eq!(session.tabs, vec!["C:/a".to_string()]);
    assert_eq!(session.active_tab, None);
    assert!(session.recent.is_empty());
}

#[test]
fn unknown_fields_are_ignored() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("session.json");
    fs::write(&path, r#"{"version":99,"tabs":["C:/a"],"future":true}"#).unwrap();

    assert_eq!(session::load_from(&path).tabs, vec!["C:/a".to_string()]);
}

/// 新打开的排在最前，且标签栏顺序里最右的算最新。
#[test]
fn merge_recent_puts_the_newest_tab_first() {
    let recent = session::merge_recent(
        &["C:/old".to_string()],
        &["C:/a".to_string(), "C:/b".to_string()],
    );
    assert_eq!(recent, vec!["C:/b", "C:/a", "C:/old"]);
}

/// 本项目的高发缺陷区：同一个仓库写成反斜杠、正斜杠、大小写不同、带尾部斜杠，
/// 都必须算同一条，否则最近列表会被同一个仓库的四种写法塞满。
#[test]
fn merge_recent_dedupes_windows_path_spellings() {
    let recent = session::merge_recent(
        &[
            r"c:\users\me\repo".to_string(),
            "C:/Users/Me/Repo/".to_string(),
            "C:/other".to_string(),
        ],
        &[r"C:\Users\Me\Repo".to_string()],
    );

    // 保留的是本次打开时用的原始写法，不是归一化后的形态
    assert_eq!(recent, vec![r"C:\Users\Me\Repo", "C:/other"]);
}

#[test]
fn merge_recent_is_capped_and_drops_empty_paths() {
    let opened: Vec<String> = (0..30).map(|i| format!("C:/r{i}")).collect();
    let recent = session::merge_recent(&["".to_string(), "C:/old".to_string()], &opened);

    assert_eq!(recent.len(), 10);
    assert_eq!(recent[0], "C:/r29");
    assert!(!recent.contains(&String::new()));
}

/// 空 tabs 不能把最近列表清空——关掉所有标签落到空态页时，
/// 恰恰是最需要那份列表的时候。
#[test]
fn merge_recent_keeps_history_when_no_tabs_are_open() {
    let recent = session::merge_recent(&["C:/a".to_string(), "C:/b".to_string()], &[]);
    assert_eq!(recent, vec!["C:/a", "C:/b"]);
}

/// 还原时打不开的路径要滤掉，还在的要留下。
#[test]
fn to_view_drops_tabs_that_are_not_repos() {
    let dir = TempDir::new().unwrap();
    let good = repo_with_commit(dir.path(), "good");
    let not_a_repo = dir.path().join("plain").to_str().unwrap().to_string();
    fs::create_dir_all(&not_a_repo).unwrap();
    let gone = dir.path().join("gone").to_str().unwrap().to_string();

    let view = session::to_view(Session::new(
        vec![not_a_repo.clone(), good.clone(), gone.clone()],
        None,
        vec![],
    ));

    assert_eq!(view.tabs, vec![good]);
    // 滤掉的要交代出去，否则前端无从知道这次还原是不完整的，
    // 会把截断后的结果立刻回存，把暂时打不开的标签永久抹掉
    assert_eq!(view.dropped_tabs, vec![not_a_repo, gone]);
}

/// 全都能打开时 dropped_tabs 必须是空的——前端拿它判断要不要跳过首次保存，
/// 这里多报一条就等于每次正常启动都不落盘。
#[test]
fn to_view_reports_nothing_dropped_when_every_tab_opens() {
    let dir = TempDir::new().unwrap();
    let good = repo_with_commit(dir.path(), "good");

    let view = session::to_view(Session::new(vec![good], None, vec![]));

    assert!(view.dropped_tabs.is_empty(), "{:?}", view.dropped_tabs);
}

/// 刚 `git init`、还没有任何提交的仓库必须能还原。
/// 用 `engine::repo::open` 做过滤会读 HEAD，把这种仓库判成打不开。
#[test]
fn to_view_keeps_a_repo_with_no_commits() {
    let dir = TempDir::new().unwrap();
    let empty = empty_repo(dir.path(), "fresh");

    let view = session::to_view(Session::new(vec![empty.clone()], None, vec![]));

    assert_eq!(view.tabs, vec![empty]);
}

/// 活动标签自己被滤掉时交回 None，让前端回落到第一个标签，
/// 而不是指向一个不存在的标签。
#[test]
fn to_view_clears_an_active_tab_that_was_filtered_out() {
    let dir = TempDir::new().unwrap();
    let good = repo_with_commit(dir.path(), "good");
    let gone = dir.path().join("gone").to_str().unwrap().to_string();

    let view = session::to_view(Session::new(vec![good], Some(gone), vec![]));

    assert_eq!(view.active_tab, None);
}

/// 活动标签的匹配也走归一化：存的时候是反斜杠、还原时是正斜杠，不能因此失配。
#[test]
fn to_view_matches_the_active_tab_across_path_spellings() {
    let dir = TempDir::new().unwrap();
    let good = repo_with_commit(dir.path(), "good");
    let other_spelling = good.replace('/', "\\").to_uppercase();

    let view = session::to_view(Session::new(
        vec![good],
        Some(other_spelling.clone()),
        vec![],
    ));

    assert_eq!(view.active_tab, Some(other_spelling));
}

/// 最近列表**不**按「是不是仓库」过滤，只标注还在不在。
/// 移动硬盘拔掉一次就把历史永久删掉，用户再也找不回那个仓库。
#[test]
fn to_view_keeps_missing_recents_but_marks_them() {
    let dir = TempDir::new().unwrap();
    let good = repo_with_commit(dir.path(), "good");
    let gone = dir.path().join("gone").to_str().unwrap().to_string();

    let view = session::to_view(Session::new(vec![], None, vec![good.clone(), gone.clone()]));

    assert_eq!(view.recent.len(), 2);
    assert_eq!(view.recent[0].path, good);
    assert!(view.recent[0].exists);
    assert_eq!(view.recent[1].path, gone);
    assert!(!view.recent[1].exists);
}
