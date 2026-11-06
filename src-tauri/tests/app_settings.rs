#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
use app_lib::engine::app_settings::{self, AppSettings, DEFAULT_SHOW_SHORTCUT};
use std::fs;
use tempfile::TempDir;

#[test]
fn missing_file_yields_defaults() {
    let dir = TempDir::new().unwrap();
    let s = app_settings::load_from(&dir.path().join("nope").join("settings.json"));
    assert_eq!(s, AppSettings::default());
    // 默认值本身就是需求：关窗进托盘，快捷键开着
    assert!(s.close_to_tray);
    assert_eq!(s.show_shortcut.as_deref(), Some(DEFAULT_SHOW_SHORTCUT));
}

#[test]
fn corrupt_file_yields_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, "{ not json").unwrap();
    assert_eq!(app_settings::load_from(&path), AppSettings::default());
}

#[test]
fn missing_fields_fall_back_to_their_own_defaults() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, r#"{ "closeToTray": false }"#).unwrap();
    let s = app_settings::load_from(&path);
    assert!(!s.close_to_tray);
    assert_eq!(s.show_shortcut.as_deref(), Some(DEFAULT_SHOW_SHORTCUT));
}

#[test]
fn explicit_null_shortcut_stays_disabled() {
    // 用户主动关掉快捷键后，下次启动不能被默认值「复活」
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("settings.json");
    fs::write(&path, r#"{ "closeToTray": true, "showShortcut": null }"#).unwrap();
    assert_eq!(app_settings::load_from(&path).show_shortcut, None);
}

#[test]
fn save_then_load_round_trips_and_overwrites() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("sub").join("settings.json");
    let first = AppSettings {
        close_to_tray: false,
        show_shortcut: None,
    };
    app_settings::save_to(&path, &first).unwrap();
    assert_eq!(app_settings::load_from(&path), first);

    let second = AppSettings {
        close_to_tray: true,
        show_shortcut: Some("Ctrl+Shift+F12".into()),
    };
    app_settings::save_to(&path, &second).unwrap();
    assert_eq!(app_settings::load_from(&path), second);

    // 原子写不能在目录里留下临时文件
    let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|n| n != "settings.json")
        .collect();
    assert!(leftovers.is_empty(), "残留文件：{leftovers:?}");
}

#[test]
fn validate_accepts_combos_with_a_real_modifier() {
    for ok in [
        "Ctrl+Alt+J",
        "Alt+F1",
        "Super+Space",
        "Ctrl+Shift+Backquote",
        "ctrl+alt+j",
        // 以下是前端 captureShortcut 会产出的形态（tests/vitest/appSettings.test.ts），
        // 两侧的键名表一旦漂移，用户录下的快捷键会在保存时被拒
        "Ctrl+Alt+Shift+Super+F12",
        "Ctrl+Shift+1",
        "Ctrl+ArrowUp",
        "Ctrl+Numpad5",
        "Ctrl+NumpadAdd",
        "Ctrl+Space",
        "Ctrl+Backquote",
    ] {
        assert!(app_settings::validate_shortcut(ok).is_ok(), "{ok} 应该合法");
    }
}

#[test]
fn validate_rejects_bare_keys_shift_only_and_garbage() {
    // 单键 / 只带 Shift 的组合注册成全局快捷键后，用户在任何程序里都打不出这个字
    for bad in [
        "",
        "J",
        "F5",
        "Shift+J",
        "Ctrl+Alt+",
        "Ctrl+Alt+NoSuchKey",
        "Ctrl+J+K",
        "+",
    ] {
        assert!(
            app_settings::validate_shortcut(bad).is_err(),
            "{bad:?} 应该被拒绝"
        );
    }
}
