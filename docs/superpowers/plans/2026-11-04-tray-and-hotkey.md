# 关闭到托盘 + 全局快捷键呼出主界面

## 目标

- 点窗口右上角 × 时，按设置决定「隐藏到托盘、后台常驻」还是「直接退出」。
- 可配置一个系统级快捷键，任何时候按下都能呼出主界面（实现时去掉了「再按一次隐藏」，见 decisions）。
- 两项设置在应用内可改、落盘、下次启动生效。

## 非目标

- 不做单实例（`tauri-plugin-single-instance`）：dev 构建和安装版共用同一个 identifier，
  单实例会让 `pnpm tauri dev` 直接把焦点交给安装版、自己退出，开发流程会被打断。
- 不做开机自启、不做系统通知。

## 设计

| 项 | 做法 |
|---|---|
| 托盘 | Tauri 2 自带 `tray-icon` feature，图标复用窗口默认图标；左键单击显示主界面，右键菜单「显示主界面 / 退出」 |
| 关闭拦截 | `on_window_event` 收到 `CloseRequested` 且 `closeToTray = true` 时 `prevent_close` + `hide`；否则走原来的退出路径 |
| 全局快捷键 | `tauri-plugin-global-shortcut`，只在 Rust 侧注册，不暴露 JS 权限；按下时切换主窗口显隐 |
| 设置文件 | `app_config_dir()/settings.json`，与 `session.json` 同目录，原子写；读坏了退回默认值 |
| 默认值 | `closeToTray = true`、`showShortcut = "Ctrl+Alt+J"` |
| 改快捷键 | 先注册新的，失败则保住旧的并报错，不落盘 |
| 前端 | 标签栏右侧加「Settings」按钮 → `SettingsDialog`；按键录制逻辑放 `src/engine/shortcut.ts` |

## Tasks

1. Rust：`engine/app_settings.rs`（结构、默认值、读写）+ `tests/app_settings.rs`
2. Rust：托盘、关闭拦截、全局快捷键、`read_app_settings` / `save_app_settings` 命令
3. 前端：`engine/shortcut.ts` + vitest；`store/appSettingsStore.ts`；`SettingsDialog` + 标签栏入口；布局场景
4. 验证：两侧门槛 + `pnpm test:layout` + `pnpm tauri dev` 实测
