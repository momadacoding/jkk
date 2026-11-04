# 2026-11-04 关闭到托盘 + 全局快捷键：实现日志

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 1 设置读写 | 完成 | `src-tauri/src/engine/app_settings.rs`、`src-tauri/tests/app_settings.rs`；`engine/session.rs` 抽出 `write_atomic` | 先红（模块不存在编译失败）后绿；`cargo test --test app_settings --test session` 全过 |
| 2 托盘 / 关闭拦截 / 快捷键 | 完成 | `src-tauri/src/desktop.rs`、`src-tauri/src/commands/app_settings.rs`、`lib.rs`、`Cargo.toml` | `cargo fmt --check`、`cargo clippy --all-targets` 无告警 |
| 3 前端设置框 | 完成 | `src/engine/appSettings.ts`、`src/store/appSettingsStore.ts`、`src/components/SettingsDialog.tsx`、`TabBar.tsx`、`App.tsx`、`styles.css`、`tests/layout/{scenarios.ts,harness.tsx}`、`tests/vitest/{appSettings.test.ts,SettingsDialog.test.tsx}` | 先红后绿；`pnpm test` 全过，`pnpm build` 通过，`pnpm test:layout` 140 passed |
| 4 验证 | 完成 | `THIRD_PARTY_NOTICES.md` 重新生成 | `node scripts/gen-notices.mjs --check` 通过；`cargo test` 1643 passed；`pnpm tauri dev` 实测见「Task 4」 |

## 决策条目

### 总体

- **新增依赖**：`tauri-plugin-global-shortcut 2.3.2`（连带 `global-hotkey 0.8.0`，Linux 上还有 x11rb）。
  用户在对话中要求「不要问我问题，自己按照最优解处理」，未单独确认。托盘用 Tauri 自带的
  `tray-icon` feature，不新增 crate；前端不引入 `@tauri-apps/plugin-global-shortcut`，
  快捷键只在 Rust 侧注册，capabilities 不用开新权限。
- **为什么不做单实例**：dev 构建与安装版共用 identifier `com.momadacoding.jkk`，
  `tauri-plugin-single-instance` 会让 `pnpm tauri dev` 起来就把焦点交给安装版并退出。
  代价：窗口藏在托盘里时再双击桌面图标会开出第二个实例。第二个实例注册同一个快捷键会失败，
  设置框里会显示「注册失败」。

### Task 1

- **计划没写的坑**：`showShortcut` 必须区分「字段缺失」（取默认）和「显式 `null`」（用户关掉了）。
  serde 的 `#[serde(default = ...)]` 对 `Option` 恰好是这个语义：缺字段走默认函数，`null` 反序列化成 `None`。
  有用例 `explicit_null_shortcut_stays_disabled` 守住。
- **计划没写的坑**：global-hotkey 的解析器接受单键（`"J"`、`"F5"`）和只带 Shift 的组合。
  这些注册成全局快捷键后，用户在任何程序里都打不出那个字符，所以 `validate_shortcut`
  额外要求至少带 Ctrl / Alt / Super 之一。前端 `captureShortcut` 用同一条规则；
  Rust 用例里列了前端会产出的每种形态，两侧的键名表漂移时 `cargo test` 会失败。
- **偏离计划**：原子写没有在 `app_settings.rs` 里再写一份，而是把 `session::save_to` 的落盘部分抽成
  `pub(crate) fn write_atomic`，两边共用。`session.rs` 的行为不变，`tests/session.rs` 17 条全过。

### Task 2

- **计划没写的坑（死锁）**：插件的 `register` / `unregister` 会把活儿派到主线程，再阻塞等结果；
  而 `CloseRequested` 就在主线程上处理。若关窗处理和保存设置共用一把锁，保存线程持锁等主线程、
  主线程等锁，会卡死。所以 `close_to_tray` 单独放 `AtomicBool`，主线程上的代码（关窗、托盘、
  快捷键回调）一律不碰 `Mutex`。`save_app_settings` 用 async 命令，只是为了不在主线程上等注册。
  （初稿写的是「同步命令会自己等自己」，review 指出不对：`run_on_main_thread` 在主线程上会就地执行。）
- **偏离计划**：计划写的是「已在前台时再按一次快捷键就隐藏」。实现时去掉了：用户在设置框里重录同一个
  组合键时，系统会先把这次按键当成全局快捷键派过来，窗口会当场消失。现在快捷键只负责呼出。
- **为什么这么改是安全的**：
  - 换快捷键时先注册新的，失败就尽量装回旧的并报错，磁盘上的设置不变；避免下次启动去注册一个注定失败的组合。
  - 托盘建不出来时强制退回「关窗即退出」（`TRAY_MISSING`），否则窗口藏起来后只剩快捷键能找回。
  - `desktop::setup` 整体失败只记日志，不让应用起不来；这时 `DesktopState` 没有 manage，
    关窗走 `try_state` 拿不到，于是照常退出；设置框读不到状态会显示「读取设置失败」。
  - 托盘菜单「退出」走 `app.exit(0)`，不经过 `CloseRequested`，不会被拦截。
  - 关掉托盘选项时，关窗仍走 Tauri 默认路径：最后一个窗口关闭即退出，与改动前一致。
  - 不涉及任何 git 操作。
- **自查修正**：最初的回滚逻辑按磁盘上的设置去「装回旧快捷键」。启动时就没注册上的情况下，用户原样点保存，
  回滚会拿同一个组合再试一次，然后误报「旧快捷键也没能恢复」。改成只装回刚才真正注册着的那个
  （`old` 为 `None` 时不重试，保留原来的启动错误）。
- **review 后修正（独立 review subagent，无高危）**：
  - 中：新快捷键注册成功但 `settings.json` 写失败时，系统里生效的是新的，设置框显示旧的。
    改成写盘失败也回滚：注销新的、装回旧的（与注册失败共用 `restore_old`），`shortcut_error` 挪到写盘成功之后才清。
  - 低：注销旧快捷键失败时原来只记日志就继续，旧组合会留在系统里，也能呼出。改成直接报错、不换。
  - 低：托盘没建出来时设置框仍显示「隐藏到托盘」已勾选，勾了也不生效。视图加 `trayAvailable`，前端置灰并说明。
  - 低：录制框收 `Tab`，但焦点圈在捕获阶段先把 Tab 拿走，Ctrl+Tab 录不进去。从可录键里去掉 `Tab`。
  - 低：AltGr 在 Windows 上同时报 Ctrl 和 Alt，会录成 Ctrl+Alt+X，注册后欧洲布局的 AltGr 字符打不出来。
    改成物理右 Alt 按着时拒收。没用 `getModifierState('AltGraph')`：带 AltGr 的布局下，
    Chromium 对左 Ctrl+左 Alt 也可能报 AltGraph，默认组合会录不进来。
  - 低：补了「改回原来的快捷键，旧错误重新出现」的组件测试。

### Task 3

- **计划没写的坑**：录制框用 `KeyboardEvent.code` 而不是 `key`：Shift 会把 `key` 从 `1` 变成 `!`，
  非 QWERTY 布局也会改 `key`。
- **计划没写的坑**：录制中的 Esc 要 `stopPropagation`，否则窗口级的 Esc 监听会把整个设置框关掉。
  React 17 起合成事件的 `stopPropagation` 也会停掉原生事件，挂在 `window` 上的监听收不到。
- **偏离计划**：无。

### Task 4（`pnpm tauri dev` 实测，2026-11-04）

确认运行的是本 worktree 的 `src-tauri\target\debug\app.exe`（dev 构建的二进制名是 `app.exe`，不是 `jkk.exe`）。
实测前备份 `%APPDATA%\com.momadacoding.jkk\session.json`，结束后恢复并删掉测试产生的 `settings.json`（哈希核对一致）。

| 场景 | 做法 | 结果 |
|---|---|---|
| 默认设置 | CDP 调 `read_app_settings` | `closeToTray=true`、`Ctrl+Alt+J`、无错误；外部 `RegisterHotKey` 探测返回 1409（已被占），确认是本应用占着 |
| 参数名反例 | `save_app_settings` 传 `close_to_tray` | 报 `missing required key closeToTray`，符合预期 |
| 点 × 进托盘 | 向主窗口 `PostMessage(WM_CLOSE)`（与点 × 同一路径） | 窗口不可见，进程仍在 |
| 快捷键呼出 | 向插件的 `global_hotkey_app` 窗口投递 `WM_HOTKEY`，id 按源码算 `(CONTROL\|ALT)<<16 \| KeyJ` | 窗口重新可见 |
| 快捷键反例 | 投递一个没注册的 id（Ctrl+Alt+K） | 窗口保持隐藏 |
| 关闭 / 重开快捷键 | `showShortcut: null` 再改回 | 外部探测分别为「空闲」「被占」，async 命令派主线程注册没有卡死 |
| 快捷键被占 | 本 shell 先占 Ctrl+Alt+K，再让应用改成它 | 报「注册失败，可能已被其他程序占用」，Ctrl+Alt+J 仍有效，`settings.json` 未变 |
| 设置框 | CDP 点 ⚙、录 Ctrl+Alt+L、截图、点取消 | 渲染正常，录制显示 `Ctrl+Alt+L`，取消后未落盘 |
| 关掉托盘后点 × | `closeToTray=false` 后 `WM_CLOSE` | 进程退出 |

review 修正后又起了一次 dev 实例复验：`read_app_settings` 带出 `trayAvailable: true`；
Ctrl+Alt+K 被占时改它报错、Ctrl+Alt+J 仍在；放开后改成 Ctrl+Alt+K 成功，J 被释放、K 被应用占住；
`closeToTray=false` 后 `WM_CLOSE` 进程退出。结束后同样恢复了 `session.json`、删掉 `settings.json`。

`pnpm test:layout` 有一次 1 failed / 139 passed，当时 `cargo test` 在并行编译，失败的是哪一条没有留下输出；
随后在空闲机器上连跑两次都是 140 passed。

- **计划没写的坑（验证手段）**：从本会话的 shell 用 `keybd_event` 注入按键触发不了任何系统热键，
  连同一个 shell 自己 `RegisterHotKey` 的对照热键也收不到 `WM_HOTKEY`。所以快捷键改成直接向插件窗口投递
  `WM_HOTKEY` 来验证，这条路径不经过「真实键盘输入 → 系统热键匹配」这一段。
- **未验证**：真实物理按键触发；托盘图标的左键单击与右键菜单「显示主界面 / 退出」（自动化点不到托盘区）；
  投递消息时窗口没有拿到前台（`FOREGROUND=False`），这是 Windows 只给收到真实输入的进程前台权的限制，
  真实按键下应能拿到，但未实测。
