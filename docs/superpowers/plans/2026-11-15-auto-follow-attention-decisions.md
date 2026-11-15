# 自动跟进被挡下时的提醒（2026-11-15）实现日志

输入：2026-11-11 使用报告 9.5「自动跟进被挡下时更醒目」。当时 14:20 的更新没有自动跟进，原因只写在窗口里的横幅上，到 16:20 才被注意到。没有对应的计划文件，本文件同时记方案取舍。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 Rust：任务栏闪烁与角标、托盘图标加点、托盘提示写原因；命令 `set_attention` | 完成 | `src-tauri/src/attention.rs`、`commands/attention.rs`；`desktop.rs` 导出 `MAIN_WINDOW` / `TRAY_ID`、接失焦 / 显示 / 页面加载；`lib.rs` 注册；`tests/svn_write_guard.rs` 归类 | 见下「全量」 |
| T2 前端：各标签上报「没自动跟进」的原因，汇总后发给后端 | 完成 | `src/engine/attention.ts`（汇总与去重）、`src/store/attentionStore.ts`（全应用一份、串行发送）、`RepoTab.tsx`（上报、关标签时撤） | 见下「全量」 |
| T3 真实窗口验证 | 完成 | — | 见「真实窗口验证」 |
| 独立 review（`/code-review high`） | 10 条：修 7 条（含改文档 1 条）、记 3 条 | 见「review 处理」 | 新增测试修之前红 |

全量：

- 前端（最终代码）：`pnpm build` 通过；`pnpm test` 99 个文件、1684 条通过。
- Rust（最终代码）：`cargo fmt --check`、`cargo clippy --all-targets` 无告警；lib 单元测试 162 条通过（`attention` 17 条）；`svn_write_guard` 9 条通过。lib 第一次跑时 `commands::watch::tests::stage_and_unstage_only_touch_the_index` 失败一次，紧接着连跑两次 162 条全过；这条在 2026-11-12 两份、2026-11-14 一份实现日志里都记过时好时坏（多收一条 `.git/hooks` 事件），本次没有改 `watch.rs`。
- Rust 集成测试全量（`cargo test --no-fail-fast`，98 个测试二进制、2041 条通过）跑的是 review 修改**之前**的代码。review 只改了 `attention.rs`、`commands/attention.rs`、`desktop.rs` 的窗口事件三处，集成测试不经过它们，修改之后只重跑了 lib 与 `svn_write_guard`。

## 方案取舍

| 方案 | 做法 | 结论 |
|---|---|---|
| A 系统通知（toast） | 加 `tauri-plugin-notification` | 不做。要新增依赖（AGENTS.md §1.2 要先征得同意）；Windows 的 toast 要求应用有登记过的 AUMID，便携版没有开始菜单快捷方式，能不能弹出来没有把握，而两台机器上用的都是便携版 |
| B 任务栏 + 托盘，只用 Tauri 自带的接口 | 任务栏按钮闪烁（`request_user_attention`）加角标（`set_overlay_icon`）；托盘图标右下角加一个点（`set_icon`）；托盘提示写原因（`set_tooltip`） | **采用**。不加依赖；使用报告本来就把「任务栏角标」列为可选做法；窗口藏在托盘里时还有托盘图标上的点 |
| C 只改横幅样式 | 横幅加粗、换色 | 不做。问题是人没在看这个窗口 |

提醒跟着横幅上的原因行走：原因行在就挂着，原因行消失（跟进了、条件变了、设置里关掉自动跟进）就撤掉。

## 决策条目

### T1 Rust

- **偏离使用报告：没有系统通知。** 报告写「发一次系统通知或任务栏角标」。理由见「方案取舍」A。角标 + 托盘点 + 闪烁覆盖了「窗口在后台」和「窗口藏在托盘」两种情况。
- **图标是现画的，不加图片资源。** 托盘图标取 `default_window_icon()` 的 RGBA，在右下角盖一个橙色实心圆（直径为短边的 46%，16×16 上约 7 像素）；任务栏角标是 16×16 透明底上的同色圆点。不做抗锯齿：16 像素上一圈半透明边反而发灰。宽高与字节数对不上时原样返回，不越界。
- **托盘提示自己截断到 127 个 UTF-16 单位**（`NOTIFYICONDATAW::szTip` 是 128 含结尾的 0）。超长时交给系统的话，有的 Windows 版本整条不显示。按 UTF-16 单位截，不拆开代理对，截过的以「…」结尾；原因里的换行合成空格。
- **窗口在前台时不闪。** 人正看着横幅。读不到焦点状态按不在前台算。
- **失败只记日志。** 命令不返回错误：托盘没建出来、平台不支持角标都不该让调用它的流程报错。
- **计划没写的坑：** 每个新 Tauri 命令都要在 `tests/svn_write_guard.rs` 的 `NOT_GIT_ONLY` 里归类（`perf_append` 那次就踩过，见 2026-11-12-performance-monitor-decisions）。`attention.rs` 整个文件放行：只动窗口，不碰仓库。
- **为什么这么改是安全的：** 不读写任何仓库、不动 git 状态；`set_icon(None)` 不会发生（撤提醒时设回 `default_window_icon` 的原图）；托盘 id 原来是字面量 `"main"`，改成常量 `TRAY_ID` 后值不变。

### T2 前端

- **汇总放在全应用一份的 store 里，按规范化路径记各标签的原因。** 任务栏和托盘只有一个，几个标签各自上报。一个仓库被挡时写「Client 没有自动跟进：原因」，几个时只写个数和仓库名（127 个字符放不下几条原因）。
- **汇总文字没变就不发**：标签每次渲染都会上报。
- **只在多了一件事时闪**：新的仓库被挡下，或者同一个仓库换了原因。别的仓库撤掉之后剩下的那条不是新的事，不闪。
- **原因取横幅上同一个值**（`autoSkip` 只认当前现场的 `autoFollowKey`），抽成 `autoFollowBlocked` 后横幅和提醒共用，保证两者一起出现、一起消失。
- **标签关掉时撤**：effect 的清理函数上报 `null`。不撤的话会留下一个没有出处的角标。
- **偏离 AGENTS.md §7.5 的分层？** 没有：组件调 store（`reportAutoFollowBlocked`），store 调 `invoke`，纯逻辑在 `src/engine/attention.ts`。

## 真实窗口验证（2026-11-15，开发机）

- 隔离：`pnpm tauri dev --config <json>`，identifier `com.momadacoding.jkk.e2eattention`，vite 5194、CDP 9233（另有并发会话占着默认端口）。核对过窗口属于本 worktree 的 `target\debug\app.exe`。fixture 由 `scripts/e2e-svn-fixture.ps1` 造在 `%TEMP%`，验完连同两个配置目录删了。
- **参数名**：`{ reason, flash }` 通过；`{ reason: null }`、空白原因通过（撤提醒）；缺 `flash` 被 Tauri 拒绝（`missing required key flash`）。`{ message, flash }` 也通过：`reason` 是 `Option`，缺了按 `null` 处理，等于撤提醒，不会出错。
- **任务栏角标**：手工挂一条之后截任务栏，jkk 按钮右上角出现橙色圆点。
- **真实的被挡下**（自动跟进 E2E 场景 4：分支上提交 e2、磁盘上 e3 用 `svn commit` 提交，重放冲突）：横幅出现「没有自动跟进：当前分支的提交重放到新基线上有冲突（Code/A.cs）……」，同时发出一次 `set_attention`，`reason` 为「wc1 没有自动跟进：……」，`flash: true`。
- **撤掉**：关掉自动跟进后挪一次 HEAD，原因行消失，发出 `{ reason: null, flash: false }`。
- **关标签**：重新被挡下（又发一次带 `flash: true` 的）之后关掉标签，发出 `{ reason: null, flash: false }`。
- **没验到的**：任务栏按钮的闪烁本身（窗口被 CDP 驱动时一直不是前台，但没有录屏看闪没闪）；托盘图标上的点和托盘提示文字（托盘图标在溢出区里，没有截到）；公司机器上真的 TortoiseSVN 钩子触发的那条路。

## review 处理（`/code-review high`，2026-11-15）

review 只读代码，没有跑。10 条：

| 条 | 问题 | 处理 |
|---|---|---|
| 1 | `set_attention` 是 async 命令，前后两次各落在线程池上，「挂上」「撤掉」可能倒着生效，角标永远留着 | **修**。前端排成一条链，上一次回来（成功或失败）才发下一次（`createAttentionReporter`）。`attentionStore.test.ts` 2 条，修之前红 |
| 2 | 页面重载（F5、开发时整页重载）后前端汇总从头开始、以为什么都没挂，角标再也撤不掉 | **修**。`desktop::on_page_load` 在 `Started` 时调 `attention::on_page_reload` 撤掉；新页面上的标签会重新上报 |
| 3 | 几个仓库同时被挡下时托盘只写名字，其中一个换了原因汇总文字不变，按文字去重就不闪 | **修**。有新的事时文字相同也发。测试 1 条，修之前红 |
| 4 | 藏到托盘时任务栏按钮被销毁，再显示后角标没了 | **修**。后端记下现在的原因，`show_main_window` 之后重挂（`attention::on_show`）。只覆盖经 `show_main_window` 显示的路径（托盘单击、托盘菜单、快捷键、单实例唤起） |
| 5 | README 说「关掉自动跟进」会撤，实际不会：`autoSkip` 要等下一次自动跟进才清 | **改文档**。README 与 E2E 改成实际行为。没有改代码：关掉设置时清原因行要让设置框通知每个标签，现在没有这条通路，而原因行本身也是同样的行为 |
| 6 | 被挡下时窗口在前台不闪，之后人切走也不再闪，只剩静态角标 | **修**。后端记一个「待闪」，主窗口失焦时补闪一次（`Attention::on_blur`，`desktop::on_window_event` 接 `Focused(false)`）。状态机单元测试 6 条 |
| 7 | 自动跟进**出错**（进错误条，ref 可能已经动过）没有提醒，比被挡下更严重 | **不做，记为遗留**。错误条是全局的、不分来源，要接提醒得先让自动跟进的出错有自己的出口；超出 9.5 的范围 |
| 8 | 同名仓库（同一个工程的几份工作副本）在托盘里分不清 | **修**。被挡下的几个里同名时多带一级目录（`trunk/Client`、`x/Client`）。测试 2 条 |
| 9 | 「空白算没有」在前端、命令入口、托盘提示里各判一次 | **不改**。三层各自独立可测；去掉任何一层都要改那一层的测试，收益只是少几行 |
| 10 | 每次更新都重画托盘图标、重设角标 | **不改**。16×16 到 32×32 的 RGBA，原因只在状态变化时才发，不在热路径上 |

**为什么第 6 条这么改是安全的**：`on_blur` 跑在主线程上（窗口事件），而 `set` 跑在 async 命令的工作线程上、会调派到主线程的窗口接口。`STATE` 的锁只包住「算决定」那几行，调窗口接口前就放开，不会出现工作线程持锁等主线程、主线程等锁的死锁（`desktop::DesktopState` 的注释记过同一个坑）。

## 遗留

- 系统通知（toast）没做。若之后要做，需要先确认依赖与便携版的 AUMID 问题。
- 自动跟进出错（review 第 7 条）没有提醒。
- review 修完之后没有重新在真实窗口里走一遍：失焦补闪、藏到托盘再显示后重挂角标、F5 撤掉，这三条只有状态机的单元测试，窗口那一侧没亲眼看到。
- 提醒只覆盖「自动跟进被挡下」。别的需要人看的状态（泄漏告警、缺 Local-Only 补丁的常驻横幅）没有接，按需再加：`reportAutoFollowBlocked` 是唯一入口，加来源时把汇总的键从仓库扩成（仓库, 来源）。
