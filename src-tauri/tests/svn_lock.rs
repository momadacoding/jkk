//! 跨进程仓库锁（spec §5.1.1）。
//!
//! 这把锁的全部价值在**崩溃之后**：清理代码跑不到的时候，锁还能不能被正确
//! 判定成「没人拿着」。所以本文件里最重要的不是 happy path，是那条真正杀掉
//! 一个外部进程的测试（见 `a_lock_whose_holder_was_killed_is_not_held`）。

use app_lib::engine::svn_lock::{acquire, holder, is_held, lock_path};

/// 锁只要求 `<repo>/.git` 存在，不需要是完整的混合仓库——
/// 它在 `init_hybrid` 之外也会被用到。
fn git_dir_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".git")).unwrap();
    d
}

#[test]
fn a_live_lock_blocks_a_second_acquisition() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();

    let guard = acquire(&root, "opid-A").unwrap();
    assert!(is_held(&root), "拿到锁之后必须报「有人持有」");

    let err = acquire(&root, "opid-B").expect_err("第二次获取必须失败，不能排队");
    assert_eq!(err.code, "sync_in_progress", "实际：{}", err.message);

    drop(guard);
    assert!(!is_held(&root), "Drop 之后必须释放");
    acquire(&root, "opid-C").expect("释放之后必须能重新获取");
}

#[test]
fn the_holder_is_readable_while_the_lock_is_held() {
    // §13 要求「显示持锁进程的 pid 与开始时间」。持锁期间读不到这些信息的话，
    // 用户看到的就只是一句「有人在同步」，无从判断是不是自己那个卡死的窗口。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let _guard = acquire(&root, "opid-XYZ").unwrap();

    let h = holder(&root).expect("持锁期间必须读得到持有者信息");
    assert_eq!(h.pid, std::process::id());
    assert_eq!(h.opid, "opid-XYZ");
    assert!(h.started_at > 0, "startedAt 必须落盘");
}

#[test]
fn a_normal_release_removes_the_lock_file_itself() {
    // 2026-09-25 由变异测试抓出来的真 bug。`Drop::drop` 在字段释放**之前**运行，
    // 所以删文件时句柄还开着；而我们拿锁时没给 `FILE_SHARE_DELETE`，于是
    // Windows 上 `remove_file` 失败、又被 `let _ =` 吞掉，锁文件**永远**留着。
    //
    // 功能上不致命（`is_held` 看的是句柄），但每次正常同步结束都会留下残锁，
    // 下一次 `acquire` 就会打出「上次同步没有正常收尾」——一条永远为假的告警。
    // 光靠 `is_held` 的断言抓不到这件事，必须直接看文件。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();

    let guard = acquire(&root, "opid-clean").unwrap();
    assert!(lock_path(&root).exists(), "持锁期间锁文件当然要在");
    drop(guard);

    assert!(
        !lock_path(&root).exists(),
        "正常释放之后锁文件必须消失，否则下次 acquire 会误报「上次没有正常收尾」"
    );
}

#[test]
fn a_lock_file_left_behind_by_a_crash_is_not_held() {
    // **文件存在 ≠ 有人拿着**（2026-09-25 实测：杀掉持锁进程后文件仍在）。
    // 把「文件存在」当判据的话，一次崩溃就让这个仓库永远同步不了。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    std::fs::write(
        lock_path(&root),
        br#"{"pid":999999,"startedAt":1,"opid":"dead"}"#,
    )
    .unwrap();

    assert!(
        !is_held(&root),
        "没有任何进程持有句柄时，残留文件不算被持有"
    );
    acquire(&root, "opid-new").expect("残锁必须可以接管");
}
