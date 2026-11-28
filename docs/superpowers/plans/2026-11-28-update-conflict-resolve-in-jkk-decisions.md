# 「更新」时 git 不管的文件冲突在 jkk 里解决：决策日志

计划：`2026-11-28-update-conflict-resolve-in-jkk.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 录制前冲突检查收窄 | 取消（见决策） | | |
| 2 S5 放行 | 完成 | `svn_conflict.rs`（分类、`describe_groups`）、`svn_sync_in.rs` S5、`svn_sync_check.rs` 预检文案、`tests/svn_sync_in.rs` 改写 2 条、新增 3 条 | `cargo test --test svn_sync_in -- conflict update_is_refused`：15 passed |
| 3 冲突列表与解决 | 完成 | `svn_conflict.rs`（列表、解决、外部合并）、`svn_xml::parse_info_conflicts`、`tests/svn_conflict.rs` | `cargo test --test svn_conflict`：11 passed |
| 4 命令与文案 | 完成 | `commands/svn_sync.rs` 两条、`commands/external.rs` 一条、`lib.rs` 注册；预检 / 录制 / 交付三处文案 | `cargo clippy --all-targets` 无告警 |
| 5 前端纯逻辑与 store | 完成 | `src/engine/svnConflicts.ts`、`repoStore` / `externalToolsStore` 三个 invoke、两个流程接 `offerSvnConflicts` | `pnpm test`：2080 passed |
| 6 弹框与接线 | 完成 | `SvnConflictsDialog.tsx`、`RepoTab.tsx`、`styles.css`、布局场景两个、组件测试 | `pnpm build` 通过；`pnpm test:layout`：218 passed |
| 7 验证 | 完成（独立 review 4 条 important 已修，见末节） | 决策条目 Task 7 | `cargo test`：lib 196 + 116 个测试二进制全过（分批）；`pnpm build` / `pnpm test` / `pnpm test:layout` 通过；`pnpm tauri dev` E2E 6 步通过 |

## 决策条目

### 设计阶段（相对预研的偏离）

- **偏离预研：再点「更新」不放行冲突。** 预研 §7 说「再点一次更新，按同样的规则放行」。实际读代码，更新预检（`svn_sync_check.rs` 的 `sync_in_svn_conflicted`）对整个工作副本的冲突一律拒绝，预研漏看了这一层。保留这道拒绝：冲突还在时 svn 会跳过这个文件、让它停在旧 revision（预研 §5.2 实测），解决的是对旧版本的冲突。先解决再更新，预研 §7.3 第 1 条的坑就不会出现。代价：冲突不解决就不能再更新，所以弹框要在报这个错时出现。
- **偏离预研：不收窄录基线前的冲突检查（`check_recordable`），只改文案。** 预研 §7.1 第 2 条要求收窄，复核时还把它列为 B 的硬前提，理由是「再更新一次会变成混合版本」。不收窄的理由有三条：
  1. 上一条决定之后，这种情况不会发生。
  2. 剩下的情况是 S5 放行之后白名单本来就是混合版本（external 没跟着更新），很少见。这时混合录制被拦、journal 停在 capturing-baseline。在冲突列表里解决之后点「继续」就从录制重来，能走完，不丢东西。
  3. 这道检查也被初始化、重建、跟进共用。现有测试（`tests/svn_follow.rs` 的 `manual_follow_refuses_when_svn_conflicted` 等）**特意**拿白名单里的非 W-file 冲突去触发它：W-file 冲突在判定那一步就被看见了，到不了这里。收窄会改变跟进的行为，并让这些测试失去它们要测的路径，超出这次的需求。
- 计划 Task 1 因此取消，文案改动并入 Task 4。

### Task 2：S5 放行

- **计划没写的坑：分类只能在 S5 现读。** 冲突读整个工作副本（同改造前），W-file 判定用 `WScope::for_repo`（尊重 `.gitignore` 的仓库里，被忽略的 `.cs` 不在 git 里，算 git 不管的文件）。目录按磁盘判：目录上的属性冲突，`svn status` 的条目上看不出是不是目录。
- **为什么这么改是安全的。** 放行的冲突都在非 W-file 上。S6 单值录制只读 W-file（`run_preflight` 的 P1 只拦 obstructed，冲突文件不进基线树，新测试 `a_conflict_on_a_non_code_file_inside_the_whitelist_is_let_through` 断言了基线树只有 `.cs`）；S9 rebase、S10 放回 WIP 只动 git 跟踪的文件。混合录制会被 `check_recordable` 拦下，见上面的决策。
- **剩余风险：崩溃恢复。** S5 之后、S6 之前进程崩溃，继续时会从 S3 重来、再跑一次 `svn update`。冲突文件这次会被跳过；服务器在这期间又有提交的话，它停在旧 revision，白名单变成混合版本，走到混合录制被拦。处理方式同上（解决之后点「继续」）。没有为它单独写测试。
- **测试改写（Task 2）。** 两条原来断言回退的用例（内容冲突、属性冲突）改成断言照常完成，名字随之改（`a_content_conflict_outside_git_no_longer_rolls_back`、`a_property_conflict_outside_git_no_longer_rolls_back`），并加了一行 `Code/A.cs` 的服务器改动，用来证明 git 那一侧确实走完了。
- **结局不带冲突清单。** 前端在流程走完后单独查一次。见设计 §2.1。
- **交付照旧拦全部冲突。** 预研 §7.3 第 2 条给了两个选项。`svn commit` 本身是 `--depth=empty` 加逐个文件（`svn_sync_out.rs` D5），不会碰到别的冲突文件；但提交之后 D6 要对白名单跑一次 `svn update`，冲突文件被跳过、停在旧 revision，D6 的冲突检查（`sync_out_update_conflict`）和 D7 的单值预检都会在**提交已经发出去之后**失败。收窄等于把失败从「开始前」推到「提交之后」，所以不收窄。设计 §2.4 的理由据此改正。

### Task 3：冲突列表与解决

- **计划没写的坑：`svn info --xml` 的冲突形状（2026-11-28 实测，svn 1.14.2）。** 文本冲突带 `prev-base-file`、`prev-wc-file`、`cur-base-file` 三个绝对路径（正斜杠）；二进制冲突**没有** `prev-wc-file`（磁盘上的文件本身就是「我的」）；属性冲突只有 `prop-file`；目录的 `<version kind="dir">`。`mergeable` 按「三个文件都在」判，二进制自然落到不可合并。
- **计划没写的坑：只有属性冲突时 `mine-full` / `theirs-full` 只动属性。** 实测 `mine-full` 后状态 ` M`、属性是本地值；`theirs-full` 后状态干净、属性是服务器值。内容没动。
- **为什么这么改是安全的。**
  - 解决前现读一次这个文件的冲突状态（`current_conflict`），树冲突、目录、W-file 一律拒绝，不信任前端手里的列表。
  - 路径过 `check_rel_path`（拒绝空串、绝对路径、`..`）。传给 svn 时带 `@` 结尾，路径里有 `@` 也不会被当成 peg revision。
  - 「标记已解决」（`working`）前检查磁盘上没有 `<<<<<<< ` / `>>>>>>> ` 开头的行，防止把带标记的文件标成已解决。
  - 解决后再读一次，仍是冲突就报错。
  - 同步锁被拿着时拒绝（`assert_not_syncing`）：与 S5 的 `svn update` 抢 wc.db。
- **外部合并不复制三方文件。** 直接把 svn 留在工作副本里的 `.mine` / `.rN` 交给工具，svn resolve 时会删掉它们。`try_lock` 与 git 冲突的外部合并共用，同一个文件不会被开两次。
- **偏离计划：Task 3 的测试是和实现一起写的**，没有先看到它失败。这几条测的是新模块（之前不存在），失败形态只能是「编译不过」；Task 2 的 S5 用例是先失败后通过的。

### Task 4：命令与文案

- 命令 `svn_update_conflicts(path)`、`svn_resolve_conflict(path, file, choice)`、`open_svn_conflict_merge(path, toolId, file)`。JS 侧参数 `toolId` ↔ Rust `tool_id`，与 `open_external_merge` 同一形状。全部 `#[tauri::command(async)]`（`tests/commands_are_async.rs` 扫源码守着）。
- `svn_preflight_conflicted` 的文案也被初始化向导用：向导里还没有配置，冲突列表打不开，所以文案写成「请先在 TortoiseSVN 里解决；已初始化的仓库里……也可以在本应用的冲突列表里解决」。

### Task 5–6：前端

- **偏离设计：冲突列表的弹出时机多了 `sync_in_update_conflict`。** 更新因为不能放行的冲突整次回退时也弹：列表把要去 TortoiseSVN 的和能在这里解决的分开给人看，比只读错误文案清楚。
- **偏离设计：停在 git 冲突上（`rebaseConflict`、`stashConflict`、`stashBlocked`）时不弹。** 用户正在解 git 的冲突，流程没走完，等点「继续」走完再查（`offerAfterOutcome`）。
- **`offerSvnConflicts` 不等、不抛。** 查冲突要跑一遍全量 `svn status`（公司机器上约 4 秒），放进 `running` 里会拖住下一次同步；查失败只记 console，主流程已经走完。代价：每次更新走完都多一遍全量 `svn status`，没有冲突时什么都不弹。
- **外部合并工具按规则选（`toolFor(cfg, path, 'merge')`）。** `.prefab` 在工具配置里配成 UnityYAMLMerge 时就用它，不会落到 BC 的文本合并。没有可用的合并工具时，「用外部工具合并」和「标记已解决」两个按钮都不给。
- **「用我的」「用服务器的」在行内确认，不另弹确认框。** 弹框里再弹框会盖住列表；错误也挂在那一行下面，长列表里挂在框底会出在视野外。

### Task 7：验证

- **全量测试里暴露的连带改动。** `tests/svn_sync_in.rs` 的 `a_rollback_blocked_by_a_server_added_file`（三条「回退被撞名挡住」用例共用的 helper）原来靠 `Art/x.psd` 的内容冲突触发回退；这类冲突现在放行，helper 改用目录 `Art` 上的属性冲突触发，三条用例要测的「回退停在 applying-stash」不变。
- **E2E（`pnpm tauri dev`，2026-11-28）。** 临时 fixture（`file://` 仓库 + wc1/wc2，白名单 `Code/`、扩展名 `.cs`），`--config` 换 identifier 隔离，假合并工具（PowerShell 把 `$REMOTE` 拷成 `$MERGED`），经 CDP 驱动真实窗口；确认运行的 `app.exe` 在本 worktree 的 `target\debug` 下。
  1. invoke 参数名：`svn_resolve_conflict { path, file, choice }`、`open_svn_conflict_merge { path, toolId, file }` 正例到达后端；写错成 `choise` / `tool_id` 的反例被 Tauri 以 `missing required key` 拒绝。
  2. 点「更新」：服务器 r2 同时改了 `Code/A.cs` 与四个 git 不管的文件（二进制 `Art/Hero.png`、属性 `Art/Icon.prefab.meta`、文本 `Art/UI_Main.prefab`、白名单内的 `Code/ui.png`）。更新完成，基线到 r2，回到 `work`；随后弹出冲突列表，四行，按钮符合预期（二进制、属性两个按钮，文本四个）。
  3. 「用服务器的」（`Hero.png`）先在行内确认，确认后字节是服务器的、状态干净；「用我的」（`Icon.prefab.meta`）属性是本地值、状态 ` M`。
  4. 带冲突标记时「标记已解决」被拒（`svn_conflict_markers_left`，错误挂在该行下面）；用外部工具合并后再点，状态干净、`.mine/.rN` 被删。
  5. 还剩 `Code/ui.png` 冲突时再点「更新」：顶部报 `sync_in_svn_conflicted`（分两组列），冲突列表重新弹出。
  6. 解决最后一个后显示「冲突都解决了。」；`svn_update_conflicts` 返回空，`svn_sync_in_preview` 正常通过。
  - 没有在真 TortoiseSVN 里看（同一个 wc.db，`svn status` 已显示 `M` 而不是 `C`）。

### 独立 review（`/code-review high`，2026-11-28）

| 发现 | 级别 | 处理 |
|---|---|---|
| 一行在跑时别的行按钮仍可点，两个 `svn resolve` 并发；先完成的清掉 busy，Esc 能在另一行没结束时关框 | important | 已修：按钮按框级 `anyBusy` 禁用，`busy` 只管按钮上的字。组件测试覆盖 |
| 内容 + 属性都冲突时「标记已解决」（`--accept working`）会把属性冲突按本地值一并解决，没有提示 | important | 已修：这种情况先在行内确认，提醒写明属性按本地值、服务器的属性改动丢掉（`needsConfirm`）。没有改成拒绝：svn 命令行不能只解决一种冲突，拒绝就等于不让合并 |
| 外部合并工具还开着时「用我的 / 用服务器的」照样执行，之后工具一保存就把结果盖回来 | important | 已修：`resolve_conflict` 先 `try_lock`，被占时报 `external_tool_busy`。Rust 测试覆盖 |
| `.gitignore` / `.gitattributes` 冲突被当成 git 不管的文件放行，S6–S7 按带冲突标记的规则录基线 | important | 已修：任何一级的这两个文件按 W-file 对待，整次回退。Rust 测试 `a_conflict_on_a_gitignore_still_rolls_back` |
| 列表在别的同步确认框打开之后才弹出来，抢焦点 | nit | 已修一半：查完时有同步流程在跑就不弹（那次走完会再查）。别的非同步弹框（删分支等）开着时仍可能叠上去，没处理 |
| 只剩要去 TortoiseSVN 的冲突时，开头仍说「这些文件 git 不管」 | nit | 已修 |
| `setPending` 的 updater 里有副作用 | nit | 已修：当前请求放 ref，updater 只赋值 |
| 交付执行中 D6 的 `sync_out_update_conflict` 不弹列表 | nit | 已修：加进错误码，交付执行与「继续」都接上 |
| `svn_resolve_conflict` 没挂起监听 | nit | 已修：命令层包 `with_watch_suspended`（签名多了 `AppHandle`，JS 侧参数不变，已在 `pnpm tauri dev` 里实调一次确认） |

修完之后：`cargo fmt --check`、`cargo clippy --all-targets`（0 告警）、`svn_sync_in`（91）/ `svn_conflict`（12）/ `svn_sync_check`（28）/ `svn_sync_out_check`（18）/ `commands_are_async` 通过；`pnpm build`、`pnpm test`（2088）、`pnpm test:layout`（218）通过。
