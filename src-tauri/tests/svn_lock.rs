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
    // 只验证拿得到，拿到即放
    drop(acquire(&root, "opid-C").expect("释放之后必须能重新获取"));
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

#[cfg(windows)]
#[test]
fn an_unreadable_lock_file_is_treated_as_held_not_as_free() {
    // 2026-09-25 独立 review（N2）。`is_held` 原先只把 errno 32/33 当成「有人持有」，
    // **其余一切错误都当成「没人持有」**——包括权限错误（errno 5）。
    //
    // 那个方向是危险的那一边：`assert_not_syncing` 会因此放行写操作，而真相是
    // 我们根本不知道有没有人在同步。判不出来时必须**关门**，不是开门。
    //
    // 用只读属性造 errno 5（review 实测过这条路径）。共享冲突之外的错误在现实里
    // 还来自网络盘 / 多账户 ACL。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let p = lock_path(&root);
    std::fs::write(&p, br#"{"pid":1,"startedAt":1,"opid":"x"}"#).unwrap();

    let mut perm = std::fs::metadata(&p).unwrap().permissions();
    perm.set_readonly(true);
    std::fs::set_permissions(&p, perm).unwrap();

    let held = is_held(&root);

    // 先恢复可写，否则 TempDir 的清理会失败。
    //
    // `#[allow]`：clippy 的 `permissions_set_readonly_false` 警告的是它在 Unix 上
    // 会放开 all users 的写权限。这条测试是 `#[cfg(windows)]`，而 Windows 上
    // `set_readonly(false)` 只是清掉 FILE_ATTRIBUTE_READONLY 这一个属性位，
    // 不涉及 ACL；对象还是进程刚在 TempDir 里建的临时文件。
    #[allow(clippy::permissions_set_readonly_false)]
    {
        let mut perm = std::fs::metadata(&p).unwrap().permissions();
        perm.set_readonly(false);
        std::fs::set_permissions(&p, perm).unwrap();
    }

    assert!(
        held,
        "判不出存活时必须当成「有人持有」。当成「没人持有」会让写操作在一次真实的\
         同步进行中被放行——那正是这把锁存在的理由"
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
    drop(acquire(&root, "opid-new").expect("残锁必须可以接管"));
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
    drop(acquire(&root, "mine").expect("崩溃留下的残锁必须可以接管"));
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
    drop(acquire(&root, "mine").expect("必须可以接管"));
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

#[cfg(windows)]
#[test]
fn release_retries_when_a_reader_briefly_holds_the_file() {
    // 2b 第二轮 review R5：UI 读锁文件（`holder()` 的宽松共享打开，没有
    // FILE_SHARE_DELETE）恰好撞上持锁方 Drop 里的 remove_file，删除失败被吞，
    // 下一次 acquire 打出一条假的「上次同步没有正常收尾」。2c 的锁被持有期间
    // 界面每 2 秒轮询一次，撞上的概率从「理论上」变成「真的会有」。
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let lock = acquire(&root, "opid-R5").unwrap();
    let path = lock_path(&root);

    // 模拟 UI：用与 `holder()` 相同的共享模式开着文件 30ms 再关
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(&path)
        .unwrap();
    let t = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(30));
        drop(reader);
    });
    drop(lock);
    t.join().unwrap();
    assert!(
        !path.exists(),
        "读者 30ms 后就松手了，释放必须等到它再删掉文件"
    );
}

#[test]
fn busy_error_says_how_long_not_a_raw_epoch() {
    // 2b 第二轮 review R6：原来打印「开始于 1757000000」。
    use app_lib::engine::svn_lock::describe_age;
    assert_eq!(describe_age(1000, 1030), "刚刚开始");
    assert_eq!(describe_age(1000, 1000 + 5 * 60), "已持续 5 分钟");
    assert_eq!(describe_age(1000, 1000 + 2 * 3600 + 60), "已持续 2 小时");
    // 时钟回拨（started_at 在未来）不能 panic，也不能报负数
    assert_eq!(describe_age(2000, 1000), "刚刚开始");
}
