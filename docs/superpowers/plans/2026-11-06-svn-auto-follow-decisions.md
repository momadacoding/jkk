# 自动跟进（事故分析 6.12） 实现日志

计划：`2026-11-06-svn-auto-follow.md`。用户 2026-11-07 指示「开始 6.12」。分支 `worktree-feat-svn-auto-follow`，基点 a41c005。

## 进度总账

| Task | 内容 | 状态 | 验证 |
|---|---|---|---|
| 1 | 后端：`follow_auto`、`AutoFollowOutcome`、条件 6 / 7 / 8 / 11 / 12，自动模式下拼索引先于发布基线 | 完成 | 新增 `tests/svn_follow_auto.rs` 20 条全过；`tests/svn_follow.rs` 136 条全过（重构了 `publish`） |
| 2 | 设置 `svnAutoFollow`（默认开）与命令 `svn_follow_auto` / `set_svn_auto_follow` | 完成 | `tests/app_settings.rs`（旧文件没有这个字段时默认开、关掉的留着关）、`tests/svn_backend.rs`（`AutoFollowOutcome` 的序列化形状）、`tests/commands_are_async.rs` 通过 |
| 3 | 前端：判定写回后触发、与横幅「跟进」共用 `exclusive`、同一现场不重试、完成提示、横幅写明没自动的原因、设置框开关 | 完成 | `pnpm build` 过；`pnpm test` 77 个文件 1297 条全过（新增 `svnSyncFlow` 6 条、`BaselineBanner` 4 条、`svnFollow` 4 条、`SettingsDialog` 2 条） |
| 4 | watcher 放行收件箱里新到的事件（钩子一写事件就跑检测） | 完成 | `watch.rs` 单测 `a_hook_event_landing_in_the_inbox_wakes_the_check` |
| 5 | 文档：分析 §6.12 状态、计划状态、E2E 清单 | 完成 | 新增 `e2e/2026-11-07-svn-auto-follow-e2e.md`（7 个场景，未跑） |
| 6 | `pnpm tauri dev` 真实窗口验证新命令 | 完成 | 2026-11-07，确认跑的是本 worktree 的 `target\debug\app.exe`，`--config` 换 identifier 隔离。E2E 场景 1、4、6（关掉那一半）、7 通过，`set_svn_auto_follow` 参数名正反例都验了，见 `e2e/2026-11-07-svn-auto-follow-e2e.md`。tauri dev 又把 `Cargo.toml` 改成 LF，用 `unix2dos` 改回 |

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
- 计划只写了 `post_update`。收件箱里的提交事件（`post_commit`）同样放行：TortoiseSVN 只提交、不更新时根 revision 不动，原来要等下一次焦点才检测到，这正是 11-05 的形态。`pre_commit` 不放行（review 低-1）：它在提交开始时写，那时去检测只会撞上被锁的工作副本。

**为什么这么改是安全的**
- 只放行改名落地的 `-post_commit.json` / `-post_update.json`，钩子先写的 `.tmp`、`pre_commit` 和目录本身不算。跟进收尾清收件箱发生在 `with_watch_suspended` 里。挂起是在去抖之后、发事件时才判的，删除若落在恢复前的 300ms 内，恢复后最多再发一次 `repo-changed`（review 低-1 指出原写法「删除事件被丢弃」不严格），那时判定已是 InSync，不会再跟进，不构成回路。

### 独立 review（opus subagent，commit 5eaee73）

没有高危。崩溃恢复与「不自动时一个 ref 都不动」逐条核过，成立；手动 `publish` 的重构逐步对照，行为不变。

| 级别 | 发现 | 处理 |
|---|---|---|
| 中-1 | 暂时性原因（外部 git 拿着索引锁、准备期间仓库在变、另一次同步在跑）跳过之后，同一现场永不重试。钩子事件让检测紧跟在 TortoiseSVN 之后，正好撞上 IDE 跑 `git status` | 这三个 code（`svn_index_busy`、`svn_follow_state_changed`、`sync_in_progress`）在流程里等 2 秒再试一次，还不行才写进横幅。不选「不记现场」：每次 `with_watch_suspended` 补发的 `repo-changed` 都会再跑一遍服务器体检 |
| 中-2 | 设置关着时，merge driver 有风险的话横幅仍写「没有自动跟进：merge driver……」 | 前端先读开关（`autoFollowEnabled`），关着直接返回，不查 merge driver；后端仍判一次 |
| 中-3 | 应用内「提交」的提交前检查会刷新判定，effect 随即发起自动跟进，与 `git commit` 交错（撞索引锁、跟进中途回退并报错） | 提交期间（`committing` ref）effect 不发起，提交结束 `autoNudge` 让它再看一次；跟进在跑时点「提交」直接提示等它做完，写好的说明留着 |
| 中-4 | `condition_1_…` 名为「未收尾的记录或同步锁」，只测了同步锁；`condition_4_…` 只断言 `starts_with("svn_")` | 补上未收尾记录（断言 `svn_follow_unfinished`、别人的记录不碰）；SVN 冲突改成 `assert_skipped_untouched(.., "svn_preflight_conflicted")` |
| 中-5 | 总账 Task 5 过时；Task 6（`pnpm tauri dev`）没做；`pnpm test:layout` 没有运行记录 | 总账已更新；`pnpm test:layout` 第一次 2 条失败（`confirm-long-warning` 打开页面超时，本 PR 没动它，当时完整 `cargo test` 正占满 CPU），重跑 140 条全过；Task 6 见总账 |
| 低-1 | watcher 也放行了 `pre_commit`；「删除事件被丢弃」措辞不严格 | 只放行 `post_commit` / `post_update`，测试补 `pre_commit` 不算；措辞已改（见 Task 4） |
| 低-2 | 自动跟进崩在基线 CAS 之后时，「继续」走手动跟进，横幅文案没说；之后自动跟进以「分支落后」跳过，原因文案与真实原因不符 | 不改：点「继续」时有人在场，结果与点「跟进」一样；记为已知限制 |
| 低-3 | `PrepareIndex` 的恢复不核对锁是不是我方的；自动模式这一步已记下指纹，窗口变大 | 有指纹且锁在时用 `lock_is_ours` 核对，不是我方的报 `svn_index_busy`、记录留着。补测试 `a_lock_swapped_for_someone_elses_before_the_baseline_was_published_is_left_alone` |
| 低-4 | `give_up` 里删 journal 失败时返回 `Err`，而什么都没动 | 不改：记录删不掉时下一次跟进会被它挡住，报错比静默返回 `Skipped` 更诚实 |
| 低-5 | 现场键的时序缝：在跑时来的新现场返回 `false`、不记，之后判定若不再写回就不会再试 | 不改：只是少一次自动尝试，下一次判定写回（存盘、切窗口、钩子事件）就会补上 |
| 低-6 | `retained_overlap` 只被一个保留下来的 commit 测过；Windows 大小写不敏感没处理 | 条件 11 的测试改成两个保留下来的 commit、只有第二个碰到 B。大小写：SVN 与 git 都以大小写敏感的形式记路径，同一个文件两边写法相同，不处理 |
| 低-7 | `saveAllSettings` 没测；merge driver 检查期间按钮没反馈 | 不改：前者是两次 invoke 的串联，失败时框里有错误；后者只有几十毫秒 |

另外：设置关着时这一次的现场也记下了，随后打开开关不会对当前现场补跑，要等下一次判定变化（review 中-2 附带的低危）。不改，记为已知限制。
