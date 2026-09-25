//! 同步期间挂起 watcher（spec §5.2）。
//!
//! 挂起状态本身是纯逻辑，单独测得了；「去抖线程真的查了它」由最后那条源码断言
//! 守着——起一个真 watcher 线程需要 `AppHandle`，那在集成测试里拿不到。

use app_lib::commands::watch::{is_watch_suspended, resume_watch, suspend_watch};

#[test]
fn suspending_and_resuming_one_repo_does_not_touch_another() {
    let a = "C:/repos/alpha";
    let b = "C:/repos/beta";
    assert!(!is_watch_suspended(a));

    suspend_watch(a);
    assert!(is_watch_suspended(a));
    assert!(!is_watch_suspended(b), "挂起一个仓库不该影响另一个");

    resume_watch(a);
    assert!(!is_watch_suspended(a));
}

#[test]
fn suspension_is_path_normalized() {
    // watcher 自己按归一化路径登记（`watch_repo` 里的 `normalize`）。挂起这边
    // 若不归一化，Windows 上 `C:\repo` 与 `C:/repo` 会变成两个键，挂起就静默
    // 失效——而失效的表现只是「界面偶尔闪一下」，没人会去查。
    suspend_watch("C:\\repos\\gamma");
    assert!(is_watch_suspended("C:/repos/gamma"));
    resume_watch("C:/repos/gamma");
    assert!(!is_watch_suspended("C:\\repos\\gamma"));
}

#[test]
fn resuming_something_never_suspended_is_harmless() {
    // 失败路径会无条件调 resume，可能 suspend 根本没跑到。
    resume_watch("C:/repos/never-suspended");
    assert!(!is_watch_suspended("C:/repos/never-suspended"));
}

#[test]
fn the_debounce_loop_actually_checks_the_suspension() {
    // 上面三条测的是集合本身。集合正确但没人查它，是这个改动最可能的失效方式，
    // 而它在运行时完全无声——只表现为「同步时界面偶尔闪一下」。
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/watch.rs"),
    )
    .unwrap();
    let emit = src
        .find("emit(\"repo-changed\"")
        .expect("找不到发事件那一句——改名了？");
    let window = &src[emit.saturating_sub(400)..emit];
    assert!(
        // 断言带上那个 `!`：只查 `is_watch_suspended` 是不够的——去掉取反会让
        // 事件**只在挂起期间**发出（功能恰好反过来），而那样的代码同样含有这个
        // 标识符，测试照样绿（2026-09-25 独立 review）。
        window.contains("!is_watch_suspended"),
        "发 repo-changed 之前没有查挂起状态，或者判断被写反了（spec §5.2）"
    );
}

#[test]
fn a_suspension_guard_resumes_on_drop_even_when_the_flow_panics() {
    // 2b 独立 review N3：挂起与恢复分两处手写，只要中间一个 `?` 早退或 panic，
    // watcher 就永远停着——之后外部的每一次改动界面都看不见，而且没有任何报错。
    use app_lib::commands::watch::WatchSuspension;
    let path = r"C:\fake\guard-panic";
    let r = std::panic::catch_unwind(|| {
        let _g = WatchSuspension::new(path);
        assert!(is_watch_suspended(path), "构造即挂起");
        panic!("模拟同步流程中途 panic");
    });
    assert!(r.is_err());
    assert!(!is_watch_suspended(path), "panic 展开时 Drop 必须恢复");
}

#[test]
fn a_suspension_guard_resumes_on_normal_return() {
    use app_lib::commands::watch::WatchSuspension;
    let path = r"C:\fake\guard-normal";
    {
        let _g = WatchSuspension::new(path);
        assert!(is_watch_suspended(path));
    }
    assert!(!is_watch_suspended(path));
}
