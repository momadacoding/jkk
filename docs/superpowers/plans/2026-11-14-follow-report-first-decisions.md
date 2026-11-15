# 跟进：报告先弹、同步后少一轮复查、记耗时（2026-11-14）实现日志

输入：2026-11-11 使用报告「跟进耗时约 70 秒」10.5 节里还没做的三项（记分步耗时、报告先弹界面后台刷新、P3 不为自己引起的 `repo-changed` 重查）。没有对应的计划文件，本文件同时记方案取舍。只改前端，Rust 一行不动。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 同步之后的强制复查与补发的 `repo-changed` 共用一轮 | 完成 | `engine/svnCheckQueue.ts`（`mark` / `since`）、`store/repoStore.ts`（`refreshSvnSync`、`readHeadInfo`、`forceCheckAtHead` 的 `since`） | `svnCheckQueue.test.ts` 新增 6 条；`repoStoreSyncSettle.test.ts` 新增 7 条，把 `refreshSvnSync` 换回旧实现时 2 条因断言变红（多跑一轮）；`repoStoreSvnSamePath.test.ts` 新增 1 条 |
| T2 横幅「跟进」完成后报告先弹，刷新在后台并发 | 完成 | `components/svnSyncFlow.ts`（`follow`、`afterFollow`、`afterSync`）、`RepoTab.tsx`（`refreshAfterFollow`） | `svnSyncFlow.test.ts` 新增 5 条、改 3 条，改之前 7 条红 |
| T3 记耗时：`--perf` 打点、报告页脚显示用时 | 完成 | `svnSyncFlow.ts`（`follow` / `follow.refresh` / `autoFollow` span、`follow.report` / `autoFollow.done` mark）、`engine/svnFollow.ts`（`elapsedMs` / `elapsed`）、`FollowReportDialog.tsx` | `svnFollow.test.ts` 2 条、`FollowReportDialog.test.tsx` 1 条；`pnpm test:layout` 164 过（超长场景带上了用时那一行） |
| T4 真实窗口验证（`pnpm tauri dev` + fixture） | 完成 | — | 见「真实窗口验证」 |
| 独立 review（`/code-review high`） | 9 条：修 7 条、记 2 条 | 见「review 处理」 | 修之前新增的 4 条测试里 3 条因断言变红；修完全量重跑 |

全量（review 修完之后的最终代码）：`pnpm build` 通过；`pnpm test` 98 个文件、1695 条通过。`pnpm test:layout` 164 条通过是 review 之前跑的，之后没有再动弹框的结构与样式，没重跑。Rust 没有改动，没跑 cargo。

## 方案取舍

### P3：怎么让两轮强制复查变成一轮

现状：同步命令（跟进、更新、交付）返回前，后端补发一次 `repo-changed`（`with_watch_suspended`）。同步多半挪了 HEAD，监听读到新 HEAD 就强制跑一轮第二档；流程收尾的 `refreshSvnSync` 又强制跑一轮。两轮各是一遍 `svn status` 全量扫描（公司机器约 4 秒一遍），结论相同。

| 方案 | 做法 | 结论 |
|---|---|---|
| A 流程不再自己复查，靠补发的 `repo-changed` | 删掉 `afterSync` 里的 `refreshSvnSync` | 不做。HEAD 没动的同步（基线分支上跟进、重建）监听那边只比 wc.db 戳，戳没变就不跑，横幅撤不掉 |
| B 后端把复查的结论随跟进一起返回 | `svn_follow` 返回值带 `SyncStatus` | 不做。要改命令的返回结构，而且后端补发的 `repo-changed` 照样会让监听再跑一轮，省不下来 |
| C 复查按现读的 HEAD 跑，并复用晚于自己开跑的强制一轮 | 见下 | **采用**。只动前端的排队逻辑，不改任何一轮检测本身 |

C 的两处配合：

1. `refreshSvnSync` 先现读 HEAD，走 `forceCheckAtHead`。这一轮在跑时监听读到同一个 HEAD，`checkSvnSyncIfHeadMoved` 认出已经有人在校验（`headRequested`），不再排；写回后记下 HEAD，之后的事件也不再跑。
2. `refreshSvnSync` 进来的那一刻在队列上取号（`svnCheckQueue.mark`）。读 HEAD 的这一拍里监听那一轮可能先开跑了：它是强制的、开跑在取号之后，结论不比自己跑的旧，直接复用。

**为什么这么改是安全的**

- 单飞不复用在跑的一轮，理由是它可能开跑在调用方的写操作之前（`singleFlight.ts` 的注释）。取号把这个理由逐条排除：只复用序号大于取号值的轮次，也就是调用之后才开跑的。调用之前就在跑的照旧补跑，有测试钉着（「复查进来之前就在跑的强制一轮不复用」）。
- 只复用强制的一轮。比戳的一轮在戳没变时不跑第二档，不能顶替。
- 读不出 HEAD 时退回原来的做法（不认 HEAD 的强制一轮）。
- 最坏情况（流程晚进来一步、监听那一轮在取号之前已经开跑）是补跑一轮，与改动前相同，结论仍然对。

**计划没写的坑**

- `refreshSvnSync` 原来不读 HEAD。现读用的若是 `refreshHeadInfo`，它会写 `state.head`：用户在这一拍里切到别的仓库时，旧仓库的分支名会挂到新仓库的标题上。新加只读的 `readHeadInfo`，读回来再核对 `openedPath`，切走了就不为旧仓库发校验。全量测试里 `repoStoreSvnSamePath.test.ts` 的 4 条旧用例因为多了一次 `open_repo` 而挂，才注意到这一点；补了应答，另加 1 条「读 HEAD 时切走」。
- 两个调用方共用一轮时各自都走 `forceCheckAtHead`，`headRequested` 被后来的那个对象顶替。先收尾的那个按 `=== head` 判断不清它，后收尾的清；`lastCheckedHead` 两次都记成同一个 HEAD。`ownCommitOnPending` 按对象认登记时的那一轮，不受影响。

### 报告先弹

- **只改横幅「跟进」完成这一条路。** 拒绝、冲突照旧先刷新完再弹：冲突的下一步是手动跟进（写磁盘），它的确认框弹出时不该还有一轮检测在读工作副本。跟进记录的「继续」在确认框的 `action` 里跑，框本身显示着进行中，没动。
- **自动跟进的提示照旧等刷新完再挂。** 它不挡操作，早挂会和还没撤的「SVN 侧有新内容」横幅叠在一起，两条说的是相反的事。
- **流程要等报告关掉和刷新完成都结束才算走完。** `running` 提前放开的话，刷新写回 HEAD 的那一刻判定还是跟进前的 ExternallyAdvanced，`RepoTab` 的自动跟进 effect 会按这个过时的现场再发起一次。有测试钉着（「报告关掉了、刷新还没完」）。
- **按钮在刷新完成时复位，不等报告。** 刷新写回之前横幅说的还是跟进前的事。
- **刷新从一项等一项改成三路同时发。** 读状态、读落后分支各一路；读图排在读 HEAD 后面成一路。第一版把四项全部并发，日志里还写了「核对过 `refreshGraph` 不读 `headHash`」，**这是错的**：它的 updater 经 `merge` → `spliceHistory` 拿 `s.headHash` 当「还被指着的提交」的起点，先写回图的话游离 HEAD 上的提交会被当成已交付藏起来（review 第 1 条）。`afterSync` 里读同步状态与复查也同时发：复查要赶在命令返回的同一拍里进去才能与监听共用一轮；顺带修了「同步状态读不出来，复查就不跑」（原来是同一个 try 里串行）。

### 耗时

- **偏离使用报告：没有做后端分阶段计时。** 报告要「判定、录基线、体检、重放、泄漏判定、复核、发布」各阶段。`--perf` 已经记每个子进程，跟进的耗时绝大部分在子进程里，按命令名就能归到阶段；另加阶段计时要改 `svn_follow.rs` 的每个入口。这次只补前端的三个时刻。阶段数据不够用时再加。
- **报告页脚的用时只量命令本身**（`svn_follow` / `svn_follow_auto` 从发出到返回），不含之后的刷新；自动跟进重试过时只算成功的那一次。`elapsedMs` 是前端补进 `FollowReport` 的字段，后端不发，崩溃后续上的那次没有。
- **偏离使用报告：release 构建没有另写日志文件。** `--perf` 就是 release 下的记录入口，报告页脚的用时不需要任何开关。

## 真实窗口验证（2026-11-14，开发机）

- 隔离：`pnpm tauri dev --config <json> -- -- --perf`，identifier `com.momadacoding.jkk.e2efollowfirst`；另一个并发会话的 dev 窗口占着 5174 / 9222，所以 json 里把 `devUrl` 与 `beforeDevCommand` 改到 5194，CDP 用 9233。核对过 9233 的 WebView 属于本 worktree 的 `target\debug\app.exe`。fixture 由 `scripts/e2e-svn-fixture.ps1` 造在 `%TEMP%`，验完连同两个配置目录一起删了。
- 驱动：Playwright `connectOverCDP`，`blur` / `focus` 用事件派发，TortoiseSVN 用命令行 `svn commit` / `svn update` 代替；IPC 次数靠包 `window.fetch` 数。
- **横幅「跟进」**（改了 A 和 D、只提交 A，自动跟进关着）：报告「跟进完成」，页脚「跟进用时 2.4 秒」；`svn_follow` 返回之后 `svn_sync_status` 只发了 **1 次**，后面两批 `repo-changed`（+0.3 s、+0.8 s）都没再发；关掉报告后横幅消失，`git status` 只剩 `D.cs`，基线记了 `Svn-Base-Rev: 3 Code/A.cs`。perf 记录里 `follow.report` 在 `svn_follow` 返回后 1 ms，`follow.refresh` 在那之后 239 ms 才结束。
- **自动跟进**（同事改 M1、我局部更新）：提示「已自动跟进 SVN 侧的新内容」，「查看详情」里页脚「跟进用时 2.2 秒」；`svn_follow_auto` 返回之后 `svn_sync_status` 同样只有 1 次。
- 之后 10 秒 0 次 IPC，没有反复检测或反复跟进。
- **没验到的**：改动前在真实窗口里是几次（「2 次」来自读代码与 vitest 里旧实现的对照，没有在 master 的窗口里数）；公司机器上的耗时；真的 TortoiseSVN 钩子触发的那条路。开发机上刷新只要 0.1–0.2 秒，「报告比刷新早」在这里看不出体感差别，只有 perf 记录里的先后。

## review 处理（`/code-review high`，2026-11-15）

review 只读代码，没有跑。9 条：

| 条 | 问题 | 处理 |
|---|---|---|
| 1 | `refreshGraph` 与 `refreshHead` 并发，而它的 updater 读 `s.headHash`；日志里「不读」的说法是错的 | **修**。读图排到读 HEAD 后面，见上面「报告先弹」 |
| 2 | 报告抢在刷新前面弹，而报告里的「放回 Local-Only 补丁」会写工作区：撞上读状态的索引锁会被拒，或者让扫描读到写了一半的文件 | **修**。流程把「刷新完成」的 Promise 交给报告（`showFollowReport` 第二个参数），放回先等它。按钮这时显示进行中 |
| 3 | 复用的那一轮可能是替另一个 HEAD 开跑的：流程的 HEAD 是在它开跑之后读的，复用后把新 HEAD 记成「校验过」，违反 `recordAfterSuccess` 的「观测值不晚于校验」 | **修**。记下在跑的那一轮是替哪次 HEAD 观测开跑的（`roundHead`），相同才复用。测试造的时序：监听读到 N 并开跑，流程随后读到 M，修之前只跑 1 轮 |
| 4 | 交付流程的 `afterSync` 仍是先等读同步状态、再复查，赶不上同一拍；日志却说交付也受益 | **修**。`svnSyncOutFlow` 的 `afterSync` 改成与 `svnSyncFlow` 相同的同时发。`handleMigrateEol` 没动：它不是同步流程，重建之后的复查本来就只有一轮 |
| 5 | 「更新」之后仍是两轮：它改了 wc.db，监听那边比戳的一轮在取号之前就开跑并跑了第二档，不是强制的，认不出来 | **不修，改正说法**。要认出它得让后端在补发的事件上带标记，或者让队列知道一轮比戳的检测实际跑没跑第二档，不在这次范围。日志原来写「更新也受益」，是错的：vitest 里那几条戳都没变 |
| 6 | 与监听共用一轮之后，这一轮失败就没有第二次尝试，横幅停在同步之前 | **修**。`refreshSvnSync` 失败后自己再试一次，恢复成原来的「两次尝试」；两次都失败才把错误交给流程 |
| 7 | `Promise.all` 在第一项失败时就返回，流程放开自动跟进，另外几项还在写 state | **修**。改 `allSettled`，全部落定后再抛第一个错 |
| 8 | 测试名写「拒绝、冲突」，只测了拒绝 | **修**。拆成两条，冲突那条断言「刷新没完时确认框不弹」 |
| 9 | 一次跟进同时发三次 `open_repo` | **记为取舍**。都是 libgit2 本地读，毫秒级；要省得改 `refreshAfterFollow` 与 `refreshSvnSync` 之间的接口 |

## 遗留

- 「更新」之后仍是两遍 `svn status`（review 第 5 条）。交付之后按代码应当是一遍，只在 vitest 里验过，没有在真实窗口里走。
- review 修完之后没有重新在真实窗口里走一遍。修的是并发顺序、失败重试和「放回等刷新」，主路径（一次跟进、一次复查、报告带用时）的代码没变；重试与「替另一个 HEAD 开跑」只有 vitest 覆盖。
- 跟进记录的「继续」没有带用时，报告也没有先弹。
- 另有两个并发会话在动相邻的代码（worktree `auto-follow-retry`、`follow-inproc-git`），合并时 `svnSyncFlow.ts` 可能冲突。
