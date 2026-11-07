# 自动跟进（事故分析 6.12） 实现日志

计划：`2026-11-06-svn-auto-follow.md`。用户 2026-11-07 指示「开始 6.12」。分支 `worktree-feat-svn-auto-follow`，基点 a41c005。

## 进度总账

| Task | 内容 | 状态 | 验证 |
|---|---|---|---|
| 1 | 后端：`follow_auto`、`AutoFollowOutcome`、条件 6 / 7 / 8 / 11 / 12，自动模式下拼索引先于发布基线 | 完成 | 新增 `tests/svn_follow_auto.rs` 20 条全过；`tests/svn_follow.rs` 136 条全过（重构了 `publish`） |
| 2 | 设置 `svnAutoFollow`（默认开）与命令 `svn_follow_auto` / `set_svn_auto_follow` | 完成 | `tests/app_settings.rs`（旧文件没有这个字段时默认开、关掉的留着关）、`tests/svn_backend.rs`（`AutoFollowOutcome` 的序列化形状）、`tests/commands_are_async.rs` 通过 |
| 3 | 前端：判定写回后触发、与横幅「跟进」共用 `exclusive`、同一现场不重试、完成提示、横幅写明没自动的原因、设置框开关 | 完成 | `pnpm build` 过；`pnpm test` 77 个文件 1297 条全过（新增 `svnSyncFlow` 6 条、`BaselineBanner` 4 条、`svnFollow` 4 条、`SettingsDialog` 2 条） |
| 4 | watcher 放行收件箱里新到的事件（钩子一写事件就跑检测） | 完成 | `watch.rs` 单测 `a_hook_event_landing_in_the_inbox_wakes_the_check` |
| 5 | 文档：分析 §6.12 状态、计划状态、E2E 清单 | 未开始 | |
| 6 | `pnpm tauri dev` 真实窗口验证新命令 | 未开始 | |

## 决策条目

### Task 1：后端

**偏离计划**
- 结局用单独的 `AutoFollowOutcome { Done, Skipped }`，不往 `FollowOutcome` 里加 `Skipped`。自动跟进不会有 `NeedsManual`（冲突连基线都不发布），也不需要区分 `Refused` 与 `Skipped`：两者对前端都是「一个 ref 都没动，挂横幅写明原因」。
- **只接 ExternallyAdvanced，不接分支跟进入口。** 分析 §6.12 的触发写的是「判定为 ExternallyAdvanced（或命中分支跟进入口）」。分支跟进入口出现在两种时候：上次跟进冲突、只发布了基线；用户切到了一条落后的分支（「仅切换」是有意的选择）。前者重放还是冲突，自动了也没用；后者自动移分支等于推翻用户刚做的选择。`prepare(repo, auto)` 在 InSync 那一臂直接返回 `svn_auto_follow_branch_behind`，不跑预检和重放。
- **条件 12 按「先拼索引、后发布基线」实现，不按计划推荐的「把判定前移到 prepare（只算不写）」。** 只算不写的话判定时不持 index.lock，判完到拼索引之间外部 `git add` 一次，算出来的决策就不是真正执行的那份，还得再比一次。把顺序倒过来，判的就是拿着锁拼好的那份，不满足时删锁即可，什么都没动。只对自动跟进倒：手动跟进不看决策，没理由多锁一次 `update-ref` 的时间，也不动已有的 136 条测试钉住的步骤顺序。
- 条件 5、9 没写新代码。5：录制已经拒绝没确认成的删除（`HealthMap::Unconfirmed` → `not_confirmed_error`），断网时确认不了，由条件 6 挡住，`removed` 在 Verified 的录制里都是服务器确认过的。9：录制已经拒绝超过 `MAX_MAP_ENTRIES` 的映射（`svn_baseline.rs`），计划写的「加上限常量」与它是同一个上限，另设一个更小的自动上限没有依据。
- 条件 11 额外把 `drift` 并进交集。按定义 drift 是交集的子集，并进来只是兜底，不改变结论。
- 不自动时删掉这次钉住旧索引的 `refs/mygitclient/follow/<opid>`：没换入过索引，钉住的就是此刻的真实索引，留着只会让每次不自动都多一个 ref。

**计划的错误**
- 条件 12 计划写「拼索引无三方合并」，分析 §6.12 原文是「没有三方合并**冲突**」，通知一节还要列出「哪些做了三方合并」，即合得干净的允许自动。按分析：只拦 `lost`（合并冲突、二进制、删除 / mode 变化、文件与目录撞名，暂存内容保不住）与 `uncertain`。`discarded` 也放行，理由见 `stage_blocker` 的文档。

**计划没写的坑**
- `record_prepared` 会把 journal 推进到 `MoveBranch`。自动模式下这时基线还没发布，崩在这里时恢复（`settle_branch_and_index`）会把分支往前移，`svn/base` 却留在原地。加了参数 `next`：自动模式留在 `PrepareIndex`，CAS 基线之后再 `advance` 到 `MoveBranch`。`the_journal_stays_at_prepare_index_until_the_baseline_is_published` 钉住每个注入点的步骤。
- 用 `git log --format= --name-only --no-renames -z new..tip` 取保留下来的 commit 改过的路径：每条路径以 NUL 结尾，commit 之间没有分隔（2026-11-07 在临时仓库实测 git 2.51）。

**为什么这么改是安全的**
- 不自动的每条路径都断言「全部 ref 与索引字节不变、没有 index.lock / journal / 同步锁残留」（`assert_skipped_untouched`）。条件 12 拦下时已经拼过索引，同样断言。
- 发布顺序倒过来之后，恢复规则不用改：步骤停在 `PrepareIndex` 时恢复是「删我方的锁、清记录、重新跟进」，基线动没动都是一致状态。11 个注入点逐个崩一次再 `resume_follow`，结果都与没崩一样（`a_crash_at_any_step_of_an_automatic_follow_is_recoverable`），并且断言「基线没动时分支也没动」与「不留下分支已移、索引是旧的」。
- 全部满足时磁盘的内容与 mtime 前后逐文件相同（`tortoise_commit_is_followed_automatically_without_touching_the_disk`）。
- 手动跟进只是把发布拆成 `compose_index` + `swap_in` 两个函数，顺序与步骤不变，原有 136 条测试全过。

### Task 2：设置与命令

**偏离计划**
- 开关放在应用级设置（`settings.json`），所有混合仓库共用一个，不做按仓库的开关。计划只写了「设置里」；设置框本来就是应用级的，按仓库的开关要另做入口。
- 单独一条命令 `set_svn_auto_follow(enabled)`，不往 `save_app_settings` 里加参数。那条命令要换全局快捷键，失败回滚的路径与这里无关；分开也少和第 128 条（快捷键改成开关，未合并）在 `DesktopState::apply` 上撞车。设置框的「保存」先存快捷键、再在开关改了时调这一条（`saveAllSettings`）。
- 开关在后端判：`svn_follow_auto` 关着时直接返回 `Skipped { code: svn_auto_follow_off }`，不读工作副本。前端不用另外读设置，改了马上生效。

**为什么这么改是安全的**
- `AppSettings` 的新字段带 `#[serde(default)]`，0.7.3 写的设置文件读出来自动跟进是开的，与用户 2026-10-30 的选择一致；显式写了 `false` 的留着关（`a_settings_file_from_before_auto_follow_turns_it_on`）。
- `apply` 存盘时带上内存里的 `svn_auto_follow`，保存快捷键不会把开关冲掉。

### Task 3：前端

**偏离计划**
- 「不打断操作的提示」做成横幅区最上面的一条灰色（有要留意的内容时黄色）横幅，带「查看详情」「知道了」，不做浮动的 toast：仓库里没有 toast 组件，横幅区已经是这类状态的位置。详情就是手动跟进用的那份报告框。
- 同一个现场只试一次：现场键是「HEAD + 判定的全部内容」（`autoFollowKey`）。不加这一条的话，不自动之后每次第二档写回（存盘、切回窗口、钩子事件）都会再跑一遍录制与服务器体检。代价：因为条件 12（暂存拿不准）没自动、随后用户改了暂存，判定与 HEAD 都没变，不会重试，要点「跟进」。
- 没自动的原因只在键还对得上时显示，过时的原因不会挂在新的横幅上。
- 只在当前标签自动：报告与提示要有人看得见。切回该标签时 effect 会再跑一次。
- merge driver 的检查：手动跟进前会弹确认框（spec §4.6），自动跟进不能停下来问人，有风险就不自动，横幅写明原因；检查本身失败也不自动（按钮那边是放行，因为有人在场）。

**计划没写的坑**
- `createSvnSyncFlow` 建在 `RepoTab` 的 loading 提前 return 之后，触发自动跟进的 effect 必须在它之前声明（hook 顺序），所以经 `autoFollowRef` 调用。
- 「有流程在走」要同步判断：`handleAutoFollow` 返回 `false` 时调用方不记现场，那个流程收尾刷新判定后 effect 会再来；`exclusive` 的 `running` 在第一个 await 之前置上，返回 `true` 时可以放心记下。

### Task 4：watcher

**偏离计划**
- 计划只写了 `post_update`。收件箱里的提交事件（`post_commit`）同样放行：TortoiseSVN 只提交、不更新时根 revision 不动，原来要等下一次焦点才检测到，这正是 11-05 的形态。

**为什么这么改是安全的**
- 只放行改名落地的 `.json`，钩子先写的 `.tmp` 和目录本身不算。跟进收尾清收件箱发生在 `with_watch_suspended` 里，删除事件被丢弃，不会形成「跟进 → 清收件箱 → 事件 → 再检测」的回路；即使有一轮，判定已是 InSync，不会再跟进。
