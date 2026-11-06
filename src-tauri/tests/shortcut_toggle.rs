#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
use app_lib::desktop::{shortcut_action, ShortcutAction, WindowState};

fn win(visible: bool, minimized: bool, focused: bool) -> WindowState {
    WindowState {
        visible,
        minimized,
        focused,
    }
}

#[test]
fn front_window_is_hidden() {
    // 窗口就在眼前：再按一次是「收起来」
    assert_eq!(
        shortcut_action(win(true, false, true), false),
        ShortcutAction::Hide
    );
}

#[test]
fn hidden_in_tray_is_shown() {
    assert_eq!(
        shortcut_action(win(false, false, false), false),
        ShortcutAction::Show
    );
}

#[test]
fn minimized_is_shown() {
    // 最小化时 Windows 仍可能报 visible，焦点也可能还在：都不算「在前台」
    assert_eq!(
        shortcut_action(win(true, true, true), false),
        ShortcutAction::Show
    );
    assert_eq!(
        shortcut_action(win(true, true, false), false),
        ShortcutAction::Show
    );
}

#[test]
fn covered_by_other_window_is_brought_front_not_hidden() {
    // 看得见但被别的窗口挡着：用户按快捷键是想看它，藏起来就反了
    assert_eq!(
        shortcut_action(win(true, false, false), false),
        ShortcutAction::Show
    );
}

#[test]
fn recording_never_hides() {
    // 设置框正在录快捷键时，系统会先把这次按键当全局快捷键派过来；
    // 这时切换的话，窗口会在用户眼前消失
    assert_eq!(
        shortcut_action(win(true, false, true), true),
        ShortcutAction::Show
    );
    assert_eq!(
        shortcut_action(win(false, false, false), true),
        ShortcutAction::Show
    );
}
