//! 跨进程仓库锁（spec §5.1.1）。
//!
//! 这把锁的全部价值在**崩溃之后**：清理代码跑不到的时候，锁还能不能被正确
//! 判定成「没人拿着」。所以本文件里最重要的不是 happy path，是那条真正杀掉
//! 一个外部进程的测试（见 `a_lock_whose_holder_was_killed_is_not_held`）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_init::InitRequest;
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

/// 用一个**真的外部进程**持锁，然后杀掉它。
///
/// 为什么非要跨进程：同进程内 drop 走的是我们自己的 `Drop`，验证的是「代码有没有
/// 写对」；崩溃走的是操作系统回收句柄，验证的是「这个机制本身成不成立」。后者才是
/// 这把锁敢用的理由，也是它区别于 pid 探测的地方。
///
/// 只在 Windows 上跑：非 Windows 分支是降级实现，没有等价语义可验。
#[cfg(windows)]
#[test]
fn a_lock_whose_holder_was_killed_is_not_held() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let path = lock_path(&root);

    // 外部进程：以「允许别人只读」的共享模式开着锁文件不放，正是 acquire 的形态
    let script = format!(
        "$f=[System.IO.File]::Open('{}','OpenOrCreate','ReadWrite','Read'); Start-Sleep 120",
        path.to_string_lossy().replace('\\', "/")
    );
    let mut child = app_lib::engine::proc::hidden_command("powershell")
        .args(["-NoProfile", "-Command", &script])
        .spawn()
        .expect("起不来 powershell");

    // 等它真的把文件打开；轮询而不是固定 sleep，避免在慢机器上假失败
    let mut opened = false;
    for _ in 0..50 {
        if is_held(&root) {
            opened = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(opened, "外部进程没能在 5 秒内拿到锁，测试前提不成立");

    assert!(
        acquire(&root, "mine").is_err(),
        "外部进程正持有时，本进程绝不能拿到锁"
    );

    child.kill().expect("杀不掉子进程");
    child.wait().ok();

    // 关键断言：没有任何清理代码跑过，但锁必须已经松开
    let mut released = false;
    for _ in 0..50 {
        if !is_held(&root) {
            released = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        released,
        "持锁进程已被杀，锁却仍报「有人持有」——崩溃之后这个仓库就再也同步不了了"
    );
    assert!(
        path.exists(),
        "锁文件本身应当还在（残留），只是被判成无人持有"
    );
    acquire(&root, "mine").expect("崩溃留下的残锁必须可以接管");
}

#[test]
fn a_stale_lock_naming_a_live_unrelated_pid_is_still_takeable() {
    // pid 复用的现实后果：pid 探测会把这把残锁永远判成「活着」，仓库被永久卡死。
    // 用当前测试进程自己的 pid 冒充——它当然活着，但它并没有持有这个句柄。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let meta = format!(
        r#"{{"pid":{},"startedAt":1,"opid":"recycled"}}"#,
        std::process::id()
    );
    std::fs::write(lock_path(&root), meta).unwrap();

    assert!(!is_held(&root), "pid 活着不等于这把锁有人拿着");
    acquire(&root, "mine").expect("必须可以接管");
}

#[test]
fn staging_is_refused_while_a_sync_holds_the_lock() {
    // 扫源码（`tests/svn_write_guard.rs`）能发现「忘了加」，发现不了「加了但没生效」。
    // 这条走真实后端，从 `backend_for` 一路打到 `assert_not_syncing`。
    let fx = svn_fixtures::svn_repo_with_native_eol();
    app_lib::engine::svn_init::init_hybrid(&InitRequest {
        repo_root: fx.wc_str(),
        whitelist: vec!["Code/".into()],
        // 扩展名**必须带前导点**：`svn_config` 会以 `svn_config / 扩展名必须以 . 开头`
        // 直接拒绝（计划里写成了 `"cs"`）。
        extensions: vec![".cs".into()],
    })
    .unwrap();
    let root = fx.wc_str();

    let backend = app_lib::vcs::backend_for(&root).unwrap();
    backend
        .stage(&["Code/Nat.cs".into()])
        .expect("没持锁时 stage 必须正常");

    let _guard = acquire(&root, "opid-sync").unwrap();
    let err = backend
        .stage(&["Code/Nat.cs".into()])
        .expect_err("持锁期间 stage 必须被拒");
    assert_eq!(err.code, "sync_in_progress", "实际：{}", err.message);

    // 读必须照常可用（spec §5.1：读操作不阻塞，否则同步期间界面整个瞎掉，
    // 而那恰恰是最需要显示状态的时候）。
    backend
        .status()
        .expect("持锁期间读操作不得被阻塞（spec §5.1）");
}
